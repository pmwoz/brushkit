#![allow(dead_code)]

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

/// IHDR color types.
const GRAY: u8 = 0;
const RGBA: u8 = 6;

/// A PNG written by hand whose IHDR declares `width` x `height` at `bit_depth`
/// and `color_type`, and whose IDAT holds `scanlines` (filter bytes included)
/// in one stored DEFLATE block. The declared size need not match the rows. The
/// fuzz seeds embed these bytes, so they must not change when the png encoder
/// does.
fn png_file(width: u32, height: u32, bit_depth: u8, color_type: u8, scanlines: &[u8]) -> Vec<u8> {
    let len = u16::try_from(scanlines.len()).expect("scanlines fit one stored block");
    let mut zlib = vec![0x78, 0x01, 1];
    zlib.extend_from_slice(&len.to_le_bytes());
    zlib.extend_from_slice(&(!len).to_le_bytes());
    zlib.extend_from_slice(scanlines);
    zlib.extend_from_slice(&adler32(scanlines).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[bit_depth, color_type, 0, 0, 0]);

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

/// `height` unfiltered rows of `width` 8-bit gray pixels of `fill`.
fn gray_scanlines(width: u32, height: u32, fill: u8) -> Vec<u8> {
    let mut scanlines = Vec::new();
    for _ in 0..height {
        scanlines.push(0);
        scanlines.extend(std::iter::repeat_n(fill, width as usize));
    }
    scanlines
}

/// An 8-bit grayscale PNG of `width` x `height` pixels of `fill`.
pub fn gray_png(width: u32, height: u32, fill: u8) -> Vec<u8> {
    png_file(width, height, 8, GRAY, &gray_scanlines(width, height, fill))
}

pub fn real_4x4_png() -> Vec<u8> {
    gray_png(4, 4, 128)
}

/// A PNG whose IHDR declares `w` x `h` at `bit_depth` and `color_type` but
/// whose IDAT holds only 4 x 4 gray pixels.
fn bomb_png(w: u32, h: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
    png_file(w, h, bit_depth, color_type, &gray_scanlines(4, 4, 128))
}

pub fn dimension_bomb_png(w: u32, h: u32) -> Vec<u8> {
    bomb_png(w, h, 8, GRAY)
}

/// A [`dimension_bomb_png`] whose IHDR declares RGBA at `bit_depth` 8 or 16,
/// so each pixel decodes to four or eight bytes.
pub fn rgba_dimension_bomb_png(w: u32, h: u32, bit_depth: u8) -> Vec<u8> {
    bomb_png(w, h, bit_depth, RGBA)
}

/// A baseline JPEG written by hand that declares `width` x `height` with
/// `components` channels (1 is grayscale, 3 is YCbCr) and holds one 8x8 block
/// of scan data per channel. At 8x8 or smaller it decodes to solid mid-gray.
/// Each Huffman table holds one 1-bit code: every block is a zero DC and an
/// immediate end-of-block.
pub fn baseline_jpeg(width: u32, height: u32, components: u8) -> Vec<u8> {
    hand_written_jpeg(
        0xC0,
        width,
        height,
        &vec![0x11; usize::from(components)],
        components,
    )
}

/// `baseline_jpeg` whose one scan holds only the first `scan` of its
/// `components`, as a non-interleaved JPEG's first scan does.
pub fn partial_scan_jpeg(width: u32, height: u32, components: u8, scan: u8) -> Vec<u8> {
    hand_written_jpeg(
        0xC0,
        width,
        height,
        &vec![0x11; usize::from(components)],
        scan,
    )
}

/// `baseline_jpeg` as a progressive JPEG with one component per `sampling`
/// byte (0xHV): one DC scan whose blocks are each a zero DC, so it decodes to
/// solid mid-gray at any size.
pub fn progressive_jpeg(width: u32, height: u32, sampling: &[u8]) -> Vec<u8> {
    let components = u8::try_from(sampling.len()).expect("a few components");
    hand_written_jpeg(0xC2, width, height, sampling, components)
}

/// A JPEG whose one scan holds the first `scan` components of the frame.
fn hand_written_jpeg(sof: u8, width: u32, height: u32, sampling: &[u8], scan: u8) -> Vec<u8> {
    let components = u8::try_from(sampling.len()).expect("a few components");
    assert!(matches!(components, 1 | 3), "grayscale or YCbCr only");
    let progressive = sof == 0xC2;
    let width = u16::try_from(width).expect("JPEG width is 16-bit");
    let height = u16::try_from(height).expect("JPEG height is 16-bit");
    let mut jpeg = vec![0xFF, 0xD8];
    jpeg.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, 0x00]);
    jpeg.extend_from_slice(&[1; 64]);
    jpeg.extend_from_slice(&[0xFF, sof, 0x00, 8 + 3 * components, 8]);
    jpeg.extend_from_slice(&height.to_be_bytes());
    jpeg.extend_from_slice(&width.to_be_bytes());
    jpeg.push(components);
    for (id, &factors) in (1..).zip(sampling) {
        jpeg.extend_from_slice(&[id, factors, 0]);
    }
    for class in [0x00, 0x10] {
        jpeg.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x14, class, 1]);
        jpeg.extend_from_slice(&[0; 15]);
        jpeg.push(0);
    }
    jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 6 + 2 * scan, scan]);
    for id in 1..=scan {
        jpeg.extend_from_slice(&[id, 0x00]);
    }
    let spectral_end = if progressive { 0 } else { 0x3F };
    jpeg.extend_from_slice(&[0, spectral_end, 0]);
    // The first MCU: a zero bit per DC and, in a baseline scan, one per
    // end-of-block, padded with one bits to the byte.
    let bits_per_block = if progressive { 1 } else { 2 };
    let blocks: u32 = sampling[..usize::from(scan)]
        .iter()
        .map(|&factors| u32::from(factors >> 4) * u32::from(factors & 0xF))
        .sum();
    let bits = bits_per_block * blocks;
    let bytes = bits.div_ceil(8);
    jpeg.extend(std::iter::repeat_n(0, bytes as usize - 1));
    jpeg.push((0xFFu16 >> (bits - 8 * (bytes - 1))) as u8);
    jpeg.extend_from_slice(&[0xFF, 0xD9]);
    jpeg
}

/// An XML plist that opens `depth` nested arrays and never closes them.
pub fn depth_bomb_plist_xml(depth: usize) -> Vec<u8> {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n",
    );
    s.push_str(&"<array>".repeat(depth));
    s.into_bytes()
}

/// A zip of `entries` in order, written by hand with stored entries and fixed
/// header fields. The fuzz seeds embed these bytes, so they must not change
/// when the zip crate or its deflate backend does.
pub fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = Vec::new();
    let mut directory = Vec::new();
    for (path, bytes) in entries {
        let offset = u32::try_from(zip.len()).expect("zip fits in 4 GiB");
        let size = u32::try_from(bytes.len()).expect("entry fits in 4 GiB");
        // The fields both headers share: version needed 1.0, no flags, stored,
        // dated 1980-01-01 00:00, then the CRC, both sizes, the name length and
        // no extra field.
        let mut fields = Vec::new();
        for half in [10u16, 0, 0, 0, 0x21] {
            fields.extend_from_slice(&half.to_le_bytes());
        }
        for word in [crc32(bytes), size, size] {
            fields.extend_from_slice(&word.to_le_bytes());
        }
        fields.extend_from_slice(&(path.len() as u16).to_le_bytes());
        fields.extend_from_slice(&0u16.to_le_bytes());

        zip.extend_from_slice(b"PK\x03\x04");
        zip.extend_from_slice(&fields);
        zip.extend_from_slice(path.as_bytes());
        zip.extend_from_slice(bytes);

        directory.extend_from_slice(b"PK\x01\x02");
        directory.extend_from_slice(&10u16.to_le_bytes()); // Made by MS-DOS, version 1.0.
        directory.extend_from_slice(&fields);
        // No comment, disk 0, no internal or external attributes.
        directory.extend_from_slice(&[0; 2 + 2 + 2 + 4]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(path.as_bytes());
    }
    let count = u16::try_from(entries.len()).expect("entry count fits in u16");
    let directory_offset = zip.len() as u32;
    let directory_size = directory.len() as u32;
    zip.extend_from_slice(&directory);
    zip.extend_from_slice(b"PK\x05\x06");
    zip.extend_from_slice(&[0; 4]); // This disk and the directory's disk.
    zip.extend_from_slice(&count.to_le_bytes());
    zip.extend_from_slice(&count.to_le_bytes());
    zip.extend_from_slice(&directory_size.to_le_bytes());
    zip.extend_from_slice(&directory_offset.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // No comment.
    zip
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
