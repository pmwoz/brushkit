# Parse an .abr pack

A consumer parses a Photoshop `.abr` pack into its version, sampled brushes
with names and tip bitmaps, computed presets, embedded patterns and a report of
what was dropped and why. Malformed input returns an error, never a panic.

## Sub-features

- `parse-eager` decodes every tip up front with `parse_abr`.
- `parse-deferred` keeps tips as byte ranges and decodes them through `decode_tip`.
- `parse-no-patterns` skips pattern payloads.
- `parse-all-deferred` decodes no tip up front.
- `parse-patterns` returns embedded patterns and counts dropped ones.
- `parse-diagnostics` reports dropped samp tips, skipped and unsupported presets and desc parse errors.
- `parse-malformed` returns `Err` with a message for broken input.

## How to get to it (user POV)

- `brushkit::abr::parse_abr(&bytes)`.
- `brushkit::abr::parse_abr_deferred(&bytes)`, then `DeferredPack::decode_tip(i)`.
- `brushkit::abr::parse_abr_deferred_without_patterns(&bytes)`.
- `brushkit::abr::parse_abr_all_deferred_without_patterns(&bytes)`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.

- **Eager.** Run `$H run parse-eager parse "$BRUSHKIT_CORPUS_DIR/<pack>.abr"`. Exit `0`. `.result.pack.version` is `v6`, `v7` or `v10`, and every `.result.pack.brushes[].tip` has `width`, `height`, `depth` 8 or 16 and `data_len` equal to `width*height*depth/8`.
- **Mode parity.** Run the same file with `--mode deferred`, `--mode deferred-no-patterns` and `--mode all-deferred` under three labels. `.result.pack.brushes` is identical across all four runs.
- **Patterns.** Run `$H run seed-patt parse fuzz/corpus/preview_abr/wellformed_v6_patt`. `.result.pack.patterns` is one 2x2 pattern named `tex`, mode `2`. The same seed with `--mode deferred-no-patterns` returns an empty `patterns`.
- **Dropped pattern.** Run `$H run seed-dropped parse fuzz/corpus/preview_abr/patt_oversized_channel`. Exit `0` with `.result.pack.diagnostics.dropped_pattern_count` `1`.
- **Dual-brush drops.** Find a corpus pack with a nonzero `dropped_samp_count` by running `--mode all-deferred` over the corpus. At least one v7 pack has `5`. `.result.pack.brushes` still lists every user-facing brush.
- **Malformed.** Run `$H run seed-bad parse fuzz/corpus/preview_abr/tip_bitmap_dim_overflow`. Exit `1` with `.result.error` `"malformed block at offset 125: bitmap dimensions out of range"`. `v2_rle_overflow` and `v2_rle_underflow` give exit `1` with `samp entry length` errors.

## Gotchas

- `eager` and `deferred` read patterns, the other two modes skip them, so `patterns` is empty there by design.
- The corpus and the fuzz seeds have no computed presets. `computed_presets` stays empty unless the user supplies a pack with computed tips.
- Exit `101` means a panic inside the library, which is a bug on any input.
- Eager parsing of a 30 MB pack holds every decoded tip in memory. Prefer `all-deferred` for corpus sweeps.
