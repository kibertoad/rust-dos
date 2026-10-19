use super::*;

impl DrawState {
    /// A triangle.
    pub(super) fn triangle_reference(&self, vram: &mut [u8], a: &Vertex, b: &Vertex, c: &Vertex) {
        let area = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        if area == 0.0 || !area.is_finite() {
            return;
        }
        let ((left, right), (top, bottom)) = (scissor(self.scissor_x), scissor(self.scissor_y));
        let x0 = (a.x.min(b.x).min(c.x).floor() as i32).max(left);
        let x1 = (a.x.max(b.x).max(c.x).ceil() as i32).min(right);
        let y0 = (a.y.min(b.y).min(c.y).floor() as i32).max(top);
        let y1 = (a.y.max(b.y).max(c.y).ceil() as i32).min(bottom);
        let textured = self.src_mode != 0;
        let z_on = self.z_mode != 0 || self.z_write;
        // Attributes the vertices lack take the state's defaults.
        let fg = argb(self.fg);
        let alpha = self.alpha as f32;
        let fog = fixed(self.fog_default);
        let z = fixed(self.z);
        let colour_of = |v: &Vertex| {
            if v.has_colour {
                [v.r, v.g, v.b]
            } else {
                [fg[0], fg[1], fg[2]]
            }
        };
        let (ca, cb, cc) = (colour_of(a), colour_of(b), colour_of(c));
        let alpha_of = |v: &Vertex| if v.has_alpha { v.a } else { alpha };
        let fog_of = |v: &Vertex| if v.has_fog { v.f } else { fog };
        let z_of = |v: &Vertex| if v.has_z { v.z } else { z };
        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            for px in x0..x1 {
                let fx = px as f32 + 0.5;
                // Barycentric weights, with the top-left rule's ties to
                // the edges that have area on the inside.
                let w0 = ((b.x - fx) * (c.y - fy) - (c.x - fx) * (b.y - fy)) / area;
                let w1 = ((c.x - fx) * (a.y - fy) - (a.x - fx) * (c.y - fy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let blend3 = |pa: f32, pb: f32, pc: f32| w0 * pa + w1 * pb + w2 * pc;
                let lerp = |f: &dyn Fn(&Vertex) -> f32| blend3(f(a), f(b), f(c));
                // Depth: linear across the screen, kept 16.0.
                let depth = if z_on {
                    lerp(&z_of).clamp(0.0, 65535.0) as u32
                } else {
                    0
                };
                if !self.z_test(vram, px, py, depth) {
                    continue;
                }
                let colour = [0, 1, 2].map(|k| blend3(ca[k], cb[k], cc[k]));
                let alpha = lerp(&alpha_of);
                let mut pixel = [colour[0], colour[1], colour[2], alpha];
                if textured {
                    let q = lerp(&|v| v.q);
                    let (u, v) = if q != 0.0 {
                        (lerp(&|v| v.u * v.q) / q, lerp(&|v| v.v * v.q) / q)
                    } else {
                        (lerp(&|v| v.u), lerp(&|v| v.v))
                    };
                    let texel = self.sample(vram, u, v);
                    pixel = match self.src_mode {
                        // Decal: the texture over the colour by its alpha.
                        2 => {
                            let t = texel[3] / 255.0;
                            [
                                mix(colour[0], texel[0], t),
                                mix(colour[1], texel[1], t),
                                mix(colour[2], texel[2], t),
                                alpha,
                            ]
                        }
                        // Modulate.
                        3 => [0, 1, 2, 3].map(|k| pixel[k] * texel[k] / 255.0),
                        _ => texel,
                    };
                }
                if self.fog {
                    let f = (lerp(&fog_of) / 255.0).clamp(0.0, 1.0);
                    let fc = argb(self.fog_colour);
                    for k in 0..3 {
                        pixel[k] = mix(fc[k], pixel[k], f);
                    }
                }
                if self.plot(vram, px, py, pixel) && self.z_write {
                    write16(vram, self.z_at(px, py), depth);
                }
            }
        }
    }
}
