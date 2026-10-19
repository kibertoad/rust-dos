use super::*;

fn channels(color: Color) -> [i32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn palette(i: u8) -> (u8, u8, u8) {
    (i, i.wrapping_mul(3), i.wrapping_mul(7))
}

#[test]
fn specialized_sampling_matches_dynamic_formats() {
    for len in [1usize, 3, 31, 32, 257, 4096] {
        let mut memory: Vec<_> = (0..len).map(|i| (i * 73 + 19) as u8).collect();
        let mut target = Target {
            vram: &mut memory,
            palette: &palette,
        };
        for destination in 0..3 {
            for format in 0..8 {
                for filter in 0..8 {
                    for mip in [0, 1, 3, 9] {
                        for perspective in [false, true] {
                            for wrap in [false, true] {
                                let cmd = CMD_3D
                                    | destination << 2
                                    | format << 5
                                    | mip << 8
                                    | filter << 12
                                    | if perspective {
                                        UNLIT_TEX_TRI_PERSP
                                    } else {
                                        UNLIT_TEX_TRI
                                    } << 27
                                    | if wrap { CMD_TWE } else { 0 };
                                let mut regs = [0; 0x80];
                                regs[0xEC >> 2] = len.saturating_sub(8) as u32;
                                regs[0xE4 >> 2] = 24;
                                regs[0xF0 >> 2] = 0x123456;
                                regs[0xF8 >> 2] = 0x102030;
                                regs[0xFC >> 2] = 0xE0D0C0;
                                regs[0x104 >> 2] = 0x1234;
                                regs[0x108 >> 2] = 0x4321;
                                let mut c = Context::new(&mut target, cmd, &regs).unwrap();
                                c.setup_texture(cmd, &regs);
                                for (u, v, w, d) in [
                                    (0, 0, 1 << 20, 0),
                                    (1 << 26, 1 << 25, 1 << 19, 1 << 26),
                                    ((1 << 27) - 1, (1 << 27) - 1, 1, 1 << 27),
                                    (-1, 0, 0, -1),
                                    (1 << 27, -100, -1, 3 << 27),
                                    (i32::MAX, i32::MIN, 0x1234567, i32::MAX),
                                ] {
                                    let at = Attr {
                                        u,
                                        v,
                                        w,
                                        d,
                                        ..Attr::default()
                                    };
                                    let specialized = channels(c.sample_texture(&at));
                                    let dynamic = channels(c.sample_texture_in::<8>(&at));
                                    assert_eq!(
                                        specialized, dynamic,
                                        "len={len}, cmd={cmd:#x}, uvwd={u},{v},{w},{d}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn selected_formats_keep_their_channel_decoding() {
    let mut memory = [0; 16];
    let mut target = Target {
        vram: &mut memory,
        palette: &palette,
    };
    let mut regs = [0; 0x80];
    regs[0xF8 >> 2] = 0x000A14;
    regs[0xFC >> 2] = 0x96A0AA;
    for (format, value, want) in [
        (TEX_ARGB8888, 0x12345678, [0x34, 0x56, 0x78, 0x12]),
        (TEX_ARGB4444, 0x1234, [0x22, 0x33, 0x44, 0x11]),
        (TEX_ARGB1555, 0xFC00, [255, 0, 0, 255]),
        (TEX_ALPHA4BLEND4, 0x83, [30, 40, 50, 136]),
        (TEX_BLEND4_LO, 0xAB, [110, 120, 130, 255]),
        (TEX_BLEND4_HI, 0xAB, [100, 110, 120, 255]),
        (TEX_PAL8, 0x42, [66, 198, 206, 255]),
        (TEX_YUV, 0x80EB, [255, 255, 255, 255]),
    ] {
        let cmd = CMD_3D | 1 << 2 | format << 5 | UNLIT_TEX_TRI << 27;
        let mut c = Context::new(&mut target, cmd, &regs).unwrap();
        c.setup_texture(cmd, &regs);
        c.tex_bdr_clr = value;
        // A coordinate outside a non-wrapping texture decodes the border
        // with the same selected format, without reading video memory.
        assert_eq!(
            channels(c.sample_texture(&Attr {
                u: -1,
                ..Attr::default()
            })),
            want
        );
    }
}
