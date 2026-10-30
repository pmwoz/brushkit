# Descriptor dump

A consumer with the `serde` feature dumps the raw brush descriptor tree of a
pack: every `(key, type, value)` node per preset, serializable to JSON, for
characterizing what real files contain.

## Sub-features

- `dump-presets` lists one entry per preset with its full item tree.
- `dump-unknown-stop` records where an unknown type tag stopped the walk.
- `dump-no-desc` handles a pack without a descriptor block.
- `dump-error` returns the parse error when `parse_abr` fails.

## How to get to it (user POV)

- `brushkit::abr::parse_abr(&bytes)?.raw_desc_block`, then `brushkit::abr::dump::dump_descriptors(&desc)`, then `serde_json::to_value(&dump)`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.

- **Seed.** Run `$H run seed-dump dump fuzz/corpus/preview_abr/unavailable_tips`. Exit `0` with 4 presets indexed 0 to 3 and no `unknown_type_stop`. `[.result.presets[].items[] | select(.key == "Nm  ") | .value.text]` is `["B","A","Round","C"]`. The sampled presets' keys are `["Nm  ","sampledData"]`, and preset 2 has a `Brsh` `Objc` of class `computedBrush`.
- **Real pack.** Run `$H run dump dump "$BRUSHKIT_CORPUS_DIR/<pack>.abr"`. Exit `0`. `jq '.result.presets | length'` equals `.result.pack.preset_count` from a `parse` run of the same file. On corpus packs every preset's first two keys are `"Nm  "` and `"Brsh"`.
- **Names match.** The `Nm  ` text of preset `i` equals the name of the brush, unavailable brush or computed preset whose `preset_index` is `i` in a `parse` run of the same file. On a pack where every preset is a readable sampled brush, the `Nm  ` list equals `[.result.pack.brushes[].name]`.
- **Value shapes.** `jq '[.. | objects | select(has("key") and has("type") and has("depth")) | {(.type): (.value | keys[0])}] | unique'` maps each type tag to its value kind, such as `{"UntF":"unit_float"}` and `{"TEXT":"text"}`. A `tdta` value is `{"tdta":{"tdta_len":N}}`.
- **No desc block.** Run `$H run seed-nodesc dump fuzz/corpus/preview_abr/zero_area_tip`. Exit `0` with `.result` `{"no_desc_block":"v2"}`.
- **Parse error.** Run `$H run seed-bad dump fuzz/corpus/preview_abr/tip_bitmap_dim_overflow`. Exit `1` with `.result.error` `"malformed block at offset 125: bitmap dimensions out of range"`.
- **Unknown tag.** Sweep the corpus with `dump` and select runs where `.result | has("unknown_type_stop")`. Its fields name the preset, depth, key and tag where the walk stopped, and its preset is the last one in `presets`.

## Gotchas

- Dump output of a real pack runs to thousands of lines. Query it with `jq` instead of reading it.
- A `tdta` value carries only its length, not its bytes.
- `unknown_type_stop` is skipped in JSON when `None`, so test for its presence, not for `null`. Its absence does not prove a complete walk: a read error or the depth limit ends the walk with no marker.
- No seed and no corpus pack has an unknown tag, so `dump-unknown-stop` is proven only by the unit tests in `crates/abr/src/dump.rs`.
- `brushes` holds only readable sampled brushes, so on a pack with unavailable or computed presets the `Nm  ` list is longer than `[.result.pack.brushes[].name]`.
- `presets | length` equals `preset_count` only for a single `desc` block whose top level is one `Brsh` list. The dump also lists other top-level items as one-item presets.
