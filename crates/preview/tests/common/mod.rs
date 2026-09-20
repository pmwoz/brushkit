//! Input builders for the preview integration tests. Every input is built
//! here; nothing reads a real brush file.

#![allow(dead_code)]

use std::io::{Cursor, Write};

/// A minimal NSKeyedArchiver `Brush.archive` whose settings dictionary
/// (`$objects[1]`) names the brush. Binary plist, like Procreate writes.
pub fn brush_archive(name: &str) -> Vec<u8> {
    use plist::{Dictionary, Uid, Value};
    let mut settings = Dictionary::new();
    settings.insert("name".into(), Value::Uid(Uid::new(2)));
    let mut top = Dictionary::new();
    top.insert("root".into(), Value::Uid(Uid::new(1)));
    let mut root = Dictionary::new();
    root.insert("$version".into(), Value::Integer(100_000.into()));
    root.insert("$archiver".into(), Value::String("NSKeyedArchiver".into()));
    root.insert(
        "$objects".into(),
        Value::Array(vec![
            Value::String("$null".into()),
            Value::Dictionary(settings),
            Value::String(name.into()),
        ]),
    );
    root.insert("$top".into(), Value::Dictionary(top));
    let mut out = Vec::new();
    Value::Dictionary(root)
        .to_writer_binary(&mut out)
        .expect("binary plist");
    out
}

/// A `brushset.plist` naming the set and listing its members in order.
pub fn brushset_plist(name: &str, uuids: &[&str]) -> Vec<u8> {
    use plist::{Dictionary, Value};
    let mut dict = Dictionary::new();
    dict.insert("name".into(), Value::String(name.into()));
    dict.insert(
        "brushes".into(),
        Value::Array(uuids.iter().map(|u| Value::String((*u).into())).collect()),
    );
    let mut out = Vec::new();
    Value::Dictionary(dict)
        .to_writer_binary(&mut out)
        .expect("binary plist");
    out
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// An 8-bit grayscale PNG of `fill`.
pub fn gray_png(width: u32, height: u32, fill: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(&vec![fill; (width * height) as usize])
        .expect("png image data");
    w.finish().expect("png finish");
    out
}

pub fn real_4x4_png() -> Vec<u8> {
    gray_png(4, 4, 128)
}

/// A real 4x4 PNG whose IHDR claims `w` by `h`, so a reader that trusts the
/// header would allocate before it decodes.
pub fn dimension_bomb_png(w: u32, h: u32) -> Vec<u8> {
    let mut png = real_4x4_png();
    // IHDR is always the first chunk: 8-byte signature, 4-byte length, 4-byte
    // type, then width and height as the first 8 bytes of its 13-byte payload.
    assert_eq!(
        &png[12..16],
        b"IHDR",
        "the png crate must emit IHDR first for these offsets to be right"
    );
    png[16..20].copy_from_slice(&w.to_be_bytes());
    png[20..24].copy_from_slice(&h.to_be_bytes());
    let crc = crc32(&png[12..29]); // chunk type + the 13 IHDR data bytes
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

/// An XML plist of 10 000 nested `<array>` opens: a reader that builds the
/// tree before checking depth overflows its stack on the recursive `Drop`.
pub fn depth_bomb_plist_xml() -> Vec<u8> {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n",
    );
    s.push_str(&"<array>".repeat(10_000));
    s.into_bytes()
}

/// A zip holding `entries` in the given order.
pub fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::write::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (path, bytes) in entries {
        zw.start_file(*path, opts).expect("start_file");
        zw.write_all(bytes).expect("write entry");
    }
    zw.finish().expect("finish zip").into_inner()
}
