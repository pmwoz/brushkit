#![allow(dead_code)]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

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

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// An 8-bit grayscale PNG written by hand, with one stored DEFLATE block. The
/// fuzz seeds embed these bytes, so they must not change when the png encoder
/// does.
pub fn gray_png(width: u32, height: u32, fill: u8) -> Vec<u8> {
    let mut scanlines = Vec::new();
    for _ in 0..height {
        scanlines.push(0);
        scanlines.extend(std::iter::repeat_n(fill, width as usize));
    }
    let len = u16::try_from(scanlines.len()).expect("scanlines fit one stored block");
    let mut zlib = vec![0x78, 0x01, 1];
    zlib.extend_from_slice(&len.to_le_bytes());
    zlib.extend_from_slice(&(!len).to_le_bytes());
    zlib.extend_from_slice(&scanlines);
    zlib.extend_from_slice(&adler32(&scanlines).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(b"IHDR", ihdr), (b"IDAT", zlib), (b"IEND", Vec::new())] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(&data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }
    png
}

pub fn real_4x4_png() -> Vec<u8> {
    gray_png(4, 4, 128)
}

pub fn dimension_bomb_png(w: u32, h: u32) -> Vec<u8> {
    let mut png = real_4x4_png();
    // IHDR is the first chunk: 8-byte signature, 4-byte length, 4-byte type,
    // then width and height as the first 8 bytes of its 13-byte payload.
    assert_eq!(
        &png[12..16],
        b"IHDR",
        "gray_png must write IHDR first for these offsets to be right"
    );
    png[16..20].copy_from_slice(&w.to_be_bytes());
    png[20..24].copy_from_slice(&h.to_be_bytes());
    let crc = crc32(&png[12..29]); // chunk type + the 13 IHDR data bytes
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

/// A [`dimension_bomb_png`] whose IHDR declares 8-bit RGBA, so each pixel
/// decodes to four bytes.
pub fn rgba_dimension_bomb_png(w: u32, h: u32) -> Vec<u8> {
    let mut png = dimension_bomb_png(w, h);
    png[25] = 6; // IHDR color type
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

/// A baseline JPEG written by hand that declares `width` x `height` with
/// `components` channels (1 is grayscale, 3 is YCbCr) and holds one 8x8 block
/// of scan data per channel. At 8x8 or smaller it decodes to solid mid-gray.
/// Each Huffman table holds one 1-bit code: every block is a zero DC and an
/// immediate end-of-block.
pub fn baseline_jpeg(width: u32, height: u32, components: u8) -> Vec<u8> {
    let width = u16::try_from(width).expect("JPEG width is 16-bit");
    let height = u16::try_from(height).expect("JPEG height is 16-bit");
    let mut jpeg = vec![0xFF, 0xD8];
    jpeg.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, 0x00]);
    jpeg.extend_from_slice(&[1; 64]);
    jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 8 + 3 * components, 8]);
    jpeg.extend_from_slice(&height.to_be_bytes());
    jpeg.extend_from_slice(&width.to_be_bytes());
    jpeg.push(components);
    for id in 1..=components {
        jpeg.extend_from_slice(&[id, 0x11, 0]);
    }
    for class in [0x00, 0x10] {
        jpeg.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x14, class, 1]);
        jpeg.extend_from_slice(&[0; 15]);
        jpeg.push(0);
    }
    jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 6 + 2 * components, components]);
    for id in 1..=components {
        jpeg.extend_from_slice(&[id, 0x00]);
    }
    jpeg.extend_from_slice(&[0, 0x3F, 0]);
    // Two zero bits per block, padded with one bits to the byte.
    jpeg.extend_from_slice(&[0xFF >> (2 * components), 0xFF, 0xD9]);
    jpeg
}

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

pub fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::write::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (path, bytes) in entries {
        zw.start_file(*path, opts).expect("start_file");
        zw.write_all(bytes).expect("write entry");
    }
    zw.finish().expect("finish zip").into_inner()
}

/// One sampled tip for [`samp_abr`]: `width` x `height` raw 8-bit pixels of
/// `fill`. A `corrupt` tip declares RLE instead, so its first row byte count
/// (two `fill` bytes) overruns the payload and decoding fails.
pub struct SampTip {
    pub width: u32,
    pub height: u32,
    pub fill: u8,
    pub corrupt: bool,
}

/// A v6 `.abr` whose single `samp` block holds `tips` in order. The entries
/// carry no uuid, so the parser lists the brushes in reverse block order.
pub fn samp_abr(tips: &[SampTip]) -> Vec<u8> {
    let mut payload = Vec::new();
    for tip in tips {
        let mut entry = Vec::new();
        entry.extend_from_slice(&0u32.to_be_bytes());
        for bound in [0, 0, tip.height as i32, tip.width as i32] {
            entry.extend_from_slice(&bound.to_be_bytes());
        }
        entry.extend_from_slice(&8u16.to_be_bytes());
        entry.push(u8::from(tip.corrupt));
        entry.extend(std::iter::repeat_n(
            tip.fill,
            (tip.width * tip.height) as usize,
        ));
        payload.extend_from_slice(&(entry.len() as u32).to_be_bytes());
        payload.extend_from_slice(&entry);
        payload.resize(payload.len().next_multiple_of(4), 0);
    }

    let mut file = Vec::new();
    file.extend_from_slice(&6u16.to_be_bytes());
    file.extend_from_slice(&2u16.to_be_bytes());
    file.extend_from_slice(b"8BIMsamp");
    file.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    file.extend_from_slice(&payload);
    file
}

/// A v2 `.abr` whose entries hold `tips` in order, RLE-compressed with one
/// repeat run per row, so `width` is at most 128. A `corrupt` tip carries only
/// its row byte counts and fails to decode. The parser lists the brushes in
/// reverse entry order.
pub fn legacy_abr(tips: &[SampTip]) -> Vec<u8> {
    let mut file = Vec::new();
    file.extend_from_slice(&2u16.to_be_bytes());
    file.extend_from_slice(&(tips.len() as u16).to_be_bytes());
    for tip in tips {
        let mut entry = Vec::new();
        // Misc, spacing, an empty name, anti-aliasing and the i16 bounds.
        entry.extend_from_slice(&[0; 4 + 2 + 4 + 1 + 8]);
        for bound in [0, 0, tip.height as i32, tip.width as i32] {
            entry.extend_from_slice(&bound.to_be_bytes());
        }
        entry.extend_from_slice(&8u16.to_be_bytes());
        entry.push(1);
        for _ in 0..tip.height {
            entry.extend_from_slice(&2u16.to_be_bytes());
        }
        if !tip.corrupt {
            for _ in 0..tip.height {
                entry.extend_from_slice(&[(1 - tip.width as i32) as u8, tip.fill]);
            }
        }
        file.extend_from_slice(&2u16.to_be_bytes());
        file.extend_from_slice(&(entry.len() as u32).to_be_bytes());
        file.extend_from_slice(&entry);
    }
    file
}

/// Every `.abr`, `.brush` and `.brushset` file under `directory`, recursively.
pub fn corpus_files(directory: &Path, files: &mut Vec<PathBuf>) {
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
