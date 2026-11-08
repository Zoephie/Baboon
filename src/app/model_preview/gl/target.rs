//! Textures and framebuffers to draw into, made so that a failure part way
//! leaves nothing behind.

use eframe::glow::{self, HasContext};

/// How a target texture stores its texels: the internal format, and the
/// format and type `tex_image_2d` is handed with no data.
#[derive(Debug, Clone, Copy)]
pub(in crate::app::model_preview) struct Texels {
    pub(in crate::app::model_preview) internal: u32,
    pub(in crate::app::model_preview) format: u32,
    pub(in crate::app::model_preview) kind: u32,
}

impl Texels {
    pub(in crate::app::model_preview) const RGBA8: Self =
        Self { internal: glow::RGBA8, format: glow::RGBA, kind: glow::UNSIGNED_BYTE };
    pub(in crate::app::model_preview) const RGBA16F: Self =
        Self { internal: glow::RGBA16F, format: glow::RGBA, kind: glow::FLOAT };
    pub(in crate::app::model_preview) const RG32F: Self =
        Self { internal: glow::RG32F, format: glow::RG, kind: glow::FLOAT };
    pub(in crate::app::model_preview) const DEPTH24: Self =
        Self { internal: glow::DEPTH_COMPONENT24, format: glow::DEPTH_COMPONENT, kind: glow::UNSIGNED_INT };
    pub(in crate::app::model_preview) const DEPTH24_STENCIL8: Self =
        Self { internal: glow::DEPTH24_STENCIL8, format: glow::DEPTH_STENCIL, kind: glow::UNSIGNED_INT_24_8 };

    /// `internal` handed as RGBA floats, as the gen3 targets are made.
    pub(in crate::app::model_preview) const fn float(internal: u32) -> Self {
        Self { internal, format: glow::RGBA, kind: glow::FLOAT }
    }
}

/// How a target texture is sampled: one filter both ways, and whether its
/// edges clamp (else GL's default, repeat).
#[derive(Debug, Clone, Copy)]
pub(in crate::app::model_preview) struct Sampling {
    pub(in crate::app::model_preview) filter: u32,
    pub(in crate::app::model_preview) clamp: bool,
}

impl Sampling {
    pub(in crate::app::model_preview) const LINEAR_CLAMPED: Self = Self { filter: glow::LINEAR, clamp: true };
    pub(in crate::app::model_preview) const NEAREST_CLAMPED: Self = Self { filter: glow::NEAREST, clamp: true };
    pub(in crate::app::model_preview) const NEAREST: Self = Self { filter: glow::NEAREST, clamp: false };
}

enum Object {
    Texture(glow::NativeTexture),
    Framebuffer(glow::NativeFramebuffer),
    Renderbuffer(glow::NativeRenderbuffer),
}

/// Everything a constructor has made so far, deleted when it is dropped
/// unless [`Made::keep`] says the whole set was built.
pub(in crate::app::model_preview) struct Made<'a> {
    gl: &'a glow::Context,
    objects: Vec<Object>,
}

impl<'a> Made<'a> {
    pub(in crate::app::model_preview) fn new(gl: &'a glow::Context) -> Self {
        Self { gl, objects: Vec::new() }
    }

    /// A texture of `size` with no contents, sampled as `sampling`. Leaves
    /// no texture bound.
    ///
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn texture(
        &mut self,
        size: [i32; 2],
        texels: Texels,
        sampling: Sampling,
    ) -> Option<glow::NativeTexture> {
        let gl = self.gl;
        unsafe {
            let texture = gl.create_texture().ok()?;
            self.objects.push(Object::Texture(texture));
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            let empty = glow::PixelUnpackData::Slice(None);
            gl.tex_image_2d(glow::TEXTURE_2D, 0, texels.internal as i32, size[0], size[1], 0, texels.format, texels.kind, empty);
            for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, sampling.filter as i32);
            }
            if sampling.clamp {
                for name in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::CLAMP_TO_EDGE as i32);
                }
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            Some(texture)
        }
    }

    /// A renderbuffer of `size` stored as `format`. Leaves none bound.
    ///
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn renderbuffer(&mut self, size: [i32; 2], format: u32) -> Option<glow::NativeRenderbuffer> {
        let gl = self.gl;
        unsafe {
            let renderbuffer = gl.create_renderbuffer().ok()?;
            self.objects.push(Object::Renderbuffer(renderbuffer));
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(renderbuffer));
            gl.renderbuffer_storage(glow::RENDERBUFFER, format, size[0], size[1]);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            Some(renderbuffer)
        }
    }

    /// A framebuffer with each of `textures` at its attachment, left bound.
    ///
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn framebuffer(
        &mut self,
        textures: &[(u32, glow::NativeTexture)],
    ) -> Option<glow::NativeFramebuffer> {
        let gl = self.gl;
        unsafe {
            let framebuffer = gl.create_framebuffer().ok()?;
            self.objects.push(Object::Framebuffer(framebuffer));
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            for (attachment, texture) in textures {
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, *attachment, glow::TEXTURE_2D, Some(*texture), 0);
            }
            Some(framebuffer)
        }
    }

    /// A [`ColorTarget`] of `size`.
    ///
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn color(&mut self, size: [i32; 2], texels: Texels, sampling: Sampling) -> Option<ColorTarget> {
        unsafe {
            let texture = self.texture(size, texels, sampling)?;
            let framebuffer = self.framebuffer(&[(glow::COLOR_ATTACHMENT0, texture)])?;
            Some(ColorTarget { framebuffer, texture, size })
        }
    }

    /// A [`DepthTarget`] of `size`, its depth (and stencil, for a format
    /// that has one) at `attachment`, point sampled.
    ///
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn depth(
        &mut self,
        size: [i32; 2],
        texels: Texels,
        sampling: Sampling,
        attachment: u32,
    ) -> Option<DepthTarget> {
        unsafe {
            let texture = self.texture(size, texels, sampling)?;
            let framebuffer = self.framebuffer(&[(attachment, texture)])?;
            self.gl.draw_buffers(&[glow::NONE]);
            self.gl.read_buffer(glow::NONE);
            Some(DepthTarget { framebuffer, texture })
        }
    }

    /// Whether every one of `framebuffers` is complete. Leaves the last bound.
    ///
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn complete(&self, framebuffers: &[glow::NativeFramebuffer]) -> bool {
        framebuffers.iter().all(|framebuffer| unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(*framebuffer));
            self.gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE
        })
    }

    /// The set was built: keep everything made.
    pub(in crate::app::model_preview) fn keep(mut self) {
        self.objects.clear();
    }
}

impl Drop for Made<'_> {
    fn drop(&mut self) {
        for object in self.objects.drain(..).rev() {
            unsafe {
                match object {
                    Object::Texture(texture) => self.gl.delete_texture(texture),
                    Object::Framebuffer(framebuffer) => self.gl.delete_framebuffer(framebuffer),
                    Object::Renderbuffer(renderbuffer) => self.gl.delete_renderbuffer(renderbuffer),
                }
            }
        }
    }
}

/// A texture drawn into through its own framebuffer, at colour 0.
#[derive(Debug)]
pub(in crate::app::model_preview) struct ColorTarget {
    pub(in crate::app::model_preview) framebuffer: glow::NativeFramebuffer,
    pub(in crate::app::model_preview) texture: glow::NativeTexture,
    pub(in crate::app::model_preview) size: [i32; 2],
}

impl ColorTarget {
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn new(gl: &glow::Context, size: [i32; 2], texels: Texels, sampling: Sampling) -> Option<Self> {
        let mut made = Made::new(gl);
        let target = unsafe { made.color(size, texels, sampling) }?;
        made.keep();
        Some(target)
    }

    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.texture);
        }
    }
}

/// A depth texture drawn into through its own framebuffer, which draws and
/// reads no colour.
#[derive(Debug)]
pub(in crate::app::model_preview) struct DepthTarget {
    pub(in crate::app::model_preview) framebuffer: glow::NativeFramebuffer,
    pub(in crate::app::model_preview) texture: glow::NativeTexture,
}

/// A color and depth target at a pane's size. One holds the whole scene while
/// game shaders are on, because the environment layer blends by destination
/// alpha, which the window's framebuffer need not keep. The other is texaccum,
/// for lightmap passes to read back at each pixel's screen position.
pub(in crate::app::model_preview) struct H2Target {
    pub(in crate::app::model_preview) framebuffer: glow::NativeFramebuffer,
    pub(in crate::app::model_preview) color: glow::NativeTexture,
    pub(in crate::app::model_preview) depth: glow::NativeRenderbuffer,
    pub(in crate::app::model_preview) size: [i32; 2],
}

impl H2Target {
    pub(in crate::app::model_preview) unsafe fn new(gl: &glow::Context, width: i32, height: i32) -> Option<Self> {
        let size = [width, height];
        let mut made = Made::new(gl);
        unsafe {
            let color = made.texture(size, Texels::RGBA8, Sampling::NEAREST_CLAMPED)?;
            let depth = made.renderbuffer(size, glow::DEPTH24_STENCIL8)?;
            let framebuffer = made.framebuffer(&[(glow::COLOR_ATTACHMENT0, color)])?;
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_STENCIL_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                eprintln!("model preview: render target incomplete: {status:#x}");
                return None;
            }
            made.keep();
            Some(Self { framebuffer, color, depth, size })
        }
    }

    pub(in crate::app::model_preview) unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.color);
            gl.delete_renderbuffer(self.depth);
        }
    }
}

/// The viewport of the framebuffer being drawn, copied into a texture of its
/// size. A blit, as what is drawn to may be multisampled.
pub(in crate::app::model_preview) unsafe fn copy_frame(gl: &glow::Context, copy: &mut Option<ColorTarget>) -> Option<glow::NativeTexture> {
    unsafe {
        let mut viewport = [0; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        let [x, y, width, height] = viewport;
        if width <= 0 || height <= 0 {
            return None;
        }
        if copy.as_ref().is_some_and(|copy| copy.size != [width, height])
            && let Some(old) = copy.take()
        {
            old.delete(gl);
        }
        let drawn_to = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        if copy.is_none() {
            *copy = Some(ColorTarget::new(gl, [width, height], Texels::RGBA8, Sampling::LINEAR_CLAMPED)?);
        }
        let made = copy.as_ref()?;
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, drawn_to);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(made.framebuffer));
        gl.blit_framebuffer(x, y, x + width, y + height, 0, 0, width, height, glow::COLOR_BUFFER_BIT, glow::NEAREST);
        gl.bind_framebuffer(glow::FRAMEBUFFER, drawn_to);
        Some(made.texture)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod gl_tests {
    use crate::app::model_preview::harness::gl::headless_gl;
    use eframe::glow::{self, HasContext as _};

    /// What [`Made`](crate::app::model_preview::gl::target::Made) builds goes when it is
    /// dropped half done, and stays once kept.
    #[test]
    fn made_targets_are_deleted_unless_kept() {
        use crate::app::model_preview::gl::target::{Made, Sampling, Texels};
        let gl = headless_gl();
        // Each kind numbers its own names from 1.
        let held = |gl: &glow::Context| -> usize {
            (1..=32u32)
                .map(|name| unsafe {
                    let name = std::num::NonZeroU32::new(name).unwrap();
                    usize::from(gl.is_texture(glow::NativeTexture(name)))
                        + usize::from(gl.is_framebuffer(glow::NativeFramebuffer(name)))
                        + usize::from(gl.is_renderbuffer(glow::NativeRenderbuffer(name)))
                })
                .sum()
        };
        unsafe {
            let mut made = Made::new(&gl);
            made.color([4, 4], Texels::RGBA8, Sampling::LINEAR_CLAMPED).unwrap();
            made.depth([4, 4], Texels::DEPTH24, Sampling::NEAREST, glow::DEPTH_ATTACHMENT).unwrap();
            made.renderbuffer([4, 4], glow::DEPTH24_STENCIL8).unwrap();
            assert_eq!(held(&gl), 5);
            drop(made);
            assert_eq!(held(&gl), 0, "a set never kept is gone");
            let mut made = Made::new(&gl);
            let target = made.color([4, 4], Texels::RGBA8, Sampling::LINEAR_CLAMPED).unwrap();
            assert!(made.complete(&[target.framebuffer]));
            made.keep();
            assert_eq!(held(&gl), 2, "a kept set stays");
            target.delete(&gl);
            assert_eq!(held(&gl), 0);
        }
    }
}
