mod common;

use std::io::Cursor;

use brushkit_preview::{
    preview_brushset, PreviewOptions, TipPreview, UnavailableReason, MAX_PREVIEW_BYTES,
};
use common::{brush_archive, brushset_plist, zip_with};
use image::{DynamicImage, GrayImage, ImageFormat, Luma};

/// A tip keeps its size at `max_cell = SIDE`, so each available one holds
/// `SIDE * SIDE` bytes. A large tip reaches the budget in few decodes.
const SIDE: u32 = 1024;
const REFERENCES: usize = 100_000;

#[test]
fn one_member_listed_100_000_times_stays_within_the_budget() {
    let mut shape = Cursor::new(Vec::new());
    DynamicImage::from(GrayImage::from_pixel(SIDE, SIDE, Luma([200])))
        .write_to(&mut shape, ImageFormat::Png)
        .expect("encode fixture");
    let archive = brush_archive("Tip");
    let plist = brushset_plist("Set", &vec!["a"; REFERENCES]);
    let bytes = zip_with(&[
        ("brushset.plist", &plist),
        ("a/Brush.archive", &archive),
        ("a/Shape.png", shape.get_ref()),
    ]);

    let entries = preview_brushset(&bytes, PreviewOptions { max_cell: SIDE })
        .unwrap()
        .entries;

    assert_eq!(entries.len(), REFERENCES);
    let fit = MAX_PREVIEW_BYTES / (SIDE * SIDE) as usize;
    let mut held = 0;
    for (i, entry) in entries.iter().enumerate() {
        assert_eq!(entry.index, i);
        match &entry.tip {
            TipPreview::Available(bitmap) if i < fit => held += bitmap.data.len(),
            TipPreview::Unavailable(UnavailableReason::OverBudget) if i >= fit => {}
            tip => panic!("entry {i}: {tip:?}"),
        }
    }
    assert!(held <= MAX_PREVIEW_BYTES);
}
