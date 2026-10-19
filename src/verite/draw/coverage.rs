use super::Vertex;
use std::ops::Range;

/// Conservative scanline bounds. Pixels in these bounds still use the
/// original f32 weights, retaining its coverage and interpolation ties.
pub(super) struct Coverage {
    columns: Range<i32>,
    values: [f64; 3],
    dx: [f64; 3],
    dy: [f64; 3],
    tolerance: f64,
}

impl Coverage {
    pub(super) fn new(
        vertices: [&Vertex; 3],
        area: f32,
        columns: Range<i32>,
        y: i32,
    ) -> Option<Self> {
        let [a, b, c] = vertices.map(|v| [v.x as f64, v.y as f64]);
        let extent = vertices.iter().fold(1.0f64, |m, v| {
            m.max((v.x as f64).abs()).max((v.y as f64).abs())
        }) + 1.0;
        if extent > 32769.0 {
            return None;
        }
        let sign = if area > 0.0 { 1.0 } else { -1.0 };
        let (x, y) = (columns.start as f64 + 0.5, y as f64 + 0.5);
        let n0 = sign * ((b[0] - x) * (c[1] - y) - (c[0] - x) * (b[1] - y));
        let n1 = sign * ((c[0] - x) * (a[1] - y) - (a[0] - x) * (c[1] - y));
        let x0 = sign * (b[1] - c[1]);
        let x1 = sign * (c[1] - a[1]);
        let y0 = sign * (c[0] - b[0]);
        let y1 = sign * (a[0] - c[0]);
        Some(Self {
            columns,
            values: [n0, n1, (area as f64).abs() - n0 - n1],
            dx: [x0, x1, -x0 - x1],
            dy: [y0, y1, -y0 - y1],
            // Each f32 edge uses two subtractions per product, two
            // products and a subtraction. With all coordinates bounded
            // by extent, 256 eps * extent^2 exceeds both edges' combined
            // rounding errors. The area term covers division and the
            // two subtractions forming 1 - w0 - w1. f64 stepping errors
            // over the 12-bit scissor range fit well within this bound.
            tolerance: 256.0 * f32::EPSILON as f64 * extent * extent
                + 16.0 * f32::EPSILON as f64 * (area as f64).abs(),
        })
    }

    pub(super) fn next(&mut self) -> Range<i32> {
        let length = (self.columns.end - self.columns.start) as f64;
        let (mut first, mut last) = (0.0f64, length - 1.0);
        for edge in 0..3 {
            let value = self.values[edge];
            self.values[edge] += self.dy[edge];
            let step = self.dx[edge];
            if step > 0.0 {
                first = first.max((-self.tolerance - value) / step);
            } else if step < 0.0 {
                last = last.min((-self.tolerance - value) / step);
            } else if value < -self.tolerance {
                // Advance all edges even when the row is empty.
                first = length;
                last = -1.0;
            }
        }
        // Include one extra pixel at each boundary for f64 intersection
        // rounding. Only the original f32 tests decide final coverage.
        let first = (first.ceil() - 1.0).clamp(0.0, length) as i32;
        let last = (last.floor() + 2.0).clamp(0.0, length) as i32;
        self.columns.start + first..self.columns.start + last
    }
}
