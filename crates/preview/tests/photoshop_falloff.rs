//! The synthesized computed tip against Photoshop 27.11's own render of the
//! same geometry at Size 512.

use std::path::PathBuf;

use brushkit_abr::ComputedGeometry;
use brushkit_preview::{synthesize_computed_tip, GrayscaleBitmap};

const SITTING_905: &str = "device-validation/ps-sitting-2026-10-31/905";
const SITTING_264: &str = "device-validation/ps-sitting-2026-11-01/264";
const SITTING_269: &str = "device-validation/ps-sitting-2026-11-01/269";
const SIZE: f64 = 512.0;
const BIN: f64 = 0.05;
/// Photoshop's `Hrdn` 0 dab reaches alpha 0 at 1.65 nominal radii, farther
/// out than any harder dab.
const TAIL_BINS: usize = 33;
const DISC_BINS: usize = 20;

fn corpus_file(relative: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("BRUSHKIT_CORPUS_DIR")?).join(relative);
    path.is_file().then_some(path)
}

fn photoshop_alpha(render: &str) -> Option<(u32, u32, Vec<f64>)> {
    let image = image::open(corpus_file(render)?)
        .expect("Photoshop render decodes")
        .into_rgba8();
    let alpha = image.pixels().map(|p| f64::from(p[3]) / 255.0).collect();
    Some((image.width(), image.height(), alpha))
}

fn e25_geometry_at_size() -> Option<ComputedGeometry> {
    let bytes = std::fs::read(corpus_file("ps-27.8.0/e25-texbc.abr")?).unwrap();
    let pack = brushkit_abr::parse_abr(&bytes).unwrap();
    let geom = pack.computed_presets[0]
        .descriptor
        .computed
        .clone()
        .unwrap();
    assert_eq!(geom.hardness_pct, Some(0.0));
    assert_eq!(geom.roundness_pct, Some(100.0));
    Some(ComputedGeometry {
        diameter_px: Some(SIZE),
        ..geom
    })
}

fn radial_profile(
    width: u32,
    alpha: &[f64],
    (cx, cy): (f64, f64),
    angle_deg: f64,
    roundness_pct: f64,
) -> Vec<f64> {
    let major = SIZE / 2.0;
    let minor = major * roundness_pct / 100.0;
    let (sin, cos) = (-angle_deg).to_radians().sin_cos();
    let mut sums = vec![(0.0, 0usize); TAIL_BINS];
    for (i, &a) in alpha.iter().enumerate() {
        let dx = (i as u32 % width) as f64 + 0.5 - cx;
        let dy = (i as u32 / width) as f64 + 0.5 - cy;
        let u = dx * cos + dy * sin;
        let v = -dx * sin + dy * cos;
        let bin = ((u / major).hypot(v / minor) / BIN) as usize;
        if let Some((sum, n)) = sums.get_mut(bin) {
            *sum += a;
            *n += 1;
        }
    }
    sums.into_iter()
        .map(|(s, n)| if n == 0 { 0.0 } else { s / n as f64 })
        .collect()
}

fn alpha_of(bitmap: &GrayscaleBitmap) -> Vec<f64> {
    bitmap.data.iter().map(|&v| f64::from(v) / 255.0).collect()
}

fn inked_columns(width: u32, alpha: &[f64], row: u32) -> usize {
    let start = (row * width) as usize;
    alpha[start..start + width as usize]
        .iter()
        .filter(|&&a| a > 0.0)
        .count()
}

fn assert_profile_matches_photoshop(
    render: &str,
    hardness_pct: f64,
    angle_deg: f64,
    roundness_pct: f64,
) {
    let (Some(geom), Some((width, _, ps))) = (e25_geometry_at_size(), photoshop_alpha(render))
    else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset or {render} is missing");
        return;
    };
    let (sum, mut cx, mut cy) = (ps.iter().sum::<f64>(), 0.0, 0.0);
    for (i, &a) in ps.iter().enumerate() {
        cx += a * ((i as u32 % width) as f64 + 0.5);
        cy += a * ((i as u32 / width) as f64 + 0.5);
    }
    let photoshop = radial_profile(width, &ps, (cx / sum, cy / sum), angle_deg, roundness_pct);

    let tip = synthesize_computed_tip(&ComputedGeometry {
        hardness_pct: Some(hardness_pct),
        angle_deg: Some(angle_deg),
        roundness_pct: Some(roundness_pct),
        ..geom
    })
    .unwrap();
    let centre = f64::from(tip.width) / 2.0;
    let ours = radial_profile(
        tip.width,
        &alpha_of(&tip),
        (centre, centre),
        angle_deg,
        roundness_pct,
    );

    let gaps: Vec<f64> = ours
        .iter()
        .zip(&photoshop)
        .map(|(o, p)| (o - p).abs())
        .collect();
    let disc_mae = gaps[..DISC_BINS].iter().sum::<f64>() / DISC_BINS as f64;
    let (widest_bin, widest) = gaps.iter().copied().enumerate().fold(
        (0, 0.0_f64),
        |m, (bin, g)| if g > m.1 { (bin, g) } else { m },
    );
    let label = format!("Hrdn {hardness_pct}, Angl {angle_deg}, Rndn {roundness_pct}");
    println!(
        "{label}: MAE inside the nominal radius {disc_mae:.4}, over the tail {:.4}, \
         largest bin gap {widest:.4} at r {:.2}",
        gaps.iter().sum::<f64>() / TAIL_BINS as f64,
        widest_bin as f64 * BIN
    );
    assert!(
        disc_mae <= 0.02,
        "{label}: MAE inside the nominal radius {disc_mae:.4}"
    );
    for (bin, gap) in gaps.iter().enumerate() {
        let r = bin as f64 * BIN;
        assert!(
            *gap <= 0.02,
            "{label}, r {r:.2}..{:.2}: ours {:.4}, Photoshop {:.4}",
            r + BIN,
            ours[bin],
            photoshop[bin]
        );
    }
}

#[test]
fn soft_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(&format!("{SITTING_905}/soft-A.png"), 0.0, 0.0, 100.0);
}

#[test]
fn hardness_10_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(&format!("{SITTING_269}/hrdn10-A.png"), 10.0, 0.0, 100.0);
}

#[test]
fn hardness_20_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(&format!("{SITTING_264}/hrdn20-A.png"), 20.0, 0.0, 100.0);
}

#[test]
fn hardness_35_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(&format!("{SITTING_269}/hrdn35-A.png"), 35.0, 0.0, 100.0);
}

#[test]
fn hardness_50_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(&format!("{SITTING_264}/hrdn50-A.png"), 50.0, 0.0, 100.0);
}

#[test]
fn hardness_65_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(&format!("{SITTING_269}/hrdn65-A.png"), 65.0, 0.0, 100.0);
}

#[test]
fn hardness_80_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(&format!("{SITTING_264}/hrdn80-A.png"), 80.0, 0.0, 100.0);
}

#[test]
fn hardness_90_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(&format!("{SITTING_269}/hrdn90-A.png"), 90.0, 0.0, 100.0);
}

#[test]
fn rotated_elliptical_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_269}/rndn50-ang30-hrdn50-A.png"),
        50.0,
        30.0,
        50.0,
    );
}

#[test]
fn hard_tip_is_as_wide_as_photoshops_hard_null() {
    let (Some(soft), Some((width, height, ps))) = (
        e25_geometry_at_size(),
        photoshop_alpha(&format!("{SITTING_905}/hard-null.png")),
    ) else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset or the 905 sitting is missing");
        return;
    };
    let ps_widest = (0..height)
        .map(|row| inked_columns(width, &ps, row))
        .max()
        .unwrap();
    assert_eq!(ps_widest, SIZE as usize);

    let tip = synthesize_computed_tip(&ComputedGeometry {
        hardness_pct: Some(100.0),
        ..soft
    })
    .unwrap();
    let ours = alpha_of(&tip);
    let ours_widest = (0..tip.height)
        .map(|row| inked_columns(tip.width, &ours, row))
        .max()
        .unwrap();
    assert_eq!(ours_widest, ps_widest);
}
