use super::{Memory, Palettes, Pipe};

pub(super) type Pixel = fn(&Pipe, &mut Memory<'_>, &Palettes<'_>, i32, i32, u32) -> Option<u32>;

impl Pipe {
    pub(super) fn select_pixel(&self) -> Pixel {
        match (
            self.z_cntl & 1 != 0,
            self.cntl >> 6 & 3 == 2,
            self.pix_width & 0xF,
        ) {
            (false, false, 3) => Self::pixel_mode::<true, false, false, 3>,
            (false, false, 4) => Self::pixel_mode::<true, false, false, 4>,
            (false, false, 6) => Self::pixel_mode::<true, false, false, 6>,
            (false, false, 15) => Self::pixel_mode::<true, false, false, 15>,
            (false, false, _) => Self::pixel_mode::<true, false, false, 16>,
            (false, true, 3) => Self::pixel_mode::<true, false, true, 3>,
            (false, true, 4) => Self::pixel_mode::<true, false, true, 4>,
            (false, true, 6) => Self::pixel_mode::<true, false, true, 6>,
            (false, true, 15) => Self::pixel_mode::<true, false, true, 15>,
            (false, true, _) => Self::pixel_mode::<true, false, true, 16>,
            (true, false, 3) => Self::pixel_mode::<true, true, false, 3>,
            (true, false, 4) => Self::pixel_mode::<true, true, false, 4>,
            (true, false, 6) => Self::pixel_mode::<true, true, false, 6>,
            (true, false, 15) => Self::pixel_mode::<true, true, false, 15>,
            (true, false, _) => Self::pixel_mode::<true, true, false, 16>,
            (true, true, 3) => Self::pixel_mode::<true, true, true, 3>,
            (true, true, 4) => Self::pixel_mode::<true, true, true, 4>,
            (true, true, 6) => Self::pixel_mode::<true, true, true, 6>,
            (true, true, 15) => Self::pixel_mode::<true, true, true, 15>,
            (true, true, _) => Self::pixel_mode::<true, true, true, 16>,
        }
    }
}
