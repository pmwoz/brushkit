# First available tips

A consumer asks for the first `n` available tips of a file, for example to draw
a few thumbnails without decoding the whole pack. The result is the same
entries the full preview returns, filtered to available ones and cut after `n`,
each keeping its full-preview index. No entry after the `n`th available one is
built.

## Sub-features

- `first-count` returns at most `n` entries, all available.
- `first-index` keeps each entry's index from the full preview, so indices may skip.
- `first-zero` with `n` 0 returns no entries, but still parses or opens the file, so a broken file is still exit `1`.
- `first-parity` returns bitmaps and a set name identical to the full preview.

## How to get to it (user POV)

- `brushkit::preview::preview_abr_first_available(&bytes, opts, n)`.
- `brushkit::preview::preview_brushset_first_available(&bytes, opts, n)`.
- `brushkit::preview::preview_brush_first_available(&bytes, opts, n)`.
- All three are `preview(&bytes, format, opts, Take::FirstAvailable(n), &mut || true)`. [Stopping a preview](./stop.md) covers `preview` itself.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.

- **Real pack.** Run `$H run first3 preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr" --first 3`. Exit `0`, `.result.counts` is `{"available":3,"entries":3,"unavailable":0}` for a pack with 3 or more available tips.
- **Parity.** Run `$H run full preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr"` with the same file. `jq -n --slurpfile a full/summary.json --slurpfile f first3/summary.json '[$a[0].result.entries[] | select(.tip.available)][:3] == $f[0].result.entries'` is `true`, and `cmp` of the matching `tips/NNN.png` files succeeds.
- **Index skip.** Run `$H run seed-first preview fuzz/corpus/preview_abr/zero_area_tip --kind abr --first 1`. `[.result.entries[] | {index,name}]` is `[{"index":1,"name":"brush_0"}]`, because entry 0 is unavailable.
- **Real set.** Run `$H run full-set preview "$BRUSHKIT_CORPUS_DIR/<set>.brushset"` and `$H run first-set preview` on the same file with `--first 2`. `first-set` has two available entries equal to the first two available entries of `full-set`, and the same `set_name`.
- **Zero.** Run `$H run first0 preview fuzz/corpus/preview_brush/root_brush --kind brush --first 0` and the same with `--first 1` as `first1`. `first0` has `.result.counts.entries` `0`. `first1` has one entry, `A` at index 0, available at 16x8.

## Gotchas

- A `.brush` whose only tip is unavailable, such as `corrupt_shape`, also returns zero entries with `--first 1`. Use `root_brush` to tell `n` 0 from an unavailable tip. A `.brush` that does not open is exit `1`, not zero entries.
- The probe does not count entry builds. The claim that later entries are not built is proven by the `record_read` unit tests in `crates/preview/src/lib.rs`, not by the probe or `crates/preview/tests/first_available.rs`, which compares results only.
- These functions have no time limit. A host that stops on a clock uses `preview` with a `keep_going` callback.
