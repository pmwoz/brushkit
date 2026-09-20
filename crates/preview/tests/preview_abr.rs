//! `preview_abr` over the committed fuzz seeds: every preset the pack declares
//! becomes exactly one entry, and every rendered tip fits the requested cell.

use brushkit_preview::{preview_abr, PreviewOptions, TipPreview};
use std::path::Path;

const SEEDS: [&str; 2] = ["wellformed_v6_min", "wellformed_v6_patt"];

fn seed(name: &str) -> Vec<u8> {
    // Read at runtime, not `env!`: a compile-time path baked into a cached test
    // binary goes stale when the tree it was built in (a worktree) is removed.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let path = Path::new(&manifest_dir)
        .join("../../fuzz/corpus/abr_parse")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("seed {name} must be readable: {e}"))
}

#[test]
fn every_preset_becomes_one_entry_that_fits_the_cell() {
    for name in SEEDS {
        let bytes = seed(name);
        let pack = brushkit_abr::parse_abr(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected =
            pack.brushes.len() + pack.computed_presets.len() + pack.unsupported_tip_presets.len();

        let set = preview_abr(&bytes, PreviewOptions { max_cell: 4 })
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            set.set_name, None,
            "{name}: an .abr pack carries no set name"
        );
        assert_eq!(
            set.entries.len(),
            expected,
            "{name}: every preset appears exactly once"
        );
        assert_eq!(
            set.entries.iter().map(|e| e.index).collect::<Vec<_>>(),
            (0..expected).collect::<Vec<_>>(),
            "{name}: indices are dense and in order"
        );
        for entry in &set.entries {
            if let TipPreview::Available(tip) = &entry.tip {
                assert!(
                    tip.width <= 4 && tip.height <= 4,
                    "{name}: tip {}x{} exceeds the cell",
                    tip.width,
                    tip.height
                );
                assert_eq!(tip.data.len(), (tip.width * tip.height) as usize);
            }
        }
    }
}

#[test]
fn a_zero_cell_is_rejected() {
    let bytes = seed("wellformed_v6_min");
    assert!(
        preview_abr(&bytes, PreviewOptions { max_cell: 0 }).is_err(),
        "max_cell 0 has no valid output size"
    );
}
