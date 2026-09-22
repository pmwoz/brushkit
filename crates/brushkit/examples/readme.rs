//! The usage block in README.md. tests/readme_example.rs checks that the
//! two are the same code.

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
