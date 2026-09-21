//! Tip bitmaps for every brush file this workspace reads.
//!
//! `preview_abr`, `preview_brush` and `preview_brushset` each take the whole
//! file as bytes and return one [`PreviewEntry`] per brush in file order,
//! available or not. A brush whose tip cannot be rendered is reported with a
//! reason rather than dropped, so a caller can lay out a complete grid.
//!
//! Every function here is pure over `&[u8]`: no filesystem, no threads, so the
//! crate builds for `wasm32-unknown-unknown`.

pub mod procreate;

mod bitmap;
#[cfg(feature = "text")]
mod sheet;
mod synth;

pub use bitmap::*;
#[cfg(feature = "text")]
pub use sheet::*;
pub use synth::*;

use brushkit_abr::{parse_abr_deferred_without_patterns, ShapeTipFamily};
use std::io::Cursor;

#[derive(Debug, Clone, Copy)]
pub struct PreviewOptions {
    /// Larger side of every returned tip is at most this many pixels (>= 1).
    pub max_cell: u32,
}

#[derive(Debug, Clone)]
pub struct PreviewSet {
    pub set_name: Option<String>,
    pub entries: Vec<PreviewEntry>,
}

#[derive(Debug, Clone)]
pub struct PreviewEntry {
    pub index: usize,
    pub name: String,
    pub tip: TipPreview,
}

#[derive(Debug, Clone)]
pub enum TipPreview {
    Available(GrayscaleBitmap),
    Unavailable(UnavailableReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    NoShapePng,
    UnsupportedTipKind(String),
    Corrupt(String),
    TooLarge { width: u32, height: u32 },
}

#[derive(Debug)]
pub struct PreviewError(pub String);

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PreviewError {}

fn check_max_cell(opts: PreviewOptions) -> Result<u32, PreviewError> {
    if opts.max_cell == 0 {
        return Err(PreviewError("max_cell must be at least 1".to_string()));
    }
    Ok(opts.max_cell)
}

/// Where an `.abr` preview row came from. Orders rows that share a preset
/// ordinal: sampled first, then computed, then unsupported.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Sampled,
    Computed,
    Unsupported,
}

struct Row {
    /// The preset ordinal from the descriptor, `usize::MAX` when unknown.
    key: usize,
    source: Source,
    name: String,
    tip: TipPreview,
}

/// Tips for a Photoshop `.abr` pack.
///
/// Sampled tips are decoded one at a time and downsampled immediately, so the
/// peak footprint holds one full-size tip rather than the whole pack. Computed
/// presets are synthesized from their geometry; a preset that declares neither
/// is reported as an unsupported tip kind.
///
/// Embedded pattern payloads are neither copied nor decoded.
pub fn preview_abr(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError> {
    let max_cell = check_max_cell(opts)?;

    let deferred =
        parse_abr_deferred_without_patterns(bytes).map_err(|e| PreviewError(e.to_string()))?;

    let mut rows: Vec<Row> = Vec::new();

    for i in 0..deferred.pack.brushes.len() {
        let brush = &deferred.pack.brushes[i];
        let key = brush.preset_index.unwrap_or(usize::MAX);
        let name = if brush.name.is_empty() {
            brush.id.clone()
        } else {
            brush.name.clone()
        };
        let tip = match deferred.decode_tip(i) {
            Ok(tip) => TipPreview::Available(downsample(&to_grayscale(&tip), max_cell)),
            Err(e) => TipPreview::Unavailable(UnavailableReason::Corrupt(e.to_string())),
        };
        rows.push(Row {
            key,
            source: Source::Sampled,
            name,
            tip,
        });
    }

    for preset in &deferred.pack.computed_presets {
        let key = preset.preset_index.unwrap_or(usize::MAX);
        let tip = match preset
            .descriptor
            .computed
            .as_ref()
            .filter(|geom| can_synthesize(geom))
            .and_then(synthesize_computed_tip)
        {
            Some(bitmap) => TipPreview::Available(downsample(&bitmap, max_cell)),
            None => TipPreview::Unavailable(UnavailableReason::UnsupportedTipKind(
                "computed".to_string(),
            )),
        };
        rows.push(Row {
            key,
            source: Source::Computed,
            name: preset.name.clone(),
            tip,
        });
    }

    for preset in &deferred.pack.unsupported_tip_presets {
        let kind = match (&preset.tip_shape, &preset.shape_tip_family) {
            (Some(shape), _) => format!("{shape:?}"),
            (None, Some(ShapeTipFamily::Bristle)) => "bristle".to_string(),
            (None, Some(ShapeTipFamily::Erodible)) => "erodible".to_string(),
            (None, None) => "shape tip".to_string(),
        };
        rows.push(Row {
            key: preset.preset_index,
            source: Source::Unsupported,
            name: preset.name.clone(),
            tip: TipPreview::Unavailable(UnavailableReason::UnsupportedTipKind(kind)),
        });
    }

    rows.sort_by_key(|row| (row.key, row.source));

    let entries = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| PreviewEntry {
            index,
            name: row.name,
            tip: row.tip,
        })
        .collect();

    Ok(PreviewSet {
        set_name: None,
        entries,
    })
}

/// Tips for a Procreate `.brushset`.
///
/// With a `brushset.plist` the set name and the member order come from it;
/// without one the members are the top-level directories that hold a
/// `Brush.archive`, in zip order, and the set has no name.
pub fn preview_brushset(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError> {
    let max_cell = check_max_cell(opts)?;
    let mut zip = open_zip(bytes)?;

    let (set_name, prefixes) = if zip.by_name("brushset.plist").is_ok() {
        let buf = procreate::read_zip_entry(&mut zip, "brushset.plist").map_err(PreviewError)?;
        let (name, uuids) = procreate::parse_brushset_plist(&buf).map_err(PreviewError)?;
        (name, uuids.into_iter().map(|u| format!("{u}/")).collect())
    } else {
        let members: Vec<String> = procreate::members_in_zip_order(&mut zip)
            .into_iter()
            .map(|d| format!("{d}/"))
            .collect();
        if members.is_empty() {
            return Err(PreviewError("no brushes found".to_string()));
        }
        (None, members)
    };

    let entries = prefixes
        .iter()
        .enumerate()
        .map(|(index, prefix)| member_entry(&mut zip, index, prefix, max_cell))
        .collect();

    Ok(PreviewSet { set_name, entries })
}

/// The tip of a single Procreate `.brush`: one entry at index 0, read from the
/// archive's root rather than from a member directory.
pub fn preview_brush(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError> {
    let max_cell = check_max_cell(opts)?;
    let mut zip = open_zip(bytes)?;

    if zip.by_name("Brush.archive").is_err() {
        return Err(PreviewError("Brush.archive not found".to_string()));
    }

    Ok(PreviewSet {
        set_name: None,
        entries: vec![member_entry(&mut zip, 0, "", max_cell)],
    })
}

fn open_zip(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, PreviewError> {
    zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| PreviewError(format!("failed to open zip: {e}")))
}

/// One member of a Procreate archive. `prefix` is `"{uuid}/"` for a
/// `.brushset` member and `""` for a root-layout `.brush`.
///
/// A member is always an entry: an archive that cannot be read names the entry
/// after its directory and reports why, rather than shifting every index after
/// it.
fn member_entry(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    index: usize,
    prefix: &str,
    max_cell: u32,
) -> PreviewEntry {
    let fallback_name = if prefix.is_empty() {
        "Brush".to_string()
    } else {
        prefix.trim_end_matches('/').to_string()
    };

    let archive = procreate::read_zip_entry(zip, &format!("{prefix}Brush.archive"))
        .and_then(|buf| procreate::brush_name(&buf));
    let name = match archive {
        Ok(name) => name.unwrap_or(fallback_name),
        Err(msg) => {
            return PreviewEntry {
                index,
                name: fallback_name,
                tip: TipPreview::Unavailable(UnavailableReason::Corrupt(msg)),
            }
        }
    };

    let shape_path = format!("{prefix}Shape.png");
    if zip.by_name(&shape_path).is_err() {
        return PreviewEntry {
            index,
            name,
            tip: TipPreview::Unavailable(UnavailableReason::NoShapePng),
        };
    }

    let tip = match procreate::read_zip_entry(zip, &shape_path) {
        Err(msg) => TipPreview::Unavailable(UnavailableReason::Corrupt(msg)),
        Ok(png) => match procreate::decode_tip_png(&png) {
            Ok(bitmap) => TipPreview::Available(downsample(&bitmap, max_cell)),
            Err(procreate::ShapePngError::TooLarge { width, height }) => {
                TipPreview::Unavailable(UnavailableReason::TooLarge { width, height })
            }
            Err(procreate::ShapePngError::Corrupt(msg)) => {
                TipPreview::Unavailable(UnavailableReason::Corrupt(msg))
            }
        },
    };

    PreviewEntry { index, name, tip }
}
