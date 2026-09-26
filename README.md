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
brushkit = "0.2"
```

```rust
use brushkit::abr::parse_abr_deferred_without_patterns;
use brushkit::preview::{preview_abr, PreviewOptions, TipPreview};

fn main() {
    let bytes = std::fs::read("pack.abr").unwrap();

    let pack = parse_abr_deferred_without_patterns(&bytes).unwrap().pack;
    println!(
        "{:?}: {} sampled brushes, {} computed presets",
        pack.version,
        pack.brushes.len(),
        pack.computed_presets.len()
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
}
```

The same code is `crates/brushkit/examples/readme.rs`. A test checks the two
are identical, so the block compiles whenever the test suite does.

`preview_abr`, `preview_brush` and `preview_brushset` return every brush in
file order exactly once, available or not. A brush whose tip cannot be
rendered carries a reason (`NoShapePng`, `UnsupportedTipKind`, `Corrupt`,
`TooLarge`, `OverBudget`) rather than being dropped, so a caller can lay out a
complete grid. The available tips of one call hold at most
`MAX_PREVIEW_BYTES` (256 MiB) of bitmap data. The first tip that would pass it
is `OverBudget`, and so is every later tip to render, which is not decoded.
Each entry also carries optional `source_dimensions` for the original raster,
independent of the preview size and retained if pixel decoding fails.
Computed tips have no source raster dimensions. No returned bitmap has a side
larger than `max_cell`, and a tip that already fits keeps its size. A tip with
a zero width or height is `Corrupt`, so every available tip has pixels. A
`.brushset` without `brushset.plist` is read in zip order and has no set name.

`preview_abr_first_available`, `preview_brush_first_available` and
`preview_brushset_first_available` take a count `n` and return only the
first `n` available entries of the full preview, in the same order. Each
entry keeps its index from the full preview, so indices may skip. Entries
after the `n`th available one are not built, which suits a thumbnail that
draws a few tips under a time budget. An `.abr` pack is still parsed in full,
but the parse decodes no tip.

`parse_abr` decodes every tip up front. `parse_abr_deferred` and
`parse_abr_deferred_without_patterns` keep tips as byte ranges to decode on
demand, except that they decode every tip of a v1 or v2 pack and every
dual-brush tip up front. `parse_abr_all_deferred_without_patterns` decodes no
tip up front. The preview path uses it, so previewing a pack with a large
pattern block does not copy or decode the patterns. A `DeferredPack` borrows
the input bytes instead of copying its tips out of them, so it cannot outlive
them.

## Features

- `text` (default, `brushkit` and `brushkit-preview`): the contact-sheet
  API. Labels need a font, so it pulls in `ab_glyph` and an embedded copy of
  Inter Regular.
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
