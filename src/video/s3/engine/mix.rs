//! Binary raster operations resolved once per command or host write.

#[derive(Clone, Copy, Debug)]
pub(super) struct Mix {
    pub(super) mode: u16,
    function: fn(u32, u32) -> u32,
}

impl Mix {
    pub(super) fn new(mode: u16) -> Self {
        let function: fn(u32, u32) -> u32 = match mode & 0x0F {
            0x00 => |_, d| !d,
            0x01 => |_, _| 0,
            0x02 => |_, _| u32::MAX,
            0x03 => |_, d| d,
            0x04 => |s, _| !s,
            0x05 => |s, d| s ^ d,
            0x06 => |s, d| !(s ^ d),
            0x07 => |s, _| s,
            0x08 => |s, d| !(s & d),
            0x09 => |s, d| !s | d,
            0x0A => |s, d| s | !d,
            0x0B => |s, d| s | d,
            0x0C => |s, d| s & d,
            0x0D => |s, d| s & !d,
            0x0E => |s, d| !s & d,
            _ => |s, d| !(s | d),
        };
        Self { mode, function }
    }

    pub(super) fn apply(self, source: u32, dest: u32) -> u32 {
        (self.function)(source, dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_selection_bits_do_not_change_the_logical_truth_table() {
        let truth = [5, 0, 15, 10, 3, 6, 9, 12, 7, 11, 13, 14, 8, 4, 2, 1];
        for high in [0, 0x20, 0x40, 0x60, 0xFF00] {
            for (low, truth) in truth.into_iter().enumerate() {
                let mode = high | low as u16;
                let mix = Mix::new(mode);
                assert_eq!(mix.mode, mode);
                assert_eq!(mix.apply(0xCCCC_CCCC, 0xAAAA_AAAA), truth * 0x1111_1111);
            }
        }
    }
}
