use std::path::PathBuf;

use brushkit_abr::ComputedGeometry;
use brushkit_preview::{synthesize_computed_tip, GrayscaleBitmap};

const SITTING_905: &str = "device-validation/ps-sitting-2026-10-31/905";
const SITTING_264: &str = "device-validation/ps-sitting-2026-11-01/264";
const SITTING_269: &str = "device-validation/ps-sitting-2026-11-01/269";
const SITTING_271: &str = "device-validation/ps-sitting-2026-11-01/271";
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

fn e25_geometry(diameter: f64) -> Option<ComputedGeometry> {
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
        diameter_px: Some(diameter),
        ..geom
    })
}

fn radial_profile(
    width: u32,
    alpha: &[f64],
    (cx, cy): (f64, f64),
    diameter: f64,
    angle_deg: f64,
    roundness_pct: f64,
) -> Vec<Option<f64>> {
    let major = diameter / 2.0;
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
        .map(|(s, n)| (n > 0).then(|| s / n as f64))
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
    diameter: f64,
    hardness_pct: f64,
    angle_deg: f64,
    roundness_pct: f64,
) {
    let (Some(geom), Some((width, _, ps))) = (e25_geometry(diameter), photoshop_alpha(render))
    else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset or {render} is missing");
        return;
    };
    let (sum, mut cx, mut cy) = (ps.iter().sum::<f64>(), 0.0, 0.0);
    for (i, &a) in ps.iter().enumerate() {
        cx += a * ((i as u32 % width) as f64 + 0.5);
        cy += a * ((i as u32 / width) as f64 + 0.5);
    }
    let photoshop = radial_profile(
        width,
        &ps,
        (cx / sum, cy / sum),
        diameter,
        angle_deg,
        roundness_pct,
    );

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
        diameter,
        angle_deg,
        roundness_pct,
    );

    let gaps: Vec<(usize, f64, f64, f64)> = ours
        .iter()
        .zip(&photoshop)
        .enumerate()
        .filter_map(|(bin, (o, p))| Some((bin, o.as_ref()?, p.as_ref()?)))
        .map(|(bin, &o, &p)| (bin, o, p, (o - p).abs()))
        .collect();
    let mean = |gaps: Vec<f64>| gaps.iter().sum::<f64>() / gaps.len() as f64;
    let disc_mae = mean(
        gaps.iter()
            .filter(|&&(bin, ..)| bin < DISC_BINS)
            .map(|&(.., gap)| gap)
            .collect(),
    );
    let &(widest_bin, .., widest) = gaps.iter().max_by(|a, b| a.3.total_cmp(&b.3)).unwrap();
    let label =
        format!("Size {diameter}, Hrdn {hardness_pct}, Angl {angle_deg}, Rndn {roundness_pct}");
    println!(
        "{label}: MAE inside the nominal radius {disc_mae:.4}, over the tail {:.4}, \
         largest bin gap {widest:.4} at r {:.2}",
        mean(gaps.iter().map(|&(.., gap)| gap).collect()),
        widest_bin as f64 * BIN
    );
    assert!(
        disc_mae <= 0.02,
        "{label}: MAE inside the nominal radius {disc_mae:.4}"
    );
    for (bin, ours, photoshop, gap) in gaps {
        let r = bin as f64 * BIN;
        assert!(
            gap <= 0.02,
            "{label}, r {r:.2}..{:.2}: ours {ours:.4}, Photoshop {photoshop:.4}",
            r + BIN,
        );
    }
}

#[test]
fn soft_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(&format!("{SITTING_905}/soft-A.png"), SIZE, 0.0, 0.0, 100.0);
}

#[test]
fn hardness_10_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_269}/hrdn10-A.png"),
        SIZE,
        10.0,
        0.0,
        100.0,
    );
}

#[test]
fn hardness_20_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_264}/hrdn20-A.png"),
        SIZE,
        20.0,
        0.0,
        100.0,
    );
}

#[test]
fn hardness_35_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_269}/hrdn35-A.png"),
        SIZE,
        35.0,
        0.0,
        100.0,
    );
}

#[test]
fn hardness_50_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_264}/hrdn50-A.png"),
        SIZE,
        50.0,
        0.0,
        100.0,
    );
}

#[test]
fn hardness_65_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_269}/hrdn65-A.png"),
        SIZE,
        65.0,
        0.0,
        100.0,
    );
}

#[test]
fn hardness_80_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_264}/hrdn80-A.png"),
        SIZE,
        80.0,
        0.0,
        100.0,
    );
}

#[test]
fn hardness_90_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_269}/hrdn90-A.png"),
        SIZE,
        90.0,
        0.0,
        100.0,
    );
}

#[test]
fn rotated_elliptical_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_269}/rndn50-ang30-hrdn50-A.png"),
        SIZE,
        50.0,
        30.0,
        50.0,
    );
}

#[test]
fn size_8_hardness_50_tip_matches_photoshops_profile() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_271}/size8-hrdn50-A.png"),
        8.0,
        50.0,
        0.0,
        100.0,
    );
}

#[test]
fn size_16_hardness_50_tip_matches_photoshops_profile() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_271}/size16-hrdn50-A.png"),
        16.0,
        50.0,
        0.0,
        100.0,
    );
}

#[test]
fn size_32_hardness_50_tip_matches_photoshops_profile() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_271}/size32-hrdn50-A.png"),
        32.0,
        50.0,
        0.0,
        100.0,
    );
}

#[test]
fn size_64_hardness_50_tip_matches_photoshops_profile() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_271}/size64-hrdn50-A.png"),
        64.0,
        50.0,
        0.0,
        100.0,
    );
}

#[test]
fn size_128_hardness_50_tip_matches_photoshops_profile() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_271}/size128-hrdn50-A.png"),
        128.0,
        50.0,
        0.0,
        100.0,
    );
}

#[test]
fn size_256_hardness_50_tip_matches_photoshops_profile() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_271}/size256-hrdn50-A.png"),
        256.0,
        50.0,
        0.0,
        100.0,
    );
}

#[test]
fn size_16_hardness_20_tip_matches_photoshops_profile() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_271}/size16-hrdn20-A.png"),
        16.0,
        20.0,
        0.0,
        100.0,
    );
}

#[test]
fn size_128_hardness_20_tip_matches_photoshops_profile() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_271}/size128-hrdn20-A.png"),
        128.0,
        20.0,
        0.0,
        100.0,
    );
}

#[test]
fn size_8_hardness_100_tip_matches_photoshops_profile() {
    assert_profile_matches_photoshop(
        &format!("{SITTING_271}/size8-hard-null.png"),
        8.0,
        100.0,
        0.0,
        100.0,
    );
}

#[test]
fn hard_tip_is_as_wide_as_photoshops_hard_null() {
    let (Some(soft), Some((width, height, ps))) = (
        e25_geometry(SIZE),
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
