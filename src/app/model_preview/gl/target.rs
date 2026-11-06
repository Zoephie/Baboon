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
