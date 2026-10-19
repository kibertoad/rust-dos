use super::*;

fn run(card: &Mach64, original: &[u8], dynamic: bool) -> (Vec<u8>, Vec<Option<u32>>) {
    let mut memory = original.to_vec();
    let mut mem = Memory {
        vram: &mut memory,
        lsb_first: false,
    };
    let dac: Vec<_> = (0..768).map(|i| (i * 71 + 17) as u8).collect();
    let palettes = Palettes {
        dac: &dac,
        dac_8bit: true,
        texture: None,
    };
    let mut pipe = Pipe::new(card);
    if dynamic {
        pipe.shade = Pipe::pixel_mode::<false, false, false, 16>;
    }
    let mut pixels = Vec::new();
    for direction in [-1, 1] {
        pipe.span(2, direction);
        for x in 0..8 {
            let pixel = pipe.pixel(&mut mem, &palettes, x, 2, 0xABCD1234);
            pixels.push(pixel);
            pipe.span_step();
        }
        pipe.step_x();
        pipe.step_y();
    }
    (memory, pixels)
}

#[test]
fn selected_pipeline_matches_dynamic_pixels_and_depth_writes() {
    let original: Vec<_> = (0..4096).map(|i| (i * 73 + i / 11) as u8).collect();
    for destination in 0..16 {
        for depth in 0..16 {
            for textured in [false, true] {
                for option in 0u32..32 {
                    let mut card = Mach64::new(false);
                    let control = if textured { 2 << 6 } else { 3 << 6 }
                        | (option & 3) << 22
                        | (option >> 2 & 3) << 11
                        | (option & 7) << 16
                        | (option >> 1 & 7) << 19
                        | (option & 1) << 2
                        | (option & 2) << 3
                        | (option & 4) << 8
                        | (option & 8) << 10
                        | (option & 16) << 10
                        | (option & 1) << 28
                        | (option & 2) << 28
                        | (option & 4) << 28
                        | (option & 8) << 21
                        | (option & 16) << 21;
                    card.set_reg(SCALE_3D_CNTL, control);
                    card.set_reg(DP_PIX_WIDTH, (option & 15) << 28 | destination);
                    card.set_reg(Z_CNTL, (depth & 1) | (depth >> 1) << 4 | (option & 16) << 4);
                    card.set_reg(Z_OFF_PITCH, 8 << 22 | 256);
                    card.set_reg(Z_START, 100 << 12);
                    card.set_reg(Z_X_INC, 3 << 12);
                    card.set_reg(RED_START, 200 << 16);
                    card.set_reg(GREEN_START, 40 << 16);
                    card.set_reg(BLUE_START, 80 << 16);
                    card.set_reg(ALPHA_START, 128 << 16);
                    card.set_reg(RED_X_INC, 1 << 16);
                    card.set_reg(ALPHA_X_INC, 5 << 16);
                    card.set_reg(TEX_SIZE_PITCH, 0x222);
                    card.set_reg(S_XINC_START, 1 << 24);
                    card.set_reg(T_Y_INC, 1 << 25);
                    card.set_reg(S_X_INC2, option << 16);
                    card.set_reg(CLR_CMP_CLR, 0x112233);
                    card.set_reg(CLR_CMP_MSK, 0xFFFF00);
                    card.set_reg(CLR_CMP_CNTL, 2 << 24 | option & 7);
                    card.set_reg(DP_FRGD_CLR, 0x123456);
                    for level in 0..11 {
                        card.set_reg(TEX_0_OFF + level, 1024 + level as u32 * 128);
                    }
                    let selected = run(&card, &original, false);
                    let dynamic = run(&card, &original, true);
                    assert_eq!(
                        selected.1, dynamic.1,
                        "destination={destination}, depth={depth}, texture={textured}, \
                         option={option}"
                    );
                    assert_eq!(
                        selected.0, dynamic.0,
                        "VRAM destination={destination}, depth={depth}, texture={textured}, \
                         option={option}"
                    );
                }
            }
        }
    }
}
