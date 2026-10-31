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
    let mut with_up_front = 0;
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
                (&brush.id, &brush.uuid, &brush.name, brush.preset_index),
                (
                    &expected.id,
                    &expected.uuid,
                    &expected.name,
                    expected.preset_index
                ),
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
                (&dropped.uuid, &dropped.owners, &dropped.bitmap.data),
                (&expected.uuid, &expected.owners, &expected.bitmap.data),
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

        let held: usize = (0..pack.brushes.len())
            .filter(|&i| !deferred.is_deferred(i))
            .map(|i| pack.brushes[i].tip.data.len())
            .chain(pack.dropped_tip_details.iter().map(|d| d.bitmap.data.len()))
            .sum();
        let up_front = deferred.up_front_decoded_len();
        let all = brushkit_abr::parse_abr_all_deferred_without_patterns(&bytes)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(all.up_front_decoded_len(), up_front, "{name}");
        let unreadable_tips = |pack: &brushkit_abr::AbrPack| {
            pack.unreadable_tip_details.len()
                + pack
                    .sampled_brushes
                    .iter()
                    .filter(|b| {
                        matches!(
                            b,
                            brushkit_abr::SampledBrush::Unavailable(
                                brushkit_abr::UnavailableBrush {
                                    cause: brushkit_abr::UnavailableTip::Unreadable(_),
                                    ..
                                }
                            )
                        )
                    })
                    .count()
        };
        if unreadable_tips(pack) > unreadable_tips(&all.pack) {
            assert!(up_front >= held, "{name}: {up_front} < {held}");
        } else {
            assert_eq!(up_front, held, "{name}");
        }
        if up_front > 0 {
            with_up_front += 1;
        }
        checked += 1;
        unreadable += eager.unreadable_tip_details.len()
            + eager
                .sampled_brushes
                .iter()
                .filter(|b| matches!(b, brushkit_abr::SampledBrush::Unavailable(_)))
                .count();
    }
    println!(
        "checked {checked} of {} corpus pack(s), {unreadable} unreadable or missing tip(s), \
         {with_up_front} with tips decoded up front",
        files.len()
    );
}

fn sampled_data(items: &[brushkit_abr::dump::DumpItem], out: &mut Vec<String>) {
    for item in items {
        match &item.value {
            brushkit_abr::dump::DumpValue::Text(text) if item.key == "sampledData" => {
                out.push(text.clone());
            }
            brushkit_abr::dump::DumpValue::Objc { items, .. } => sampled_data(items, out),
            _ => {}
        }
    }
}

#[test]
fn every_corpus_brush_uuid_is_a_sampled_data_uuid_of_its_preset() {
    let Some((root, files)) = corpus() else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset");
        return;
    };
    let (mut matched, mut repeats, mut without) = (0, 0, 0);
    for path in &files {
        let bytes = std::fs::read(path).expect("corpus file is readable");
        let name = path.strip_prefix(&root).unwrap_or(path).display();
        let pack = brushkit_abr::parse_abr(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let presets = pack
            .raw_desc_block
            .as_deref()
            .map(|desc| brushkit_abr::dump::dump_descriptors(desc).presets)
            .unwrap_or_default();
        let sampled = pack.sampled_brushes.iter().map(|b| match b {
            brushkit_abr::SampledBrush::Readable(i) => {
                let b = &pack.brushes[*i];
                (&b.id, &b.uuid, b.preset_index)
            }
            brushkit_abr::SampledBrush::Unavailable(b) => (&b.id, &b.uuid, b.preset_index),
        });
        for (id, uuid, preset_index) in sampled {
            let Some(uuid) = uuid else {
                assert!(id.starts_with("brush_"), "{name}: {id} has no uuid");
                without += 1;
                continue;
            };
            if id != uuid {
                let n = id.strip_prefix(&format!("{uuid}-"));
                assert!(
                    n.is_some_and(|n| n.parse::<usize>().is_ok_and(|n| n >= 2)),
                    "{name}: id {id} is not uuid {uuid} or a repeat of it"
                );
                repeats += 1;
            }
            if let Some(preset) = preset_index.and_then(|pi| presets.get(pi)) {
                let mut uuids = Vec::new();
                sampled_data(&preset.items, &mut uuids);
                assert!(uuids.contains(uuid), "{name}: {uuid} is not in {uuids:?}");
                matched += 1;
            }
        }
    }
    println!("{matched} uuid(s) matched a preset, {repeats} repeat(s), {without} without a uuid");
    assert!(matched > 0, "no corpus brush matched a preset");
}

const BRVR_CONTROL_FIELDS: [&str; 8] = [
    "flow_control",
    "wetness_control",
    "mix_control",
    "roundness_control",
    "scatter_control",
    "count_control",
    "texture_depth_control",
    "color_fg_bg_control",
];

fn brvr_controls(d: &brushkit_abr::BrushDescriptor) -> [Option<i32>; 8] {
    [
        d.flow_control,
        d.wetness_control,
        d.mix_control,
        d.roundness_control,
        d.scatter_control,
        d.count_control,
        d.texture_depth_control,
        d.color_fg_bg_control,
    ]
}

#[test]
fn corpus_sets_a_nonzero_control_on_every_brvr_panel() {
    let Some((root, files)) = corpus() else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset");
        return;
    };
    let mut presets = [0usize; 8];
    for path in &files {
        let bytes = std::fs::read(path).expect("corpus file is readable");
        let name = path.strip_prefix(&root).unwrap_or(path).display();
        let all = brushkit_abr::parse_abr_all_deferred_without_patterns(&bytes)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let descriptors = all
            .pack
            .brushes
            .iter()
            .map(|b| &b.descriptor)
            .chain(all.pack.computed_presets.iter().map(|p| &p.descriptor));
        for d in descriptors {
            for (count, control) in presets.iter_mut().zip(brvr_controls(d)) {
                if control.is_some_and(|c| c != 0) {
                    *count += 1;
                }
            }
        }
    }
    for (field, count) in BRVR_CONTROL_FIELDS.iter().zip(presets) {
        println!("{field}: {count} preset(s) with a nonzero control");
        assert!(count > 0, "no corpus preset sets a nonzero {field}");
    }
}
