use brushkit_abr::TipBitmap;
use image::{ColorType, ImageBuffer, ImageDecoder, Rgba, RgbaImage};
use std::io::Cursor;
use zune_jpeg::SampleRatios;

#[derive(Debug, Clone)]
pub struct GrayscaleBitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub fn to_grayscale(tip: &TipBitmap) -> GrayscaleBitmap {
    match tip.depth {
        8 => GrayscaleBitmap {
            width: tip.width,
            height: tip.height,
            data: tip.data.clone(),
        },
        16 => {
            let data: Vec<u8> = tip
                .data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| pair[0])
                .collect();
            GrayscaleBitmap {
                width: tip.width,
                height: tip.height,
                data,
            }
        }
        _ => GrayscaleBitmap {
            width: tip.width,
            height: tip.height,
            data: tip.data.clone(),
        },
    }
}

/// Both formats use the same polarity — a `Shape.png` is white=stamp coverage
/// and a `TipBitmap` is 255=full ink — so the pixels are copied unchanged.
pub fn tip_bitmap_of(bitmap: &GrayscaleBitmap) -> TipBitmap {
    TipBitmap {
        width: bitmap.width,
        height: bitmap.height,
        depth: 8,
        data: bitmap.data.clone(),
    }
}

pub const MAX_IMPORT_DIMENSION: u32 = 16384;
/// Equal to `image`'s default `max_alloc`, so every image that decoded
/// through `image::load_from_memory` still decodes.
const MAX_IMPORT_DECODED_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub enum TipImageError {
    Decode(String),
    /// A side over `MAX_IMPORT_DIMENSION`, or a decode over 512 MiB: the image
    /// in its own pixel format, plus the DCT coefficients of a progressive JPEG.
    TooLarge {
        width: u32,
        height: u32,
    },
}

impl std::fmt::Display for TipImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TipImageError::Decode(msg) => f.write_str(msg),
            TipImageError::TooLarge { width, height } => write!(
                f,
                "image is {width}x{height}px; a brush tip must be at most {MAX_IMPORT_DIMENSION}px per side and {} MiB to decode",
                MAX_IMPORT_DECODED_BYTES / (1024 * 1024)
            ),
        }
    }
}

impl std::error::Error for TipImageError {}

/// Decode an imported brush-tip image (PNG/JPEG bytes) into a grayscale tip
/// buffer. Alpha becomes intensity when the image has an alpha channel;
/// otherwise luminance is inverted (dark = opaque), matching Photoshop's
/// convention.
pub fn decode_tip_image(bytes: &[u8]) -> Result<GrayscaleBitmap, TipImageError> {
    let img = decode_guarded(bytes, MAX_IMPORT_DIMENSION, MAX_IMPORT_DECODED_BYTES).map_err(
        |e| match e {
            GuardedDecodeError::TooLarge { width, height } => {
                TipImageError::TooLarge { width, height }
            }
            GuardedDecodeError::Decode(e) => TipImageError::Decode(e.to_string()),
        },
    )?;
    Ok(GrayscaleBitmap {
        width: img.width,
        height: img.height,
        data: tip_plane(img, TipSample::Coverage),
    })
}

/// A decoded image in its own pixel format. 16-bit and float samples are in
/// native byte order, as `ImageDecoder::read_image` writes them.
pub(crate) struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub bytes: Vec<u8>,
}

/// Every pixel format `image` 0.25 decodes to. The float formats come from
/// decoders a dependent crate may enable on the shared `image` dependency.
#[derive(Clone, Copy)]
pub(crate) enum PixelFormat {
    L8,
    La8,
    Rgb8,
    Rgba8,
    L16,
    La16,
    Rgb16,
    Rgba16,
    Rgb32F,
    Rgba32F,
}

impl PixelFormat {
    fn of(color: ColorType) -> Option<Self> {
        Some(match color {
            ColorType::L8 => Self::L8,
            ColorType::La8 => Self::La8,
            ColorType::Rgb8 => Self::Rgb8,
            ColorType::Rgba8 => Self::Rgba8,
            ColorType::L16 => Self::L16,
            ColorType::La16 => Self::La16,
            ColorType::Rgb16 => Self::Rgb16,
            ColorType::Rgba16 => Self::Rgba16,
            ColorType::Rgb32F => Self::Rgb32F,
            ColorType::Rgba32F => Self::Rgba32F,
            _ => return None,
        })
    }

    fn color_type(self) -> ColorType {
        match self {
            Self::L8 => ColorType::L8,
            Self::La8 => ColorType::La8,
            Self::Rgb8 => ColorType::Rgb8,
            Self::Rgba8 => ColorType::Rgba8,
            Self::L16 => ColorType::L16,
            Self::La16 => ColorType::La16,
            Self::Rgb16 => ColorType::Rgb16,
            Self::Rgba16 => ColorType::Rgba16,
            Self::Rgb32F => ColorType::Rgb32F,
            Self::Rgba32F => ColorType::Rgba32F,
        }
    }

    fn bytes_per_pixel(self) -> usize {
        usize::from(self.color_type().bytes_per_pixel())
    }
}

/// What a tip keeps of each pixel.
#[derive(Clone, Copy)]
pub(crate) enum TipSample {
    /// Alpha when the image has it, otherwise inverted luma, so dark is opaque.
    Coverage,
    Luma,
}

/// One byte per pixel, written over the front of the decoded buffer so the tip
/// allocates no second buffer. Values are `image`'s own `into_luma8` and
/// `into_luma_alpha8` conversions. The final shrink is a `realloc`, measured in
/// place from 16 to 512 MiB on glibc and both wasm32 targets, and on macOS
/// except a 16 MiB block shrunk to 2 MiB. An allocator that moves the block
/// adds the tip's size to the peak.
pub(crate) fn tip_plane(image: DecodedImage, sample: TipSample) -> Vec<u8> {
    const CHUNK: usize = 4096;
    let format = image.format;
    let bytes_per_pixel = format.bytes_per_pixel();
    let coverage = matches!(sample, TipSample::Coverage);
    let alpha = coverage && format.color_type().has_alpha();
    let mut raw = image.bytes;
    let pixels = raw.len() / bytes_per_pixel;
    if alpha && matches!(format, PixelFormat::La8 | PixelFormat::Rgba8) {
        for i in 0..pixels {
            raw[i] = raw[i * bytes_per_pixel + bytes_per_pixel - 1];
        }
    } else if bytes_per_pixel > 1 {
        // Chunk `start..end` is written to bytes `start..end`, which precede
        // its own source bytes, so no unread pixel is overwritten.
        for start in (0..pixels).step_by(CHUNK) {
            let end = (start + CHUNK).min(pixels);
            let chunk = &raw[start * bytes_per_pixel..end * bytes_per_pixel];
            let plane = convert_chunk(format, alpha, chunk);
            raw[start..end].copy_from_slice(&plane);
        }
    }
    raw.truncate(pixels);
    raw.shrink_to_fit();
    if coverage && !alpha {
        raw.iter_mut().for_each(|v| *v = 255 - *v);
    }
    raw
}

/// One byte per pixel of `chunk`, alpha when `alpha` and luma otherwise,
/// through a one-row image so `image` does the conversion.
fn convert_chunk(format: PixelFormat, alpha: bool, chunk: &[u8]) -> Vec<u8> {
    use image::DynamicImage::*;
    let width = (chunk.len() / format.bytes_per_pixel()) as u32;
    let u8s = || chunk.to_vec();
    let u16s = || {
        let samples = chunk.as_chunks::<2>().0.iter();
        samples.map(|s| u16::from_ne_bytes(*s)).collect()
    };
    let f32s = || {
        let samples = chunk.as_chunks::<4>().0.iter();
        samples.map(|s| f32::from_ne_bytes(*s)).collect()
    };
    let pixels = match format {
        PixelFormat::L8 => ImageBuffer::from_raw(width, 1, u8s()).map(ImageLuma8),
        PixelFormat::La8 => ImageBuffer::from_raw(width, 1, u8s()).map(ImageLumaA8),
        PixelFormat::Rgb8 => ImageBuffer::from_raw(width, 1, u8s()).map(ImageRgb8),
        PixelFormat::Rgba8 => ImageBuffer::from_raw(width, 1, u8s()).map(ImageRgba8),
        PixelFormat::L16 => ImageBuffer::from_raw(width, 1, u16s()).map(ImageLuma16),
        PixelFormat::La16 => ImageBuffer::from_raw(width, 1, u16s()).map(ImageLumaA16),
        PixelFormat::Rgb16 => ImageBuffer::from_raw(width, 1, u16s()).map(ImageRgb16),
        PixelFormat::Rgba16 => ImageBuffer::from_raw(width, 1, u16s()).map(ImageRgba16),
        PixelFormat::Rgb32F => ImageBuffer::from_raw(width, 1, f32s()).map(ImageRgb32F),
        PixelFormat::Rgba32F => ImageBuffer::from_raw(width, 1, f32s()).map(ImageRgba32F),
    }
    .expect("a chunk holds whole pixels");
    if alpha {
        let luma_alpha = pixels.into_luma_alpha8().into_raw();
        luma_alpha.into_iter().skip(1).step_by(2).collect()
    } else {
        pixels.into_luma8().into_raw()
    }
}

/// Why `decode_guarded` returned no image.
pub(crate) enum GuardedDecodeError {
    /// A side over `max_side`, or a decode over `max_bytes`.
    TooLarge {
        width: u32,
        height: u32,
    },
    Decode(image::ImageError),
}

/// Decode an image of at most `max_side` px per side and `max_bytes` decoded:
/// the image in its own pixel format, plus the DCT coefficients of a
/// progressive JPEG. Oversize is decided from the header before any pixels are
/// decoded.
pub(crate) fn decode_guarded(
    bytes: &[u8],
    max_side: u32,
    max_bytes: u64,
) -> Result<DecodedImage, GuardedDecodeError> {
    if let Some((width, height)) = header_dimensions(bytes) {
        if width > max_side || height > max_side {
            return Err(GuardedDecodeError::TooLarge { width, height });
        }
    }
    // On 32-bit targets `png` rejects an output buffer over `isize::MAX` while
    // `image` builds its decoder, so an over-budget PNG is sized from IHDR first.
    // IHDR undercounts indexed color, which `image` expands to RGB or RGBA.
    if let Some(info) = png_header(bytes) {
        let min_decoded =
            u64::from(info.width) * u64::from(info.height) * info.bytes_per_pixel() as u64;
        if min_decoded > max_bytes {
            return Err(GuardedDecodeError::TooLarge {
                width: info.width,
                height: info.height,
            });
        }
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| GuardedDecodeError::Decode(image::ImageError::IoError(e)))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
    limits.max_alloc = Some(max_bytes);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(GuardedDecodeError::Decode)?;
    // What `ImageReader::decode` does, with the budget failure reported as
    // `TooLarge`: the output buffer is reserved and the decoder keeps the rest.
    // `image` does not pass the rest to zune-jpeg, so the coefficients of a
    // progressive JPEG are reserved here too.
    let decode_bytes = decoder
        .total_bytes()
        .saturating_add(progressive_jpeg_coefficient_bytes(bytes));
    if limits.reserve(decode_bytes).is_err() {
        let (width, height) = decoder.dimensions();
        return Err(GuardedDecodeError::TooLarge { width, height });
    }
    decoder
        .set_limits(limits)
        .map_err(GuardedDecodeError::Decode)?;
    let (width, height) = decoder.dimensions();
    let color = decoder.color_type();
    let format = PixelFormat::of(color).ok_or_else(|| {
        GuardedDecodeError::Decode(image::ImageError::Unsupported(
            image::error::UnsupportedError::from_format_and_kind(
                image::error::ImageFormatHint::Unknown,
                image::error::UnsupportedErrorKind::Color(color.into()),
            ),
        ))
    })?;
    // One buffer in the decoder's own format, so a converting tip reuses it.
    let total_bytes = usize::try_from(decoder.total_bytes())
        .map_err(|_| GuardedDecodeError::TooLarge { width, height })?;
    let mut bytes = vec![0; total_bytes];
    decoder
        .read_image(&mut bytes)
        .map_err(GuardedDecodeError::Decode)?;
    Ok(DecodedImage {
        width,
        height,
        format,
        bytes,
    })
}

/// Reads header dimensions without allocating pixels or applying decode limits.
/// PNG goes through the `png` header alone: `image` sizes the output buffer
/// before it reports dimensions, which fails on 32-bit targets for large ones.
pub(crate) fn header_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if image::guess_format(bytes).ok()? == image::ImageFormat::Png {
        let info = png_header(bytes)?;
        return Some((info.width, info.height));
    }
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// The DCT coefficients zune-jpeg holds for a progressive JPEG until its last
/// scan: 2 bytes per sample, with each side padded to whole MCUs. zune-jpeg
/// reports only the largest sampling factors, so every component is counted at
/// full resolution, and 4:2:0 at twice its real size. 0 for any other image.
fn progressive_jpeg_coefficient_bytes(bytes: &[u8]) -> u64 {
    if image::guess_format(bytes).ok() != Some(image::ImageFormat::Jpeg) {
        return 0;
    }
    // The options `image` decodes with.
    let options = zune_jpeg::zune_core::options::DecoderOptions::default()
        .set_strict_mode(false)
        .set_max_width(usize::MAX)
        .set_max_height(usize::MAX);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
        zune_jpeg::zune_core::bytestream::ZCursor::new(bytes),
        options,
    );
    if decoder.decode_headers().is_err() {
        return 0;
    }
    let Some(info) = decoder.info() else {
        return 0;
    };
    if !info.sof.is_progressive() {
        return 0;
    }
    let (h_max, v_max) = match info.sample_ratio {
        SampleRatios::None => (1, 1),
        SampleRatios::H => (2, 1),
        SampleRatios::V => (1, 2),
        SampleRatios::HV => (2, 2),
        SampleRatios::Generic(h, v) => (h as u64, v as u64),
    };
    let padded = |side: u16, factor: u64| u64::from(side).div_ceil(8 * factor) * 8 * factor;
    2 * u64::from(info.components) * padded(info.width, h_max) * padded(info.height, v_max)
}

/// A PNG's IHDR, parsed without allocating pixels. `None` for other formats
/// and for a PNG whose header does not parse.
fn png_header(bytes: &[u8]) -> Option<png::Info<'static>> {
    if image::guess_format(bytes).ok()? != image::ImageFormat::Png {
        return None;
    }
    png::Decoder::new(Cursor::new(bytes))
        .read_header_info()
        .ok()
        .cloned()
}

/// Box-filter `bitmap` so its larger side is at most `max_side` (>= 1). Returns
/// a clone when it already fits. Average of the covered source pixels per
/// destination pixel, the same filter `generate_preview_png` uses.
pub fn downsample(bitmap: &GrayscaleBitmap, max_side: u32) -> GrayscaleBitmap {
    let max_side = max_side.max(1);

    let (tw, th) = if bitmap.width > max_side || bitmap.height > max_side {
        let scale = f64::min(
            max_side as f64 / bitmap.width as f64,
            max_side as f64 / bitmap.height as f64,
        );
        let w = ((bitmap.width as f64 * scale).round() as u32).max(1);
        let h = ((bitmap.height as f64 * scale).round() as u32).max(1);
        (w, h)
    } else {
        return bitmap.clone();
    };

    let src_w = bitmap.width as f64;
    let src_h = bitmap.height as f64;
    let tw_f = tw as f64;
    let th_f = th as f64;

    let mut data = Vec::with_capacity((tw as usize) * (th as usize));
    for y in 0..th {
        let sy0 = (y as f64 * src_h / th_f) as u32;
        let sy1 = (((y + 1) as f64 * src_h / th_f) as u32).min(bitmap.height);
        for x in 0..tw {
            let sx0 = (x as f64 * src_w / tw_f) as u32;
            let sx1 = (((x + 1) as f64 * src_w / tw_f) as u32).min(bitmap.width);

            let mut sum = 0u32;
            let mut count = 0u32;
            for sy in sy0..sy1 {
                let row_offset = (sy * bitmap.width) as usize;
                for sx in sx0..sx1 {
                    sum += bitmap.data[row_offset + sx as usize] as u32;
                    count += 1;
                }
            }

            #[allow(clippy::manual_checked_ops)]
            let value = if count > 0 { (sum / count) as u8 } else { 0 };
            data.push(value);
        }
    }

    GrayscaleBitmap {
        width: tw,
        height: th,
        data,
    }
}

/// A 200 px RGBA thumbnail: black ink, alpha from the tip's gray value.
pub fn generate_preview_png(bitmap: &GrayscaleBitmap) -> Result<Vec<u8>, String> {
    let small = downsample(bitmap, 200);

    let img: RgbaImage = ImageBuffer::from_fn(small.width, small.height, |x, y| {
        let alpha = small.data[(y * small.width + x) as usize];
        Rgba([0, 0, 0, alpha])
    });

    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("thumbnail encoding failed: {e}"))?;

    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_grayscale_16bit_takes_high_byte() {
        let tip = TipBitmap {
            width: 2,
            height: 1,
            depth: 16,
            data: vec![0xAB, 0x12, 0xCD, 0x34],
        };
        let result = to_grayscale(&tip);
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 1);
        assert_eq!(result.data.len(), 2);
        assert_eq!(result.data, vec![0xAB, 0xCD], "must keep the high byte");
    }

    #[test]
    fn downsample_averages_covered_pixels() {
        let bitmap = GrayscaleBitmap {
            width: 4,
            height: 2,
            data: vec![0, 255, 0, 255, 0, 255, 0, 255],
        };
        let small = downsample(&bitmap, 2);
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(small.data, vec![127, 127]);
    }

    #[test]
    fn downsample_keeps_a_fitting_bitmap() {
        let bitmap = GrayscaleBitmap {
            width: 3,
            height: 2,
            data: vec![1, 2, 3, 4, 5, 6],
        };
        let same = downsample(&bitmap, 8);
        assert_eq!((same.width, same.height), (3, 2));
        assert_eq!(same.data, bitmap.data);
    }

    #[test]
    fn preview_png_is_200_wide_with_alpha_from_gray() {
        let bitmap = GrayscaleBitmap {
            width: 400,
            height: 100,
            data: vec![200u8; 400 * 100],
        };
        let png = generate_preview_png(&bitmap).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!((img.width(), img.height()), (200, 50));
        assert!(img.pixels().all(|p| p.0 == [0, 0, 0, 200]));
    }
}

#[cfg(test)]
mod tip_image_tests {
    use super::*;

    fn rgba_png(width: u32, height: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
        let img: RgbaImage =
            ImageBuffer::from_fn(width, height, |x, y| Rgba(pixels[(y * width + x) as usize]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    fn luma_png(width: u32, height: u32, luma: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, width, height);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(luma).unwrap();
        }
        buf
    }

    #[test]
    fn alpha_channel_becomes_intensity() {
        let png = rgba_png(3, 1, &[[255, 0, 0, 255], [255, 0, 0, 128], [255, 0, 0, 0]]);
        let bitmap = decode_tip_image(&png).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (3, 1));
        assert_eq!(bitmap.data, vec![255, 128, 0]);
    }

    #[test]
    fn opaque_image_inverts_luminance() {
        let png = luma_png(3, 1, &[0, 40, 255]);
        let bitmap = decode_tip_image(&png).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (3, 1));
        assert_eq!(bitmap.data, vec![255, 215, 0]);
    }

    #[test]
    fn oversized_image_is_rejected_with_the_ceiling_message() {
        let over = MAX_IMPORT_DIMENSION + 1;
        let png = luma_png(over, 1, &vec![0u8; over as usize]);
        let err = decode_tip_image(&png).expect_err("over-ceiling image must be rejected");
        assert!(
            matches!(err, TipImageError::TooLarge { width, height } if width == over && height == 1),
            "expected TooLarge, got {err:?}"
        );
        assert!(
            err.to_string().starts_with("image is "),
            "unexpected message: {err}"
        );
    }

    /// Every decodable pixel format, filled with varied samples, over several
    /// conversion chunks.
    fn every_format() -> [(PixelFormat, image::DynamicImage); 10] {
        let base = image::DynamicImage::ImageRgba16(ImageBuffer::from_fn(100, 100, |x, y| {
            let i = (y * 100 + x) * 4;
            image::Rgba([0, 1, 2, 3].map(|c| ((i + c).wrapping_mul(0x9E37_79B9) >> 16) as u16))
        }));
        [
            (PixelFormat::L8, base.to_luma8().into()),
            (PixelFormat::La8, base.to_luma_alpha8().into()),
            (PixelFormat::Rgb8, base.to_rgb8().into()),
            (PixelFormat::Rgba8, base.to_rgba8().into()),
            (PixelFormat::L16, base.to_luma16().into()),
            (PixelFormat::La16, base.to_luma_alpha16().into()),
            (PixelFormat::Rgb16, base.to_rgb16().into()),
            (PixelFormat::Rgb32F, base.to_rgb32f().into()),
            (PixelFormat::Rgba32F, base.to_rgba32f().into()),
            (PixelFormat::Rgba16, base),
        ]
    }

    #[test]
    fn tips_match_image_conversions_for_every_format() {
        for (format, image) in every_format() {
            let name = format!("{:?}", image.color());
            let coverage = if image.color().has_alpha() {
                image.to_luma_alpha8().pixels().map(|p| p.0[1]).collect()
            } else {
                let luma = image.to_luma8().into_raw();
                luma.into_iter().map(|v| 255 - v).collect::<Vec<_>>()
            };
            let luma = image.to_luma8().into_raw();
            let decoded = |image: &image::DynamicImage| DecodedImage {
                width: image.width(),
                height: image.height(),
                format,
                bytes: image.as_bytes().to_vec(),
            };
            assert_eq!(
                tip_plane(decoded(&image), TipSample::Coverage),
                coverage,
                "coverage {name}"
            );
            assert_eq!(
                tip_plane(decoded(&image), TipSample::Luma),
                luma,
                "luma {name}"
            );
            if !matches!(format, PixelFormat::Rgb32F | PixelFormat::Rgba32F) {
                let mut png = std::io::Cursor::new(Vec::new());
                image.write_to(&mut png, image::ImageFormat::Png).unwrap();
                let png = png.into_inner();
                assert_eq!(
                    decode_tip_image(&png).unwrap().data,
                    coverage,
                    "decode_tip_image {name}"
                );
                assert_eq!(
                    crate::procreate::decode_tip_png(&png).unwrap().data,
                    luma,
                    "decode_tip_png {name}"
                );
            }
        }
    }

    #[test]
    fn garbage_bytes_report_a_decode_error() {
        let err = decode_tip_image(b"not an image").expect_err("garbage must not decode");
        assert!(matches!(err, TipImageError::Decode(_)), "got {err:?}");
    }
}

#[cfg(test)]
mod tip_bitmap_of_tests {
    use super::*;

    #[test]
    fn tip_bitmap_of_copies_pixels_and_declares_depth_8() {
        let bitmap = GrayscaleBitmap {
            width: 3,
            height: 2,
            data: vec![0, 17, 255, 128, 1, 0],
        };
        let tip = tip_bitmap_of(&bitmap);
        assert_eq!((tip.width, tip.height), (3, 2));
        assert_eq!(tip.depth, 8, "GrayscaleBitmap is 8-bit by construction");
        assert_eq!(tip.data, bitmap.data, "polarity matches, so no conversion");
    }

    #[test]
    fn tip_bitmap_of_round_trips_an_8_bit_tip() {
        let tip = TipBitmap {
            width: 2,
            height: 2,
            depth: 8,
            data: vec![9, 200, 0, 255],
        };
        let back = tip_bitmap_of(&to_grayscale(&tip));
        assert_eq!((back.width, back.height, back.depth), (2, 2, 8));
        assert_eq!(back.data, tip.data);
    }
}
