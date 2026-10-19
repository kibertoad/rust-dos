//! Keep the browser's RGBA picture, converting only changed RGB rows.

use std::ops::Range;

#[derive(Default)]
pub(super) struct Pixels {
    pub(super) rgba: Vec<u8>,
    pub(super) rows: Range<usize>,
    size: (usize, usize),
}

impl Pixels {
    /// Compare the composed pictures, including overlays. Keep a complete
    /// RGBA copy for canvas resizes and restored WebGL contexts.
    pub(super) fn update(
        &mut self,
        previous: &[u8],
        current: &[u8],
        width: usize,
        height: usize,
    ) -> bool {
        let row_bytes = width * 3;
        assert_eq!(current.len(), row_bytes * height);
        let full = self.size != (width, height) || previous.len() != current.len();
        self.size = (width, height);
        self.rgba.resize(width * height * 4, 0);
        self.rows = if full {
            0..height
        } else if row_bytes == 0 {
            0..0
        } else {
            let mut rows = previous
                .chunks_exact(row_bytes)
                .zip(current.chunks_exact(row_bytes));
            match rows.position(|(a, b)| a != b) {
                Some(first) => {
                    let last = rows.rposition(|(a, b)| a != b);
                    first..last.map_or(first + 1, |last| first + last + 2)
                }
                None => 0..0,
            }
        };
        let start = self.rows.start * width;
        let end = self.rows.end * width;
        for (rgba, &[r, g, b]) in self.rgba[start * 4..end * 4]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(current[start * 3..end * 3].as_chunks::<3>().0)
        {
            *rgba = [r, g, b, 0xFF];
        }
        !self.rows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_blank_picture_is_uploaded_and_an_unchanged_one_is_not() {
        let mut pixels = Pixels::default();
        let rgb = vec![0; 4 * 5 * 3];
        assert!(pixels.update(&rgb, &rgb, 4, 5));
        assert_eq!(pixels.rows, 0..5);
        assert!(
            pixels
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [0, 0, 0, 255])
        );
        assert!(!pixels.update(&rgb, &rgb, 4, 5));
        assert_eq!(pixels.rows, 0..0);
    }

    #[test]
    fn changes_at_either_edge_and_in_separate_rows_bound_the_upload() {
        let mut pixels = Pixels::default();
        let mut previous = vec![0; 4 * 5 * 3];
        pixels.update(&[], &previous, 4, 5);
        for expected in [0..1, 4..5, 1..4, 2..3] {
            let mut current = previous.clone();
            current[expected.start * 12] ^= 0x53;
            current[expected.end * 12 - 1] ^= 0x19;
            assert!(pixels.update(&previous, &current, 4, 5));
            assert_eq!(pixels.rows, expected);
            let full: Vec<_> = current
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|&[r, g, b]| [r, g, b, 255])
                .collect();
            assert_eq!(pixels.rgba, full);
            previous = current;
        }
    }

    #[test]
    fn a_mode_change_uploads_every_row_even_with_the_same_byte_count() {
        let mut pixels = Pixels::default();
        let rgb = vec![23; 4 * 5 * 3];
        pixels.update(&[], &rgb, 4, 5);
        assert!(pixels.update(&rgb, &rgb, 5, 4));
        assert_eq!(pixels.rows, 0..4);
        assert!(pixels.update(&rgb, &rgb[..3], 1, 1));
        assert_eq!(pixels.rows, 0..1);
        assert_eq!(pixels.rgba, [23, 23, 23, 255]);
    }

    #[test]
    fn partial_updates_match_full_conversion_over_repeated_composed_frames() {
        let (width, height) = (37, 29);
        let mut previous = vec![0; width * height * 3];
        let mut pixels = Pixels::default();
        pixels.update(&[], &previous, width, height);
        for frame in 0..200 {
            let mut current = previous.clone();
            // Include an overlay disappearing from its old position.
            for row in [frame % height, (frame + height - 1) % height] {
                current[row * width * 3..(row + 1) * width * 3].fill(frame as u8);
            }
            pixels.update(&previous, &current, width, height);
            for (rgba, rgb) in pixels
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(current.as_chunks::<3>().0)
            {
                assert_eq!(&rgba[..3], rgb);
                assert_eq!(rgba[3], 255);
            }
            previous = current;
        }
    }
}
