#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = brushkit_abr::parse_abr(data);
    if let Ok(deferred) = brushkit_abr::parse_abr_all_deferred_without_patterns(data) {
        for i in 0..deferred.pack.brushes.len() {
            let _ = deferred.decode_tip(i);
        }
    }
});
