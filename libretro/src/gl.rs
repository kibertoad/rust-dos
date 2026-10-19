//! The picture drawn with the frontend's OpenGL (libretro's hardware
//! rendering), for the 3dfx card's OpenGL renderer (`voodoo_renderer=
//! opengl`): the card's picture at `voodoo_scale` times its resolution, as
//! the rust-dos window draws it (`rust_dos::voodoo::gl`), and every other
//! picture uploaded as a texture. The frontend makes the context when it
//! likes (`context_reset`), and may take it away and make another
//! (`context_destroy`); the card's recording then starts again from its
//! memory.

use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_void};
use std::num::NonZeroU32;

use glow::HasContext;
use rust_dos::bus::Bus;
use rust_dos::video::Frame;
use rust_dos::video::shader::{self, Glsl, Shader};
use rust_dos::voodoo::gl::VoodooGl;
use rust_dos::voodoo::{Renderer, VoodooSettings};

use crate::Callbacks;
use crate::ffi::*;
use crate::present::Pixels;

thread_local! {
    /// The frontend's hardware rendering, once it agreed to it.
    static HW: Cell<Option<retro_hw_render_callback>> = const { Cell::new(None) };
    /// What is drawn with the context the frontend made, while there is one.
    static SCREEN: RefCell<Option<Screen>> = const { RefCell::new(None) };
}

/// Ask the frontend for an OpenGL 3.2 core context, or a compatibility one
/// (of 3.0 or later, as the shaders need): whether it has one to give.
pub fn request(cb: &Callbacks) -> bool {
    let contexts = [(RETRO_HW_CONTEXT_OPENGL_CORE, 3, 2), (RETRO_HW_CONTEXT_OPENGL, 3, 0)];
    for (context_type, version_major, version_minor) in contexts {
        let mut hw = retro_hw_render_callback {
            context_type,
            context_reset: Some(context_reset),
            get_current_framebuffer: None,
            get_proc_address: None,
            depth: false,
            stencil: false,
            // The picture's first row is at the top, as OpenGL draws.
            bottom_left_origin: true,
            version_major,
            version_minor,
            cache_context: false,
            context_destroy: Some(context_destroy),
            debug_context: false,
        };
        // SAFETY: SET_HW_RENDER takes a retro_hw_render_callback, and
        // fills in the frontend's functions.
        if unsafe { cb.env(RETRO_ENVIRONMENT_SET_HW_RENDER, &mut hw as *mut _ as *mut c_void) } {
            HW.with(|cell| cell.set(Some(hw)));
            return true;
        }
    }
    false
}

/// Whether the frontend draws with OpenGL for the core: every picture has
/// to go through `present` then.
pub fn granted() -> bool {
    HW.with(Cell::get).is_some()
}

/// Run `f` on what is drawn with the context, if there is one.
pub fn with_screen<R>(f: impl FnOnce(&mut Screen) -> R) -> Option<R> {
    SCREEN.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

/// The frontend made a context: everything is made again in it.
unsafe extern "C" fn context_reset() {
    let Some(hw) = HW.with(Cell::get) else { return };
    let (Some(proc_address), Some(framebuffer)) = (hw.get_proc_address, hw.get_current_framebuffer) else { return };
    // SAFETY: the frontend's context is current while it resets it, and
    // its functions are what it hands out for it.
    let gl = unsafe {
        glow::Context::from_loader_function(|name| {
            let Ok(name) = CString::new(name) else { return std::ptr::null() };
            proc_address(name.as_ptr()).map_or(std::ptr::null(), |f| f as *const c_void)
        })
    };
    let screen = Screen::new(gl, framebuffer);
    if let Err(e) = &screen {
        crate::callbacks().log(RETRO_LOG_ERROR, &format!("The frontend's OpenGL can't draw: {}", e));
    }
    SCREEN.with(|cell| {
        if let Ok(mut current) = cell.try_borrow_mut() {
            *current = screen.ok();
        }
    });
}

/// The frontend takes its context away: what was made in it goes, while
/// it is still current.
unsafe extern "C" fn context_destroy() {
    SCREEN.with(|cell| {
        if let Some(screen) = cell.try_borrow_mut().ok().and_then(|mut s| s.take()) {
            screen.destroy();
        }
    });
}

pub struct Screen {
    gl: glow::Context,
    glsl: Glsl,
    framebuffer: retro_hw_get_current_framebuffer_t,
    /// The plain look: a texture over the whole viewport.
    program: glow::Program,
    vao: glow::VertexArray,
    /// The machine's picture when the card's isn't shown.
    texture: glow::Texture,
    texture_size: (u32, u32),
    upload: Pixels<[u8; 4]>,
    last_composite: bool,
    voodoo: Option<VoodooGl>,
    /// The scale, samples and anisotropy `voodoo` draws with.
    voodoo_key: Option<(u32, u32, u32)>,
    /// Why OpenGL can't draw the card, once it couldn't.
    voodoo_failed: Option<String>,
}

impl Screen {
    fn new(gl: glow::Context, framebuffer: retro_hw_get_current_framebuffer_t) -> Result<Self, String> {
        let version = gl.version();
        let glsl = Glsl::for_gl(version.major, version.minor, version.is_embedded)
            .ok_or_else(|| format!("OpenGL {}.{} is older than 3.0", version.major, version.minor))?;
        let program = compile(&gl, glsl)?;
        // SAFETY: the context is current (`context_reset`).
        unsafe {
            let vao = gl.create_vertex_array()?;
            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for (name, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            Ok(Screen {
                gl,
                glsl,
                framebuffer,
                program,
                vao,
                texture,
                texture_size: (0, 0),
                upload: Pixels::default(),
                last_composite: false,
                voodoo: None,
                voodoo_key: None,
                voodoo_failed: None,
            })
        }
    }

    fn destroy(mut self) {
        self.drop_voodoo();
        // SAFETY: the context is current (`context_destroy`).
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_texture(self.texture);
        }
    }

    fn drop_voodoo(&mut self) {
        if let Some(voodoo) = self.voodoo.take() {
            voodoo.destroy(&self.gl);
        }
        self.voodoo_key = None;
    }

    /// With `voodoo_renderer=opengl`, draw what the 3dfx card recorded;
    /// true if its picture is the one to show (`present`'s `voodoo`).
    /// Otherwise the card stops recording, and its picture is drawn in
    /// software.
    pub fn run_voodoo(&mut self, bus: &mut Bus, settings: &VoodooSettings) -> bool {
        let Some(card) = bus.voodoo.as_mut() else {
            self.drop_voodoo();
            return false;
        };
        // As in the desktop frontend, the Banshee's desktop, video
        // overlay and hardware cursor need software scanout.
        if settings.renderer != Renderer::OpenGl
            || card.banshee.is_some()
            || self.voodoo_failed.is_some()
        {
            card.set_mirror(false);
            self.drop_voodoo();
            return false;
        }
        // A new card, context, scale or sampling setting: start again from
        // the card's memory.
        let key = (settings.scale, settings.msaa, settings.anisotropy);
        if !card.mirror_attached() || self.voodoo.is_none() || self.voodoo_key != Some(key) {
            card.set_mirror(false);
            self.drop_voodoo();
            match VoodooGl::new(&self.gl, self.glsl, key.0, key.1, key.2) {
                Ok(voodoo) => {
                    card.set_mirror(true);
                    self.voodoo = Some(voodoo);
                    self.voodoo_key = Some(key);
                }
                Err(e) => {
                    bus.log_string(&format!("[3DFX] voodoo_renderer=opengl: {}; the software renderer draws", e));
                    self.voodoo_failed = Some(e);
                    return false;
                }
            }
        }
        let Some(recording) = card.take_mirror() else { return false };
        let shown = recording.output && recording.front.is_some();
        let Some(voodoo) = &mut self.voodoo else { return false };
        voodoo.run(&self.gl, recording);
        shown
    }

    /// Draw the picture into the frontend's framebuffer: with `voodoo`,
    /// the machine's picture without what is drawn over it, the 3dfx
    /// card's picture `run_voodoo` drew with what `screen` has over it;
    /// else `screen`. Returns the size and whether a new frame was drawn.
    pub fn present(
        &mut self,
        screen: &Frame,
        voodoo: Option<&Frame>,
        can_dupe: bool,
    ) -> (u32, u32, bool) {
        let Screen {
            gl,
            framebuffer,
            program,
            vao,
            texture: own,
            texture_size,
            upload,
            last_composite,
            voodoo: card,
            ..
        } = self;
        let composite = voodoo.and_then(|base| card.as_mut()?.composite(gl, screen, base));
        let was_composite = std::mem::replace(last_composite, composite.is_some());
        // SAFETY: the frontend's context is current while the core runs.
        unsafe {
            let (texture, (w, h)) = match composite {
                Some(picture) => picture,
                None => {
                    let rows = upload.update(screen, |[r, g, b]| [r, g, b, 255]);
                    if rows.is_none() && !was_composite && can_dupe {
                        return (screen.width, screen.height, false);
                    }
                    gl.active_texture(glow::TEXTURE0);
                    gl.bind_texture(glow::TEXTURE_2D, Some(*own));
                    // The rows come from memory, whatever the frontend
                    // left set for its own uploads.
                    gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
                    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                    gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
                    gl.pixel_store_i32(glow::UNPACK_SKIP_ROWS, 0);
                    gl.pixel_store_i32(glow::UNPACK_SKIP_PIXELS, 0);
                    let (w, h) = (screen.width as i32, screen.height as i32);
                    if *texture_size != (screen.width, screen.height) {
                        gl.tex_image_2d(
                            glow::TEXTURE_2D,
                            0,
                            glow::RGBA8 as i32,
                            w,
                            h,
                            0,
                            glow::RGBA,
                            glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(None),
                        );
                        *texture_size = (screen.width, screen.height);
                    }
                    if let Some(rows) = rows {
                        let pixels = &upload.data[rows.start * w as usize..rows.end * w as usize];
                        gl.tex_sub_image_2d(
                            glow::TEXTURE_2D,
                            0,
                            0,
                            rows.start as i32,
                            w,
                            rows.len() as i32,
                            glow::RGBA,
                            glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(Some(pixels.as_flattened())),
                        );
                    }
                    (*own, (screen.width, screen.height))
                }
            };
            let target = NonZeroU32::new(framebuffer() as u32).map(glow::NativeFramebuffer);
            gl.bind_framebuffer(glow::FRAMEBUFFER, target);
            gl.viewport(0, 0, w as i32, h as i32);
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::CULL_FACE);
            gl.color_mask(true, true, true, true);
            gl.use_program(Some(*program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.bind_vertex_array(Some(*vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.use_program(None);
            (w, h, true)
        }
    }
}

/// The plain look's program (`shader::sources`), its texture on unit 0.
fn compile(gl: &glow::Context, glsl: Glsl) -> Result<glow::Program, String> {
    let (vertex, fragment) = shader::sources(Shader::None, glsl);
    // SAFETY: the context is current (`context_reset`).
    unsafe {
        let program = gl.create_program()?;
        let mut stages = Vec::new();
        let mut problem = None;
        for (kind, source) in [(glow::VERTEX_SHADER, vertex), (glow::FRAGMENT_SHADER, fragment)] {
            let stage = gl.create_shader(kind)?;
            gl.shader_source(stage, &source);
            gl.compile_shader(stage);
            gl.attach_shader(program, stage);
            stages.push(stage);
            if !gl.get_shader_compile_status(stage) {
                problem = Some(format!("the shader doesn't compile: {}", gl.get_shader_info_log(stage)));
                break;
            }
        }
        if problem.is_none() {
            if glsl != Glsl::Es300 {
                gl.bind_frag_data_location(program, 0, "o_color");
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                problem = Some(format!("the shader doesn't link: {}", gl.get_program_info_log(program)));
            }
        }
        for stage in stages {
            gl.detach_shader(program, stage);
            gl.delete_shader(stage);
        }
        if let Some(problem) = problem {
            gl.delete_program(program);
            return Err(problem);
        }
        gl.use_program(Some(program));
        let frame = gl.get_uniform_location(program, "u_frame");
        gl.uniform_1_i32(frame.as_ref(), 0);
        gl.use_program(None);
        Ok(program)
    }
}

#[cfg(all(test, feature = "gl-tests"))]
mod tests {
    use super::*;

    unsafe extern "C" fn default_framebuffer() -> usize {
        0
    }

    #[test]
    #[ignore = "requires a desktop OpenGL context"]
    fn partial_uploads_and_geometry_changes_preserve_the_picture() {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        video
            .gl_attr()
            .set_context_profile(sdl2::video::GLProfile::Core);
        video.gl_attr().set_context_version(3, 3);
        let window = video
            .window("Libretro upload regression", 8, 8)
            .opengl()
            .hidden()
            .build()
            .unwrap();
        let _context = window.gl_create_context().unwrap();
        // SAFETY: this thread owns the current SDL OpenGL context.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| video.gl_get_proc_address(name).cast())
        };
        let mut screen = Screen::new(gl, default_framebuffer).unwrap();
        let mut frame = Frame::from_rgb(3, 2, vec![17; 18]);
        assert_eq!(screen.present(&frame, None, true), (3, 2, true));
        assert_eq!(screen.present(&frame, None, true), (3, 2, false));
        frame.rgb[9..].fill(37);
        assert_eq!(screen.present(&frame, None, true), (3, 2, true));
        assert_eq!(screen.present(&frame, None, false), (3, 2, true));
        let read = |screen: &Screen, frame: &Frame| {
            let mut pixels = vec![0; (frame.width * frame.height * 4) as usize];
            // SAFETY: the texture and context belong to this thread.
            unsafe {
                screen
                    .gl
                    .bind_texture(glow::TEXTURE_2D, Some(screen.texture));
                screen.gl.get_tex_image(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut pixels)),
                );
                assert_eq!(screen.gl.get_error(), glow::NO_ERROR);
            }
            let expected: Vec<_> = frame
                .rgb
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|&[r, g, b]| [r, g, b, 255])
                .collect();
            assert_eq!(pixels, expected);
        };
        read(&screen, &frame);
        // Same pixel count, different geometry: storage must be resized.
        frame = Frame::from_rgb(2, 3, vec![73; 18]);
        assert_eq!(screen.present(&frame, None, true), (2, 3, true));
        read(&screen, &frame);
        assert_eq!(screen.present(&frame, None, true), (2, 3, false));
        screen.destroy();
    }
}
