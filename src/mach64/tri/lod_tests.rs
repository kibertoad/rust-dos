use super::lod::Lod;
use super::*;

/// The level of detail as `sample` worked it out for every pixel before
/// it was cached.
fn original_lod(control: u32, largest: u32, steps: [i64; 4]) -> f64 {
    let mut lod = 0.0;
    if control & (1 << 24) == 0 && largest > 0 {
        let scale = (1u64 << largest) as f64 / 1024.0 / 65536.0;
        let rho = steps
            .iter()
            .map(|&d| (d as f64).abs() * scale)
            .fold(0.0, f64::max);
        if rho > 1.0 {
            lod = rho.log2();
        }
    }
    lod
}

fn reference(pipe: &Pipe, texels: &TexelSource<'_, '_>) -> Argb {
    let largest = pipe.tex_size >> 4 & 0xF;
    let lod = original_lod(pipe.cntl, largest, pipe.lod_steps());
    let (s, t) = (pipe.span_s.0, pipe.span_t.0);
    if lod <= 0.0 {
        return pipe.sample_level(texels, largest, s, t, pipe.cntl & (1 << 25) != 0);
    }
    let whole = (lod as u32).min(largest);
    let level = largest - whole;
    let larger = level.saturating_sub(1);
    match pipe.cntl >> 26 & 3 {
        1 => {
            let near = pipe.sample_level(texels, level, s, t, false);
            let next = pipe.sample_level(texels, larger, s, t, false);
            near.lerp(next, ((lod - whole as f64) * 256.0) as i32)
        }
        2 => pipe.sample_level(texels, level, s, t, true),
        3 => pipe.sample_level(texels, larger, s, t, true),
        _ => pipe.sample_level(texels, level, s, t, false),
    }
}

#[test]
fn affine_and_curved_spans_keep_their_original_mip_samples() {
    let mut memory: Vec<_> = (0..16384).map(|i| (i * 73 + i / 11) as u8).collect();
    let mem = Memory {
        vram: &mut memory,
        lsb_first: false,
    };
    let palettes = Palettes {
        dac: &[0; 768],
        dac_8bit: true,
        texture: None,
    };
    let texels = TexelSource {
        mem: &mem,
        palettes: &palettes,
        ci4: 0,
        signed_uv: false,
    };
    for curvature in [0u32, 1 << 20, 0x07FF_FFFF] {
        for mode in 0..4 {
            for largest in [1, 4, 10] {
                for disabled in [false, true] {
                    let mut card = Mach64::new(false);
                    card.set_reg(
                        SCALE_3D_CNTL,
                        2 << 6 | mode << 26 | 1 << 25 | if disabled { 1 << 24 } else { 0 },
                    );
                    card.set_reg(DP_PIX_WIDTH, 6 << 28 | 6);
                    card.set_reg(TEX_SIZE_PITCH, largest << 8 | largest << 4 | largest);
                    card.set_reg(S_XINC_START, 1 << 25);
                    card.set_reg(T_XINC_START, 1 << 24);
                    card.set_reg(S_Y_INC, 1 << 26);
                    card.set_reg(T_Y_INC, 1 << 23);
                    card.set_reg(S_X_INC2, curvature);
                    card.set_reg(T_X_INC2, curvature);
                    card.set_reg(S_XY_INC2, curvature);
                    card.set_reg(T_XY_INC2, curvature);
                    for i in 0..11 {
                        card.set_reg(TEX_0_OFF + i, i as u32 * 256);
                    }
                    let mut pipe = Pipe::new(&card);
                    for direction in [-1, 1] {
                        for offset in [0, 3, 31] {
                            pipe.span(offset, direction);
                            for _ in 0..64 {
                                assert_eq!(
                                    pipe.sample(&texels),
                                    reference(&pipe, &texels),
                                    "curvature={curvature}, mode={mode}, largest={largest}, \
                                     disabled={disabled}, direction={direction}, offset={offset}"
                                );
                                pipe.span_step();
                            }
                            pipe.step_x();
                            pipe.step_y();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn cached_lod_matches_the_original_floating_point_operations() {
    let mut random = 0x123456789ABCDEFu64;
    for largest in 0..16 {
        for disabled in [false, true] {
            for curvature in [0, 1] {
                let control = 2 << 6 | if disabled { 1 << 24 } else { 0 };
                let mut cache = Lod::new(control, largest << 4, [curvature; 4]);
                for i in 0..4096 {
                    let steps = std::array::from_fn(|j| {
                        random ^= random << 13;
                        random ^= random >> 7;
                        random ^= random << 17;
                        match i {
                            0 => 0,
                            1 => i64::MIN,
                            2 => i64::MAX,
                            3..=66 => (1i64 << (i - 3)).wrapping_add(j as i64).wrapping_sub(2),
                            _ => random as i64,
                        }
                    });
                    // An affine span works its level out at the start; a
                    // curved one at every pixel, whatever the start saw.
                    if curvature == 0 {
                        cache.start(steps);
                    }
                    assert_eq!(
                        cache.value(steps).to_bits(),
                        original_lod(control, largest, steps).to_bits(),
                        "largest={largest}, disabled={disabled}, curvature={curvature}, \
                         steps={steps:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn affine_spans_refresh_at_their_start_and_ignore_smaller_derivatives() {
    let mut cache = Lod::new(2 << 6, 10 << 4, [0; 4]);
    cache.start([1 << 20, 0, 0, 0]);
    assert_eq!(cache.value([1 << 20, 1 << 19, 10, 100]), 4.0);
    cache.start([-(1 << 22), 0, 0, 0]);
    assert_eq!(cache.value([0; 4]), 6.0);
}
