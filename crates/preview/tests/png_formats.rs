use brushkit_preview::decode_tip_image;
use brushkit_preview::procreate::decode_tip_png;

fn assert_matches_image(bytes: &[u8]) {
    let reference = image::load_from_memory(bytes).unwrap();
    let coverage: Vec<u8> = if reference.color().has_alpha() {
        reference
            .to_luma_alpha8()
            .pixels()
            .map(|p| p.0[1])
            .collect()
    } else {
        reference
            .to_luma8()
            .pixels()
            .map(|p| 255 - p.0[0])
            .collect()
    };
    for (tip, expected) in [
        (decode_tip_image(bytes).unwrap(), coverage),
        (
            decode_tip_png(bytes).unwrap(),
            reference.to_luma8().into_raw(),
        ),
    ] {
        assert_eq!(
            (tip.width, tip.height),
            (reference.width(), reference.height())
        );
        assert_eq!(tip.data, expected);
    }
}

#[test]
fn low_bit_and_indexed_pngs_match_image() {
    for depth in [png::BitDepth::One, png::BitDepth::Two, png::BitDepth::Four] {
        for color in [png::ColorType::Grayscale, png::ColorType::Indexed] {
            for transparent in [false, true] {
                let mut bytes = Vec::new();
                {
                    let mut encoder = png::Encoder::new(&mut bytes, 8, 1);
                    encoder.set_color(color);
                    encoder.set_depth(depth);
                    if color == png::ColorType::Indexed {
                        encoder.set_palette(vec![0, 0, 0, 255, 128, 64]);
                        if transparent {
                            encoder.set_trns(vec![0, 128]);
                        }
                    } else if transparent {
                        encoder.set_trns(vec![0, 1]);
                    }
                    let mut writer = encoder.write_header().unwrap();
                    let data: &[u8] = match depth {
                        png::BitDepth::One => &[0x55],
                        png::BitDepth::Two => &[0x11, 0x11],
                        png::BitDepth::Four => &[0x01; 4],
                        _ => unreachable!(),
                    };
                    writer.write_image_data(data).unwrap();
                }
                assert_matches_image(&bytes);
            }
        }
    }
}

#[test]
fn apng_default_image_matches_image() {
    for separate_default in [false, true] {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .set_animated(if separate_default { 1 } else { 2 }, 0)
                .unwrap();
            encoder.set_sep_def_img(separate_default).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0x40; 4]).unwrap();
            writer.write_image_data(&[0x80; 4]).unwrap();
        }
        assert_matches_image(&bytes);
    }
}
