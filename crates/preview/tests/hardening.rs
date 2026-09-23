mod common;

use brushkit_preview::procreate::MAX_PNG_DIMENSION;
use brushkit_preview::{preview_brush, preview_brushset, PreviewOptions, TipPreview};
use brushkit_preview::{PreviewSet, UnavailableReason};
use common::{
    brush_archive, depth_bomb_plist_xml, dimension_bomb_png, gray_jpeg, gray_png, real_4x4_png,
    zip_with,
};
use std::io::{self, Cursor, Read};

fn only_tip(set: &PreviewSet) -> &TipPreview {
    assert_eq!(set.entries.len(), 1, "expected a single entry");
    &set.entries[0].tip
}

#[test]
fn huge_declared_entry_is_rejected_before_inflate() {
    const OVER: u64 = 256 * 1024 * 1024 + 1;
    let mut zw = zip::write::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zw.start_file("brushset.plist", opts).expect("start_file");
    io::copy(&mut io::repeat(0u8).take(OVER), &mut zw).expect("stream zeros");
    let zip_bytes = zw.finish().expect("finish zip").into_inner();

    assert!(
        zip_bytes.len() < 8 * 1024 * 1024,
        "zip should stay tiny, got {} bytes",
        zip_bytes.len()
    );

    let err = preview_brushset(&zip_bytes, PreviewOptions { max_cell: 8 })
        .expect_err("oversize entry must be rejected");
    assert!(
        err.0.contains("exceeds"),
        "error must mention the exceeded limit: {err}"
    );
}

#[test]
fn png_dimension_bomb_is_rejected_not_allocated() {
    // u32::MAX overflows the output buffer size on 64-bit targets as well.
    for (width, height) in [
        (60000, 60000),
        (u32::MAX, u32::MAX),
        (MAX_PNG_DIMENSION + 1, 1),
        (1, MAX_PNG_DIMENSION + 1),
    ] {
        let zip_bytes = zip_with(&[
            ("Brush.archive", &brush_archive("bomb")),
            ("Shape.png", &dimension_bomb_png(width, height)),
        ]);

        let set = preview_brush(&zip_bytes, PreviewOptions { max_cell: 8 }).expect("brush reads");
        let TipPreview::Unavailable(reason) = only_tip(&set) else {
            panic!("expected Unavailable, got {:?}", only_tip(&set));
        };
        assert_eq!(
            *reason,
            UnavailableReason::TooLarge { width, height },
            "the declared dimensions must be reported, not allocated"
        );
    }
}

#[test]
fn jpeg_dimension_bomb_is_rejected_not_allocated() {
    // Shape.png is sniffed by content, so a JPEG under that name decodes too.
    for (width, height) in [
        (65535, 65535),
        (MAX_PNG_DIMENSION + 1, 1),
        (1, MAX_PNG_DIMENSION + 1),
    ] {
        let zip_bytes = zip_with(&[
            ("Brush.archive", &brush_archive("bomb")),
            ("Shape.png", &gray_jpeg(width, height)),
        ]);

        let set = preview_brush(&zip_bytes, PreviewOptions { max_cell: 8 }).expect("brush reads");
        let TipPreview::Unavailable(reason) = only_tip(&set) else {
            panic!("expected Unavailable, got {:?}", only_tip(&set));
        };
        assert_eq!(
            *reason,
            UnavailableReason::TooLarge { width, height },
            "the declared dimensions must be reported, not allocated"
        );
    }
}

#[test]
fn png_at_the_dimension_limit_decodes() {
    for (width, height) in [(MAX_PNG_DIMENSION, 1), (1, MAX_PNG_DIMENSION)] {
        let zip_bytes = zip_with(&[
            ("Brush.archive", &brush_archive("edge")),
            ("Shape.png", &gray_png(width, height, 255)),
        ]);

        let set = preview_brush(&zip_bytes, PreviewOptions { max_cell: 8 }).expect("brush reads");
        assert!(
            matches!(only_tip(&set), TipPreview::Available(_)),
            "a {width}x{height} tip is within the limit, got {:?}",
            only_tip(&set)
        );
    }
}

#[test]
fn depth_bomb_brushset_plist_is_rejected_not_recursed() {
    let zip_bytes = zip_with(&[("brushset.plist", &depth_bomb_plist_xml())]);

    let err = preview_brushset(&zip_bytes, PreviewOptions { max_cell: 8 })
        .expect_err("depth bomb must be rejected");
    assert!(err.0.contains("depth"), "error must mention depth: {err}");
}

#[test]
fn depth_bomb_brush_archive_is_rejected_not_recursed() {
    let zip_bytes = zip_with(&[
        ("Brush.archive", &depth_bomb_plist_xml()),
        ("Shape.png", &real_4x4_png()),
    ]);

    let set = preview_brush(&zip_bytes, PreviewOptions { max_cell: 8 }).expect("brush reads");
    let TipPreview::Unavailable(UnavailableReason::Corrupt(msg)) = only_tip(&set) else {
        panic!("expected Corrupt, got {:?}", only_tip(&set));
    };
    assert!(msg.contains("depth"), "error must mention depth: {msg}");
}
