# Contact sheets and tip PNGs

A consumer renders previewed tips as PNGs: a 200 px black-ink thumbnail per
tip, and a contact sheet that lays out many tips in a grid with optional name
and size labels. The contact sheet needs the default `text` feature.

## Sub-features

- `sheet-grid` lays out every given tip, up to 10 per row by default.
- `sheet-labels` draws the brush name and the ink-bounds size under each tip.
- `tip-png` encodes one tip as a black RGBA PNG with alpha from the gray value.

## How to get to it (user POV)

- `brushkit::preview::generate_contact_sheet_png(&[SheetBrush { name, bitmap }], &ContactSheetConfig { .. })`.
- `brushkit::preview::generate_preview_png(&bitmap)`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.
- The probe always calls both with `show_names: true` and the other `ContactSheetConfig` defaults.

- **Real pack sheet.** Run `$H run sheet preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr"`. Open `/tmp/verify-brushkit/$RUN_ID/sheet/sheet.png` with the Read tool. It shows `.result.counts.available` tips in rows of 10, each with its name and a `{w}×{h}` size line, ink dark on white.
- **Dimensions.** Run `file /tmp/verify-brushkit/$RUN_ID/sheet/sheet.png`. It reports `PNG image data` with `8-bit gray+alpha`.
- **Per-tip PNG.** Open `tips/000.png` from the same run with the Read tool. It shows the first tip as black ink on a transparent background.
- **Empty sheet.** Run `$H run sheet-empty preview fuzz/corpus/preview_brush/corrupt_shape --kind brush`. `file .../sheet-empty/sheet.png` reports a 1 x 1 PNG, because no tip is available.

## Gotchas

- The size label is the tight bounds of nonzero ink, not the bitmap size, so it can be smaller than the `width` and `height` in `summary.json`.
- `generate_preview_png` downsamples to 200 px on its own, independent of `--max-cell`.
- Transparent PNGs can look blank in some viewers. Judge the Read tool output, and check the alpha range with a script if in doubt.
