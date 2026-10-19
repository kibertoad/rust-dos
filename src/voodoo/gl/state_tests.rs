use super::*;
use crate::voodoo::mirror::{DrawState, Layout};

fn snapshot() -> Command {
    Command::Resync(Box::new(Snapshot {
        layout: Layout {
            width: 8,
            height: 8,
            rowpixels: 8,
            color: vec![0, 64],
            aux: Some(128),
        },
        color: vec![vec![0; 64]; 2],
        aux: Some(vec![0xFFFF; 64]),
    }))
}

fn state() -> DrawState {
    DrawState {
        dest: 0,
        fbz_color_path: 0,
        fbz_mode: (1 << 4) | (1 << 5) | (1 << 9) | (1 << 10),
        alpha_mode: 0,
        fog_mode: 0,
        za_color: 0,
        chroma_key: 0,
        chroma_range: 0,
        color0: 0,
        color1: 0,
        fog_color: 0,
        stipple: 0,
        yorigin: 7,
        clip: None,
        aux: true,
        fogblend: [0; 64],
        fogdelta: [0; 64],
        send_config: None,
        tmu: [None; 2],
    }
}

fn triangle(state: DrawState, color: [f32; 4], z: f32) -> Command {
    Command::Draw(Box::new(Draw {
        state,
        vertices: [[0.0, 0.0], [16.0, 0.0], [0.0, 16.0]]
            .map(|pos| Vertex {
                pos,
                color,
                zw: [z, 0.0],
                ..Vertex::default()
            })
            .to_vec(),
    }))
}

fn recording(commands: Vec<Command>) -> Recording {
    Recording {
        commands,
        front: Some(0),
        output: true,
        width: 8,
        height: 8,
        clut: [0; 33],
    }
}

fn foreign_state(gl: &glow::Context) {
    // SAFETY: this test owns the current context. These are valid states
    // another frontend pass may leave before replay is entered again.
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.viewport(1, 1, 2, 2);
        gl.enable(glow::BLEND);
        gl.blend_func_separate(glow::ZERO, glow::ONE, glow::ZERO, glow::ONE);
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::NEVER);
        gl.depth_mask(false);
        gl.color_mask(false, true, false, false);
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(0, 0, 1, 1);
        gl.enable(glow::STENCIL_TEST);
        gl.stencil_func(glow::NEVER, 0, 0xFF);
        gl.enable(glow::CULL_FACE);
        gl.cull_face(glow::FRONT_AND_BACK);
    }
}

fn buffers(gl: &glow::Context, renderer: &VoodooGl) -> Vec<u8> {
    let mut result = Vec::new();
    for offset in [0, 64] {
        let target = &renderer.targets[&offset];
        target.resolve(gl, (8, 8));
        let mut pixels = vec![0; 8 * 8 * 4];
        // SAFETY: this test owns the current context and these targets.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.fbo));
            gl.read_pixels(
                0,
                0,
                8,
                8,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            result.extend_from_slice(&pixels);
            if renderer.samples == 1 {
                gl.read_pixels(
                    0,
                    0,
                    8,
                    8,
                    glow::DEPTH_COMPONENT,
                    glow::FLOAT,
                    glow::PixelPackData::Slice(Some(&mut pixels)),
                );
                result.extend_from_slice(&pixels);
            }
        }
    }
    result
}

#[test]
#[ignore = "requires a desktop OpenGL context"]
fn cached_replay_matches_rebinding_each_command() {
    let sdl = sdl2::init().unwrap();
    let video = sdl.video().unwrap();
    let attrs = video.gl_attr();
    attrs.set_context_profile(sdl2::video::GLProfile::Core);
    attrs.set_context_version(3, 3);
    let window = video
        .window("Replay state regression", 8, 8)
        .opengl()
        .hidden()
        .build()
        .unwrap();
    let _context = window.gl_create_context().unwrap();
    // SAFETY: this thread owns the current SDL OpenGL context.
    let gl =
        unsafe { glow::Context::from_loader_function(|s| video.gl_get_proc_address(s).cast()) };
    for samples in [1, 4] {
        let mut cached = VoodooGl::new(&gl, Glsl::Gl150, 1, samples, 1).unwrap();
        let mut rebound = VoodooGl::new(&gl, Glsl::Gl150, 1, samples, 1).unwrap();
        // The baseline sends every state call, so a wrongly skipped one
        // in the cached replay shows up as a difference.
        rebound.state.bypass = true;
        for frame in 0..4 {
            let mut commands = if frame % 2 == 0 {
                vec![snapshot()]
            } else {
                Vec::new()
            };
            for n in 0..24 {
                let mut state = state();
                state.dest = if n % 5 == 0 { 64 } else { 0 };
                state.clip = match n % 4 {
                    0 => Some([0, 4, 0, 8]),
                    1 => Some([2, 8, 2, 8]),
                    _ => None,
                };
                match n % 8 {
                    0 => state.fbz_mode |= 1 << 20,
                    1 => state.fbz_mode &= !(1 << 10),
                    2 => state.fbz_mode &= !(1 << 4),
                    3 => state.fbz_mode = 1 << 9,
                    _ => {}
                }
                if n % 3 == 0 {
                    state.alpha_mode = (1 << 4) | (1 << 8) | (5 << 12) | (4 << 16);
                }
                let color = [30.0 + n as f32 * 7.0, 180.0, 40.0, 128.0];
                // Consecutive draws share GL state despite new vertices.
                commands.push(triangle(state.clone(), color, 30000.0 - n as f32 * 500.0));
                commands.push(triangle(state, color, 25000.0 - n as f32 * 500.0));
                if n % 4 == 0 {
                    commands.extend([
                        Command::Pixels(Pixels {
                            dest: Some(0),
                            x: 1,
                            y: 3,
                            values: vec![0xF800; 4],
                        }),
                        Command::Pixels(Pixels {
                            dest: None,
                            x: 1,
                            y: 3,
                            values: vec![0xFFFF; 4],
                        }),
                        Command::Fill(Fill {
                            dest: 0,
                            rect: [4, 8, 0, 3],
                            color: Some(0x2040C0),
                            aux: Some(0xFFFF),
                            alpha_planes: false,
                        }),
                        Command::Fill(Fill {
                            dest: 0,
                            rect: [0, 3, 5, 8],
                            color: None,
                            aux: Some(0x80 + n as u16),
                            alpha_planes: true,
                        }),
                    ]);
                }
                if n == 12 && frame % 2 == 0 {
                    // Recreating targets can recycle framebuffer names.
                    commands.push(snapshot());
                }
            }
            foreign_state(&gl);
            cached.run(&gl, recording(commands.clone()));
            for command in commands {
                foreign_state(&gl);
                rebound.run(&gl, recording(vec![command]));
            }
            assert_eq!(
                buffers(&gl, &cached),
                buffers(&gl, &rebound),
                "samples={samples}, frame={frame}"
            );
            let screen = Frame::from_rgb(8, 8, vec![0; 8 * 8 * 3]);
            // Composite and resolve use GL outside the replay state cache.
            // The next frame must still establish all of its own state.
            for renderer in [&mut cached, &mut rebound] {
                renderer.composite(&gl, &screen, &screen).unwrap();
            }
            // SAFETY: all queried operations belong to this context.
            unsafe { assert_eq!(gl.get_error(), glow::NO_ERROR) };
        }
        cached.destroy(&gl);
        rebound.destroy(&gl);
    }
}
