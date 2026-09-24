mod common;
#[path = "../../../fuzz/fuzz_targets/tip_shapes.rs"]
mod tip_shapes;

use brushkit_preview::procreate::MAX_PLIST_DEPTH;
use brushkit_preview::{
    preview_abr, preview_abr_first_available, preview_brush, preview_brush_first_available,
    preview_brushset, preview_brushset_first_available, PreviewOptions, TipPreview,
    UnavailableReason,
};
use common::{
    brush_archive, brushset_plist, depth_bomb_plist_xml, dimension_bomb_png, gray_png, legacy_abr,
    samp_abr, zip_with, SampTip,
};
use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use tip_shapes::assert_tip_shapes;

const OPTIONS: PreviewOptions = PreviewOptions { max_cell: 8 };
/// The first depth the guard rejects. Deleting one `<array>` takes a mutant
/// below the guard, so the fuzzer explores both sides of the limit.
const SEED_DEPTH: usize = MAX_PLIST_DEPTH + 1;
/// A 64x32 8-bit grayscale PNG written once with Python's zlib at level 9, one
/// IDAT holding one dynamic Huffman block. Row `y` uses filter `y % 5`. The
/// pixels vary, but every 8x8 block averages 200, so its 8x4 preview matches
/// the other valid seeds.
const FILTERED_SHAPE: &[u8] = include_bytes!("fixtures/filtered_shape.png");
const ABR_SEEDS: [&str; 9] = [
    "patt_long_gray",
    "patt_oversized_channel",
    "patt_oversized_name",
    "patt_short_gray",
    "tip_bitmap_dim_overflow",
    "v2_rle_overflow",
    "v2_rle_underflow",
    "wellformed_v6_min",
    "wellformed_v6_patt",
];

type Reader = fn(&[u8]);
const TARGETS: [(&str, Reader); 3] = [
    ("preview_abr", |bytes| {
        assert_tip_shapes(preview_abr(bytes, OPTIONS), OPTIONS);
        assert_tip_shapes(preview_abr_first_available(bytes, OPTIONS, 4), OPTIONS);
    }),
    ("preview_brush", |bytes| {
        assert_tip_shapes(preview_brush(bytes, OPTIONS), OPTIONS);
        assert_tip_shapes(preview_brush_first_available(bytes, OPTIONS, 4), OPTIONS);
    }),
    ("preview_brushset", |bytes| {
        assert_tip_shapes(preview_brushset(bytes, OPTIONS), OPTIONS);
        assert_tip_shapes(preview_brushset_first_available(bytes, OPTIONS, 4), OPTIONS);
    }),
];

fn corpus(target: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/corpus")
        .join(target)
}

fn generated_seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let archive_a = brush_archive("A");
    let archive_b = brush_archive("B");
    let shape = gray_png(16, 8, 200);
    let tip = |width, height| SampTip {
        width,
        height,
        fill: 200,
        corrupt: false,
    };
    // The v6 parser rejects a zero-area tip, so that seed is legacy. Its other
    // side fits max_cell: past that, downsample would widen the zero side to 1.
    let mut seeds = vec![
        ("preview_abr", "sampled_tip", samp_abr(&[tip(16, 8)])),
        ("preview_abr", "small_tip", samp_abr(&[tip(4, 4)])),
        (
            "preview_abr",
            "zero_area_tip",
            legacy_abr(&[tip(1, 1), tip(0, 4)]),
        ),
    ];
    for name in ABR_SEEDS {
        seeds.push((
            "preview_abr",
            name,
            std::fs::read(corpus("abr_parse").join(name)).expect("original ABR seed exists"),
        ));
    }
    for (name, entries) in [
        (
            "root_brush",
            vec![
                ("Brush.archive", archive_a.clone()),
                ("Shape.png", shape.clone()),
            ],
        ),
        ("missing_shape", vec![("Brush.archive", archive_a.clone())]),
        (
            "corrupt_shape",
            vec![
                ("Brush.archive", archive_a.clone()),
                ("Shape.png", b"not a PNG".to_vec()),
            ],
        ),
        (
            "oversized_png",
            vec![
                ("Brush.archive", archive_a.clone()),
                ("Shape.png", dimension_bomb_png(60000, 60000)),
            ],
        ),
        (
            "deep_archive",
            vec![
                ("Brush.archive", depth_bomb_plist_xml(SEED_DEPTH)),
                ("Shape.png", shape.clone()),
            ],
        ),
        (
            "filtered_shape",
            vec![
                ("Brush.archive", archive_a.clone()),
                ("Shape.png", FILTERED_SHAPE.to_vec()),
            ],
        ),
    ] {
        let entries: Vec<_> = entries
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        seeds.push(("preview_brush", name, zip_with(&entries)));
    }
    let members = [
        ("a/Brush.archive", archive_a.as_slice()),
        ("a/Shape.png", shape.as_slice()),
        ("b/Brush.archive", archive_b.as_slice()),
        ("b/Shape.png", shape.as_slice()),
    ];
    let plist = brushset_plist("Ordered", &["b", "a"]);
    let mut ordered = vec![("brushset.plist", plist.as_slice())];
    ordered.extend_from_slice(&members);
    seeds.extend([
        ("preview_brushset", "ordered_set", zip_with(&ordered)),
        ("preview_brushset", "no_metadata", zip_with(&members)),
        (
            "preview_brushset",
            "bad_shapes",
            zip_with(&[
                ("a/Brush.archive", &archive_a),
                ("b/Brush.archive", &archive_b),
                ("b/Shape.png", b"not a PNG"),
            ]),
        ),
        (
            "preview_brushset",
            "deep_metadata",
            zip_with(&[("brushset.plist", &depth_bomb_plist_xml(SEED_DEPTH))]),
        ),
    ]);
    seeds
}

#[test]
fn every_corpus_file_replays_with_valid_tip_shapes() {
    for (target, read) in TARGETS {
        let mut count = 0;
        for entry in std::fs::read_dir(corpus(target)).expect("corpus directory must exist") {
            let path = entry.unwrap().path();
            if path.is_file() {
                println!("replaying {}", path.display());
                read(&std::fs::read(path).expect("corpus file is readable"));
                count += 1;
            }
        }
        assert!(count > 0, "{target} corpus must not be empty");
    }
}

#[test]
fn committed_seeds_match_generated_bytes() {
    for (target, name, expected) in generated_seeds() {
        let path = corpus(target).join(name);
        let actual = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(actual, expected, "{target}/{name}: regenerate the seeds");
    }
}

#[test]
fn valid_seeds_decode_names_order_and_downsampled_tips() {
    for (target, name, expected_set_name, expected_names) in [
        ("preview_brush", "root_brush", None, vec!["A"]),
        ("preview_brush", "root_brush_deflated", None, vec!["A"]),
        ("preview_brush", "filtered_shape", None, vec!["A"]),
        (
            "preview_brushset",
            "ordered_set",
            Some("Ordered"),
            vec!["B", "A"],
        ),
        ("preview_brushset", "no_metadata", None, vec!["A", "B"]),
    ] {
        let bytes = std::fs::read(corpus(target).join(name)).unwrap();
        let read = if target == "preview_brush" {
            preview_brush
        } else {
            preview_brushset
        };
        let set = read(&bytes, OPTIONS).expect("valid seed decodes");
        assert_eq!(set.set_name.as_deref(), expected_set_name);
        assert_eq!(
            set.entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            expected_names
        );
        for (index, entry) in set.entries.iter().enumerate() {
            assert_eq!(entry.index, index);
            let TipPreview::Available(tip) = &entry.tip else {
                panic!(
                    "{target}/{name}: expected a decoded tip, got {:?}",
                    entry.tip
                );
            };
            assert_eq!((tip.width, tip.height), (8, 4));
            assert_eq!(tip.data, vec![200; 32]);
        }
    }
    for (name, (width, height)) in [("sampled_tip", (8, 4)), ("small_tip", (4, 4))] {
        let bytes = std::fs::read(corpus("preview_abr").join(name)).unwrap();
        let set = preview_abr(&bytes, OPTIONS).expect("valid ABR seed decodes");
        assert_eq!(set.entries.len(), 1);
        let TipPreview::Available(tip) = &set.entries[0].tip else {
            panic!(
                "{name}: expected a decoded tip, got {:?}",
                set.entries[0].tip
            );
        };
        assert_eq!((tip.width, tip.height), (width, height));
        assert_eq!(tip.data, vec![200; (width * height) as usize]);
    }
    let bytes = std::fs::read(corpus("preview_abr").join("zero_area_tip")).unwrap();
    let set = preview_abr(&bytes, OPTIONS).expect("zero_area_tip decodes");
    assert!(
        matches!(
            set.entries.as_slice(),
            [zero, drawable]
                if matches!(&zero.tip, TipPreview::Unavailable(UnavailableReason::Corrupt(msg)) if msg == "tip has zero area")
                    && matches!(&drawable.tip, TipPreview::Available(tip) if (tip.width, tip.height) == (1, 1))
        ),
        "zero_area_tip: {:?}",
        set.entries
    );
}

#[test]
fn depth_seeds_reach_the_depth_guard() {
    let bytes = std::fs::read(corpus("preview_brush").join("deep_archive")).unwrap();
    let set = preview_brush(&bytes, OPTIONS).expect("brush reads");
    let [entry] = set.entries.as_slice() else {
        panic!("expected one entry, got {}", set.entries.len());
    };
    let TipPreview::Unavailable(UnavailableReason::Corrupt(msg)) = &entry.tip else {
        panic!("expected Corrupt, got {:?}", entry.tip);
    };
    assert!(msg.contains("depth"), "deep_archive: {msg}");

    let bytes = std::fs::read(corpus("preview_brushset").join("deep_metadata")).unwrap();
    let err = preview_brushset(&bytes, OPTIONS).expect_err("depth bomb must be rejected");
    assert!(err.0.contains("depth"), "deep_metadata: {err}");
}

/// `root_brush_deflated` is a committed file, not generated, so a deflate
/// backend change does not move it. Every generated seed is stored, so this is
/// the one seed that takes the zip reader through inflate.
#[test]
fn deflated_seed_stays_deflated() {
    let bytes = std::fs::read(corpus("preview_brush").join("root_brush_deflated")).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("seed is a zip");
    for index in 0..zip.len() {
        let entry = zip.by_index(index).unwrap();
        assert_eq!(
            entry.compression(),
            zip::CompressionMethod::Deflated,
            "{}",
            entry.name()
        );
    }
}

/// `filtered_shape.png` is a committed file, not encoded at test time, so a
/// compressor change does not move it. It is the one seed whose Shape.png takes
/// the decoder through a Huffman-coded IDAT and every row filter.
#[test]
fn filtered_shape_stays_compressed_and_filtered() {
    let at = FILTERED_SHAPE
        .windows(4)
        .position(|b| b == b"IDAT")
        .unwrap();
    let len = u32::from_be_bytes(FILTERED_SHAPE[at - 4..at].try_into().unwrap()) as usize;
    let idat = &FILTERED_SHAPE[at + 4..at + 4 + len];
    assert_eq!(
        (idat[2] >> 1) & 3,
        2,
        "first DEFLATE block is dynamic Huffman"
    );
    let mut rows = Vec::new();
    flate2::read::ZlibDecoder::new(idat)
        .read_to_end(&mut rows)
        .unwrap();
    assert_eq!(
        rows.len(),
        32 * 65,
        "32 rows of a filter byte and 64 pixels"
    );
    let filters: BTreeSet<u8> = rows.iter().step_by(65).copied().collect();
    assert_eq!(filters, BTreeSet::from([0, 1, 2, 3, 4]));
}

#[test]
#[ignore = "writes known synthetic seeds to fuzz/corpus"]
fn regenerate_fuzz_seeds() {
    for (target, name, bytes) in generated_seeds() {
        let dir = corpus(target);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}
