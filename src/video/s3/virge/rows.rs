//! Whole rows when the command needs no pixel pipeline. Validate all
//! ranges first, so a rejected operation has not changed video memory.

use super::Group;
use std::ops::Range;

fn row(base: u32, stride: u32, x: u32, y: u32, bytes: u32, width: u32) -> Option<Range<usize>> {
    let start = base
        .checked_add(y.checked_mul(stride)?)?
        .checked_add(x.checked_mul(bytes)?)?;
    let end = start.checked_add(width.checked_mul(bytes)?)?;
    Some(start as usize..end as usize)
}

fn stepped(start: u32, step: u32, positive: bool) -> Option<u32> {
    if positive {
        start.checked_add(step)
    } else {
        start.checked_sub(step)
    }
}

/// Row `y` of the copy, `width` pixels from columns `sx` and `dx`.
fn copy_rows(
    g: &Group,
    sx: u32,
    dx: u32,
    width: u32,
    y: u32,
) -> Option<(Range<usize>, Range<usize>)> {
    let positive_y = g.command_set & (1 << 26) != 0;
    let sy = stepped(g.rect_src_y, y, positive_y)?;
    let dy = stepped(g.rect_dst_y, y, positive_y)?;
    let source = row(g.src_base, g.src_stride, sx, sy, g.bypp(), width)?;
    let dest = row(g.dst_base, g.dst_stride, dx, dy, g.bypp(), width)?;
    Some((source, dest))
}

pub(super) fn copy(g: &Group, vram: &mut [u8]) -> bool {
    if g.rop() != 0xCC
        || g.command_set & 0x20 == 0
        || g.bypp() == 0
        || g.rect_width == 0
        || g.rect_height == 0
        || g.rect_width
            .checked_mul(g.rect_height)
            .is_none_or(|n| n > 1 << 22)
    {
        return false;
    }
    let positive_x = g.command_set & (1 << 25) != 0;
    let positive_y = g.command_set & (1 << 26) != 0;
    // A right to left copy starts at the rectangle's right edge.
    let offset = if positive_x { 0 } else { g.rect_width - 1 };
    let (Some(mut sx), Some(mut dx)) = (
        g.rect_src_x.checked_sub(offset),
        g.rect_dst_x.checked_sub(offset),
    ) else {
        return false;
    };
    let Some(mut right) = dx.checked_add(g.rect_width - 1) else {
        return false;
    };
    // Rows run from 0 to rect_height - 1 in command order; with clipping
    // on, those outside the clip rectangle are skipped.
    let (mut first, mut last) = (0, g.rect_height - 1);
    if g.command_set & 2 != 0 {
        // The pixel path writes only inside the clip rectangle and in the
        // same order, so the copy shrinks to that part of each row.
        let left = dx.max(g.left_clip);
        right = right.min(g.right_clip);
        if left > right {
            return true;
        }
        let Some(shifted) = sx.checked_add(left - dx) else {
            return false;
        };
        (sx, dx) = (shifted, left);
        let Some(end) = stepped(g.rect_dst_y, last, positive_y) else {
            return false;
        };
        let (top, bottom) = if positive_y {
            (g.rect_dst_y, end)
        } else {
            (end, g.rect_dst_y)
        };
        let (top, bottom) = (top.max(g.top_clip), bottom.min(g.bottom_clip));
        if top > bottom {
            return true;
        }
        (first, last) = if positive_y {
            (top - g.rect_dst_y, bottom - g.rect_dst_y)
        } else {
            (g.rect_dst_y - bottom, g.rect_dst_y - top)
        };
    }
    let width = right - dx + 1;
    for y in first..=last {
        let Some((source, dest)) = copy_rows(g, sx, dx, width, y) else {
            return false;
        };
        if source.end > vram.len() || dest.end > vram.len() {
            return false;
        }
        let overlaps = source.start < dest.end && dest.start < source.end;
        if overlaps
            && ((positive_x && dest.start > source.start)
                || (!positive_x && dest.start < source.start))
        {
            return false;
        }
    }
    for y in first..=last {
        let (source, dest) = copy_rows(g, sx, dx, width, y).unwrap();
        vram.copy_within(source, dest.start);
    }
    true
}

pub(super) fn fill(g: &Group, vram: &mut [u8], [left, right, top, bottom]: [u32; 4]) -> bool {
    if g.rop() != 0xF0
        || g.command_set & 0x20 == 0
        || g.bypp() == 0
        || (g.command_set & 0x200 != 0 && g.mono_pat != u64::MAX)
        || right < left
        || bottom < top
    {
        return false;
    }
    let Some(width) = right.checked_sub(left).and_then(|n| n.checked_add(1)) else {
        return false;
    };
    for y in top..=bottom {
        let Some(range) = row(g.dst_base, g.dst_stride, left, y, g.bypp(), width) else {
            return false;
        };
        if range.end > vram.len() {
            return false;
        }
    }
    let pixel = g.mono_pat_fg.to_le_bytes();
    let pixel = &pixel[..g.bypp() as usize];
    let same_bytes = pixel.iter().all(|&byte| byte == pixel[0]);
    for y in top..=bottom {
        let range = row(g.dst_base, g.dst_stride, left, y, g.bypp(), width).unwrap();
        let dest = &mut vram[range];
        if same_bytes {
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
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::s3::virge::{BITBLT, Virge, draw, draw_clipped, draw_rect, read_src};

    fn memory() -> Vec<u8> {
        (0..251).map(|i| (i * 73 + 19) as u8).collect()
    }

    fn scalar_copy(g: &Group, vram: &mut [u8]) {
        for row in 0..g.rect_height {
            let Some(y) = stepped(g.rect_dst_y, row, g.command_set & (1 << 26) != 0) else {
                break;
            };
            let sy = if g.command_set & (1 << 26) != 0 {
                g.rect_src_y.wrapping_add(row)
            } else {
                g.rect_src_y.wrapping_sub(row)
            };
            for col in 0..g.rect_width {
                let Some(x) = stepped(g.rect_dst_x, col, g.command_set & (1 << 25) != 0) else {
                    break;
                };
                let sx = if g.command_set & (1 << 25) != 0 {
                    g.rect_src_x.wrapping_add(col)
                } else {
                    g.rect_src_x.wrapping_sub(col)
                };
                let value = read_src(g, vram, sx, sy);
                draw_clipped(g, vram, x, y, value);
            }
        }
    }

    #[test]
    fn row_copies_match_pixel_order_with_overlap_and_memory_edges() {
        let (mut used_rows, mut used_clipped) = (0, 0);
        for format in 0..=2 {
            for direction in [0, 1 << 25, 1 << 26, 3 << 25] {
                for (sx, sy) in [(2, 2), (6, 2), (14, 4), (0, 0)] {
                    for (dx, dy) in [(3, 2), (1, 2), (2, 3), (2, 1), (12, 4)] {
                        for (src_base, dst_base) in [(0, 0), (1, 17), (u32::MAX - 7, 0)] {
                            for clip in [0, 2] {
                                for strides in [(16, 16), (11, 16), (0, 0)] {
                                    let g = Group {
                                        command_set: direction
                                            | (0xCC << 17)
                                            | 0x20
                                            | (format << 2)
                                            | clip,
                                        src_base,
                                        dst_base,
                                        src_stride: strides.0 * (format + 1),
                                        dst_stride: strides.1 * (format + 1),
                                        rect_width: 4,
                                        rect_height: 3,
                                        rect_src_x: sx,
                                        rect_src_y: sy,
                                        rect_dst_x: dx,
                                        rect_dst_y: dy,
                                        left_clip: 1,
                                        right_clip: 14,
                                        top_clip: 1,
                                        bottom_clip: 3,
                                        ..Group::default()
                                    };
                                    let mut expected = memory();
                                    scalar_copy(&g, &mut expected);
                                    let mut probe = memory();
                                    if copy(&g, &mut probe) {
                                        used_rows += 1;
                                        if clip != 0 && probe != memory() {
                                            used_clipped += 1;
                                        }
                                        assert_eq!(probe, expected, "{g:?}");
                                    } else {
                                        assert_eq!(probe, memory(), "rejected copy wrote memory");
                                    }
                                    let mut actual = memory();
                                    let mut card = Virge::default();
                                    card.groups[BITBLT] = g;
                                    assert!(card.bitblt(&mut actual));
                                    assert_eq!(actual, expected, "{:?}", card.groups[BITBLT]);
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(used_rows > 100);
        assert!(used_clipped > 20);
    }

    #[test]
    fn solid_pattern_rows_preserve_transparency_clipping_and_coordinates() {
        let mut used_rows = 0;
        for format in 0..=2 {
            for (x, y) in [(0, 0), (2, 2), (14, 4)] {
                for base in [0, 1, u32::MAX - 7] {
                    for clip in [0, 2] {
                        for transparent in [0, 0x200] {
                            for pattern in [0, u64::MAX, 0xAA55_AA55_AA55_AA55] {
                                let g = Group {
                                    command_set: (0xF0 << 17)
                                        | 0x20
                                        | (format << 2)
                                        | clip
                                        | transparent,
                                    dst_base: base,
                                    dst_stride: 16 * (format + 1),
                                    rect_width: 4,
                                    rect_height: 3,
                                    rect_dst_x: x,
                                    rect_dst_y: y,
                                    left_clip: 1,
                                    right_clip: 14,
                                    top_clip: 1,
                                    bottom_clip: 3,
                                    mono_pat: pattern,
                                    mono_pat_fg: 0x1234_5678,
                                    ..Group::default()
                                };
                                let (left, right, top, bottom) = if clip != 0 {
                                    (x.max(1), (x + 3).min(14), y.max(1), (y + 2).min(3))
                                } else {
                                    (x, x + 3, y, y + 2)
                                };
                                let mut expected = memory();
                                for dy in top..=bottom {
                                    for dx in left..=right {
                                        let bit = ((dy - y) & 7) * 8 + 7 - ((dx - x) & 7);
                                        if transparent == 0 || pattern >> bit & 1 != 0 {
                                            draw(&g, &mut expected, dx, dy, g.mono_pat_fg);
                                        }
                                    }
                                }
                                let mut probe = memory();
                                if fill(&g, &mut probe, [left, right, top, bottom]) {
                                    used_rows += 1;
                                    assert_eq!(probe, expected, "{g:?}");
                                } else {
                                    assert_eq!(probe, memory(), "rejected fill wrote memory");
                                }
                                let mut actual = memory();
                                let mut actual_group = g.clone();
                                draw_rect(&mut actual_group, &mut actual);
                                assert_eq!(actual, expected, "{g:?}");
                                assert_eq!(actual_group.rect_dst_x, right.wrapping_add(1));
                                assert_eq!(actual_group.rect_dst_y, bottom.wrapping_add(1));
                            }
                        }
                    }
                }
            }
        }
        assert!(used_rows > 20);
    }
}
