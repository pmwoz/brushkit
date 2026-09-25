# First available tips

A consumer asks for the first `n` available tips of a file, for example to draw
a thumbnail under a time budget. The result is the same entries the full
preview returns, filtered to available ones and cut after `n`, each keeping its
full-preview index.

## Sub-features

- `first-count` returns at most `n` entries, all available.
- `first-index` keeps each entry's index from the full preview, so indices may skip.
- `first-zero` with `n` 0 returns no entries.
- `first-parity` returns bitmaps identical to the matching full-preview entries.

## How to get to it (user POV)

- `brushkit::preview::preview_abr_first_available(&bytes, opts, n)`.
- `brushkit::preview::preview_brushset_first_available(&bytes, opts, n)`.
- `brushkit::preview::preview_brush_first_available(&bytes, opts, n)`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.

- **Real pack.** Run `$H run first3 preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr" --first 3`. Exit `0`, `.result.counts` is `{"available":3,"entries":3,"unavailable":0}` for a pack with 3 or more available tips.
- **Parity.** Run `$H run full preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr"` with the same file. Every entry in `first3` equals the entry with the same `index` in `full`, and `cmp` of the matching `tips/NNN.png` files succeeds.
- **Index skip.** Run `$H run seed-first preview fuzz/corpus/preview_abr/zero_area_tip --kind abr --first 1`. The only entry is `{"index":1,"name":"brush_0"}`, because entry 0 is unavailable.
- **Real set.** Run `$H run first-set preview "$BRUSHKIT_CORPUS_DIR/<set>.brushset" --first 2`. Two entries, both available, matching the first two available entries of the full set preview.
- **Zero.** Run `$H run first0 preview fuzz/corpus/preview_brush/corrupt_shape --kind brush --first 0`. Exit `0` with `.result.counts.entries` `0`.

## Gotchas

- A broken `.brush` with `--first 1` also returns zero entries, so an empty result alone does not tell `n` 0 from an unavailable tip.
- The probe does not time the call. A claim that later entries are not built needs the tests in `crates/preview/tests/first_available.rs`, not the probe.
