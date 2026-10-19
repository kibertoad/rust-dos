use super::{Memory, Palettes, Pipe};

/// A pixel pipeline, as `Pipe::pixel` runs it.
pub(super) type Pixel = fn(&Pipe, &mut Memory<'_>, &Palettes<'_>, i32, i32, u32) -> Option<u32>;

impl Pipe {
    /// The pipeline for a command's Z_CNTL, SCALE_3D_CNTL and
    /// DP_PIX_WIDTH: Z testing and texturing compiled in, and the
    /// destination format where it is one of the common ones.
    pub(super) fn select_pixel(z_cntl: u32, cntl: u32, pix_width: u32) -> Pixel {
        match (z_cntl & 1 != 0, cntl >> 6 & 3 == 2) {
            (false, false) => Self::by_format::<false, false>(pix_width),
            (false, true) => Self::by_format::<false, true>(pix_width),
            (true, false) => Self::by_format::<true, false>(pix_width),
            (true, true) => Self::by_format::<true, true>(pix_width),
        }
    }

    fn by_format<const ZTEST: bool, const TEXTURED: bool>(pix_width: u32) -> Pixel {
        match pix_width & 0xF {
            3 => Self::pixel_mode::<true, ZTEST, TEXTURED, 3>,
            4 => Self::pixel_mode::<true, ZTEST, TEXTURED, 4>,
            6 => Self::pixel_mode::<true, ZTEST, TEXTURED, 6>,
            15 => Self::pixel_mode::<true, ZTEST, TEXTURED, 15>,
            _ => Self::pixel_mode::<true, ZTEST, TEXTURED, 16>,
        }
    }
}
