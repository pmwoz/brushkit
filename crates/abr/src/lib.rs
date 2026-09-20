//! Pure Rust reader for Adobe Photoshop .abr brush files.
//!
//! Supports ABR versions 1 and 2 (legacy entry stream, Photoshop 6 to CS) and
//! 6, 7, 9 and 10 (8BIM block-based format, Photoshop CS and later).
//! Extracts brush tip bitmaps, names, and raw descriptors.
//! Reading only: no writer, no filesystem access — `parse_abr` takes bytes and
//! returns pure data structs for downstream use.

pub(crate) mod descriptor;
pub mod dump;
mod limits;
mod parser;
mod pattern;

pub use parser::{parse_abr, parse_abr_deferred, DeferredPack, DeferredTip};
pub use pattern::AbrPattern;

/// A parsed ABR brush pack.
#[derive(Debug, Clone)]
pub struct AbrPack {
    pub version: AbrVersion,
    pub brushes: Vec<AbrBrush>,
    /// Total number of brush presets the file declares — including
    /// computed/procedural presets that carry no sampled bitmap and therefore
    /// do NOT appear in `brushes`.
    ///
    /// v6/v10: the number of presets in the descriptor (`desc`) block. The
    /// relationship to `brushes.len()` depends on which pairing path ran. On the
    /// presets-first path (desc references sampled tips by uuid) `brushes` holds
    /// one entry per *resolvable* preset, so `preset_count >= brushes.len()`
    /// (unresolvable/computed presets are counted but not surfaced here, and
    /// dual-brush component tips in the samp block are intentionally dropped). On
    /// the fallback path (desc missing/unparseable, or no resolvable uuids)
    /// `brushes` holds one entry per samp bitmap, so `preset_count` may be LESS
    /// than `brushes.len()` (bitmaps decode without descriptors). Consumers must
    /// tolerate `preset_count` differing from `brushes.len()` in either direction.
    ///
    /// v1/v2: the entry count from the file header; type-1 (computed) entries are
    /// skipped by the parser and never reach `brushes`.
    pub preset_count: usize,
    pub raw_desc_block: Option<Vec<u8>>,
    /// Computed/procedural presets: they carry tip geometry but no sampled
    /// bitmap, so they are counted by `preset_count` yet absent from `brushes`.
    /// Empty for v2 files. A consumer synthesizes tips from this geometry.
    pub computed_presets: Vec<ComputedPreset>,
    /// Texture patterns embedded in the `patt` block (`AbrPattern`), keyed by
    /// UUID. Empty when the block is absent or empty (10/10 real packs, 17/18
    /// fixtures). A consumer matches these to a brush's `Txtr > Ptrn > Idnt` to
    /// build a grain.
    pub patterns: Vec<AbrPattern>,
    /// `patt` records that failed to decode, so the textures they carry are
    /// absent from `patterns` — a silent loss an earlier walker had. Zero across
    /// the whole corpus today; a whole-corpus census is the tripwire that holds
    /// it there.
    pub dropped_pattern_count: usize,
    /// Per-item detail for the dropped `patt` records. Invariant:
    /// `dropped_pattern_count == dropped_pattern_details.len()`. Empty when the
    /// block is absent, empty, or fully decodable.
    pub dropped_pattern_details: Vec<DroppedPatternDetail>,
    /// Chunks inside a `patt` block whose header did not parse. These are NOT
    /// counted as dropped patterns: an unreadable header carries no id and no
    /// mode, and on a file with trailing junk inside the block nothing was
    /// lost. They are counted because invisibility is what let an earlier bug
    /// hide — a walk misaligned by two bytes produced 27 of these corpus-wide
    /// and no test could see them. Zero across the whole corpus today; a
    /// whole-corpus census holds it there.
    pub unreadable_patt_chunk_count: usize,
    /// Per-item detail. Invariant:
    /// `unreadable_patt_chunk_count == unreadable_patt_chunks.len()`.
    pub unreadable_patt_chunks: Vec<UnreadablePattChunk>,
    /// Samp bitmaps referenced by no emitted preset — dual-brush component tips
    /// the presets-first pairing intentionally drops (they are not user-facing
    /// brushes). Only the presets-first path produces a nonzero value; the
    /// index-fallback path and v2 files always report 0.
    pub dropped_samp_count: usize,
    /// Presets whose `sampledData` uuid resolved to no samp bitmap (dangling
    /// references), so they surface as neither a paired brush nor a computed
    /// preset. Presets carrying NO uuid are excluded here (they are computed
    /// candidates counted via `computed_presets`). Presets-first path only; 0
    /// on the fallback path and for v2 files.
    pub skipped_preset_count: usize,
    /// Per-item detail for the dropped samp tips. Invariant:
    /// `dropped_samp_count == dropped_tip_details.len()`. Empty on the fallback
    /// path and for v2 files.
    pub dropped_tip_details: Vec<DroppedTipDetail>,
    /// Per-item detail for the skipped (dangling) presets. Invariant:
    /// `skipped_preset_count == skipped_preset_details.len()`. Empty on the
    /// fallback path and for v2 files.
    pub skipped_preset_details: Vec<SkippedPresetDetail>,
    /// Presets that yield no output at all: neither a sampled tip (`sampledData`
    /// uuid) nor computed geometry, so they surface as neither a brush nor a
    /// computed preset. Derived from the desc block alone, so it is independent
    /// of which pairing path ran. Always 0 for v2 files (no desc block exists).
    pub unsupported_tip_count: usize,
    /// Per-item detail for the unsupported-tip presets. Invariant:
    /// `unsupported_tip_count == unsupported_tip_presets.len()`. Empty for v2
    /// files.
    pub unsupported_tip_presets: Vec<UnsupportedTipPresetDetail>,
    /// `Some(message)` when a `desc` block failed to parse (first failing block
    /// wins), `None` for well-formed files and for desc blocks that legitimately
    /// contain zero presets. Informative only: a failing desc block still takes
    /// the index-fallback pairing path exactly as before. v2 files always `None`.
    pub desc_parse_error: Option<String>,
}

/// An empty pack: no brushes, no presets, no diagnostics, version `V10`.
///
/// This exists for **test and fixture construction** — `..Default::default()`
/// lets a test spell out only the fields it exercises, so adding a new
/// diagnostic field does not ripple through every test module in the
/// workspace. Production parse paths construct every field explicitly and
/// deliberately do NOT use this: a new field should force a decision there.
impl Default for AbrPack {
    fn default() -> Self {
        Self {
            version: AbrVersion::V10,
            brushes: Vec::new(),
            preset_count: 0,
            raw_desc_block: None,
            computed_presets: Vec::new(),
            patterns: Vec::new(),
            dropped_pattern_count: 0,
            dropped_pattern_details: Vec::new(),
            unreadable_patt_chunk_count: 0,
            unreadable_patt_chunks: Vec::new(),
            dropped_samp_count: 0,
            skipped_preset_count: 0,
            dropped_tip_details: Vec::new(),
            skipped_preset_details: Vec::new(),
            unsupported_tip_count: 0,
            unsupported_tip_presets: Vec::new(),
            desc_parse_error: None,
        }
    }
}

/// A samp tip the presets-first pairing dropped (referenced by no emitted
/// preset). `owner_preset_names` lists the presets that reference it as
/// their dual-brush component (deduplicated, desc order); empty when no
/// dualBrush reference points at it.
#[derive(Debug, Clone)]
pub struct DroppedTipDetail {
    pub uuid: Option<String>,
    pub width: u32,
    pub height: u32,
    pub owner_preset_names: Vec<String>,
    /// The decoded tip pixels, kept so the app can preview dropped content.
    /// Small — only dropped tips carry it.
    pub bitmap: TipBitmap,
}

/// A preset skipped because its sampledData uuid resolved to no samp entry.
#[derive(Debug, Clone)]
pub struct SkippedPresetDetail {
    pub name: String,
    pub uuid: String,
}

/// Which Photoshop dynamic-tip family a `Shp `-carrying preset belongs to,
/// read from the inner `Brsh` object's class id. Witnessed over the whole
/// corpus: 114 `dBrush` + 103 `dTips` = the entire 217-preset `Shp `
/// population, with no `Shp ` under any other class and no `dTips`/`dBrush`
/// without one (whole-corpus census).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeTipFamily {
    /// `dBrush` — bristle tips (Photoshop's Angle/Blunt/Fan × Flat/Round grid).
    Bristle,
    /// `dTips` — erodible and airbrush tips (pencils, pastels, erasers,
    /// airbrushes).
    Erodible,
}

/// The tip shape a `Shp `-carrying preset was authored with, read from the
/// `Brsh > Shp ` integer *together with* the enclosing class id. Measured with
/// a controlled Photoshop 27.9.1 fixture set — sixteen presets whose Shape was
/// picked from the Brush Settings dropdown, exported and read back, all sixteen
/// predictions holding.
///
/// The integer alone is ambiguous: `Shp `=5 is Flat Point under `dBrush` and
/// the airbrush tip under `dTips`, so any lookup that drops the
/// [`ShapeTipFamily`] is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipShape {
    /// `dBrush` 0 — Round Point.
    RoundPoint,
    /// `dBrush` 1 — Round Blunt.
    RoundBlunt,
    /// `dBrush` 2 — Round Curve.
    RoundCurve,
    /// `dBrush` 3 — Round Angle.
    RoundAngle,
    /// `dBrush` 4 — Round Fan.
    RoundFan,
    /// `dBrush` 5 — Flat Point.
    FlatPoint,
    /// `dBrush` 6 — Flat Blunt.
    FlatBlunt,
    /// `dBrush` 7 — Flat Curve.
    FlatCurve,
    /// `dBrush` 8 — Flat Angle.
    FlatAngle,
    /// `dBrush` 9 — Flat Fan.
    FlatFan,
    /// `dTips` 0 — Erodible Point.
    ErodiblePoint,
    /// `dTips` 1 — Erodible Flat (the chisel).
    ErodibleFlat,
    /// `dTips` 2 — Erodible Round.
    ErodibleRound,
    /// `dTips` 3 — Erodible Square.
    ErodibleSquare,
    /// `dTips` 4 — Erodible Triangle. Photoshop's `Custom` erodible tip stores
    /// this same 4, so a preset the UI called `Custom` reads back as Triangle.
    ErodibleTriangle,
    /// `dTips` 5 — the **airbrush** tip type, NOT a sixth erodible shape.
    /// Never describe it as an "erodible airbrush": the family says `dTips`,
    /// the shape says airbrush.
    ///
    /// Its sitting witness is weaker than the rest of the table: Photoshop
    /// shows no Shape dropdown for airbrush tips, so this row could not be
    /// *set* the way 0–4 were; it was read back off an airbrush preset rather
    /// than round-tripped through a control we chose. The corpus supplies the
    /// witness the sitting could not: over all 103 real-pack `dTips` presets,
    /// `dtipsType == 1` holds on exactly the 27 with `Shp ` == 5 and on no
    /// other.
    AirbrushTip,
}

/// Why a `patt` record was dropped instead of decoded.
///
/// Both variants presuppose a record whose HEADER parsed — that is the evidence
/// `parse_patt_block` requires before it will call a chunk a record at all. A
/// chunk with an unreadable header is not reported here; see that function's doc
/// comment for why the corpus forces that line.
///
/// Deliberately COARSE. The record parser bails from ~28 distinct field reads,
/// and threading a separate reason through each would be a large mechanical
/// refactor whose extra granularity has, today, exactly one witness to justify
/// it — and that witness is already an `UnsupportedImageMode`-adjacent case with
/// its own follow-up task. Splitting `Undecodable` is a follow-up **with a real
/// pack behind it**, per this repo's rule that claims about Photoshop bytes need
/// a corpus witness. The coarseness is a decision, not an oversight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DroppedPatternReason {
    /// The header parsed and named a PSD image mode the pattern decoder does
    /// not read. The silent class of loss: the bytes are fine, our mode
    /// coverage is not.
    UnsupportedImageMode(u32),
    /// **The header parsed and the body did not** — a truncated field, a
    /// defensive cap exceeded, a channel that failed to decode, or a supported
    /// mode whose plane count `resolve_gray` refuses.
    ///
    /// Note the asymmetry this name might otherwise hide: a record damaged
    /// badly enough that its own header will not parse is NOT reported as
    /// `Undecodable`, because such bytes are indistinguishable from the
    /// non-record trailing content real packs routinely carry.
    Undecodable,
}

/// A `patt` record the parser could not turn into an [`AbrPattern`], so any
/// brush referencing it gets no grain image while its texture params still
/// map. A consumer joins on `id` to tell that case apart from a tile that was
/// never embedded in the first place.
///
/// `id` and `mode` are plain values, not `Option`s: a detail exists only when
/// the record's header parsed, and the header yields both or neither.
#[derive(Debug, Clone)]
pub struct DroppedPatternDetail {
    /// Ordinal of the chunk within the `patt` block's walk (0-based), offset by
    /// the records of any earlier `patt` block in the same file.
    pub record_index: usize,
    /// The record's `pattern_id`.
    pub id: String,
    /// PSD image mode from the header.
    pub mode: u32,
    pub reason: DroppedPatternReason,
}

/// A chunk inside a `patt` block whose header did not parse, so it is not a
/// pattern record we can name — distinct from a `DroppedPatternDetail`, which
/// is a record we identified and failed to decode.
#[derive(Debug, Clone)]
pub struct UnreadablePattChunk {
    /// Ordinal of the `patt` block within the file (0-based), so a file with
    /// more than one block stays unambiguous.
    pub block_index: usize,
    /// Byte offset of the chunk's length prefix within that block's data.
    pub offset: usize,
    /// The `pattern_length` the chunk declares, i.e. its size after the prefix.
    pub declared_length: usize,
}

/// A preset that yields no output at all: it declares neither a sampled tip
/// (`sampledData` uuid) nor computed geometry, so it surfaces as neither a
/// brush nor a computed preset. Witnessed population: bristle/erodible/
/// airbrush presets carrying the `Shp ` tip descriptor.
#[derive(Debug, Clone)]
pub struct UnsupportedTipPresetDetail {
    pub name: String,
    /// Ordinal in the desc-block preset list (aligned with `preset_index`
    /// elsewhere).
    pub preset_index: usize,
    /// True when the preset carries the `Shp ` dynamic-tip descriptor —
    /// distinguishes "bristle/erodible/airbrush tip" from a degenerate
    /// preset with no tip data at all.
    pub has_shape_tip: bool,
    /// The tip family, when the inner `Brsh` class id names one. `None` on a
    /// preset with no `Shp ` at all, and also on the defensive case of a
    /// `Shp ` under an unrecognised class — callers fall back to the generic
    /// wording rather than guessing. Invariant over the corpus:
    /// `shape_tip_family.is_some() == has_shape_tip`.
    pub shape_tip_family: Option<ShapeTipFamily>,
    /// The tip shape, when the `(class id, `Shp ` index)` pair is in the
    /// measured table. `None` on a preset with no
    /// `Shp ` at all, on a `Shp ` whose type tag is not `long` — the flag stays
    /// tag-agnostic on purpose — and on an index the table does not cover.
    /// Callers fall back to the family wording rather than guess.
    pub tip_shape: Option<TipShape>,
}

/// A computed (procedural) brush preset — one that declares tip geometry but no
/// sampled bitmap. The witnessed geometry lives in `descriptor.computed`.
#[derive(Debug, Clone)]
pub struct ComputedPreset {
    /// Preset name from the descriptor (`Nm  `), or empty if absent.
    pub name: String,
    /// The parsed descriptor; `descriptor.computed` is `Some`.
    pub descriptor: BrushDescriptor,
    /// This preset's ordinal in the desc-block preset list, aligned with
    /// `dump::PresetDump.index`; `None` when no descriptor preset backs it.
    pub preset_index: Option<usize>,
}

/// ABR file format version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbrVersion {
    /// Legacy entry stream (Photoshop 6 and older): sampled tips with no names and no descriptors.
    V1,
    /// Legacy entry stream (Photoshop 7 to CS): the v1 layout plus a Unicode name per entry.
    V2,
    /// 8BIM block format (Photoshop CS+).
    V6,
    /// 8BIM block format, same block and entry layout as v9.
    V7,
    /// 8BIM block format, same block and entry layout as v6/v10.
    V9,
    /// 8BIM block format (Photoshop CS6+).
    V10,
}

impl std::fmt::Display for AbrVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AbrVersion::V1 => write!(f, "v1"),
            AbrVersion::V2 => write!(f, "v2"),
            AbrVersion::V6 => write!(f, "v6"),
            AbrVersion::V7 => write!(f, "v7"),
            AbrVersion::V9 => write!(f, "v9"),
            AbrVersion::V10 => write!(f, "v10"),
        }
    }
}

/// Procedural brush-tip geometry witnessed on `computedBrush` presets: the
/// `Dmtr`/`Hrdn`/`Angl`/`Rndn` descriptor keys.
///
/// Each field is `Option`: the parser reports exactly what the file carries and
/// performs no validation or defaulting — that is a consumer's job.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComputedGeometry {
    /// Photoshop `Dmtr` — tip diameter in pixels (`#Pxl`).
    pub diameter_px: Option<f64>,
    /// Photoshop `Hrdn` — hardness as a percentage (`#Prc`).
    pub hardness_pct: Option<f64>,
    /// Photoshop `Angl` — tip angle in degrees (`#Ang`).
    pub angle_deg: Option<f64>,
    /// Photoshop `Rndn` — roundness as a percentage (`#Prc`).
    pub roundness_pct: Option<f64>,
}

/// The `dualBrush` section of a preset: Photoshop's SECONDARY tip plus the
/// dual-side scatter block that drives it. Surfaced whole so a consumer can
/// carry it into Procreate as a nested `Sub01` sub-brush.
///
/// Field names mirror the [`BrushDescriptor`] fields they feed, because the
/// converter assembles a synthetic `BrushDescriptor` from this struct and runs
/// it through the SAME `map_params` path a main brush uses — there is no second
/// mapper. Each field is `Option`/`false` when the file omits the key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DualBrush {
    /// `dualBrush/Brsh/sampledData` — the secondary tip's samp uuid. `None` for
    /// a COMPUTED secondary tip (80 of the 806 enabled duals in the 2026-09-04
    /// corpus scan); then `computed` carries the geometry instead.
    pub uuid: Option<String>,
    /// `dualBrush/Brsh/Spcn` (`UntF #Prc`) — the secondary TIP's spacing, which
    /// maps to `plotSpacing`. NOT `dualBrush/Spcn` (see `scatter_spacing_pct`).
    pub spacing_pct: Option<f64>,
    /// `Some` iff the inner `Brsh` class id is `computedBrush` — the secondary
    /// tip is procedural and the converter synthesizes its bitmap.
    pub computed: Option<ComputedGeometry>,
    /// `dualBrush/Brsh/Dmtr` (`UntF #Pxl`) — secondary tip diameter, read for
    /// both sampled and computed inner classes. Drives the sub-brush's
    /// `max_size`, normalized against the pack's MAIN tips only.
    pub diameter_px: Option<f64>,
    /// `dualBrush/Brsh:sampledBrush/Angl` (`#Ang`) — sampled secondary rotation.
    pub shape_angle_deg: Option<f64>,
    /// `dualBrush/Brsh:sampledBrush/Rndn` (`#Prc`) — sampled secondary roundness.
    pub shape_roundness_pct: Option<f64>,
    /// `dualBrush/Brsh:sampledBrush/flipX` (`bool`) — static secondary tip flip.
    pub tip_flip_x: Option<bool>,
    /// `dualBrush/Brsh:sampledBrush/flipY` (`bool`) — static secondary tip flip.
    pub tip_flip_y: Option<bool>,
    /// `dualBrush/useScatter` (`bool`) — gate for the four scatter fields below,
    /// exactly as the top-level `useScatter` gates the main brush's.
    pub use_scatter: bool,
    /// `dualBrush/Cnt ` (`doub`) — dual scatter count, gated by `use_scatter`.
    pub scatter_count: Option<f64>,
    /// `dualBrush/bothAxes` (`bool`) — gated by `use_scatter`.
    pub scatter_both_axes: Option<bool>,
    /// `dualBrush/scatterDynamics:brVr/jitter` (`#Prc`) — gated by `use_scatter`.
    pub scatter_amount_pct: Option<f64>,
    /// `dualBrush/countDynamics:brVr/jitter` (`#Prc`) — gated by `use_scatter`.
    pub count_jitter_pct: Option<f64>,
    /// `dualBrush/Spcn` (`UntF #Prc`) — the dual-side SCATTER-mode spacing, the
    /// twin of the top-level `Spcn` trap (two distinct `Spcn` keys). Read so
    /// the trap is visible in the data, and
    /// deliberately NOT mapped — `spacing_pct` is the one that becomes
    /// `plotSpacing`.
    pub scatter_spacing_pct: Option<f64>,
    /// `dualBrush/BlnM` (`enum`) — how PS composites the secondary tip over the
    /// primary. Carried onto the parent's `dualBlendMode` through the
    /// device-measured dictionary.
    pub blend_mode: Option<String>,
    /// `dualBrush/Flip` (`bool`) — PS "Flip" for the secondary tip. Carried onto
    /// the `Sub01` sub-brush's `shapeFlipXJitter`/`shapeFlipYJitter` pair, where
    /// Procreate's own importer puts it.
    pub flip: Option<bool>,
}

/// Per-brush Photoshop dynamics extracted from the ABR descriptor.
///
/// Each field is `Option`: `None` means the source `.abr` carried no value for
/// that parameter, so a consumer's mapper leaves the corresponding output
/// parameter at its own default — output changes only where source data
/// exists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrushDescriptor {
    /// Photoshop `Spcn` — spacing as a percentage of tip diameter (e.g. `25.0`
    /// = 25 %). Maps to Procreate `plotSpacing`.
    pub spacing_pct: Option<f64>,
    /// `Some` iff the inner `Brsh` object's class id is `computedBrush` — i.e.
    /// the preset is procedural, not sampled. Present even when all four
    /// geometry fields resolve to `None`. The parser only surfaces this as
    /// data; synthesis into a tip bitmap happens in a consumer.
    pub computed: Option<ComputedGeometry>,
    /// Photoshop `Dmtr` (`UntF #Pxl`) — tip diameter in px (the tip INK EXTENT,
    /// not the selection bbox). Present for both sampled and computed presets.
    /// Drives the within-pack relative `maxSize` normalization;
    /// computed synthesis keeps reading `computed.diameter_px`.
    pub diameter_px: Option<f64>,
    /// Photoshop `scatterDynamics:brVr/jitter` (`#Prc`) — scatter amount, present
    /// only when `useScatter` is true. Maps to Procreate `shapeScatter`.
    pub scatter_amount_pct: Option<f64>,
    /// Photoshop `szVr:brVr/jitter` (`#Prc`) — size-jitter amount, present only
    /// when `useTipDynamics` is true. Maps to Procreate `dynamicsJitterSize`.
    pub size_jitter_pct: Option<f64>,
    /// Photoshop `angleDynamics:brVr/bVTy` (numeric `long` control selector),
    /// present only when `useTipDynamics` is true. 6 = Direction drives
    /// Procreate `shapeRotation`; 3 = Pen Tilt drives `shapeAzimuth` plus the
    /// −π/2 convention offset.
    pub angle_control: Option<i32>,
    /// Photoshop `angleDynamics:brVr/jitter` (`#Prc`) — angle-jitter amount,
    /// present only when `useTipDynamics` is true. Drives Procreate
    /// `shapeScatter` (per-dab rotation, `2·jitter%/100` clamped to [0, 1]);
    /// `shapeRandomise` randomises once per stroke and is
    /// deliberately not written from this key.
    pub angle_jitter_pct: Option<f64>,
    /// Photoshop `szVr:brVr/bVTy` (numeric `long` control selector; 2 = Pen
    /// Pressure), present only when `useTipDynamics` is true. Drives Procreate
    /// `dynamicsPressureSize` (1.0 iff control == Pen Pressure).
    pub pressure_size_control: Option<i32>,
    /// Photoshop `opVr:brVr/bVTy` (numeric `long` control selector; 2 = Pen
    /// Pressure), present only when `usePaintDynamics` is true. Drives Procreate
    /// `dynamicsPressureOpacityTransfer` (1.0 iff control == Pen Pressure).
    pub pressure_opacity_control: Option<i32>,
    /// Photoshop `opVr:brVr/jitter` (`#Prc`) — opacity-jitter amount, present only
    /// when `usePaintDynamics` is true. Maps to Procreate `dynamicsJitterOpacity`.
    pub opacity_jitter_pct: Option<f64>,
    /// Photoshop `prVr:brVr/jitter` (`#Prc`) — PS Transfer-panel Flow Jitter amount,
    /// present only when `usePaintDynamics` is true. Disclosed, not mapped: Procreate
    /// has no flow-jitter target.
    pub flow_jitter_pct: Option<f64>,
    /// Photoshop `wtVr:brVr/jitter` (`#Prc`) — PS Transfer-panel Wetness Jitter
    /// amount, present only when `usePaintDynamics` is true. Maps to Procreate
    /// `dynamicsWetnessJitter` (via `pct / 100`).
    pub wetness_jitter_pct: Option<f64>,
    /// Photoshop `mxVr:brVr/jitter` (`#Prc`) — PS Transfer-panel Mix Jitter amount,
    /// present only when `usePaintDynamics` is true. Disclosed, not mapped: Procreate
    /// has no mix-jitter target.
    pub mix_jitter_pct: Option<f64>,
    /// Photoshop `Brsh:sampledBrush/Angl` (`#Ang`, degrees) — sampled-tip rotation.
    /// Distinct from `ComputedGeometry::angle_deg` (which drives computed-tip
    /// synthesis). Maps to Procreate `shapeAngle` (static tip angle;
    /// `shapeRotation` is the follow-stroke fraction).
    pub shape_angle_deg: Option<f64>,
    /// Photoshop `Brsh:sampledBrush/Rndn` (`#Prc`) — sampled-tip roundness percent.
    /// Distinct from `ComputedGeometry::roundness_pct`. Maps to Procreate
    /// `shapeRoundness` via `Rndn / 100`.
    pub shape_roundness_pct: Option<f64>,
    /// Photoshop top-level `Cnt ` (`doub`) — scatter count (dabs per stamp),
    /// present only when `useScatter` is true. Maps to Procreate `shapeCount`.
    pub scatter_count: Option<f64>,
    /// Photoshop top-level `bothAxes` (`bool`) — scatter both-axes flag, present
    /// only when `useScatter` is true. Extracted but intentionally NOT mapped:
    /// no confirmed Procreate axis target.
    pub scatter_both_axes: Option<bool>,
    /// Photoshop `countDynamics:brVr/jitter` (`#Prc`) — scatter count-jitter
    /// amount, present only when `useScatter` is true. Maps to Procreate
    /// `shapeCountJitter`.
    pub count_jitter_pct: Option<f64>,
    /// Photoshop top-level `minimumDiameter` (`UntF` `#Prc`) — Shape Dynamics
    /// "Minimum Diameter", the floor a pressure-driven tip shrinks to at zero
    /// pressure. Witnessed 1:1 with the UI percent (corpus e13-minimum40, UI 40%
    /// → 40.0). Present only when `useTipDynamics` is true (the brVr `Mnm`
    /// sub-key is vestigial in PS 27.8). Maps to Procreate `minSize`.
    pub minimum_diameter_pct: Option<f64>,
    /// Photoshop `roundnessDynamics:brVr/jitter` (`#Prc`) — Shape Dynamics
    /// Roundness Jitter. Present only when `useTipDynamics` is true; corpus
    /// control (`bVTy`) is always 0 (pure jitter). Maps to Procreate
    /// `jitterShapeRoundness`.
    pub roundness_jitter_pct: Option<f64>,
    /// Photoshop top-level `minimumRoundness` (`#Prc`) — floor of the roundness
    /// jitter range. Present only when `useTipDynamics` is true. Extracted, not
    /// mapped: Procreate's roundness floors are pressure/tilt-scoped.
    pub minimum_roundness_pct: Option<f64>,
    /// Photoshop top-level `flipX` (`bool`) — Shape Dynamics Flip X Jitter. Present
    /// only when `useTipDynamics` is true. Maps to Procreate `shapeFlipXJitter`.
    pub flip_x_jitter: Option<bool>,
    /// Photoshop top-level `flipY` (`bool`) — Shape Dynamics Flip Y Jitter. Present
    /// only when `useTipDynamics` is true. Maps to Procreate `shapeFlipYJitter`.
    pub flip_y_jitter: Option<bool>,
    /// Photoshop `Brsh:sampledBrush/flipX` (`bool`) — static tip flip. No Procreate
    /// parameter; baked into Shape.png by mirroring the tip bitmap in the converter.
    pub tip_flip_x: Option<bool>,
    /// Photoshop `Brsh:sampledBrush/flipY` (`bool`) — static tip flip. No Procreate
    /// parameter; baked into Shape.png by mirroring the tip bitmap in the converter.
    pub tip_flip_y: Option<bool>,
    /// Photoshop `Txtr:Ptrn/Idnt` (`TEXT`) — the texture pattern UUID, keyed to
    /// a `patt` record. Present only when `useTexture` is true. Drives grain
    /// resolution in a consumer.
    pub texture_pattern_id: Option<String>,
    /// Photoshop `Txtr:Ptrn/Nm  ` (`TEXT`) — the texture pattern display name.
    /// Present only when `useTexture` is true. Diagnostic only.
    pub texture_pattern_name: Option<String>,
    /// Photoshop top-level `textureDepth` (`UntF` `#Prc`) — texture depth percent.
    /// Present only when `useTexture` is true. Drives grain depth.
    pub texture_depth_pct: Option<f64>,
    /// Photoshop top-level `minimumDepth` (`UntF` `#Prc`) — minimum texture depth
    /// percent. Present only when `useTexture` is true. Feeds grain resolution.
    pub texture_minimum_depth_pct: Option<f64>,
    /// Photoshop top-level `textureScale` (`UntF` `#Prc`) — texture scale percent.
    /// Present only when `useTexture` is true. Feeds grain resolution.
    pub texture_scale_pct: Option<f64>,
    /// Photoshop top-level `InvT` (`bool`) — invert-texture flag. Present only
    /// when `useTexture` is true. Feeds grain resolution.
    pub texture_invert: Option<bool>,
    /// Photoshop top-level `TxtC` (`bool`) — "Texture Each Tip". Present only
    /// when `useTexture` is true. `Option` is load-bearing: `false` is the value
    /// that selects Procreate's Texturised grain regime, so an absent key must
    /// stay absent rather than collapse to `false`.
    pub texture_each_tip: Option<bool>,
    /// Photoshop top-level `textureBlendMode` (`enum` `BlnM`) — the raw blend-mode
    /// value id (e.g. "height"). Present only when `useTexture` is true.
    /// Extracted-but-unmapped: no confirmed Procreate target yet.
    pub texture_blend_mode: Option<String>,
    /// Photoshop `textureDepthDynamics:brVr/jitter` (`#Prc`) — texture depth-jitter
    /// amount. Present only when `useTexture` is true. Feeds grain resolution.
    pub texture_depth_jitter_pct: Option<f64>,
    /// Photoshop top-level `textureBrightness` (`long`) — texture brightness.
    /// Present only when `useTexture` is true. Extracted-but-unmapped until a
    /// calibration sitting measures it.
    pub texture_brightness: Option<i64>,
    /// Photoshop top-level `textureContrast` (`long`) — texture contrast. Present
    /// only when `useTexture` is true. Extracted-but-unmapped until a
    /// calibration sitting measures it.
    pub texture_contrast: Option<i64>,
    /// Photoshop `dualBrush > useDualBrush` (`bool`) — true iff the preset carries
    /// an enabled dual-brush (secondary tip) section, even when the tip is COMPUTED
    /// (no sampledData uuid). Drops with no Procreate equivalent; surfaced only for
    /// the per-brush "notConverted" disclosure.
    pub use_dual_brush: bool,
    /// The whole `dualBrush` section, `Some` iff `use_dual_brush` is true — the
    /// data a consumer turns into a nested `Sub01` sub-brush.
    /// A dual whose secondary tip cannot be resolved (dangling samp uuid, or
    /// geometry too degenerate to synthesize) still lands here; the converter
    /// then emits no `Sub01` and the old "secondary tip dropped" disclosure
    /// stands.
    pub dual: Option<DualBrush>,
    /// Photoshop `Wtdg` (`bool`) — Wet Edges flag. Disclose-only, not
    /// converted.
    pub wet_edges: bool,
    /// Photoshop `Nose` (`bool`) — Noise flag. Disclose-only, not converted.
    pub noise: bool,
    /// Photoshop `Rpt ` (`bool`, note trailing space) — Build-up flag.
    /// Disclose-only, not converted.
    pub buildup: bool,
    /// Photoshop `useColorDynamics` (`bool`) — Color Dynamics panel enable.
    /// Surfaced for the per-brush "notConverted" disclosure and as the gate for
    /// the six `color_*` sub-values below.
    pub use_color_dynamics: bool,
    /// Photoshop `H   ` (`UntF` `#Prc`, key = "H" + three spaces) — Color Dynamics
    /// Hue Jitter percent. Present only when `useColorDynamics` is true. Maps to
    /// Procreate `dynamicsJitterHue`/`dynamicsJitterStrokeHue` (e22 witness).
    pub color_hue_jitter_pct: Option<f64>,
    /// Photoshop `Strt` (`UntF` `#Prc`) — Color Dynamics Saturation Jitter percent.
    /// Present only when `useColorDynamics` is true. Maps to Procreate
    /// `dynamicsJitterSaturation`/`...StrokeSaturation` (e22 witness).
    pub color_saturation_jitter_pct: Option<f64>,
    /// Photoshop `Brgh` (`UntF` `#Prc`) — Color Dynamics Brightness Jitter percent.
    /// Present only when `useColorDynamics` is true. Maps to Procreate
    /// `dynamicsJitterLightness`/`...StrokeLightness` (e22 witness).
    pub color_brightness_jitter_pct: Option<f64>,
    /// Photoshop `purity` (`UntF` `#Prc`) — Color Dynamics Purity. Present only
    /// when `useColorDynamics` is true. NO Procreate counterpart; surfaced for the
    /// notConverted residue disclosure only.
    pub color_purity_pct: Option<f64>,
    /// Photoshop `clVr` (`Objc` class `brVr`, `jitter` `#Prc`) — Color Dynamics
    /// Foreground/Background Jitter percent. Present only when `useColorDynamics`
    /// is true. NO Procreate counterpart; surfaced for the notConverted residue
    /// disclosure only.
    pub color_fg_bg_jitter_pct: Option<f64>,
    /// Photoshop `colorDynamicsPerTip` (`bool`) — "Apply Per Tip". Present only
    /// when `useColorDynamics` is true. Routes the three color jitters to the
    /// per-tip (`true`, e22-witnessed) vs per-stroke (`false`, e23-witnessed)
    /// Procreate keys.
    pub color_dynamics_per_tip: Option<bool>,
    /// Photoshop `useBrushPose` (`bool`) — Brush Pose flag. Disclose-only, not
    /// converted.
    pub use_brush_pose: bool,
    /// Photoshop top-level `brushProjection` (`bool`) — tip projection follows
    /// stylus tilt/rotation. No Procreate equivalent; surfaced via `not_converted`.
    pub brush_projection: bool,
}

/// A single brush extracted from the ABR pack.
#[derive(Debug, Clone)]
pub struct AbrBrush {
    /// Unique identifier — from descriptor UUID or synthesized from index.
    pub id: String,
    /// Brush name from Unicode descriptor, or empty if not found.
    pub name: String,
    /// The brush tip bitmap (decompressed grayscale).
    pub tip: TipBitmap,
    /// Per-brush dynamics extracted from the descriptor.
    /// Empty (`BrushDescriptor::default()`) when the pack carries no descriptor
    /// for this brush (e.g. ABR v2, or a sparse/malformed descriptor).
    pub descriptor: BrushDescriptor,
    /// `false` for every parser-emitted (sampled) brush. The converter sets it
    /// `true` for tips it synthesizes from a `computedBrush` preset.
    pub synthesized: bool,
    /// This brush's ordinal in the desc-block preset list, aligned with
    /// `dump::PresetDump.index`; `None` when no descriptor preset backs the
    /// brush (v2 packs, or the index-fallback path with no matched desc info).
    pub preset_index: Option<usize>,
}

/// Decompressed brush tip bitmap.
#[derive(Debug, Clone)]
pub struct TipBitmap {
    pub width: u32,
    pub height: u32,
    /// Bit depth: 8 or 16 (grayscale).
    pub depth: u8,
    /// Decompressed pixel data, row-major, grayscale.
    /// For 16-bit depth, values are stored as big-endian u16 pairs.
    pub data: Vec<u8>,
}

/// Errors that can occur during ABR parsing.
#[derive(thiserror::Error, Debug)]
pub enum AbrError {
    /// Carries the raw major version read from the header, so a version this
    /// crate has no `AbrVersion` variant for can still be named in the message.
    ///
    /// The message claims only what the parser has actually seen — a header
    /// that read cleanly and a plausible version number — and not that the
    /// rest of the file is sound.
    #[error("unsupported ABR version {0}: the header reads fine, this version is not supported")]
    UnsupportedVersion(u16),

    #[error("invalid or truncated ABR file header")]
    InvalidHeader,

    #[error("malformed block at offset {offset}: {reason}")]
    MalformedBlock { offset: u64, reason: String },

    #[error("decompression failed: {0}")]
    Decompression(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
