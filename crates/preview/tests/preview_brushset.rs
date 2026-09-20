//! `preview_brushset` and `preview_brush` over synthetic Procreate archives:
//! member order, per-member availability and the max-cell fit.

mod common;

use brushkit_preview::{
    preview_brush, preview_brushset, PreviewOptions, TipPreview, UnavailableReason,
};
use common::{brush_archive, brushset_plist, gray_png, real_4x4_png, zip_with};

fn names(set: &brushkit_preview::PreviewSet) -> Vec<&str> {
    set.entries.iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn plist_order_wins_over_zip_order() {
    let archive_a = brush_archive("A");
    let archive_b = brush_archive("B");
    assert_eq!(
        brushkit_preview::procreate::brush_name(&archive_a),
        Ok(Some("A".to_string())),
        "the archive builder must produce a readable name"
    );
    let png = real_4x4_png();
    let plist = brushset_plist("My Set", &["uuid-b", "uuid-a"]);

    let bytes = zip_with(&[
        ("brushset.plist", &plist),
        ("uuid-a/Brush.archive", &archive_a),
        ("uuid-a/Shape.png", &png),
        ("uuid-b/Brush.archive", &archive_b),
        ("uuid-b/Shape.png", &png),
    ]);

    let set = preview_brushset(&bytes, PreviewOptions { max_cell: 8 }).expect("set reads");
    assert_eq!(set.set_name, Some("My Set".to_string()));
    assert_eq!(names(&set), vec!["B", "A"]);
}

#[test]
fn no_plist_falls_back_to_zip_order() {
    let archive_a = brush_archive("A");
    let archive_b = brush_archive("B");
    let png = real_4x4_png();

    let bytes = zip_with(&[
        ("A/Brush.archive", &archive_a),
        ("A/Shape.png", &png),
        ("A/Reset/Brush.archive", &archive_a),
        ("A/QuickLook/Thumbnail.png", &png),
        ("B.brush/Brush.archive", &archive_b),
        ("B.brush/Shape.png", &png),
    ]);

    let set = preview_brushset(&bytes, PreviewOptions { max_cell: 8 }).expect("set reads");
    assert_eq!(set.set_name, None, "a set without a plist has no name");
    assert_eq!(
        names(&set),
        vec!["A", "B"],
        "Reset/ and QuickLook/ are not members"
    );
}

#[test]
fn member_without_shape_png_is_unavailable_at_its_index() {
    let png = real_4x4_png();
    let plist = brushset_plist("S", &["u0", "u1", "u2"]);
    let a0 = brush_archive("zero");
    let a1 = brush_archive("one");
    let a2 = brush_archive("two");

    let bytes = zip_with(&[
        ("brushset.plist", &plist),
        ("u0/Brush.archive", &a0),
        ("u0/Shape.png", &png),
        ("u1/Brush.archive", &a1),
        ("u2/Brush.archive", &a2),
        ("u2/Shape.png", &png),
    ]);

    let set = preview_brushset(&bytes, PreviewOptions { max_cell: 8 }).expect("set reads");
    assert_eq!(set.entries.len(), 3);
    assert_eq!(
        set.entries.iter().map(|e| e.index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(matches!(
        set.entries[1].tip,
        TipPreview::Unavailable(UnavailableReason::NoShapePng)
    ));
    assert!(matches!(set.entries[0].tip, TipPreview::Available(_)));
    assert!(matches!(set.entries[2].tip, TipPreview::Available(_)));
}

#[test]
fn tips_fit_max_cell() {
    let archive = brush_archive("wide");
    let png = gray_png(64, 16, 200);
    let plist = brushset_plist("S", &["u"]);

    let bytes = zip_with(&[
        ("brushset.plist", &plist),
        ("u/Brush.archive", &archive),
        ("u/Shape.png", &png),
    ]);

    let set = preview_brushset(&bytes, PreviewOptions { max_cell: 8 }).expect("set reads");
    let TipPreview::Available(tip) = &set.entries[0].tip else {
        panic!("expected an available tip, got {:?}", set.entries[0].tip);
    };
    assert_eq!(tip.width, 8);
    assert!(tip.height <= 8, "height {} exceeds the cell", tip.height);
    assert_eq!(tip.data.len(), (tip.width * tip.height) as usize);
}

#[test]
fn single_brush_root_layout() {
    let bytes = zip_with(&[
        ("Brush.archive", &brush_archive("Solo")),
        ("Shape.png", &real_4x4_png()),
    ]);

    let set = preview_brush(&bytes, PreviewOptions { max_cell: 8 }).expect("brush reads");
    assert_eq!(set.set_name, None);
    assert_eq!(set.entries.len(), 1);
    assert_eq!(set.entries[0].index, 0);
    assert_eq!(set.entries[0].name, "Solo");
    assert!(matches!(set.entries[0].tip, TipPreview::Available(_)));
}
