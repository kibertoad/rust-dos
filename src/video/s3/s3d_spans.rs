use super::{Attr, Context};

pub(super) type Span<'a, 'b> = fn(&mut Context<'a, 'b>, i32, i32, i32, i32, &mut Attr, &Attr);

impl<'a, 'b> Context<'a, 'b> {
    pub(super) fn select_span(&self) -> Span<'a, 'b> {
        match (self.ztest, self.textured, self.bypp) {
            (false, false, 1) => Self::span::<false, false, 1>,
            (false, false, 2) => Self::span::<false, false, 2>,
            (false, false, 3) => Self::span::<false, false, 3>,
            (false, true, 1) => Self::span::<false, true, 1>,
            (false, true, 2) => Self::span::<false, true, 2>,
            (false, true, 3) => Self::span::<false, true, 3>,
            (true, false, 1) => Self::span::<true, false, 1>,
            (true, false, 2) => Self::span::<true, false, 2>,
            (true, false, 3) => Self::span::<true, false, 3>,
            (true, true, 1) => Self::span::<true, true, 1>,
            (true, true, 2) => Self::span::<true, true, 2>,
            (true, true, 3) => Self::span::<true, true, 3>,
            _ => unreachable!("destination format checked by Context::new"),
        }
    }

    fn span<const ZTEST: bool, const TEXTURED: bool, const BYPP: u32>(
        &mut self,
        mut x: i32,
        y: i32,
        count: i32,
        direction: i32,
        at: &mut Attr,
        step: &Attr,
    ) {
        for _ in 0..count {
            self.pixel_in::<ZTEST, TEXTURED, BYPP>(x, y, at);
            at.add(step);
            x += direction;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    fn attributes(at: &Attr) -> [i32; 9] {
        [at.r, at.g, at.b, at.a, at.z, at.u, at.v, at.w, at.d]
    }

    fn run(vram: &mut [u8], cmd: u32, reverse: bool, specialized: bool) -> [i32; 9] {
        let mut target = Target {
            vram,
            palette: &|i| (i, i.wrapping_mul(3), i.wrapping_mul(7)),
        };
        let mut regs = [0; 0x80];
        regs[0xD4 >> 2] = 1024;
        regs[0xD8 >> 2] = 8;
        regs[0xDC >> 2] = 2 << 16 | 6;
        regs[0xE0 >> 2] = 1 << 16 | 2;
        regs[0xE4 >> 2] = 64 << 16 | 16;
        regs[0xE8 >> 2] = 64;
        regs[0xEC >> 2] = 1536;
        regs[0xF4 >> 2] = 0x123456;
        regs[0xF8 >> 2] = 0xAABBCC;
        regs[0xFC >> 2] = 0x112233;
        let mut c = Context::new(&mut target, cmd, &regs).unwrap();
        assert!(c.setup_triangle(cmd, &regs));
        let mut at = Attr {
            r: 200 << 7,
            g: 40 << 7,
            b: 80 << 7,
            a: 128 << 7,
            z: 10 << 15,
            u: -100,
            v: 1000,
            w: 1 << 20,
            d: 128 << 19,
        };
        let step = Attr {
            r: -100,
            g: 150,
            b: -50,
            a: 200,
            z: 1 << 15,
            u: 1 << 26,
            v: -100,
            w: 17,
            d: 1 << 17,
        };
        let direction = if reverse { -1 } else { 1 };
        let start = if reverse { 9 } else { -2 };
        if specialized {
            c.select_span()(&mut c, start, 2, 12, direction, &mut at, &step);
        } else {
            for i in 0..12 {
                c.pixel(start + i * direction, 2, &at);
                at.add(&step);
            }
        }
        attributes(&at)
    }

    #[test]
    fn every_span_variant_matches_dynamic_pixels() {
        // A xorshift stream, so that each command field varies on its own
        // and texture format, filter, Z compare and the flags combine freely.
        let mut seed = 0x2545_F491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for format in 0..3 {
            for command in [
                GOURAUD_TRI,
                LIT_TEX_TRI,
                LIT_TEX_TRI_PERSP,
                UNLIT_TEX_TRI,
                UNLIT_TEX_TRI_PERSP,
            ] {
                for mode in 0..4 {
                    for _ in 0..32 {
                        let r = next();
                        let cmd = CMD_3D
                            | format << 2
                            | (r & 7) << 5
                            | 1 << 8
                            | (r >> 3 & 7) << 12
                            | ((r >> 6 & 3) % 3) << 15
                            | (r >> 8 & 3) << 18
                            | (r >> 10 & 7) << 20
                            | mode << 24
                            | command << 27
                            | if r & 1 << 13 != 0 { CMD_HC } else { 0 }
                            | if r & 1 << 14 != 0 { CMD_FE } else { 0 }
                            | if r & 1 << 15 != 0 { CMD_ZUP } else { 0 }
                            | if r & 1 << 16 != 0 { CMD_TWE } else { 0 };
                        for reverse in [false, true] {
                            let mut expected: Vec<_> =
                                (0..2048).map(|i| (i * 73 + 19) as u8).collect();
                            let mut actual = expected.clone();
                            let scalar = run(&mut expected, cmd, reverse, false);
                            let span = run(&mut actual, cmd, reverse, true);
                            assert_eq!(actual, expected, "cmd={cmd:#x}, reverse={reverse}");
                            assert_eq!(span, scalar, "cmd={cmd:#x}, reverse={reverse}");
                        }
                    }
                }
            }
        }
    }
}
