//! Triangle edges prepared once, sampled independently by the workers.

use std::ops::Range;

/// X to whole pixels, with a half tie going left.
fn round_coordinate(value: f32) -> i32 {
    let result = (value as f64).floor() as i32;
    result + (value - result as f32 > 0.5) as i32
}

pub(crate) struct Triangle {
    vertices: [(f32, f32); 3],
    slopes: [f32; 3],
    pub(crate) rows: Range<i32>,
}

impl Triangle {
    pub(crate) fn new(verts: [(f32, f32); 3]) -> Option<Self> {
        // Stable Y order retains the original ties at equal vertices.
        let (mut a, mut b, mut c) = (verts[0], verts[1], verts[2]);
        if b.1 < a.1 {
            std::mem::swap(&mut a, &mut b);
        }
        if c.1 < b.1 {
            std::mem::swap(&mut b, &mut c);
            if b.1 < a.1 {
                std::mem::swap(&mut a, &mut b);
            }
        }
        let first = round_coordinate(a.1);
        let end = round_coordinate(c.1);
        if end - first <= 0 {
            return None;
        }
        let slope = |a: (f32, f32), b: (f32, f32)| {
            if b.1 == a.1 {
                0.0
            } else {
                (b.0 - a.0) / (b.1 - a.1)
            }
        };
        Some(Self {
            vertices: [a, b, c],
            slopes: [slope(a, b), slope(a, c), slope(b, c)],
            rows: first.max(-2048)..end.min(4096),
        })
    }

    /// Keep the exact original edge expressions at every row, so splitting
    /// a triangle does not introduce accumulated floating-point error.
    pub(crate) fn draw(
        &self,
        rows: impl IntoIterator<Item = i32>,
        mut scanline: impl FnMut(i32, i32, i32),
    ) {
        let [a, b, _] = self.vertices;
        let [ab, ac, bc] = self.slopes;
        for y in rows {
            let centre = y as f32 + 0.5;
            let start = a.0 + (centre - a.1) * ac;
            let stop = if centre < b.1 {
                a.0 + (centre - a.1) * ab
            } else {
                b.0 + (centre - b.1) * bc
            };
            let (mut x0, mut x1) = (round_coordinate(start), round_coordinate(stop));
            if x0 > x1 {
                std::mem::swap(&mut x0, &mut x1);
            }
            if x0 >= x1 {
                (x0, x1) = (0, 0);
            }
            scanline(y, x0, x1.min(4096));
        }
    }
}

/// Scan-convert the complete triangle for the synchronous path.
pub(crate) fn render_triangle(verts: [(f32, f32); 3], scanline: impl FnMut(i32, i32, i32)) {
    if let Some(triangle) = Triangle::new(verts) {
        triangle.draw(triangle.rows.clone(), scanline);
    }
}

#[cfg(test)]
fn reference(verts: [(f32, f32); 3], mut scanline: impl FnMut(i32, i32, i32)) {
    // Sorted by Y as the original sorts, keeping equal Ys in their order.
    let (mut v1, mut v2, mut v3) = (verts[0], verts[1], verts[2]);
    if v2.1 < v1.1 {
        std::mem::swap(&mut v1, &mut v2);
    }
    if v3.1 < v2.1 {
        std::mem::swap(&mut v2, &mut v3);
        if v2.1 < v1.1 {
            std::mem::swap(&mut v1, &mut v2);
        }
    }

    let v1y = round_coordinate(v1.1);
    let v3y = round_coordinate(v3.1);
    if v3y - v1y <= 0 {
        return;
    }
    let slope = |a: (f32, f32), b: (f32, f32)| {
        if b.1 == a.1 {
            0.0f32
        } else {
            (b.0 - a.0) / (b.1 - a.1)
        }
    };
    let dxdy_v1v2 = slope(v1, v2);
    let dxdy_v1v3 = slope(v1, v3);
    let dxdy_v2v3 = slope(v2, v3);
    // Scanlines beyond the 10-bit Y range can't be anywhere on screen.
    for y in v1y.max(-2048)..v3y.min(4096) {
        let fully = y as f32 + 0.5;
        let startx = v1.0 + (fully - v1.1) * dxdy_v1v3;
        let stopx = if fully < v2.1 {
            v1.0 + (fully - v1.1) * dxdy_v1v2
        } else {
            v2.0 + (fully - v2.1) * dxdy_v2v3
        };
        let (mut x0, mut x1) = (round_coordinate(startx), round_coordinate(stopx));
        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
        }
        if x0 >= x1 {
            (x0, x1) = (0, 0);
        }
        scanline(y, x0, x1.min(4096));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(verts: [(f32, f32); 3]) -> Vec<(i32, i32, i32)> {
        let mut out = Vec::new();
        render_triangle(verts, |y, x0, x1| out.push((y, x0, x1)));
        out
    }

    #[test]
    fn scan_conversion_samples_pixel_centres() {
        // A right triangle over a 4x4 square's lower left half.
        let s = spans([(0.0, 0.0), (0.0, 4.0), (4.0, 4.0)]);
        assert_eq!(s, vec![(0, 0, 0), (1, 0, 1), (2, 0, 2), (3, 0, 3)]);
        // Rounding: x.5 goes left.
        assert_eq!(round_coordinate(2.5), 2);
        assert_eq!(round_coordinate(2.51), 3);
        assert_eq!(round_coordinate(-0.5), -1);
    }

    #[test]
    fn flat_triangles_draw_nothing() {
        assert!(spans([(0.0, 1.0), (5.0, 1.0), (9.0, 1.2)]).is_empty());
    }

    #[test]
    fn independently_sampled_bands_match_the_original_edges_exactly() {
        let mut seed = 0x123456789ABCDEFu64;
        for case in 0..2048 {
            let mut random = || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u32
            };
            let mut vertices = std::array::from_fn(|_| {
                (
                    (random() % 32768) as f32 / 16.0 - 512.0,
                    (random() % 128000) as f32 / 16.0 - 3000.0,
                )
            });
            if case % 7 == 0 {
                vertices[2] = vertices[0];
            }
            let mut expected = Vec::new();
            reference(vertices, |y, x0, x1| expected.push((y, x0, x1)));
            assert_eq!(spans(vertices), expected);
            let mut bands = Vec::new();
            if let Some(triangle) = Triangle::new(vertices) {
                for start in triangle.rows.clone().step_by(17) {
                    triangle.draw(start..(start + 17).min(triangle.rows.end), |y, x0, x1| {
                        bands.push((y, x0, x1));
                    });
                }
            }
            assert_eq!(bands, expected, "{vertices:?}");
        }
    }
}
