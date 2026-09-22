mod common;

use brushkit_preview::{
    preview_abr, preview_abr_first_available, preview_brush, preview_brush_first_available,
    preview_brushset, preview_brushset_first_available, PreviewOptions, TipPreview,
};
use common::{
    brush_archive, brushset_plist, depth_bomb_plist_xml, dimension_bomb_png, gray_png, zip_with,
};
use std::path::PathBuf;

const OPTIONS: PreviewOptions = PreviewOptions { max_cell: 8 };
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
        let _ = preview_abr(bytes, OPTIONS);
        let _ = preview_abr_first_available(bytes, OPTIONS, 4);
    }),
    ("preview_brush", |bytes| {
        let _ = preview_brush(bytes, OPTIONS);
        let _ = preview_brush_first_available(bytes, OPTIONS, 4);
    }),
    ("preview_brushset", |bytes| {
        let _ = preview_brushset(bytes, OPTIONS);
        let _ = preview_brushset_first_available(bytes, OPTIONS, 4);
    }),
];

fn corpus(target: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/corpus")
        .join(target)
}

fn sampled_abr() -> Vec<u8> {
    let mut entry = Vec::new();
    for value in [0u32, 0, 0, 8, 16] {
        entry.extend_from_slice(&value.to_be_bytes());
    }
    entry.extend_from_slice(&8u16.to_be_bytes());
    entry.push(0); // Uncompressed 8-bit samples.
    entry.extend_from_slice(&[200; 16 * 8]);
    let mut payload = (entry.len() as u32).to_be_bytes().to_vec();
    payload.extend_from_slice(&entry);
    payload.resize(payload.len().next_multiple_of(4), 0);
    let mut file = vec![0, 6, 0, 2];
    file.extend_from_slice(b"8BIMsamp");
    file.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    file.extend_from_slice(&payload);
    file
}

fn generated_seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let archive_a = brush_archive("A");
    let archive_b = brush_archive("B");
    let shape = gray_png(16, 8, 200);
    let mut seeds = vec![("preview_abr", "sampled_tip", sampled_abr())];
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
                ("Brush.archive", depth_bomb_plist_xml()),
                ("Shape.png", shape.clone()),
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
            zip_with(&[("brushset.plist", &depth_bomb_plist_xml())]),
        ),
    ]);
    seeds
}

#[test]
fn every_corpus_file_replays_without_panic() {
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
    let bytes = std::fs::read(corpus("preview_abr").join("sampled_tip")).unwrap();
    let set = preview_abr(&bytes, OPTIONS).expect("valid ABR seed decodes");
    assert_eq!(set.entries.len(), 1);
    let TipPreview::Available(tip) = &set.entries[0].tip else {
        panic!(
            "sampled_tip: expected a decoded tip, got {:?}",
            set.entries[0].tip
        );
    };
    assert_eq!((tip.width, tip.height), (8, 4));
    assert_eq!(tip.data, vec![200; 32]);
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
