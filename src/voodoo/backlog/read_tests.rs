use super::*;
use crate::voodoo::raster::Stats;

fn queue() -> Backlog {
    Backlog {
        layout: TileLayout::new(
            [Some(0), Some(640 * 480), None, Some(2 * 640 * 480)],
            640,
            480,
            640,
            3 * 640 * 480,
        ),
        ..Default::default()
    }
}

fn append(queue: &mut Backlog, slot: usize, tiles: u64, pinned: bool) {
    let mut access = Access {
        pinned,
        ..Default::default()
    };
    access.writes[slot] = tiles;
    assert!(
        queue
            .push(
                Job::Pixels {
                    fb: Vram::new(2),
                    words: vec![]
                },
                access
            )
            .is_empty()
    );
}

#[test]
fn reads_release_only_the_required_prefix_including_pinned_jobs() {
    let mut q = queue();
    append(&mut q, 0, 1, false);
    append(&mut q, 1, 1, false);
    append(&mut q, 0, 4, false);
    assert_eq!(q.take_for_read([0, 1]).len(), 1);
    assert_eq!(q.len(), 2);
    assert!(q.take_for_read([0, 1]).is_empty());
    append(&mut q, 0, 0, true);
    append(&mut q, 1, 2, false);
    assert_eq!(q.take_for_read([0, 1]).len(), 3);
    assert_eq!(q.len(), 1);
    assert_eq!(q.take_for_read([640 * 480 + 80, 640 * 480 + 81]).len(), 1);
    assert!(q.is_empty());
}

#[test]
fn pending_write_cache_is_invalidated_by_merged_words_and_drains() {
    let mut q = queue();
    let fb = Vram::new(3 * 640 * 480 * 2);
    let a = Word {
        row: 0,
        at: 640 * 480,
        value: 1,
    };
    q.push_words(&fb, &[a]);
    assert!(q.take_for_read([0, 1]).is_empty());
    assert!(q.pending_access.is_some());
    q.push_words(&fb, &[Word { at: 0, ..a }]);
    assert!(q.pending_access.is_none());
    assert_eq!(q.take_for_read([0, 1]).len(), 1);
    append(&mut q, 0, 1, false);
    assert_eq!(q.take_for_read([0, 1]).len(), 1);
    append(&mut q, 0, 1, false);
    assert_eq!(q.take().len(), 1);
    assert!(q.take_for_read([0, 1]).is_empty());
}

#[test]
fn raw_word_tiles_handle_buffer_edges_padding_and_invalid_layouts() {
    let mut q = queue();
    assert_eq!(q.layout.word_tiles([79, 80]), [3, 0, 0, 0]);
    assert_eq!(
        q.layout.word_tiles([640 * 480 - 1, 640 * 480]),
        [1 << 63, 1, 0, 0]
    );
    q.layout.rowpixels = 1024;
    assert_eq!(q.layout.word_tiles([640, 1023]), [0; 4]);
    append(&mut q, 0, 1, false);
    q.layout.prunable = false;
    assert_eq!(q.take_for_read([usize::MAX; 2]).len(), 1);
}

#[test]
fn interleaved_writes_and_partial_reads_match_immediate_memory() {
    let mut q = queue();
    let bytes = 3 * 640 * 480 * 2;
    let fb = Vram::new(bytes);
    let expected = Vram::new(bytes);
    let mut random = 0x123456789ABCDEFu64;
    for step in 0..8192 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let row = (random % 480) as usize;
        let x = (random >> 16) as usize % 639;
        let slot = (random >> 32) as usize % 3;
        let at = slot * 640 * 480 + row * 640 + x;
        if step % 3 == 0 {
            for job in q.take_for_read([at, at + 1]) {
                job.run_all(&mut 0, &mut Stats::default());
            }
            assert_eq!(fb.get(at), expected.get(at));
            assert_eq!(fb.get(at + 1), expected.get(at + 1));
        } else {
            let word = Word {
                row: row as u16,
                at: at as u32,
                value: random as u16,
            };
            let access = words(&q.layout, &[word]);
            let released = q.push(
                Job::Pixels {
                    fb: fb.clone(),
                    words: vec![word],
                },
                access,
            );
            for job in released {
                job.run_all(&mut 0, &mut Stats::default());
            }
            expected.set(at, word.value);
        }
    }
    for job in q.take() {
        job.run_all(&mut 0, &mut Stats::default());
    }
    assert_eq!(fb.to_bytes(), expected.to_bytes());
}

#[test]
fn direct_word_tiles_match_rectangles_at_irregular_dimensions() {
    for (width, height) in [(1, 1), (3, 5), (17, 13), (641, 479)] {
        let stride = width + 7;
        let words = stride as usize * height as usize;
        let layout = TileLayout::new([Some(0), None, None, None], width, height, stride, words);
        for y in 0..height {
            for x in 0..width {
                let at = (y * stride + x) as usize;
                assert_eq!(
                    layout.word_tiles([at; 2]),
                    [layout.tiles((x, x + 1, y, y + 1), false), 0, 0, 0]
                );
            }
        }
    }
}

#[test]
fn immediate_writes_follow_older_readers_as_well_as_writers() {
    let mut q = queue();
    let mut reader = Access::default();
    reader.reads[0] = 1;
    reader.writes[1] = 1;
    q.push(
        Job::Pixels {
            fb: Vram::new(2),
            words: vec![],
        },
        reader,
    );
    append(&mut q, 1, 2, false);
    assert!(q.take_for_read([0, 1]).is_empty());
    assert!(q.pending_access.is_some());
    let mut write = Access::default();
    write.writes[0] = 1;
    assert_eq!(q.take_for_access(write).len(), 1);
    assert_eq!(q.len(), 1);
    assert_eq!(q.take_for_access(Access::PINNED).len(), 1);
}

fn pipeline_state() -> RasterState {
    use crate::voodoo::raster::Stride;
    RasterState {
        fb: Vram::new(3 * 640 * 480 * 2),
        fbz_color_path: 0,
        fbz_mode: 1 << 9,
        alpha_mode: 0,
        fog_mode: 0,
        za_color: 0,
        chroma_key: 0,
        color0: 0,
        color1: 0,
        fog_color: 0,
        clip_left_right: 640,
        clip_low_y_high_y: 480,
        clip_mask: 0xFFF,
        chroma_range: 0,
        fogdelta_mask: 0xFC,
        yorigin: 479,
        ymask: 0x3FF,
        rowpixels: 640,
        stride: Stride::linear(640),
        aux_stride: Stride::linear(640),
        dest: 0,
        aux: Some(2 * 640 * 480),
        fogblend: [0; 64],
        fogdelta: [0; 64],
        send_config: None,
        stipple: 0,
        tmu: [None, None],
    }
}

#[test]
fn pipeline_accesses_cover_colour_mirror_depth_tests_and_alpha_planes() {
    let q = queue();
    let mut st = pipeline_state();
    let access = lfb(&q.layout, &st, 0, 0);
    assert_eq!(access.reads, [1, 0, 0, 0]);
    assert_eq!(access.writes, [1, 0, 0, 0]);
    st.fbz_mode |= 1 << 4;
    for function in 0..8 {
        st.fbz_mode = (st.fbz_mode & !(7 << 5)) | function << 5;
        assert_eq!(
            lfb(&q.layout, &st, 0, 0).reads[3],
            u64::from((1..=6).contains(&function))
        );
    }
    st.fbz_mode = 1 << 18;
    st.alpha_mode = 1 << 4;
    assert_eq!(lfb(&q.layout, &st, 0, 0).reads, [1, 0, 0, 1]);
    assert_eq!(lfb(&q.layout, &st, 0, 0).writes, [0; 4]);
    st.fbz_mode |= 1 << 10;
    assert_eq!(lfb(&q.layout, &st, 0, 0).writes, [0, 0, 0, 1]);
    assert!(lfb(&q.layout, &st, -1, 0).pinned);
    assert!(lfb(&q.layout, &st, 0, -1).pinned);
    st.stride.tiled = true;
    assert!(lfb(&q.layout, &st, 0, 0).pinned);
}
