mod common;

use brushkit_preview::procreate::Zip;
use common::{brush_archive, corpus_files, real_4x4_png, zip_with};
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
