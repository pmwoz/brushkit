mod counting_alloc;

use std::borrow::Cow;

use brushkit_preview::procreate::decode_tip_png;
use brushkit_preview::{decode_tip_image, GrayscaleBitmap};
use counting_alloc::{live, peak, reset_peak};

const SIDE: u32 = 16;
const PIXELS: usize = (SIDE * SIDE) as usize;
const DECODER_SLACK: usize = 512 * 1024;

fn png(profile_bytes: usize) -> Vec<u8> {
    let mut info = png::Info::with_size(SIDE, SIDE);
    info.color_type = png::ColorType::Grayscale;
    info.bit_depth = png::BitDepth::Eight;
    if profile_bytes != 0 {
        info.icc_profile = Some(Cow::Owned(vec![0; profile_bytes]));
    }
    let mut bytes = Vec::new();
    {
        let mut writer = png::Encoder::with_info(&mut bytes, info)
            .unwrap()
            .write_header()
            .unwrap();
        writer.write_image_data(&[0x40; PIXELS]).unwrap();
    }
    bytes
}

#[test]
fn unused_iccp_profile_stays_within_decode_budget() {
    type Decode = fn(&[u8]) -> GrayscaleBitmap;
    let decoders: [(&str, Decode, u8); 2] = [
        (
            "decode_tip_image",
            |bytes| decode_tip_image(bytes).unwrap(),
            0xBF,
        ),
        (
            "decode_tip_png",
            |bytes| decode_tip_png(bytes).unwrap(),
            0x40,
        ),
    ];
    let mut measurements = Vec::with_capacity(4);
    for profile_bytes in [0, 8 * 1024 * 1024] {
        let bytes = png(profile_bytes);
        for (name, decode, expected) in decoders {
            let before = live();
            reset_peak();
            let bitmap = decode(&bytes);
            let growth = peak() - before;
            assert_eq!((bitmap.width, bitmap.height), (SIDE, SIDE));
            assert_eq!(bitmap.data, [expected; PIXELS]);
            println!("{name}, {profile_bytes} profile bytes: {growth} peak bytes");
            measurements.push((name, profile_bytes, growth));
        }
    }
    for (name, profile_bytes, growth) in measurements {
        assert!(
            growth <= PIXELS + DECODER_SLACK,
            "{name}, {profile_bytes} profile bytes: peak {growth} exceeds counted image bytes plus decoder slack ({})",
            PIXELS + DECODER_SLACK
        );
    }
}
