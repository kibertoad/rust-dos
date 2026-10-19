//! The LOD calculation over a 64-pixel affine or curved texture span.

#[path = "../../src/mach64/tri/lod.rs"]
mod implementation;

use criterion::{BenchmarkId, Criterion};
use std::hint::black_box;

fn original(steps: [i64; 4]) -> f64 {
    let scale = (1u64 << 10) as f64 / 1024.0 / 65536.0;
    let rho = steps
        .iter()
        .map(|&d| (d as f64).abs() * scale)
        .fold(0.0, f64::max);
    if rho > 1.0 { rho.log2() } else { 0.0 }
}

pub fn spans(c: &mut Criterion) {
    let mut group = c.benchmark_group("mach64_lod");
    for curved in [false, true] {
        let name = if curved { "curved" } else { "affine" };
        group.bench_function(BenchmarkId::new("original", name), |b| {
            let mut base = 1i64 << 30;
            b.iter(|| {
                base += 1;
                let mut steps = [black_box(base), 1 << 29, 1 << 28, 1 << 27];
                for _ in 0..64 {
                    black_box(original(black_box(steps)));
                    if curved {
                        steps[0] += 12345;
                    }
                }
            });
        });
        group.bench_function(BenchmarkId::new("cached", name), |b| {
            let mut cache = black_box(implementation::Lod::new(
                2 << 6,
                10 << 4,
                [curved as i64; 4],
            ));
            let mut base = 1i64 << 30;
            b.iter(|| {
                base += 1;
                let mut steps = [black_box(base), 1 << 29, 1 << 28, 1 << 27];
                cache.start(steps);
                for _ in 0..64 {
                    black_box(cache.value(black_box(steps)));
                    if curved {
                        steps[0] += 12345;
                    }
                }
            });
        });
    }
    group.finish();
}
