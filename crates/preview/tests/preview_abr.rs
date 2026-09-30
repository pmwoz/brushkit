use brushkit_preview::{
    preview_abr, PreviewEntry, PreviewOptions, PreviewSet, TipPreview, UnavailableReason,
};
use std::path::Path;

mod common;
use common::{desc_abr, legacy_abr, DescPreset, SampTip, TIP_A, TIP_B, TIP_C};

const SEEDS: [&str; 2] = ["wellformed_v6_min", "wellformed_v6_patt"];

fn seed(name: &str) -> Vec<u8> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let path = Path::new(&manifest_dir)
        .join("../../fuzz/corpus/abr_parse")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("seed {name} must be readable: {e}"))
}

#[test]
fn every_preset_becomes_one_entry_that_fits_the_cell() {
    for name in SEEDS {
        let bytes = seed(name);
        let pack = brushkit_abr::parse_abr(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = pack.sampled_brushes.len()
            + pack.computed_presets.len()
            + pack.unsupported_tip_presets.len();

        let set = preview_abr(&bytes, PreviewOptions { max_cell: 4 })
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            set.set_name, None,
            "{name}: an .abr pack carries no set name"
        );
        assert_eq!(
            set.entries.len(),
            expected,
            "{name}: every preset appears exactly once"
        );
        assert_eq!(
            set.entries.iter().map(|e| e.index).collect::<Vec<_>>(),
            (0..expected).collect::<Vec<_>>(),
            "{name}: indices are dense and in order"
        );
        for entry in &set.entries {
            if let TipPreview::Available(tip) = &entry.tip {
                assert!(
                    tip.width <= 4 && tip.height <= 4,
                    "{name}: tip {}x{} exceeds the cell",
                    tip.width,
                    tip.height
                );
                assert_eq!(tip.data.len(), (tip.width * tip.height) as usize);
            }
        }
    }
}

#[test]
fn a_zero_cell_is_rejected() {
    let bytes = seed("wellformed_v6_min");
    assert!(
        preview_abr(&bytes, PreviewOptions { max_cell: 0 }).is_err(),
        "max_cell 0 has no valid output size"
    );
}

const SAMPLED_TIP: &[u8] = include_bytes!("../../../fuzz/corpus/preview_abr/sampled_tip");

fn samp_records(unreadable: &[bool]) -> Vec<u8> {
    const HEADER_AND_BLOCK_TAG: usize = 12;
    const RECORD_START: usize = HEADER_AND_BLOCK_TAG + 4;
    const DEPTH_IN_RECORD: usize = 24;
    const DEPTH_NO_HEADER_ACCEPTS: u16 = 0x20;
    let record = &SAMPLED_TIP[RECORD_START..];
    let mut payload = Vec::new();
    for &bad in unreadable {
        let at = payload.len() + DEPTH_IN_RECORD;
        payload.extend_from_slice(record);
        if bad {
            payload[at..at + 2].copy_from_slice(&DEPTH_NO_HEADER_ACCEPTS.to_be_bytes());
        }
    }
    let mut file = SAMPLED_TIP[..HEADER_AND_BLOCK_TAG].to_vec();
    file.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    file.extend_from_slice(&payload);
    file
}

fn corrupt(entry: &PreviewEntry) -> bool {
    matches!(
        entry.tip,
        TipPreview::Unavailable(UnavailableReason::Corrupt(_))
    )
}

#[test]
fn an_unreadable_samp_record_stays_a_corrupt_entry_in_its_place() {
    let set = preview_abr(
        &samp_records(&[false, true]),
        PreviewOptions { max_cell: 16 },
    )
    .unwrap();

    assert_eq!(set.entries.len(), 2, "both records are brushes");
    assert_eq!(set.not_reached, 0);
    let (unreadable, readable) = (&set.entries[0], &set.entries[1]);
    assert_eq!((unreadable.index, unreadable.name.as_str()), (0, "brush_1"));
    assert!(corrupt(unreadable), "{:?}", unreadable.tip);
    assert_eq!(unreadable.source_dimensions, None);
    assert_eq!((readable.index, readable.name.as_str()), (1, "brush_0"));
    assert!(matches!(readable.tip, TipPreview::Available(_)));
}

#[test]
fn a_pack_of_one_unreadable_samp_record_is_not_an_empty_success() {
    let set = preview_abr(&samp_records(&[true]), PreviewOptions { max_cell: 16 }).unwrap();

    assert_eq!(set.entries.len(), 1);
    assert_eq!(set.entries[0].name, "brush_0");
    assert!(corrupt(&set.entries[0]), "{:?}", set.entries[0].tip);
}

#[test]
fn a_legacy_entry_with_an_unknown_compression_stays_a_corrupt_entry_in_its_place() {
    const COMPRESSION_IN_ENTRY: usize = 2 + 4 + 37;
    let tip = || SampTip {
        width: 4,
        height: 4,
        fill: 200,
        corrupt: false,
    };
    let second_entry = legacy_abr(&[tip()]).len();
    let mut bytes = legacy_abr(&[tip(), tip()]);
    bytes[second_entry + COMPRESSION_IN_ENTRY] = 2;

    let set = preview_abr(&bytes, PreviewOptions { max_cell: 16 }).unwrap();

    assert_eq!(set.entries.len(), 2, "both entries are brushes");
    let (unknown, readable) = (&set.entries[0], &set.entries[1]);
    assert_eq!((unknown.index, unknown.name.as_str()), (0, "brush_1"));
    assert!(corrupt(unknown), "{:?}", unknown.tip);
    assert_eq!((readable.index, readable.name.as_str()), (1, "brush_0"));
    assert!(matches!(readable.tip, TipPreview::Available(_)));
}

fn names_and_tips(set: &PreviewSet) -> Vec<(&str, String)> {
    set.entries
        .iter()
        .map(|entry| {
            let tip = match &entry.tip {
                TipPreview::Available(_) => "available".to_string(),
                TipPreview::Unavailable(reason) => format!("{reason:?}"),
            };
            (entry.name.as_str(), tip)
        })
        .collect()
}

#[test]
fn named_presets_with_an_unreadable_or_missing_tip_stay_in_preset_order() {
    let bytes = desc_abr(
        &[(TIP_A, true), (TIP_B, false)],
        &[
            DescPreset::Sampled("A", TIP_A),
            DescPreset::Sampled("B", TIP_B),
            DescPreset::Computed("Round"),
            DescPreset::Sampled("C", TIP_C),
        ],
    );

    let set = preview_abr(&bytes, PreviewOptions { max_cell: 16 }).unwrap();

    assert_eq!(
        names_and_tips(&set),
        [
            ("A", "available".to_string()),
            ("B", r#"Corrupt("no bitmap header found")"#.to_string()),
            ("Round", "available".to_string()),
            ("C", format!(r#"Corrupt("sampled tip {TIP_C} is missing")"#)),
        ]
    );
}

#[test]
fn a_sampled_preset_with_no_samp_block_is_a_corrupt_entry() {
    let bytes = desc_abr(&[], &[DescPreset::Sampled("A", TIP_A)]);

    let set = preview_abr(&bytes, PreviewOptions { max_cell: 16 }).unwrap();

    assert_eq!(
        names_and_tips(&set),
        [("A", format!(r#"Corrupt("sampled tip {TIP_A} is missing")"#))]
    );
}
