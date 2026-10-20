//! Variant selection for full, combine-only and unmatched pipelines.

use crate::voodoo::two_unit_state;
use criterion::{BenchmarkId, Criterion};
use rust_dos::voodoo::raster;
use std::hint::black_box;

pub fn lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("voodoo_variant_lookup");
    for (name, count, regs) in [
        (
            "first_full",
            2,
            [0x90110, 0x0D420009, 0x40400, 0, 0x8C261A0F, 0x84221C0F],
        ),
        (
            "last_full",
            2,
            [0x90110, 0x0D420009, 0x6010, 0, 0x8C26130F, 0x8422130F],
        ),
        ("untextured", 0, [0x90110, 0x0542610A, 0x45110, 0, 0, 0]),
        (
            "combine",
            2,
            [0x843E1, 0x0D422409, 0x5110, 0xC0, 0x80000006, 0x8C261ACF],
        ),
        (
            "last_combine",
            2,
            [0x843E1, 0x0C482405, 0x5110, 0xC0, 0x80000006, 0x8C261C0F],
        ),
        ("generic", 2, [0; 6]),
        ("one_unit", 1, [0; 6]),
    ] {
        let state = two_unit_state(regs);
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| black_box(raster::variant(black_box(&state), black_box(count))));
        });
    }
    group.finish();
}
