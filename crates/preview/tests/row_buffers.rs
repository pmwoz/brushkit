mod common;
mod counting_alloc;

use std::io::Cursor;

use brushkit_preview::decode_tip_image;
use common::{baseline_jpeg, partial_scan_jpeg, progressive_jpeg};
use counting_alloc::{live, peak, reset_peak};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

const WIDTH: u32 = 4096;
/// Large enough that shrinking the decoded buffer to the tip stays in place.
/// macOS moves a 3 MiB block shrunk to 1 MiB.
const SHORT: u32 = 1024;
const TALL: u32 = 2048;
/// Room for the PNG decoder's 128 KiB input buffer, which sometimes moves when
/// it grows, so the old and new blocks are briefly both live. Under 0.1 byte
/// per pixel the tall image adds, so a buffer that grows with the height still
/// fails the check.
const NOISE: usize = 256 * 1024;

fn rgba_png(height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::from(ImageBuffer::from_pixel(
        WIDTH,
        height,
        Rgba([0x10u8, 0x20, 0x30, 0xC0]),
    ))
    .write_to(&mut bytes, ImageFormat::Png)
    .expect("encode fixture");
    bytes.into_inner()
}

/// How far the peak of `decode_tip_image` rises above what the decode budget
/// counts, `counted_per_pixel` bytes for each pixel.
fn uncounted(name: &str, bytes: &[u8], height: u32, counted_per_pixel: usize) -> usize {
    let before = live();
    reset_peak();
    let bitmap = decode_tip_image(bytes).expect("tip decodes");
    let growth = peak() - before;
    assert_eq!((bitmap.width, bitmap.height), (WIDTH, height), "{name}");
    growth
        .checked_sub(counted_per_pixel * (WIDTH * height) as usize)
        .unwrap_or_else(|| panic!("{name}: the peak is below the counted bytes: {growth}"))
}

/// The budget counts the decoded image and the coefficients of a progressive
/// JPEG or of a baseline JPEG whose first scan leaves out a component, but not
/// those of a frame after the first scan, which zune-jpeg never allocates.
/// What it leaves out, the decoder's row buffers, must not grow with the
/// height. Heights are whole MCUs, so the coefficients are exact per pixel:
/// 2 bytes per sample of each component.
#[test]
fn uncounted_decode_buffers_do_not_grow_with_the_height() {
    type Fixture = fn(u32) -> Vec<u8>;
    let cases: [(&str, Fixture, usize); 11] = [
        ("baseline gray JPEG", |h| baseline_jpeg(WIDTH, h, 1), 1),
        (
            "baseline gray JPEG, RGB JPEG appended",
            |h| [baseline_jpeg(WIDTH, h, 1), baseline_jpeg(WIDTH, h, 3)].concat(),
            1,
        ),
        ("baseline RGB JPEG", |h| baseline_jpeg(WIDTH, h, 3), 3),
        (
            "baseline 4:4:4 JPEG, one component in the first scan",
            |h| partial_scan_jpeg(WIDTH, h, &[0x11; 3], 1),
            3 + 6,
        ),
        (
            "baseline 4:2:0 JPEG, one component in the first scan",
            |h| partial_scan_jpeg(WIDTH, h, &[0x22, 0x11, 0x11], 1),
            3 + 3,
        ),
        (
            "progressive gray JPEG",
            |h| progressive_jpeg(WIDTH, h, &[0x11]),
            1 + 2,
        ),
        (
            "progressive gray JPEG, RGB JPEG appended",
            |h| {
                [
                    progressive_jpeg(WIDTH, h, &[0x11]),
                    progressive_jpeg(WIDTH, h, &[0x11; 3]),
                ]
                .concat()
            },
            1 + 2,
        ),
        (
            "progressive 4:4:4 JPEG",
            |h| progressive_jpeg(WIDTH, h, &[0x11; 3]),
            3 + 6,
        ),
        (
            "progressive 4:2:0 JPEG",
            |h| progressive_jpeg(WIDTH, h, &[0x22, 0x11, 0x11]),
            3 + 3,
        ),
        (
            "progressive 1x4 luma JPEG",
            |h| progressive_jpeg(WIDTH, h, &[0x14, 0x11, 0x11]),
            3 + 3,
        ),
        ("RGBA PNG", rgba_png, 4),
    ];
    for (name, fixture, counted_per_pixel) in cases {
        let short = uncounted(name, &fixture(SHORT), SHORT, counted_per_pixel);
        let tall = uncounted(name, &fixture(TALL), TALL, counted_per_pixel);
        println!("{name}: {short} bytes uncounted at {SHORT} rows, {tall} at {TALL}");
        assert!(
            short.abs_diff(tall) <= NOISE,
            "{name}: the uncounted buffers depend on the height: {short} bytes at {SHORT} rows, {tall} at {TALL}"
        );
    }
}
