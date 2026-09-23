mod counting_alloc;

use brushkit_abr::{parse_abr, parse_abr_deferred};
use brushkit_preview::{preview_abr, PreviewOptions, TipPreview};
use counting_alloc::{live, peak, reset_peak};

const MIB: usize = 1 << 20;
const TIP_SIDE: u32 = 1024;
const TIPS: usize = 6;

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

fn with_patterns(mut bytes: Vec<u8>, count: usize) -> Vec<u8> {
    let side = 2048u32;
    let mut record = Vec::new();
    for value in [1u32, 1] {
        record.extend_from_slice(&value.to_be_bytes());
    }
    record.extend_from_slice(&(side as u16).to_be_bytes());
    record.extend_from_slice(&(side as u16).to_be_bytes());
    record.extend_from_slice(&0u32.to_be_bytes());
    record.push(1);
    record.push(b'p');
    for value in [
        3u32,
        0,
        0,
        0,
        side,
        side,
        1,
        1,
        23 + side * side,
        8,
        0,
        0,
        side,
        side,
    ] {
        record.extend_from_slice(&value.to_be_bytes());
    }
    record.extend_from_slice(&8u16.to_be_bytes());
    record.push(0);
    record.resize(record.len() + (side * side) as usize, 128);

    let record_len = (4 + record.len()).next_multiple_of(4);
    bytes.extend_from_slice(b"8BIMpatt");
    bytes.extend_from_slice(&((record_len * count) as u32).to_be_bytes());
    for _ in 0..count {
        bytes.extend_from_slice(&(record.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&record);
        bytes.resize(bytes.len().next_multiple_of(4), 0);
    }
    bytes
}

fn padded_samp_fixture(payload_size: usize) -> Vec<u8> {
    let entry = build_simple_entry(8, 8, 8, 0x80);
    let mut bytes = Vec::with_capacity(16 + payload_size);
    bytes.extend_from_slice(&6u16.to_be_bytes());
    bytes.extend_from_slice(&2u16.to_be_bytes());
    bytes.extend_from_slice(b"8BIMsamp");
    bytes.extend_from_slice(&(payload_size as u32).to_be_bytes());
    bytes.extend_from_slice(&((payload_size - 4) as u32).to_be_bytes());
    bytes.extend_from_slice(&entry);
    // Opaque trailing bytes enlarge the payload without changing its tip.
    bytes.resize(16 + payload_size, 0);
    bytes
}

fn padded_samp_peak(payload_size: usize) -> usize {
    let bytes = padded_samp_fixture(payload_size);
    let before = live();
    reset_peak();
    let set = preview_abr(&bytes, PreviewOptions { max_cell: 64 }).expect("padded samp preview");
    let growth = peak() - before;
    assert_eq!(set.entries.len(), 1);
    let TipPreview::Available(tip) = &set.entries[0].tip else {
        panic!("expected padded samp tip");
    };
    assert_eq!((tip.width, tip.height), (8, 8));
    assert_eq!(tip.data, vec![0x80; 64]);
    println!(
        "preview with {} MiB samp payload: {growth} bytes",
        payload_size / MIB
    );
    growth
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

    for count in [1, 8] {
        let patterned = with_patterns(bytes.clone(), count);
        let pack = parse_abr(&patterned).expect("pattern fixture parses");
        assert_eq!(pack.patterns.len(), count);
        assert!(pack.patterns.iter().all(|p| p.gray.len() == 4 * MIB));
        drop(pack);

        let before = live();
        reset_peak();
        let set = preview_abr(&patterned, PreviewOptions { max_cell: 64 }).expect("preview");
        let pattern_peak = peak() - before;
        assert_eq!(set.entries.len(), TIPS);
        drop(set);
        println!(
            "preview with {} MiB of patterns: {:.2} MiB",
            count * 4,
            pattern_peak as f64 / MIB as f64
        );
        assert!(
            pattern_peak <= p_prev + MIB,
            "patterns must not increase preview allocations: without {p_prev}, with {pattern_peak} bytes"
        );
    }

    let small = padded_samp_peak(MIB);
    let large = padded_samp_peak(32 * MIB);
    assert!(
        large <= small + 64 * 1024,
        "preview allocations must not grow with samp payload size: small {small}, large {large} bytes"
    );
}
