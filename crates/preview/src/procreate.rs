//! Reading Procreate `.brush` and `.brushset` archives: the guarded zip and
//! plist readers, the `Shape.png` decoder and the two name/member lookups the
//! preview API needs.
//!
//! Every entry here parses untrusted bytes, so each size, count and dimension
//! is checked against a ceiling before anything is allocated.

use crate::bitmap::{decode_guarded, tip_plane, GuardedDecodeError, TipSample};
use crate::GrayscaleBitmap;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Cursor, Read};

/// Defensive ceilings for untrusted archives: anything above them is treated
/// as malformed rather than allocated from.
pub const MAX_ENTRY_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_PNG_DIMENSION: u32 = 16384;
/// The plist entry read cap. The largest of 5803 real `Brush.archive` and
/// `brushset.plist` files is 10164 bytes. The whole entry is held before any
/// guard runs, and an XML plist's strings are allocated before the stream
/// guard sees them, so this ceiling alone bounds both.
pub const MAX_PLIST_BYTES: usize = 1024 * 1024;
pub const MAX_PLIST_DEPTH: usize = 64;
/// A binary plist object may be referenced any number of times, and each
/// reference is expanded into its own value, so the tree can be far larger
/// than the file. Every value costs a `plist::Value` slot of about 80 bytes,
/// and a dictionary also carries an `IndexMap` table, so a tree of one-entry
/// dictionaries costs about 160 bytes per value. A census of 5803 real
/// `Brush.archive` and `brushset.plist` files found at most 949 values. It
/// also caps the objects and collection references a binary plist declares.
pub const MAX_PLIST_VALUES: usize = 10_000;
/// The expanded string and data bytes of one plist. The same census found at
/// most 7 KiB. Each accepted string is decoded twice, once per parse, and a
/// UTF-16 string holds its code units and its UTF-8 copy at once, so in a
/// binary plist this ceiling bounds what strings cost beyond the entry
/// itself. An XML plist's string is allocated before the stream guard sees
/// it, so only [`MAX_PLIST_BYTES`] bounds it.
pub const MAX_PLIST_PAYLOAD_BYTES: usize = 1024 * 1024;
/// The most bytes from the earliest byte zip could read as an archive's
/// central directory or zip64 end record to the end of the file. zip 9 reads
/// central headers from the directory's start to as far as the file goes,
/// whatever size the directory declares, and holds up to about 20 times their
/// bytes when they are dense with extra fields. The largest central directory
/// of 278 real Procreate files is 79,320 bytes.
pub const MAX_CENTRAL_DIRECTORY_BYTES: usize = 2 * 1024 * 1024;

/// Reads a zip entry of at most [`MAX_ENTRY_BYTES`]. Plist entries go through
/// [`read_zip_plist`].
///
/// `path` is matched against the raw name bytes. zip decodes a name that is
/// not UTF-8 as CP437, so the name that `name_for_index`, `file_names` or
/// [`members_in_zip_order`] returns for such an entry is not found. [`Zip`]
/// finds such entries.
pub fn read_zip_entry(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<Vec<u8>, String> {
    let index = zip.index_for_name(path);
    read_capped(zip, index, path, MAX_ENTRY_BYTES).map_err(|e| e.to_string())
}

/// [`read_zip_entry`] capped at [`MAX_PLIST_BYTES`], so an oversize plist
/// fails before more than the plist ceiling is read.
pub fn read_zip_plist(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<Vec<u8>, String> {
    let index = zip.index_for_name(path);
    read_capped(zip, index, path, MAX_PLIST_BYTES).map_err(|e| e.to_string())
}

/// Why a [`Zip`] read failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipReadError {
    /// The entry declares more bytes than `limit`, the limit the read
    /// applied, so none were read.
    OverLimit {
        name: String,
        declared: u64,
        limit: usize,
    },
    /// Any other failure, with a message naming the entry.
    Failed(String),
}

impl std::fmt::Display for ZipReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZipReadError::OverLimit { name, declared, .. } => {
                write!(f, "{name}: declared size {declared} exceeds limit")
            }
            ZipReadError::Failed(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ZipReadError {}

/// A Procreate `.brush` or `.brushset` zip with two guards that
/// `zip::ZipArchive::new` lacks, so a consumer opens one through
/// [`Zip::open`] instead:
///
/// 1. Before zip parses the file, [`Zip::open`] rejects more than one end of
///    central directory record that zip would read a central directory for,
///    and a central directory or zip64 end record that starts more than
///    [`MAX_CENTRAL_DIRECTORY_BYTES`] before the end of the file. Without
///    that, zip can hold many times the file size. It also reads the archive
///    at offset 0 and rejects overlapping entries. See
///    <https://github.com/pmwoz/brushkit/issues/207>,
///    <https://github.com/pmwoz/brushkit/issues/210> and
///    <https://github.com/pmwoz/brushkit/issues/154>.
/// 2. A name that [`Zip::names`] or [`Zip::members_in_zip_order`] returns
///    finds an entry with that name. zip 9 finds an entry by its raw name
///    bytes but decodes a name that is not UTF-8 as CP437, so a raw lookup of
///    such a name misses. See <https://github.com/pmwoz/brushkit/issues/208>.
///    When a UTF-8 name equals another entry's CP437 decoding, both entries
///    list the same name and the lookup finds the entry whose raw name
///    matches.
#[derive(Debug)]
pub struct Zip<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
    /// The entries a raw lookup of their decoded name misses. It holds at most
    /// one entry per 46-byte central header, so
    /// [`MAX_CENTRAL_DIRECTORY_BYTES`] bounds it.
    decoded: HashMap<String, usize>,
    /// Finding these names walks the central headers zip read, each at least
    /// 46 bytes, and copies each one zip dropped once, so
    /// [`MAX_CENTRAL_DIRECTORY_BYTES`] bounds the walk, the names and each
    /// copy.
    conflicting_names: Vec<String>,
}

impl<'a> Zip<'a> {
    /// Opens `bytes` with both guards [`Zip`] describes.
    pub fn open(bytes: &'a [u8]) -> Result<Self, String> {
        const EOCD_SEARCH_BYTES: usize = 65_572;
        let fail = |e: zip::result::ZipError| format!("failed to open zip: {e}");
        // zip 9 searches the last `size_of::<Zip32CentralDirectoryEnd>() + 4 + 65_536`
        // bytes, 65,572 on 64-bit targets, for end of central directory records. It
        // reads a central directory for each one that passes its checks and holds on
        // to its headers, which can take many times the file size. This window must
        // not be smaller.
        // See <https://github.com/pmwoz/brushkit/issues/207>.
        let tail_start = bytes.len().saturating_sub(EOCD_SEARCH_BYTES);
        let mut starts = (tail_start..bytes.len()).filter_map(|end| directory_start(bytes, end));
        let start = starts.next();
        if starts.next().is_some() {
            return Err(fail(zip::result::ZipError::InvalidArchive(
                "more than one end of central directory record".into(),
            )));
        }
        // zip 9 reads a zip64 end record's extensible data into memory whole, and
        // central headers as [`MAX_CENTRAL_DIRECTORY_BYTES`] describes.
        // See <https://github.com/pmwoz/brushkit/issues/210>.
        if let Some(start) = start {
            let len = bytes.len() - start as usize;
            if len > MAX_CENTRAL_DIRECTORY_BYTES {
                return Err(fail(zip::result::ZipError::InvalidArchive(
                    format!(
                        "central directory or zip64 end record starts {len} bytes before the end of the file, over the limit of {MAX_CENTRAL_DIRECTORY_BYTES}"
                    )
                    .into(),
                )));
            }
        }
        // The default `ArchiveOffset::Detect` stores the offset of every central
        // header signature in the file, which can take many times the file size.
        let config = zip::read::Config {
            archive_offset: zip::read::ArchiveOffset::Known(0),
        };
        let mut archive = zip::ZipArchive::with_config(config, Cursor::new(bytes)).map_err(fail)?;
        // Overlapping entries let two names read the same stored bytes. Rejecting
        // them keeps the stored bytes all entries read within the file size.
        // See <https://github.com/pmwoz/brushkit/issues/154>.
        if archive.has_overlapping_entries().map_err(fail)? {
            return Err(fail(zip::result::ZipError::InvalidArchive(
                "entries overlap".into(),
            )));
        }
        let decoded = (0..archive.len())
            .filter_map(|i| {
                let name = archive.name_for_index(i)?.ok()?;
                archive
                    .index_for_name(&name)
                    .is_none()
                    .then(|| (name.into_owned(), i))
            })
            .collect();
        let conflicting_names = conflicting_names(&archive, bytes).map_err(fail)?;
        Ok(Zip {
            archive,
            decoded,
            conflicting_names,
        })
    }

    /// The names, as [`Zip::names`] decodes them, that two or more central
    /// headers list with a different CRC-32 or uncompressed size, sorted and
    /// each once. zip 9 keeps only the last of the headers with one raw name,
    /// so no read reaches the others. Headers with an equal CRC-32 and size,
    /// such as the 0-byte `.empty` entries real Procreate files repeat, are
    /// not reported.
    /// See <https://github.com/pmwoz/brushkit/issues/248>.
    pub fn conflicting_names(&self) -> &[String] {
        &self.conflicting_names
    }

    /// Every entry name as `name_for_index` decodes it, in central-directory
    /// order. An entry whose name does not decode is left out. A name can
    /// repeat, as [`Zip`] describes.
    pub fn names(&self) -> Vec<String> {
        (0..self.archive.len())
            .filter_map(|i| Some(self.archive.name_for_index(i)?.ok()?.into_owned()))
            .collect()
    }

    /// Whether an entry is found by `name`, raw or decoded.
    pub fn contains(&self, name: &str) -> bool {
        self.index_for_name(name).is_some()
    }

    fn index_for_name(&self, name: &str) -> Option<usize> {
        self.archive
            .index_for_name(name)
            .or_else(|| self.decoded.get(name).copied())
    }

    /// Reads the entry `name` finds, of at most [`MAX_ENTRY_BYTES`]. Plist
    /// entries go through [`Zip::read_plist`].
    pub fn read_entry(&mut self, name: &str) -> Result<Vec<u8>, ZipReadError> {
        self.read_entry_within(name, MAX_ENTRY_BYTES)
    }

    /// [`Zip::read_entry`] capped at [`MAX_PLIST_BYTES`].
    pub fn read_plist(&mut self, name: &str) -> Result<Vec<u8>, ZipReadError> {
        self.read_entry_within(name, MAX_PLIST_BYTES)
    }

    /// Reads the entry `name` finds, of at most `limit` bytes and never more
    /// than [`MAX_ENTRY_BYTES`]. An entry whose declared size exceeds the
    /// smaller of the two fails with [`ZipReadError::OverLimit`] before its
    /// buffer is allocated. An entry that reads fewer bytes than it declares
    /// fails, so an `Ok` buffer holds exactly its declared size.
    pub fn read_entry_within(&mut self, name: &str, limit: usize) -> Result<Vec<u8>, ZipReadError> {
        let index = self.index_for_name(name);
        read_capped(&mut self.archive, index, name, limit.min(MAX_ENTRY_BYTES))
    }

    /// The members [`members_in_zip_order`] lists. Each finds its entries
    /// through [`Zip::read_entry`] and [`Zip::read_plist`].
    pub fn members_in_zip_order(&self) -> Vec<String> {
        members_of(&self.archive)
    }
}

/// The earliest byte zip 9, opening at archive offset 0, could read as a
/// central header or a zip64 end record when it tries the end of central
/// directory record at `end`, or `None` when zip holds neither for that record.
/// zip tries records from the end of the file backwards and stops at the first
/// whose directory reads, so a record counts here even when one after it would
/// open.
fn directory_start(bytes: &[u8], end: usize) -> Option<u64> {
    fn le(bytes: &[u8], at: usize, len: usize) -> Option<u64> {
        let field = bytes.get(at..at.checked_add(len)?)?;
        Some(field.iter().rev().fold(0, |n, &b| (n << 8) | u64::from(b)))
    }
    if !bytes[end..].starts_with(b"PK\x05\x06") {
        return None;
    }
    let files = le(bytes, end + 10, 2)?;
    let size = le(bytes, end + 12, 4)?;
    let offset = le(bytes, end + 16, 4)?;
    let comment = le(bytes, end + 20, 2)?;
    if end as u64 + 22 + comment > bytes.len() as u64 {
        return None;
    }
    let zip64 = files == 0xffff || size == 0xffff_ffff || offset == 0xffff_ffff;
    let locator = end
        .checked_sub(20)
        .filter(|&at| zip64 && bytes[at..].starts_with(b"PK\x06\x07"));
    if let Some(locator) = locator {
        let zip64_end = le(bytes, locator + 8, 8)?;
        let disks = le(bytes, locator + 16, 4)?;
        let room = (locator as u64).checked_sub(zip64_end)?;
        if room < 4 || disks > 1 || !bytes[zip64_end as usize..].starts_with(b"PK\x06\x06") {
            return None;
        }
        let directory = le(bytes, zip64_end as usize + 48, 8).unwrap_or(u64::MAX);
        return Some(zip64_end.min(directory));
    }
    // With no files, zip starts the directory at the end record itself, so any
    // central header it reads there fails its signature check.
    let directory = files > 0 && offset + 4 <= end as u64;
    (directory && bytes[offset as usize..].starts_with(b"PK\x01\x02")).then_some(offset)
}

/// zip 9 reads central headers back to back from the directory's start and,
/// of the headers with one raw name, keeps the data of the last. So the last
/// header it read is kept, and every header before it that it did not keep
/// was dropped.
fn conflicting_names(
    archive: &zip::ZipArchive<Cursor<&[u8]>>,
    bytes: &[u8],
) -> zip::result::ZipResult<Vec<String>> {
    let mut kept = HashSet::new();
    let mut headers = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index_data(i)?;
        kept.insert(entry.central_header_start());
        headers.extend(header_fields(&entry));
    }
    if let Some(&last) = kept.iter().max() {
        let mut at = archive.central_directory_start();
        while at <= last {
            let header = central_header(bytes, at).ok_or_else(|| {
                zip::result::ZipError::InvalidArchive("central header out of range".into())
            })?;
            if !kept.contains(&at) {
                headers.extend(dropped_header_fields(header)?);
            }
            at += header.len() as u64;
        }
    }
    let mut first = HashMap::new();
    let mut conflicting = BTreeSet::new();
    for (name, crc32, size) in headers {
        if *first.entry(name.clone()).or_insert((crc32, size)) != (crc32, size) {
            conflicting.insert(name);
        }
    }
    Ok(conflicting.into_iter().collect())
}

/// The central header at `at`, its 46 fixed bytes and the name, extra field
/// and comment whose lengths they declare.
fn central_header(bytes: &[u8], at: u64) -> Option<&[u8]> {
    let at = usize::try_from(at).ok()?;
    let fixed: &[u8; 46] = bytes.get(at..at.checked_add(46)?)?.try_into().ok()?;
    let len = |field: usize| usize::from(u16::from_le_bytes([fixed[field], fixed[field + 1]]));
    bytes.get(at..at.checked_add(46 + len(28) + len(30) + len(32))?)
}

/// zip parses a header it dropped on its own, so the zip64 sizes and the
/// Unicode path field apply as they did when it read the archive.
fn dropped_header_fields(header: &[u8]) -> zip::result::ZipResult<Option<(String, u32, u64)>> {
    let mut one = header.to_vec();
    // An end of central directory record for one header at offset 0.
    one.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
    one.extend_from_slice(&(header.len() as u32).to_le_bytes());
    one.extend_from_slice(&[0; 6]);
    let config = zip::read::Config {
        archive_offset: zip::read::ArchiveOffset::Known(0),
    };
    let archive = zip::ZipArchive::with_config(config, Cursor::new(one))?;
    let entry = archive.by_index_data(0)?;
    Ok(header_fields(&entry))
}

fn header_fields(entry: &zip::read::ZipFileEntry) -> Option<(String, u32, u64)> {
    Some((entry.name().ok()?.into_owned(), entry.crc32(), entry.size()))
}

/// The declared uncompressed size is an attacker-controlled zip-header field,
/// and a small declared size can hide a huge inflate. So the read stops at the
/// declared size, which fits the buffer allocated for it. One more read
/// reaches the end of the entry, where zip checks the CRC and rejects bytes
/// past the declared size before this function sees them. zip accepts data
/// that ends before the declared size when the CRC matches it, so a short
/// entry is rejected here. Otherwise it would hold the declared capacity for
/// fewer bytes.
fn read_capped(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    index: Option<usize>,
    path: &str,
    limit: usize,
) -> Result<Vec<u8>, ZipReadError> {
    let index = index.ok_or_else(|| ZipReadError::Failed(format!("{path} not found")))?;
    let failed =
        |e: &dyn std::fmt::Display| ZipReadError::Failed(format!("failed to read {path}: {e}"));
    let size = zip.by_index_data(index).map_err(|e| failed(&e))?.size();
    if size > limit as u64 {
        return Err(ZipReadError::OverLimit {
            name: path.to_string(),
            declared: size,
            limit,
        });
    }
    let mut file = zip.by_index(index).map_err(|e| failed(&e))?;
    let mut buf = Vec::with_capacity(size as usize);
    (&mut file)
        .take(size)
        .read_to_end(&mut buf)
        .map_err(|e| failed(&e))?;
    if (buf.len() as u64) < size {
        return Err(ZipReadError::Failed(format!(
            "{path}: entry is shorter than its declared size {size}"
        )));
    }
    if file.read(&mut [0]).map_err(|e| failed(&e))? > 0 {
        return Err(ZipReadError::Failed(format!(
            "{path}: entry exceeds its declared size"
        )));
    }
    Ok(buf)
}

/// The streaming pre-check builds no tree, so it bails early: without it a
/// deeply nested plist produces a `Value` whose recursive `Drop` overflows the
/// call stack, and a small binary plist whose objects reference one another
/// many times expands into a tree far larger than the file. The stream yields
/// one event per expanded value, so counting values and payload bytes bounds
/// the tree before it is built. A table scan runs first, then the bytes are
/// parsed twice, stream then tree.
pub fn parse_plist_guarded(bytes: &[u8], label: &str) -> Result<plist::Value, String> {
    use plist::stream::Event;
    if bytes.len() > MAX_PLIST_BYTES {
        return Err(format!("{label}: plist size {} exceeds limit", bytes.len()));
    }
    check_binary_tables(bytes, label)?;
    let mut depth: usize = 0;
    let mut values: usize = 0;
    let mut payload: usize = 0;
    for event in plist::stream::Reader::new(Cursor::new(bytes)) {
        match event.map_err(|e| format!("failed to parse {label}: {e}"))? {
            Event::StartArray(_) | Event::StartDictionary(_) => {
                depth += 1;
                if depth > MAX_PLIST_DEPTH {
                    return Err(format!("{label}: plist nesting depth exceeds limit"));
                }
            }
            Event::EndCollection => {
                depth = depth.saturating_sub(1);
                continue;
            }
            Event::Data(data) => payload = payload.saturating_add(data.len()),
            Event::String(string) => payload = payload.saturating_add(string.len()),
            _ => {}
        }
        values += 1;
        if values > MAX_PLIST_VALUES {
            return Err(format!(
                "{label}: plist expands to over {MAX_PLIST_VALUES} values"
            ));
        }
        if payload > MAX_PLIST_PAYLOAD_BYTES {
            return Err(format!(
                "{label}: plist expands to over {MAX_PLIST_PAYLOAD_BYTES} bytes of strings and data"
            ));
        }
    }
    plist::Value::from_reader(Cursor::new(bytes))
        .map_err(|e| format!("failed to parse {label}: {e}"))
}

/// plist 1.8's binary reader allocates the offset table, each collection's
/// references and each decoded string before the event the stream guard
/// counts, and a UTF-16 string holds its code units and the growing UTF-8
/// string at once. When every object is reachable, the object count and the
/// declared references are each at most the expanded value count that guard
/// caps, and the declared string and data bytes are at most the expanded
/// payload, so a plist it accepts passes here. A bad magic or offset width,
/// or an offset table that does not fit, is left to plist.
fn check_binary_tables(bytes: &[u8], label: &str) -> Result<(), String> {
    let Some(tail) = bytes
        .strip_prefix(b"bplist00")
        .and_then(|body| body.last_chunk::<32>())
    else {
        return Ok(());
    };
    let trailer = bytes.len() - tail.len();
    let width = usize::from(tail[6]);
    let count = big_endian(&tail[8..16]);
    let table = big_endian(&tail[24..32]);
    if !matches!(width, 1 | 2 | 3 | 4 | 8) {
        return Ok(());
    }
    let Some(entries) = count
        .checked_mul(width as u64)
        .and_then(|n| n.checked_add(table))
        .filter(|&end| end <= trailer as u64)
        .and_then(|end| bytes.get(table as usize..end as usize))
    else {
        return Ok(());
    };
    if count > MAX_PLIST_VALUES as u64 {
        return Err(format!(
            "{label}: plist declares over {MAX_PLIST_VALUES} objects"
        ));
    }
    let mut references: u64 = 0;
    let mut payload: u64 = 0;
    for entry in entries.chunks_exact(width) {
        let offset = big_endian(entry);
        if offset >= trailer as u64 {
            continue;
        }
        let (object_references, object_payload) = declared(bytes, trailer, offset as usize);
        references = references.saturating_add(object_references);
        payload = payload.saturating_add(object_payload);
        if references > MAX_PLIST_VALUES as u64 {
            return Err(format!(
                "{label}: plist collections declare over {MAX_PLIST_VALUES} references"
            ));
        }
        if payload > MAX_PLIST_PAYLOAD_BYTES as u64 {
            return Err(format!(
                "{label}: plist declares over {MAX_PLIST_PAYLOAD_BYTES} bytes of strings and data"
            ));
        }
    }
    Ok(())
}

/// The references and the string or data bytes plist's reader allocates
/// for the object at `at` before it yields the object's event.
fn declared(bytes: &[u8], trailer: usize, at: usize) -> (u64, u64) {
    let Some(&token) = bytes.get(at) else {
        return (0, 0);
    };
    let (len, start) = if token & 0xF == 0xF {
        let Some(&marker) = bytes.get(at.saturating_add(1)) else {
            return (0, 0);
        };
        let start = at.saturating_add(2);
        let end = start.saturating_add(1 << (marker & 3));
        let Some(field) = bytes.get(start..end) else {
            return (0, 0);
        };
        (big_endian(field), end)
    } else {
        (u64::from(token & 0xF), at + 1)
    };
    // plist rejects content that does not fit before the trailer without
    // allocating it, and measures the fit from the object's offset because
    // `PosReader::read` never advances `pos`.
    let content = |unit: u64| {
        let size = usize::try_from(len.checked_mul(unit)?).ok()?;
        at.checked_add(size).filter(|&end| end <= trailer)?;
        bytes.get(start..start.checked_add(size)?)
    };
    match token >> 4 {
        0x4 | 0x5 => (0, content(1).map_or(0, |data| data.len() as u64)),
        0x6 => (0, content(2).map_or(0, utf8_len)),
        0xA => (len, 0),
        0xD => (len.saturating_mul(2), 0),
        _ => (0, 0),
    }
}

/// The UTF-8 length of big-endian UTF-16 `units`, exact for a valid string.
/// plist allocates for every unit before it finds an unpaired surrogate, so a
/// lone surrogate counts as half a pair.
fn utf8_len(units: &[u8]) -> u64 {
    units
        .as_chunks()
        .0
        .iter()
        .map(|&unit| match u16::from_be_bytes(unit) {
            0..=0x7F => 1,
            0x80..=0x7FF | 0xD800..=0xDFFF => 2,
            _ => 3,
        })
        .sum()
}

/// `field`, at most eight bytes, as a big-endian unsigned integer.
fn big_endian(field: &[u8]) -> u64 {
    field.iter().fold(0, |n, &b| n << 8 | u64::from(b))
}

/// Why a `Shape.png` did not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapePngError {
    /// A side over [`MAX_PNG_DIMENSION`], or a decode over [`MAX_ENTRY_BYTES`],
    /// counted as [`TipImageError::TooLarge`](crate::TipImageError::TooLarge)
    /// describes.
    TooLarge { width: u32, height: u32 },
    /// The bytes did not decode, or a PNG carries an eXIf chunk over 64 KiB
    /// before the image data.
    Corrupt(String),
}

impl std::fmt::Display for ShapePngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapePngError::TooLarge { width, height } => write!(
                f,
                "Shape.png is {width}x{height}px; a brush tip must be at most {MAX_PNG_DIMENSION}px per side and {} MiB to decode",
                MAX_ENTRY_BYTES / (1024 * 1024)
            ),
            ShapePngError::Corrupt(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ShapePngError {}

/// Decode a Procreate `Shape.png` into a grayscale tip. White is stamp
/// coverage, so the luminance is taken as-is. Oversize, by either dimension
/// or decoded size, is decided from the header before any pixels are decoded.
pub fn decode_tip_png(bytes: &[u8]) -> Result<GrayscaleBitmap, ShapePngError> {
    let image =
        decode_guarded(bytes, MAX_PNG_DIMENSION, MAX_ENTRY_BYTES as u64).map_err(|e| match e {
            GuardedDecodeError::TooLarge { width, height } => {
                ShapePngError::TooLarge { width, height }
            }
            GuardedDecodeError::Decode(e) => {
                ShapePngError::Corrupt(format!("failed to decode Shape.png: {e}"))
            }
        })?;
    Ok(GrayscaleBitmap {
        width: image.width,
        height: image.height,
        data: tip_plane(image, TipSample::Luma),
    })
}

/// The set name and the member uuids a `brushset.plist` declares, in its order.
pub fn parse_brushset_plist(bytes: &[u8]) -> Result<(Option<String>, Vec<String>), String> {
    let value = parse_plist_guarded(bytes, "brushset.plist")?;
    let dict = value
        .as_dictionary()
        .ok_or("brushset.plist root is not a dictionary")?;
    let name = dict
        .get("name")
        .and_then(|v| v.as_string())
        .map(str::to_string);
    let array = dict
        .get("brushes")
        .and_then(|v| v.as_array())
        .ok_or("brushset.plist missing brushes array")?;
    let uuids = array
        .iter()
        .enumerate()
        .map(|(i, v)| {
            v.as_string()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("brushset.plist brushes[{i}] is not a string"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((name, uuids))
}

/// The NSKeyedArchiver settings dictionary lives at `$objects[1]`, and every
/// non-scalar field of it is a UID into the same `$objects` array — so a
/// caller needs the pair, not just the dictionary.
pub fn archive_objects_and_main(
    value: &plist::Value,
) -> Result<(&[plist::Value], &plist::Dictionary), String> {
    let root = value
        .as_dictionary()
        .ok_or("Brush.archive root is not a dictionary")?;
    let objects = root
        .get("$objects")
        .and_then(|v| v.as_array())
        .ok_or("Brush.archive missing $objects array")?;
    let main_dict = objects
        .get(1)
        .and_then(|v| v.as_dictionary())
        .ok_or("$objects[1] is not a dictionary")?;
    Ok((objects, main_dict))
}

/// Follow a UID-valued field of the settings dictionary to the string it names,
/// treating NSKeyedArchiver's literal `"$null"` marker as absent.
pub fn resolve_string(
    objects: &[plist::Value],
    main_dict: &plist::Dictionary,
    key: &str,
) -> Option<String> {
    main_dict
        .get(key)
        .and_then(|v| v.as_uid())
        .and_then(|u| objects.get(u.get() as usize))
        .and_then(|v| v.as_string())
        .filter(|s| *s != "$null")
        .map(str::to_string)
}

/// The display name stored in a `Brush.archive` (`$objects[1].name`), `None`
/// when the archive has no readable name.
pub fn brush_name(archive_bytes: &[u8]) -> Result<Option<String>, String> {
    let value = parse_plist_guarded(archive_bytes, "Brush.archive")?;
    let (objects, main_dict) = archive_objects_and_main(&value)?;
    Ok(resolve_string(objects, main_dict, "name"))
}

/// Members of a set that has no `brushset.plist`: every top-level directory `d`
/// with an entry named exactly `d/Brush.archive`, in first-appearance zip order.
/// `d/Reset/Brush.archive` does not make `d/Reset` a member.
/// A name that is not UTF-8 comes back decoded as CP437, so [`read_zip_entry`]
/// and [`read_zip_plist`] do not find that member's entries. [`Zip`] finds
/// them.
pub fn members_in_zip_order(zip: &mut zip::ZipArchive<Cursor<&[u8]>>) -> Vec<String> {
    members_of(zip)
}

fn members_of(zip: &zip::ZipArchive<Cursor<&[u8]>>) -> Vec<String> {
    // By index, not `file_names()`: only the index walk is guaranteed to follow
    // the central directory, and the member order is part of the contract.
    (0..zip.len())
        .filter_map(|i| {
            let name = zip.name_for_index(i)?.ok()?;
            let dir = name.strip_suffix("/Brush.archive")?;
            (!dir.is_empty() && !dir.contains('/')).then(|| dir.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `body` after the magic, then a trailer with one-byte references.
    fn bplist(body: &[u8], offset_width: u8, count: u64, table: u64) -> Vec<u8> {
        let mut out = b"bplist00".to_vec();
        out.extend_from_slice(body);
        out.extend_from_slice(&[0, 0, 0, 0, 0, 0, offset_width, 1]);
        out.extend_from_slice(&count.to_be_bytes());
        out.extend_from_slice(&0u64.to_be_bytes());
        out.extend_from_slice(&table.to_be_bytes());
        out
    }

    #[test]
    fn tables_that_overflow_or_do_not_fit_are_left_to_plist() {
        let over = MAX_PLIST_VALUES as u64 + 1;
        for (width, count, table) in [
            (8, u64::MAX, 8),
            (1, 1, u64::MAX),
            (1, over, 8),
            (5, over, 8),
        ] {
            let plist = bplist(&[0x10, 0, 8], width, count, table);
            let case = format!("{width} {count} {table}");
            assert_eq!(check_binary_tables(&plist, "t"), Ok(()), "{case}");
            let err = parse_plist_guarded(&plist, "t").expect_err(&case);
            assert!(err.starts_with("failed to parse t:"), "{case}: {err}");
        }
    }

    #[test]
    fn extended_lengths_are_read_at_the_marked_width() {
        let over = (MAX_PLIST_VALUES as u64 + 1).to_be_bytes();
        let rejected = Err(format!(
            "t: plist collections declare over {MAX_PLIST_VALUES} references"
        ));
        for (marker, expected) in [(0x10, Ok(())), (0x13, rejected.clone()), (0xF3, rejected)] {
            let mut body = vec![0xAF, marker];
            body.extend_from_slice(&over);
            body.push(8);
            assert_eq!(
                check_binary_tables(&bplist(&body, 1, 1, 18), "t"),
                expected,
                "{marker:#x}"
            );
        }
    }

    #[test]
    fn utf16_strings_are_counted_as_utf8() {
        let plist = |ascii: usize| {
            let string = "\u{5b57}".repeat(MAX_PLIST_PAYLOAD_BYTES / 3) + &"a".repeat(ascii);
            let mut out = Vec::new();
            plist::Value::String(string)
                .to_writer_binary(&mut out)
                .expect("binary plist");
            out
        };
        let fits = MAX_PLIST_PAYLOAD_BYTES % 3;
        assert!(parse_plist_guarded(&plist(fits), "t").is_ok());
        assert_eq!(
            parse_plist_guarded(&plist(fits + 1), "t"),
            declared_payload()
        );
    }

    fn declared_payload() -> Result<plist::Value, String> {
        Err(format!(
            "t: plist declares over {MAX_PLIST_PAYLOAD_BYTES} bytes of strings and data"
        ))
    }

    /// One UTF-16 string whose content runs `into_trailer` bytes into the
    /// trailer, with the offset table's one entry as its last byte before it.
    fn utf16_string(units: &[u16], into_trailer: usize) -> Vec<u8> {
        let mut body = vec![0x6F, 0x12];
        body.extend_from_slice(&(units.len() as u32).to_be_bytes());
        body.extend(units.iter().flat_map(|unit| unit.to_be_bytes()));
        body.truncate(body.len() - into_trailer);
        *body.last_mut().expect("content") = 8;
        let table = 8 + body.len() as u64 - 1;
        bplist(&body, 1, 1, table)
    }

    #[test]
    fn utf16_strings_that_plist_fails_or_reads_into_the_trailer_are_counted() {
        let cjk = vec![0x5B57; MAX_PLIST_PAYLOAD_BYTES / 3 + 3];
        let lone_surrogate = [&[0xDC00][..], &cjk[3..]].concat();
        for plist in [utf16_string(&lone_surrogate, 0), utf16_string(&cjk, 6)] {
            assert_eq!(parse_plist_guarded(&plist, "t"), declared_payload());
        }
    }
}
