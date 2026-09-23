//! Reading Procreate `.brush` and `.brushset` archives: the guarded zip and
//! plist readers, the `Shape.png` decoder and the two name/member lookups the
//! preview API needs.
//!
//! Every entry here parses untrusted bytes, so each size, count and dimension
//! is checked against a ceiling before anything is allocated.

use crate::GrayscaleBitmap;
use std::io::{Cursor, Read};

/// Defensive ceilings for untrusted archives: anything above them is treated
/// as malformed rather than allocated from.
pub const MAX_ENTRY_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_PNG_DIMENSION: u32 = 16384;
pub const MAX_PLIST_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PLIST_DEPTH: usize = 64;

/// The declared uncompressed size is an attacker-controlled zip-header field,
/// so it is only a clamped pre-allocation hint and the read is capped
/// independently — a small declared size can hide a huge inflate.
pub fn read_zip_entry(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<Vec<u8>, String> {
    let file = zip.by_name(path).map_err(|_| format!("{path} not found"))?;
    if file.size() > MAX_ENTRY_BYTES as u64 {
        return Err(format!(
            "{path}: declared size {} exceeds limit",
            file.size()
        ));
    }
    let mut buf = Vec::with_capacity((file.size() as usize).min(MAX_ENTRY_BYTES));
    file.take(MAX_ENTRY_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("failed to read {path}: {e}"))?;
    if buf.len() > MAX_ENTRY_BYTES {
        return Err(format!("{path}: entry exceeds size limit"));
    }
    Ok(buf)
}

/// The streaming depth pre-check builds no tree, so it bails early: without it
/// a deeply nested plist produces a `Value` whose recursive `Drop` overflows
/// the call stack. The bytes are parsed twice, stream then tree.
pub fn parse_plist_guarded(bytes: &[u8], label: &str) -> Result<plist::Value, String> {
    if bytes.len() > MAX_PLIST_BYTES {
        return Err(format!("{label}: plist size {} exceeds limit", bytes.len()));
    }
    let mut depth: usize = 0;
    for event in plist::stream::Reader::new(Cursor::new(bytes)) {
        match event.map_err(|e| format!("failed to parse {label}: {e}"))? {
            plist::stream::Event::StartArray(_) | plist::stream::Event::StartDictionary(_) => {
                depth += 1;
                if depth > MAX_PLIST_DEPTH {
                    return Err(format!("{label}: plist nesting depth exceeds limit"));
                }
            }
            plist::stream::Event::EndCollection => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    plist::Value::from_reader(Cursor::new(bytes))
        .map_err(|e| format!("failed to parse {label}: {e}"))
}

/// Why a `Shape.png` did not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapePngError {
    TooLarge { width: u32, height: u32 },
    Corrupt(String),
}

impl std::fmt::Display for ShapePngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapePngError::TooLarge { width, height } => write!(
                f,
                "Shape.png is {width}x{height}px; the maximum supported brush-tip dimension is {MAX_PNG_DIMENSION}px"
            ),
            ShapePngError::Corrupt(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ShapePngError {}

/// Decode a Procreate `Shape.png` into a grayscale tip. White is stamp
/// coverage, so the luminance is taken as-is. Oversize is decided from the
/// header, before any decoder is built.
pub fn decode_tip_png(bytes: &[u8]) -> Result<GrayscaleBitmap, ShapePngError> {
    if let Some((width, height)) = header_dimensions(bytes) {
        if width > MAX_PNG_DIMENSION || height > MAX_PNG_DIMENSION {
            return Err(ShapePngError::TooLarge { width, height });
        }
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ShapePngError::Corrupt(format!("failed to sniff Shape.png: {e}")))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PNG_DIMENSION);
    limits.max_image_height = Some(MAX_PNG_DIMENSION);
    limits.max_alloc = Some(MAX_ENTRY_BYTES as u64);
    reader.limits(limits);
    let luma = reader
        .decode()
        .map_err(|e| ShapePngError::Corrupt(format!("failed to decode Shape.png: {e}")))?
        .to_luma8();
    Ok(GrayscaleBitmap {
        width: luma.width(),
        height: luma.height(),
        data: luma.into_raw(),
    })
}

/// Reads header dimensions without allocating pixels or applying decode limits.
/// PNG goes through the `png` header alone: `image` sizes the output buffer
/// before it reports dimensions, which fails on 32-bit targets for large ones.
pub(crate) fn header_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if image::guess_format(bytes).ok()? == image::ImageFormat::Png {
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        let info = decoder.read_header_info().ok()?;
        return Some((info.width, info.height));
    }
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
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
