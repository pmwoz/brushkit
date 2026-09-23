use brushkit_abr::TipBitmap;
use image::{ImageBuffer, ImageDecoder, Rgba, RgbaImage};
use std::io::Cursor;

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
    /// A side over `MAX_IMPORT_DIMENSION`, or a decoded buffer, in the image's
    /// own pixel format, over 512 MiB.
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
                "image is {width}x{height}px; a brush tip must be at most {MAX_IMPORT_DIMENSION}px per side and {} MiB decoded in its own pixel format",
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
    use image::GenericImageView;

    let img = decode_guarded(bytes, MAX_IMPORT_DIMENSION, MAX_IMPORT_DECODED_BYTES).map_err(
        |e| match e {
            GuardedDecodeError::TooLarge { width, height } => {
                TipImageError::TooLarge { width, height }
            }
            GuardedDecodeError::Decode(e) => TipImageError::Decode(e.to_string()),
        },
    )?;
    let (width, height) = img.dimensions();

    let data: Vec<u8> = if img.color().has_alpha() {
        img.into_luma_alpha8().pixels().map(|p| p.0[1]).collect()
    } else {
        let mut data = img.into_luma8().into_raw();
        data.iter_mut().for_each(|v| *v = 255 - *v);
        data
    };

    Ok(GrayscaleBitmap {
        width,
        height,
        data,
    })
}

/// Why `decode_guarded` returned no image.
pub(crate) enum GuardedDecodeError {
    /// A side over `max_side`, or a decoded buffer over `max_bytes`.
    TooLarge {
        width: u32,
        height: u32,
    },
    Decode(image::ImageError),
}

/// Decode an image of at most `max_side` px per side and `max_bytes` decoded
/// in its own pixel format. Oversize is decided from the header before any
/// pixels are decoded.
pub(crate) fn decode_guarded(
    bytes: &[u8],
    max_side: u32,
    max_bytes: u64,
) -> Result<image::DynamicImage, GuardedDecodeError> {
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
    if limits.reserve(decoder.total_bytes()).is_err() {
        let (width, height) = decoder.dimensions();
        return Err(GuardedDecodeError::TooLarge { width, height });
    }
    decoder
        .set_limits(limits)
        .map_err(GuardedDecodeError::Decode)?;
    image::DynamicImage::from_decoder(decoder).map_err(GuardedDecodeError::Decode)
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
