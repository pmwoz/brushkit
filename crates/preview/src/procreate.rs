//! Reading Procreate `.brush` and `.brushset` archives: the guarded zip and
//! plist readers, the `Shape.png` decoder and the two name/member lookups the
//! preview API needs.
//!
//! Every entry here parses untrusted bytes, so each size, count and dimension
//! is checked against a ceiling before anything is allocated.

use crate::bitmap::{decode_guarded, tip_plane, GuardedDecodeError, TipSample};
use crate::GrayscaleBitmap;
use std::io::{Cursor, Read};

/// Defensive ceilings for untrusted archives: anything above them is treated
/// as malformed rather than allocated from.
pub const MAX_ENTRY_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_PNG_DIMENSION: u32 = 16384;
pub const MAX_PLIST_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PLIST_DEPTH: usize = 64;
/// A binary plist object may be referenced any number of times, and each
/// reference is expanded into its own value, so the tree can be far larger
/// than the file. Every value costs a `plist::Value` slot of about 80 bytes,
/// so this keeps the largest accepted tree near 8 MiB. A census of 5803 real
/// `Brush.archive` and `brushset.plist` files found at most 949 values. It
/// also caps the objects and collection references a binary plist declares.
pub const MAX_PLIST_VALUES: usize = 100_000;
/// The expanded string and data bytes of one plist. Equal to
/// [`MAX_PLIST_BYTES`], so a plist that shares nothing can never reach it.
/// The same census found at most 7 KiB.
pub const MAX_PLIST_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

/// Reads a zip entry of at most [`MAX_ENTRY_BYTES`].
pub fn read_zip_entry(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<Vec<u8>, String> {
    read_capped(zip, path, MAX_ENTRY_BYTES)
}

/// [`read_zip_entry`] capped at [`MAX_PLIST_BYTES`], so an oversize plist
/// fails before more than the plist ceiling is read.
pub fn read_zip_plist(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<Vec<u8>, String> {
    read_capped(zip, path, MAX_PLIST_BYTES)
}

/// The declared uncompressed size is an attacker-controlled zip-header field,
/// and a small declared size can hide a huge inflate. So the read stops at the
/// declared size, which fits the buffer allocated for it, and one byte more
/// rejects the entry. That extra read also reaches the end of the entry, where
/// the zip reader checks the CRC.
fn read_capped(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut file = zip.by_name(path).map_err(|_| format!("{path} not found"))?;
    let size = file.size();
    if size > limit as u64 {
        return Err(format!("{path}: declared size {size} exceeds limit"));
    }
    let read_error = |e: std::io::Error| format!("failed to read {path}: {e}");
    let mut buf = Vec::with_capacity(size as usize);
    (&mut file)
        .take(size)
        .read_to_end(&mut buf)
        .map_err(read_error)?;
    if file.read(&mut [0]).map_err(read_error)? > 0 {
        return Err(format!("{path}: entry exceeds its declared size"));
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
pub fn members_in_zip_order(zip: &mut zip::ZipArchive<Cursor<&[u8]>>) -> Vec<String> {
    // By index, not `file_names()`: only the index walk is guaranteed to follow
    // the central directory, and the member order is part of the contract.
    (0..zip.len())
        .filter_map(|i| {
            let dir = zip.name_for_index(i)?.strip_suffix("/Brush.archive")?;
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
