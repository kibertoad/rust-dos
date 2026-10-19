//! State owned during one replay call. Frontends and target allocation
//! also use the context, so discard the shadow when crossing those paths.

use glow::HasContext;
use std::cell::Cell;

#[derive(Default)]
pub(super) struct State {
    framebuffer: Cell<Option<glow::Framebuffer>>,
    viewport: Cell<Option<[i32; 4]>>,
    blend: Cell<Option<bool>>,
    blend_func: Cell<Option<[u32; 4]>>,
    depth: Cell<Option<bool>>,
    depth_func: Cell<Option<u32>>,
    depth_mask: Cell<Option<bool>>,
    color_mask: Cell<Option<[bool; 4]>>,
    scissor: Cell<Option<bool>>,
    scissor_rect: Cell<Option<[i32; 4]>>,
    stencil: Cell<Option<bool>>,
    /// Send every call, as if nothing were cached: the baseline the
    /// regression test compares the cached replay with.
    #[cfg(test)]
    pub bypass: bool,
}

impl State {
    fn changed<T: Copy + PartialEq>(&self, slot: &Cell<Option<T>>, value: T) -> bool {
        let changed = slot.replace(Some(value)) != Some(value);
        #[cfg(test)]
        let changed = changed || self.bypass;
        changed
    }

    fn capability(&self, slot: &Cell<Option<bool>>, gl: &glow::Context, name: u32, enabled: bool) {
        if !self.changed(slot, enabled) {
            return;
        }
        // SAFETY: the replay's OpenGL context is current on this thread.
        unsafe {
            if enabled {
                gl.enable(name);
            } else {
                gl.disable(name);
            }
        }
    }

    pub fn invalidate(&mut self) {
        *self = Self {
            #[cfg(test)]
            bypass: self.bypass,
            ..Self::default()
        };
    }

    pub fn framebuffer(&self, gl: &glow::Context, fbo: glow::Framebuffer) {
        if self.changed(&self.framebuffer, fbo) {
            // SAFETY: the replay's OpenGL context is current on this thread.
            unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo)) };
        }
    }

    pub fn viewport(&self, gl: &glow::Context, rect: [i32; 4]) {
        if self.changed(&self.viewport, rect) {
            // SAFETY: the replay's OpenGL context is current on this thread.
            unsafe { gl.viewport(rect[0], rect[1], rect[2], rect[3]) };
        }
    }

    pub fn blend(&self, gl: &glow::Context, enabled: bool) {
        self.capability(&self.blend, gl, glow::BLEND, enabled);
    }

    pub fn blend_func(&self, gl: &glow::Context, factors: [u32; 4]) {
        if self.changed(&self.blend_func, factors) {
            // SAFETY: the replay's OpenGL context is current on this thread.
            unsafe { gl.blend_func_separate(factors[0], factors[1], factors[2], factors[3]) };
        }
    }

    pub fn depth(&self, gl: &glow::Context, enabled: bool) {
        self.capability(&self.depth, gl, glow::DEPTH_TEST, enabled);
    }

    pub fn depth_func(&self, gl: &glow::Context, function: u32) {
        if self.changed(&self.depth_func, function) {
            // SAFETY: the replay's OpenGL context is current on this thread.
            unsafe { gl.depth_func(function) };
        }
    }

    pub fn depth_mask(&self, gl: &glow::Context, enabled: bool) {
        if self.changed(&self.depth_mask, enabled) {
            // SAFETY: the replay's OpenGL context is current on this thread.
            unsafe { gl.depth_mask(enabled) };
        }
    }

    pub fn color_mask(&self, gl: &glow::Context, mask: [bool; 4]) {
        if self.changed(&self.color_mask, mask) {
            // SAFETY: the replay's OpenGL context is current on this thread.
            unsafe { gl.color_mask(mask[0], mask[1], mask[2], mask[3]) };
        }
    }

    pub fn scissor(&self, gl: &glow::Context, rect: Option<[i32; 4]>) {
        self.capability(&self.scissor, gl, glow::SCISSOR_TEST, rect.is_some());
        if let Some(rect) = rect
            && self.changed(&self.scissor_rect, rect)
        {
            // SAFETY: the replay's OpenGL context is current on this thread.
            unsafe { gl.scissor(rect[0], rect[1], rect[2], rect[3]) };
        }
    }

    pub fn stencil(&self, gl: &glow::Context, enabled: bool) {
        self.capability(&self.stencil, gl, glow::STENCIL_TEST, enabled);
    }
}
