use brushkit_preview::{preview_abr, PreviewOptions, PreviewSet, TipPreview};
use std::path::{Path, PathBuf};

fn without_patterns(bytes: &[u8]) -> (Vec<u8>, usize) {
    let version = u16::from_be_bytes(bytes[..2].try_into().unwrap());
    if matches!(version, 1 | 2) {
        return (bytes.to_vec(), 0);
    }
    let mut stripped = bytes[..4].to_vec();
    let mut offset = 4;
    let mut omitted = 0;
    while offset + 12 <= bytes.len() && &bytes[offset..offset + 4] == b"8BIM" {
        let length =
            u32::from_be_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
        let end = offset + 12 + length;
        assert!(
            end <= bytes.len(),
            "the corpus block must fit inside the file"
        );
        let next = end.next_multiple_of(4).min(bytes.len());
        if &bytes[offset + 4..offset + 8] == b"patt" {
            omitted += 1;
        } else {
            stripped.extend_from_slice(&bytes[offset..next]);
        }
        offset = next;
    }
    stripped.extend_from_slice(&bytes[offset..]);
    (stripped, omitted)
}

fn assert_same_preview(actual: &PreviewSet, expected: &PreviewSet) {
    assert_eq!(actual.set_name, expected.set_name);
    assert_eq!(actual.entries.len(), expected.entries.len());
    for (actual, expected) in actual.entries.iter().zip(&expected.entries) {
        assert_eq!(actual.index, expected.index);
        assert_eq!(actual.name, expected.name);
        assert_eq!(actual.source_dimensions, expected.source_dimensions);
        match (&actual.tip, &expected.tip) {
            (TipPreview::Available(actual), TipPreview::Available(expected)) => {
                assert_eq!(actual.width, expected.width);
                assert_eq!(actual.height, expected.height);
                assert_eq!(actual.data, expected.data);
            }
            (TipPreview::Unavailable(actual), TipPreview::Unavailable(expected)) => {
                assert_eq!(actual, expected);
            }
            _ => panic!("preview availability changed for {}", actual.name),
        }
    }
}

fn check_preview(bytes: &[u8]) -> usize {
    let options = PreviewOptions { max_cell: 160 };
    let original = preview_abr(bytes, options).expect("original pack previews");
    let (stripped, omitted) = without_patterns(bytes);
    let without = preview_abr(&stripped, options).expect("pack without patterns previews");
    assert_same_preview(&original, &without);
    omitted
}

#[test]
fn patterns_do_not_change_preview_output() {
    let bytes = include_bytes!("../../../fuzz/corpus/abr_parse/wellformed_v6_patt");
    assert!(!brushkit_abr::parse_abr(bytes).unwrap().patterns.is_empty());
    assert!(check_preview(bytes) > 0);
}

fn abr_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("corpus directory is readable") {
        let path = entry.expect("corpus entry is readable").path();
        if path.is_dir() {
            abr_files(&path, files);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("abr"))
        {
            files.push(path);
        }
    }
}

#[test]
fn corpus_patterns_do_not_change_preview_output() {
    let Some(root) = std::env::var_os("BRUSHKIT_CORPUS_DIR") else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset");
        return;
    };
    let mut files = Vec::new();
    abr_files(Path::new(&root), &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "the configured corpus must contain ABR files"
    );
    let mut patterned = 0;
    for path in &files {
        println!("checking {}", path.display());
        let bytes = std::fs::read(path).expect("corpus file is readable");
        if check_preview(&bytes) > 0 {
            patterned += 1;
        }
    }
    println!(
        "checked {} packs, {patterned} with pattern blocks",
        files.len()
    );
}
