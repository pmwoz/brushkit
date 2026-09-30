//! Tip bitmaps for every brush file this workspace reads.
//!
//! `preview_abr`, `preview_brush` and `preview_brushset` each take the whole
//! file as bytes and return one [`PreviewEntry`] per brush in file order,
//! available or not. A brush whose tip cannot be rendered is reported with a
//! reason rather than dropped, so a caller can lay out a complete grid. Each
//! has a `_first_available` twin that returns only the first `n` available
//! entries and builds no entry after them.
//!
//! [`preview`] is the function all six call. It also takes a `keep_going`
//! callback that can stop the call between entries, for a host that shows
//! previews under a time limit.
//!
//! The available tips of one call hold at most [`MAX_PREVIEW_BYTES`] of bitmap
//! data together. Entries past that point are
//! [`UnavailableReason::OverBudget`] and their tips are not decoded.
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

use brushkit_abr::{
    parse_abr_all_deferred_without_patterns, DeferredPack, SampledBrush, ShapeTipFamily,
    UnavailableTip,
};
use std::collections::HashMap;
use std::io::Cursor;
use std::num::NonZeroU32;

#[derive(Debug, Clone, Copy)]
pub struct PreviewOptions {
    /// Larger side of every returned tip is at most this many pixels (>= 1).
    pub max_cell: u32,
}

/// The most bytes of bitmap data ([`GrayscaleBitmap::data`]) the `Available`
/// tips of one [`preview`] call, or one call of a function built on it, hold
/// together.
pub const MAX_PREVIEW_BYTES: usize = 256 * 1024 * 1024;

// A `Shape.png` at the largest size the reader accepts must fit on its own, or
// such a tip could never be available.
const _: () = assert!(MAX_PREVIEW_BYTES >= (procreate::MAX_PNG_DIMENSION as usize).pow(2));

#[derive(Debug, Clone)]
pub struct PreviewSet {
    pub set_name: Option<String>,
    pub entries: Vec<PreviewEntry>,
    /// Entries of the file that were not built because the `keep_going`
    /// callback of [`preview`] returned `false`. 0 when the call ran to its
    /// end.
    pub not_reached: usize,
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
    /// The tip was not returned because it would take the call's available
    /// tips past [`MAX_PREVIEW_BYTES`]. Every later entry that has a tip to
    /// render is `OverBudget` too, and its tip is not decoded.
    OverBudget,
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

/// The kind of brush file [`preview`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Abr,
    Brush,
    Brushset,
}

/// Which entries a [`preview`] returns.
#[derive(Debug, Clone, Copy)]
pub enum Take {
    /// Every entry, available or not.
    All,
    /// The first `n` entries whose tip is available, with the `index` they
    /// have under `All`, so indices may skip. No entry is built after the
    /// `n`th available one or after the first `OverBudget` one, since no
    /// later entry can be available.
    FirstAvailable(usize),
}

impl Take {
    /// The entries to return and how many were not reached because
    /// `keep_going` returned `false`. `entries` must be lazy, as it is pulled
    /// only as far as needed.
    fn collect(
        self,
        mut entries: impl ExactSizeIterator<Item = PreviewEntry>,
        keep_going: &mut dyn FnMut() -> bool,
    ) -> (Vec<PreviewEntry>, usize) {
        let mut stopped = false;
        let gated = std::iter::from_fn(|| {
            // Checked first so `keep_going` is not asked about an entry that
            // does not exist.
            if entries.len() == 0 {
                return None;
            }
            if !keep_going() {
                stopped = true;
                return None;
            }
            entries.next()
        });
        let taken = match self {
            Take::All => gated.collect(),
            Take::FirstAvailable(n) => gated
                .take_while(|entry| {
                    !matches!(
                        entry.tip,
                        TipPreview::Unavailable(UnavailableReason::OverBudget)
                    )
                })
                .filter(|entry| matches!(entry.tip, TipPreview::Available(_)))
                .take(n)
                .collect(),
        };
        let not_reached = if stopped { entries.len() } else { 0 };
        (taken, not_reached)
    }
}

/// Tips for a brush file of `format`, as `take` selects them.
///
/// `keep_going` is called before each entry is built. The first `false` stops
/// the call: it returns the entries `take` selected so far and counts the
/// entries not built in [`PreviewSet::not_reached`]. The work before the
/// first entry, such as opening a zip or parsing the `.abr` index, and the
/// entry being built are not interrupted. A call that ends because `take` has
/// all it needs is not stopped, and `keep_going` is not called after that.
///
/// The entries of a file are the rows of an `.abr` preview, the members a
/// `.brushset` lists, or the single brush of a `.brush`.
pub fn preview(
    bytes: &[u8],
    format: Format,
    opts: PreviewOptions,
    take: Take,
    keep_going: &mut dyn FnMut() -> bool,
) -> Result<PreviewSet, PreviewError> {
    match format {
        Format::Abr => abr(bytes, opts, take, keep_going, MAX_PREVIEW_BYTES),
        Format::Brush => brush(bytes, opts, take, keep_going, MAX_PREVIEW_BYTES),
        Format::Brushset => brushset(bytes, opts, take, keep_going, MAX_PREVIEW_BYTES),
    }
}

/// What is left of the bitmap byte budget of one preview call, `None` once a
/// tip did not fit.
struct Budget(Option<usize>);

impl Budget {
    fn new(bytes: usize) -> Self {
        Budget(Some(bytes))
    }

    /// Runs `render` and keeps an available tip only if it fits in what is
    /// left. The first tip that does not fit spends the budget for good, so
    /// no later tip is rendered.
    fn render(&mut self, render: impl FnOnce() -> TipPreview) -> TipPreview {
        let over = TipPreview::Unavailable(UnavailableReason::OverBudget);
        let Some(left) = self.0 else {
            return over;
        };
        match render() {
            TipPreview::Available(bitmap) => match left.checked_sub(bitmap.data.len()) {
                Some(rest) => {
                    self.0 = Some(rest);
                    TipPreview::Available(bitmap)
                }
                None => {
                    self.0 = None;
                    over
                }
            },
            unavailable => unavailable,
        }
    }
}

/// Where an `.abr` preview row came from: an index into `sampled_brushes`,
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
/// is reported as an unsupported tip kind. A sampled tip that is missing, or
/// whose samp record or pixels fail to decode, is a `Corrupt` entry, not an
/// error for the whole preview. The exception is a raw v1 or v2 tip whose
/// pixels run past the end of the input, which fails the whole preview.
///
/// Embedded pattern payloads are neither copied nor decoded.
pub fn preview_abr(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError> {
    preview(bytes, Format::Abr, opts, Take::All, &mut || true)
}

/// The first `n` entries of [`preview_abr`] whose tip is available, in the
/// same order and with the same `index` they have there, so indices may skip.
/// Unavailable entries do not count toward `n`, and entries after the `n`th
/// available one or after the first `OverBudget` one are not built, so their
/// tips are not decoded or downsampled.
///
/// The whole pack is still parsed, but the parse decodes no tip.
pub fn preview_abr_first_available(
    bytes: &[u8],
    opts: PreviewOptions,
    n: usize,
) -> Result<PreviewSet, PreviewError> {
    preview(
        bytes,
        Format::Abr,
        opts,
        Take::FirstAvailable(n),
        &mut || true,
    )
}

fn abr(
    bytes: &[u8],
    opts: PreviewOptions,
    take: Take,
    keep_going: &mut dyn FnMut() -> bool,
    budget_bytes: usize,
) -> Result<PreviewSet, PreviewError> {
    let max_cell = check_max_cell(opts)?;

    let deferred =
        parse_abr_all_deferred_without_patterns(bytes).map_err(|e| PreviewError(e.to_string()))?;
    let pack = &deferred.pack;

    let mut rows: Vec<Row> = Vec::new();
    rows.extend(pack.sampled_brushes.iter().enumerate().map(|(j, sampled)| {
        let preset_index = match sampled {
            SampledBrush::Readable(i) => pack.brushes[*i].preset_index,
            SampledBrush::Unavailable(brush) => brush.preset_index,
        };
        Row {
            key: preset_index.unwrap_or(usize::MAX),
            source: Source::Sampled(j),
        }
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

    let mut budget = Budget::new(budget_bytes);
    let entries = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| abr_entry(&deferred, index, row.source, max_cell, &mut budget));

    let (entries, not_reached) = take.collect(entries, keep_going);
    Ok(PreviewSet {
        set_name: None,
        entries,
        not_reached,
    })
}

/// Render one `.abr` row. A sampled tip is decoded and downsampled here, so
/// only the rows a caller takes are decoded.
fn abr_entry(
    deferred: &DeferredPack<'_>,
    index: usize,
    source: Source,
    max_cell: u32,
    budget: &mut Budget,
) -> PreviewEntry {
    #[cfg(test)]
    tests::record_read();
    let pack = &deferred.pack;
    let (name, tip, source_dimensions) = match source {
        Source::Sampled(j) => match &pack.sampled_brushes[j] {
            &SampledBrush::Readable(i) => {
                let brush = &pack.brushes[i];
                let tip = budget.render(|| match deferred.decode_tip(i) {
                    Ok(tip) => tip_preview(&to_grayscale(&tip), max_cell),
                    Err(e) => TipPreview::Unavailable(UnavailableReason::Corrupt(e.to_string())),
                });
                (
                    name_or_id(&brush.name, &brush.id),
                    tip,
                    SourceDimensions::new(brush.tip.width, brush.tip.height),
                )
            }
            SampledBrush::Unavailable(brush) => {
                let text = match &brush.cause {
                    UnavailableTip::Missing { uuid } => format!("sampled tip {uuid} is missing"),
                    UnavailableTip::Unreadable(message) => message.clone(),
                };
                (
                    name_or_id(&brush.name, &brush.id),
                    TipPreview::Unavailable(UnavailableReason::Corrupt(text)),
                    None,
                )
            }
        },
        Source::Computed(i) => {
            let preset = &pack.computed_presets[i];
            let unsupported = || {
                TipPreview::Unavailable(UnavailableReason::UnsupportedTipKind(
                    "computed".to_string(),
                ))
            };
            let tip = match preset
                .descriptor
                .computed
                .as_ref()
                .filter(|geom| can_synthesize(geom))
            {
                Some(geom) => budget.render(|| {
                    synthesize_computed_tip(geom)
                        .map_or_else(unsupported, |bitmap| tip_preview(&bitmap, max_cell))
                }),
                None => unsupported(),
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

fn name_or_id(name: &str, id: &str) -> String {
    if name.is_empty() { id } else { name }.to_string()
}

/// A full-size tip downsampled to `max_cell`. A zero-area tip is not
/// drawable, and `downsample` would widen its zero side to 1 when the other
/// side exceeds `max_cell`.
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
    preview(bytes, Format::Brushset, opts, Take::All, &mut || true)
}

/// The first `n` entries of [`preview_brushset`] whose tip is available, in
/// the same order and with the same `index` they have there, so indices may
/// skip. Unavailable entries do not count toward `n`, and members after the
/// `n`th available one or after the first `OverBudget` one are not read.
pub fn preview_brushset_first_available(
    bytes: &[u8],
    opts: PreviewOptions,
    n: usize,
) -> Result<PreviewSet, PreviewError> {
    preview(
        bytes,
        Format::Brushset,
        opts,
        Take::FirstAvailable(n),
        &mut || true,
    )
}

fn brushset(
    bytes: &[u8],
    opts: PreviewOptions,
    take: Take,
    keep_going: &mut dyn FnMut() -> bool,
    budget_bytes: usize,
) -> Result<PreviewSet, PreviewError> {
    let max_cell = check_max_cell(opts)?;
    let mut zip = open_zip(bytes)?;

    let (set_name, prefixes) = if zip.by_name("brushset.plist").is_ok() {
        let buf = procreate::read_zip_plist(&mut zip, "brushset.plist").map_err(PreviewError)?;
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

    // The last index that lists each member. A member listed again later is
    // kept after its entry instead of read again. Inserted one at a time, as
    // `collect` would size the map for every reference, not every member.
    let mut last: HashMap<&str, usize> = HashMap::new();
    for (index, prefix) in prefixes.iter().enumerate() {
        last.insert(prefix, index);
    }

    let mut budget = Budget::new(budget_bytes);
    let mut kept: HashMap<&str, Member> = HashMap::new();
    let entries = prefixes.iter().enumerate().map(|(index, prefix)| {
        let member = match kept.remove(prefix.as_str()) {
            Some(member) => member.again(&mut budget),
            None => read_member(&mut zip, prefix, max_cell, &mut budget),
        };
        if last[prefix.as_str()] > index {
            kept.insert(prefix, member.clone());
        }
        member.entry(index)
    });

    let (entries, not_reached) = take.collect(entries, keep_going);
    Ok(PreviewSet {
        set_name,
        entries,
        not_reached,
    })
}

/// The tip of a single Procreate `.brush`: one entry at index 0, read from the
/// archive's root rather than from a member directory.
pub fn preview_brush(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError> {
    preview(bytes, Format::Brush, opts, Take::All, &mut || true)
}

/// [`preview_brush`] limited to available tips: its single entry when the tip
/// is available and `n >= 1`, otherwise no entries. With `n == 0` the tip is
/// not decoded.
pub fn preview_brush_first_available(
    bytes: &[u8],
    opts: PreviewOptions,
    n: usize,
) -> Result<PreviewSet, PreviewError> {
    preview(
        bytes,
        Format::Brush,
        opts,
        Take::FirstAvailable(n),
        &mut || true,
    )
}

fn brush(
    bytes: &[u8],
    opts: PreviewOptions,
    take: Take,
    keep_going: &mut dyn FnMut() -> bool,
    budget_bytes: usize,
) -> Result<PreviewSet, PreviewError> {
    let max_cell = check_max_cell(opts)?;
    let mut zip = open_zip(bytes)?;

    if zip.by_name("Brush.archive").is_err() {
        return Err(PreviewError("Brush.archive not found".to_string()));
    }

    let mut budget = Budget::new(budget_bytes);
    let entry = std::iter::once_with(|| read_member(&mut zip, "", max_cell, &mut budget).entry(0));
    let (entries, not_reached) = take.collect(entry, keep_going);
    Ok(PreviewSet {
        set_name: None,
        entries,
        not_reached,
    })
}

fn open_zip(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, PreviewError> {
    let fail = |e: zip::result::ZipError| PreviewError(format!("failed to open zip: {e}"));
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(fail)?;
    if has_overlapping_entries(&mut zip).map_err(fail)? {
        return Err(fail(zip::result::ZipError::InvalidArchive(
            "entries overlap",
        )));
    }
    Ok(zip)
}

/// The zip format lets many central-directory names point at one local header,
/// or place one entry's header inside another entry's data, and the `zip` crate
/// rejects neither. Both let two names read the same stored bytes. Disjoint
/// `[header_start, data_start + compressed_size)` ranges keep the bytes one call
/// reads within the file size. See <https://github.com/pmwoz/brushkit/issues/154>.
fn has_overlapping_entries(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
) -> zip::result::ZipResult<bool> {
    let mut ranges = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i)?;
        let end = entry.data_start().saturating_add(entry.compressed_size());
        ranges.push((entry.header_start(), end));
    }
    ranges.sort_unstable();
    Ok(ranges.windows(2).any(|pair| pair[1].0 < pair[0].1))
}

/// One member of a Procreate archive, as the entry for one reference to it.
#[derive(Clone)]
struct Member {
    name: String,
    source_dimensions: Option<SourceDimensions>,
    tip: MemberTip,
}

#[derive(Clone)]
enum MemberTip {
    /// A reason found without decoding the tip, which holds for every
    /// reference.
    Fixed(UnavailableReason),
    /// The tip after the budget, which a later reference passes through the
    /// budget again.
    Budgeted(TipPreview),
}

impl Member {
    /// The member for a later reference: its tip goes through the budget
    /// again, so a repeated member still spends budget and is `OverBudget`
    /// after the stop, as it would be if read again.
    fn again(self, budget: &mut Budget) -> Member {
        let tip = match self.tip {
            MemberTip::Budgeted(tip) => MemberTip::Budgeted(budget.render(|| tip)),
            fixed => fixed,
        };
        Member { tip, ..self }
    }

    fn entry(self, index: usize) -> PreviewEntry {
        PreviewEntry {
            index,
            name: self.name,
            tip: match self.tip {
                MemberTip::Fixed(reason) => TipPreview::Unavailable(reason),
                MemberTip::Budgeted(tip) => tip,
            },
            source_dimensions: self.source_dimensions,
        }
    }
}

/// Reads one member of a Procreate archive. `prefix` is `"{uuid}/"` for a
/// `.brushset` member and `""` for a root-layout `.brush`.
///
/// A member is always an entry: an archive that cannot be read names the entry
/// after its directory and reports why, rather than shifting every index after
/// it. A readable `Shape.png` reports its size either way.
fn read_member(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    prefix: &str,
    max_cell: u32,
    budget: &mut Budget,
) -> Member {
    #[cfg(test)]
    tests::record_read();
    let fallback_name = if prefix.is_empty() {
        "Brush".to_string()
    } else {
        prefix.trim_end_matches('/').to_string()
    };

    let archive = procreate::read_zip_plist(zip, &format!("{prefix}Brush.archive"))
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
        Ok(name) => (
            name.unwrap_or(fallback_name),
            shape_tip(shape, max_cell, budget),
        ),
        Err(msg) => (
            fallback_name,
            MemberTip::Fixed(UnavailableReason::Corrupt(msg)),
        ),
    };

    Member {
        name,
        source_dimensions,
        tip,
    }
}

/// The tip for a member's `Shape.png`: `None` when the member has no shape,
/// otherwise the read result.
fn shape_tip(
    shape: Option<Result<Vec<u8>, String>>,
    max_cell: u32,
    budget: &mut Budget,
) -> MemberTip {
    let png = match shape {
        None => return MemberTip::Fixed(UnavailableReason::NoShapePng),
        Some(Err(msg)) => return MemberTip::Fixed(UnavailableReason::Corrupt(msg)),
        Some(Ok(png)) => png,
    };
    MemberTip::Budgeted(budget.render(|| match procreate::decode_tip_png(&png) {
        Ok(bitmap) => tip_preview(&bitmap, max_cell),
        Err(procreate::ShapePngError::TooLarge { width, height }) => {
            TipPreview::Unavailable(UnavailableReason::TooLarge { width, height })
        }
        Err(procreate::ShapePngError::Corrupt(msg)) => {
            TipPreview::Unavailable(UnavailableReason::Corrupt(msg))
        }
    }))
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
        static READS: Cell<usize> = const { Cell::new(0) };
    }

    /// Called once per `.abr` row or Procreate member read, before any of
    /// its tip is read or decoded.
    pub(super) fn record_read() {
        READS.with(|count| count.set(count.get() + 1));
    }

    /// Rows and members read by `f` on this thread.
    fn reads<T>(f: impl FnOnce() -> T) -> usize {
        READS.with(|count| count.set(0));
        f();
        READS.with(Cell::get)
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
            reads(|| preview_abr_first_available(&bytes, OPTS, 2).unwrap()),
            2
        );
        assert_eq!(
            reads(|| preview_abr_first_available(&bytes, OPTS, 0).unwrap()),
            0
        );
        assert_eq!(reads(|| preview_abr(&bytes, OPTS).unwrap()), 6);
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
            reads(|| set = Some(preview_abr_first_available(&bytes, OPTS, 1).unwrap())),
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
        let sized = |width, height| SampTip {
            width,
            height,
            fill: 0x80,
            corrupt: false,
        };
        // Legacy entries are listed in reverse, so the drawable tip comes last.
        // The 0x20 and 20x0 tips exceed max_cell on one side, which downsample
        // would widen to 1.
        let bytes = legacy_abr(&[
            sized(1, 1),
            sized(0, 20),
            sized(0, 1),
            sized(0, 1),
            sized(20, 0),
        ]);

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
            reads(|| preview_brushset_first_available(&bytes, OPTS, 1).unwrap()),
            1
        );
        assert_eq!(reads(|| preview_brushset(&bytes, OPTS).unwrap()), 4);
    }

    #[test]
    fn a_member_listed_many_times_is_read_once() {
        let bytes = brushset_of(&["a"; 1000]);
        let mut set = None;
        assert_eq!(
            reads(|| set = Some(preview_brushset(&bytes, OPTS).unwrap())),
            1
        );
        let entries = set.unwrap().entries;
        assert_eq!(entries.len(), 1000);
        for (i, entry) in entries.iter().enumerate() {
            assert_eq!((entry.index, entry.name.as_str()), (i, "Tip"));
            assert!(matches!(&entry.tip, TipPreview::Available(b) if b.data == [200; 16]));
        }
    }

    #[test]
    fn interleaved_members_are_each_read_once() {
        let bytes = brushset_of(&["c", "a", "n", "c", "a", "n"]);
        let mut set = None;
        assert_eq!(
            reads(|| set = Some(preview_brushset(&bytes, OPTS).unwrap())),
            3
        );
        let reasons: Vec<Option<UnavailableReason>> = set
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| match entry.tip {
                TipPreview::Available(_) => None,
                TipPreview::Unavailable(reason) => Some(reason),
            })
            .collect();
        assert!(matches!(
            reasons[..3],
            [
                Some(UnavailableReason::Corrupt(_)),
                None,
                Some(UnavailableReason::NoShapePng)
            ]
        ));
        assert_eq!(reasons[..3], reasons[3..]);
    }

    fn sized(width: u32, height: u32) -> SampTip {
        SampTip {
            width,
            height,
            fill: 0x80,
            corrupt: false,
        }
    }

    /// A `.brushset` whose plist lists `members` in order, each a member
    /// directory. Member `a` has a 4x4 `Shape.png`, `c` has a `Shape.png` that
    /// fails to decode, `n` has none and `x` has an unreadable `Brush.archive`.
    fn brushset_of(members: &[&str]) -> Vec<u8> {
        let archive = brush_archive("Tip");
        let shape = gray_png(4, 4, 200);
        let plist = brushset_plist("Set", members);
        zip_with(&[
            ("brushset.plist", &plist),
            ("a/Brush.archive", &archive),
            ("a/Shape.png", &shape),
            ("c/Brush.archive", &archive),
            ("c/Shape.png", b"not a png"),
            ("n/Brush.archive", &archive),
            ("x/Brush.archive", b"not a plist"),
            ("x/Shape.png", &shape),
        ])
    }

    /// Asserts that `bounded` is `full` with every tip from index `fit` on
    /// `OverBudget`, and that the tips it keeps hold at most `budget` bytes.
    fn assert_bounded(full: &PreviewSet, bounded: &PreviewSet, fit: usize, budget: usize) {
        assert_eq!(bounded.entries.len(), full.entries.len());
        let mut bytes = 0;
        for (i, (got, want)) in bounded.entries.iter().zip(&full.entries).enumerate() {
            assert_eq!(
                (got.index, &got.name, got.source_dimensions),
                (i, &want.name, want.source_dimensions)
            );
            match &got.tip {
                TipPreview::Available(bitmap) if i < fit => bytes += bitmap.data.len(),
                TipPreview::Unavailable(UnavailableReason::OverBudget) if i >= fit => {}
                tip => panic!("entry {i}: {tip:?}"),
            }
        }
        assert!(bytes <= budget, "{bytes} bytes over a budget of {budget}");
    }

    #[test]
    fn brushset_tips_past_the_budget_are_over_budget() {
        let bytes = brushset_of(&["a"; 6]);
        let full = brushset(&bytes, OPTS, Take::All, &mut || true, MAX_PREVIEW_BYTES).unwrap();
        // Each 4x4 tip is 16 bytes, so three fit in 50.
        let bounded = brushset(&bytes, OPTS, Take::All, &mut || true, 50).unwrap();
        assert_bounded(&full, &bounded, 3, 50);
    }

    #[test]
    fn abr_tips_past_the_first_that_does_not_fit_are_over_budget() {
        // Listed in reverse: 16, 16, 36 and 4 bytes. The 4-byte tip would fit
        // after the 36-byte one does not.
        let bytes = samp_abr(&[sized(2, 2), sized(6, 6), sized(4, 4), sized(4, 4)]);
        let full = abr(&bytes, OPTS, Take::All, &mut || true, MAX_PREVIEW_BYTES).unwrap();
        let bounded = abr(&bytes, OPTS, Take::All, &mut || true, 40).unwrap();
        assert_bounded(&full, &bounded, 2, 40);
    }

    #[test]
    fn entries_unavailable_for_their_own_reason_keep_it_past_the_budget() {
        // `c` is `Corrupt` only when decoded, so `OverBudget` after the stop
        // shows its tip was not decoded.
        let bytes = brushset_of(&["c", "a", "a", "n", "x", "c", "a"]);
        let reasons: Vec<Option<UnavailableReason>> =
            brushset(&bytes, OPTS, Take::All, &mut || true, 20)
                .unwrap()
                .entries
                .into_iter()
                .map(|entry| match entry.tip {
                    TipPreview::Available(_) => None,
                    TipPreview::Unavailable(reason) => Some(reason),
                })
                .collect();
        assert!(matches!(
            reasons.as_slice(),
            [
                Some(UnavailableReason::Corrupt(_)),
                None,
                Some(UnavailableReason::OverBudget),
                Some(UnavailableReason::NoShapePng),
                Some(UnavailableReason::Corrupt(_)),
                Some(UnavailableReason::OverBudget),
                Some(UnavailableReason::OverBudget),
            ]
        ));
    }

    #[test]
    fn first_available_stops_at_the_first_over_budget_entry() {
        let bytes = samp_abr(&std::array::from_fn::<_, 6, _>(|_| tip(false)));
        let mut set = None;
        assert_eq!(
            reads(|| set =
                Some(abr(&bytes, OPTS, Take::FirstAvailable(5), &mut || true, 40).unwrap())),
            3
        );
        let indices: Vec<usize> = set.unwrap().entries.iter().map(|e| e.index).collect();
        assert_eq!(indices, [0, 1]);
    }

    /// A `keep_going` that returns `true` for its first `k` calls and `false`
    /// after that.
    fn stop_after(k: usize) -> impl FnMut() -> bool {
        let mut calls = 0;
        move || {
            calls += 1;
            calls <= k
        }
    }

    fn debug(entries: &[PreviewEntry]) -> Vec<String> {
        entries.iter().map(|entry| format!("{entry:?}")).collect()
    }

    fn brush_file() -> Vec<u8> {
        zip_with(&[
            ("Brush.archive", &brush_archive("Tip")),
            ("Shape.png", &gray_png(4, 4, 200)),
        ])
    }

    #[test]
    fn a_stop_returns_the_entries_built_before_it() {
        let files = [
            (
                Format::Abr,
                samp_abr(&[tip(false), tip(true), tip(false), tip(false), tip(true)]),
            ),
            (
                Format::Brushset,
                brushset_of(&["a", "c", "n", "a", "x", "a"]),
            ),
        ];
        for (format, bytes) in files {
            let mut calls = 0;
            let full = preview(&bytes, format, OPTS, Take::All, &mut || {
                calls += 1;
                true
            })
            .unwrap();
            let total = full.entries.len();
            assert_eq!(calls, total, "{format:?}");
            for k in 0..=total {
                let all = preview(&bytes, format, OPTS, Take::All, &mut stop_after(k)).unwrap();
                assert_eq!(all.set_name, full.set_name);
                assert_eq!(
                    debug(&all.entries),
                    debug(&full.entries[..k]),
                    "{format:?} {k}"
                );
                assert_eq!(all.not_reached, total - k, "{format:?} {k}");

                let take = Take::FirstAvailable(total);
                let first = preview(&bytes, format, OPTS, take, &mut stop_after(k)).unwrap();
                let available: Vec<PreviewEntry> = full.entries[..k]
                    .iter()
                    .filter(|entry| matches!(entry.tip, TipPreview::Available(_)))
                    .cloned()
                    .collect();
                assert_eq!(debug(&first.entries), debug(&available), "{format:?} {k}");
                assert_eq!(first.not_reached, total - k, "{format:?} {k}");
            }
        }
    }

    #[test]
    fn a_stop_on_the_first_call_builds_no_entry() {
        let files = [
            (
                Format::Abr,
                samp_abr(&[tip(false), tip(false), tip(false)]),
                3,
            ),
            (Format::Brushset, brushset_of(&["a", "n", "a", "c"]), 4),
            (Format::Brush, brush_file(), 1),
        ];
        for (format, bytes, total) in files {
            for take in [Take::All, Take::FirstAvailable(total)] {
                let mut set = None;
                assert_eq!(
                    reads(|| set = Some(preview(&bytes, format, OPTS, take, &mut || false))),
                    0,
                    "{format:?} {take:?}"
                );
                let set = set.unwrap().unwrap();
                assert!(set.entries.is_empty(), "{format:?} {take:?}");
                assert_eq!(set.not_reached, total, "{format:?} {take:?}");
            }
        }
    }

    #[test]
    fn a_stopped_brushset_does_not_read_later_members() {
        // `c` is `Corrupt` only once its `Shape.png` is read and decoded.
        let bytes = brushset_of(&["a", "c"]);
        let full = preview_brushset(&bytes, OPTS).unwrap();
        assert!(matches!(
            full.entries[1].tip,
            TipPreview::Unavailable(UnavailableReason::Corrupt(_))
        ));

        let mut set = None;
        let stop = || {
            preview(
                &bytes,
                Format::Brushset,
                OPTS,
                Take::All,
                &mut stop_after(1),
            )
        };
        assert_eq!(reads(|| set = Some(stop())), 1);
        let set = set.unwrap().unwrap();
        assert!(set.entries.iter().all(|entry| !matches!(
            entry.tip,
            TipPreview::Unavailable(UnavailableReason::Corrupt(_))
        )));
        assert_eq!(set.not_reached, 1);
    }

    #[test]
    fn first_available_that_takes_all_it_needs_is_not_stopped() {
        let bytes = samp_abr(&std::array::from_fn::<_, 6, _>(|_| tip(false)));
        let set = abr(
            &bytes,
            OPTS,
            Take::FirstAvailable(1),
            &mut stop_after(1),
            MAX_PREVIEW_BYTES,
        )
        .unwrap();
        assert_eq!((set.entries.len(), set.not_reached), (1, 0));

        // Two 16-byte tips fit in 40, so the third entry is `OverBudget`.
        let set = abr(
            &bytes,
            OPTS,
            Take::FirstAvailable(5),
            &mut stop_after(3),
            40,
        )
        .unwrap();
        assert_eq!((set.entries.len(), set.not_reached), (2, 0));
    }

    #[test]
    fn the_preview_functions_run_to_the_end() {
        let abr_bytes = samp_abr(&[tip(false), tip(false), tip(false)]);
        let set_bytes = brushset_of(&["a", "a", "a"]);
        let brush_bytes = brush_file();
        let sets = [
            preview_abr(&abr_bytes, OPTS),
            preview_abr_first_available(&abr_bytes, OPTS, 1),
            preview_brushset(&set_bytes, OPTS),
            preview_brushset_first_available(&set_bytes, OPTS, 1),
            preview_brush(&brush_bytes, OPTS),
            preview_brush_first_available(&brush_bytes, OPTS, 0),
        ];
        for set in sets {
            assert_eq!(set.unwrap().not_reached, 0);
        }
    }
}
