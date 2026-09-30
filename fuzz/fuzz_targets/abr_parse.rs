#![no_main]
use brushkit_abr::{parse_abr, parse_abr_all_deferred_without_patterns, AbrPack, SampledBrush};
use libfuzzer_sys::fuzz_target;

/// The `Readable` indices of `sampled_brushes`, which must be exactly
/// `0..brushes.len()` in order.
fn readable_indices(pack: &AbrPack) -> Vec<usize> {
    let indices: Vec<usize> = pack
        .sampled_brushes
        .iter()
        .filter_map(|brush| match brush {
            SampledBrush::Readable(i) => Some(*i),
            SampledBrush::Unavailable(_) => None,
        })
        .collect();
    assert!(indices.iter().copied().eq(0..pack.brushes.len()));
    indices
}

fuzz_target!(|data: &[u8]| {
    if let Ok(pack) = parse_abr(data) {
        readable_indices(&pack);
    }
    if let Ok(deferred) = parse_abr_all_deferred_without_patterns(data) {
        for i in readable_indices(&deferred.pack) {
            let _ = deferred.decode_tip(i);
        }
    }
});
