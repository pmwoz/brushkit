#![no_main]
use brushkit_preview::{preview_brushset, PreviewOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = preview_brushset(data, PreviewOptions { max_cell: 8 });
});
