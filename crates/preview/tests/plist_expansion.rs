//! A binary plist may reference one object any number of times, and each
//! reference expands into its own value. The guards must stop that before
//! the tree is built, on both surfaces: `brushset.plist` before the first
//! member and `Brush.archive` inside a member.
//!
//! One test, because the counting allocator measures the whole binary.
mod common;
mod counting_alloc;

use brushkit_preview::procreate::{MAX_PLIST_PAYLOAD_BYTES, MAX_PLIST_VALUES};
use brushkit_preview::{
    preview_brush, preview_brushset, PreviewOptions, PreviewSet, TipPreview, UnavailableReason,
};
use common::{gray_png, shared_array_values, shared_arrays_plist, shared_data_plist, zip_with};
use counting_alloc::{live, peak, reset_peak};
use std::time::{Duration, Instant};

const MIB: usize = 1 << 20;
/// One reference over the payload guard: 129 references to a 128 KiB data
/// object, 131 KiB on disk and 16.1 MiB expanded.
const DATA_CHUNK: usize = 128 * 1024;
const DATA_REFS: usize = MAX_PLIST_PAYLOAD_BYTES / DATA_CHUNK + 1;
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
/// Generous for the guarded walk, which takes under a second, and far under
/// the unbounded walk.
const BOUNDED: Duration = Duration::from_secs(10);
const OPTIONS: PreviewOptions = PreviewOptions { max_cell: 8 };

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
fn shared_object_plists_are_rejected_before_expansion() {
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

    for (name, plist, guard) in [
        ("shared data", &data, &payload_guard),
        ("shared arrays", &arrays, &values_guard),
    ] {
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
            growth < MIB,
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
            growth < MIB,
            "{name} archive grew the heap by {growth} bytes"
        );
        assert!(elapsed < BOUNDED, "{name} archive took {elapsed:?}");
    }

    let under = shared_data_plist(DATA_REFS / 2, DATA_CHUNK);
    let set = preview_brushset(&zip_with(&[("brushset.plist", &under)]), OPTIONS)
        .expect("shared data under the guard reads");
    assert_eq!(set.set_name.as_deref(), Some("Shared data"));
    assert_eq!(set.entries.len(), 1);
    assert_eq!(set.entries[0].name, "a");

    // The two guards share one budget: the largest accepted tree costs less
    // than the payload ceiling, so an accepted plist stays under twice it.
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
        growth < MAX_PLIST_PAYLOAD_BYTES,
        "largest accepted tree grew the heap by {growth} bytes"
    );
}
