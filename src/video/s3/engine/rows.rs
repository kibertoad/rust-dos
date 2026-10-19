//! Row operations for plain copies and solid fills. Rows are trimmed to
//! the scissors first, since the engine draws nothing outside them.
//! Anything whose mask, memory bounds or copy direction needs pixels uses
//! the engine.

use super::{Engine, Surface};
use std::ops::Range;

impl Surface<'_> {
    /// The bytes of `count` pixels from `left` rightwards on row `y`, when
    /// all of them are on that row and in video memory.
    fn row_range(&self, left: i32, y: i32, count: u32) -> Option<Range<usize>> {
        let right = left + count as i32 - 1;
        if count == 0 || y < 0 || left < 0 || right as u32 >= self.width {
            return None;
        }
        let start = self.at(left as u32, y as u32)?;
        let end = self.at(right as u32, y as u32)? + self.bytes as usize;
        Some(start..end)
    }
}

impl Engine {
    pub(super) fn can_write_rows(&self, surface: &Surface) -> bool {
        let mask = match surface.bytes {
            1..=4 => u32::MAX >> (32 - 8 * surface.bytes),
            _ => return false,
        };
        self.writes() && self.write_mask & mask == mask
    }

    /// The part of a row of `count` pixels from `x` in direction `dx`
    /// that is inside the scissors: its leftmost pixel and its length,
    /// zero when the scissors leave none of it.
    fn clip_row(&self, x: i32, y: i32, count: u32, dx: i32) -> (i32, u32) {
        let last = x + (count as i32 - 1) * dx;
        let [left, top, right, bottom] = self.clip.map(i32::from);
        let from = x.min(last).max(left);
        let to = x.max(last).min(right);
        if y < top || y > bottom || from > to {
            return (from, 0);
        }
        (from, (to - from + 1) as u32)
    }

    pub(super) fn fill_row(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        count: u32,
        dx: i32,
        value: u32,
    ) -> bool {
        let (left, count) = self.clip_row(x, y, count, dx);
        if count == 0 {
            return true;
        }
        let Some(row) = surface.row_range(left, y, count) else {
            return false;
        };
        let pixel = value.to_le_bytes();
        let pixel = &pixel[..surface.bytes as usize];
        let dest = &mut surface.vram[row];
        if pixel.iter().all(|&byte| byte == pixel[0]) {
            dest.fill(pixel[0]);
        } else {
            dest[..pixel.len()].copy_from_slice(pixel);
            let mut filled = pixel.len();
            while filled < dest.len() {
                let count = filled.min(dest.len() - filled);
                dest.copy_within(..count, filled);
                filled += count;
            }
        }
        true
    }

    pub(super) fn copy_row(
        &self,
        surface: &mut Surface,
        src: (i32, i32),
        dest: (i32, i32),
        count: u32,
        dx: i32,
    ) -> bool {
        // Source pixels whose destination is outside the scissors are read
        // but never written, so only the trimmed part matters.
        let (left, count) = self.clip_row(dest.0, dest.1, count, dx);
        if count == 0 {
            return true;
        }
        let Some(source) = surface.row_range(left + src.0 - dest.0, src.1, count) else {
            return false;
        };
        let Some(dest) = surface.row_range(left, dest.1, count) else {
            return false;
        };
        let overlaps = source.start < dest.end && dest.start < source.end;
        if overlaps
            && ((dx > 0 && dest.start > source.start) || (dx < 0 && dest.start < source.start))
        {
            // Hardware follows the requested direction, even when writes
            // destroy source pixels. A memmove would change that result.
            return false;
        }
        surface.vram.copy_within(source, dest.start);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(bytes: u32) -> Vec<u8> {
        (0..16 * 6 * bytes)
            .map(|at| (at.wrapping_mul(73).wrapping_add(19)) as u8)
            .collect()
    }

    #[test]
    fn fills_match_pixels_with_clips_masks_directions_and_partial_rows() {
        for bytes in 1..=4 {
            for direction in [0, 0x20, 0x80, 0xA0] {
                for (x, y) in [(0, 0), (2, 2), (14, 4), (18, 2)] {
                    for major in [0, 3, 16] {
                        for clip in [[0, 0, 0xFFF, 0xFFF], [1, 1, 14, 3], [3, 0, 9, 5]] {
                            for mask in [u32::MAX, 0x00FF_FFFF, 0x5555_5555] {
                                for mode in [0x27, 0x25, 0x07, 0x67] {
                                    for write in [0x11, 0x10] {
                                        for omit in [0, 4] {
                                            let mut actual = memory(bytes);
                                            let mut expected = actual.clone();
                                            let mut engine = Engine {
                                                clip,
                                                command: write,
                                                write_mask: mask,
                                                fore: 0xAABB_CCDD,
                                                back: 0x5151_5151,
                                                fore_mix: mode,
                                                cur_x: x,
                                                cur_y: y,
                                                major,
                                                minor: 2,
                                                ..Engine::new()
                                            };
                                            let value = direction | omit;
                                            let dx = if direction & 0x20 != 0 { 1 } else { -1 };
                                            let dy = if direction & 0x80 != 0 { 1 } else { -1 };
                                            let mut end = (x as i32, y as i32);
                                            if omit == 0 || major != 0 {
                                                let count = major as i32 + 1 - i32::from(omit != 0);
                                                let (mode, src) = engine.solid_source();
                                                let mut surface = Surface {
                                                    vram: &mut expected,
                                                    width: 16,
                                                    bytes,
                                                };
                                                for row in 0..3 {
                                                    for col in 0..count {
                                                        engine.mix_point(
                                                            &mut surface,
                                                            (x as i32 + dx * col) as u32,
                                                            (y as i32 + dy * row) as u32,
                                                            mode,
                                                            src,
                                                        );
                                                    }
                                                }
                                                end = (x as i32 + dx * count, y as i32 + dy * 3);
                                            }
                                            engine.rectangle(
                                                value,
                                                &mut Surface {
                                                    vram: &mut actual,
                                                    width: 16,
                                                    bytes,
                                                },
                                            );
                                            assert_eq!(actual, expected, "fill {engine:?}");
                                            assert_eq!(
                                                (engine.cur_x, engine.cur_y),
                                                (end.0 as u16, end.1 as u16)
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn copies_preserve_destructive_overlap_and_pixel_fallbacks() {
        for bytes in 1..=4 {
            for direction in [0, 0x20, 0x80, 0xA0] {
                for (sx, sy) in [(2, 2), (6, 2), (14, 4), (0, 0)] {
                    for (tx, ty) in [(3, 2), (1, 2), (2, 3), (2, 1), (12, 4)] {
                        for clip in [[0, 0, 0xFFF, 0xFFF], [1, 1, 14, 3], [3, 0, 9, 5]] {
                            for mask in [u32::MAX, 0x00FF_FFFF, 0x5555_5555] {
                                for select in [0, 3] {
                                    for misc in [0, 0x100, 0x180] {
                                        for mode in [0x67, 0x65, 0x27] {
                                            let mut actual = memory(bytes);
                                            let mut expected = actual.clone();
                                            let mut engine = Engine {
                                                clip,
                                                command: 0x11,
                                                write_mask: mask,
                                                read_mask: 0xFF,
                                                color_compare: 0x93,
                                                fore: 0xAABB_CCDD,
                                                fore_mix: mode,
                                                back_mix: 0x05,
                                                cur_x: sx,
                                                cur_y: sy,
                                                dest_x: tx,
                                                dest_y: ty,
                                                major: 3,
                                                minor: 2,
                                                pix_cntl: select << 6,
                                                misc,
                                                ..Engine::new()
                                            };
                                            let dx = if direction & 0x20 != 0 { 1 } else { -1 };
                                            let dy = if direction & 0x80 != 0 { 1 } else { -1 };
                                            let mut surface = Surface {
                                                vram: &mut expected,
                                                width: 16,
                                                bytes,
                                            };
                                            let compare = engine.color_compare & surface.mask();
                                            for row in 0..3 {
                                                for col in 0..4 {
                                                    let pixel = surface.get(
                                                        (sx as i32 + dx * col) as u32,
                                                        (sy as i32 + dy * row) as u32,
                                                    );
                                                    let mode = if select == 3 {
                                                        engine.screen_mix(pixel)
                                                    } else {
                                                        engine.fore_mix
                                                    };
                                                    let src = engine.source(mode, pixel);
                                                    let write = misc & 0x100 == 0
                                                        || ((src != compare) as u16
                                                            ^ (misc >> 7 & 1))
                                                            != 0;
                                                    if write {
                                                        engine.mix_point(
                                                            &mut surface,
                                                            (tx as i32 + dx * col) as u32,
                                                            (ty as i32 + dy * row) as u32,
                                                            mode,
                                                            src,
                                                        );
                                                    }
                                                }
                                            }
                                            engine.blit(
                                                direction,
                                                &mut Surface {
                                                    vram: &mut actual,
                                                    width: 16,
                                                    bytes,
                                                },
                                            );
                                            assert_eq!(actual, expected, "copy {engine:?}");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
