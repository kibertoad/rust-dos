//! A 64-pixel Mach64 span, with Z testing and texturing on or off.

use criterion::{BenchmarkId, Criterion};
use rust_dos::mach64::Mach64;
use rust_dos::mach64::engine::{Memory, Size};
use rust_dos::mach64::regs::*;
use rust_dos::mach64::tri::{Palettes, Pipe};
use std::hint::black_box;

pub fn pipelines(c: &mut Criterion) {
    let mut group = c.benchmark_group("mach64_pipeline");
    for format in [3u32, 4, 6, 15] {
        for textured in [false, true] {
            for depth in [false, true] {
                let mut card = Mach64::new(false);
                card.set_reg(
                    SCALE_3D_CNTL,
                    if textured { 2 << 6 | 1 << 22 } else { 3 << 6 } | 1 << 24,
                );
                card.set_reg(DP_PIX_WIDTH, 6 << 28 | format);
                card.set_reg(Z_CNTL, if depth { 0x171 } else { 0 });
                card.set_reg(Z_OFF_PITCH, 512);
                card.set_reg(Z_START, 100 << 12);
                card.set_reg(RED_START, 200 << 16);
                card.set_reg(GREEN_START, 40 << 16);
                card.set_reg(BLUE_START, 80 << 16);
                card.set_reg(ALPHA_START, 255 << 16);
                card.set_reg(RED_X_INC, 1 << 16);
                card.set_reg(TEX_SIZE_PITCH, 0x222);
                card.set_reg(TEX_0_OFF + 2, 8192);
                card.set_reg(S_XINC_START, 1 << 24);
                let mut vram: Vec<_> = (0..16384).map(|i| (i * 73 + i / 11) as u8).collect();
                let mut mem = Memory {
                    vram: &mut vram,
                    lsb_first: false,
                };
                let palettes = Palettes {
                    dac: &[0; 768],
                    dac_8bit: true,
                    texture: None,
                };
                let size = Size::of(format);
                let id = format!(
                    "fmt{format}_texture{text}_z{z}",
                    text = textured as u8,
                    z = depth as u8
                );
                // One pipe for the whole command, as the engine has it;
                // each iteration is a span of it.
                let mut pipe = Pipe::new(&card);
                group.bench_function(BenchmarkId::from_parameter(id), |b| {
                    b.iter(|| {
                        pipe.span(black_box(0), 1);
                        for x in 0..64 {
                            let dst = mem.read(x as u32, size);
                            if let Some(out) = pipe.pixel(&mut mem, &palettes, x, 0, dst) {
                                mem.write(x as u32, size, out);
                            }
                            pipe.span_step();
                        }
                        black_box(&mem);
                    });
                });
            }
        }
    }
    group.finish();
}
