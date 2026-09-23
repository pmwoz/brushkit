mod common;

use brushkit_preview::procreate::MAX_PNG_DIMENSION;
use brushkit_preview::{decode_tip_image, TipImageError, MAX_IMPORT_DIMENSION};
use brushkit_preview::{preview_brush, preview_brushset, PreviewOptions, TipPreview};
use brushkit_preview::{PreviewSet, UnavailableReason};
use common::{
    baseline_jpeg, brush_archive, depth_bomb_plist_xml, dimension_bomb_png, gray_png,
    partial_scan_jpeg, progressive_jpeg, real_4x4_png, rgba_dimension_bomb_png, zip_with,
};
use std::io::{self, Cursor, Read};

/// Deep enough that a tree parse would overflow the stack on `Drop`, so the
/// depth guard must fire before the tree is built.
const STACK_BUSTING_DEPTH: usize = 10_000;

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
            ("Shape.png", &baseline_jpeg(width, height, 1)),
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
fn image_within_the_dimension_limit_over_the_memory_budget_is_too_large() {
    // 9000x9000 RGBA8 decodes to about 324 MB and 12000x12000 RGB to 432 MB,
    // over the 256 MiB budget with each side under MAX_PNG_DIMENSION. RGBA16
    // at the limit is 2 GiB, past isize::MAX on 32-bit targets.
    for (shape, width, height) in [
        (rgba_dimension_bomb_png(9000, 9000, 8), 9000, 9000),
        (
            rgba_dimension_bomb_png(MAX_PNG_DIMENSION, MAX_PNG_DIMENSION, 16),
            MAX_PNG_DIMENSION,
            MAX_PNG_DIMENSION,
        ),
        (baseline_jpeg(12000, 12000, 3), 12000, 12000),
    ] {
        let zip_bytes = zip_with(&[
            ("Brush.archive", &brush_archive("budget")),
            ("Shape.png", &shape),
        ]);

        let set = preview_brush(&zip_bytes, PreviewOptions { max_cell: 8 }).expect("brush reads");
        let TipPreview::Unavailable(reason) = only_tip(&set) else {
            panic!("expected Unavailable, got {:?}", only_tip(&set));
        };
        assert_eq!(
            *reason,
            UnavailableReason::TooLarge { width, height },
            "an image over the memory budget is too large, not corrupt"
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
    let zip_bytes = zip_with(&[("brushset.plist", &depth_bomb_plist_xml(STACK_BUSTING_DEPTH))]);

    let err = preview_brushset(&zip_bytes, PreviewOptions { max_cell: 8 })
        .expect_err("depth bomb must be rejected");
    assert!(err.0.contains("depth"), "error must mention depth: {err}");
}

#[test]
fn depth_bomb_brush_archive_is_rejected_not_recursed() {
    let zip_bytes = zip_with(&[
        ("Brush.archive", &depth_bomb_plist_xml(STACK_BUSTING_DEPTH)),
        ("Shape.png", &real_4x4_png()),
    ]);

    let set = preview_brush(&zip_bytes, PreviewOptions { max_cell: 8 }).expect("brush reads");
    let TipPreview::Unavailable(UnavailableReason::Corrupt(msg)) = only_tip(&set) else {
        panic!("expected Corrupt, got {:?}", only_tip(&set));
    };
    assert!(msg.contains("depth"), "error must mention depth: {msg}");
}

#[test]
fn imported_image_dimension_bomb_is_rejected_not_allocated() {
    // The IDAT holds 4x4 pixels, so a decode-first path fails on the truncated
    // data instead of the declared size.
    let over = MAX_IMPORT_DIMENSION + 1;
    for (bytes, width, height) in [
        (dimension_bomb_png(60000, 60000), 60000, 60000),
        (dimension_bomb_png(20000, 20000), 20000, 20000),
        (dimension_bomb_png(u32::MAX, u32::MAX), u32::MAX, u32::MAX),
        (dimension_bomb_png(over, 1), over, 1),
        (dimension_bomb_png(1, over), 1, over),
        (baseline_jpeg(65535, 65535, 1), 65535, 65535),
        (baseline_jpeg(over, 1, 1), over, 1),
        (baseline_jpeg(1, over, 3), 1, over),
    ] {
        let err = decode_tip_image(&bytes).expect_err("an oversized image must be rejected");
        assert!(
            matches!(err, TipImageError::TooLarge { width: w, height: h } if (w, h) == (width, height)),
            "{width}x{height}: expected TooLarge with the declared dimensions, got {err:?}"
        );
    }
}

#[test]
fn imported_image_over_the_memory_budget_is_too_large() {
    // Both sides are within MAX_IMPORT_DIMENSION. RGBA8 at the limit decodes to
    // 1 GiB and RGBA16 to 2 GiB, past isize::MAX on 32-bit targets. The PNGs are
    // sized from IHDR, the RGB JPEG (768 MiB) from the decoder.
    let side = MAX_IMPORT_DIMENSION;
    for bytes in [
        rgba_dimension_bomb_png(side, side, 8),
        rgba_dimension_bomb_png(side, side, 16),
        baseline_jpeg(side, side, 3),
    ] {
        let err = decode_tip_image(&bytes).expect_err("an over-budget image must be rejected");
        assert!(
            matches!(err, TipImageError::TooLarge { width, height } if (width, height) == (side, side)),
            "expected TooLarge, got {err:?}"
        );
    }
}

#[test]
fn progressive_jpeg_over_the_budget_with_its_coefficients_is_too_large() {
    // Each decoded image fits the 512 MiB budget, but not with the DCT
    // coefficients a progressive decode holds, 2 bytes per sample: gray
    // 256 + 512 MiB, RGB 192 + 384 MiB. The last one is over by 340 KiB only
    // because a vertical sampling factor of 3 pads its height to 16368 rows.
    for (width, height, sampling) in [
        (MAX_IMPORT_DIMENSION, MAX_IMPORT_DIMENSION, &[0x11][..]),
        (
            MAX_IMPORT_DIMENSION / 2,
            MAX_IMPORT_DIMENSION / 2,
            &[0x11; 3],
        ),
        (3648, 16352, &[0x13; 3]),
    ] {
        let name = format!("{width}x{height} sampling {sampling:x?}");
        // A decoded tip is not printed: at this size it is hundreds of MiB.
        match decode_tip_image(&progressive_jpeg(width, height, sampling)) {
            Err(TipImageError::TooLarge {
                width: w,
                height: h,
            }) => {
                assert_eq!((w, h), (width, height), "{name}: the declared dimensions");
            }
            Err(err) => panic!("{name}: expected TooLarge, got {err:?}"),
            Ok(_) => panic!("{name}: an over-budget progressive JPEG decoded"),
        }
    }
}

#[test]
fn baseline_jpeg_with_a_partial_first_scan_counts_its_coefficients() {
    // zune-jpeg holds every coefficient of a baseline JPEG for the whole
    // decode when its first scan leaves out a component. At 8192x8192 RGB is
    // 192 MiB, which fits the budget, and with 384 MiB of coefficients does
    // not. A gray JPEG after the end of the image, as a gain map is stored,
    // holds a one-component scan that zune-jpeg never reads.
    let side = MAX_IMPORT_DIMENSION / 2;
    let mut jpeg = baseline_jpeg(side, side, 3);
    jpeg.extend_from_slice(&baseline_jpeg(64, 64, 1));
    let tip = decode_tip_image(&jpeg)
        .expect("a baseline JPEG with every component in its first scan decodes");
    assert_eq!((tip.width, tip.height), (side, side));

    match decode_tip_image(&partial_scan_jpeg(side, side, &[0x11; 3], 1)) {
        Err(TipImageError::TooLarge { width, height }) => {
            assert_eq!((width, height), (side, side));
        }
        Err(err) => panic!("expected TooLarge, got {err:?}"),
        Ok(_) => panic!("a partial first scan decoded without its coefficients counted"),
    }
}

#[test]
fn frame_header_after_the_first_scan_is_not_counted() {
    // A gray JPEG of 256 MiB followed by a color JPEG of the same size, as an
    // MPF secondary image is stored. zune-jpeg parses one frame header, before
    // the first scan, so the color frame's 1.5 GiB of coefficients never exist.
    let side = MAX_IMPORT_DIMENSION;
    let mut jpeg = baseline_jpeg(side, side, 1);
    jpeg.extend_from_slice(&baseline_jpeg(side, side, 3));
    let tip = decode_tip_image(&jpeg).expect("the gray JPEG fits the budget");
    assert_eq!((tip.width, tip.height), (side, side));
}

#[test]
fn progressive_420_jpeg_is_held_to_the_budget_by_its_real_coefficients() {
    // 4:2:0 codes six blocks per 16x16 MCU, so its coefficients take 3 bytes
    // per pixel next to 3 bytes of RGB. 512 MiB is 5461.3 such rows of 16384.
    let sampling = [0x22, 0x11, 0x11];
    let tip = decode_tip_image(&progressive_jpeg(MAX_IMPORT_DIMENSION, 5456, &sampling))
        .expect("a 4:2:0 JPEG within the budget decodes");
    assert_eq!((tip.width, tip.height), (MAX_IMPORT_DIMENSION, 5456));

    match decode_tip_image(&progressive_jpeg(MAX_IMPORT_DIMENSION, 5472, &sampling)) {
        Err(TipImageError::TooLarge { width, height }) => {
            assert_eq!((width, height), (MAX_IMPORT_DIMENSION, 5472));
        }
        Err(err) => panic!("expected TooLarge, got {err:?}"),
        Ok(_) => panic!("a 4:2:0 JPEG over the budget decoded"),
    }
}

#[test]
fn progressive_jpeg_with_a_second_frame_header_counts_the_larger_one() {
    // The 4:2:0 image that fits the budget above, with a comment after its
    // frame header holding a 4:4:4 frame header of the same size. zune-jpeg
    // skips the comment, but the coefficient count cannot tell which header
    // it parsed, so every component counts at full resolution: 6 bytes per
    // pixel of coefficients.
    let (width, height) = (MAX_IMPORT_DIMENSION, 5456);
    let mut jpeg = progressive_jpeg(width, height, &[0x22, 0x11, 0x11]);
    let mut comment = vec![0xFF, 0xFE, 0x00, 21, 0xFF, 0xC2, 0x00, 17, 8];
    comment.extend_from_slice(&(height as u16).to_be_bytes());
    comment.extend_from_slice(&(width as u16).to_be_bytes());
    comment.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x22, 0, 3, 0x22, 0]);
    let huffman_tables = jpeg
        .windows(2)
        .position(|marker| marker == [0xFF, 0xC4])
        .expect("the fixture has a DHT segment");
    jpeg.splice(huffman_tables..huffman_tables, comment);

    match decode_tip_image(&jpeg) {
        Err(TipImageError::TooLarge {
            width: w,
            height: h,
        }) => assert_eq!((w, h), (width, height)),
        Err(err) => panic!("expected TooLarge, got {err:?}"),
        Ok(_) => panic!("the smaller frame header was counted"),
    }
}

#[test]
fn frame_header_zune_jpeg_would_reject_is_not_counted() {
    // A comment holding a frame header of the image's size with 255 components
    // sampled 15x15, which would count about 2.4 GB of coefficients. zune-jpeg
    // accepts at most 4 components and factors up to 4.
    let (width, height) = (2048u16, 2048u16);
    let mut jpeg = progressive_jpeg(width.into(), height.into(), &[0x11]);
    let mut header = vec![0xFF, 0xC2];
    header.extend_from_slice(&(8u16 + 3 * 255).to_be_bytes());
    header.push(8);
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&width.to_be_bytes());
    header.push(255);
    for id in 0..255u8 {
        header.extend_from_slice(&[id, 0xFF, 0]);
    }
    let mut comment = vec![0xFF, 0xFE];
    comment.extend_from_slice(&(2 + header.len() as u16).to_be_bytes());
    comment.extend_from_slice(&header);
    jpeg.splice(2..2, comment);

    let tip = decode_tip_image(&jpeg).expect("the rejected frame header is not counted");
    assert_eq!((tip.width, tip.height), (width.into(), height.into()));
}
