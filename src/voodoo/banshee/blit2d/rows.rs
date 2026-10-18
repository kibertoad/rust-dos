//! Unkeyed SRCCOPY rows. VRAM remains atomic: aligned word stores replace
//! pairs of byte read/modify/writes without borrowing its memory mutably.

use super::*;
use std::ops::Range;

fn bytes(e: &Engine) -> Option<usize> {
    if e.dst_tiled
        || e.command_extra & (CMDEXTRA_SRC_COLORKEY | CMDEXTRA_DST_COLORKEY) != 0
        || e.rops[0] != 0xCC
        || e.use_pattern_trans()
    {
        return None;
    }
    match e.dst_format & COL_MASK {
        COL_8BPP => Some(1),
        COL_16BPP => Some(2),
        COL_24BPP => Some(3),
        COL_32BPP => Some(4),
        _ => None,
    }
}

fn destination(e: &Engine, mask: u32, bytes: usize, y: i32, dx: i32) -> Option<Range<usize>> {
    if e.dst_size_x <= 0 || y < 0 {
        return None;
    }
    let last = e.dst_x.checked_add((e.dst_size_x - 1).checked_mul(dx)?)?;
    let (left, right) = (e.dst_x.min(last), e.dst_x.max(last));
    let clip = e.clip_of_command();
    if left < 0 || left < clip.x_min || right >= clip.x_max || y < clip.y_min || y >= clip.y_max {
        return None;
    }
    let start = e
        .dst_base
        .checked_add((y as u32).checked_mul(e.dst_stride)?)?
        .checked_add((left as u32).checked_mul(bytes as u32)?)?;
    let end = start.checked_add((e.dst_size_x as u32).checked_mul(bytes as u32)?)?;
    if end - 1 > mask {
        return None;
    }
    Some(start as usize..end as usize)
}

fn load_word(e: &Engine, ram: &Vram, src: Source, offset: usize) -> u16 {
    match src {
        Source::Vram(base) => {
            let at = base as usize + offset;
            if at & 1 == 0 {
                ram.get(at >> 1)
            } else {
                (ram.get(at >> 1) >> 8) | (ram.get((at >> 1) + 1) << 8)
            }
        }
        Source::Host(base) => {
            u16::from_le_bytes([e.host_data[base + offset], e.host_data[base + offset + 1]])
        }
    }
}

pub(super) fn copy(
    e: &mut Engine,
    ram: &Vram,
    mask: u32,
    src: Source,
    src_x: i32,
    dx: i32,
    src_tiled: bool,
) -> bool {
    let Some(bytes) = bytes(e) else { return false };
    if src_tiled
        || e.src_format & COL_MASK != e.dst_format & COL_MASK
        || e.src_bpp != bytes as u32 * 8
    {
        return false;
    }
    let Some(dest) = destination(e, mask, bytes, e.dst_y, dx) else {
        return false;
    };
    if dest.end > ram.bytes() {
        return false;
    }
    let Some(last) = src_x.checked_add((e.dst_size_x - 1) * dx) else {
        return false;
    };
    let left = src_x.min(last);
    if left < 0 {
        return false;
    }
    let Some(offset) = (left as usize).checked_mul(bytes) else {
        return false;
    };
    let len = dest.len();
    match src {
        Source::Vram(base) => {
            let Some(start) = (base as usize).checked_add(offset) else {
                return false;
            };
            let Some(end) = start.checked_add(len) else {
                return false;
            };
            if end > ram.bytes() || end - 1 > mask as usize {
                return false;
            }
            if start < dest.end
                && dest.start < end
                && ((dx > 0 && dest.start > start) || (dx < 0 && dest.start < start))
            {
                // Keep destructive overlaps in the pixel path: word
                // prefetching must not change which source bytes survive.
                return false;
            }
        }
        Source::Host(base) => {
            if base
                .checked_add(offset)
                .and_then(|at| at.checked_add(len))
                .is_none_or(|end| end > e.host_data.len())
            {
                return false;
            }
        }
    }
    let copy_byte = |at: usize| {
        let value = e.src_byte(ram, src, offset + at, mask);
        ram.set_byte(dest.start + at, value);
    };
    if dx > 0 {
        let mut at = 0;
        if dest.start & 1 != 0 {
            copy_byte(at);
            at += 1;
        }
        while at + 2 <= len {
            ram.set((dest.start + at) >> 1, load_word(e, ram, src, offset + at));
            at += 2;
        }
        if at < len {
            copy_byte(at);
        }
    } else {
        let mut at = len;
        if dest.end & 1 != 0 {
            at -= 1;
            copy_byte(at);
        }
        while at >= 2 {
            at -= 2;
            ram.set((dest.start + at) >> 1, load_word(e, ram, src, offset + at));
        }
        if at != 0 {
            copy_byte(0);
        }
    }
    e.drawn = true;
    true
}

pub(super) fn fill(e: &mut Engine, ram: &Vram, mask: u32, y: i32) -> bool {
    let Some(bytes) = bytes(e) else { return false };
    let dx = step(e.command & COMMAND_DX != 0);
    let Some(dest) = destination(e, mask, bytes, y, dx) else {
        return false;
    };
    if dest.end > ram.bytes() {
        return false;
    }
    let pixel = e.color_fore.to_le_bytes();
    let mut at = dest.start;
    let mut phase = 0;
    if at & 1 != 0 {
        ram.set_byte(at, pixel[0]);
        at += 1;
        phase = 1 % bytes;
    }
    // Two-byte stores repeat every one, two or three words. Construct
    // their cycle once rather than finding a pixel's byte per store.
    let period = match bytes {
        3 => 3,
        4 => 2,
        _ => 1,
    };
    let words: [u16; 3] = std::array::from_fn(|n| {
        u16::from_le_bytes([
            pixel[(phase + n * 2) % bytes],
            pixel[(phase + n * 2 + 1) % bytes],
        ])
    });
    let mut n = 0;
    if period == 1 {
        ram.fill(at >> 1..dest.end >> 1, words[0]);
        at = dest.end & !1;
    } else {
        while at + 2 <= dest.end {
            ram.set(at >> 1, words[n]);
            at += 2;
            n += 1;
            if n == period {
                n = 0;
            }
        }
    }
    if at < dest.end {
        ram.set_byte(at, pixel[(phase + n * 2) % bytes]);
    }
    e.drawn = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Vram {
        let memory = Vram::new(512);
        for at in 0..512 {
            memory.set_byte(at, (at * 73 + 19) as u8);
        }
        memory
    }

    fn engine(format: u32) -> Engine {
        let mut engine = Engine {
            dst_format: format,
            src_format: format,
            dst_stride: 32,
            dst_size_x: 5,
            dst_size_y: 3,
            dst_y: 1,
            src_y: 2,
            src_bpp: match format {
                COL_8BPP => 8,
                COL_16BPP => 16,
                COL_24BPP => 24,
                _ => 32,
            },
            color_fore: 0xA1B2_C3D4,
            rops: [0xCC; 4],
            rop_functions: [crate::rasterop::ternary(0xCC); 4],
            ..Engine::default()
        };
        engine.clip[0] = Clip {
            x_min: 0,
            x_max: 32,
            y_min: 0,
            y_max: 32,
        };
        for (at, byte) in engine.host_data.iter_mut().enumerate() {
            *byte = (at * 37 + 11) as u8;
        }
        engine
    }

    fn scalar(engine: &Engine) -> Engine {
        let mut scalar = engine.clone();
        // All four ROPs copy the source. Enabling the key comparisons
        // keeps this result but forces the original pixel path.
        scalar.command_extra |= CMDEXTRA_SRC_COLORKEY | CMDEXTRA_DST_COLORKEY;
        scalar
    }

    #[test]
    fn word_copies_preserve_odd_edges_for_host_and_vram_sources() {
        for format in [COL_8BPP, COL_16BPP, COL_24BPP, COL_32BPP] {
            for base in [0, 1] {
                for source in [Source::Vram(128), Source::Vram(129), Source::Host(0)] {
                    for direction in [0, COMMAND_DX] {
                        let mut e = engine(format);
                        e.dst_base = base;
                        e.dst_x = 8;
                        e.command = direction;
                        let dx = step(direction != 0);
                        let actual = memory();
                        let expected = memory();
                        let mut reference = scalar(&e);
                        reference.copy_line(&expected, 511, source, true, 8, false);
                        assert!(copy(&mut e, &actual, 511, source, 8, dx, false));
                        assert_eq!(actual.to_bytes(), expected.to_bytes());
                        assert!(e.drawn);
                    }
                }
            }
        }
    }

    #[test]
    fn rows_match_pixels_across_overlap_clip_tiles_and_wrap() {
        for format in [COL_8BPP, COL_16BPP, COL_24BPP, COL_32BPP] {
            for source in [
                Source::Vram(0),
                Source::Vram(1),
                Source::Vram(480),
                Source::Host(0),
                Source::Host(HOST_DATA - 2),
            ] {
                for src_x in [0, 1, 5] {
                    for dst_x in [0, 1, 4] {
                        for direction in [0, COMMAND_DX, COMMAND_DY, COMMAND_DX | COMMAND_DY] {
                            for use_x_dir in [false, true] {
                                for base in [0, 1, 480] {
                                    for (src_tiled, dst_tiled) in
                                        [(false, false), (true, false), (false, true)]
                                    {
                                        for clipped in [false, true] {
                                            let mut e = engine(format);
                                            e.dst_base = base;
                                            e.dst_tiled = dst_tiled;
                                            e.dst_x = dst_x;
                                            e.command = direction;
                                            if clipped {
                                                e.clip[0] = Clip {
                                                    x_min: 1,
                                                    x_max: 4,
                                                    y_min: 1,
                                                    y_max: 3,
                                                };
                                            }
                                            let mut reference = scalar(&e);
                                            let actual = memory();
                                            let expected = memory();
                                            e.copy_line(
                                                &actual, 511, source, use_x_dir, src_x, src_tiled,
                                            );
                                            reference.copy_line(
                                                &expected, 511, source, use_x_dir, src_x, src_tiled,
                                            );
                                            assert_eq!(
                                                actual.to_bytes(),
                                                expected.to_bytes(),
                                                "copy {e:?}"
                                            );
                                            assert_eq!(
                                                (e.src_y, e.dst_y, e.drawn),
                                                (reference.src_y, reference.dst_y, reference.drawn)
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
    fn solid_rows_keep_pattern_transparency_and_command_coordinates() {
        for format in [COL_8BPP, COL_16BPP, COL_24BPP, COL_32BPP] {
            for direction in [0, COMMAND_DX, COMMAND_DY, COMMAND_DX | COMMAND_DY] {
                for x in [0, 1, 8] {
                    for y in [0, 1, 8] {
                        for base in [0, 1, 480] {
                            for clipped in [false, true] {
                                for transparent in [false, true] {
                                    let mut e = engine(format);
                                    e.dst_x = x;
                                    e.dst_y = y;
                                    e.dst_base = base;
                                    e.command =
                                        direction | COMMAND_INC_X_START | COMMAND_INC_Y_START;
                                    if transparent {
                                        e.command |= COMMAND_PATTERN_MONO | COMMAND_TRANS_MONO;
                                    }
                                    if clipped {
                                        e.clip[0] = Clip {
                                            x_min: 1,
                                            x_max: 4,
                                            y_min: 1,
                                            y_max: 3,
                                        };
                                    }
                                    let mut reference = scalar(&e);
                                    let actual = memory();
                                    let expected = memory();
                                    e.rectfill(&actual, 511);
                                    reference.rectfill(&expected, 511);
                                    assert_eq!(
                                        actual.to_bytes(),
                                        expected.to_bytes(),
                                        "fill {e:?}"
                                    );
                                    assert_eq!(
                                        (e.dst_xy, e.dst_x, e.dst_y, e.drawn),
                                        (
                                            reference.dst_xy,
                                            reference.dst_x,
                                            reference.dst_y,
                                            reference.drawn
                                        )
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
