# Contact sheets and tip PNGs

A consumer renders previewed tips as PNGs: a black-ink thumbnail per tip of at
most 200 px per side, and a contact sheet that lays out many tips in a grid
with optional name and size labels. The contact sheet needs the default `text`
feature.

## Sub-features

- `sheet-grid` lays out every given tip, up to 10 per row by default, in 120 px cells with 8 px margins, dark ink on opaque white.
- `sheet-labels` draws the brush name and the ink-bounds size under each tip.
- `sheet-empty` encodes no tips as a 1 x 1 transparent PNG.
- `tip-png` encodes one tip as a black RGBA PNG with alpha from the gray value, downsampled to at most 200 px per side.

## How to get to it (user POV)

- `brushkit::preview::generate_contact_sheet_png(&[SheetBrush { name, bitmap }], &ContactSheetConfig { .. })`.
- `brushkit::preview::generate_preview_png(&bitmap)`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.
- The probe draws the sheet with `show_names: true` and the other `ContactSheetConfig` defaults, and calls `generate_preview_png` for each available tip. Tip files are named by entry index.

- **Real pack sheet.** Run `$H run sheet preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr"` on a pack with more than 10 available tips. Open `/tmp/verify-brushkit/$RUN_ID/sheet/sheet.png` with the Read tool. It shows `.result.counts.available` tips in rows of 10, each with its name and a `{w}×{h}` size line, ink dark on white.
- **Dimensions.** Run `file /tmp/verify-brushkit/$RUN_ID/sheet/sheet.png`. It reports `PNG image data`, `8-bit gray+alpha`, and `8 + 128·min(N,10)` x `8 + 128·ceil(N/10)` for N available tips, for example `1288 x 776` for 54.
- **Per-tip PNG.** `file sheet/tips/*.png` reports `8-bit/color RGBA` for each, at the `width` x `height` of its entry's `.tip.available`. Open the file `jq -r '[.result.entries[].tip.available.png // empty][0]'` prints with the Read tool. It shows the first available tip as black ink on a transparent background.
- **200 px cap.** Run `$H run sheet-cap preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr" --max-cell 512` on a pack whose tips are larger than 200 px. Every `tips/*.png` has a longer side of at most 200 at the entry's aspect ratio, for example 512x391 becomes 200x153.
- **Seed sheet.** Run `$H run sheet-seed preview fuzz/corpus/preview_brushset/ordered_set --kind brushset`. `sheet.png` is `264 x 136` with two dark 16×8 blocks labeled `B` and `A`. `ls tips` gives `000.png` and `001.png`.
- **Skipped entries.** Run `$H run sheet-skip preview fuzz/corpus/preview_abr/zero_area_tip --kind abr`. `tips/` holds only `001.png`, and `sheet.png` is `136 x 136` with one cell labeled `brush_0`.
- **Empty sheet.** Run `$H run sheet-empty preview fuzz/corpus/preview_brush/corrupt_shape --kind brush`. `tips/` is empty and `file .../sheet-empty/sheet.png` reports a 1 x 1 PNG, because no tip is available.

## Gotchas

- The size label is the tight bounds of nonzero ink in the bitmap after `max_cell`, so it is at most the `width` and `height` in `summary.json` and never the source size.
- `generate_preview_png` downsamples to 200 px on its own, independent of `--max-cell`. At the default `--max-cell 128` every tip PNG keeps its entry's size.
- `tips/000.png` exists only when entry 0 is available.
- Transparent PNGs can look blank in some viewers. Judge the Read tool output, and check the alpha range with a script if in doubt. A non-empty sheet is opaque.
- The probe's sheet config is fixed, so `columns`, `cell_size`, `padding`, `gap`, `show_sizes: false` and other backgrounds are not reachable.
- A run that exits `1` writes no `sheet.png` or `tips/`, and the helper does not clear a label's directory. A reused label can show PNGs from an earlier run.
