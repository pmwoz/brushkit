mod common;

use std::time::{Duration, Instant};

use brushkit_preview::decode_tip_image;
use common::baseline_jpeg;

const EXTENDED_XMP: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

fn with_incomplete_xmp_parts(jpeg: &[u8], namespace: &[u8], parts: usize) -> Vec<u8> {
    const GUID: [u8; 32] = [b'A'; 32];
    const TOTAL_SIZE: u32 = 1_000;
    const OFFSET: u32 = 0;
    let mut part = vec![0xFF, 0xE1];
    let length = u16::try_from(2 + namespace.len() + GUID.len() + 8).expect("segment fits");
    part.extend_from_slice(&length.to_be_bytes());
    part.extend_from_slice(namespace);
    part.extend_from_slice(&GUID);
    part.extend_from_slice(&TOTAL_SIZE.to_be_bytes());
    part.extend_from_slice(&OFFSET.to_be_bytes());
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&part.repeat(parts));
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn fastest_decode(bytes: &[u8]) -> Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            decode_tip_image(bytes).expect("decodes");
            start.elapsed()
        })
        .min()
        .expect("three runs")
}

#[test]
fn incomplete_extended_xmp_parts_decode_in_linear_time() {
    const PARTS: usize = 20_000;
    let jpeg = baseline_jpeg(8, 8, 1);
    let xmp = with_incomplete_xmp_parts(&jpeg, EXTENDED_XMP, PARTS);
    let mut unknown_namespace = EXTENDED_XMP.to_vec();
    unknown_namespace[0] = b'H';
    let unknown = with_incomplete_xmp_parts(&jpeg, &unknown_namespace, PARTS);
    assert_eq!(xmp.len(), unknown.len());

    let unknown_time = fastest_decode(&unknown);
    let xmp_time = fastest_decode(&xmp);
    assert!(
        xmp_time < 3 * unknown_time + Duration::from_millis(100),
        "{PARTS} extended XMP parts took {xmp_time:?}, the same bytes of unknown APP1 segments {unknown_time:?}"
    );
}
