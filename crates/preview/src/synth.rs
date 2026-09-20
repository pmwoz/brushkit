use crate::GrayscaleBitmap;

const MAX_TIP_PX: f64 = 4096.0;

pub fn can_synthesize(geom: &brushkit_abr::ComputedGeometry) -> bool {
    geom.diameter_px.is_some_and(|d| d >= 1.0)
}

/// Rasterize a monochrome elliptical tip from computed-brush geometry.
///
/// `None` iff `can_synthesize` is false. Output is single-channel grayscale,
/// white=stamp (`255`) / black=transparent (`0`).
pub fn synthesize_computed_tip(geom: &brushkit_abr::ComputedGeometry) -> Option<GrayscaleBitmap> {
    if !can_synthesize(geom) {
        return None;
    }

    let diameter = geom.diameter_px.unwrap().clamp(1.0, MAX_TIP_PX);
    let hardness = geom.hardness_pct.unwrap_or(100.0).clamp(0.0, 100.0);
    let angle = geom.angle_deg.unwrap_or(0.0);
    let roundness = geom.roundness_pct.unwrap_or(100.0).clamp(0.0, 100.0);

    let side = (diameter.ceil() as u32) + 2;
    let center = side as f64 / 2.0;

    let a = diameter / 2.0;
    let b = (a * roundness / 100.0).max(0.5);

    let h = hardness / 100.0;
    let h_eff = h.min(1.0 - 1.0 / a).max(0.0);

    // Photoshop's `Angl` is CCW-positive in canvas (y-up) orientation; the
    // bitmap's y axis points down, so the angle is negated to rotate the tip
    // the way Photoshop does.
    let (sin, cos) = (-angle).to_radians().sin_cos();

    let k = falloff_k(hardness);

    let mut data = vec![0u8; (side * side) as usize];
    for y in 0..side {
        for x in 0..side {
            let dx = (x as f64 + 0.5) - center;
            let dy = (y as f64 + 0.5) - center;
            let u = dx * cos + dy * sin;
            let v = -dx * sin + dy * cos;
            let r = ((u / a).powi(2) + (v / b).powi(2)).sqrt();
            let alpha = if r <= h_eff {
                1.0
            } else if r >= 1.0 {
                0.0
            } else {
                let t = (r - h_eff) / (1.0 - h_eff);
                (-k * t * t).exp()
            };
            data[(y * side + x) as usize] = (alpha * 255.0).round() as u8;
        }
    }

    Some(GrayscaleBitmap {
        width: side,
        height: side,
        data,
    })
}

fn falloff_k(hardness_pct: f64) -> f64 {
    const KNOTS: [(f64, f64); 2] = [(0.0, 2.29), (50.0, 1.14)];
    let h = hardness_pct.clamp(KNOTS[0].0, KNOTS[KNOTS.len() - 1].0);
    for w in KNOTS.windows(2) {
        let (h0, k0) = w[0];
        let (h1, k1) = w[1];
        if h <= h1 {
            return k0 + (h - h0) / (h1 - h0) * (k1 - k0);
        }
    }
    KNOTS[KNOTS.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;
    use brushkit_abr::ComputedGeometry;

    fn geom(
        diameter_px: Option<f64>,
        hardness_pct: Option<f64>,
        angle_deg: Option<f64>,
        roundness_pct: Option<f64>,
    ) -> ComputedGeometry {
        ComputedGeometry {
            diameter_px,
            hardness_pct,
            angle_deg,
            roundness_pct,
        }
    }

    fn nonzero_in_row(bmp: &GrayscaleBitmap) -> usize {
        let y = bmp.height / 2;
        (0..bmp.width)
            .filter(|&x| bmp.data[(y * bmp.width + x) as usize] != 0)
            .count()
    }

    fn nonzero_in_col(bmp: &GrayscaleBitmap) -> usize {
        let x = bmp.width / 2;
        (0..bmp.height)
            .filter(|&y| bmp.data[(y * bmp.width + x) as usize] != 0)
            .count()
    }

    fn pixel_at(bmp: &GrayscaleBitmap, x: u32, y: u32) -> u8 {
        bmp.data[(y * bmp.width + x) as usize]
    }

    #[test]
    fn canvas_side_from_diameter() {
        let bmp = synthesize_computed_tip(&geom(Some(30.0), None, None, Some(100.0))).unwrap();
        assert_eq!(bmp.width, 32);
        assert_eq!(bmp.height, 32);
    }

    #[test]
    fn white_is_stamp_black_is_transparent() {
        let bmp =
            synthesize_computed_tip(&geom(Some(30.0), Some(100.0), None, Some(100.0))).unwrap();
        let mid = bmp.height / 2;
        let center = bmp.data[(mid * bmp.width + bmp.width / 2) as usize];
        assert!(center >= 250, "centre pixel was {center}, expected stamp");
        assert_eq!(bmp.data[0], 0, "corner pixel should be transparent");
    }

    #[test]
    fn roundness_shrinks_minor_axis() {
        let bmp = synthesize_computed_tip(&geom(Some(40.0), None, Some(0.0), Some(50.0))).unwrap();
        let row = nonzero_in_row(&bmp);
        let col = nonzero_in_col(&bmp);
        assert!((38..=42).contains(&row), "row extent was {row}");
        assert!((18..=22).contains(&col), "col extent was {col}");
    }

    #[test]
    fn angle_rotates_the_major_axis() {
        let bmp = synthesize_computed_tip(&geom(Some(40.0), None, Some(90.0), Some(25.0))).unwrap();
        let row = nonzero_in_row(&bmp);
        let col = nonzero_in_col(&bmp);
        assert!(
            col > row,
            "expected major axis vertical: row={row}, col={col}"
        );
    }

    #[test]
    fn angle_direction_matches_photoshop() {
        let bmp = synthesize_computed_tip(&geom(Some(40.0), Some(100.0), Some(45.0), Some(40.0)))
            .unwrap();
        let c = (bmp.width / 2) as i32;
        let probe = |dx: i32, dy: i32| pixel_at(&bmp, (c + dx) as u32, (c + dy) as u32);
        let top_right = probe(8, -8);
        let bottom_right = probe(8, 8);
        assert!(
            top_right > 0,
            "major axis should reach top-right (mirror regression?): {top_right}"
        );
        assert_eq!(
            bottom_right, 0,
            "minor axis must not reach bottom-right: {bottom_right}"
        );
    }

    #[test]
    fn hardness_controls_radial_profile() {
        let soft =
            synthesize_computed_tip(&geom(Some(40.0), Some(0.0), None, Some(100.0))).unwrap();
        let hard =
            synthesize_computed_tip(&geom(Some(40.0), Some(100.0), None, Some(100.0))).unwrap();
        let intermediate =
            |bmp: &GrayscaleBitmap| bmp.data.iter().filter(|&&v| (1..=254).contains(&v)).count();
        assert!(
            intermediate(&soft) > intermediate(&hard),
            "soft should have more intermediate alphas than hard"
        );
    }

    #[test]
    fn diameter_is_capped() {
        let bmp = synthesize_computed_tip(&geom(Some(100000.0), None, None, Some(100.0))).unwrap();
        assert!(bmp.width <= 4098, "width was {}", bmp.width);
    }

    #[test]
    fn unsynthesizable_geometry_is_none() {
        let none = geom(None, Some(100.0), None, None);
        assert!(!can_synthesize(&none));
        assert!(synthesize_computed_tip(&none).is_none());

        let tiny = geom(Some(0.5), None, None, None);
        assert!(!can_synthesize(&tiny));
        assert!(synthesize_computed_tip(&tiny).is_none());
    }

    #[test]
    fn falloff_k_knots_interpolation_and_clamp() {
        let approx = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(approx(falloff_k(0.0), 2.29), "k(0) = {}", falloff_k(0.0));
        assert!(approx(falloff_k(50.0), 1.14), "k(50) = {}", falloff_k(50.0));
        assert!(
            approx(falloff_k(25.0), 1.715),
            "k(25) = {}",
            falloff_k(25.0)
        );
        assert!(
            approx(falloff_k(100.0), 1.14),
            "k(100) = {}",
            falloff_k(100.0)
        );
        assert!(
            approx(falloff_k(-10.0), 2.29),
            "k(-10) = {}",
            falloff_k(-10.0)
        );
    }

    #[test]
    fn gaussian_falloff_value_at_named_pixel() {
        let bmp =
            synthesize_computed_tip(&geom(Some(200.0), Some(0.0), Some(0.0), Some(100.0))).unwrap();
        let v = pixel_at(&bmp, 151, 101);
        assert!((139..=145).contains(&v), "expected 142 ± 3, got {v}");
    }

    #[test]
    fn radial_profile_is_monotone_non_increasing() {
        let bmp =
            synthesize_computed_tip(&geom(Some(200.0), Some(0.0), Some(0.0), Some(100.0))).unwrap();
        let y = bmp.height / 2;
        let cx = bmp.width / 2;
        let mut prev = pixel_at(&bmp, cx, y);
        for x in cx..bmp.width {
            let v = pixel_at(&bmp, x, y);
            assert!(v <= prev, "non-monotone at x={x}: {v} > {prev}");
            prev = v;
        }
    }

    #[test]
    fn no_core_seam_at_boundary() {
        let bmp =
            synthesize_computed_tip(&geom(Some(40.0), Some(50.0), Some(0.0), Some(100.0))).unwrap();
        let y = bmp.height / 2;
        let cx = bmp.width / 2;
        let first_falloff = (cx..bmp.width)
            .map(|x| pixel_at(&bmp, x, y))
            .find(|&v| v < 255)
            .expect("expected a falloff pixel below 255");
        assert!(
            first_falloff >= 250,
            "core seam: first falloff pixel was {first_falloff}"
        );
    }

    #[test]
    fn falloff_uses_capped_h_eff_not_raw_h() {
        let bmp = synthesize_computed_tip(&geom(Some(40.0), Some(100.0), Some(0.0), Some(100.0)))
            .unwrap();
        let ring = bmp.data.iter().filter(|&&v| (1..=254).contains(&v)).count();
        assert!(ring > 0, "expected an antialiased ring, got {ring}");
    }
}
