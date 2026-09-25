---
name: verify-brushkit
description: Drive the brushkit library the way a downstream Rust consumer does and capture proof. Builds a probe crate that calls the public `brushkit` facade (parse_abr*, preview_abr/brush/brushset, *_first_available, contact sheets, descriptor dump) on real or synthetic brush files and writes JSON summaries and PNGs. Use to prove a reader or renderer change works on real files, beyond `cargo test`.
---

# verify-brushkit

brushkit is a library with no app to launch. Its user is a Rust program that
depends on the `brushkit` crate. This skill ships that program: `probe/`, a
separate crate with its own `[workspace]` that depends on `crates/brushkit` by
path, with the `serde` and default `text` features. It calls only public API,
so it sees exactly what a consumer sees from the current checkout, including
uncommitted changes under `crates/`.

All commands run from the repo root through one helper:

```sh
H=.claude/skills/verify-brushkit/scripts/verify-brushkit
```

## Launch

```sh
$H build
```

Release build of the probe into `target/verify-brushkit/`. The first build
takes about 10 s on a warm cargo cache. `$H run` rebuilds on every call, so a
run never uses a stale binary. Ready means `build` exits 0. There is no process
to keep alive and no port, so any number of runs can go in parallel.
Parallel runs need distinct `RUN_ID`s or labels, or they overwrite each other's
evidence.

## Doctor

```sh
BRUSHKIT_CORPUS_DIR=<corpus dir> $H doctor
```

Prints the checkout HEAD (flags uncommitted changes under `crates/`), the
toolchain, the probe path, the `links:` line, the corpus file counts and the
evidence root. Healthy means exit 0 and `links:` naming
`brushkit vX.Y.Z (<this repo>/crates/brushkit)`. Any other path means the probe
is not testing this checkout.

Real brush packs are copyrighted and not committed. `BRUSHKIT_CORPUS_DIR`
points at a local directory of them, the same variable the real-file tests
read. If it is unset, ask the user for the path. Never copy corpus files into
the repo or into evidence you commit or post. Without a corpus, the committed
fuzz seeds under `fuzz/corpus/` are synthetic inputs for the mechanics only. A
claim about real files needs a real file.

## Drive

```sh
RUN_ID=<run id> $H run <label> <command> <file> [flags]
```

The helper appends `--out <evidence dir>` itself. Commands:

| Command | Public API called | Flags |
|---|---|---|
| `parse` | `abr::parse_abr` or a deferred variant, then `DeferredPack::decode_tip` per brush | `--mode eager\|deferred\|deferred-no-patterns\|all-deferred` (default `eager`) |
| `preview` | `preview::preview_abr`, `preview_brush` or `preview_brushset`, or the `_first_available` twin with `--first`, then `generate_preview_png` per tip and `generate_contact_sheet_png` | `--kind abr\|brush\|brushset` (default from the file extension, required for extensionless fuzz seeds), `--max-cell N` (default 128), `--first N` |
| `dump` | `abr::parse_abr`, then `abr::dump::dump_descriptors` on `raw_desc_block`, serialized by serde | none |

Examples:

```sh
RUN_ID=demo $H run preview-real preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr"
RUN_ID=demo $H run preview-seed preview fuzz/corpus/preview_brushset/ordered_set --kind brushset
RUN_ID=demo $H run parse-real parse "$BRUSHKIT_CORPUS_DIR/<pack>.abr" --mode all-deferred
```

Read the result with `jq`, for example
`jq '.result.counts' /tmp/verify-brushkit/demo/preview-real/summary.json`.
Probe exit codes: `0` the library returned `Ok`, `1` it returned `Err` (the
message is in `result.error`), `2` probe usage or I/O error. `101` is a panic
inside the library. Malformed input must never panic, so `101` is a bug.

The feature map in [`features/README.md`](features/README.md) lists each
feature, its entry points and the proof for each.

## Evidence

Each run writes `/tmp/verify-brushkit/<RUN_ID>/<label>/` (root overridable with
`VERIFY_BRUSHKIT_EVIDENCE`):

- `command.txt`: probe arguments, the full HEAD sha and an uncommitted-changes flag.
- `stdout.txt` and `summary.json`: the same JSON, with the library result.
- `stderr.txt` and `exit_code.txt`.
- `preview` only: `tips/NNN.png` per available tip, named by entry index, and
  `sheet.png`, a labeled contact sheet of the available tips.

Proof standards:

- Go through the probe. `cargo test` proves the tests. The probe proves what a
  consumer gets. Do not add test-only hooks or call private modules.
- Capture the call and the result: `command.txt` plus `summary.json`. A final
  PNG without the JSON that says which entries it holds is incomplete.
- Look at rendered output. Open `sheet.png` with the Read tool and check the
  tips are the right shapes, not blank, clipped or inverted. Report counts from
  `summary.json`, not from the image.
- For a fix, capture the same file before and after (`git stash` or a second
  worktree) under two labels and compare the two `summary.json` files.
- This repository and its pull requests are public, and the corpus is
  copyrighted. Renders of a real file (`sheet.png`, `tips/*.png`) are copies
  of it: look at them locally, never attach, commit or post them. Evidence
  posted from a real file is text: counts and dimensions from `summary.json`
  and a description of what `sheet.png` shows. Leave out local paths, corpus
  file names and brush names, and identify a real input by its format version
  and entry count. Images and names from the synthetic fuzz seeds may be
  posted.
- The readers are pure over `&[u8]`, so the only side effects are the files the
  probe writes into the evidence directory. Nothing else should change.
  `git status` after a run must match `git status` before it.

## Cleanup

```sh
$H cleanup
```

Removes `target/verify-brushkit/` and `probe/Cargo.lock`. It never touches
`/tmp/verify-brushkit/`, so the evidence survives. There are no processes to
kill. Delete a run's evidence yourself (`rm -rf /tmp/verify-brushkit/<RUN_ID>`)
only after it has been reported.

## Helpers

- `scripts/verify-brushkit`: `build`, `doctor`, `run`, `cleanup`, as above.
  With no arguments it prints its usage.
- `probe/src/main.rs`: the consumer program. It can also be run directly:
  `target/verify-brushkit/release/brushkit-probe <command> <file> --out <dir> [flags]`.
  Extend it only through the public `brushkit` API when a new feature needs
  a new entry point.
