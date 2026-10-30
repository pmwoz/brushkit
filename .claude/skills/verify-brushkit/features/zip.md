# Reading a Procreate zip

A consumer that reads `.brush` or `.brushset` bytes itself opens them through
`procreate::Zip`, the same guarded zip the previews use. It rejects archives
that would make the zip crate hold far more memory than the file size, finds an
entry by its raw or decoded name, caps every read, and reports names whose
central headers disagree.

## Sub-features

- `zip-open` opens a zip with the guards, or returns an error starting `failed to open zip: `.
- `zip-names` lists entry names in central-directory order, and `members_in_zip_order` lists the brush directories.
- `zip-read` reads one entry by name, under `MAX_ENTRY_BYTES` or a smaller caller limit.
- `zip-over-limit` fails a read whose declared size is over the limit with the typed `ZipReadError::OverLimit { name, declared, limit }` before reading it.
- `zip-conflicting` lists names that two central headers give a different CRC-32 or size.

## How to get to it (user POV)

- `brushkit::preview::procreate::Zip::open(&bytes)`.
- `Zip::names`, `Zip::members_in_zip_order`, `Zip::conflicting_names`, `Zip::contains`.
- `Zip::read_entry(name)`, `Zip::read_plist(name)`, `Zip::read_entry_within(name, limit)`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.
- The probe's `zip` command opens the file and lists `names`, `members_in_zip_order` and `conflicting_names`. `--read NAME` adds `.result.read` with `contains`, `limit` and `result`, and `--limit N` reads through `read_entry_within`. A failed read is exit `1`.

- **Seed set.** Run `$H run zip-seed zip fuzz/corpus/preview_brushset/ordered_set`. Exit `0`. `names` is `["brushset.plist","a/Brush.archive","a/Shape.png","b/Brush.archive","b/Shape.png"]`, `members_in_zip_order` is `["a","b"]` and `conflicting_names` is `[]`.
- **Read.** Add `--read brushset.plist`. Exit `0` with `.result.read.result` `{"len":80}`.
- **Over the limit.** Add `--read brushset.plist --limit 1`. Exit `1` with `.result.read.result` `{"over_limit":{"declared":80,"limit":1,"name":"brushset.plist"}}`.
- **Missing entry.** Add `--read nope`. Exit `1` with `contains` `false` and `result` `{"failed":"nope not found"}`.
- **Not a zip.** Run `$H run zip-bad zip fuzz/corpus/preview_abr/sampled_tip`. Exit `1` with `.result.error` `"failed to open zip: invalid Zip archive: Could not find EOCD"`.
- **Real set.** Run `$H run zip-set zip "$BRUSHKIT_CORPUS_DIR/<set>.brushset"`. Exit `0`, `conflicting_names` is `[]`, and `members_in_zip_order | length` equals `.result.counts.entries` of a `preview` run of the same set. Read one member's `Shape.png` with `--limit 100` for a real `over_limit` whose `declared` is the PNG size.

## Gotchas

- No seed or corpus set has a conflicting name. `zip-conflicting` and the lookup of a CP437-decoded name are proven by `crates/preview/tests/procreate_zip.rs`.
- The archive guards (several end records, an oversized central directory, overlapping entries) have no committed seed. The hardening and peak tests under `crates/preview/tests/` cover them.
- A `.brush` lists `Brush.archive` and `Shape.png` at its root, so its `members_in_zip_order` is `[]`.
- Member names and entry names of a real set can contain brush names. Keep them out of anything posted.
