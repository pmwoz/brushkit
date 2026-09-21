# brushkit

Rust crates for reading brush files and rendering their tip shapes.

- Photoshop `.abr` (versions 2, 6 and 10): brush names, tip bitmaps,
  sampled and computed tips, embedded patterns.
- Procreate `.brush` and `.brushset`: brush names, set order, tip shapes.
- Tip rendering: grayscale bitmaps, thumbnails and contact-sheet grids.

## Crates

| Crate | Purpose |
|---|---|
| `brushkit` | One dependency over the two below, as `brushkit::abr` and `brushkit::preview`. |
| `brushkit-abr` | Reader for `.abr` files. Parses the pack into brushes with names, tip bitmaps or computed geometry, and patterns. |
| `brushkit-preview` | Tip bitmaps for any supported file: decodes sampled tips, synthesizes computed tips, reads Procreate `Shape.png`, and lays out contact sheets. |
| `brushkit-fixture` | Generates synthetic `.abr` files for tests and fuzzing. |

The public contract of `brushkit-preview`:

```rust
pub struct PreviewOptions { pub max_cell: u32 }

pub struct PreviewSet { pub set_name: Option<String>, pub entries: Vec<PreviewEntry> }
pub struct PreviewEntry { pub index: usize, pub name: String, pub tip: TipPreview }

pub enum TipPreview {
    Available(GrayscaleBitmap),   // { width: u32, height: u32, data: Vec<u8> }
    Unavailable(UnavailableReason),
}

pub enum UnavailableReason {
    NoShapePng,
    UnsupportedTipKind(String),
    Corrupt(String),
    TooLarge { width: u32, height: u32 },
}

pub fn preview_abr(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError>;
pub fn preview_brush(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError>;
pub fn preview_brushset(bytes: &[u8], opts: PreviewOptions) -> Result<PreviewSet, PreviewError>;
```

Every brush in file order appears exactly once, available or not. `max_cell`
is the largest side of every returned tip, so a caller gets thumbnails sized
for its grid without decoding a whole pack at full size. A `.brushset` without
`brushset.plist` is read in zip order and has no set name.

## Building

```
cargo build
cargo test
cargo +nightly fuzz run abr_parse   # optional, needs cargo-fuzz
```

See [fuzzing instructions](fuzz/README.md) for all targets and seed regeneration.

The crates compile for `wasm32-unknown-unknown` as well as native targets.

## Real-file tests

Real brush packs are copyrighted and are not part of the repository. Tests
that assert facts about real files read `BRUSHKIT_CORPUS_DIR` and are skipped
when it is unset. Synthetic fixtures from `brushkit-fixture` cover the parsing
mechanics without it.

## License

MIT. See `LICENSE`.
