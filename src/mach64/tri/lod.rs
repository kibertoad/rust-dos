/// LOD is constant across affine spans. Curved spans retain the original
/// per-pixel calculation so their changing derivatives select each mip.
#[derive(Default)]
pub(super) struct Lod {
    scale: f64,
    varying: bool,
    cached: f64,
}

impl Lod {
    pub(super) fn new(control: u32, size: u32, curvature: [i64; 4]) -> Self {
        let largest = size >> 4 & 0xF;
        let scale = if control >> 6 & 3 == 2 && control & (1 << 24) == 0 && largest > 0 {
            (1u64 << largest) as f64 / 1024.0 / 65536.0
        } else {
            0.0
        };
        Self {
            scale,
            varying: scale != 0.0 && curvature.iter().any(|&d| d != 0),
            cached: 0.0,
        }
    }

    pub(super) fn start(&mut self, steps: [i64; 4]) {
        if !self.varying {
            self.cached = self.calculate(steps);
        }
    }

    pub(super) fn value(&self, steps: [i64; 4]) -> f64 {
        if self.varying {
            self.calculate(steps)
        } else {
            self.cached
        }
    }

    fn calculate(&self, steps: [i64; 4]) -> f64 {
        if self.scale == 0.0 {
            return 0.0;
        }
        let rho = steps
            .iter()
            .map(|&d| (d as f64).abs() * self.scale)
            .fold(0.0, f64::max);
        if rho > 1.0 { rho.log2() } else { 0.0 }
    }
}
