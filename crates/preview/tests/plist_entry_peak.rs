mod common;
mod counting_alloc;

use std::io::{self, Cursor, Read, Write};

use brushkit_preview::procreate::MAX_PLIST_BYTES;
use brushkit_preview::{
    preview_brush, preview_brushset, PreviewOptions, TipPreview, UnavailableReason,
};
use common::gray_png;
use counting_alloc::{live, peak, reset_peak};

const MIB: usize = 1 << 20;
const PLIST_BYTES: u64 = 2 * MAX_PLIST_BYTES as u64;

/// A deflated zip whose first entry is a binary plist header followed by
/// zeros up to [`PLIST_BYTES`].
fn zip_with_oversize_plist(path: &str, rest: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::write::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(path, opts).expect("start plist");
    zip.write_all(b"bplist00").expect("plist header");
    io::copy(&mut io::repeat(0).take(PLIST_BYTES - 8), &mut zip).expect("plist zeros");
    for (path, bytes) in rest {
        zip.start_file(*path, opts).expect("start entry");
        zip.write_all(bytes).expect("entry bytes");
    }
    zip.finish().expect("finish zip").into_inner()
}

/// Rewrites the uncompressed size of the zip's first entry in its central
/// directory record, the only size the zip reader consults.
fn declare_size(mut zip: Vec<u8>, size: u32) -> Vec<u8> {
    let central = zip
        .windows(4)
        .position(|w| w == b"PK\x01\x02")
        .expect("central directory record");
    zip[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
    zip
}

fn growth(read: impl FnOnce()) -> usize {
    let before = live();
    reset_peak();
    read();
    peak() - before
}

#[test]
fn an_oversize_plist_entry_is_rejected_before_it_is_read() {
    let declared = zip_with_oversize_plist("brushset.plist", &[]);
    let understated = declare_size(declared.clone(), 1024);
    let opts = PreviewOptions { max_cell: 8 };

    for (label, bytes, reason) in [
        ("declared", &declared, "declared size"),
        ("understated", &understated, "exceeds its declared size"),
    ] {
        let peak = growth(|| {
            let err = preview_brushset(bytes, opts).expect_err("oversize brushset.plist");
            assert!(
                err.0.starts_with("brushset.plist") && err.0.contains(reason),
                "{label}: {err}"
            );
        });
        println!("brushset.plist, {label} size: {peak} bytes");
        assert!(
            peak <= MIB,
            "{label}: a {PLIST_BYTES}-byte brushset.plist grew the heap by {peak} bytes"
        );
    }

    let png = gray_png(4, 4, 0x80);
    let brush = zip_with_oversize_plist("Brush.archive", &[("Shape.png", &png)]);
    let peak = growth(|| {
        let set = preview_brush(&brush, opts).expect("brush reads");
        assert!(
            matches!(
                &set.entries[0].tip,
                TipPreview::Unavailable(UnavailableReason::Corrupt(msg))
                    if msg.contains("declared size")
            ),
            "{:?}",
            set.entries[0].tip
        );
    });
    println!("Brush.archive: {peak} bytes");
    assert!(
        peak <= MIB,
        "a {PLIST_BYTES}-byte Brush.archive grew the heap by {peak} bytes"
    );
}
