//! BMP stands in for the formats a dependent crate enables on `image`, which
//! `decode_guarded` decodes through `image`'s own decoders.

use std::io::Cursor;

use brushkit_preview::{decode_tip_image, TipImageError, MAX_IMPORT_DIMENSION};
use image::{ImageBuffer, ImageFormat, Rgb};

#[test]
fn bmp_tip_is_inverted_luma() {
    let rgb = ImageBuffer::from_fn(5, 3, |x, y| Rgb([x as u8 * 40, y as u8 * 90, 17]));
    let mut bmp = Cursor::new(Vec::new());
    rgb.write_to(&mut bmp, ImageFormat::Bmp)
        .expect("encode BMP");
    let expected = image::load_from_memory(bmp.get_ref()).expect("decode BMP");
    let expected: Vec<u8> = expected.to_luma8().iter().map(|v| 255 - v).collect();

    let tip = decode_tip_image(bmp.get_ref()).expect("BMP tip decodes");
    assert_eq!((tip.width, tip.height), (5, 3));
    assert_eq!(tip.data, expected);
}

/// A 24-bit BMP header at the dimension limit, with no pixel data. Its
/// 768 MiB of RGB is over the budget, so it is rejected before any pixels.
#[test]
fn bmp_over_the_memory_budget_is_too_large() {
    let side = MAX_IMPORT_DIMENSION;
    let mut bmp = b"BM".to_vec();
    bmp.extend_from_slice(&54u32.to_le_bytes()); // file size
    bmp.extend_from_slice(&0u32.to_le_bytes()); // reserved
    bmp.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
    bmp.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
    bmp.extend_from_slice(&(side as i32).to_le_bytes());
    bmp.extend_from_slice(&(side as i32).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
    bmp.extend_from_slice(&24u16.to_le_bytes()); // bits per pixel
    bmp.extend_from_slice(&[0; 24]); // no compression, default sizes and palette

    let err = decode_tip_image(&bmp).expect_err("over the budget");
    assert!(
        matches!(err, TipImageError::TooLarge { width, height } if width == side && height == side),
        "got {err:?}"
    );
}
