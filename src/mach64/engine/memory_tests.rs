use super::{Memory, Size, read_le};

#[test]
fn whole_word_access_matches_each_wrapped_byte() {
    for len in [1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 32, 128] {
        let original: Vec<u8> = (0..len).map(|at| (at * 73 + 19) as u8).collect();
        for (size, bytes, shift) in [(Size::Word, 2, 1), (Size::Dword, 4, 2)] {
            for at in (0..64u32).chain([u32::MAX, 0x7FFF_FFFF, 0xF000_0001, 0x1234_5678]) {
                let mut actual = original.clone();
                let mut memory = Memory {
                    vram: &mut actual,
                    lsb_first: false,
                };
                let start = at << shift;
                let expected = (0..bytes).fold(0, |word, i| {
                    word | (original[(start as usize + i) & (len - 1)] as u32) << (8 * i)
                });
                assert_eq!(
                    memory.read(at, size),
                    expected,
                    "len={len}, size={size:?}, at={at}"
                );
                for value in [0, u32::MAX, 0x1234_ABCD] {
                    let mut expected = original.clone();
                    for i in 0..bytes {
                        expected[(start as usize + i) & (len - 1)] = (value >> (8 * i)) as u8;
                    }
                    memory.vram.copy_from_slice(&original);
                    memory.write(at, size, value);
                    assert_eq!(memory.vram, expected, "len={len}, size={size:?}, at={at}");
                }
            }
        }
    }
}

#[test]
fn byte_addressed_reads_match_each_wrapped_byte() {
    for len in [1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 32, 128] {
        let mut vram: Vec<u8> = (0..len).map(|at| (at * 73 + 19) as u8).collect();
        let memory = Memory {
            vram: &mut vram,
            lsb_first: false,
        };
        for start in (0..64u32).chain([u32::MAX, u32::MAX - 2, 0xF000_0001, 0x1234_5677]) {
            let expected = |bytes: usize| {
                (0..bytes).fold(0, |word, i| {
                    let at = (start as usize).wrapping_add(i) & (len - 1);
                    word | (memory.vram[at] as u32) << (8 * i)
                })
            };
            let at = format!("len={len}, start={start:#x}");
            assert_eq!(memory.read_bytes::<2>(start), expected(2), "{at}");
            assert_eq!(memory.read_bytes::<4>(start), expected(4), "{at}");
            assert_eq!(read_le::<4>(memory.vram, start), expected(4), "{at}");
        }
    }
}
