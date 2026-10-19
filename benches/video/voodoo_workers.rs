//! Worker scheduling with small, concentrated, thin and tall triangles.

use criterion::{BenchmarkId, Criterion};
use rust_dos::voodoo::mem::Vram;
use rust_dos::voodoo::raster::{RasterState, Stride, TriParams};
use rust_dos::voodoo::workers::{Job, Pool};
use std::hint::black_box;

pub fn scheduling(c: &mut Criterion) {
    let mut group = c.benchmark_group("voodoo_worker_scheduling");
    for (name, count, height) in [
        ("small", 256, 16),
        ("concentrated", 256, 16),
        ("thin", 256, 480),
        ("tall", 32, 480),
    ] {
        for workers in [0, 1, 4, 8] {
            let state = RasterState {
                fb: Vram::new(512 * 1024 * 2),
                fbz_color_path: 0,
                fbz_mode: 1 << 9,
                alpha_mode: 0,
                fog_mode: 0,
                za_color: 0,
                chroma_key: 0,
                color0: 0,
                color1: 0,
                fog_color: 0,
                clip_left_right: 512,
                clip_low_y_high_y: 480,
                clip_mask: 0xFFF,
                chroma_range: 0,
                fogdelta_mask: 0xFC,
                yorigin: 479,
                ymask: 1023,
                rowpixels: 512,
                stride: Stride::linear(512),
                aux_stride: Stride::linear(512),
                dest: 0,
                aux: None,
                fogblend: [0; 64],
                fogdelta: [0; 64],
                send_config: None,
                stipple: 0,
                tmu: [None, None],
            };
            let parameters = TriParams {
                startr: 180 << 12,
                startg: 80 << 12,
                startb: 40 << 12,
                starta: 255 << 12,
                ..Default::default()
            };
            let pool = Pool::new(workers);
            group.bench_function(BenchmarkId::new(name, workers), |bench| {
                bench.iter(|| {
                    for i in 0..count {
                        let y = if name == "small" {
                            (i * 17 % 464) as f32
                        } else {
                            0.0
                        };
                        let width = if name == "thin" {
                            4.0
                        } else if height == 480 {
                            512.0
                        } else {
                            64.0
                        };
                        pool.submit(Job::Triangle {
                            st: black_box(&state).clone(),
                            p: black_box(parameters),
                            verts: [
                                (0.0, y),
                                (0.0, y + height as f32),
                                (width, y + height as f32),
                            ],
                            texcount: 0,
                        });
                    }
                    pool.flush();
                    black_box(pool.stats());
                    black_box(&state.fb);
                });
            });
        }
    }
    group.finish();
}
