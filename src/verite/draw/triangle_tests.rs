use super::*;

fn bounds(vertices: &[Vertex; 3]) -> (i32, i32, i32, i32) {
    let [a, b, c] = vertices;
    (
        (a.x.min(b.x).min(c.x).floor() as i32).max(0),
        (a.x.max(b.x).max(c.x).ceil() as i32).min(64),
        (a.y.min(b.y).min(c.y).floor() as i32).max(0),
        (a.y.max(b.y).max(c.y).ceil() as i32).min(64),
    )
}

fn random(random: &mut u64) -> u32 {
    *random ^= *random << 13;
    *random ^= *random >> 7;
    *random ^= *random << 17;
    *random as u32
}

#[test]
fn edge_bounds_never_drop_pixels_accepted_by_the_original_weights() {
    let mut seed = 0x123456789ABCDEFu64;
    for i in 0..12000 {
        let mut vertices = std::array::from_fn(|_| Vertex {
            x: (random(&mut seed) % (128 << 16)) as f32 / 65536.0 - 32.0,
            y: (random(&mut seed) % (128 << 16)) as f32 / 65536.0 - 32.0,
            ..Default::default()
        });
        if i % 3 == 0 {
            let t = (i % 100) as f32 / 100.0;
            vertices[2].x = vertices[0].x + (vertices[1].x - vertices[0].x) * t;
            vertices[2].y =
                vertices[0].y + (vertices[1].y - vertices[0].y) * t + f32::EPSILON * (i % 7) as f32;
        }
        if i % 11 == 0 {
            vertices[0].x = -32768.0;
            vertices[1].x = 32767.999;
        }
        let [a, b, c] = &vertices;
        let area = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        let (x0, x1, y0, y1) = bounds(&vertices);
        if area == 0.0 || !area.is_finite() || x0 >= x1 || y0 >= y1 {
            continue;
        }
        let mut spans = coverage::Coverage::new([a, b, c], area, x0..x1, y0);
        for y in y0..y1 {
            let span = spans.as_mut().map_or(x0..x1, coverage::Coverage::next);
            for x in x0..x1 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b.x - fx) * (c.y - fy) - (c.x - fx) * (b.y - fy)) / area;
                let w1 = ((c.x - fx) * (a.y - fy) - (a.x - fx) * (c.y - fy)) / area;
                let w2 = 1.0 - w0 - w1;
                if !(w0 < 0.0 || w1 < 0.0 || w2 < 0.0) {
                    assert!(
                        span.contains(&x),
                        "pixel={x},{y}, span={span:?}, {vertices:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn optimized_triangles_keep_every_colour_and_depth_byte() {
    let formats = [fmt::P565, fmt::P8888, fmt::I4_1555, fmt::P4444];
    let initial: Vec<_> = (0..32768).map(|i| (i * 73 + i / 11) as u8).collect();
    let mut seed = 0xFEDCBA987654321u64;
    for option in 0u32..32 {
        let state = DrawState {
            dst_base: 0,
            dst_stride: 256,
            dst_format: formats[(option & 1) as usize],
            scissor_x: (option % 4) << 16 | 64,
            scissor_y: (option % 3) << 16 | 64,
            tex_base: 24576,
            tex_stride: 32,
            tex_last_u: if option & 16 != 0 { 6 } else { 7 },
            tex_last_v: if option & 8 != 0 { 4 } else { 7 },
            src_format: formats[(option % 4) as usize],
            src_mode: option & 3,
            clamp_u: option & 4 != 0,
            clamp_v: option & 8 != 0,
            src_bgr: option & 16 != 0,
            filter: option & 4 != 0,
            palette: (0..16).map(|i| (i * 40503) & 0xFFFF).collect(),
            z_base: 16384,
            z_stride: 128,
            z_mode: option & 7,
            z_write: option & 8 != 0,
            blend: option & 8 != 0,
            blend_src: if option & 16 != 0 { 10 } else { 2 },
            blend_dst: 3,
            chroma: option & 16 != 0,
            chroma_colour: 0,
            alpha_test: option & 16 != 0,
            alpha_threshold: 128,
            fog: option & 4 != 0,
            fog_colour: 0x123456,
            dither: option & 2 != 0,
            ..Default::default()
        };
        for shape in 0..48 {
            let vertices: [Vertex; 3] = std::array::from_fn(|k| Vertex {
                x: (random(&mut seed) % 16384) as f32 / 128.0 - 32.0,
                y: (random(&mut seed) % 16384) as f32 / 128.0 - 32.0,
                z: (50 + k * 30) as f32,
                r: 127.999 + k as f32,
                g: 200.25 - k as f32,
                b: 30.5 + k as f32,
                a: 100.0 + k as f32 * 30.0,
                f: 80.5 + k as f32 * 30.0,
                u: k as f32 * 3.125 - 2.5,
                v: k as f32 * -2.75 + 3.0,
                q: if shape % 4 == 0 { 0.0 } else { 0.25 + k as f32 },
                has_colour: k & 1 == 0,
                has_z: k & 1 == 0,
                has_alpha: k & 1 != 0,
                has_fog: k & 1 != 0,
            });
            let [a, b, c] = &vertices;
            let mut expected = initial.clone();
            let mut actual = initial.clone();
            state.triangle_reference(&mut expected, a, b, c);
            state.triangle(&mut actual, a, b, c);
            assert_eq!(
                actual, expected,
                "option={option}, shape={shape}, {vertices:?}"
            );
        }
    }
}

#[test]
fn tiny_degenerate_large_and_shared_edges_keep_their_old_coverage() {
    let state = DrawState {
        dst_stride: 256,
        dst_format: fmt::P8888,
        scissor_x: 64,
        scissor_y: 64,
        blend: true,
        blend_src: 7,
        blend_dst: 7,
        fg: 0xFF101010,
        ..Default::default()
    };
    let vertex = |x, y| Vertex {
        x,
        y,
        q: 1.0,
        ..Default::default()
    };
    for points in [
        [(0.0, 0.0), (64.0, 0.0), (64.0, 64.0)],
        [(0.0, 0.0), (64.0, 64.0), (0.0, 64.0)],
        [(0.5, 0.5), (1.5, 1.5), (0.5, 1.5)],
        [(0.0, 0.0), (64.0, 64.0), (32.0, 32.0)],
        [(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
        [(1e30, 0.0), (1e30, 1.0), (0.0, 1.0)],
        [(1e20, 1e20), (0.0, 0.0), (1e20, 1e20 + 1e13)],
        [(0.0, 0.0), (f32::INFINITY, 0.0), (0.0, 64.0)],
        [(0.0, 0.0), (f32::NAN, 0.0), (0.0, 64.0)],
    ] {
        let vertices = points.map(|(x, y)| vertex(x, y));
        let [a, b, c] = &vertices;
        let mut expected = vec![0x55u8; 32768];
        let mut actual = expected.clone();
        for (a, b, c) in [(a, b, c), (c, b, a)] {
            state.triangle_reference(&mut expected, a, b, c);
            state.triangle(&mut actual, a, b, c);
        }
        assert_eq!(actual, expected, "{points:?}");
    }
}

#[test]
fn power_of_two_masks_match_signed_euclidean_texture_wrapping() {
    for last in [
        0,
        1,
        2,
        3,
        7,
        8,
        15,
        16,
        255,
        256,
        1023,
        65535,
        0x7FFF_FFFE,
        0x8000_0000,
    ] {
        for value in (-1024..1024).chain([i32::MIN, i32::MIN + 1, i32::MAX - 1, i32::MAX]) {
            assert_eq!(
                wrapped_coordinate(value, last),
                value.rem_euclid(last as i32 + 1) as u32,
                "value={value}, last={last}"
            );
        }
    }
}
