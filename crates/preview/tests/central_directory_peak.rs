mod counting_alloc;

use brushkit_preview::procreate::MAX_CENTRAL_DIRECTORY_BYTES;
use brushkit_preview::{preview_brush, PreviewOptions};
use counting_alloc::{live, peak, reset_peak};

const MIB: usize = 1 << 20;
/// Empty extra fields per central header. zip 9 keeps a parsed record for
/// every one in a vector that doubles as it grows, and 4,097 leaves it nearly
/// half empty, which costs about 20 times the header bytes.
const EXTRA_FIELDS: u16 = 4_097;
const HEADER_BYTES: usize = 46 + 4 * EXTRA_FIELDS as usize;
const ENTRIES: usize = MAX_CENTRAL_DIRECTORY_BYTES / HEADER_BYTES + 1;

fn dense_directory() -> Vec<u8> {
    let mut header = vec![0u8; 46];
    header[..4].copy_from_slice(b"PK\x01\x02");
    header[4..6].copy_from_slice(&20u16.to_le_bytes()); // Made by version 2.0.
    header[6..8].copy_from_slice(&20u16.to_le_bytes()); // Needs version 2.0.
    header[30..32].copy_from_slice(&(4 * EXTRA_FIELDS).to_le_bytes()); // Extra field length.
    header.extend(b"\x22\x22\0\0".repeat(EXTRA_FIELDS.into()));
    header.repeat(ENTRIES)
}

fn end_record(entries: u16, total_entries: u16, size: u32, offset: u32) -> Vec<u8> {
    let mut record = b"PK\x05\x06\0\0\0\0".to_vec(); // This disk and the directory's disk.
    record.extend(entries.to_le_bytes());
    record.extend(total_entries.to_le_bytes());
    record.extend(size.to_le_bytes());
    record.extend(offset.to_le_bytes());
    record.extend(0u16.to_le_bytes()); // Comment length.
    record
}

/// A zip64 end record with `extensible` bytes of extensible data right after a
/// directory of `size` bytes, and the locator that points to it.
fn zip64_end(entries: u64, size: u64, offset: u64, extensible: usize) -> Vec<u8> {
    let mut record = b"PK\x06\x06".to_vec();
    record.extend((44 + extensible as u64).to_le_bytes()); // Size of the rest of the record.
    record.extend([45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // Versions and disks.
    record.extend(entries.to_le_bytes());
    record.extend(entries.to_le_bytes());
    record.extend(size.to_le_bytes());
    record.extend(offset.to_le_bytes());
    record.resize(record.len() + extensible, 0);
    record.extend(b"PK\x06\x07\0\0\0\0"); // Locator and the zip64 record's disk.
    record.extend(size.to_le_bytes());
    record.extend(1u32.to_le_bytes()); // Total disks.
    record
}

#[test]
fn central_directory_over_the_limit_is_rejected_in_little_memory() {
    let directory = dense_directory();
    let entries = u16::try_from(ENTRIES).unwrap();
    let size = u32::try_from(directory.len()).unwrap();
    let (n, size64) = (u64::from(entries), u64::from(size));
    let archives = [
        (
            "zip32",
            [directory.clone(), end_record(entries, entries, size, 0)].concat(),
        ),
        (
            "zip64, zip32 offset near the end",
            [
                directory.clone(),
                zip64_end(n, size64, 0, 0),
                end_record(entries, u16::MAX, size, size),
            ]
            .concat(),
        ),
        (
            "zip32, zip64 offset near the end",
            [
                directory,
                zip64_end(n, size64, size64, 0),
                end_record(entries, entries, size, 0),
            ]
            .concat(),
        ),
        (
            "zip64 end record with extensible data",
            [
                zip64_end(
                    0,
                    0,
                    56 + MAX_CENTRAL_DIRECTORY_BYTES as u64,
                    MAX_CENTRAL_DIRECTORY_BYTES,
                ),
                end_record(u16::MAX, u16::MAX, u32::MAX, u32::MAX),
            ]
            .concat(),
        ),
    ];
    for (shape, zip) in archives {
        let before = live();
        reset_peak();
        let err = preview_brush(&zip, PreviewOptions { max_cell: 8 }).expect_err(shape);
        let peak = peak() - before;
        println!("{shape}, {} bytes: {peak} bytes", zip.len());
        assert!(err.0.starts_with("failed to open zip: "), "{shape}: {err}");
        assert!(
            peak <= MIB,
            "{shape}: a {}-byte zip grew the heap by {peak} bytes",
            zip.len()
        );
    }
}
