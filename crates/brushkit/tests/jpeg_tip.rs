//! Lives in the facade crate because `cargo test -p brushkit` builds
//! brushkit-preview without its dev-dependencies, so `image` has no JPEG
//! decoder here, as in a consumer's build.

use brushkit::preview::decode_tip_image;

/// Baseline grayscale JPEG, 16x8, quality 100: left half black, right half white.
const HALF_BLACK: &[u8] = include_bytes!("fixtures/half_black_16x8.jpg");

#[test]
fn jpeg_tip_decodes_with_inverted_luminance() {
    let tip = decode_tip_image(HALF_BLACK).expect("JPEG tip decodes");
    assert_eq!((tip.width, tip.height), (16, 8));
    assert_eq!(tip.data.len(), 16 * 8);

    // Columns next to the edge at x = 8 may ring, so only the outer ones are checked.
    const TOLERANCE: u8 = 8;
    for (y, row) in tip.data.as_chunks::<16>().0.iter().enumerate() {
        for (x, &v) in row.iter().enumerate() {
            if x < 6 {
                assert!(v >= 255 - TOLERANCE, "black ({x},{y}) is {v}");
            } else if x >= 10 {
                assert!(v <= TOLERANCE, "white ({x},{y}) is {v}");
            }
        }
    }
}
