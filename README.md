# brushkit

Rust crates for reading brush files and rendering their tip shapes.

- Photoshop `.abr` (versions 2, 6 and 10): brush names, tip bitmaps,
  sampled and computed tips, embedded patterns.
- Procreate `.brush` and `.brushset`: brush names, set order, tip shapes.
- Tip rendering: grayscale bitmaps, thumbnails and contact-sheet grids.

Status: workspace bootstrapped, crates not yet imported. See the open issues.

## Crates

| Crate | Purpose |
|---|---|
| `brushkit-abr` | Reader for `.abr` files. Parses the pack into brushes with names, tip bitmaps or computed geometry, and patterns. |
| `brushkit-preview` | Tip bitmaps for any supported file: decodes sampled tips, synthesizes computed tips, reads Procreate `Shape.png`, and lays out contact sheets. |
| `brushkit-fixture` | Generates synthetic `.abr` files for tests and fuzzing. |

The public contract of `brushkit-preview`:

```
PreviewSet   { set_name, entries: [PreviewEntry] }
PreviewEntry { index, name, tip: TipPreview }
TipPreview   = Available { width, height, gray: bytes }
             | Unavailable { reason }
reason       = NoShapePng | UnsupportedTipKind(kind) | Corrupt(message) | TooLarge { width, height }
```

Every brush in file order appears exactly once, available or not.

## Building

```
cargo build
cargo test
cargo +nightly fuzz run abr_parse   # optional, needs cargo-fuzz
```

The crates compile for `wasm32-unknown-unknown` as well as native targets.

## Real-file tests

Real brush packs are copyrighted and are not part of the repository. Tests
that assert facts about real files read `BRUSHKIT_CORPUS_DIR` and are skipped
when it is unset. Synthetic fixtures from `brushkit-fixture` cover the parsing
mechanics without it.

## License

MIT. See `LICENSE`.
