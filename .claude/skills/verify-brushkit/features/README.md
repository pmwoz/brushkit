# brushkit verification map

This directory is the maintained source for verifying what a consumer of the
`brushkit` crate gets back. Read this index first, then use the matching
feature file as the recipe.

## Baseline preconditions

- Work from the repo root with `H=.claude/skills/verify-brushkit/scripts/verify-brushkit`.
- `$H doctor` exits 0 and its `links:` line names this checkout's `crates/brushkit`.
- `BRUSHKIT_CORPUS_DIR` points at the local directory of real brush packs.
  If it is unset, ask the user. Real files are never committed or posted.
- Set a fresh `RUN_ID` per verification so runs do not overwrite each other.
- `<pack>.abr`, `<set>.brushset` and `<one>.brush` below mean any matching
  file under `$BRUSHKIT_CORPUS_DIR`. Find them with
  `find -L "$BRUSHKIT_CORPUS_DIR" -iname '*.abr'`.

## Driving conventions

- Every action is a `$H run <label> <command> <file> [flags]` call. Flags are
  literal.
- Assert on `/tmp/verify-brushkit/$RUN_ID/<label>/summary.json` with `jq`.
- Extensionless fuzz seeds need `--kind`.
- Synthetic seeds under `fuzz/corpus/` prove mechanics. Real files prove
  claims about real files. Say which one each proof used.

## Proof and skip reporting

- Report the label, the input file, `exit_code.txt` and the `jq` value you
  asserted on.
- For rendering, open `sheet.png` with the Read tool and describe what it shows.
- Anything posted to a pull request or issue follows the public-evidence rule
  in `SKILL.md`: no renders, paths, file names or brush names from real files.
- A reported fix needs a before and after run of the same input.
- Report a path you could not reach, with the missing input or precondition.
  Do not report a synthetic-seed run as proof about real files.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph describing the
consumer-visible behavior, then exactly four H2 sections in this order:

1. `Sub-features` lists short IDs with one line each.
2. `How to get to it (user POV)` lists the public API entry points.
3. `Driving it with verify-brushkit` starts with `Preconditions:` and pairs
   each call with an exact command and the observable result.
4. `Gotchas` lists traps that waste or invalidate a run.

## Features

- [Tip previews](./preview.md) covers `preview_abr`, `preview_brush` and
  `preview_brushset`: every entry in order, unavailable reasons, `max_cell`,
  source dimensions.
- [First available tips](./first-available.md) covers the three
  `_first_available` functions.
- [Contact sheets and tip PNGs](./contact-sheet.md) covers
  `generate_contact_sheet_png` and `generate_preview_png`.
- [Parse an .abr pack](./parse-abr.md) covers the four `parse_abr*` entry
  points, patterns, diagnostics and malformed input.
- [Descriptor dump](./descriptor-dump.md) covers `dump::dump_descriptors` with
  the `serde` feature.
