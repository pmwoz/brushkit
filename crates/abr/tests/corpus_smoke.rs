use std::path::{Path, PathBuf};

fn abr_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            abr_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("abr"))
        {
            out.push(path);
        }
    }
}

/// Every `.abr` under `BRUSHKIT_CORPUS_DIR`, sorted, with the root; `None`
/// when the variable is unset or not a directory.
fn corpus() -> Option<(PathBuf, Vec<PathBuf>)> {
    let root = std::env::var_os("BRUSHKIT_CORPUS_DIR").map(PathBuf::from)?;
    if !root.is_dir() {
        return None;
    }
    let mut files = Vec::new();
    abr_files(&root, &mut files);
    files.sort();
    Some((root, files))
}

#[test]
fn every_corpus_pack_parses() {
    let Some((root, files)) = corpus() else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset");
        return;
    };
    for path in &files {
        let bytes = std::fs::read(path).expect("corpus file is readable");
        let name = path.strip_prefix(&root).unwrap_or(path).display();
        assert!(
            brushkit_abr::parse_abr(&bytes).is_ok(),
            "{name} failed to parse"
        );
    }
    println!("parsed {} corpus pack(s)", files.len());
}

#[test]
fn all_deferred_parse_decodes_every_corpus_tip_like_parse_abr() {
    let Some((root, files)) = corpus() else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset");
        return;
    };
    let mut checked = 0;
    let mut legacy = 0;
    for path in &files {
        let bytes = std::fs::read(path).expect("corpus file is readable");
        let name = path.strip_prefix(&root).unwrap_or(path).display();
        let Ok(eager) = brushkit_abr::parse_abr(&bytes) else {
            continue;
        };
        let all = brushkit_abr::parse_abr_all_deferred_without_patterns(&bytes)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(all.pack.brushes.len(), eager.brushes.len(), "{name}");
        for (i, brush) in eager.brushes.iter().enumerate() {
            let tip = all
                .decode_tip(i)
                .unwrap_or_else(|e| panic!("{name}: brush {i}: {e}"));
            assert_eq!(
                (tip.width, tip.height, tip.depth, &tip.data),
                (
                    brush.tip.width,
                    brush.tip.height,
                    brush.tip.depth,
                    &brush.tip.data
                ),
                "{name}: brush {i}"
            );
        }
        assert_eq!(
            all.pack.dropped_tip_details.len(),
            eager.dropped_tip_details.len(),
            "{name}"
        );
        for (j, (a, e)) in all
            .pack
            .dropped_tip_details
            .iter()
            .zip(&eager.dropped_tip_details)
            .enumerate()
        {
            assert_eq!(
                (a.bitmap.width, a.bitmap.height),
                (e.bitmap.width, e.bitmap.height),
                "{name}: dropped tip {j}"
            );
        }
        checked += 1;
        if matches!(
            eager.version,
            brushkit_abr::AbrVersion::V1 | brushkit_abr::AbrVersion::V2
        ) {
            legacy += 1;
        }
    }
    println!(
        "checked {checked} of {} corpus pack(s), {legacy} v1/v2",
        files.len()
    );
}

#[test]
fn deferred_parse_yields_the_pack_parse_abr_does() {
    let Some((root, files)) = corpus() else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset");
        return;
    };
    let mut checked = 0;
    let mut unreadable = 0;
    for path in &files {
        let bytes = std::fs::read(path).expect("corpus file is readable");
        let name = path.strip_prefix(&root).unwrap_or(path).display();
        let Ok(eager) = brushkit_abr::parse_abr(&bytes) else {
            continue;
        };
        let deferred =
            brushkit_abr::parse_abr_deferred(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let pack = &deferred.pack;
        assert_eq!(pack.sampled_brushes, eager.sampled_brushes, "{name}");
        assert_eq!(
            pack.unreadable_tip_details, eager.unreadable_tip_details,
            "{name}"
        );
        assert_eq!(pack.brushes.len(), eager.brushes.len(), "{name}");
        for (i, (brush, expected)) in pack.brushes.iter().zip(&eager.brushes).enumerate() {
            assert_eq!(
                (&brush.id, &brush.name, brush.preset_index),
                (&expected.id, &expected.name, expected.preset_index),
                "{name}: brush {i}"
            );
            let tip = deferred
                .decode_tip(i)
                .unwrap_or_else(|e| panic!("{name}: brush {i}: {e}"));
            assert_eq!(
                (tip.width, tip.height, tip.depth, &tip.data),
                (
                    expected.tip.width,
                    expected.tip.height,
                    expected.tip.depth,
                    &expected.tip.data
                ),
                "{name}: brush {i}"
            );
        }
        assert_eq!(
            pack.dropped_tip_details.len(),
            eager.dropped_tip_details.len(),
            "{name}"
        );
        for (j, (dropped, expected)) in pack
            .dropped_tip_details
            .iter()
            .zip(&eager.dropped_tip_details)
            .enumerate()
        {
            assert_eq!(
                (
                    &dropped.uuid,
                    &dropped.owner_preset_names,
                    &dropped.bitmap.data
                ),
                (
                    &expected.uuid,
                    &expected.owner_preset_names,
                    &expected.bitmap.data
                ),
                "{name}: dropped tip {j}"
            );
        }
        let skipped = |pack: &brushkit_abr::AbrPack| {
            pack.skipped_preset_details
                .iter()
                .map(|s| (s.name.clone(), s.uuid.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(skipped(pack), skipped(&eager), "{name}");
        checked += 1;
        unreadable += eager.unreadable_tip_details.len()
            + eager
                .sampled_brushes
                .iter()
                .filter(|b| matches!(b, brushkit_abr::SampledBrush::Unavailable(_)))
                .count();
    }
    println!(
        "checked {checked} of {} corpus pack(s), {unreadable} unreadable or missing tip(s)",
        files.len()
    );
}
