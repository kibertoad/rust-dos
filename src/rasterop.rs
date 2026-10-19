//! Ternary raster operations selected before drawing. Each entry has a
//! constant truth table, so the compiler removes its per-bit decisions.

pub(crate) type Ternary = fn(u32, u32, u32) -> u32;

fn apply<const ROP: u8>(dest: u32, src: u32, pattern: u32) -> u32 {
    match ROP {
        0x00 => 0,
        0x55 => !dest,
        0x5A => dest ^ pattern,
        0x66 => dest ^ src,
        0x88 => dest & src,
        0x96 => dest ^ src ^ pattern,
        0xAA => dest,
        0xB8 => pattern ^ ((pattern ^ dest) & src),
        0xC0 => pattern & src,
        0xCC => src,
        0xE2 => dest ^ ((dest ^ pattern) & src),
        0xEE => dest | src,
        0xF0 => pattern,
        0xFF => u32::MAX,
        _ => {
            let mut result = 0;
            for bit in 0..8 {
                if ROP & (1 << bit) != 0 {
                    let p = if bit & 4 != 0 { pattern } else { !pattern };
                    let s = if bit & 2 != 0 { src } else { !src };
                    let d = if bit & 1 != 0 { dest } else { !dest };
                    result |= p & s & d;
                }
            }
            result
        }
    }
}

macro_rules! row {
    ($base:expr) => {
        [
            apply::<{ $base }>,
            apply::<{ $base | 1 }>,
            apply::<{ $base | 2 }>,
            apply::<{ $base | 3 }>,
            apply::<{ $base | 4 }>,
            apply::<{ $base | 5 }>,
            apply::<{ $base | 6 }>,
            apply::<{ $base | 7 }>,
            apply::<{ $base | 8 }>,
            apply::<{ $base | 9 }>,
            apply::<{ $base | 10 }>,
            apply::<{ $base | 11 }>,
            apply::<{ $base | 12 }>,
            apply::<{ $base | 13 }>,
            apply::<{ $base | 14 }>,
            apply::<{ $base | 15 }>,
        ]
    };
}

const TABLE: [[Ternary; 16]; 16] = [
    row!(0x00),
    row!(0x10),
    row!(0x20),
    row!(0x30),
    row!(0x40),
    row!(0x50),
    row!(0x60),
    row!(0x70),
    row!(0x80),
    row!(0x90),
    row!(0xA0),
    row!(0xB0),
    row!(0xC0),
    row!(0xD0),
    row!(0xE0),
    row!(0xF0),
];

/// Bit P*4 + S*2 + D of `rop` is the result for those input bits.
/// Arguments of the returned function are destination, source, pattern.
pub(crate) const fn ternary(rop: u8) -> Ternary {
    TABLE[(rop >> 4) as usize][(rop & 15) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_selected_rop_matches_its_truth_table() {
        let mut seed = 0x1829_3746u32;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for rop in 0..=255u8 {
            let function = ternary(rop);
            assert_eq!(
                function(0xAAAA_AAAA, 0xCCCC_CCCC, 0xF0F0_F0F0),
                u32::from(rop) * 0x0101_0101,
                "rop={rop:02X}"
            );
            for _ in 0..64 {
                let (dest, src, pattern) = (random(), random(), random());
                let mut expected = 0;
                for bit in 0..32 {
                    let index =
                        ((pattern >> bit & 1) << 2) | ((src >> bit & 1) << 1) | (dest >> bit & 1);
                    expected |= (u32::from(rop) >> index & 1) << bit;
                }
                assert_eq!(function(dest, src, pattern), expected, "rop={rop:02X}");
            }
        }
    }
}
