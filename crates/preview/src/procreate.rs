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
    /// A side over `MAX_PNG_DIMENSION`, or a decode over `MAX_ENTRY_BYTES`:
    /// the image in its own pixel format, plus the DCT coefficients of a
    /// progressive JPEG or of a baseline JPEG whose first scan leaves out a
    /// component. The decoder's row buffers, which grow with the width only,
    /// and a PNG's eXIf chunk of at most 64 KiB are not counted.
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
