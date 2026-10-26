mod common;

use brushkit_preview::procreate::{Zip, ZipReadError, MAX_ENTRY_BYTES};
use common::{
    brush_archive, corpus_files, declare_size, gray_png_with_scanlines, real_4x4_png, zip_with,
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
}

#[test]
fn real_procreate_names_find_their_entries() {
    let Some(root) = std::env::var_os("BRUSHKIT_CORPUS_DIR") else {
        return;
    };
    let mut files = Vec::new();
    corpus_files(Path::new(&root), &mut files);
    let (mut opened, mut read) = (0, 0);
    for path in files {
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("brush") && !ext.eq_ignore_ascii_case("brushset") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let mut zip =
            Zip::open(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        opened += 1;
        for name in zip.names().into_iter().filter(|name| !name.ends_with('/')) {
            assert!(zip.contains(&name), "{}: {name}", path.display());
            zip.read_entry(&name)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            read += 1;
        }
    }
    println!("{opened} Procreate files, {read} entries read by listed name");
    assert!(opened > 0, "corpus must contain Procreate files");
}
