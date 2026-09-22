#![no_main]
use brushkit_preview::{preview_brush, preview_brush_first_available, PreviewOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = preview_brush(data, PreviewOptions { max_cell: 8 });
    let _ = preview_brush_first_available(data, PreviewOptions { max_cell: 8 }, 4);
});
