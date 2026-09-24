# Run the fuzz targets

From the repository root, install nightly Rust and cargo-fuzz:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz --version 0.13.2 --locked
```

Run any target with its committed corpus:

```sh
cargo +nightly fuzz run abr_parse
cargo +nightly fuzz run preview_abr
cargo +nightly fuzz run preview_brush
cargo +nightly fuzz run preview_brushset
```

The preview targets use `max_cell: 8` to exercise decoding and downsampling.
They also assert that every available tip has pixels, no side over `max_cell`,
and one byte per pixel.
Each target reads seeds from `fuzz/corpus/<target>`. The ABR seeds are synthetic
fixtures. The Procreate seeds cover valid ZIP archives, binary plists, PNG
shapes, missing and corrupt shapes, oversized PNG dimensions, and plist depth.
The `filtered_shape` seed embeds `crates/preview/tests/fixtures/filtered_shape.png`,
whose IDAT is compressed and whose rows are filtered. The PNG is committed rather
than generated, so a compressor change does not move its bytes.

To replay the corpus and then run a bounded campaign, replace `preview_brush`
with the target name:

```sh
cargo +nightly fuzz run preview_brush -- -runs=0 -timeout=10 -rss_limit_mb=2048 -max_len=1048576
cargo +nightly fuzz run preview_brush -- -max_total_time=30 -timeout=10 -rss_limit_mb=2048 -max_len=1048576
```

CI runs both commands for all four targets and uploads failure artifacts.
Ordinary `cargo test` replays every corpus file without nightly Rust or
cargo-fuzz. It also checks generated seed bytes and valid preview output.

## Regenerate the synthetic seeds

Run the ignored generator after changing its fixtures:

```sh
cargo test -p brushkit-preview --test fuzz_corpus regenerate_fuzz_seeds -- --ignored --exact
```

The generator copies the nine original ABR seeds into `preview_abr` and writes
sampled ABR tips that need downsampling, fit `max_cell` or have zero area, and
the named Procreate seeds. It preserves other files,
including regression inputs. Review and commit the changed seeds with their
generator changes.

## Reproduce a failure

Replace `<target>` and `<artifact>` with the target name and reported path:

```sh
cargo +nightly fuzz run <target> <artifact> -- -timeout=10 -rss_limit_mb=2048
cargo +nightly fuzz tmin <target> <artifact> -- -timeout=10 -rss_limit_mb=2048
```

Fix the cause, then copy the minimized input into `fuzz/corpus/<target>/` with a
descriptive name. Run `cargo test` and replay that target before committing the
fix and regression input. Keep copyrighted brush packs out of the repository.
