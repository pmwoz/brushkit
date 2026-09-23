//! A counting global allocator for the peak-memory test binaries. A binary
//! that includes this module holds EXACTLY ONE `#[test]`: cargo runs a
//! binary's tests on parallel threads, so a second test would allocate
//! underneath the measurement and corrupt every peak.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

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
            if new_ptr != ptr {
                // A moved block is copied while the old one is still live.
                record(CURRENT.load(Ordering::Relaxed) + new_size);
            }
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
pub fn reset_peak() {
    PEAK.store(CURRENT.load(Ordering::Relaxed), Ordering::Relaxed);
}

pub fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

pub fn live() -> usize {
    CURRENT.load(Ordering::Relaxed)
}
