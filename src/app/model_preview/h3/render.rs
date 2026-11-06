//! Halo 3's and ODST's place in the gen3 pipeline: their self shadow, from
//! a map generated through dx9 shadow passes and applied after the opaque
//! pass.

use eframe::glow::{self, HasContext};

use super::super::gen3::render::{Gen3Batch, Gen3Game};
use super::super::gen3::screen_effects::H3ScreenSettings;
use super::super::renderer::FrameCx;
use super::bloom::H3Bloom;

/// Halo 3's and ODST's hooks: the pipeline's defaults, the self shadow, and
/// their bloom and composite.
#[derive(Default)]
pub(in crate::app::model_preview) struct H3Game {
    bloom: Option<H3Bloom>,
    /// The final composite's program.
    pub(super) composite_program: Option<glow::NativeProgram>,
}

impl Gen3Game for H3Game {
    unsafe fn self_shadow(
        &mut self,
        cx: &mut FrameCx<'_>,
        _batches: &[Gen3Batch],
        order: &[usize],
        scene: glow::NativeFramebuffer,
    ) -> bool {
        if !cx.frame.self_shadow {
            return false;
        }
        // Generated from the light, for the apply pass after the opaque one.
        unsafe {
            cx.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene));
            let drawn = cx.core.draw_h2_shadow(cx.gl, cx.frame, cx.slot, order);
            cx.core.bind_slot_vertices(cx.gl, cx.slot);
            drawn
        }
    }

    unsafe fn bloom(
        &mut self,
        cx: &mut FrameCx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            let vertices = cx.core.h3_empty_array(cx.gl);
            if self.bloom.is_none() {
                self.bloom = H3Bloom::new(cx.gl, cx.core.h3_no_bloom);
            }
            self.bloom.as_mut()?.build(cx.gl, vertices, framebuffer, accumulation, size)
        }
    }

    unsafe fn composite(
        &mut self,
        cx: &mut FrameCx<'_>,
        _framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: H3ScreenSettings,
        _size: [i32; 2],
    ) {
        unsafe { self.draw_composite(cx, accumulation, bloom, screen) }
    }
}
