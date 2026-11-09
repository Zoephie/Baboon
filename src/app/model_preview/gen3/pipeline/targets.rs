//! The frame's render targets, sized to the scene.

use super::*;

/// The targets a frame draws through.
pub(in crate::app::model_preview) struct Gen3Targets {
    pub(in crate::app::model_preview) size: [i32; 2],
    pub(in crate::app::model_preview) albedo_framebuffer: glow::NativeFramebuffer,
    pub(in crate::app::model_preview) accumulation_framebuffer: glow::NativeFramebuffer,
    pub(in crate::app::model_preview) albedo: glow::NativeTexture,
    pub(in crate::app::model_preview) normal: glow::NativeTexture,
    pub(in crate::app::model_preview) accumulation: glow::NativeTexture,
    /// A copy of the scene behind the object, for active camo.
    pub(in crate::app::model_preview) behind: glow::NativeTexture,
    /// The finished frame as light, for what draws over all of it.
    pub(in crate::app::model_preview) picture: glow::NativeTexture,
    pub(in crate::app::model_preview) picture_framebuffer: glow::NativeFramebuffer,
    /// `behind`, to blit into.
    pub(in crate::app::model_preview) behind_framebuffer: glow::NativeFramebuffer,
    /// A copy of the depth, for what fades against it.
    pub(in crate::app::model_preview) depth_copy: glow::NativeTexture,
    pub(in crate::app::model_preview) depth_copy_framebuffer: glow::NativeFramebuffer,
    /// `apply_distortions`' two: how far each pixel leans, and what it reads.
    pub(in crate::app::model_preview) distortion: glow::NativeTexture,
    pub(in crate::app::model_preview) distortion_framebuffer: glow::NativeFramebuffer,
    pub(in crate::app::model_preview) scene_copy: glow::NativeTexture,
    pub(in crate::app::model_preview) scene_copy_framebuffer: glow::NativeFramebuffer,
}

impl Gen3Targets {
    pub(super) unsafe fn new(gl: &glow::Context, size: [i32; 2], depth: glow::NativeRenderbuffer) -> Option<Self> {
        let mut made = Made::new(gl);
        unsafe {
            let mut target = |internal| made.texture(size, Texels::float(internal), Sampling::NEAREST);
            let albedo = target(glow::RGBA16F)?;
            let normal = target(glow::RGBA16F)?;
            let accumulation = target(glow::RGBA16F)?;
            let behind = target(glow::SRGB8_ALPHA8)?;
            let picture = target(glow::SRGB8_ALPHA8)?;
            let picture_framebuffer = made.framebuffer(&[(glow::COLOR_ATTACHMENT0, picture)])?;
            let behind_framebuffer = made.framebuffer(&[(glow::COLOR_ATTACHMENT0, behind)])?;
            let albedo_framebuffer =
                made.framebuffer(&[(glow::COLOR_ATTACHMENT0, albedo), (glow::COLOR_ATTACHMENT1, normal)])?;
            gl.draw_buffers(&[glow::COLOR_ATTACHMENT0, glow::COLOR_ATTACHMENT1]);
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_STENCIL_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
            let accumulation_framebuffer = made.framebuffer(&[(glow::COLOR_ATTACHMENT0, accumulation)])?;
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_STENCIL_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
            let depth_copy =
                made.depth(size, Texels::DEPTH24_STENCIL8, Sampling::NEAREST, glow::DEPTH_STENCIL_ATTACHMENT)?;
            let distortion = made.color(size, Texels::float(glow::RG16F), Sampling::NEAREST)?;
            let scene_copy = made.color(size, Texels::RGBA16F, Sampling::NEAREST)?;
            made.keep();
            Some(Self {
                size,
                albedo_framebuffer,
                accumulation_framebuffer,
                albedo,
                normal,
                accumulation,
                behind,
                picture,
                picture_framebuffer,
                behind_framebuffer,
                depth_copy: depth_copy.texture,
                depth_copy_framebuffer: depth_copy.framebuffer,
                distortion: distortion.texture,
                distortion_framebuffer: distortion.framebuffer,
                scene_copy: scene_copy.texture,
                scene_copy_framebuffer: scene_copy.framebuffer,
            })
        }
    }

    pub(in crate::app::model_preview) unsafe fn delete(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.albedo_framebuffer);
            gl.delete_framebuffer(self.accumulation_framebuffer);
            gl.delete_framebuffer(self.depth_copy_framebuffer);
            gl.delete_framebuffer(self.distortion_framebuffer);
            gl.delete_framebuffer(self.scene_copy_framebuffer);
            gl.delete_framebuffer(self.picture_framebuffer);
            gl.delete_framebuffer(self.behind_framebuffer);
            for texture in [
                self.albedo,
                self.normal,
                self.accumulation,
                self.behind,
                self.picture,
                self.depth_copy,
                self.distortion,
                self.scene_copy,
            ] {
                gl.delete_texture(texture);
            }
        }
    }
}
