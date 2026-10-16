mod common;
mod counting_alloc;

use brushkit_preview::{preview_brush, PreviewOptions, TipPreview};
use common::{brush_archive, gray_png, zip_with};
use counting_alloc::{live, peak, reset_peak};

const MIB: usize = 1 << 20;

#[test]
fn central_header_signatures_in_entry_data_are_not_collected() {
    let signatures = b"PK\x01\x02".repeat(MIB);
    let zip = zip_with(&[
        ("Brush.archive", &brush_archive("A")),
        ("Shape.png", &gray_png(4, 4, 0x80)),
        ("padding", &signatures),
    ]);
    let before = live();
    reset_peak();
    let set = preview_brush(&zip, PreviewOptions { max_cell: 8 }).expect("brush reads");
    let peak = peak() - before;
    println!("{} bytes: {peak} bytes", zip.len());
    assert!(
        matches!(set.entries[0].tip, TipPreview::Available(_)),
        "{:?}",
        set.entries[0].tip
    );
    assert!(
        peak <= MIB,
        "a {}-byte zip grew the heap by {peak} bytes",
        zip.len()
    );
}
