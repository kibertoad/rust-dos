use super::lod::Lod;

fn reference(control: u32, largest: u32, steps: [i64; 4]) -> f64 {
    let mut lod = 0.0;
    if control & (1 << 24) == 0 && largest > 0 {
        let scale = (1u64 << largest) as f64 / 1024.0 / 65536.0;
        let rho = steps
            .iter()
            .map(|&d| (d as f64).abs() * scale)
            .fold(0.0, f64::max);
        if rho > 1.0 {
            lod = rho.log2();
        }
    }
    lod
}

#[test]
fn cached_lod_matches_the_original_floating_point_operations() {
    let mut random = 0x123456789ABCDEFu64;
    for largest in 0..16 {
        for disabled in [false, true] {
            let control = 2 << 6 | if disabled { 1 << 24 } else { 0 };
            let mut cache = Lod::new(control, largest << 4, [1; 4]);
            for i in 0..4096 {
                let steps = std::array::from_fn(|j| {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    match i {
                        0 => 0,
                        1 => i64::MIN,
                        2 => i64::MAX,
                        3..=66 => (1i64 << (i - 3)).wrapping_add(j as i64).wrapping_sub(2),
                        _ => random as i64,
                    }
                });
                if i == 0 {
                    cache.start(steps);
                }
                assert_eq!(
                    cache.value(steps).to_bits(),
                    reference(control, largest, steps).to_bits(),
                    "largest={largest}, disabled={disabled}, steps={steps:?}"
                );
            }
        }
    }
}

#[test]
fn affine_spans_refresh_at_their_start_and_ignore_smaller_derivatives() {
    let mut cache = Lod::new(2 << 6, 10 << 4, [0; 4]);
    cache.start([1 << 20, 0, 0, 0]);
    assert_eq!(cache.value([1 << 20, 1 << 19, 10, 100]), 4.0);
    cache.start([-(1 << 22), 0, 0, 0]);
    assert_eq!(cache.value([0; 4]), 6.0);
}
