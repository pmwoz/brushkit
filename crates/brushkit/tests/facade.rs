//! The facade exposes both crates under their own names and forwards the
//! `text` feature. The input is the smallest well-formed v6 pack: version 6,
//! subversion 1, one empty `8BIMpatt` section.

use brushkit::preview::{preview_abr, PreviewOptions};

const V6_MIN: &[u8] = b"\x00\x06\x00\x018BIMpatt\x00\x00\x00\x00";

#[test]
fn abr_and_preview_are_reachable_through_the_facade() {
    let pack = brushkit::abr::parse_abr(V6_MIN).expect("minimal v6 pack parses");
    assert!(pack.brushes.is_empty());

    let set =
        preview_abr(V6_MIN, PreviewOptions { max_cell: 64 }).expect("minimal v6 pack previews");
    assert!(set.entries.is_empty());
}

#[cfg(feature = "text")]
#[test]
fn text_feature_forwards_to_preview() {
    let _config: brushkit::preview::ContactSheetConfig = Default::default();
}
