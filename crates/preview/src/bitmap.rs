//! Grayscale tip bitmaps: the common currency between a parsed `.abr` tip, a
//! Procreate `Shape.png` and anything this crate renders.

use brushkit_abr::TipBitmap;
use image::{ImageBuffer, Rgba, RgbaImage};

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

#[derive(Debug)]
pub enum TipImageError {
    Decode(String),
    TooLarge { width: u32, height: u32 },
}

impl std::fmt::Display for TipImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TipImageError::Decode(msg) => f.write_str(msg),
            TipImageError::TooLarge { width, height } => write!(
                f,
                "image is {width}x{height}px; the maximum supported brush-tip dimension is {MAX_IMPORT_DIMENSION}px"
            ),
        }
    }
}

impl std::error::Error for TipImageError {}

/// Decode an imported brush-tip image (PNG/JPEG bytes) into a grayscale tip
/// buffer. Alpha becomes intensity when the image has an alpha channel;
/// otherwise luminance is inverted (dark = opaque), matching Photoshop's
/// convention.
///
/// Takes bytes rather than a path because this crate also compiles to wasm32,
/// where `std::fs` is not usable.
pub fn decode_tip_image(bytes: &[u8]) -> Result<GrayscaleBitmap, TipImageError> {
    use image::GenericImageView;

    let img = image::load_from_memory(bytes).map_err(|e| TipImageError::Decode(e.to_string()))?;
    let (width, height) = img.dimensions();
    if width > MAX_IMPORT_DIMENSION || height > MAX_IMPORT_DIMENSION {
        return Err(TipImageError::TooLarge { width, height });
    }

    let data: Vec<u8> = if img.color().has_alpha() {
        img.to_rgba8().pixels().map(|p| p.0[3]).collect()
    } else {
        img.to_luma8().pixels().map(|p| 255 - p.0[0]).collect()
    };

    Ok(GrayscaleBitmap {
        width,
        height,
        data,
    })
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
