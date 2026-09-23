mod counting_alloc;

use std::io::Cursor;

use brushkit_preview::procreate::decode_tip_png;
use brushkit_preview::{decode_tip_image, GrayscaleBitmap};
use counting_alloc::{live, peak, reset_peak};
use image::{DynamicImage, GrayAlphaImage, GrayImage, ImageFormat, Luma, LumaA, Rgba, RgbaImage};

const SIDE: u32 = 1024;
const PIXELS: usize = (SIDE * SIDE) as usize;
/// Room for the PNG decoder's own buffers, which do not grow with the image.
const DECODER_SLACK: usize = 512 * 1024;

fn png(image: DynamicImage) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("encode fixture");
    bytes.into_inner()
}

fn measure(decode: impl FnOnce() -> GrayscaleBitmap) -> (usize, GrayscaleBitmap) {
    let before = live();
    reset_peak();
    let bitmap = decode();
    (peak() - before, bitmap)
}

#[test]
fn tip_decoders_hold_no_copy_of_the_decoded_image() {
    let gray = png(GrayImage::from_pixel(SIDE, SIDE, Luma([0x40])).into());
    let gray_alpha = png(GrayAlphaImage::from_pixel(SIDE, SIDE, LumaA([0x40, 0xC0])).into());
    let rgba = png(RgbaImage::from_pixel(SIDE, SIDE, Rgba([0x10, 0x20, 0x30, 0xC0])).into());

    // The decoded four-byte image, its alpha compacted in place into the tip.
    let (growth, bitmap) = measure(|| decode_tip_image(&rgba).expect("RGBA tip decodes"));
    assert_eq!(bitmap.data, vec![0xC0; PIXELS], "alpha is the tip");
    println!("decode_tip_image RGBA: {growth} bytes");
    assert!(
        growth <= 4 * PIXELS + DECODER_SLACK,
        "decode_tip_image must extract RGBA alpha inside the decoded buffer: {growth} bytes"
    );

    // The decoded two-byte image, its alpha compacted in place into the tip.
    let (growth, bitmap) =
        measure(|| decode_tip_image(&gray_alpha).expect("gray+alpha tip decodes"));
    assert_eq!(bitmap.data, vec![0xC0; PIXELS], "alpha is the tip");
    println!("decode_tip_image gray+alpha: {growth} bytes");
    assert!(
        growth <= 2 * PIXELS + DECODER_SLACK,
        "decode_tip_image must extract gray+alpha alpha inside the decoded buffer: {growth} bytes"
    );

    // The decoded gray plane, inverted in place into the tip.
    let (growth, bitmap) = measure(|| decode_tip_image(&gray).expect("gray tip decodes"));
    assert_eq!(bitmap.data, vec![0xBF; PIXELS], "gray is inverted");
    println!("decode_tip_image gray: {growth} bytes");
    assert!(
        growth <= PIXELS + DECODER_SLACK,
        "decode_tip_image must not copy a gray image: {growth} bytes"
    );

    // The decoded gray plane, returned as the tip.
    let (growth, bitmap) = measure(|| decode_tip_png(&gray).expect("gray Shape.png decodes"));
    assert_eq!(bitmap.data, vec![0x40; PIXELS], "gray is the tip");
    println!("decode_tip_png gray: {growth} bytes");
    assert!(
        growth <= PIXELS + DECODER_SLACK,
        "decode_tip_png must not copy a gray image: {growth} bytes"
    );
}
