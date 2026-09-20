//! The preview pipeline's peak allocation must follow the LARGEST tip, not the
//! pack, while the eager parse's grows with the pack. One counting allocator
//! measures both.
//!
//! EXACTLY ONE `#[test]` lives in this file. The allocator is global and cargo
//! runs a binary's tests on parallel threads, so a second test here would
//! allocate underneath the measurement and corrupt every peak.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use brushkit_abr::{parse_abr, parse_abr_deferred};
use brushkit_preview::{preview_abr, PreviewOptions, TipPreview};

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

fn record(current: usize) {
    PEAK.fetch_max(current, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record(CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                let grew = new_size - layout.size();
                record(CURRENT.fetch_add(grew, Ordering::Relaxed) + grew);
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Peaks are measured as growth above whatever is live right now, so the
/// fixture bytes the caller already holds do not count.
fn reset_peak() {
    PEAK.store(CURRENT.load(Ordering::Relaxed), Ordering::Relaxed);
}

fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

fn live() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

const MIB: usize = 1 << 20;
const TIP_SIDE: u32 = 1024;
const TIPS: usize = 6;

/// One uncompressed v6 samp entry: `u32 0`, the rect, depth, compression 0 and
/// `w * h` bytes of `val`.
fn build_simple_entry(w: u32, h: u32, depth: u16, val: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(23 + (w as usize) * (h as usize));
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&0i32.to_be_bytes());
    buf.extend_from_slice(&0i32.to_be_bytes());
    buf.extend_from_slice(&(h as i32).to_be_bytes());
    buf.extend_from_slice(&(w as i32).to_be_bytes());
    buf.extend_from_slice(&depth.to_be_bytes());
    buf.push(0);
    buf.extend(std::iter::repeat_n(val, (w as usize) * (h as usize)));
    buf
}

/// A v6/sub-2 pack of `TIPS` uncompressed 1 MiB tips in one `samp` block.
fn fixture() -> Vec<u8> {
    let mut payload = Vec::new();
    for _ in 0..TIPS {
        let entry = build_simple_entry(TIP_SIDE, TIP_SIDE, 8, 0x80);
        payload.extend_from_slice(&(entry.len() as u32).to_be_bytes());
        payload.extend_from_slice(&entry);
        payload.resize(payload.len().next_multiple_of(4), 0);
    }

    let mut file = Vec::with_capacity(payload.len() + 16);
    file.extend_from_slice(&6u16.to_be_bytes());
    file.extend_from_slice(&2u16.to_be_bytes());
    file.extend_from_slice(b"8BIM");
    file.extend_from_slice(b"samp");
    file.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    file.extend_from_slice(&payload);
    file
}

#[test]
fn preview_peak_follows_one_tip_not_the_pack() {
    let bytes = fixture();
    assert_eq!(
        parse_abr(&bytes).expect("fixture parses").brushes.len(),
        TIPS,
        "the fixture must carry {TIPS} sampled tips"
    );

    let before = live();

    reset_peak();
    let deferred = parse_abr_deferred(&bytes).expect("deferred parse");
    let p_def = peak() - before;
    drop(deferred);

    reset_peak();
    let set = preview_abr(&bytes, PreviewOptions { max_cell: 64 }).expect("preview");
    let p_prev = peak() - before;
    for entry in &set.entries {
        let TipPreview::Available(tip) = &entry.tip else {
            panic!("expected an available tip, got {:?}", entry.tip);
        };
        assert!(
            tip.width <= 64 && tip.height <= 64,
            "tip {}x{} exceeds the cell",
            tip.width,
            tip.height
        );
    }
    drop(set);

    reset_peak();
    let pack = parse_abr(&bytes).expect("eager parse");
    let p_eager = peak() - before;
    drop(pack);

    println!(
        "peak growth: deferred {:.2} MiB, preview {:.2} MiB, eager {:.2} MiB",
        p_def as f64 / MIB as f64,
        p_prev as f64 / MIB as f64,
        p_eager as f64 / MIB as f64
    );

    assert!(
        p_prev < p_def + 3 * MIB,
        "the preview must hold one tip above the deferred parse, not {TIPS}: \
         preview {p_prev} bytes vs deferred {p_def} bytes"
    );
    assert!(
        p_eager >= TIPS * MIB,
        "the eager parse materializes every tip: {p_eager} bytes"
    );
}
