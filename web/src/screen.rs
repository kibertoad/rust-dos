//! Changed-row metadata for the canonical RGBA picture.

use rust_dos::video;
use std::ops::Range;

#[derive(Default)]
pub(super) struct Rows {
    pub(super) rows: Range<usize>,
    size: (usize, usize),
}

impl Rows {
    /// The composed Frame owns the complete uploadable picture. Only
    /// geometry and the changed band are kept here.
    pub(super) fn update(
        &mut self,
        previous: &[u8],
        current: &[u8],
        width: usize,
        height: usize,
        candidates: Range<usize>,
    ) -> bool {
        let row_bytes = width * 4;
        assert_eq!(current.len(), row_bytes * height);
        let full = self.size != (width, height) || previous.len() != current.len();
        self.size = (width, height);
        self.rows = if full {
            0..height
        } else {
            video::changed_rows_in(previous, current, row_bytes, candidates).unwrap_or(0..0)
        };
        !self.rows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_blank_picture_is_uploaded_and_an_unchanged_one_is_not() {
        let mut rows = Rows::default();
        let rgba = vec![0; 4 * 5 * 4];
        assert!(rows.update(&rgba, &rgba, 4, 5, 0..5));
        assert_eq!(rows.rows, 0..5);
        assert!(!rows.update(&rgba, &rgba, 4, 5, 0..5));
        assert_eq!(rows.rows, 0..0);
    }

    #[test]
    fn changes_at_either_edge_and_in_separate_rows_bound_the_upload() {
        let mut rows = Rows::default();
        let mut previous = vec![0; 4 * 5 * 4];
        rows.update(&[], &previous, 4, 5, 0..5);
        for expected in [0..1, 4..5, 1..4, 2..3] {
            let mut current = previous.clone();
            current[expected.start * 16] ^= 0x53;
            current[expected.end * 16 - 1] ^= 0x19;
            assert!(rows.update(&previous, &current, 4, 5, 0..5));
            assert_eq!(rows.rows, expected);
            previous = current;
        }
    }

    #[test]
    fn a_mode_change_uploads_every_row_even_with_the_same_byte_count() {
        let mut rows = Rows::default();
        let rgba = vec![23; 4 * 5 * 4];
        rows.update(&[], &rgba, 4, 5, 0..5);
        assert!(rows.update(&rgba, &rgba, 5, 4, 0..4));
        assert_eq!(rows.rows, 0..4);
        assert!(rows.update(&rgba, &rgba[..4], 1, 1, 0..1));
        assert_eq!(rows.rows, 0..1);
    }

    #[test]
    fn repeated_overlay_updates_include_the_new_and_erased_rows() {
        let (width, height) = (37, 29);
        let mut previous = vec![0; width * height * 4];
        let mut rows = Rows::default();
        rows.update(&[], &previous, width, height, 0..height);
        for frame in 1..200 {
            let mut current = previous.clone();
            let a = frame % height;
            let b = (frame + height - 1) % height;
            for row in [a, b] {
                current[row * width * 4..(row + 1) * width * 4].fill(frame as u8);
            }
            assert!(rows.update(&previous, &current, width, height, a.min(b)..a.max(b) + 1));
            assert_eq!(rows.rows, a.min(b)..a.max(b) + 1);
            previous = current;
        }
    }

    #[test]
    fn bounded_changes_keep_their_frame_row_offsets() {
        let mut rows = Rows::default();
        let old = vec![0; 4 * 5 * 4];
        rows.update(&[], &old, 4, 5, 0..0);
        let mut new = old.clone();
        new[2 * 16] = 9;
        new[4 * 16 - 1] = 7;
        assert!(rows.update(&old, &new, 4, 5, 1..5));
        assert_eq!(rows.rows, 2..4);
        assert!(!rows.update(&new, &new, 4, 5, 2..4));
        assert_eq!(rows.rows, 0..0);
        assert!(!rows.update(&new, &new, 4, 5, 0..0));
    }
}
