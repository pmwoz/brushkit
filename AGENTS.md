# Working in this repository

brushkit is a Rust workspace for reading brush files and rendering their tip
shapes: Photoshop `.abr` and Procreate `.brush` and `.brushset`. Read
`README.md` first for the crate layout and the build commands.

## Process

Work is tracked in GitHub Issues and pull requests. A pull request is judged
on the diff, the tests and the description. Everything written down is
English: code, comments, commits, issues, pull requests and docs.

## Build and test

`cargo build` and `cargo test` must pass on a clean checkout with a stable Rust
toolchain and no network access beyond crates.io. Nothing in the build or the
tests reaches a private repository, a token or a secret. If a change would
require one, it is the wrong change.

## Code rules

- Readers parse untrusted bytes. Every size, count and dimension read from a
  file is checked against a limit before allocation. No panic on malformed
  input; the fuzz targets are the witness.
- Claims about what real files contain need a test on a real file, not only a
  synthetic fixture. Real brush packs are copyrighted and are not committed, so
  such a test reads `BRUSHKIT_CORPUS_DIR` and is skipped when it is unset.
  Keep the synthetic fixture for the mechanics.
- Public API changes follow semver. Consumers pin a tag.
- No conversion or writing of brush files. This workspace reads and renders.
