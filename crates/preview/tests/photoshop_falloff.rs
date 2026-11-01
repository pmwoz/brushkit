//! The synthesized computed tip against Photoshop 27.11's own render of the
//! same geometry. Photoshop drew one dab of the `e25-texbc` preset's tip
//! (`Hrdn` 0, `Rndn` 100) at Size 512, and one dab of a `Hrdn` 100 null at the
//! same size.

use std::path::PathBuf;

use brushkit_abr::ComputedGeometry;
use brushkit_preview::{synthesize_computed_tip, GrayscaleBitmap};

const SITTING: &str = "device-validation/ps-sitting-2026-10-31/905";
const SIZE: f64 = 512.0;
const BIN: f64 = 0.05;
/// Photoshop's soft dab reaches alpha 0 at 1.65 nominal radii.
const TAIL_BINS: usize = 33;
const DISC_BINS: usize = 20;

fn corpus_file(relative: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("BRUSHKIT_CORPUS_DIR")?).join(relative);
    path.is_file().then_some(path)
}

fn photoshop_alpha(name: &str) -> Option<(u32, u32, Vec<f64>)> {
    let image = image::open(corpus_file(&format!("{SITTING}/{name}"))?)
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

fn radial_profile(width: u32, alpha: &[f64], cx: f64, cy: f64) -> Vec<f64> {
    let nominal_radius = SIZE / 2.0;
    let mut sums = vec![(0.0, 0usize); TAIL_BINS];
    for (i, &a) in alpha.iter().enumerate() {
        let x = (i as u32 % width) as f64 + 0.5;
        let y = (i as u32 / width) as f64 + 0.5;
        let bin = ((x - cx).hypot(y - cy) / nominal_radius / BIN) as usize;
        if let Some((sum, n)) = sums.get_mut(bin) {
            *sum += a;
            *n += 1;
        }
    }
    sums.into_iter().map(|(s, n)| s / n as f64).collect()
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

#[test]
fn soft_tip_matches_photoshops_profile_inside_and_past_the_nominal_edge() {
    let (Some(geom), Some((width, _, ps))) =
        (e25_geometry_at_size(), photoshop_alpha("soft-A.png"))
    else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset or the 905 sitting is missing");
        return;
    };
    let (sum, mut cx, mut cy) = (ps.iter().sum::<f64>(), 0.0, 0.0);
    for (i, &a) in ps.iter().enumerate() {
        cx += a * ((i as u32 % width) as f64 + 0.5);
        cy += a * ((i as u32 / width) as f64 + 0.5);
    }
    let photoshop = radial_profile(width, &ps, cx / sum, cy / sum);

    let tip = synthesize_computed_tip(&geom).unwrap();
    let centre = f64::from(tip.width) / 2.0;
    let ours = radial_profile(tip.width, &alpha_of(&tip), centre, centre);

    let gaps: Vec<f64> = ours
        .iter()
        .zip(&photoshop)
        .map(|(o, p)| (o - p).abs())
        .collect();
    let disc_mae = gaps[..DISC_BINS].iter().sum::<f64>() / DISC_BINS as f64;
    println!(
        "MAE inside the nominal radius {disc_mae:.4}, over the tail {:.4}, largest bin gap {:.4}",
        gaps.iter().sum::<f64>() / TAIL_BINS as f64,
        gaps.iter().fold(0.0_f64, |m, &g| m.max(g))
    );
    assert!(
        disc_mae <= 0.02,
        "MAE inside the nominal radius {disc_mae:.4}"
    );
    for (bin, gap) in gaps.iter().enumerate() {
        let r = bin as f64 * BIN;
        assert!(
            *gap <= 0.02,
            "r {r:.2}..{:.2}: ours {:.4}, Photoshop {:.4}",
            r + BIN,
            ours[bin],
            photoshop[bin]
        );
    }
}

#[test]
fn hard_tip_is_as_wide_as_photoshops_hard_null() {
    let (Some(soft), Some((width, height, ps))) =
        (e25_geometry_at_size(), photoshop_alpha("hard-null.png"))
    else {
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
