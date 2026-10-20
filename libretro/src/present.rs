//! Cached pixel conversion and bounded comparison of frontend pictures.

use rust_dos::video::{self, Frame};
use std::ops::Range;

/// Rows that differ, including all rows after a geometry change.
pub fn changed_rows(old: &Frame, new: &Frame, candidates: Range<usize>) -> Option<Range<usize>> {
    if (old.width, old.height) != (new.width, new.height) {
        return Some(0..new.height as usize);
    }
    let bytes = new.width as usize * 4;
    if bytes == 0 {
        return None;
    }
    video::changed_rows_in(&old.rgba, &new.rgba, bytes, candidates)
}

/// The last picture converted into a frontend's pixel format. Each
/// update converts and copies only the rows that changed.
pub struct Pixels<T> {
    frame: Frame,
    pub data: Vec<T>,
}

impl<T> Default for Pixels<T> {
    fn default() -> Self {
        Self {
            frame: Frame::new(0, 0),
            data: Vec::new(),
        }
    }
}

impl<T: Copy + Default> Pixels<T> {
    pub fn update(
        &mut self,
        frame: &Frame,
        candidates: Range<usize>,
        convert: impl Fn([u8; 4]) -> T,
    ) -> Option<Range<usize>> {
        let rows = changed_rows(&self.frame, frame, candidates)?;
        self.frame.resize(frame.width, frame.height);
        self.data
            .resize((frame.width * frame.height) as usize, T::default());
        let pixels = rows.start * frame.width as usize..rows.end * frame.width as usize;
        let bytes = pixels.start * 4..pixels.end * 4;
        let rgb = &frame.rgba[bytes.clone()];
        for (out, &pixel) in self.data[pixels].iter_mut().zip(rgb.as_chunks::<4>().0) {
            *out = convert(pixel);
        }
        self.frame.rgba[bytes].copy_from_slice(rgb);
        Some(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_only_changed_rows_and_preserves_others() {
        let mut cache = Pixels::<u32>::default();
        let mut frame = Frame::new(3, 4);
        let convert = |[r, g, b, _]: [u8; 4]| u32::from_be_bytes([0, r, g, b]);
        assert_eq!(
            cache.update(&frame, 0..frame.height as usize, convert),
            Some(0..4)
        );
        assert_eq!(
            cache.update(&frame, 0..0, |_| panic!("unchanged pixels converted")),
            None
        );
        frame.rgba[12..24].as_chunks_mut::<4>().0.fill([7, 7, 7, 255]);
        let calls = std::cell::Cell::new(0);
        assert_eq!(
            cache.update(&frame, 1..2, |p| {
                calls.set(calls.get() + 1);
                convert(p)
            }),
            Some(1..2)
        );
        assert_eq!(calls.get(), 3);
        assert_eq!(&cache.data[..3], &[0; 3]);
        assert_eq!(&cache.data[3..6], &[0x0007_0707; 3]);
        assert_eq!(&cache.data[6..], &[0; 6]);
        // The same pixel count with different geometry still invalidates
        // the whole texture, even though all pixels are black.
        frame = Frame::new(4, 3);
        assert_eq!(
            cache.update(&frame, 0..frame.height as usize, convert),
            Some(0..3)
        );
        assert_eq!(cache.data, vec![0; 12]);
    }

    #[test]
    fn rgba_pixels_have_opaque_alpha_and_overlays_are_erased() {
        let base = Frame::from_rgb(1, 2, vec![1, 2, 3, 4, 5, 6]);
        let mut cache = Pixels::<[u8; 4]>::default();
        cache.update(&base, 0..base.height as usize, |pixel| pixel);
        assert_eq!(cache.data, [[1, 2, 3, 255], [4, 5, 6, 255]]);
        let mut screen = base.clone();
        screen.rgba[4..].fill(255);
        screen.copy_rows_from(&base, 1..2);
        assert_eq!(screen.rgba, base.rgba);
        screen.copy_rows_from(&Frame::new(2, 1), 0..1);
        assert_eq!((screen.width, screen.height), (2, 1));
        assert_eq!(screen.rgba, [0, 0, 0, 255, 0, 0, 0, 255]);
    }
}
