# brushkit

Rust crates that read brush files and render their tip shapes. They read
only. Nothing here writes or converts a brush file.

- Photoshop `.abr`, versions 1, 2, 6, 7, 9 and 10: brush names (v1 files
  have none), sampled tip bitmaps (8 or 16 bit, raw, RLE or zlib), computed-tip geometry, the full
  brush descriptor and embedded patterns.
- Procreate `.brush` and `.brushset`: brush names, set order from
  `brushset.plist`, tip shapes from `Shape.png`.
- Tip rendering: grayscale bitmaps, thumbnails sized for a grid, contact
  sheets.

## Crates

| Crate | Purpose |
|---|---|
| `brushkit` | One dependency over the two readers, exposed as `brushkit::abr` and `brushkit::preview`. |
| `brushkit-abr` | Parses an `.abr` pack into brushes (name, tip bitmap, descriptor), computed presets, patterns, and a per-file report of what was dropped and why. |
| `brushkit-preview` | One tip bitmap per brush for any supported file, plus contact sheets. |
| `brushkit-fixture` | Byte writers for the descriptor encoding, used by `brushkit-abr`'s own tests to build synthetic descriptors. |

## Usage

```toml
[dependencies]
brushkit = "0.1"
```

```rust
use brushkit::abr::parse_abr;
use brushkit::preview::{preview_abr, PreviewOptions, TipPreview};

let bytes = std::fs::read("pack.abr").unwrap();

let pack = parse_abr(&bytes).unwrap();
println!(
    "{:?}: {} sampled brushes, {} computed presets, {} patterns",
    pack.version,
    pack.brushes.len(),
    pack.computed_presets.len(),
    pack.patterns.len()
);

let set = preview_abr(&bytes, PreviewOptions { max_cell: 128 }).unwrap();
for entry in &set.entries {
    match &entry.tip {
        TipPreview::Available(bitmap) => {
            println!("{}: {}x{}", entry.name, bitmap.width, bitmap.height)
        }
        TipPreview::Unavailable(reason) => println!("{}: {reason:?}", entry.name),
    }
}
```

The same code is `crates/brushkit/examples/readme.rs`, so it compiles with
the test suite.

`preview_abr`, `preview_brush` and `preview_brushset` return every brush in
file order exactly once, available or not. A brush whose tip cannot be
rendered carries a reason (`NoShapePng`, `UnsupportedTipKind`, `Corrupt`,
`TooLarge`) rather than being dropped, so a caller can lay out a complete
grid. `max_cell` is the largest side of every returned bitmap. A `.brushset`
without `brushset.plist` is read in zip order and has no set name.

`parse_abr` decodes every tip up front. `parse_abr_deferred` and
`parse_abr_deferred_without_patterns` keep tips as byte ranges to decode on
demand. The preview path uses the latter, so previewing a pack with a large
pattern block does not copy or decode the patterns.

## Features

- `text` (default, `brushkit` and `brushkit-preview`): contact-sheet labels.
  Pulls in `ab_glyph` and an embedded copy of Inter Regular.
- `serde` (`brushkit` and `brushkit-abr`): `Serialize` on the raw descriptor
  dump in `brushkit_abr::dump`.

Every crate builds for `wasm32-unknown-unknown`. The readers take `&[u8]` and
touch neither the filesystem nor threads.

## Untrusted input

Every size, count and dimension read from a file is checked against a ceiling
before anything is allocated, and malformed input is an error, not a panic.
Four libFuzzer targets under `fuzz/` cover both readers. CI replays their
committed seed corpus and fuzzes each target for 30 seconds on every pull
request and push to `main`.

## Building

```sh
cargo build
cargo test
cargo +nightly fuzz run abr_parse   # needs cargo-fuzz
```

See [fuzz/README.md](fuzz/README.md) for the other targets and seed
regeneration.

## Real-file tests

Real brush packs are copyrighted and are not in the repository. Tests that
assert facts about real files read `BRUSHKIT_CORPUS_DIR` and are skipped when
it is unset. Synthetic files in the unit tests and the fuzz corpus cover the
parsing mechanics without it.

## Versioning

The crates are at 0.x. Minor releases may change the public API. Pin the
minor version. Every release is a git tag `vX.Y.Z` on this repository.

## License

MIT, see `LICENSE`. The Inter font embedded by the `text` feature is under
the SIL Open Font License 1.1, see `crates/preview/assets/fonts/OFL.txt`.
