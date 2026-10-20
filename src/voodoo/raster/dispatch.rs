use super::{FIRST_COMBINE, VARIANT_COUNTS, variant_index};

/// The two mask families have distinct keys: the registers' fixed bits
/// under the masks of `full` or `combine_only`, so a variant's key is its
/// `FIXED` values and a triangle's is its registers under the same masks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Key {
    Full(usize, [u32; 6]),
    Combine(usize, [u32; 3]),
}

impl Key {
    #[inline]
    pub(super) const fn full(tmus: usize, regs: [u32; 6]) -> Self {
        let f = super::full(tmus, regs);
        Self::Full(tmus, [f[0].1, f[1].1, f[2].1, f[3].1, f[4].1, f[5].1])
    }

    #[inline]
    pub(super) const fn combine_only(tmus: usize, regs: [u32; 3]) -> Self {
        let f = super::combine_only(tmus, regs);
        Self::Combine(tmus, [f[1].1, f[4].1, f[5].1])
    }
}

#[inline(always)]
pub(super) fn variant(regs: [u32; 6], texcount: usize) -> Option<usize> {
    match texcount {
        0 => lookup::<0>(regs),
        1 => lookup::<1>(regs),
        _ => lookup::<2>(regs),
    }
}

#[inline(always)]
fn lookup<const TMUS: usize>(regs: [u32; 6]) -> Option<usize> {
    if VARIANT_COUNTS & (1 << TMUS) == 0 {
        return None;
    }
    let full = variant_index(Key::full(TMUS, regs));
    if full.is_some_and(|index| index < FIRST_COMBINE) {
        return full;
    }
    let combine = variant_index(Key::combine_only(TMUS, [regs[1], regs[4], regs[5]]));
    // Keep declaration order even when both mask families match.
    match (full, combine) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::super::VARIANTS;
    use super::*;

    fn linear(regs: [u32; 6], count: usize) -> Option<usize> {
        VARIANTS.iter().position(|(tmus, fixed)| {
            *tmus == count.min(2)
                && regs
                    .iter()
                    .zip(fixed)
                    .all(|(r, (mask, value))| r & mask == *value)
        })
    }

    #[test]
    fn keys_match_the_original_masks_at_every_bit_and_unit_count() {
        for &(_, fixed) in VARIANTS {
            let regs = fixed.map(|(_, value)| value);
            for count in [0, 1, 2, 3, usize::MAX] {
                assert_eq!(variant(regs, count), linear(regs, count));
                for reg in 0..6 {
                    for bit in 0..32 {
                        let mut changed = regs;
                        changed[reg] ^= 1 << bit;
                        assert_eq!(
                            variant(changed, count),
                            linear(changed, count),
                            "count={count}, regs={changed:08X?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn random_and_partly_fixed_states_match_the_linear_search() {
        let mut random = 0x123456789ABCDEFu64;
        for &(_, fixed) in VARIANTS {
            for i in 0..4096 {
                let regs = fixed.map(|(mask, value)| {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    if i & 1 == 0 {
                        value | random as u32 & !mask
                    } else {
                        random as u32
                    }
                });
                let count = i % 4;
                assert_eq!(
                    variant(regs, count),
                    linear(regs, count),
                    "count={count}, regs={regs:08X?}"
                );
            }
        }
    }
}
