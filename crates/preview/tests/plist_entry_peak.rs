mod common;
mod counting_alloc;

use brushkit_preview::procreate::MAX_PLIST_BYTES;
use brushkit_preview::{preview_brush, preview_brushset, PreviewOptions, TipPreview};
use common::{gray_png, zip_with};
use counting_alloc::{live, peak, reset_peak};

const MIB: usize = 1 << 20;

fn oversize_plist() -> Vec<u8> {
    let mut plist = b"bplist00".to_vec();
    plist.resize(2 * MAX_PLIST_BYTES, 0);
    plist
}

/// Rewrites the uncompressed size of the zip's only entry in both its local
/// header and its central directory record.
fn declare_size(mut zip: Vec<u8>, size: u32) -> Vec<u8> {
    let central = zip
        .windows(4)
        .rposition(|w| w == b"PK\x01\x02")
        .expect("central directory record");
    for at in [22, central + 24] {
        zip[at..at + 4].copy_from_slice(&size.to_le_bytes());
    }
    zip
}

fn growth(read: impl FnOnce()) -> usize {
    let before = live();
    reset_peak();
    read();
    peak() - before
}

#[test]
fn an_oversize_plist_entry_is_read_no_further_than_the_plist_limit() {
    let plist = oversize_plist();
    let honest = zip_with(&[("brushset.plist", &plist)]);
    let understated = declare_size(honest.clone(), 1024);
    let opts = PreviewOptions { max_cell: 8 };

    for (label, bytes) in [("declared", &honest), ("understated", &understated)] {
        let peak = growth(|| {
            let err = preview_brushset(bytes, opts).expect_err("oversize brushset.plist");
            assert!(err.0.starts_with("brushset.plist"), "{label}: {err}");
        });
        println!("brushset.plist, {label} size: {peak} bytes");
        assert!(
            peak <= MAX_PLIST_BYTES + MIB,
            "{label}: a {} MiB brushset.plist grew the heap by {peak} bytes",
            plist.len() / MIB
        );
    }

    let png = gray_png(4, 4, 0x80);
    let brush = zip_with(&[("Brush.archive", &plist), ("Shape.png", &png)]);
    let peak = growth(|| {
        let set = preview_brush(&brush, opts).expect("brush reads");
        assert!(
            matches!(&set.entries[0].tip, TipPreview::Unavailable(_)),
            "{:?}",
            set.entries[0].tip
        );
    });
    println!("Brush.archive: {peak} bytes");
    assert!(
        peak <= MAX_PLIST_BYTES + MIB,
        "a {} MiB Brush.archive grew the heap by {peak} bytes",
        plist.len() / MIB
    );
}
