mod counting_alloc;

use brushkit_preview::{preview_brush, PreviewOptions};
use counting_alloc::{live, peak, reset_peak};

const MIB: usize = 1 << 20;

/// Every header carries an NTFS extra field of length 0, which zip rejects
/// only after it has read all `entries` headers of a directory.
fn many_directory_ends(records: usize, entries: u16) -> Vec<u8> {
    let mut header = [0u8; 50];
    header[..4].copy_from_slice(b"PK\x01\x02");
    header[4..6].copy_from_slice(&20u16.to_le_bytes()); // Made by version 2.0.
    header[6..8].copy_from_slice(&20u16.to_le_bytes()); // Needs version 2.0.
    header[30..32].copy_from_slice(&4u16.to_le_bytes()); // Extra field length.
    header[46..48].copy_from_slice(&0x000au16.to_le_bytes()); // NTFS, no data.
    let mut zip = header.repeat(records + usize::from(entries));
    let size = u32::from(entries) * header.len() as u32;
    for k in 0..records {
        let offset = u32::try_from(k * header.len()).unwrap();
        zip.extend_from_slice(b"PK\x05\x06");
        zip.extend_from_slice(&[0; 4]); // This disk and the directory's disk.
        zip.extend_from_slice(&entries.to_le_bytes());
        zip.extend_from_slice(&entries.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&offset.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes()); // Comment length.
    }
    zip
}

#[test]
fn many_end_of_central_directory_records_are_rejected_in_little_memory() {
    let zip = many_directory_ends(250, 12_500);
    let before = live();
    reset_peak();
    let err = preview_brush(&zip, PreviewOptions { max_cell: 8 }).expect_err("ambiguous zip");
    let peak = peak() - before;
    println!("{} bytes: {peak} bytes", zip.len());
    assert!(err.0.starts_with("failed to open zip: "), "{err}");
    assert!(
        peak <= MIB,
        "a {}-byte zip grew the heap by {peak} bytes",
        zip.len()
    );
}
