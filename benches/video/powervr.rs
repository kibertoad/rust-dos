//! PowerVR's CPU render cost with fresh or persistent host resources.

use criterion::{BenchmarkId, Criterion, Throughput};
use rust_dos::powervr::render::{Memory, Renderer};
use rust_dos::powervr::tsp::Shader;
use rust_dos::powervr::{Chip, regs};
use std::hint::black_box;

const TILE_HEADER: u32 = 1 << 30;
const VERY_LAST: u32 = 1 << 31;

fn scene(
    chip: Chip,
    count: usize,
    height: u32,
    depth: f32,
    color: u32,
) -> (Vec<u32>, Vec<u8>, Vec<u8>) {
    let mut regs = vec![0; regs::COUNT];
    regs[regs::TLB + 1] = 1;
    regs[regs::OBJECT_OFFSET] = 0x1001;
    regs[regs::IEEEFP] = u32::from(chip == Chip::Pcx2);
    let mut ram = vec![0; 0x4000];
    let mut put = |at: usize, value: u32| ram[at..at + 4].copy_from_slice(&value.to_le_bytes());
    let plane = if chip == Chip::Pcx2 {
        vec![0, 0, depth.to_bits(), 8 | 4 << 4]
    } else {
        vec![
            8 << 26,
            4 << 20,
            (depth as f64 * 2_147_483_648.0) as i32 as u32,
        ]
    };
    for (i, word) in plane.into_iter().enumerate() {
        put(0x100 + i * 4, word);
    }
    for i in 0..count {
        put(
            0x1000 + i * 8,
            TILE_HEADER
                | (height - 1) << 5
                | (i as u32 % 20) << 15
                | (i as u32 / 20 * height) << 20,
        );
        put(
            0x1004 + i * 8,
            1 << 19 | 0x100 >> 2 | if i + 1 == count { VERY_LAST } else { 0 },
        );
    }
    let mut tex = vec![0; 64];
    tex[32..36].copy_from_slice(&(0x2000_0000 | (color >> 16)).to_le_bytes());
    tex[36..40].copy_from_slice(&((color & 0xFFFF) << 16).to_le_bytes());
    (regs, ram, tex)
}

/// A flat-shaded 640x480 scene of 300 independent 32x32 tiles. Parsing,
/// pixel shading and host-resource management are timed; fixture creation
/// and the bus's final pixel-format conversion/copy-out are excluded.
pub fn frames(c: &mut Criterion) {
    let (regs, ram, textures) = scene(Chip::Pcx2, 300, 32, 0.5, 0x234567);
    let shader = Shader::default();
    let mut group = c.benchmark_group("video/powervr/frame");
    group.throughput(Throughput::Elements(640 * 480));
    for threads in [1, 4] {
        let mut renderer = Renderer::with_threads(threads);
        let initial = renderer.render(Chip::Pcx2, &regs, &textures, Memory { ram: &ram }, &shader);
        assert_eq!(initial.tiles.len(), 300);
        assert_eq!(initial.tsp_clocks, 640 * 480);
        assert!(
            initial
                .tiles
                .iter()
                .all(|tile| tile.pixels.iter().all(|&p| p == 0x234567))
        );
        renderer.recycle(initial);
        group.bench_function(BenchmarkId::new("reused", threads), |b| {
            b.iter(|| {
                let frame =
                    renderer.render(Chip::Pcx2, &regs, &textures, Memory { ram: &ram }, &shader);
                black_box(frame.tiles[0].pixels[0]);
                renderer.recycle(frame);
            });
        });
        group.bench_with_input(
            BenchmarkId::new("fresh", threads),
            &threads,
            |b, &threads| {
                b.iter(|| {
                    let mut renderer = Renderer::with_threads(threads);
                    let frame = renderer.render(
                        Chip::Pcx2,
                        &regs,
                        &textures,
                        Memory { ram: &ram },
                        &shader,
                    );
                    black_box(frame.tiles[0].pixels[0]);
                });
            },
        );
    }
    group.finish();
}
