//! The example in README.md. Keep the two in sync.

use brushkit::abr::parse_abr;
use brushkit::preview::{preview_abr, PreviewOptions, TipPreview};

fn main() {
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
}
