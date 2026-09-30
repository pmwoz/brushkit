//! A binary plist may reference one object any number of times, and each
//! reference expands into its own value. It may also declare objects,
//! collection lengths and UTF-16 strings that plist's binary reader allocates
//! before it yields an event. The guards must stop all of them before they
//! are allocated, on both surfaces: `brushset.plist` before the first member
//! and `Brush.archive` inside a member.
//!
//! One test, because the counting allocator measures the whole binary.
mod common;
mod counting_alloc;

use brushkit_preview::procreate::{MAX_PLIST_PAYLOAD_BYTES, MAX_PLIST_VALUES};
use brushkit_preview::{
    preview_brush, preview_brushset, PreviewOptions, PreviewSet, TipPreview, UnavailableReason,
};
use common::{
    brushset_plist, gray_png, shared_array_values, shared_arrays_plist, shared_data_plist, zip_with,
};
use counting_alloc::{live, peak, reset_peak};
use std::time::{Duration, Instant};

const MIB: usize = 1 << 20;
/// The fewest references to one shared 128 KiB data object that take the
/// plist over the payload guard. The plist's own strings are the extra bytes.
const DATA_CHUNK: usize = 128 * 1024;
const DATA_REFS: usize = MAX_PLIST_PAYLOAD_BYTES / DATA_CHUNK;
/// The reported shape. Six levels of 64 references expand to 64^6 strings,
/// which a walk that does not stop at the value guard takes hours over.
const ARRAY_LEVELS: usize = 6;
const ARRAY_FANOUT: usize = 64;
/// The largest tree the value guard accepts: three levels of 46 references.
const ACCEPTED_LEVELS: usize = 3;
const ACCEPTED_FANOUT: usize = 46;
const _: () = {
    let accepted = shared_array_values(ACCEPTED_LEVELS, ACCEPTED_FANOUT);
    let next = shared_array_values(ACCEPTED_LEVELS, ACCEPTED_FANOUT + 1);
    assert!(accepted <= MAX_PLIST_VALUES && next > MAX_PLIST_VALUES);
};
/// `shared_arrays_plist(1, fanout)` declares `fanout + 7` references: three
/// dictionary pairs, the one member and the payload array. The largest fanout
/// the value guard accepts declares one reference under the budget.
const ACCEPTED_PAYLOAD_FANOUT: usize = MAX_PLIST_VALUES - 8;
const _: () = {
    assert!(shared_array_values(1, ACCEPTED_PAYLOAD_FANOUT) == MAX_PLIST_VALUES);
    assert!(shared_array_values(1, ACCEPTED_PAYLOAD_FANOUT + 1) > MAX_PLIST_VALUES);
};
const CJK_UNITS: usize = 8_000_000;
/// Generous for the guarded walk, which takes under a second, and far under
/// the unbounded walk.
const BOUNDED: Duration = Duration::from_secs(10);
const OPTIONS: PreviewOptions = PreviewOptions { max_cell: 8 };

const ARRAY: u8 = 0xA0;
const DICTIONARY: u8 = 0xD0;

/// `depth` collection headers of one kind, each declaring `count` one-byte
/// references and each starting inside the references of the one before, then
/// one string. Every header's references fit before the trailer, so the
/// reader allocates them all before it yields an event.
fn overlapping_collections(kind: u8, count: u32, depth: usize) -> Vec<u8> {
    let mut out = b"bplist00".to_vec();
    let mut offsets = Vec::new();
    for i in 0..depth {
        offsets.push(out.len() as u32);
        out.extend_from_slice(&[kind | 0xF, 0x12]);
        out.extend_from_slice(&count.to_be_bytes());
        out.push((i + 1) as u8);
    }
    offsets.push(out.len() as u32);
    out.extend_from_slice(&[0x51, b'x']);
    let per_entry = if kind == DICTIONARY { 2 } else { 1 };
    out.resize(out.len() + count as usize * per_entry, 0);
    let table = out.len() as u64;
    for offset in &offsets {
        out.extend_from_slice(&offset.to_be_bytes());
    }
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 4, 1]);
    out.extend_from_slice(&(offsets.len() as u64).to_be_bytes());
    out.extend_from_slice(&0u64.to_be_bytes());
    out.extend_from_slice(&table.to_be_bytes());
    out
}

/// An offset table of `count` one-byte entries, all pointing at one integer.
fn repeated_offsets(count: usize) -> Vec<u8> {
    let mut out = b"bplist00".to_vec();
    out.extend_from_slice(&[0x10, 0]);
    let table = out.len() as u64;
    out.resize(out.len() + count, 8);
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 1]);
    out.extend_from_slice(&(count as u64).to_be_bytes());
    out.extend_from_slice(&0u64.to_be_bytes());
    out.extend_from_slice(&table.to_be_bytes());
    out
}

fn corrupt_message(set: &PreviewSet) -> &str {
    let [entry] = set.entries.as_slice() else {
        panic!("expected one entry, got {}", set.entries.len());
    };
    let TipPreview::Unavailable(UnavailableReason::Corrupt(msg)) = &entry.tip else {
        panic!("expected Corrupt, got {:?}", entry.tip);
    };
    msg
}

#[test]
fn oversized_plists_are_rejected_before_they_are_allocated() {
    let data = shared_data_plist(DATA_REFS, DATA_CHUNK);
    assert!(
        data.len() < 2 * DATA_CHUNK,
        "the writer must share the data object, got {} bytes",
        data.len()
    );
    let arrays = shared_arrays_plist(ARRAY_LEVELS, ARRAY_FANOUT);
    assert!(arrays.len() < 1024, "got {} bytes", arrays.len());
    let shape = gray_png(16, 8, 200);

    let payload_guard =
        format!("plist expands to over {MAX_PLIST_PAYLOAD_BYTES} bytes of strings and data");
    let values_guard = format!("plist expands to over {MAX_PLIST_VALUES} values");
    let references_guard = format!("plist collections declare over {MAX_PLIST_VALUES} references");
    let objects_guard = format!("plist declares over {MAX_PLIST_VALUES} objects");
    let declared_payload_guard =
        format!("plist declares over {MAX_PLIST_PAYLOAD_BYTES} bytes of strings and data");

    let wide_array = overlapping_collections(ARRAY, 150_000, 1);
    let wide_dictionary = overlapping_collections(DICTIONARY, 75_000, 1);
    let nested_arrays = overlapping_collections(ARRAY, 125_000, 32);
    let nested_dictionaries = overlapping_collections(DICTIONARY, 62_500, 32);
    let offset_table = repeated_offsets(400_000);
    let utf16 = brushset_plist(&"\u{5b57}".repeat(CJK_UNITS), &["a"]);

    for (name, plist, guard) in [
        ("shared data", &data, &payload_guard),
        ("shared arrays", &arrays, &values_guard),
        ("wide array", &wide_array, &references_guard),
        ("wide dictionary", &wide_dictionary, &references_guard),
        ("nested arrays", &nested_arrays, &references_guard),
        (
            "nested dictionaries",
            &nested_dictionaries,
            &references_guard,
        ),
        ("offset table", &offset_table, &objects_guard),
        ("utf-16 string", &utf16, &declared_payload_guard),
    ] {
        let budget = plist.len() + MIB;
        let metadata = zip_with(&[("brushset.plist", plist)]);
        let archive = zip_with(&[("Brush.archive", plist), ("Shape.png", &shape)]);

        let before = live();
        reset_peak();
        let start = Instant::now();
        let err = preview_brushset(&metadata, OPTIONS).expect_err("metadata must be rejected");
        let elapsed = start.elapsed();
        let growth = peak() - before;
        assert_eq!(err.0, format!("brushset.plist: {guard}"), "{name} metadata");
        assert!(
            growth < budget,
            "{name} metadata grew the heap by {growth} bytes"
        );
        assert!(elapsed < BOUNDED, "{name} metadata took {elapsed:?}");

        let before = live();
        reset_peak();
        let start = Instant::now();
        let set = preview_brush(&archive, OPTIONS).expect("brush reads");
        let elapsed = start.elapsed();
        let growth = peak() - before;
        assert_eq!(
            corrupt_message(&set),
            format!("Brush.archive: {guard}"),
            "{name} archive"
        );
        assert!(
            growth < budget,
            "{name} archive grew the heap by {growth} bytes"
        );
        assert!(elapsed < BOUNDED, "{name} archive took {elapsed:?}");
    }

    let under = shared_data_plist(DATA_REFS - 1, DATA_CHUNK);
    let set = preview_brushset(&zip_with(&[("brushset.plist", &under)]), OPTIONS)
        .expect("shared data under the guard reads");
    assert_eq!(set.set_name.as_deref(), Some("Shared data"));
    assert_eq!(set.entries.len(), 1);
    assert_eq!(set.entries[0].name, "a");

    // The widest shared-array tree the value guard accepts stays under 16 MiB.
    let accepted = zip_with(&[(
        "brushset.plist",
        &shared_arrays_plist(ACCEPTED_LEVELS, ACCEPTED_FANOUT),
    )]);
    let before = live();
    reset_peak();
    let set = preview_brushset(&accepted, OPTIONS).expect("largest accepted tree reads");
    let growth = peak() - before;
    assert_eq!(set.set_name.as_deref(), Some("Shared arrays"));
    println!("largest accepted tree grew the heap by {growth} bytes");
    assert!(
        growth < 16 * MIB,
        "largest accepted tree grew the heap by {growth} bytes"
    );

    // plist decodes an accepted string once per parse, and each decode holds
    // the UTF-16 units and the UTF-8 copy at once. The keys and the member
    // uuid take the other 12 bytes.
    let name = "\u{5b57}".repeat((MAX_PLIST_PAYLOAD_BYTES - 12) / 3);
    let accepted = zip_with(&[("brushset.plist", &brushset_plist(&name, &["a"]))]);
    let before = live();
    reset_peak();
    let set = preview_brushset(&accepted, OPTIONS).expect("largest accepted string reads");
    let growth = peak() - before;
    assert_eq!(set.set_name.as_deref(), Some(name.as_str()));
    println!("largest accepted string grew the heap by {growth} bytes");
    assert!(
        growth < 8 * MIB,
        "largest accepted string grew the heap by {growth} bytes"
    );

    // A plist the value guard accepts declares fewer references than it
    // expands to values, so the reference budget never rejects it first.
    let read = |fanout: usize| {
        let plist = shared_arrays_plist(1, fanout);
        preview_brushset(&zip_with(&[("brushset.plist", &plist)]), OPTIONS)
    };
    let set = read(ACCEPTED_PAYLOAD_FANOUT).expect("largest accepted payload reads");
    assert_eq!(set.set_name.as_deref(), Some("Shared arrays"));
    assert_eq!(
        read(ACCEPTED_PAYLOAD_FANOUT + 1)
            .expect_err("one value over")
            .0,
        format!("brushset.plist: {values_guard}")
    );
    assert_eq!(
        read(ACCEPTED_PAYLOAD_FANOUT + 2)
            .expect_err("one reference over")
            .0,
        format!("brushset.plist: {references_guard}")
    );
}
