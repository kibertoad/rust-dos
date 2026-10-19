//! The on-screen message: a line at the top of the picture for what the
//! hotkeys did ("Paused", "CPU speed 20000", "Sound off"), drawn like the
//! settings window over the picture. A message lasts a moment; one that
//! stays, such as "Paused", shows until it is cleared, under the passing
//! ones.
//!
//! Notifications, such as an achievement unlocked, are two lines at the
//! bottom, one after another; and the values of the leaderboards being
//! attempted show in the bottom right corner.

use super::draw::{self, Grid, Layout};
use crate::video::Frame;
use web_time::{Duration, Instant};

/// How long a passing message shows.
const SHOWN: Duration = Duration::from_millis(2000);
/// How long a notification shows, and a big one.
const NOTIFIED: Duration = Duration::from_millis(3500);
const NOTIFIED_BIG: Duration = Duration::from_millis(6000);

#[derive(Default)]
pub struct Osd {
    /// A message that shows until `clear_lasting`.
    lasting: Option<String>,
    /// A message that shows until the time.
    passing: Option<(String, Instant)>,
    /// Notifications waiting, and the one showing until its time.
    queued: std::collections::VecDeque<(String, String, bool)>,
    notice: Option<(String, String, Instant)>,
    /// What shows in the bottom right corner.
    corner: Vec<String>,
}

impl Osd {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether nothing shows (or is about to).
    pub fn is_empty(&self) -> bool {
        self.lasting.is_none() && self.passing.is_none() && self.queued.is_empty() && self.notice.is_none() && self.corner.is_empty()
    }

    /// Show `text` for a moment.
    pub fn show(&mut self, text: impl Into<String>) {
        self.show_at(text, Instant::now());
    }

    fn show_at(&mut self, text: impl Into<String>, now: Instant) {
        self.passing = Some((text.into(), now + SHOWN));
    }

    /// Show `text` until `clear_lasting`.
    pub fn show_lasting(&mut self, text: impl Into<String>) {
        self.lasting = Some(text.into());
    }

    pub fn clear_lasting(&mut self) {
        self.lasting = None;
    }

    /// A notification: `title` over `detail`, shown longer if `big`.
    pub fn notify(&mut self, title: impl Into<String>, detail: impl Into<String>, big: bool) {
        self.queued.push_back((title.into(), detail.into(), big));
    }

    /// The lines of the bottom right corner (leaderboard values).
    pub fn set_corner(&mut self, lines: Vec<String>) {
        self.corner = lines;
    }

    /// The notification to show at `now`, if any.
    fn notice_at(&mut self, now: Instant) -> Option<(&str, &str)> {
        if self.notice.as_ref().is_some_and(|(_, _, until)| now >= *until) {
            self.notice = None;
        }
        if self.notice.is_none()
            && let Some((title, detail, big)) = self.queued.pop_front()
        {
            self.notice = Some((title, detail, now + if big { NOTIFIED_BIG } else { NOTIFIED }));
        }
        self.notice.as_ref().map(|(t, d, _)| (t.as_str(), d.as_str()))
    }

    /// The message to show at `now`, if any.
    fn text_at(&mut self, now: Instant) -> Option<&str> {
        if self.passing.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.passing = None;
        }
        self.passing.as_ref().map(|(text, _)| text.as_str()).or(self.lasting.as_deref())
    }

    /// Draw the message, if there is one, over `frame`.
    pub fn draw(&mut self, frame: &mut Frame) {
        self.draw_at(frame, Instant::now());
    }

    /// Draw messages and report their actual rows, including notices
    /// that became visible after the base picture was composed.
    pub fn draw_rows(&mut self, frame: &mut Frame) -> std::ops::Range<usize> {
        self.draw_at(frame, Instant::now())
    }

    fn draw_at(&mut self, frame: &mut Frame, now: Instant) -> std::ops::Range<usize> {
        use crate::video::compose::union;
        let rows = union(self.draw_notice(frame, now), self.draw_corner(frame));
        let Some(text) = self.text_at(now) else { return rows };
        let (width, height) = (frame.width as usize, frame.height as usize);
        let cols = (text.chars().count() + 2).min(width / 8);
        if cols < 3 {
            return rows;
        }
        let cell_h = if height >= 300 { 16 } else { 8 };
        let layout = Layout { x: (width - cols * 8) / 2, y: cell_h / 2, cell_h, cols, rows: 1 };
        let mut grid = Grid::new(cols, 1);
        grid.text_to(1, 0, text, draw::BRIGHT, cols - 1);
        draw::render(&grid, &layout, frame);
        union(rows, layout.y.min(height)..(layout.y + cell_h).min(height))
    }

    fn draw_notice(&mut self, frame: &mut Frame, now: Instant) -> std::ops::Range<usize> {
        let (width, height) = (frame.width as usize, frame.height as usize);
        let Some((title, detail)) = self.notice_at(now) else { return 0..0 };
        let len = title.chars().count().max(detail.chars().count());
        let cols = (len + 4).min(width / 8);
        let cell_h = if height >= 300 { 16 } else { 8 };
        if cols < 3 || height < cell_h * 4 {
            return 0..0;
        }
        let layout = Layout { x: (width - cols * 8) / 2, y: height - cell_h * 7 / 2, cell_h, cols, rows: 2 };
        let mut grid = Grid::new(cols, 2);
        grid.text_to(2, 0, title, draw::KEY, cols - 1);
        grid.text_to(2, 1, detail, draw::TEXT, cols - 1);
        draw::render(&grid, &layout, frame);
        layout.y..(layout.y + 2 * cell_h).min(height)
    }

    fn draw_corner(&mut self, frame: &mut Frame) -> std::ops::Range<usize> {
        let (width, height) = (frame.width as usize, frame.height as usize);
        let cell_h = if height >= 300 { 16 } else { 8 };
        let mut rows = 0..0;
        for (i, line) in self.corner.iter().rev().enumerate() {
            let cols = (line.chars().count() + 2).min(width / 8);
            let y = height.saturating_sub(cell_h * (i + 1) + cell_h / 2);
            if cols < 3 || y < cell_h {
                break;
            }
            let layout = Layout { x: width - cols * 8 - 8, y, cell_h, cols, rows: 1 };
            let mut grid = Grid::new(cols, 1);
            grid.text_to(1, 0, line, draw::BRIGHT, cols - 1);
            draw::render(&grid, &layout, frame);
            rows = crate::video::compose::union(rows, y..(y + cell_h).min(height));
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_bounds_restore_expired_messages_without_retinting_the_picture() {
        use crate::video::{compose::Composition, mono};
        for (width, height) in [(320, 200), (640, 400)] {
            let mut osd = Osd::new();
            let mut reference = Osd::new();
            let mut composition = Composition::default();
            let mut base = Frame::from_rgb(width, height, vec![120; (width * height * 3) as usize]);
            let mut output = base.clone();
            let start = Instant::now();
            let mut narrow = 0;
            for step in 0..20 {
                let now = start + Duration::from_secs(step);
                for messages in [&mut osd, &mut reference] {
                    if step % 4 == 0 {
                        messages.show_at("A completed capture", now);
                        messages.notify("An achievement", "Its detail", false);
                    }
                    let corner = if step % 3 == 0 { vec![] } else { vec!["1:23.45".into()] };
                    messages.set_corner(corner);
                }
                let y = step as usize * 7 % height as usize;
                let bytes = width as usize * 4;
                base.rgba[y * bytes..y * bytes + 3].fill(step as u8);
                let monitor = mono::Monochrome::ALL[step as usize / 5 % 4];
                let rows = composition.restore(&mut output, &base, y..y + 1, 0..0, monitor);
                mono::apply_rows(&mut output, monitor, rows);
                let painted = osd.draw_at(&mut output, now);
                if !painted.is_empty() && painted.len() < height as usize {
                    narrow += 1;
                }
                composition.include_overlay(painted);
                let mut full = base.clone();
                mono::apply(&mut full, monitor);
                reference.draw_at(&mut full, now);
                assert!(output == full, "{width}x{height}, step {step}");
            }
            assert!(narrow > 10);
        }
    }

    fn lit(frame: &Frame) -> bool {
        frame.rgba.as_chunks::<4>().0.iter().any(|p| p[..3].iter().any(|&b| b != 0))
    }

    #[test]
    fn a_message_shows_for_a_moment_over_the_lasting_one() {
        let mut osd = Osd::new();
        let start = Instant::now();
        let mut frame = Frame::new(640, 400);
        osd.draw_at(&mut frame, start);
        assert!(!lit(&frame), "nothing to show");

        osd.show_lasting("Paused");
        osd.show_at("CPU speed 20000", start);
        assert_eq!(osd.text_at(start), Some("CPU speed 20000"));
        assert_eq!(osd.text_at(start + SHOWN), Some("Paused"));
        osd.clear_lasting();
        assert_eq!(osd.text_at(start + SHOWN), None);

        osd.show_at("Sound off", start);
        osd.draw_at(&mut frame, start);
        assert!(lit(&frame));
        // At the top, centred.
        let row = |y: usize| {
            frame.rgba[y * 640 * 4..(y + 1) * 640 * 4]
                .as_chunks::<4>().0.iter()
                .any(|p| p[..3].iter().any(|&b| b != 0))
        };
        assert!(row(10) && !row(100));
        // Even on a tiny picture.
        osd.draw_at(&mut Frame::new(16, 8), start);
        let rows = osd.draw_at(&mut Frame::new(640, 1), start);
        assert!(rows.is_empty() && rows.end <= 1);
    }

    #[test]
    fn notifications_show_one_after_another_at_the_bottom() {
        let mut osd = Osd::new();
        let start = Instant::now();
        osd.notify("Achievement unlocked: First", "Byte 0 is 1 (5 points)", true);
        osd.notify("Leaderboard started", "Fast", false);
        assert_eq!(osd.notice_at(start), Some(("Achievement unlocked: First", "Byte 0 is 1 (5 points)")));
        assert_eq!(osd.notice_at(start + NOTIFIED), Some(("Achievement unlocked: First", "Byte 0 is 1 (5 points)")));
        assert_eq!(osd.notice_at(start + NOTIFIED_BIG), Some(("Leaderboard started", "Fast")));
        assert_eq!(osd.notice_at(start + NOTIFIED_BIG + NOTIFIED), None);
        let mut frame = Frame::new(640, 400);
        osd.notify("A", "B", false);
        osd.set_corner(vec!["1:23.45".into()]);
        osd.draw_at(&mut frame, start);
        let row = |y: usize| {
            frame.rgba[y * 640 * 4..(y + 1) * 640 * 4]
                .as_chunks::<4>().0.iter()
                .any(|p| p[..3].iter().any(|&b| b != 0))
        };
        assert!(row(350) && !row(100) && !row(10));
        osd.draw_at(&mut Frame::new(16, 8), start);
    }
}
