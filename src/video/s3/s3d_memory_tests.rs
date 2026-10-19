use super::*;

fn palette(_: u8) -> (u8, u8, u8) {
    (0, 0, 0)
}

#[test]
fn packed_reads_match_byte_reads_at_wraps_and_memory_holes() {
    for len in [1usize, 2, 3, 4, 5, 7, 8, 9, 15, 16, 31, 32, 257] {
        let mut vram: Vec<_> = (0..len).map(|i| (i * 73 + 19) as u8).collect();
        let mut target = Target {
            vram: &mut vram,
            palette: &palette,
        };
        let regs = [0; 0x80];
        let c = Context::new(&mut target, CMD_3D | 1 << 2, &regs).unwrap();
        let addresses = (0..len as u32 * 3).chain([
            0x8000_0000,
            0xFFFF_FFFC,
            0xFFFF_FFFD,
            0xFFFF_FFFE,
            0xFFFF_FFFF,
        ]);
        for addr in addresses {
            let oracle = |count| {
                let mut value = 0;
                for i in 0..count {
                    let at = (addr.wrapping_add(i) & c.memmask) as usize;
                    value |= (c.t.vram.get(at).copied().unwrap_or(0) as u32) << (i * 8);
                }
                value
            };
            assert_eq!(c.word(addr), oracle(2), "len={len}, addr={addr:#x}");
            assert_eq!(c.packed::<3>(addr), oracle(3), "len={len}, addr={addr:#x}");
            assert_eq!(c.dword(addr), oracle(4), "len={len}, addr={addr:#x}");
        }
    }
}

#[test]
fn packed_writes_keep_the_physical_modulo_wrap() {
    for len in [1usize, 2, 3, 4, 5, 7, 8, 9, 15, 16, 31, 32, 257] {
        let initial: Vec<_> = (0..len).map(|i| (i * 73 + 19) as u8).collect();
        for addr in (0..len as u32 * 3).chain([0x8000_0000, 0xFFFF_FFFE]) {
            for value in [0u16, 0xFFFF, 0x1234, 0xABCD] {
                let mut expected = initial.clone();
                // Writes wrap at the physical length, whereas reads use
                // a rounded mask and return zero in missing memory.
                for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
                    expected[(addr as usize + i) % len] = byte;
                }
                let mut actual = initial.clone();
                let mut target = Target {
                    vram: &mut actual,
                    palette: &palette,
                };
                let regs = [0; 0x80];
                let mut c = Context::new(&mut target, CMD_3D | 1 << 2, &regs).unwrap();
                c.write_word(addr, value);
                assert_eq!(
                    actual, expected,
                    "len={len}, addr={addr:#x}, value={value:#x}"
                );
            }
        }
    }
}
