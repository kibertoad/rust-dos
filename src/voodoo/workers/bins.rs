//! Bin commands by owned rows, retaining command order at every row.

use super::{Job, Stats};
use crate::voodoo::raster::{self, Scanline};
use crate::voodoo::setup::scan::Triangle;
use std::ops::Range;

enum Work {
    Rows {
        job: usize,
        rows: Range<i32>,
    },
    Words {
        job: usize,
        words: Range<usize>,
    },
    #[cfg(test)]
    Panic,
}

pub(super) struct Batch {
    jobs: Vec<Job>,
    triangles: Vec<Option<(Triangle, Scanline)>>,
    bins: Vec<Vec<Work>>,
}

fn bin_segment(
    bins: &mut [Vec<Work>],
    job: usize,
    rows: Range<i32>,
    screen: i32,
    descending: bool,
) {
    let workers = bins.len();
    for (k, bin) in bins.iter_mut().enumerate() {
        let difference = if descending {
            screen as i64 - k as i64
        } else {
            k as i64 - screen as i64
        };
        let offset = difference.rem_euclid(workers as i64);
        if offset < (rows.end - rows.start) as i64 {
            bin.push(Work::Rows {
                job,
                rows: rows.start + offset as i32..rows.end,
            });
        }
    }
}

fn bin_rows(
    bins: &mut [Vec<Work>],
    job: usize,
    rows: Range<i32>,
    descending: bool,
    screen_y: impl Fn(i32) -> i32,
) {
    let mut first = None;
    let mut previous = 0;
    let direction = if descending { -1 } else { 1 };
    for y in rows.clone() {
        let screen = screen_y(y);
        // Split at the flipped Y origin's wrap. Within a monotonic
        // segment each worker draws an arithmetic progression of rows.
        if (screen < 0 || screen != previous + direction)
            && let Some((start, value)) = first.take()
        {
            bin_segment(bins, job, start..y, value, descending);
        }
        if screen >= 0 && first.is_none() {
            first = Some((y, screen));
        }
        previous = screen;
    }
    if let Some((start, screen)) = first {
        bin_segment(bins, job, start..rows.end, screen, descending);
    }
}

impl Batch {
    pub(super) fn new(jobs: Vec<Job>, workers: usize) -> Self {
        let mut bins: Vec<Vec<Work>> = (0..workers).map(|_| Vec::new()).collect();
        let mut triangles = Vec::with_capacity(jobs.len());
        for (index, job) in jobs.iter().enumerate() {
            let mut prepared = None;
            match job {
                Job::Triangle {
                    st,
                    verts,
                    texcount,
                    ..
                } => {
                    if let Some(triangle) = Triangle::new(*verts) {
                        let flip = st.fbz_mode & (1 << 17) != 0;
                        bin_rows(&mut bins, index, triangle.rows.clone(), flip, |y| {
                            raster::screen_y(st, y, flip)
                        });
                        let draw = raster::pipeline(raster::variant(st, *texcount), *texcount);
                        prepared = Some((triangle, draw));
                    }
                }
                Job::Fastfill { st, y0, y1, .. } => {
                    let flip = st.fbz_mode & (1 << 17) != 0;
                    bin_rows(&mut bins, index, *y0..*y1, flip, |y| {
                        raster::screen_y(st, y, flip)
                    });
                }
                Job::Pixels { words, .. } => {
                    let mut first = 0;
                    while first < words.len() {
                        let owner = words[first].row as usize % workers;
                        let mut end = first + 1;
                        while end < words.len() && words[end].row as usize % workers == owner {
                            end += 1;
                        }
                        bins[owner].push(Work::Words {
                            job: index,
                            words: first..end,
                        });
                        first = end;
                    }
                }
                #[cfg(test)]
                Job::Panic => bins[0].push(Work::Panic),
            }
            triangles.push(prepared);
        }
        Self {
            jobs,
            triangles,
            bins,
        }
    }

    pub(super) fn run(&self, worker: usize, stipple: &mut u32, stats: &mut Stats) {
        for work in &self.bins[worker] {
            match work {
                Work::Rows { job, rows } => match &self.jobs[*job] {
                    Job::Triangle { st, p, .. } => {
                        let (triangle, draw) = self.triangles[*job].as_ref().unwrap();
                        triangle.draw(rows.clone().step_by(self.bins.len()), |y, x0, x1| {
                            draw(st, p, y, x0, x1, stipple, stats);
                        });
                    }
                    Job::Fastfill {
                        st, dither, x0, x1, ..
                    } => {
                        for y in rows.clone().step_by(self.bins.len()) {
                            raster::fastfill_row(st, dither, y, *x0, *x1, stats);
                        }
                    }
                    _ => unreachable!("row work belongs to a draw command"),
                },
                Work::Words {
                    job,
                    words: indices,
                } => {
                    let Job::Pixels { fb, words } = &self.jobs[*job] else {
                        unreachable!("word work belongs to frame buffer writes");
                    };
                    for word in &words[indices.clone()] {
                        if (word.at as usize) < fb.len() {
                            fb.set(word.at as usize, word.value);
                        }
                    }
                }
                #[cfg(test)]
                Work::Panic => panic!("a worker failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bins_cover_each_owned_row_once_in_order_including_y_origin_wrap() {
        for workers in [1, 2, 4, 7, 64] {
            for (origin, mask, flip) in [(479, 1023, false), (31, 1023, true), (37, -1, true)] {
                let screen = |y| if flip { (origin - y) & mask } else { y };
                let mut bins: Vec<Vec<Work>> = (0..workers).map(|_| Vec::new()).collect();
                bin_rows(&mut bins, 0, -2048..4096, flip, screen);
                let mut seen = vec![0; 6144];
                for (k, bin) in bins.iter().enumerate() {
                    let mut previous = -2049;
                    for work in bin {
                        let Work::Rows { rows, .. } = work else {
                            unreachable!()
                        };
                        for y in rows.clone().step_by(workers) {
                            assert!(y > previous);
                            previous = y;
                            assert!(screen(y) >= 0);
                            assert_eq!(screen(y) as usize % workers, k);
                            seen[(y + 2048) as usize] += 1;
                        }
                    }
                }
                for y in -2048..4096 {
                    assert_eq!(seen[(y + 2048) as usize], u32::from(screen(y) >= 0));
                }
            }
        }
    }
}
