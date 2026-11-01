use crate::GrayscaleBitmap;

const MAX_TIP_PX: f64 = 4096.0;

pub fn can_synthesize(geom: &brushkit_abr::ComputedGeometry) -> bool {
    geom.diameter_px.is_some_and(|d| d >= 1.0)
}

/// The width and height of the square tip `synthesize_computed_tip` returns,
/// without rasterizing it. `None` iff `can_synthesize` is false.
pub fn synthesized_tip_side(geom: &brushkit_abr::ComputedGeometry) -> Option<u32> {
    can_synthesize(geom).then(|| Profile::new(geom).side())
}

const HALF_LEVEL: f64 = 0.5 / 255.0;
const ANTIALIAS_PX: f64 = 1.0;

#[derive(Clone, Copy)]
struct Knot {
    hardness_pct: f64,
    opaque_until: f64,
    width: f64,
}

/// Fitted to Photoshop 27.11 renders of a computed round tip at Size 512 (#264,
/// #269).
const KNOTS: [Knot; 9] = [
    Knot {
        hardness_pct: 0.0,
        opaque_until: 0.0,
        width: 0.659,
    },
    Knot {
        hardness_pct: 10.0,
        opaque_until: 0.160,
        width: 0.567,
    },
    Knot {
        hardness_pct: 20.0,
        opaque_until: 0.303,
        width: 0.484,
    },
    Knot {
        hardness_pct: 35.0,
        opaque_until: 0.496,
        width: 0.370,
    },
    Knot {
        hardness_pct: 50.0,
        opaque_until: 0.659,
        width: 0.266,
    },
    Knot {
        hardness_pct: 65.0,
        opaque_until: 0.792,
        width: 0.174,
    },
    Knot {
        hardness_pct: 80.0,
        opaque_until: 0.897,
        width: 0.093,
    },
    Knot {
        hardness_pct: 90.0,
        opaque_until: 0.950,
        width: 0.047,
    },
    Knot {
        hardness_pct: 100.0,
        opaque_until: 1.0,
        width: 0.0,
    },
];

fn falloff_at(hardness_pct: f64) -> Knot {
    let h = hardness_pct.clamp(0.0, 100.0);
    Knot {
        hardness_pct: h,
        opaque_until: monotone_cubic(h, |k| k.opaque_until),
        width: monotone_cubic(h, |k| k.width),
    }
}

fn monotone_cubic(h: f64, y: fn(&Knot) -> f64) -> f64 {
    let last = KNOTS.len() - 1;
    let x = |k: usize| KNOTS[k].hardness_pct;
    let step = |k: usize| x(k + 1) - x(k);
    let secant = |k: usize| (y(&KNOTS[k + 1]) - y(&KNOTS[k])) / step(k);
    let slope = |k: usize| {
        if k == 0 {
            return secant(0);
        }
        if k == last {
            return secant(last - 1);
        }
        let (d0, d1) = (secant(k - 1), secant(k));
        if d0 * d1 <= 0.0 {
            return 0.0;
        }
        let w1 = 2.0 * step(k) + step(k - 1);
        let w2 = step(k) + 2.0 * step(k - 1);
        (w1 + w2) / (w1 / d0 + w2 / d1)
    };
    let i = (0..last).find(|&i| h <= x(i + 1)).unwrap_or(last - 1);
    let t = (h - x(i)) / step(i);
    let (t2, t3) = (t * t, t * t * t);
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    h00 * y(&KNOTS[i])
        + h10 * step(i) * slope(i)
        + h01 * y(&KNOTS[i + 1])
        + h11 * step(i) * slope(i + 1)
}

fn antialias_steepness(hardness_pct: f64) -> f64 {
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

struct Profile {
    diameter: f64,
    opaque_until: f64,
    ramp: f64,
    steepness: f64,
    clear_from: f64,
}

impl Profile {
    fn new(geom: &brushkit_abr::ComputedGeometry) -> Self {
        let diameter = geom.diameter_px.unwrap().clamp(1.0, MAX_TIP_PX);
        let hardness = geom
            .hardness_pct
            .filter(|h| !h.is_nan())
            .unwrap_or(100.0)
            .clamp(0.0, 100.0);
        let radius = diameter / 2.0;
        let antialias_only = (1.0 - hardness / 100.0) * radius <= ANTIALIAS_PX;
        // Photoshop's soft falloff runs past the nominal edge until alpha
        // rounds to 0. Its hard tip stops at the edge.
        let (opaque_until, ramp, steepness, clear_from) = if antialias_only {
            let opaque_until = (1.0 - ANTIALIAS_PX / radius).max(0.0);
            let steepness = antialias_steepness(hardness);
            (opaque_until, 1.0 - opaque_until, steepness, 1.0)
        } else {
            let knot = falloff_at(hardness);
            let clear_from = knot.opaque_until + knot.width * (-HALF_LEVEL.ln()).sqrt();
            (knot.opaque_until, knot.width, 1.0, clear_from)
        };
        Profile {
            diameter,
            opaque_until,
            ramp,
            steepness,
            clear_from,
        }
    }

    fn side(&self) -> u32 {
        ((self.diameter * self.clear_from).ceil() as u32) + 2
    }

    fn alpha(&self, r: f64) -> f64 {
        if r <= self.opaque_until {
            1.0
        } else if r >= self.clear_from {
            0.0
        } else {
            let t = (r - self.opaque_until) / self.ramp;
            (-self.steepness * t * t).exp()
        }
    }
}

/// Rasterize a monochrome elliptical tip from computed-brush geometry.
///
/// `None` iff `can_synthesize` is false. Output is single-channel grayscale,
/// white=stamp (`255`) / black=transparent (`0`).
///
/// The tip's nominal diameter is `diameter_px`, centred in the bitmap. Its
/// alpha follows Photoshop 27.11's computed tip: an opaque core, then a Gaussian
/// falloff that runs past the nominal edge until alpha rounds to 0. So the
/// bitmap is wider than the diameter: about 1.65 times at 0% hardness and 1.13
/// times at 80%. A tip whose ramp, `100 - hardness_pct` percent of the radius,
/// is one pixel or less only antialiases its edge and ends at the nominal edge.
pub fn synthesize_computed_tip(geom: &brushkit_abr::ComputedGeometry) -> Option<GrayscaleBitmap> {
    if !can_synthesize(geom) {
        return None;
    }

    let profile = Profile::new(geom);
    let angle = geom.angle_deg.unwrap_or(0.0);
    let roundness = geom.roundness_pct.unwrap_or(100.0).clamp(0.0, 100.0);

    let side = profile.side();
    let center = side as f64 / 2.0;

    let a = profile.diameter / 2.0;
    let b = (a * roundness / 100.0).max(0.5);

    // Photoshop's `Angl` is CCW-positive in canvas (y-up) orientation; the
    // bitmap's y axis points down, so the angle is negated to rotate the tip
    // the way Photoshop does.
    let (sin, cos) = (-angle).to_radians().sin_cos();

    let mut data = vec![0u8; (side * side) as usize];
    for y in 0..side {
        for x in 0..side {
            let dx = (x as f64 + 0.5) - center;
            let dy = (y as f64 + 0.5) - center;
            let u = dx * cos + dy * sin;
            let v = -dx * sin + dy * cos;
            let r = ((u / a).powi(2) + (v / b).powi(2)).sqrt();
            data[(y * side + x) as usize] = (profile.alpha(r) * 255.0).round() as u8;
        }
    }

    Some(GrayscaleBitmap {
        width: side,
        height: side,
        data,
    })
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
        let largest = (0..=10_000)
            .map(|h| {
                synthesized_tip_side(&geom(Some(100000.0), Some(h as f64 / 100.0), None, None))
            })
            .max()
            .flatten()
            .unwrap();
        assert!(largest <= 6742, "largest side was {largest}");
    }

    #[test]
    fn unsynthesizable_geometry_is_none() {
        let none = geom(None, Some(100.0), None, None);
        assert!(!can_synthesize(&none));
        assert!(synthesize_computed_tip(&none).is_none());

        let tiny = geom(Some(0.5), None, None, None);
        assert!(!can_synthesize(&tiny));
        assert!(synthesize_computed_tip(&tiny).is_none());
        assert_eq!(synthesized_tip_side(&none), None);
        assert_eq!(synthesized_tip_side(&tiny), None);
    }

    #[test]
    fn synthesized_tip_side_is_the_side_synthesis_allocates() {
        for diameter in [1.0, 1.5, 30.0, 39.2, 4096.0, 100000.0] {
            let g = geom(Some(diameter), Some(0.0), Some(30.0), Some(40.0));
            let bmp = synthesize_computed_tip(&g).unwrap();
            assert_eq!(bmp.width, bmp.height, "diameter {diameter}");
            assert_eq!(
                synthesized_tip_side(&g),
                Some(bmp.width),
                "diameter {diameter}"
            );
            assert_eq!(bmp.data.len(), (bmp.width * bmp.height) as usize);
        }
    }

    #[test]
    fn gaussian_falloff_value_at_named_pixel() {
        let bmp =
            synthesize_computed_tip(&geom(Some(200.0), Some(0.0), Some(0.0), Some(100.0))).unwrap();
        let v = pixel_at(&bmp, bmp.width / 2 + 50, bmp.height / 2);
        assert!((139..=145).contains(&v), "expected 142 ± 3, got {v}");
    }

    /// Photoshop 27.11 draws a `Hrdn` 0 dab as `exp(-2.30 r²)` of the nominal
    /// radius with no cut: alpha 0.10 at the nominal edge, 0.036 at 1.2 radii,
    /// and 0 once it rounds below half a level, at about 1.65 radii.
    #[test]
    fn soft_tip_carries_the_tail_past_the_nominal_edge() {
        let bmp =
            synthesize_computed_tip(&geom(Some(200.0), Some(0.0), Some(0.0), Some(100.0))).unwrap();
        let (c, y) = (bmp.width / 2, bmp.height / 2);
        let at = |r: f64| pixel_at(&bmp, c + (r * 100.0) as u32, y);
        assert!((24..=28).contains(&at(1.0)), "r 1.0: {}", at(1.0));
        assert!((7..=11).contains(&at(1.2)), "r 1.2: {}", at(1.2));
        assert!(at(1.6) > 0, "r 1.6 should still hold ink");
        let last_inked = (c..bmp.width).rev().find(|&x| pixel_at(&bmp, x, y) > 0);
        assert!(
            last_inked.is_some_and(|x| x < bmp.width - 1),
            "the tail must fit inside the bitmap: last inked {last_inked:?} of {}",
            bmp.width
        );
    }

    #[test]
    fn intermediate_hardness_follows_photoshops_core_and_falloff() {
        let bmp = synthesize_computed_tip(&geom(Some(200.0), Some(50.0), Some(0.0), Some(100.0)))
            .unwrap();
        let (c, y) = (bmp.width / 2, bmp.height / 2);
        let at = |r: f64| pixel_at(&bmp, c + (r * 100.0) as u32, y);
        assert_eq!(at(0.6), 255, "r 0.6 is inside the opaque core");
        assert!((47..=51).contains(&at(1.0)), "r 1.0: {}", at(1.0));
        assert!(at(1.3) > 0, "r 1.3 should still hold ink");
        assert!(
            (c + 133..bmp.width).all(|x| pixel_at(&bmp, x, y) == 0),
            "ink from r 1.33 on"
        );
    }

    #[test]
    fn a_ramp_of_one_pixel_or_less_gets_no_tail() {
        let one_px = synthesize_computed_tip(&geom(Some(32.0), Some(93.75), None, None)).unwrap();
        assert_eq!(one_px.width, 34);
        let tiny = synthesize_computed_tip(&geom(Some(1.0), Some(0.0), None, None)).unwrap();
        assert_eq!(tiny.width, 3);
        assert_eq!(pixel_at(&tiny, 1, 1), 255);
    }

    #[test]
    fn a_ramp_of_one_pixel_or_less_keeps_its_antialiased_ring() {
        let row = |d: f64, h: f64, y: u32| {
            let bmp =
                synthesize_computed_tip(&geom(Some(d), Some(h), Some(0.0), Some(100.0))).unwrap();
            bmp.data[(y * bmp.width) as usize..((y + 1) * bmp.width) as usize].to_vec()
        };
        assert_eq!(row(2.0, 0.0, 1), [0, 81, 81, 0]);
        assert_eq!(row(1.4678140119077667, 100.0, 1), [0, 88, 88, 0]);
        assert_eq!(
            row(12.175668985559687, 100.0, 1),
            [0, 0, 0, 0, 0, 0, 83, 99, 83, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn nan_hardness_draws_like_missing_hardness() {
        let nan = synthesize_computed_tip(&geom(Some(40.0), Some(f64::NAN), None, None)).unwrap();
        let missing = synthesize_computed_tip(&geom(Some(40.0), None, None, None)).unwrap();
        assert_eq!(
            (nan.width, nan.height, nan.data),
            (missing.width, missing.height, missing.data)
        );
    }

    #[test]
    fn hard_tip_ends_at_the_nominal_edge() {
        let bmp = synthesize_computed_tip(&geom(Some(40.0), Some(100.0), Some(0.0), Some(100.0)))
            .unwrap();
        assert_eq!(bmp.width, 42);
        let c = f64::from(bmp.width) / 2.0;
        for y in 0..bmp.height {
            for x in 0..bmp.width {
                let r = (f64::from(x) + 0.5 - c).hypot(f64::from(y) + 0.5 - c) / 20.0;
                if r >= 1.0 {
                    assert_eq!(pixel_at(&bmp, x, y), 0, "ink at ({x}, {y}), r {r:.3}");
                }
            }
        }
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
            first_falloff >= 246,
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
