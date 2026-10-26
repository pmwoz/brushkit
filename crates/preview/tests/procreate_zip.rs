mod common;

use brushkit_preview::procreate::{Zip, ZipReadError, MAX_ENTRY_BYTES, MAX_PLIST_BYTES};
use common::{
    brush_archive, corpus_files, crc32, declare_size, gray_png_with_scanlines, real_4x4_png,
    zip_with,
};
use std::io::{Cursor, Write};
use std::path::Path;

#[test]
fn a_listed_cp437_name_reads_its_entry() {
    let archive = brush_archive("Soft");
    let png = real_4x4_png();
    let bytes = zip_with(&[
        (&b"P\x81nsel/Brush.archive"[..], &archive),
        (&b"P\x81nsel/Shape.png"[..], &png),
    ]);

    let mut zip = Zip::open(&bytes).expect("zip opens");
    assert_eq!(zip.names(), ["Pünsel/Brush.archive", "Pünsel/Shape.png"]);
    assert!(zip.contains("Pünsel/Shape.png"));
    assert_eq!(zip.read_plist("Pünsel/Brush.archive"), Ok(archive));
    assert_eq!(zip.read_entry("Pünsel/Shape.png"), Ok(png));
}

#[test]
fn an_entry_over_the_caller_limit_fails_on_its_declared_size() {
    let png = real_4x4_png();
    let size = png.len();
    let bytes = zip_with(&[("Shape.png", &png[..])]);

    let mut zip = Zip::open(&bytes).expect("zip opens");
    assert_eq!(zip.read_entry_within("Shape.png", size), Ok(png.clone()));
    assert_eq!(zip.read_entry_within("Shape.png", usize::MAX), Ok(png));
    let error = zip.read_entry_within("Shape.png", size - 1).unwrap_err();
    assert_eq!(
        error,
        ZipReadError::OverLimit {
            name: "Shape.png".into(),
            declared: size as u64,
            limit: size - 1,
        }
    );
    assert_eq!(
        error.to_string(),
        format!("Shape.png: declared size {size} exceeds limit")
    );
}

#[test]
fn a_caller_limit_cannot_raise_the_entry_ceiling() {
    let over = MAX_ENTRY_BYTES as u32 + 1;
    let bytes = declare_size(zip_with(&[("Shape.png", &real_4x4_png()[..])]), over);

    let mut zip = Zip::open(&bytes).expect("zip opens");
    assert_eq!(
        zip.read_entry_within("Shape.png", usize::MAX),
        Err(ZipReadError::OverLimit {
            name: "Shape.png".into(),
            declared: over.into(),
            limit: MAX_ENTRY_BYTES,
        })
    );
}

#[test]
fn a_plist_over_its_ceiling_reports_the_plist_limit() {
    let over = MAX_PLIST_BYTES as u32 + 1;
    let bytes = declare_size(
        zip_with(&[("Brush.archive", &brush_archive("Big")[..])]),
        over,
    );

    let mut zip = Zip::open(&bytes).expect("zip opens");
    assert_eq!(
        zip.read_plist("Brush.archive"),
        Err(ZipReadError::OverLimit {
            name: "Brush.archive".into(),
            declared: over.into(),
            limit: MAX_PLIST_BYTES,
        })
    );
}

#[test]
fn an_entry_shorter_than_its_declared_size_fails() {
    let png = real_4x4_png();
    let declared = png.len() as u32 + 1;
    let mut deflated = zip::write::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    deflated.start_file("Shape.png", opts).expect("start entry");
    deflated.write_all(&png).expect("entry bytes");
    let deflated = deflated.finish().expect("finish zip").into_inner();

    for (method, bytes) in [
        ("stored", zip_with(&[("Shape.png", &png[..])])),
        ("deflated", deflated),
    ] {
        let bytes = declare_size(bytes, declared);
        let mut zip = Zip::open(&bytes).expect("zip opens");
        let error = Err(ZipReadError::Failed(format!(
            "Shape.png: entry is shorter than its declared size {declared}"
        )));
        assert_eq!(zip.read_entry("Shape.png"), error, "{method}");
        assert_eq!(
            zip.read_entry_within("Shape.png", usize::MAX),
            error,
            "{method}"
        );
    }
}

#[test]
fn two_end_of_central_directory_records_are_rejected() {
    let mut bytes = zip_with(&[("Brush.archive", &brush_archive("Solo")[..])]);
    assert!(Zip::open(&bytes).is_ok());
    let end = bytes[bytes.len() - 22..].to_vec();
    bytes.extend_from_slice(&end);

    let error = Zip::open(&bytes).unwrap_err();
    assert!(
        error.contains("more than one end of central directory record"),
        "{error}"
    );
}

#[test]
fn the_end_record_signature_in_entry_data_opens() {
    let png = gray_png_with_scanlines(4, 1, b"\0PK\x05\x06");
    let note = b"bytes PK\x05\x06 inside a member".to_vec();
    let mut empty_zip = b"PK\x05\x06".to_vec();
    empty_zip.resize(22, 0);
    let mut zip64_locator_past_itself = b"PK\x06\x07\0\0\0\0".to_vec();
    zip64_locator_past_itself.extend_from_slice(&u64::MAX.to_le_bytes());
    zip64_locator_past_itself.extend_from_slice(&1u32.to_le_bytes());
    zip64_locator_past_itself.extend_from_slice(b"PK\x05\x06\0\0\0\0\xff\xff\xff\xff");
    zip64_locator_past_itself.resize(42, 0);
    let mut zip64_locator_at_a_local_header = zip64_locator_past_itself.clone();
    zip64_locator_at_a_local_header[8..16].copy_from_slice(&0u64.to_le_bytes());
    for (name, data) in [
        ("Shape.png", png),
        ("Signature/note.bin", note),
        ("Signature/empty.zip", empty_zip),
        ("Signature/zip64.bin", zip64_locator_past_itself),
        ("Signature/zip64_at_0.bin", zip64_locator_at_a_local_header),
    ] {
        let archive = brush_archive("Solo");
        let bytes = zip_with(&[("Brush.archive", &archive[..]), (name, &data)]);

        let mut zip = Zip::open(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(zip.read_entry(name), Ok(data), "{name}");
    }
}

#[test]
fn an_end_record_in_entry_data_that_passes_zip_checks_is_rejected() {
    // zip reads a directory for the inner end record if the real one fails, so
    // the file is rejected although the real one would open.
    let archive = brush_archive("Solo");
    let mut note = b"PK\x01\x02".to_vec();
    note.resize(26, 0);
    let placeholder = zip_with(&[("Brush.archive", &archive[..]), ("note.bin", &note)]);
    let header = placeholder
        .windows(4)
        .position(|w| w == b"PK\x01\x02")
        .unwrap();
    note.truncate(4);
    note.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0\x2e\0\0\0");
    note.extend_from_slice(&u32::try_from(header).unwrap().to_le_bytes());
    note.extend_from_slice(&[0, 0]);
    let bytes = zip_with(&[("Brush.archive", &archive[..]), ("note.bin", &note)]);

    let error = Zip::open(&bytes).unwrap_err();
    assert!(
        error.contains("more than one end of central directory record"),
        "{error}"
    );
}

#[test]
fn a_utf8_name_wins_over_an_equal_cp437_name() {
    let bytes = zip_with(&[
        (&b"P\x81nsel/Shape.png"[..], &b"cp437"[..]),
        ("Pünsel/Shape.png".as_bytes(), &b"utf8"[..]),
    ]);

    let mut zip = Zip::open(&bytes).expect("zip opens");
    assert_eq!(zip.names(), ["Pünsel/Shape.png", "Pünsel/Shape.png"]);
    assert_eq!(zip.read_entry("Pünsel/Shape.png"), Ok(b"utf8".to_vec()));
    assert_eq!(zip.conflicting_names(), ["Pünsel/Shape.png"]);
}

#[test]
fn a_raw_name_listed_with_a_different_crc_or_size_is_reported() {
    let archive = brush_archive("Twice");
    let png = real_4x4_png();
    let mut flipped = png.clone();
    *flipped.last_mut().unwrap() ^= 1;
    let size = png.len() as u32 + 1;
    for (case, bytes) in [
        (
            "crc",
            zip_with(&[
                ("Brush.archive", &archive[..]),
                ("Shape.png", &png),
                ("Shape.png", &flipped),
            ]),
        ),
        (
            "size",
            declare_size(
                zip_with(&[("Shape.png", &png[..]), ("Shape.png", &png)]),
                size,
            ),
        ),
    ] {
        let zip = Zip::open(&bytes).unwrap_or_else(|error| panic!("{case}: {error}"));
        assert_eq!(zip.conflicting_names(), ["Shape.png"], "{case}");
    }
}

#[test]
fn a_name_whose_entries_match_is_not_reported() {
    let png = real_4x4_png();
    let bytes = zip_with(&[
        ("Brush/.empty", &b""[..]),
        ("Brush/Shape.png", &png),
        ("Brush/.empty", &b""[..]),
        ("Brush/Shape.png", &png),
    ]);

    let zip = Zip::open(&bytes).expect("zip opens");
    assert!(zip.conflicting_names().is_empty());
}

fn first_header_extra(mut zip: Vec<u8>, extra: &[u8]) -> Vec<u8> {
    let central = zip.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    let name_len = usize::from(u16::from_le_bytes([zip[central + 28], zip[central + 29]]));
    assert_eq!(
        zip[central + 30..central + 32],
        [0, 0],
        "no extra field yet"
    );
    let extra_len = u16::try_from(extra.len()).unwrap();
    zip[central + 30..central + 32].copy_from_slice(&extra_len.to_le_bytes());
    let at = central + 46 + name_len;
    zip.splice(at..at, extra.iter().copied());
    let end = zip.len() - 22;
    let directory =
        u32::from_le_bytes(zip[end + 12..end + 16].try_into().unwrap()) + u32::from(extra_len);
    zip[end + 12..end + 16].copy_from_slice(&directory.to_le_bytes());
    zip
}

/// Moves both sizes of the zip's first central header into a zip64 extra
/// field, with `size` as the uncompressed size there.
fn zip64_sizes(mut zip: Vec<u8>, size: u64) -> Vec<u8> {
    let central = zip.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    let compressed = u32::from_le_bytes(zip[central + 20..central + 24].try_into().unwrap());
    zip[central + 20..central + 28].fill(0xff);
    let mut extra = vec![1, 0, 16, 0];
    extra.extend_from_slice(&size.to_le_bytes());
    extra.extend_from_slice(&u64::from(compressed).to_le_bytes());
    first_header_extra(zip, &extra)
}

#[test]
fn a_dropped_entry_is_compared_by_its_zip64_size() {
    let png = real_4x4_png();
    let size = png.len() as u64;
    let twice = zip_with(&[("Shape.png", &png[..]), ("Shape.png", &png)]);
    for (declared, expected) in [(size, &[][..]), (size + 1, &["Shape.png"][..])] {
        let bytes = zip64_sizes(twice.clone(), declared);
        let zip = Zip::open(&bytes).unwrap_or_else(|error| panic!("{declared}: {error}"));
        assert_eq!(zip.conflicting_names(), expected, "{declared}");
    }
}

#[test]
fn a_dropped_entry_is_compared_by_its_unicode_path_name() {
    let png = real_4x4_png();
    let mut flipped = png.clone();
    *flipped.last_mut().unwrap() ^= 1;
    let mut unicode_path = vec![0x75, 0x70, 14, 0, 1];
    unicode_path.extend_from_slice(&crc32(b"Other.png").to_le_bytes());
    unicode_path.extend_from_slice(b"Shape.png");
    let bytes = first_header_extra(
        zip_with(&[("Other.png", &png[..]), ("Shape.png", &flipped)]),
        &unicode_path,
    );

    let zip = Zip::open(&bytes).expect("zip opens");
    assert_eq!(zip.names(), ["Shape.png"]);
    assert_eq!(zip.conflicting_names(), ["Shape.png"]);
}

#[test]
fn real_procreate_names_find_their_entries() {
    let Some(root) = std::env::var_os("BRUSHKIT_CORPUS_DIR") else {
        return;
    };
    let mut files = Vec::new();
    corpus_files(Path::new(&root), &mut files);
    let (mut opened, mut read, mut repeated) = (0, 0, 0);
    for path in files {
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("brush") && !ext.eq_ignore_ascii_case("brushset") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let mut zip =
            Zip::open(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(zip.conflicting_names().is_empty(), "{}", path.display());
        let end = bytes.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        let declared = usize::from(u16::from_le_bytes([bytes[end + 10], bytes[end + 11]]));
        repeated += declared - zip.names().len();
        opened += 1;
        for name in zip.names().into_iter().filter(|name| !name.ends_with('/')) {
            assert!(zip.contains(&name), "{}: {name}", path.display());
            zip.read_entry(&name)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            read += 1;
        }
    }
    println!(
        "{opened} Procreate files, {read} entries read by listed name, {repeated} repeated raw names"
    );
    assert!(opened > 0, "corpus must contain Procreate files");
    assert!(repeated > 0, "corpus must repeat a raw name");
}
