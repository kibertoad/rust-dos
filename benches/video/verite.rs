//! Verite HLE triangles: filled, thin, small and textured.

use criterion::{BenchmarkId, Criterion};
use rust_dos::verite::draw::{DrawState, Vertex, fmt};
use std::hint::black_box;

pub fn triangles(c: &mut Criterion) {
    let mut group = c.benchmark_group("verite_hle_triangle");
    for (name, points, textured) in [
        ("wide", [(0.0, 0.0), (512.0, 0.0), (0.0, 512.0)], false),
        ("thin", [(0.0, 0.0), (512.0, 512.0), (510.0, 512.0)], false),
        ("flat", [(0.0, 0.0), (512.0, 1.0), (0.0, 1.0)], false),
        ("tiny", [(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)], false),
        ("textured", [(0.0, 0.0), (512.0, 0.0), (0.0, 512.0)], true),
        (
            "textured_npot",
            [(0.0, 0.0), (512.0, 0.0), (0.0, 512.0)],
            true,
        ),
    ] {
        let state = DrawState {
            dst_format: fmt::P8888,
            dst_stride: 512 * 4,
            scissor_x: 512,
            scissor_y: 512,
            tex_base: 512 * 512 * 4,
            tex_stride: 8 * 4,
            tex_last_u: if name == "textured_npot" { 6 } else { 7 },
            tex_last_v: if name == "textured_npot" { 4 } else { 7 },
            src_format: fmt::P8888,
            src_mode: if textured { 3 } else { 0 },
            filter: textured,
            ..Default::default()
        };
        let [a, b, c] = std::array::from_fn(|k| Vertex {
            x: points[k].0,
            y: points[k].1,
            r: 180.25 + k as f32,
            g: 40.5 + k as f32,
            b: 70.75 - k as f32,
            a: 200.0,
            has_colour: true,
            has_alpha: true,
            u: k as f32 * 3.0,
            v: k as f32 * 2.0,
            q: 1.0 / (k + 1) as f32,
            ..Default::default()
        });
        let mut memory: Vec<_> = (0..512 * 512 * 4 + 256)
            .map(|i| (i * 73 + i / 11) as u8)
            .collect();
        group.bench_function(BenchmarkId::from_parameter(name), |bench| {
            bench.iter(|| {
                state.triangle(
                    black_box(&mut memory),
                    black_box(&a),
                    black_box(&b),
                    black_box(&c),
                );
                black_box(&memory);
            });
        });
    }
    group.finish();
}
