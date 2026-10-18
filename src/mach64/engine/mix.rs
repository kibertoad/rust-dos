//! The data-path mix selected when an operation snapshots its registers.

use super::Setup;

#[derive(Clone, Copy, Debug)]
pub(super) struct Mix {
    pub(super) mode: u32,
    function: fn(&Setup, u32, u32) -> u32,
}

impl Default for Mix {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Mix {
    pub(super) fn new(mode: u32) -> Self {
        let function: fn(&Setup, u32, u32) -> u32 = match mode {
            0x0 => |_, _, d| !d,
            0x1 => |_, _, _| 0,
            0x2 => |_, _, _| u32::MAX,
            0x3 => |_, _, d| d,
            0x4 => |_, s, _| !s,
            0x5 => |_, s, d| s ^ d,
            0x6 => |_, s, d| !(s ^ d),
            0x7 => |_, s, _| s,
            0x8 => |_, s, d| !(s & d),
            0x9 => |_, s, d| !s | d,
            0xA => |_, s, d| s | !d,
            0xB => |_, s, d| s | d,
            0xC => |_, s, d| s & d,
            0xD => |_, s, d| s & !d,
            0xE => |_, s, d| !s & d,
            0xF => |_, s, d| !(s | d),
            0x17 => Setup::average,
            _ => |_, s, _| s,
        };
        Self { mode, function }
    }

    pub(super) fn apply(self, setup: &Setup, source: u32, dest: u32) -> u32 {
        (self.function)(setup, source, dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mach64::engine::Size;

    #[test]
    fn selected_logic_and_fallback_mixes_keep_their_truth_tables() {
        let setup = Setup::default();
        let truth = [5, 0, 15, 10, 3, 6, 9, 12, 7, 11, 13, 14, 8, 4, 2, 1];
        for (mode, truth) in truth.into_iter().enumerate() {
            let mix = Mix::new(mode as u32);
            assert_eq!(mix.mode, mode as u32);
            assert_eq!(
                mix.apply(&setup, 0xCCCC_CCCC, 0xAAAA_AAAA),
                truth * 0x1111_1111
            );
        }
        for mode in (0x10..=0x1F).chain([32, 255, u32::MAX]) {
            if mode != 0x17 {
                assert_eq!(
                    Mix::new(mode).apply(&setup, 0x1234_ABCD, 0xFEDC_5678),
                    0x1234_ABCD
                );
            }
        }
    }

    #[test]
    fn average_uses_the_operations_pixel_width_and_chain_mask() {
        for size in [Size::Byte, Size::Word, Size::Dword] {
            for chain_mask in [0, 0x8421, 0x7FFF, 0xFFFF] {
                let setup = Setup {
                    dst_size: size,
                    chain_mask,
                    ..Setup::default()
                };
                for (source, dest) in [(0x1234_ABCD, 0xFEDC_5678), (0, u32::MAX), (0xFFFF, 1)] {
                    assert_eq!(
                        Mix::new(0x17).apply(&setup, source, dest),
                        setup.average(source, dest)
                    );
                }
            }
        }
    }
}
