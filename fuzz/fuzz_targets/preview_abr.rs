#![no_main]
use brushkit_preview::{preview_abr, preview_abr_first_available, PreviewOptions};
use libfuzzer_sys::fuzz_target;

mod tip_shapes;

const OPTIONS: PreviewOptions = PreviewOptions { max_cell: 8 };

fuzz_target!(|data: &[u8]| {
    tip_shapes::assert_tip_shapes(preview_abr(data, OPTIONS), OPTIONS);
    tip_shapes::assert_tip_shapes(preview_abr_first_available(data, OPTIONS, 4), OPTIONS);
});
