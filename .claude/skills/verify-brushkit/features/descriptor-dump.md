# Descriptor dump

A consumer with the `serde` feature dumps the raw brush descriptor tree of a
pack: every `(key, type, value)` node per preset, serializable to JSON, for
characterizing what real files contain.

## Sub-features

- `dump-presets` lists one entry per preset with its full item tree.
- `dump-unknown-stop` records where an unknown type tag stopped the walk.
- `dump-no-desc` handles a pack without a descriptor block.

## How to get to it (user POV)

- `brushkit::abr::parse_abr(&bytes)?.raw_desc_block`, then `brushkit::abr::dump::dump_descriptors(&desc)`, then `serde_json::to_value(&dump)`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.

- **Real pack.** Run `$H run dump dump "$BRUSHKIT_CORPUS_DIR/<pack>.abr"`. Exit `0`. `jq '.result.presets | length'` equals `.result.pack.preset_count` from a `parse` run of the same file, and `jq '.result.presets[0].items | map(.key)'` starts with `"Nm  "` and `"Brsh"`.
- **Names match.** `jq '[.result.presets[].items[] | select(.key == "Nm  ") | .value.text]'` equals the brush names from a `parse` run of the same file.
- **Complete walk.** `.result.unknown_type_stop` is absent. Its presence names the preset, depth, key and tag where the walk stopped.
- **No desc block.** Run `$H run seed-nodesc dump fuzz/corpus/preview_abr/zero_area_tip`. Exit `0` with `.result` `{"no_desc_block":"v2"}`.

## Gotchas

- Dump output of a real pack runs to thousands of lines. Query it with `jq` instead of reading it.
- A `Tdta` value carries only its length, not its bytes.
- `unknown_type_stop` is skipped in JSON when `None`, so test for its presence, not for `null`.
