mod common;

use brushkit_preview::{
    preview_abr, preview_brush, preview_brushset, PreviewOptions, SourceDimensions, TipPreview,
    UnavailableReason,
};
use common::{brush_archive, brushset_plist, dimension_bomb_png, gray_png, zip_with};
use std::path::{Path, PathBuf};

#[test]
fn dimensions_require_two_positive_sides() {
    assert_eq!(SourceDimensions::new(0, 1), None);
    assert_eq!(SourceDimensions::new(1, 0), None);
    let dimensions = SourceDimensions::new(16, 8).unwrap();
    assert_eq!((dimensions.width(), dimensions.height()), (16, 8));
}

#[test]
fn sampled_abr_dimensions_survive_downsampling() {
    let bytes = include_bytes!("../../../fuzz/corpus/preview_abr/sampled_tip");
    for max_cell in [1, 4, 64] {
        let set = preview_abr(bytes, PreviewOptions { max_cell }).unwrap();
        assert_eq!(set.entries.len(), 1);
        assert_eq!(
            set.entries[0].source_dimensions,
            SourceDimensions::new(16, 8)
        );
        assert!(matches!(set.entries[0].tip, TipPreview::Available(_)));
    }
}

#[test]
fn sampled_dimensions_survive_decode_failure() {
    let mut bytes = include_bytes!("../../../fuzz/corpus/preview_abr/sampled_tip").to_vec();
    bytes[42] = 1; // RLE, with row lengths larger than the remaining payload.
    let set = preview_abr(&bytes, PreviewOptions { max_cell: 8 }).unwrap();
    assert_eq!(set.entries.len(), 1);
    assert_eq!(
        set.entries[0].source_dimensions,
        SourceDimensions::new(16, 8)
    );
    assert!(matches!(
        set.entries[0].tip,
        TipPreview::Unavailable(UnavailableReason::Corrupt(_))
    ));
}

#[test]
fn procreate_dimensions_follow_members_and_survive_pixel_failures() {
    let archive = brush_archive("Same name");
    let shape = gray_png(64, 16, 200);
    // Retain IHDR and the IDAT header, but remove the compressed pixel data.
    let idat = shape.windows(4).position(|bytes| bytes == b"IDAT").unwrap();
    let corrupt_pixels = &shape[..idat + 4];
    let plist = brushset_plist(
        "Sizes",
        &["missing", "valid", "pixels", "header", "large", "archive"],
    );
    let bytes = zip_with(&[
        ("brushset.plist", &plist),
        ("valid/Brush.archive", &archive),
        ("valid/Shape.png", &shape),
        ("missing/Brush.archive", &archive),
        ("pixels/Brush.archive", &archive),
        ("pixels/Shape.png", corrupt_pixels),
        ("header/Brush.archive", &archive),
        ("header/Shape.png", b"bad header"),
        ("large/Brush.archive", &archive),
        ("large/Shape.png", &dimension_bomb_png(60000, 30000)),
        ("archive/Brush.archive", b"bad archive"),
        ("archive/Shape.png", &shape),
    ]);
    for max_cell in [1, 8, 128] {
        let set = preview_brushset(&bytes, PreviewOptions { max_cell }).unwrap();
        assert_eq!(
            set.entries
                .iter()
                .map(|entry| entry.index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5]
        );
        assert_eq!(
            set.entries
                .iter()
                .map(|entry| entry.source_dimensions)
                .collect::<Vec<_>>(),
            vec![
                None,
                SourceDimensions::new(64, 16),
                SourceDimensions::new(64, 16),
                None,
                SourceDimensions::new(60000, 30000),
                SourceDimensions::new(64, 16)
            ]
        );
        assert!(matches!(
            set.entries[0].tip,
            TipPreview::Unavailable(UnavailableReason::NoShapePng)
        ));
        assert!(matches!(set.entries[1].tip, TipPreview::Available(_)));
        assert!(matches!(
            set.entries[2].tip,
            TipPreview::Unavailable(UnavailableReason::Corrupt(_))
        ));
        assert!(matches!(
            set.entries[4].tip,
            TipPreview::Unavailable(UnavailableReason::TooLarge {
                width: 60000,
                height: 30000
            })
        ));
        assert!(matches!(
            set.entries[5].tip,
            TipPreview::Unavailable(UnavailableReason::Corrupt(_))
        ));
    }
}

#[test]
fn computed_and_unsupported_previews_have_no_source_raster() {
    // One computed preset, with a diameter and otherwise default geometry.
    let mut descriptor = vec![0, 0, 0, 16, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    descriptor.extend_from_slice(b"null\0\0\0\x01\0\0\0\0BrshVlLs\0\0\0\x01Objc\0\0\0\x01\0\0\0\0\0\x0bbrushPreset\0\0\0\x01\0\0\0\0BrshObjc\0\0\0\x01\0\0\0\0\0\x0dcomputedBrush\0\0\0\x01\0\0\0\0DmtrUntF#Pxl");
    descriptor.extend_from_slice(&30f64.to_be_bytes());
    let mut bytes = vec![0, 6, 0, 2];
    bytes.extend_from_slice(b"8BIMdesc");
    bytes.extend_from_slice(&(descriptor.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&descriptor);
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    let set = preview_abr(&bytes, PreviewOptions { max_cell: 8 }).unwrap();
    assert_eq!(set.entries.len(), 1);
    assert_eq!(set.entries[0].source_dimensions, None);
    assert!(matches!(set.entries[0].tip, TipPreview::Available(_)));

    let class = bytes
        .windows(13)
        .position(|value| value == b"computedBrush")
        .unwrap();
    bytes[class..class + 13].copy_from_slice(b"computedBrusX");
    let set = preview_abr(&bytes, PreviewOptions { max_cell: 8 }).unwrap();
    assert_eq!(set.entries.len(), 1);
    assert_eq!(set.entries[0].source_dimensions, None);
    assert!(matches!(
        set.entries[0].tip,
        TipPreview::Unavailable(UnavailableReason::UnsupportedTipKind(_))
    ));
}

fn corpus_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("read corpus directory") {
        let path = entry.unwrap().path();
        if path.is_dir() {
            corpus_files(&path, files);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext.to_lowercase().as_str(), "abr" | "brush" | "brushset"))
        {
            files.push(path);
        }
    }
}

#[test]
fn real_file_dimensions_are_independent_of_cell_size() {
    let Some(root) = std::env::var_os("BRUSHKIT_CORPUS_DIR") else {
        return;
    };
    let mut files = Vec::new();
    corpus_files(Path::new(&root), &mut files);
    assert!(
        !files.is_empty(),
        "configured corpus must contain brush files"
    );
    let mut known = 0;
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let reader = match path
            .extension()
            .unwrap()
            .to_str()
            .unwrap()
            .to_lowercase()
            .as_str()
        {
            "abr" => preview_abr,
            "brush" => preview_brush,
            _ => preview_brushset,
        };
        let small = reader(&bytes, PreviewOptions { max_cell: 8 }).unwrap();
        let large = reader(&bytes, PreviewOptions { max_cell: 160 }).unwrap();
        assert_eq!(small.entries.len(), large.entries.len());
        for (small, large) in small.entries.iter().zip(&large.entries) {
            assert_eq!(small.name, large.name);
            assert_eq!(small.index, large.index);
            assert_eq!(small.source_dimensions, large.source_dimensions);
            if let Some(dimensions) = large.source_dimensions {
                known += 1;
                assert!(dimensions.width() > 0 && dimensions.height() > 0);
                if let TipPreview::Available(bitmap) = &large.tip {
                    assert!(
                        dimensions.width() >= bitmap.width && dimensions.height() >= bitmap.height
                    );
                }
            }
        }
        println!(
            "checked {} entries in {}",
            large.entries.len(),
            path.display()
        );
    }
    assert!(known > 0, "corpus must exercise known raster dimensions");
}
