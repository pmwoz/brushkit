mod common;

use brushkit_preview::{
    preview_abr, preview_abr_first_available, preview_brush, preview_brush_first_available,
    preview_brushset, preview_brushset_first_available, PreviewError, PreviewOptions, PreviewSet,
    TipPreview,
};
use common::{
    brush_archive, brushset_plist, corpus_files, gray_png, legacy_abr, samp_abr, zip_with, SampTip,
};
use std::path::Path;

type Preview = fn(&[u8], PreviewOptions) -> Result<PreviewSet, PreviewError>;
type FirstAvailable = fn(&[u8], PreviewOptions, usize) -> Result<PreviewSet, PreviewError>;

const OPTS: PreviewOptions = PreviewOptions { max_cell: 8 };

fn is_available(tip: &TipPreview) -> bool {
    matches!(tip, TipPreview::Available(_))
}

/// Asserts that `first` is the first `n` available entries of `full`, entry
/// for entry, with the same set name.
fn assert_first_available(full: &PreviewSet, first: &PreviewSet, n: usize, context: &str) {
    assert_eq!(first.set_name, full.set_name, "{context}: set name");
    let expected: Vec<_> = full
        .entries
        .iter()
        .filter(|entry| is_available(&entry.tip))
        .take(n)
        .collect();
    assert_eq!(
        first.entries.len(),
        expected.len(),
        "{context}: entry count for n = {n}"
    );
    for (got, want) in first.entries.iter().zip(expected) {
        assert_eq!(got.index, want.index, "{context}: n = {n}");
        assert_eq!(got.name, want.name, "{context}: n = {n}");
        assert_eq!(
            got.source_dimensions, want.source_dimensions,
            "{context}: n = {n}"
        );
        let (TipPreview::Available(got), TipPreview::Available(want)) = (&got.tip, &want.tip)
        else {
            panic!("{context}: n = {n}: entry {} is not available", want.index);
        };
        assert!(
            got.width > 0 && got.height > 0,
            "{context}: n = {n}: zero-area tip"
        );
        assert_eq!(
            (got.width, got.height, &got.data),
            (want.width, want.height, &want.data),
            "{context}: n = {n}"
        );
    }
}

/// Checks every `n` from 0 to one past the entry count and returns the full
/// preview.
fn check_every_n(bytes: &[u8], full: Preview, first: FirstAvailable) -> PreviewSet {
    let full = full(bytes, OPTS).unwrap();
    for n in 0..=full.entries.len() + 1 {
        let first = first(bytes, OPTS, n).unwrap();
        assert_first_available(&full, &first, n, "fixture");
    }
    full
}

fn availability(set: &PreviewSet) -> Vec<bool> {
    set.entries
        .iter()
        .map(|entry| is_available(&entry.tip))
        .collect()
}

#[test]
fn abr_returns_the_first_available_tips_of_the_full_preview() {
    let tip = |side: u32, fill: u8, corrupt: bool| SampTip {
        width: side,
        height: side / 2 + 1,
        fill,
        corrupt,
    };
    // The parser lists these brushes in reverse, so the preview starts with
    // the corrupt tip at the end of the block.
    let bytes = samp_abr(&[
        tip(3, 0x20, false),
        tip(12, 0x40, false),
        tip(5, 0x60, true),
        tip(16, 0x80, true),
        tip(7, 0xa0, false),
        tip(20, 0xc0, true),
    ]);
    let full = check_every_n(&bytes, preview_abr, preview_abr_first_available);
    assert_eq!(
        availability(&full),
        vec![false, true, false, false, true, true]
    );

    // The failed first tip is skipped and the next available one takes its place.
    let first = preview_abr_first_available(&bytes, OPTS, 1).unwrap();
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].index, 1);
    let indices: Vec<_> = preview_abr_first_available(&bytes, OPTS, 3)
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.index)
        .collect();
    assert_eq!(indices, vec![1, 4, 5]);
}

#[test]
fn v2_abr_returns_the_first_available_tips_of_the_full_preview() {
    let tip = |side: u32, fill: u8, corrupt: bool| SampTip {
        width: side,
        height: side / 2 + 1,
        fill,
        corrupt,
    };
    // Listed in reverse, as in the v6 test above.
    let bytes = legacy_abr(&[
        tip(3, 0x20, false),
        tip(12, 0x40, true),
        tip(5, 0x60, false),
        tip(16, 0x80, false),
        tip(7, 0xa0, true),
    ]);
    let full = check_every_n(&bytes, preview_abr, preview_abr_first_available);
    assert_eq!(availability(&full), vec![false, true, true, false, true]);
}

#[test]
fn brushset_returns_the_first_available_tips_of_the_full_preview() {
    let archive = brush_archive("Tip");
    let small = gray_png(4, 2, 50);
    let large = gray_png(32, 16, 150);
    let square = gray_png(6, 6, 250);
    let idat = small.windows(4).position(|bytes| bytes == b"IDAT").unwrap();
    let corrupt_pixels = &small[..idat + 4];
    let members = ["missing", "pixels", "small", "archive", "large", "square"];
    let plist = brushset_plist("Mixed", &members);
    let bytes = zip_with(&[
        ("brushset.plist", &plist),
        ("missing/Brush.archive", &archive),
        ("pixels/Brush.archive", &archive),
        ("pixels/Shape.png", corrupt_pixels),
        ("small/Brush.archive", &archive),
        ("small/Shape.png", &small),
        ("archive/Brush.archive", b"bad archive"),
        ("archive/Shape.png", &small),
        ("large/Brush.archive", &archive),
        ("large/Shape.png", &large),
        ("square/Brush.archive", &archive),
        ("square/Shape.png", &square),
    ]);
    let full = check_every_n(&bytes, preview_brushset, preview_brushset_first_available);
    assert_eq!(full.set_name.as_deref(), Some("Mixed"));
    assert_eq!(
        availability(&full),
        vec![false, false, true, false, true, true]
    );
}

#[test]
fn brush_returns_its_tip_only_when_available() {
    let archive = brush_archive("Single");
    let available = zip_with(&[
        ("Brush.archive", &archive),
        ("Shape.png", &gray_png(10, 4, 90)),
    ]);
    let full = check_every_n(&available, preview_brush, preview_brush_first_available);
    assert_eq!(availability(&full), vec![true]);

    let missing = zip_with(&[("Brush.archive", &archive)]);
    let full = check_every_n(&missing, preview_brush, preview_brush_first_available);
    assert_eq!(availability(&full), vec![false]);
}

#[test]
fn real_files_return_the_first_available_tips_of_the_full_preview() {
    let Some(root) = std::env::var_os("BRUSHKIT_CORPUS_DIR") else {
        return;
    };
    let mut files = Vec::new();
    corpus_files(Path::new(&root), &mut files);
    assert!(
        !files.is_empty(),
        "configured corpus must contain brush files"
    );
    let mut checked = 0;
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let (full, first): (Preview, FirstAvailable) = match path
            .extension()
            .unwrap()
            .to_str()
            .unwrap()
            .to_lowercase()
            .as_str()
        {
            "abr" => (preview_abr, preview_abr_first_available),
            "brush" => (preview_brush, preview_brush_first_available),
            _ => (preview_brushset, preview_brushset_first_available),
        };
        let full = match full(&bytes, OPTS) {
            Ok(set) => set,
            Err(error) => {
                println!("skipped {}: {error}", path.display());
                assert!(first(&bytes, OPTS, 1).is_err());
                continue;
            }
        };
        for n in [1, 4] {
            let first = first(&bytes, OPTS, n)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_first_available(&full, &first, n, &path.display().to_string());
        }
        checked += 1;
        println!(
            "checked {} entries in {}",
            full.entries.len(),
            path.display()
        );
    }
    assert!(checked > 0, "corpus must contain a readable brush file");
}
