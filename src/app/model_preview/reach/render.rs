//! Reach's place in the gen3 pipeline: its exposure stops, its self shadow
//! mask, its lights inline and on the screen, its camo, shield, bloom and
//! composite.

use eframe::glow::{self, HasContext};

use super::super::gen3::render::{Gen3Batch, Gen3Game};
use super::super::gen3::effects::screen::H3ScreenSettings;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::pipeline::h3_set_cull;
use super::lights::ReachLitLight;

/// Reach's hooks, and the self shadow mask from its making to the lit pass.
#[derive(Default)]
pub(in crate::app::model_preview) struct ReachGame {
    mask: Option<glow::NativeTexture>,
    /// The shadow's map and mask, kept between frames, and its apply program.
    pub(super) shadow: Option<super::shadow_pass::ReachShadowTargets>,
    pub(super) shadow_program: Option<glow::NativeProgram>,
    bloom: Option<super::bloom::ReachBloom>,
    /// The final composite's program.
    pub(super) composite_program: Option<glow::NativeProgram>,
    /// `rasterizer_globals`' screen noise, uploaded on first use.
    pub(super) noise: Option<glow::NativeTexture>,
    /// The composites drawn, for the noise's frame-to-frame offset.
    pub(super) noise_frame: u32,
}

impl Gen3Game for ReachGame {
    /// `c_player_view::render` hands the screen effects' stops to
    /// `setup_camera_fx_parameters` before anything draws.
    fn stops(&self, screen: &H3ScreenSettings) -> f32 {
        screen.stops()
    }

    /// `render_fullscreen_shadow_buffer`: the mask, between the albedo and
    /// the static lighting that reads it.
    unsafe fn self_shadow(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        batches: &[Gen3Batch],
        _order: &[usize],
        _scene: glow::NativeFramebuffer,
    ) -> bool {
        self.mask = if cx.frame.self_shadow {
            unsafe { self.draw_shadow(cx, batches) }
        } else {
            None
        };
        false
    }

    /// `build_simple_light_draw_list`, before the static pass reads it.
    fn frame_lights(&mut self, cx: &mut Gen3Cx<'_>) -> Vec<ReachLitLight> {
        Self::frame_lights_of(cx)
    }

    fn lit_values(&mut self, cx: &mut Gen3Cx<'_>) {
        let state = &mut *cx.state;
        if let Some(mask) = self.mask.take() {
            state.shadow_mask = Some(mask);
            state.pass_values = Some([("shadow_mask_enabled", [1.0; 4])].into_iter().collect());
        }
    }

    /// `c_lights_view::render`: after the static lighting, before what blends.
    unsafe fn screen_lights(&mut self, cx: &mut Gen3Cx<'_>, lit: &[ReachLitLight], accumulation: glow::NativeFramebuffer) {
        unsafe {
            self.draw_screen_lights(cx, lit, accumulation);
            cx.core.bind_slot_vertices(cx.gl, cx.slot);
            h3_set_cull(cx.gl, cx.frame);
            cx.gl.depth_func(glow::LEQUAL);
            cx.gl.depth_mask(false);
            cx.gl.blend_func(glow::ONE, glow::ONE);
        }
    }

    /// Full camo, through `globals`' curves: only where the object has it.
    fn camo(&mut self, cx: &mut Gen3Cx<'_>) -> Option<bool> {
        let (frame, state) = (cx.frame, &mut *cx.state);
        let draw = frame.preview.reach.camo.as_ref().map(|camo| camo.draw(1.0))?;
        let values = [("k_vs_active_camo_factor", draw.factor), ("p_lighting_constant_9", draw.factor)];
        state.pass_values = Some(values.into_iter().collect());
        frame.active_camo.then_some(draw.opaque)
    }

    unsafe fn shield(&mut self, cx: &mut Gen3Cx<'_>) {
        unsafe { cx.state.draw_reach_shield(cx.core, cx.gl, cx.frame, cx.slot) }
    }

    unsafe fn bloom(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            let vertices = cx.state.empty_array(cx.gl);
            if self.bloom.is_none() {
                self.bloom = super::bloom::ReachBloom::new(cx.gl, cx.core.black_flat);
            }
            self.bloom.as_mut()?.build(cx.gl, vertices, framebuffer, accumulation, size)
        }
    }

    unsafe fn composite(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: H3ScreenSettings,
        _size: [i32; 2],
    ) {
        unsafe {
            let depth = cx.state.object_depth(cx.gl, framebuffer);
            self.draw_composite(cx, accumulation, bloom, depth, screen);
        }
    }
}
