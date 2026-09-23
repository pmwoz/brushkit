mod common;
mod counting_alloc;

use std::io::{Cursor, Write};

use brushkit_preview::procreate::decode_tip_png;
use brushkit_preview::{decode_tip_image, GrayscaleBitmap};
use common::{crc32, progressive_jpeg};
use counting_alloc::{live, peak, reset_peak};
use flate2::write::ZlibEncoder;
use flate2::Compression;
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

/// A gray JPEG with 100 APP2 gain-map segments of 65,000 bytes after SOI.
/// zune-jpeg keeps its own copy of each one, which the tip never reads. The
/// segments must outweigh `PIXELS + DECODER_SLACK`, so a second copy, made
/// before the pixels are allocated, breaks the bound.
fn gain_map_jpeg() -> Vec<u8> {
    const GAIN_MAP: &[u8] = b"urn:iso:std:iso:ts:21496:-1\0";
    let mut segment = vec![0xFF, 0xE2];
    let length = u16::try_from(2 + GAIN_MAP.len() + 65_000).expect("segment fits");
    segment.extend_from_slice(&length.to_be_bytes());
    segment.extend_from_slice(GAIN_MAP);
    segment.resize(2 + usize::from(length), 0x5A);
    let mut jpeg = encode(
        ImageBuffer::from_pixel(SIDE, SIDE, Luma([0x40u8])).into(),
        ImageFormat::Jpeg,
    );
    jpeg.splice(2..2, segment.repeat(100));
    jpeg
}

/// `png` with a `kind` chunk holding `data` after IHDR.
fn with_chunk(png: &[u8], kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    chunk.extend_from_slice(&crc32(&chunk[4..]).to_be_bytes());
    // The signature and IHDR take 33 bytes.
    let mut out = png[..33].to_vec();
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&png[33..]);
    out
}

/// `png` with an iCCP chunk whose profile inflates to 64 MiB of zeros. The
/// tip never reads the profile, and it outweighs `PIXELS + DECODER_SLACK`, so
/// inflating it breaks the bound.
fn iccp_png(png: &[u8]) -> Vec<u8> {
    let mut zlib = ZlibEncoder::new(b"icc\0\0".to_vec(), Compression::best());
    zlib.write_all(&vec![0; 64 * 1024 * 1024])
        .expect("compress profile");
    with_chunk(png, b"iCCP", &zlib.finish().expect("compress profile"))
}

/// `png` with a `kind` chunk of 16 MiB: the fields before the text or EXIF
/// data, then filler. The tip never reads the chunk, and it outweighs
/// `PIXELS + DECODER_SLACK`, so keeping it breaks the bound.
fn metadata_png(png: &[u8], kind: &[u8; 4], fields: &[u8]) -> Vec<u8> {
    let mut data = fields.to_vec();
    data.resize(16 * 1024 * 1024, b'x');
    with_chunk(png, kind, &data)
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
/// progressive JPEG also holds its DCT coefficients, 2 bytes per sample of
/// each component: 4:2:0 subsamples two of three by 4.
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
    let gray_progressive_jpeg = progressive_jpeg(SIDE, SIDE, &[0x11]);
    let rgb_progressive_jpeg = progressive_jpeg(SIDE, SIDE, &[0x11; 3]);
    let rgb_420_progressive_jpeg = progressive_jpeg(SIDE, SIDE, &[0x22, 0x11, 0x11]);

    let cases: [(&str, Decode, &[u8], usize); 21] = [
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
        (
            "decode_tip_image RGB 4:2:0 progressive JPEG",
            tip_image,
            &rgb_420_progressive_jpeg,
            3 + 3,
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

    let iccp = iccp_png(&gray);
    for (name, decode) in [
        ("decode_tip_image gray PNG with iCCP", tip_image as Decode),
        ("decode_tip_png gray PNG with iCCP", shape_png),
    ] {
        let growth = measure(name, || decode(&iccp));
        println!("{name}: {growth} bytes");
        assert!(
            growth <= PIXELS + DECODER_SLACK,
            "{name} must not inflate the profile: {growth} bytes"
        );
    }

    for (kind, fields) in [
        (b"tEXt", &b"Comment\0"[..]),
        (b"zTXt", b"Comment\0\0"),
        (b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0"),
    ] {
        let text = metadata_png(&gray, kind, fields);
        let kind = std::str::from_utf8(kind).expect("ASCII chunk type");
        for (name, decode) in [
            ("decode_tip_image", tip_image as Decode),
            ("decode_tip_png", shape_png),
        ] {
            let name = format!("{name} gray PNG with {kind}");
            let growth = measure(&name, || decode(&text));
            println!("{name}: {growth} bytes");
            assert!(
                growth <= PIXELS + DECODER_SLACK,
                "{name} must not keep the text: {growth} bytes"
            );
        }
    }

    // `png` keeps an eXIf chunk whatever its options, in its chunk buffer and
    // in a copy. One of the 64 KiB a JPEG's EXIF holds decodes, and one of
    // 16 MiB does not.
    const EXIF: usize = 64 * 1024;
    let exif = with_chunk(&gray, b"eXIf", &[b'x'; EXIF]);
    let large_exif = metadata_png(&gray, b"eXIf", b"MM\0*");
    for (name, decode) in [
        ("decode_tip_image", tip_image as Decode),
        ("decode_tip_png", shape_png),
    ] {
        let name = format!("{name} gray PNG with eXIf");
        let growth = measure(&name, || decode(&exif));
        println!("{name}: {growth} bytes");
        assert!(
            growth <= PIXELS + DECODER_SLACK + 2 * EXIF,
            "{name} must hold at most two copies of the eXIf: {growth} bytes"
        );
    }
    let before = live();
    reset_peak();
    let image = decode_tip_image(&large_exif);
    let png = decode_tip_png(&large_exif);
    let growth = peak() - before;
    println!("gray PNG with a 16 MiB eXIf: {growth} bytes");
    assert!(image.is_err(), "decode_tip_image must reject a 16 MiB eXIf");
    assert!(png.is_err(), "decode_tip_png must reject a 16 MiB eXIf");
    assert!(
        growth <= DECODER_SLACK,
        "a rejected eXIf must not be kept: {growth} bytes"
    );

    let name = "decode_tip_image gray JPEG with gain-map segments";
    let jpeg = gain_map_jpeg();
    let growth = measure(name, || tip_image(&jpeg));
    println!("{name}: {growth} bytes");
    assert!(
        growth <= PIXELS + jpeg.len() + DECODER_SLACK,
        "{name} must hold at most one copy of its metadata: {growth} bytes"
    );
}
