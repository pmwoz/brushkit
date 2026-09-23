mod common;
mod counting_alloc;

use std::io::Cursor;

use brushkit_preview::procreate::decode_tip_png;
use brushkit_preview::{decode_tip_image, GrayscaleBitmap};
use common::progressive_jpeg;
use counting_alloc::{live, peak, reset_peak};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageBuffer, ImageFormat, Luma, LumaA, Rgb, Rgba};

/// Large enough that shrinking the decoded buffer to the tip stays in place.
/// macOS moves a 4 MiB block shrunk to 1 MiB.
const SIDE: u32 = 2048;
const PIXELS: usize = (SIDE * SIDE) as usize;
/// Room for the PNG decoder's own buffers, which do not grow with the image.
const DECODER_SLACK: usize = 512 * 1024;

fn encode(image: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, format).expect("encode fixture");
    bytes.into_inner()
}

fn png(image: DynamicImage) -> Vec<u8> {
    encode(image, ImageFormat::Png)
}

/// Gray noise at quality 100, so the encoded bytes are most of the decoded
/// size and a copy of them shows in the peak.
fn noise_jpeg() -> Vec<u8> {
    let mut state = 0x2545_F491u32;
    let noise = ImageBuffer::from_fn(SIDE, SIDE, |_, _| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        Luma([state as u8])
    });
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 100)
        .encode_image(&noise)
        .expect("encode fixture");
    bytes
}

/// The peak growth of `decode`, after checking that the tip it returns holds
/// one byte per pixel and nothing of the decoded image. Pixel values are
/// checked by the unit tests in `bitmap.rs`.
fn measure(name: &str, decode: impl FnOnce() -> GrayscaleBitmap) -> usize {
    let before = live();
    reset_peak();
    let bitmap = decode();
    let retained = live() - before;
    assert_eq!(bitmap.data.len(), PIXELS, "{name}: one byte per pixel");
    assert!(
        retained <= PIXELS,
        "{name}: the tip must hold only its own plane: {retained} bytes"
    );
    peak() - before
}

type Decode = fn(&[u8]) -> GrayscaleBitmap;

fn tip_image(bytes: &[u8]) -> GrayscaleBitmap {
    decode_tip_image(bytes).expect("tip decodes")
}

fn shape_png(bytes: &[u8]) -> GrayscaleBitmap {
    decode_tip_png(bytes).expect("Shape.png decodes")
}

/// Each decoder's peak is the decoded image, `bytes_per_pixel` wide, plus the
/// decoder's fixed slack: the tip is converted inside the decoded buffer. A
/// progressive JPEG also holds its DCT coefficients, 2 bytes per sample.
#[test]
fn tip_decoders_hold_no_copy_of_the_decoded_image() {
    let gray = png(ImageBuffer::from_pixel(SIDE, SIDE, Luma([0x40u8])).into());
    let gray_alpha = png(ImageBuffer::from_pixel(SIDE, SIDE, LumaA([0x40u8, 0xC0])).into());
    let rgb = png(ImageBuffer::from_pixel(SIDE, SIDE, Rgb([0x10u8, 0x20, 0x30])).into());
    let rgba = png(ImageBuffer::from_pixel(SIDE, SIDE, Rgba([0x10u8, 0x20, 0x30, 0xC0])).into());
    let gray16 = png(ImageBuffer::from_pixel(SIDE, SIDE, Luma([0x4040u16])).into());
    let gray_alpha16 = png(ImageBuffer::from_pixel(SIDE, SIDE, LumaA([0x4040u16, 0xC0C0])).into());
    let rgb16 = png(ImageBuffer::from_pixel(SIDE, SIDE, Rgb([0x1010u16, 0x2020, 0x3030])).into());
    let rgba16 =
        png(ImageBuffer::from_pixel(SIDE, SIDE, Rgba([0x1010u16, 0x2020, 0x3030, 0xC0C0])).into());
    // `image` encodes baseline JPEG only.
    let rgb_baseline_jpeg = encode(
        ImageBuffer::from_pixel(SIDE, SIDE, Rgb([0x10u8, 0x20, 0x30])).into(),
        ImageFormat::Jpeg,
    );
    let gray_noise_jpeg = noise_jpeg();
    let gray_progressive_jpeg = progressive_jpeg(SIDE, SIDE, 1, 0x11);
    let rgb_progressive_jpeg = progressive_jpeg(SIDE, SIDE, 3, 0x11);

    let cases: [(&str, Decode, &[u8], usize); 20] = [
        ("decode_tip_image gray", tip_image, &gray, 1),
        ("decode_tip_image gray+alpha", tip_image, &gray_alpha, 2),
        ("decode_tip_image RGB", tip_image, &rgb, 3),
        ("decode_tip_image RGBA", tip_image, &rgba, 4),
        ("decode_tip_image gray 16", tip_image, &gray16, 2),
        (
            "decode_tip_image gray+alpha 16",
            tip_image,
            &gray_alpha16,
            4,
        ),
        ("decode_tip_image RGB 16", tip_image, &rgb16, 6),
        ("decode_tip_image RGBA 16", tip_image, &rgba16, 8),
        (
            "decode_tip_image RGB baseline JPEG",
            tip_image,
            &rgb_baseline_jpeg,
            3,
        ),
        (
            "decode_tip_image gray noise JPEG",
            tip_image,
            &gray_noise_jpeg,
            1,
        ),
        (
            "decode_tip_image gray progressive JPEG",
            tip_image,
            &gray_progressive_jpeg,
            1 + 2,
        ),
        (
            "decode_tip_image RGB progressive JPEG",
            tip_image,
            &rgb_progressive_jpeg,
            3 + 3 * 2,
        ),
        ("decode_tip_png gray", shape_png, &gray, 1),
        ("decode_tip_png gray+alpha", shape_png, &gray_alpha, 2),
        ("decode_tip_png RGB", shape_png, &rgb, 3),
        ("decode_tip_png RGBA", shape_png, &rgba, 4),
        ("decode_tip_png gray 16", shape_png, &gray16, 2),
        ("decode_tip_png gray+alpha 16", shape_png, &gray_alpha16, 4),
        ("decode_tip_png RGB 16", shape_png, &rgb16, 6),
        ("decode_tip_png RGBA 16", shape_png, &rgba16, 8),
    ];
    for (name, decode, bytes, bytes_per_pixel) in cases {
        let growth = measure(name, || decode(bytes));
        println!("{name}: {growth} bytes");
        assert!(
            growth <= bytes_per_pixel * PIXELS + DECODER_SLACK,
            "{name} must hold only the decoded image: {growth} bytes"
        );
    }
}
