//! Tip bitmaps for every brush file this workspace reads.
//!
//! `preview_abr`, `preview_brush` and `preview_brushset` each take the whole
//! file as bytes and return one [`PreviewEntry`] per brush in file order,
//! available or not. A brush whose tip cannot be rendered is reported with a
//! reason rather than dropped, so a caller can lay out a complete grid. Each
//! has a `_first_available` twin that returns only the first `n` available
//! entries and builds no entry after them.
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

use brushkit_abr::{parse_abr_all_deferred_without_patterns, DeferredPack, ShapeTipFamily};
use std::io::Cursor;
use std::num::NonZeroU32;

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

/// File-declared raster dimensions, independent of the returned preview size.
/// A readable header does not guarantee valid pixels or a safe allocation size.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SourceDimensions {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl SourceDimensions {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        Some(Self {
            width: NonZeroU32::new(width)?,
            height: NonZeroU32::new(height)?,
        })
    }

    pub fn width(self) -> u32 {
        self.width.get()
    }
    pub fn height(self) -> u32 {
        self.height.get()
    }
}

#[derive(Debug, Clone)]
pub struct PreviewEntry {
    pub index: usize,
    pub name: String,
    pub tip: TipPreview,
    /// None when the brush has no source raster or its dimensions cannot be read.
    pub source_dimensions: Option<SourceDimensions>,
}

#[derive(Debug, Clone)]
pub enum TipPreview {
    /// A bitmap with a nonzero width and height. A tip that decodes to zero
    /// area is `Unavailable` with [`UnavailableReason::Corrupt`].
    Available(GrayscaleBitmap),
    Unavailable(UnavailableReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    NoShapePng,
    UnsupportedTipKind(String),
    Corrupt(String),
    /// A side over [`procreate::MAX_PNG_DIMENSION`], or a decode over
    /// [`procreate::MAX_ENTRY_BYTES`], counted as [`TipImageError::TooLarge`]
    /// describes.
    TooLarge {
        width: u32,
        height: u32,
    },
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

/// Which entries a preview returns.
#[derive(Clone, Copy)]
enum Take {
    All,
    /// The first `n` entries whose tip is available. This stops pulling after
    /// the `n`th; it saves work only because callers pass a lazy iterator.
    FirstAvailable(usize),
}

impl Take {
    fn collect(self, entries: impl Iterator<Item = PreviewEntry>) -> Vec<PreviewEntry> {
        match self {
            Take::All => entries.collect(),
            Take::FirstAvailable(n) => entries
                .filter(|entry| matches!(entry.tip, TipPreview::Available(_)))
                .take(n)
                .collect(),
        }
    }
}

/// Where an `.abr` preview row came from: an index into `brushes`,
/// `computed_presets` or `unsupported_tip_presets` of the parsed pack. The
/// derived order, variant first and then index, orders rows that share a
/// preset ordinal: sampled first, then computed, then unsupported, each in
/// file order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Sampled(usize),
    Computed(usize),
    Unsupported(usize),
}

struct Row {
    /// The preset ordinal from the descriptor, `usize::MAX` when unknown.
    key: usize,
    source: Source,
}

/// Tips for a Photoshop `.abr` pack.
///
/// Sampled tips are decoded one at a time and downsampled immediately, so the
/// peak footprint holds one full-size tip rather than the whole pack. Computed
/// presets are synthesized from their geometry; a preset that declares neither
/// is reported as an unsupported tip kind. A sampled tip whose pixels fail to
/// decode is a `Corrupt` entry, not an error for the whole preview. The
/// exception is a raw v1 or v2 tip whose pixels run past the end of the input,
/// which fails the whole preview.
///
/// Embedded pattern payloads are neither copied nor decoded.
pub fn preview_abr(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError> {
    abr(bytes, opts, Take::All)
}

/// The first `n` entries of [`preview_abr`] whose tip is available, in the
/// same order and with the same `index` they have there, so indices may skip.
/// Unavailable entries do not count toward `n`, and entries after the `n`th
/// available one are not built, so their tips are not decoded or downsampled.
///
/// The whole pack is still parsed, but the parse decodes no tip.
pub fn preview_abr_first_available(
    bytes: &[u8],
    opts: PreviewOptions,
    n: usize,
) -> Result<PreviewSet, PreviewError> {
    abr(bytes, opts, Take::FirstAvailable(n))
}

fn abr(bytes: &[u8], opts: PreviewOptions, take: Take) -> Result<PreviewSet, PreviewError> {
    let max_cell = check_max_cell(opts)?;

    let deferred =
        parse_abr_all_deferred_without_patterns(bytes).map_err(|e| PreviewError(e.to_string()))?;
    let pack = &deferred.pack;

    let mut rows: Vec<Row> = Vec::new();
    rows.extend(pack.brushes.iter().enumerate().map(|(i, brush)| Row {
        key: brush.preset_index.unwrap_or(usize::MAX),
        source: Source::Sampled(i),
    }));
    rows.extend(
        pack.computed_presets
            .iter()
            .enumerate()
            .map(|(i, preset)| Row {
                key: preset.preset_index.unwrap_or(usize::MAX),
                source: Source::Computed(i),
            }),
    );
    rows.extend(
        pack.unsupported_tip_presets
            .iter()
            .enumerate()
            .map(|(i, preset)| Row {
                key: preset.preset_index,
                source: Source::Unsupported(i),
            }),
    );

    rows.sort_by_key(|row| (row.key, row.source));

    let entries = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| abr_entry(&deferred, index, row.source, max_cell));

    Ok(PreviewSet {
        set_name: None,
        entries: take.collect(entries),
    })
}

/// Render one `.abr` row. A sampled tip is decoded and downsampled here, so
/// only the rows a caller takes are decoded.
fn abr_entry(
    deferred: &DeferredPack<'_>,
    index: usize,
    source: Source,
    max_cell: u32,
) -> PreviewEntry {
    #[cfg(test)]
    tests::record_entry();
    let pack = &deferred.pack;
    let (name, tip, source_dimensions) = match source {
        Source::Sampled(i) => {
            let brush = &pack.brushes[i];
            let name = if brush.name.is_empty() {
                brush.id.clone()
            } else {
                brush.name.clone()
            };
            let tip = match deferred.decode_tip(i) {
                Ok(tip) => tip_preview(&to_grayscale(&tip), max_cell),
                Err(e) => TipPreview::Unavailable(UnavailableReason::Corrupt(e.to_string())),
            };
            (
                name,
                tip,
                SourceDimensions::new(brush.tip.width, brush.tip.height),
            )
        }
        Source::Computed(i) => {
            let preset = &pack.computed_presets[i];
            let tip = match preset
                .descriptor
                .computed
                .as_ref()
                .filter(|geom| can_synthesize(geom))
                .and_then(synthesize_computed_tip)
            {
                Some(bitmap) => tip_preview(&bitmap, max_cell),
                None => TipPreview::Unavailable(UnavailableReason::UnsupportedTipKind(
                    "computed".to_string(),
                )),
            };
            (preset.name.clone(), tip, None)
        }
        Source::Unsupported(i) => {
            let preset = &pack.unsupported_tip_presets[i];
            let kind = match (&preset.tip_shape, &preset.shape_tip_family) {
                (Some(shape), _) => format!("{shape:?}"),
                (None, Some(ShapeTipFamily::Bristle)) => "bristle".to_string(),
                (None, Some(ShapeTipFamily::Erodible)) => "erodible".to_string(),
                (None, None) => "shape tip".to_string(),
            };
            (
                preset.name.clone(),
                TipPreview::Unavailable(UnavailableReason::UnsupportedTipKind(kind)),
                None,
            )
        }
    };
    PreviewEntry {
        index,
        name,
        tip,
        source_dimensions,
    }
}

/// A full-size tip downsampled to `max_cell`. The area check runs first
/// because `downsample` widens a zero side to 1.
fn tip_preview(bitmap: &GrayscaleBitmap, max_cell: u32) -> TipPreview {
    if bitmap.width == 0 || bitmap.height == 0 {
        return TipPreview::Unavailable(UnavailableReason::Corrupt(
            "tip has zero area".to_string(),
        ));
    }
    TipPreview::Available(downsample(bitmap, max_cell))
}

/// Tips for a Procreate `.brushset`.
///
/// With a `brushset.plist` the set name and the member order come from it;
/// without one the members are the top-level directories that hold a
/// `Brush.archive`, in zip order, and the set has no name.
pub fn preview_brushset(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError> {
    brushset(bytes, opts, Take::All)
}

/// The first `n` entries of [`preview_brushset`] whose tip is available, in
/// the same order and with the same `index` they have there, so indices may
/// skip. Unavailable entries do not count toward `n`, and members after the
/// `n`th available one are not read.
pub fn preview_brushset_first_available(
    bytes: &[u8],
    opts: PreviewOptions,
    n: usize,
) -> Result<PreviewSet, PreviewError> {
    brushset(bytes, opts, Take::FirstAvailable(n))
}

fn brushset(bytes: &[u8], opts: PreviewOptions, take: Take) -> Result<PreviewSet, PreviewError> {
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
        .map(|(index, prefix)| member_entry(&mut zip, index, prefix, max_cell));

    Ok(PreviewSet {
        set_name,
        entries: take.collect(entries),
    })
}

/// The tip of a single Procreate `.brush`: one entry at index 0, read from the
/// archive's root rather than from a member directory.
pub fn preview_brush(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError> {
    brush(bytes, opts, Take::All)
}

/// [`preview_brush`] limited to available tips: its single entry when the tip
/// is available and `n >= 1`, otherwise no entries. With `n == 0` the tip is
/// not decoded.
pub fn preview_brush_first_available(
    bytes: &[u8],
    opts: PreviewOptions,
    n: usize,
) -> Result<PreviewSet, PreviewError> {
    brush(bytes, opts, Take::FirstAvailable(n))
}

fn brush(bytes: &[u8], opts: PreviewOptions, take: Take) -> Result<PreviewSet, PreviewError> {
    let max_cell = check_max_cell(opts)?;
    let mut zip = open_zip(bytes)?;

    if zip.by_name("Brush.archive").is_err() {
        return Err(PreviewError("Brush.archive not found".to_string()));
    }

    let entry = std::iter::once_with(|| member_entry(&mut zip, 0, "", max_cell));
    Ok(PreviewSet {
        set_name: None,
        entries: take.collect(entry),
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
/// it. A readable `Shape.png` reports its size either way.
fn member_entry(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    index: usize,
    prefix: &str,
    max_cell: u32,
) -> PreviewEntry {
    #[cfg(test)]
    tests::record_entry();
    let fallback_name = if prefix.is_empty() {
        "Brush".to_string()
    } else {
        prefix.trim_end_matches('/').to_string()
    };

    let archive = procreate::read_zip_entry(zip, &format!("{prefix}Brush.archive"))
        .and_then(|buf| procreate::brush_name(&buf));

    let shape_path = format!("{prefix}Shape.png");
    let shape = if zip.by_name(&shape_path).is_ok() {
        Some(procreate::read_zip_entry(zip, &shape_path))
    } else {
        None
    };
    let source_dimensions = match &shape {
        Some(Ok(png)) => bitmap::header_dimensions(png)
            .and_then(|(width, height)| SourceDimensions::new(width, height)),
        _ => None,
    };

    let (name, tip) = match archive {
        Ok(name) => (name.unwrap_or(fallback_name), shape_tip(shape, max_cell)),
        Err(msg) => (
            fallback_name,
            TipPreview::Unavailable(UnavailableReason::Corrupt(msg)),
        ),
    };

    PreviewEntry {
        index,
        name,
        tip,
        source_dimensions,
    }
}

/// The tip for a member's `Shape.png`: `None` when the member has no shape,
/// otherwise the read result.
fn shape_tip(shape: Option<Result<Vec<u8>, String>>, max_cell: u32) -> TipPreview {
    let png = match shape {
        None => return TipPreview::Unavailable(UnavailableReason::NoShapePng),
        Some(Err(msg)) => return TipPreview::Unavailable(UnavailableReason::Corrupt(msg)),
        Some(Ok(png)) => png,
    };
    match procreate::decode_tip_png(&png) {
        Ok(bitmap) => tip_preview(&bitmap, max_cell),
        Err(procreate::ShapePngError::TooLarge { width, height }) => {
            TipPreview::Unavailable(UnavailableReason::TooLarge { width, height })
        }
        Err(procreate::ShapePngError::Corrupt(msg)) => {
            TipPreview::Unavailable(UnavailableReason::Corrupt(msg))
        }
    }
}

// The integration tests' fixture builders, shared with the unit tests below.
#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod common;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{
        brush_archive, brushset_plist, gray_png, legacy_abr, samp_abr, zip_with, SampTip,
    };
    use std::cell::Cell;

    thread_local! {
        // Thread-local because cargo runs unit tests on parallel threads.
        static ENTRIES: Cell<usize> = const { Cell::new(0) };
    }

    /// Called once per entry built, before any of its tip is read or decoded.
    pub(super) fn record_entry() {
        ENTRIES.with(|count| count.set(count.get() + 1));
    }

    /// Entries built by `f` on this thread.
    fn entries_built<T>(f: impl FnOnce() -> T) -> usize {
        ENTRIES.with(|count| count.set(0));
        f();
        ENTRIES.with(Cell::get)
    }

    const OPTS: PreviewOptions = PreviewOptions { max_cell: 8 };

    fn tip(corrupt: bool) -> SampTip {
        SampTip {
            width: 4,
            height: 4,
            fill: 0x80,
            corrupt,
        }
    }

    #[test]
    fn abr_builds_only_the_entries_it_returns() {
        let bytes = samp_abr(&[
            tip(false),
            tip(false),
            tip(false),
            tip(false),
            tip(false),
            tip(false),
        ]);
        assert_eq!(
            entries_built(|| preview_abr_first_available(&bytes, OPTS, 2).unwrap()),
            2
        );
        assert_eq!(
            entries_built(|| preview_abr_first_available(&bytes, OPTS, 0).unwrap()),
            0
        );
        assert_eq!(entries_built(|| preview_abr(&bytes, OPTS).unwrap()), 6);
    }

    #[test]
    fn abr_failed_decode_does_not_count_toward_n() {
        let bytes = samp_abr(&[tip(false), tip(false), tip(true)]);
        assert!(matches!(
            preview_abr(&bytes, OPTS).unwrap().entries[0].tip,
            TipPreview::Unavailable(UnavailableReason::Corrupt(_))
        ));
        let mut set = None;
        assert_eq!(
            entries_built(|| set = Some(preview_abr_first_available(&bytes, OPTS, 1).unwrap())),
            2
        );
        let entries = set.unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].index, 1);
    }

    #[test]
    fn abr_v2_corrupt_tip_is_one_corrupt_entry() {
        let bytes = legacy_abr(&[tip(false), tip(true), tip(false)]);
        let entries = preview_abr(&bytes, OPTS).unwrap().entries;
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["brush_2", "brush_1", "brush_0"]);
        assert!(matches!(entries[0].tip, TipPreview::Available(_)));
        assert!(matches!(
            entries[1].tip,
            TipPreview::Unavailable(UnavailableReason::Corrupt(_))
        ));
        assert!(matches!(entries[2].tip, TipPreview::Available(_)));
    }

    #[test]
    fn abr_zero_area_tip_is_unavailable_and_does_not_count_toward_n() {
        let tip = |width, height| SampTip {
            width,
            height,
            fill: 0x80,
            corrupt: false,
        };
        // Legacy entries are listed in reverse, so the drawable tip comes last.
        // A 0x20 tip is taller than max_cell, which downsample would widen to 1.
        let bytes = legacy_abr(&[tip(1, 1), tip(0, 20), tip(0, 1), tip(0, 1), tip(0, 1)]);

        let entries = preview_abr(&bytes, OPTS).unwrap().entries;
        assert_eq!(entries.len(), 5);
        for entry in &entries[..4] {
            assert!(
                matches!(
                    &entry.tip,
                    TipPreview::Unavailable(UnavailableReason::Corrupt(msg)) if msg == "tip has zero area"
                ),
                "{entry:?}"
            );
        }

        let entries = preview_abr_first_available(&bytes, OPTS, 4)
            .unwrap()
            .entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].index, 4);
        assert!(matches!(
            &entries[0].tip,
            TipPreview::Available(b) if (b.width, b.height, b.data.as_slice()) == (1, 1, &[0x80][..])
        ));
    }

    #[test]
    fn brushset_reads_only_the_members_it_returns() {
        let archive = brush_archive("Tip");
        let shape = gray_png(4, 4, 200);
        let plist = brushset_plist("Set", &["a", "b", "c", "d"]);
        let mut files: Vec<(String, &[u8])> = vec![("brushset.plist".into(), &plist)];
        for member in ["a", "b", "c", "d"] {
            files.push((format!("{member}/Brush.archive"), &archive));
            files.push((format!("{member}/Shape.png"), &shape));
        }
        let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.as_str(), *b)).collect();
        let bytes = zip_with(&files);
        assert_eq!(
            entries_built(|| preview_brushset_first_available(&bytes, OPTS, 1).unwrap()),
            1
        );
        assert_eq!(entries_built(|| preview_brushset(&bytes, OPTS).unwrap()), 4);
    }
}
