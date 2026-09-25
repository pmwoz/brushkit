# Tip previews

A consumer passes the bytes of an `.abr`, `.brush` or `.brushset` file and gets
one entry per brush in file order, each either an available grayscale bitmap
no larger than `max_cell` on either side or an unavailable reason. No brush is
dropped.

## Sub-features

- `preview-abr` returns every sampled brush of an `.abr` pack.
- `preview-brushset` returns every member of a `.brushset`, in `brushset.plist` order, with the set name.
- `preview-brushset-no-plist` falls back to zip order with no set name.
- `preview-brush` returns the single tip of a `.brush`.
- `preview-unavailable` reports a broken tip as an unavailable entry instead of dropping it.
- `preview-max-cell` bounds every bitmap side by `max_cell` and keeps a tip that already fits.
- `preview-source-dims` reports the original raster size independent of `max_cell`.

## How to get to it (user POV)

- `brushkit::preview::preview_abr(&bytes, PreviewOptions { max_cell })`.
- `brushkit::preview::preview_brushset(&bytes, opts)`.
- `brushkit::preview::preview_brush(&bytes, opts)`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.
- `$BRUSHKIT_CORPUS_DIR` contains at least one `.abr` and one `.brushset`.

- **Real pack.** Run `$H run preview-abr preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr"`. Exit `0`. The indices in `.result.entries` run from 0 to `.result.counts.entries - 1` with no gap, and each entry is either `available` or `unavailable`.
- **Real set.** Run `$H run preview-set preview "$BRUSHKIT_CORPUS_DIR/<set>.brushset"`. Exit `0`. `.result.set_name` is the set's display name and `.result.entries[].name` follows the set order.
- **Plist order.** Run `$H run seed-ordered preview fuzz/corpus/preview_brushset/ordered_set --kind brushset`. `.result.set_name` is `"Ordered"` and the names are `["B","A"]`, the plist order, both 16x8.
- **No plist.** Run `$H run seed-noplist preview fuzz/corpus/preview_brushset/no_metadata --kind brushset`. `.result.set_name` is `null` and the names are `["A","B"]` in zip order.
- **Broken .brush tip.** Run `$H run seed-brush preview fuzz/corpus/preview_brush/corrupt_shape --kind brush`. Exit `0` with one entry named `A` whose tip is `{"unavailable":"Corrupt(\"failed to decode Shape.png: ...\")"}`.
- **Zero-area tip.** Run `$H run seed-zero preview fuzz/corpus/preview_abr/zero_area_tip --kind abr`. Entry 0 `brush_1` is `Corrupt("tip has zero area")` with `source_dimensions: null`. Entry 1 `brush_0` is available at 1x1.
- **max_cell bound.** Run `$H run cell32 preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr" --max-cell 32`. `jq '[.result.entries[].tip.available | [.width,.height] | max] | max'` is at most `32`, and `source_dimensions` match the default run.
- **Downsample keeps aspect.** Run `$H run seed-cell4 preview fuzz/corpus/preview_abr/sampled_tip --kind abr --max-cell 4`. The 16x8 source comes back 4x2 with `source_dimensions` still 16x8.
- **max_cell zero.** Run `$H run cell0 preview fuzz/corpus/preview_abr/sampled_tip --kind abr --max-cell 0`. Exit `1` with `.result.error` `"max_cell must be at least 1"`.
- **Proof.** Open `sheet.png` of the real-pack run with the Read tool and confirm the drawn tips match `.result.counts.available`.

## Gotchas

- The corpus holds no `.brush` files. `preview-brush` on a real file needs one from the user. The committed seed only proves the unavailable path.
- The corpus has v6, v7 and v10 packs only. v1, v2 and v9 paths exist only as synthetic seeds such as `zero_area_tip` (v2).
- An unavailable tip is `Ok` with an unavailable entry. Only a whole-file failure (bad zip, bad header, `max_cell` 0) is exit `1`.
- `sheet.png` holds only available tips, so its cell count can be lower than `.result.counts.entries`.
