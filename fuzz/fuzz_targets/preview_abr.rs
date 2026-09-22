#![no_main]
use brushkit_preview::{preview_abr, preview_abr_first_available, PreviewOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = preview_abr(data, PreviewOptions { max_cell: 8 });
    let _ = preview_abr_first_available(data, PreviewOptions { max_cell: 8 }, 4);
});
