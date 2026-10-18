//! Plain 2D rectangles, one row at a time. Memory wrapping, clipping,
//! sub-byte pixels and source patterns retain the pixel pipeline.

use super::*;
use std::ops::Range;

fn span(base: u32, pitch: i32, x: i32, y: i32, width: i32, bytes: usize) -> Option<Range<usize>> {
    let pixel = i64::from(base) + i64::from(y) * i64::from(pitch) + i64::from(x);
    let start = usize::try_from(pixel.checked_mul(bytes as i64)?).ok()?;
    let end = start.checked_add((width as usize).checked_mul(bytes)?)?;
    Some(start..end)
}

fn fill(dest: &mut [u8], pattern: &[u8]) {
    let mut filled = pattern.len().min(dest.len());
    dest[..filled].copy_from_slice(&pattern[..filled]);
    while filled < dest.len() {
        let count = filled.min(dest.len() - filled);
        dest.copy_within(..count, filled);
        filled += count;
    }
}

pub(super) fn draw(rect: &mut Rect, memory: &mut Memory) -> Option<bool> {
    let s = &rect.setup;
    if rect.col != 0
        || rect.empty()
        || !memory.vram.len().is_power_of_two()
        || s.mono != 0
        || s.mix_fg.mode != 7
        || s.function != 0
        || s.compare.2 & 7 != 0
        || s.dst_cntl & POLYGON != 0
        || s.src_cntl & (SRC_LINEAR | SRC_PATTERN_ON | SRC_ROTATE | SRC_TRACK_DST) != 0
        || rect.src_cols_left < rect.width
    {
        return None;
    }
    let bytes = match s.dst_size {
        Size::Byte => 1,
        Size::Word => 2,
        Size::Dword => 4,
        _ => return None,
    };
    let rotate = s.dst_cntl & ROTATE_24 != 0;
    if rotate && bytes != 1 {
        return None;
    }
    let mask = if rotate {
        0xFF_FFFF
    } else {
        u32::MAX >> (32 - bytes * 8)
    };
    if s.write_mask & mask != mask {
        return None;
    }
    let y = rect.y0.checked_add(rect.row.checked_mul(s.y_dir)?)?;
    let last = rect
        .x0
        .checked_add((rect.width - 1).checked_mul(s.x_dir)?)?;
    let left = rect.x0.min(last);
    let right = rect.x0.max(last);
    if left < 0 || y < 0 || !s.inside(left, y) || !s.inside(right, y) {
        return None;
    }
    let dest = span(s.dst_base, s.dst_pitch, left, y, rect.width, bytes)?;
    if dest.end > memory.vram.len() {
        return None;
    }
    match s.src_fg {
        SRC_BLIT => {
            if s.src_size != s.dst_size {
                return None;
            }
            let sx = rect.src_x0.checked_add(rect.src_x)?;
            let sx = if s.x_dir < 0 {
                sx.checked_sub(rect.width - 1)?
            } else {
                sx
            };
            let sy = rect.src_y0.checked_add(rect.src_y)?;
            let source = span(s.src_base, s.src_pitch, sx, sy, rect.width, bytes)?;
            if source.end > memory.vram.len() {
                return None;
            }
            if source.start < dest.end
                && dest.start < source.end
                && ((s.x_dir > 0 && dest.start > source.start)
                    || (s.x_dir < 0 && dest.start < source.start))
            {
                return None;
            }
            memory.vram.copy_within(source, dest.start);
        }
        SRC_FRGD | SRC_BKGD => {
            let value = if s.src_fg == SRC_FRGD { s.frgd } else { s.bkgd };
            let pixel = value.to_le_bytes();
            if rotate {
                let phase = if s.x_dir > 0 {
                    rect.rot0
                } else {
                    (rect.rot0 + 3 - (rect.width as u32 - 1) % 3) % 3
                } as usize;
                let pattern = std::array::from_fn::<_, 3, _>(|n| pixel[(phase + n) % 3]);
                fill(&mut memory.vram[dest], &pattern);
            } else {
                fill(&mut memory.vram[dest], &pixel[..bytes]);
            }
        }
        _ => return None,
    }
    // Both paths finish through the same source/rotation/row counters.
    // No per-pixel source wrap occurs in the eligible row.
    Some(rect.end_row())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(size: Size, source: u32, dx: i32, dy: i32, width: i32) -> Rect {
        Rect {
            setup: Setup {
                dst_size: size,
                src_size: size,
                dst_pitch: 16,
                src_pitch: 16,
                src_base: 32,
                mix_fg: Mix::new(7),
                src_fg: source,
                scissors: (0, 31, 0, 31),
                x_dir: dx,
                y_dir: dy,
                frgd: 0xA1B2_C3D4,
                bkgd: 0x1020_3040,
                write_mask: u32::MAX,
                ..Setup::default()
            },
            x0: 8,
            y0: 3,
            width,
            rows_left: 3,
            src_x0: 8,
            src_y0: 3,
            src_cols_left: ENDLESS,
            src_rows_left: ENDLESS,
            src_starts: (8, 3, 4, 2),
            widths: (ENDLESS, 4),
            heights: (ENDLESS, 2),
            rot0: 2,
            rot: 2,
            ..Rect::default()
        }
    }

    fn memory() -> Vec<u8> {
        (0..512).map(|at| (at * 73 + 19) as u8).collect()
    }

    fn scalar_row(rect: &mut Rect, memory: &mut Memory) -> bool {
        let palettes = Palettes {
            dac: &[],
            dac_8bit: false,
            texture: None,
        };
        let previous_row = rect.row;
        loop {
            let done = rect.pixel(memory, &palettes, None, 0);
            if done || rect.row != previous_row {
                return done;
            }
        }
    }

    fn compare(mut actual: Rect) -> usize {
        let mut reference = actual.clone();
        let mut pixels = memory();
        let mut expected = pixels.clone();
        let mut rows = 0;
        loop {
            let mut mem = Memory {
                vram: &mut pixels,
                lsb_first: actual.setup.lsb_first,
            };
            let done = if let Some(done) = draw(&mut actual, &mut mem) {
                rows += 1;
                done
            } else {
                scalar_row(&mut actual, &mut mem)
            };
            let lsb_first = reference.setup.lsb_first;
            let expected_done = scalar_row(
                &mut reference,
                &mut Memory {
                    vram: &mut expected,
                    lsb_first,
                },
            );
            assert_eq!(pixels, expected, "{actual:?}");
            assert_eq!(done, expected_done);
            assert_eq!(format!("{actual:?}"), format!("{reference:?}"));
            if done {
                return rows;
            }
        }
    }

    #[test]
    fn rows_preserve_pixels_and_all_walk_counters() {
        let mut rows = 0;
        for size in [Size::Byte, Size::Word, Size::Dword] {
            for source in [SRC_FRGD, SRC_BKGD, SRC_BLIT] {
                for (dx, dy) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
                    for width in [1, 2, 5] {
                        for control in [0, SRC_LINEAR, SRC_PATTERN_ON | SRC_ROTATE, SRC_TRACK_DST] {
                            for rotate in [false, true] {
                                for narrow in [false, true] {
                                    for restricted in 0..4 {
                                        let mut r = rect(size, source, dx, dy, width);
                                        r.setup.src_cntl = control;
                                        if rotate {
                                            r.setup.dst_cntl |= ROTATE_24
                                        }
                                        if narrow {
                                            r.src_cols_left = 2;
                                            r.widths = (2, 3);
                                            r.src_rows_left = 1;
                                            r.heights = (1, 2);
                                        }
                                        match restricted {
                                            0 => {}
                                            1 => r.setup.scissors = (6, 9, 2, 4),
                                            2 => r.setup.write_mask = 0x5555_5555,
                                            _ => r.setup.compare = (0x30, 0xFF, 5),
                                        }
                                        rows += compare(r);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(rows > 100);
    }

    #[test]
    fn copy_directions_and_wrapped_addresses_keep_pixel_order() {
        let mut rows = 0;
        for size in [Size::Byte, Size::Word, Size::Dword] {
            for dx in [-1, 1] {
                for sx in [0, 1, 4, 8] {
                    for x in [0, 1, 4, 8] {
                        for base in [0, 1, 127] {
                            let mut r = rect(size, SRC_BLIT, dx, 1, 5);
                            r.setup.src_base = base;
                            r.setup.dst_base = base;
                            r.x0 = x;
                            r.src_x0 = sx;
                            r.src_starts.0 = sx;
                            rows += compare(r);
                        }
                    }
                }
            }
        }
        assert!(rows > 100);
    }
}
