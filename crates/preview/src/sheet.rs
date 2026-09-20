//! Contact sheets: a grid of tip thumbnails with optional name and size
//! labels, rendered as an 8-bit gray+alpha PNG.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

use crate::GrayscaleBitmap;

static LABEL_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn label_font() -> FontRef<'static> {
    use std::sync::OnceLock;
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| {
        FontRef::try_from_slice(LABEL_FONT_BYTES).expect("embedded Inter font must parse")
    })
    .clone()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SheetBackground {
    Transparent,
    #[default]
    White,
    Checker,
}

pub struct SheetBrush<'a> {
    pub name: &'a str,
    pub bitmap: &'a GrayscaleBitmap,
}

pub struct ContactSheetConfig {
    pub cell_size: u32,
    pub columns: Option<u32>,
    pub padding: u32,
    pub show_names: bool,
    /// Draws a size line under each thumbnail. The measurement is the tight
    /// bounds of ink greater than zero in the stored tip (`{w}×{h}`),
    /// not the canvas size — a padded square canvas still reports its content
    /// rectangle. An all-zero tip reads `Empty`.
    pub show_sizes: bool,
    pub gap: u32,
    pub background: SheetBackground,
}

impl Default for ContactSheetConfig {
    fn default() -> Self {
        Self {
            cell_size: 120,
            columns: None,
            padding: 8,
            show_names: false,
            show_sizes: true,
            gap: 8,
            background: SheetBackground::White,
        }
    }
}

pub fn generate_contact_sheet_png(
    brushes: &[SheetBrush],
    config: &ContactSheetConfig,
) -> Result<Vec<u8>, String> {
    use image::{ImageBuffer, LumaA};

    if brushes.is_empty() {
        let img: ImageBuffer<LumaA<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, LumaA([255, 0]));
        return encode_luma_alpha_png(&img);
    }

    let cols = config.columns.unwrap_or_else(|| {
        let count = brushes.len() as u32;
        count.clamp(1, 10)
    });
    let rows = (brushes.len() as u32).div_ceil(cols);

    let margin: u32 = 8;
    let gap: u32 = config.gap;
    let actual_cols = (brushes.len() as u32).min(cols);
    let img_width =
        2 * margin + actual_cols * config.cell_size + actual_cols.saturating_sub(1) * gap;
    let img_height = 2 * margin + rows * config.cell_size + rows.saturating_sub(1) * gap;

    let mut canvas: ImageBuffer<LumaA<u8>, Vec<u8>> = match config.background {
        SheetBackground::Transparent => {
            ImageBuffer::from_pixel(img_width, img_height, LumaA([255, 0]))
        }
        SheetBackground::White => ImageBuffer::from_pixel(img_width, img_height, LumaA([255, 255])),
        SheetBackground::Checker => {
            let sq: u32 = 16;
            let mut data: Vec<u8> = Vec::with_capacity((img_width * img_height * 2) as usize);
            for y in 0..img_height {
                for x in 0..img_width {
                    let is_dim = ((x / sq) + (y / sq)) % 2 == 1;
                    let v = if is_dim { 232 } else { 255 };
                    data.push(v);
                    data.push(255);
                }
            }
            ImageBuffer::from_vec(img_width, img_height, data)
                .expect("checker buffer length matches dimensions")
        }
    };

    let px_size = label_px_size(config.cell_size);
    let line_h = label_line_height(px_size);
    let name_line_h = if config.show_names { line_h } else { 0 };
    let size_line_h = if config.show_sizes { line_h } else { 0 };
    let label_gap: u32 = if config.show_names && config.show_sizes {
        (px_size * 0.15).ceil() as u32
    } else {
        0
    };
    let label_margin_top: u32 = if config.show_names || config.show_sizes {
        (px_size * 0.45).ceil() as u32
    } else {
        0
    };
    let label_block_h = name_line_h + label_gap + size_line_h + label_margin_top;

    for (i, brush) in brushes.iter().enumerate() {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        let cell_x = margin + col * (config.cell_size + gap);
        let cell_y = margin + row * (config.cell_size + gap);

        let available = config.cell_size.saturating_sub(2 * config.padding);
        if available == 0 {
            continue;
        }

        let avail_h = available.saturating_sub(label_block_h);
        if avail_h == 0 {
            continue;
        }

        let bitmap = brush.bitmap;

        let fit_x = available as f64 / bitmap.width as f64;
        let fit_y = avail_h as f64 / bitmap.height as f64;
        let fit = fit_x.min(fit_y).min(1.0);

        let thumb_w = ((bitmap.width as f64 * fit).round() as u32).max(1);
        let thumb_h = ((bitmap.height as f64 * fit).round() as u32).max(1);

        let offset_x = cell_x + config.padding + (available.saturating_sub(thumb_w)) / 2;
        let offset_y = cell_y + config.padding + (avail_h.saturating_sub(thumb_h)) / 2;

        let src_w = bitmap.width as f64;
        let src_h = bitmap.height as f64;
        let tw_f = thumb_w as f64;
        let th_f = thumb_h as f64;

        for ty in 0..thumb_h {
            let sy0 = (ty as f64 * src_h / th_f) as u32;
            let sy1 = (((ty + 1) as f64 * src_h / th_f) as u32).min(bitmap.height);
            for tx in 0..thumb_w {
                let sx0 = (tx as f64 * src_w / tw_f) as u32;
                let sx1 = (((tx + 1) as f64 * src_w / tw_f) as u32).min(bitmap.width);

                let mut sum = 0u32;
                let mut count = 0u32;
                for sy in sy0..sy1 {
                    let row_off = (sy * bitmap.width) as usize;
                    for sx in sx0..sx1 {
                        sum += bitmap.data[row_off + sx as usize] as u32;
                        count += 1;
                    }
                }
                #[allow(clippy::manual_checked_ops)]
                let ink = if count > 0 { (sum / count) as u8 } else { 0 };
                if ink == 0 {
                    continue;
                }

                let cx = offset_x + tx;
                let cy = offset_y + ty;
                if cx >= canvas.width() || cy >= canvas.height() {
                    continue;
                }

                let dst = canvas.get_pixel(cx, cy).0;
                let (dst_l, dst_a) = (dst[0] as u32, dst[1] as u32);
                let sa = ink as u32;
                let out_a = sa + dst_a * (255 - sa) / 255;
                #[allow(clippy::manual_checked_ops)]
                let out_l = if out_a == 0 {
                    0
                } else {
                    #[allow(clippy::erasing_op)]
                    let num = 0 * sa + dst_l * dst_a * (255 - sa) / 255;
                    num / out_a
                };
                canvas.put_pixel(cx, cy, LumaA([out_l as u8, out_a as u8]));
            }
        }

        let mut label_y = cell_y + config.padding + avail_h + label_margin_top;
        if config.show_names {
            let text = fit_label(brush.name, available, px_size);
            let lw = label_pixel_width(&text, px_size);
            let lx = cell_x + config.padding + (available.saturating_sub(lw)) / 2;
            draw_label(&mut canvas, lx as i64, label_y as i64, &text, px_size);
            label_y += name_line_h + label_gap;
        }
        if config.show_sizes {
            let raw = match ink_bounds(bitmap) {
                Some((w, h)) => format!("{w}×{h}"),
                None => "Empty".to_string(),
            };
            let text = fit_label(&raw, available, px_size);
            let lw = label_pixel_width(&text, px_size);
            let lx = cell_x + config.padding + (available.saturating_sub(lw)) / 2;
            draw_label(&mut canvas, lx as i64, label_y as i64, &text, px_size);
        }
    }

    encode_luma_alpha_png(&canvas)
}

fn encode_luma_alpha_png(
    img: &image::ImageBuffer<image::LumaA<u8>, Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("preview PNG encoding failed: {e}"))?;
    Ok(buf.into_inner())
}

/// Tight axis-aligned bounds of the stored tip's ink, in stored tip pixels.
///
/// A pixel counts as ink when its value is greater than zero: value-1 pixels
/// and detached specks are content. Returns `(width, height)` of that
/// rectangle, or `None` for an
/// all-zero, zero-sized or malformed bitmap. Allocation free.
fn ink_bounds(bitmap: &GrayscaleBitmap) -> Option<(u32, u32)> {
    let width = bitmap.width as usize;
    let height = bitmap.height as usize;
    if width == 0 || height == 0 || bitmap.data.len() != width * height {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..height {
        let row = &bitmap.data[y * width..(y + 1) * width];
        for (x, &v) in row.iter().enumerate() {
            if v > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == usize::MAX {
        return None;
    }
    Some(((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32))
}

const LABEL_COLOR: u8 = 110;

fn label_px_size(cell_size: u32) -> f32 {
    (cell_size as f32 / 11.0).clamp(11.0, 36.0)
}

fn label_line_height(px_size: f32) -> u32 {
    let font = label_font();
    let scaled = font.as_scaled(PxScale::from(px_size));
    (scaled.ascent() - scaled.descent() + scaled.line_gap()).ceil() as u32
}

fn label_pixel_width(text: &str, px_size: f32) -> u32 {
    let font = label_font();
    let scaled = font.as_scaled(PxScale::from(px_size));
    let mut width = 0.0_f32;
    let mut prev: Option<ab_glyph::GlyphId> = None;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        if let Some(pg) = prev {
            width += scaled.kern(pg, gid);
        }
        width += scaled.h_advance(gid);
        prev = Some(gid);
    }
    width.ceil() as u32
}

fn fit_label(text: &str, max_width: u32, px_size: f32) -> String {
    if label_pixel_width(text, px_size) <= max_width {
        return text.to_string();
    }
    let ellipsis = "…";
    let ell_w = label_pixel_width(ellipsis, px_size);
    if ell_w > max_width {
        return String::new();
    }
    let budget = max_width - ell_w;

    let font = label_font();
    let scaled = font.as_scaled(PxScale::from(px_size));

    let mut result = String::new();
    let mut cursor = 0.0_f32;
    let mut prev: Option<ab_glyph::GlyphId> = None;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let kern = prev.map(|pg| scaled.kern(pg, gid)).unwrap_or(0.0);
        let advance = scaled.h_advance(gid);
        if (cursor + kern + advance).ceil() as u32 > budget {
            break;
        }
        cursor += kern + advance;
        result.push(ch);
        prev = Some(gid);
    }
    while matches!(result.chars().last(), Some(' ') | Some('_')) {
        result.pop();
    }
    format!("{}{}", result, ellipsis)
}

fn draw_label(
    canvas: &mut image::ImageBuffer<image::LumaA<u8>, Vec<u8>>,
    x: i64,
    y: i64,
    text: &str,
    px_size: f32,
) {
    use ab_glyph::point;
    use image::LumaA;

    let font = label_font();
    let scaled = font.as_scaled(PxScale::from(px_size));
    let ascent = scaled.ascent();

    let mut cursor_x = x as f32;
    let mut prev: Option<ab_glyph::GlyphId> = None;

    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        if let Some(pg) = prev {
            cursor_x += scaled.kern(pg, gid);
        }
        let glyph =
            gid.with_scale_and_position(PxScale::from(px_size), point(cursor_x, y as f32 + ascent));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            let origin_x = bounds.min.x.floor() as i64;
            let origin_y = bounds.min.y.floor() as i64;
            outlined.draw(|dx, dy, coverage| {
                let px = origin_x + dx as i64;
                let py = origin_y + dy as i64;
                if px < 0 || py < 0 {
                    return;
                }
                let px = px as u32;
                let py = py as u32;
                if px >= canvas.width() || py >= canvas.height() {
                    return;
                }
                let sa = (coverage.clamp(0.0, 1.0) * 255.0) as u32;
                if sa == 0 {
                    return;
                }
                let dst = canvas.get_pixel(px, py).0;
                let (dl, da) = (dst[0] as u32, dst[1] as u32);
                let out_a = sa + da * (255 - sa) / 255;
                #[allow(clippy::manual_checked_ops)]
                let out_l = if out_a == 0 {
                    0
                } else {
                    let num = (LABEL_COLOR as u32) * sa + dl * da * (255 - sa) / 255;
                    num / out_a
                };
                canvas.put_pixel(px, py, LumaA([out_l as u8, out_a as u8]));
            });
        }
        cursor_x += scaled.h_advance(gid);
        prev = Some(gid);
    }
}

#[cfg(test)]
mod contact_sheet_tests {
    use super::*;

    #[test]
    fn checker_background_fill_is_stable() {
        let bitmap = GrayscaleBitmap {
            width: 4,
            height: 4,
            data: vec![0u8; 16],
        };
        let brushes = [SheetBrush {
            name: "x",
            bitmap: &bitmap,
        }];
        let config = ContactSheetConfig {
            cell_size: 40,
            columns: Some(1),
            padding: 4,
            show_names: false,
            show_sizes: false,
            gap: 0,
            background: SheetBackground::Checker,
        };

        let png = generate_contact_sheet_png(&brushes, &config).expect("sheet generated");
        let decoded = image::load_from_memory(&png).expect("decode sheet");
        let luma_a = decoded.to_luma_alpha8();
        let (w, h) = luma_a.dimensions();

        let sq: u32 = 16;
        for y in 0..h.min(sq) {
            for x in 0..w {
                let is_dim = ((x / sq) + (y / sq)) % 2 == 1;
                let v = if is_dim { 232 } else { 255 };
                assert_eq!(
                    luma_a.get_pixel(x, y).0,
                    [v, 255],
                    "checker pixel at ({x},{y}) drifted"
                );
            }
        }
    }

    fn bitmap(width: u32, height: u32, fill: &[(u32, u32, u8)]) -> GrayscaleBitmap {
        let mut data = vec![0u8; (width * height) as usize];
        for &(x, y, v) in fill {
            data[(y * width + x) as usize] = v;
        }
        GrayscaleBitmap {
            width,
            height,
            data,
        }
    }

    /// A `w` by `h` block of full ink at the top-left of a `cw` by `ch` canvas.
    fn block(cw: u32, ch: u32, w: u32, h: u32) -> GrayscaleBitmap {
        let mut data = vec![0u8; (cw * ch) as usize];
        for y in 0..h {
            for x in 0..w {
                data[(y * cw + x) as usize] = 255;
            }
        }
        GrayscaleBitmap {
            width: cw,
            height: ch,
            data,
        }
    }

    /// Center `bitmap` on a square canvas of black. A test-local input
    /// builder: `ink_bounds` must report the content, not the canvas.
    fn pad_to_square(bitmap: &mut GrayscaleBitmap) {
        let width = bitmap.width as usize;
        let height = bitmap.height as usize;
        if width == height || width == 0 || height == 0 || bitmap.data.len() != width * height {
            return;
        }
        let n = width.max(height);
        let left = (n - width) / 2;
        let top = (n - height) / 2;

        let mut out = vec![0u8; n * n];
        for y in 0..height {
            let src = &bitmap.data[y * width..(y + 1) * width];
            let dst_start = (top + y) * n + left;
            out[dst_start..dst_start + width].copy_from_slice(src);
        }

        bitmap.width = n as u32;
        bitmap.height = n as u32;
        bitmap.data = out;
    }

    #[test]
    fn ink_bounds_measures_content_not_canvas() {
        // Landscape content centered on a padded square canvas.
        let mut landscape = block(103, 80, 103, 80);
        pad_to_square(&mut landscape);
        assert_eq!((landscape.width, landscape.height), (103, 103));
        assert_eq!(ink_bounds(&landscape), Some((103, 80)));

        // Portrait content on the same square canvas.
        let mut portrait = block(80, 103, 80, 103);
        pad_to_square(&mut portrait);
        assert_eq!(ink_bounds(&portrait), Some((80, 103)));

        // Content that really fills a square canvas stays square.
        assert_eq!(ink_bounds(&block(64, 64, 64, 64)), Some((64, 64)));
    }

    #[test]
    fn ink_bounds_counts_faint_and_detached_pixels() {
        // A single value-1 pixel in the corner is content.
        assert_eq!(ink_bounds(&bitmap(10, 10, &[(9, 9, 1)])), Some((1, 1)));

        // Two detached specks: the bounds span both.
        assert_eq!(
            ink_bounds(&bitmap(10, 10, &[(1, 2, 1), (7, 5, 255)])),
            Some((7, 4))
        );
    }

    #[test]
    fn ink_bounds_rejects_empty_and_malformed_bitmaps() {
        assert_eq!(ink_bounds(&bitmap(8, 8, &[])), None, "all-zero tip");
        assert_eq!(
            ink_bounds(&GrayscaleBitmap {
                width: 0,
                height: 4,
                data: Vec::new(),
            }),
            None,
            "zero-sized tip"
        );
        assert_eq!(
            ink_bounds(&GrayscaleBitmap {
                width: 4,
                height: 4,
                data: vec![255; 7],
            }),
            None,
            "data length disagrees with dimensions"
        );
    }

    #[test]
    fn longest_realistic_size_label_fits_every_published_cell() {
        let label = "2500×1667";
        for cell in [100u32, 120, 200] {
            let available = cell - 16; // cell minus 2 * the default 8px padding
            let width = label_pixel_width(label, label_px_size(cell));
            assert!(
                width <= available,
                "{label:?} needs {width}px but cell {cell} offers {available}px"
            );
        }
    }

    /// Renders one brush in a 100px cell and counts label text row bands.
    fn label_band_count(bitmap: &GrayscaleBitmap, show_sizes: bool) -> usize {
        let brushes = [SheetBrush { name: "x", bitmap }];
        let config = ContactSheetConfig {
            cell_size: 100,
            columns: Some(1),
            padding: 8,
            show_names: false,
            show_sizes,
            gap: 8,
            background: SheetBackground::White,
        };
        let png = generate_contact_sheet_png(&brushes, &config).expect("sheet generated");
        let luma_a = image::load_from_memory(&png)
            .expect("decode sheet")
            .to_luma_alpha8();
        let (w, h) = luma_a.dimensions();
        let mut bands = 0;
        let mut in_band = false;
        for y in 0..h {
            let has_label = (0..w).any(|x| {
                let v = luma_a.get_pixel(x, y).0[0];
                (LABEL_COLOR..255).contains(&v)
            });
            if has_label && !in_band {
                bands += 1;
            }
            in_band = has_label;
        }
        bands
    }

    #[test]
    fn show_sizes_false_draws_no_label_line() {
        let tip = block(32, 32, 20, 12);
        assert_eq!(label_band_count(&tip, false), 0, "no label when sizes off");
        assert_eq!(label_band_count(&tip, true), 1, "one size line when on");
    }
}
