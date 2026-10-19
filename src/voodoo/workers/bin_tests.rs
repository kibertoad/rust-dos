//! Compare binned scheduling with the original per-worker traversal.

use super::*;
use crate::voodoo::raster::Stride;

fn state(fb: Vram, flip: bool, mask: i32, tiled: bool) -> RasterState {
    let stride = if tiled {
        Stride {
            words: 2048,
            tiled: true,
        }
    } else {
        Stride::linear(64)
    };
    RasterState {
        fb,
        fbz_color_path: 0,
        fbz_mode: 1 | 3 << 9 | 7 << 5 | if flip { 1 << 17 } else { 0 },
        alpha_mode: 0,
        fog_mode: 0,
        za_color: 0x1234,
        chroma_key: 0,
        color0: 0,
        color1: 0,
        fog_color: 0,
        clip_left_right: 64,
        clip_low_y_high_y: 192,
        clip_mask: 0xFFF,
        chroma_range: 0,
        fogdelta_mask: 0xFC,
        yorigin: 191,
        ymask: mask,
        rowpixels: 64,
        stride,
        aux_stride: stride,
        dest: 0,
        aux: Some(65536),
        fogblend: [0; 64],
        fogdelta: [0; 64],
        send_config: None,
        stipple: 0x5A5A_1234,
        tmu: [None, None],
    }
}

fn jobs(st: &RasterState) -> Vec<Job> {
    let mut jobs = Vec::new();
    for i in 0..270 {
        let y = (i * 17 % 192) as i32 - 16;
        let mut st = st.clone();
        if i % 3 == 0 {
            st.fbz_mode |= 1 << 2 | 1 << 12;
        }
        // Reach the automatic 128-job publication before exercising the
        // immediate publication of frame buffer writes.
        match if i < 140 { 2 } else { i % 7 } {
            0 => jobs.push(Job::Fastfill {
                st,
                dither: [i as u16 * 117; 16],
                x0: 3,
                x1: 57,
                y0: y,
                y1: y + 69,
            }),
            1 => jobs.push(Job::Pixels {
                fb: st.fb.clone(),
                words: (0..128)
                    .map(|word| {
                        let row = (i * 13 + word * 17) % 192;
                        Word {
                            row: row as u16,
                            at: (st.stride.row(row) + word % 64) as u32,
                            value: (i * 191 + word) as u16,
                        }
                    })
                    .chain([Word {
                        row: 65535,
                        at: u32::MAX,
                        value: 17,
                    }])
                    .collect(),
            }),
            _ => jobs.push(Job::Triangle {
                st,
                p: TriParams {
                    startr: (i as i32 % 250) << 12,
                    startg: 80 << 12,
                    startb: 140 << 12,
                    starta: 255 << 12,
                    startz: 100 << 12,
                    startw: 1 << 32,
                    drdx: 17,
                    dgdy: 13,
                    ..Default::default()
                },
                verts: [
                    (3.25, y as f32),
                    (58.5, y as f32 + 127.0),
                    (7.0, y as f32 + 63.5),
                ],
                texcount: 0,
            }),
        }
    }
    jobs
}

#[test]
fn mixed_batches_preserve_framebuffer_words_and_counters_for_each_worker_count() {
    for (flip, mask, tiled) in [
        (false, 1023, false),
        (true, 1023, false),
        (true, -1, false),
        (false, 1023, true),
        (true, 1023, true),
    ] {
        for workers in [1, 2, 4, 7] {
            let reference = Vram::new(262144);
            let input = jobs(&state(reference.clone(), flip, mask, tiled));
            let mut expected = Stats::default();
            // The old worker implementation, with its original ownership
            // rule, is the reference, including off-screen counters.
            for k in 0..workers {
                for job in &input {
                    job.run(
                        |row| row >= 0 && row as usize % workers == k,
                        &mut 0,
                        &mut expected,
                    );
                }
            }
            let output = Vram::new(262144);
            let pool = Pool::new(workers);
            for job in jobs(&state(output.clone(), flip, mask, tiled)) {
                pool.submit(job);
            }
            pool.flush();
            assert_eq!(
                pool.stats(),
                expected,
                "workers={workers}, flip={flip}, mask={mask}"
            );
            assert!(expected.pixels_out > 0);
            let difference = (0..output.len()).find(|&i| output.get(i) != reference.get(i));
            assert_eq!(
                difference, None,
                "workers={workers}, flip={flip}, tiled={tiled}"
            );
            assert!(!pool.busy());
        }
    }
}
