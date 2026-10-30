# Parse an .abr pack

A consumer parses a Photoshop `.abr` pack into its version, sampled brushes
with names and tip bitmaps, the unavailable brushes in their place among them,
computed presets, embedded patterns and a report of what was dropped and why.
Malformed input returns an error, never a panic.

## Sub-features

- `parse-eager` decodes every tip up front with `parse_abr`.
- `parse-deferred` keeps tips as byte ranges and decodes them through `decode_tip`, except v1/v2 tips, dual-brush tips, unused records and repeated uuids, which it decodes up front.
- `parse-up-front` reports the bytes `parse_abr_deferred` decodes up front through `up_front_decoded_len`, the same value in every deferred mode.
- `parse-no-patterns` skips pattern payloads.
- `parse-all-deferred` decodes no tip up front.
- `parse-unavailable` lists every sampled brush in `sampled_brushes`, a brush whose tip is missing or does not parse as an unavailable brush in its place.
- `parse-uuid` gives each brush and unavailable brush the samp record uuid it samples, `null` for v1/v2.
- `parse-patterns` returns embedded patterns and counts dropped ones.
- `parse-diagnostics` reports dropped samp tips, unreadable unused samp records, skipped and unsupported presets and desc parse errors.
- `parse-malformed` returns `Err` with a message for broken input.

## How to get to it (user POV)

- `brushkit::abr::parse_abr(&bytes)`.
- `brushkit::abr::parse_abr_deferred(&bytes)`, then `DeferredPack::decode_tip(i)` and `DeferredPack::up_front_decoded_len()`.
- `brushkit::abr::parse_abr_deferred_without_patterns(&bytes)`.
- `brushkit::abr::parse_abr_all_deferred_without_patterns(&bytes)`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.
- The probe prints `up_front_decoded_len` in `.result.pack` for the three deferred modes only.

- **Eager.** Run `$H run parse-eager parse "$BRUSHKIT_CORPUS_DIR/<pack>.abr"`. Exit `0`. `.result.pack.version` is `v6`, `v7` or `v10` for a corpus pack (the library also reads `v1`, `v2` and `v9`), every `.result.pack.brushes[].uuid` is a string, and every `.result.pack.brushes[].tip` has `width`, `height`, `depth` 8 or 16 and `data_len` equal to `width*height*depth/8`.
- **Mode parity.** Run the same file with `--mode deferred`, `--mode deferred-no-patterns` and `--mode all-deferred` under three labels. `.result.pack.brushes` is identical across all four runs.
- **Unavailable brushes.** Run `$H run seed-unavail parse fuzz/corpus/preview_abr/unavailable_tips`. Exit `0`. `.result.pack.brushes` holds only `A` at `preset_index` `1` with `uuid` `a1b2c3d4-e5f6-7890-abcd-ef1234567890`, and `.result.pack.computed_presets` holds `Round` at `2` with `diameter_px` `30`. `.result.pack.sampled_brushes` is `B` at `preset_index` `0` with `.unavailable.cause` `{"unreadable":"no bitmap header found"}`, then `{"readable":0}`, then `C` at `preset_index` `3` with `.unavailable.cause` `{"missing":{"uuid":...}}` and the same uuid as its `id` and `uuid`. The other three modes give the same `sampled_brushes`, because `B` fails in its header.
- **Patterns.** Run `$H run seed-patt parse fuzz/corpus/preview_abr/wellformed_v6_patt`. `.result.pack.patterns` is one 2x2 pattern named `tex`, mode `2`. The same seed with `--mode deferred-no-patterns` returns an empty `patterns`.
- **Dropped pattern.** Run `$H run seed-dropped parse fuzz/corpus/preview_abr/patt_oversized_channel`. Exit `0` with `.result.pack.diagnostics.dropped_pattern_count` `1`.
- **Dual-brush drops.** Find a corpus pack with a nonzero `dropped_samp_count` by running `--mode all-deferred` over the corpus. At least one v7 pack has `5`. `.result.pack.brushes` still lists every user-facing brush. `.result.pack.diagnostics.dropped_tip_details` lists one entry per dropped tip with `uuid`, `width`, `height` and `owners`, the presets that use it as their dual brush, one entry per preset with `preset_index` and `name`. When records share a uuid, only the record the uuid resolves to lists owners. The details are identical in `eager` and `all-deferred`.
- **Up-front bytes.** On that v7 pack, `.result.pack.up_front_decoded_len` is the same in the three deferred modes and equals `[.result.pack.diagnostics.dropped_tip_details[] | .width*.height] | add` for 8-bit tips. A pack with no dropped tips has `0`. The v2 seed `zero_area_tip` with `--mode all-deferred` has `1`, because v1/v2 tips are always decoded up front.
- **Unreadable unused records.** `.result.pack.diagnostics.unreadable_tip_details` lists each samp record that no sampled brush uses and whose tip does not parse, with `uuid`, `message` and `owners`, each entry with `preset_index` and `name`. No corpus pack or fuzz seed has one, so it is `[]`. To get one, copy the v7 pack outside the repo and zero the bytes that follow the `$` and the uuid in one dropped tip's samp entry, keeping the entry length. The copy lists that uuid with `message` `"no bitmap header found"` and the same `owners` as its dropped tip detail, and `dropped_samp_count` is one lower. Zeroing the uuid too removes the record instead.
- **Malformed.** Run `$H run seed-bad parse fuzz/corpus/preview_abr/tip_bitmap_dim_overflow`. Exit `1` with `.result.error` `"malformed block at offset 125: bitmap dimensions out of range"`. `v2_rle_overflow` gives `"malformed block at offset 48: samp entry length exceeds buffer"` and `v2_rle_underflow` gives `"malformed block at offset 48: samp entry length shorter than header"`, both exit `1`.

## Gotchas

- `eager` and `deferred` read patterns, the other two modes skip them, so `patterns` is empty there by design.
- The corpus and the fuzz seeds other than `unavailable_tips` have no computed presets. `computed_presets` stays empty unless the user supplies a pack with computed tips.
- A tip whose header parses but whose pixels do not decode is `unreadable` in `sampled_brushes` only where it is decoded up front. A v6/v7/v9/v10 tip is decoded up front in `eager`, and in `deferred` and `deferred-no-patterns` only when it is a dual-brush tip or shares its uuid with another record. A v1/v2 RLE entry is `unreadable` in every mode but `all-deferred`. Where it is not `unreadable`, it is `readable` with the error in its `brushes[i].tip.error`. On such a pack `sampled_brushes` and `brushes` differ across modes by design. An unused v6/v7/v9/v10 record is in `unreadable_tip_details` in `eager`, `deferred` and `deferred-no-patterns`, and in `dropped_tip_details` in `all-deferred`.
- Exit `101` means a panic inside the library, which is a bug on any input.
- Eager parsing of a 30 MB pack holds every decoded tip in memory. Prefer `all-deferred` for corpus sweeps.
