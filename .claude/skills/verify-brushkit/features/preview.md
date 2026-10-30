# Tip previews

A consumer passes the bytes of an `.abr`, `.brush` or `.brushset` file and gets
one entry per brush, each either an available grayscale bitmap no larger than
`max_cell` on either side or an unavailable reason. No brush is dropped. An
`.abr` preview lists its sampled, computed and unsupported presets in preset
order. A pack without presets, such as v1 or v2, lists its brushes in reverse
file order.

## Sub-features

- `preview-abr` returns one entry per sampled, computed or unsupported preset of an `.abr` pack.
- `preview-computed` draws a computed preset as a square of side `synthesized_tip_side`, with `source_dimensions` null.
- `preview-brushset` returns every member of a `.brushset`, in `brushset.plist` order, with the set name.
- `preview-brushset-no-plist` falls back to zip order with no set name.
- `preview-brush` returns the single tip of a `.brush`.
- `preview-unavailable` reports a tip that cannot be drawn as an entry with one of `NoShapePng`, `UnsupportedTipKind`, `Corrupt`, `MissingTip`, `TooLarge` or `OverBudget`.
- `preview-budget` returns at most `MAX_PREVIEW_BYTES` (256 MiB) of bitmaps per call. Every entry with a tip after the first that does not fit is `OverBudget`.
- `preview-max-cell` bounds every bitmap side by `max_cell` and keeps a tip that already fits.
- `preview-source-dims` reports the original raster size independent of `max_cell`.

## How to get to it (user POV)

- `brushkit::preview::preview_abr(&bytes, PreviewOptions { max_cell })`.
- `brushkit::preview::preview_brushset(&bytes, opts)`.
- `brushkit::preview::preview_brush(&bytes, opts)`.
- `brushkit::preview::synthesized_tip_side(&geometry)`, the side of a computed tip without drawing it.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.
- `$BRUSHKIT_CORPUS_DIR` contains at least one `.abr` and one `.brushset`.

- **Real pack.** Run `$H run preview-abr preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr"`. Exit `0`. `[.result.entries[].index] == [range(.result.counts.entries)]`, and each entry's `tip` is either `available` or `unavailable`.
- **Real set.** Run `$H run preview-set preview "$BRUSHKIT_CORPUS_DIR/<set>.brushset"`. Exit `0`. `.result.set_name` is the set's display name and `.result.entries[].name` follows the set order.
- **Plist order.** Run `$H run seed-ordered preview fuzz/corpus/preview_brushset/ordered_set --kind brushset`. `.result.set_name` is `"Ordered"` and the names are `["B","A"]`, the plist order, both 16x8.
- **No plist.** Run `$H run seed-noplist preview fuzz/corpus/preview_brushset/no_metadata --kind brushset`. `.result.set_name` is `null` and the names are `["A","B"]` in zip order.
- **Available .brush.** Run `$H run seed-brush-ok preview fuzz/corpus/preview_brush/root_brush --kind brush`. Exit `0` with one entry named `A`, available at 16x8, `source_dimensions` 16x8.
- **Broken .brush tips.** Run each of these with `--kind brush`. All exit `0` with one entry:
  - `corrupt_shape`: `A` with `{"unavailable":"Corrupt(\"failed to decode Shape.png: ...\")"}` and `source_dimensions` null.
  - `missing_shape`: `A` with `"NoShapePng"`.
  - `oversized_png`: `"TooLarge { width: 60000, height: 60000 }"` with `source_dimensions` 60000x60000.
  - `deep_archive`: named `Brush` with `"Corrupt(\"Brush.archive: plist nesting depth exceeds limit\")"` and `source_dimensions` 16x8, because the tip size comes from `Shape.png`.
- **Mixed .abr rows.** Run `$H run seed-unavail preview fuzz/corpus/preview_abr/unavailable_tips --kind abr`. The names are `["B","A","Round","C"]`. `B` is `Corrupt("no bitmap header found")`, `A` is available at 4x4, `Round` is the computed preset available at 32x32 with `source_dimensions` null, and `C` is `MissingTip { uuid: "a1b2c3d4-e5f6-7890-abcd-ef1234567892" }`. A `parse` run of the same seed gives `.result.pack.computed_presets[0].synthesized_side` `32`.
- **Zero-area tip.** Run `$H run seed-zero preview fuzz/corpus/preview_abr/zero_area_tip --kind abr`. Entry 0 `brush_1` is `Corrupt("tip has zero area")` with `source_dimensions: null`. Entry 1 `brush_0` is available at 1x1.
- **max_cell bound.** Run `$H run cell32 preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr" --max-cell 32`. `jq '[.result.entries[].tip.available // empty | [.width,.height] | max] | max'` is at most `32`, and `source_dimensions` match the default run.
- **Downsample keeps aspect.** Run `$H run seed-cell4 preview fuzz/corpus/preview_abr/sampled_tip --kind abr --max-cell 4`. The 16x8 source comes back 4x2 with `source_dimensions` still 16x8.
- **Tip that fits.** Run `$H run seed-fits preview fuzz/corpus/preview_abr/small_tip --kind abr --max-cell 4`. The 4x4 source comes back 4x4.
- **Budget.** Run `$H run budget preview "$BRUSHKIT_CORPUS_DIR/<large pack>.abr" --max-cell 16384` on a pack of 150 MB or more. Some entries are `"OverBudget"`, `[.result.entries[].tip.available // empty | .width*.height] | add` is at most `268435456`, and no entry after the first `OverBudget` one is available.
- **Whole-file failures.** Each exits `1` with `.result.error`:
  - `sampled_tip --kind abr --max-cell 0`: `"max_cell must be at least 1"`.
  - `fuzz/corpus/preview_brush/root_brush --kind abr`: `"invalid or truncated ABR file header"`.
  - `fuzz/corpus/preview_abr/sampled_tip --kind brush`: starts with `"failed to open zip: "`.
  - `fuzz/corpus/preview_brushset/deep_metadata --kind brushset`: `"brushset.plist: plist nesting depth exceeds limit"`.
- **Proof.** Open `sheet.png` of the real-pack run with the Read tool and confirm the drawn tips match `.result.counts.available`.

## Gotchas

- The corpus holds no `.brush` files. `preview-brush` on a real file needs one from the user. The seeds cover the mechanics: `root_brush` and `filtered_shape` are available, the others are not.
- The corpus has v6, v7 and v10 packs only. v2 exists only as synthetic seeds such as `zero_area_tip`. No seed or corpus file reaches v1 or v9.
- The corpus has no computed or unsupported presets, so `UnsupportedTipKind` is not reached. `unavailable_tips` is the only computed row.
- An unavailable tip is `Ok` with an unavailable entry. Only a whole-file failure is exit `1`: `max_cell` 0, a bad `.abr` header or block, a zip that does not open, a `brushset.plist` that does not parse, a `.brushset` with no brushes, or a `.brush` without `Brush.archive`.
- A budget run holds up to 256 MiB of bitmaps and about 650 MB of memory.
- `sheet.png` holds only available tips, so its cell count can be lower than `.result.counts.entries`.
