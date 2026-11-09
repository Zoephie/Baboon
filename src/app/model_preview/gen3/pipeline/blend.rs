//! A pass's blend, cull and decal depth state.

use super::*;

/// `g_decal_slope_depth_bias` and `g_decal_depth_bias`.
pub(super) const DECAL_SLOPE_BIAS: f32 = -0.5;

pub(super) const DECAL_DEPTH_BIAS: f32 = -0.000005;

/// What each blended material draws under, after any sprites farther
/// than it: blended, depth tested and not written, culled as the model is,
/// every channel written, the model's vertices bound. Its own blend and any
/// decal offset follow per material.
pub(super) fn blended_material_state(
    show_backfaces: bool,
    vertex_array: glow::NativeVertexArray,
) -> PassState {
    PassState::new()
        .enable(glow::BLEND)
        .depth_mask(false)
        .depth_func(glow::LEQUAL)
        .cull_back_unless(show_backfaces)
        .color_mask([true, true, true, true])
        .bind_vertex_array(vertex_array)
}

/// `c_object_renderer::render_albedo` and its like: `_cull_mode_cw` throughout.
/// Tags lean on it, storing some faces twice with opposite winding, and with
/// both drawn the back one takes the depth tie and its lighting with it.
pub(in crate::app::model_preview) unsafe fn gen3_set_cull(
    gl: &glow::Context,
    frame: &ModelGpuFrame,
) {
    unsafe {
        if frame.show_backfaces {
            gl.disable(glow::CULL_FACE);
        } else {
            gl.enable(glow::CULL_FACE);
            gl.cull_face(glow::BACK);
        }
    }
}

/// `set_alpha_blend_mode_no_cache`: source, destination, source alpha and
/// destination alpha factors, and the equation. Motion blur's alpha factors
/// read a blend factor no target here keeps, so its alpha stays as the source.
fn gen3_blend_state(blend: Gen3Blend) -> ([u32; 4], u32) {
    let add = glow::FUNC_ADD;
    match blend {
        Gen3Blend::Opaque => ([glow::ONE, glow::ZERO, glow::ONE, glow::ZERO], add),
        Gen3Blend::Additive => ([glow::ONE; 4], add),
        Gen3Blend::Multiply => (
            [glow::DST_COLOR, glow::ZERO, glow::DST_ALPHA, glow::ZERO],
            add,
        ),
        Gen3Blend::AlphaBlend => (
            [
                glow::SRC_ALPHA,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::SRC_ALPHA,
                glow::ONE_MINUS_SRC_ALPHA,
            ],
            add,
        ),
        Gen3Blend::InvAlphaBlend => (
            [
                glow::ONE_MINUS_SRC_ALPHA,
                glow::SRC_ALPHA,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::SRC_ALPHA,
            ],
            add,
        ),
        Gen3Blend::DoubleMultiply => (
            [
                glow::DST_COLOR,
                glow::SRC_COLOR,
                glow::DST_ALPHA,
                glow::SRC_ALPHA,
            ],
            add,
        ),
        Gen3Blend::PreMultiplied | Gen3Blend::MultiplyAdd => (
            [
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
            ],
            add,
        ),
        Gen3Blend::Maximum => ([glow::ONE; 4], glow::MAX),
        Gen3Blend::AddSrcTimesDstAlpha => (
            [glow::DST_ALPHA, glow::ONE, glow::DST_ALPHA, glow::ONE],
            add,
        ),
        Gen3Blend::AddSrcTimesSrcAlpha | Gen3Blend::AddSrcTimesSrcAlphaAccumAlpha => (
            [glow::SRC_ALPHA, glow::ONE, glow::SRC_ALPHA, glow::ONE],
            add,
        ),
        Gen3Blend::MotionBlurStatic | Gen3Blend::MotionBlurInhibit => {
            ([glow::ONE, glow::ZERO, glow::ONE, glow::ZERO], add)
        }
    }
}

/// Whether a mode only adds to the frame. The game's alpha means nothing in a
/// frame it shows whole, but here alpha is what covers the preview's background.
pub(super) fn gen3_adds_light(blend: Gen3Blend) -> bool {
    matches!(
        blend,
        Gen3Blend::Additive
            | Gen3Blend::Maximum
            | Gen3Blend::AddSrcTimesDstAlpha
            | Gen3Blend::AddSrcTimesSrcAlpha
            | Gen3Blend::AddSrcTimesSrcAlphaAccumAlpha
    )
}

pub(in crate::app::model_preview) unsafe fn gen3_set_blend(gl: &glow::Context, blend: Gen3Blend) {
    let ([source, destination, source_alpha, destination_alpha], equation) =
        gen3_blend_state(blend);
    unsafe {
        gl.blend_equation(equation);
        gl.blend_func_separate(source, destination, source_alpha, destination_alpha);
        gl.color_mask(true, true, true, !gen3_adds_light(blend));
    }
}

/// What `render_mesh_part_default` lights a part with.
pub(super) fn gen3_lit_pass<'a>(
    frame: &ModelGpuFrame,
    batch: &RenderModelPreviewBatch,
    material: &'a Gen3Material,
) -> &'a Arc<Gen3Pass> {
    let vertex = frame
        .preview
        .indices
        .get(batch.index_start as usize)
        .copied()
        .unwrap_or(0);
    let prt = frame.preview.gen3.prt.as_ref().filter(|_| frame.prt);
    let kind = prt.map_or(Gen3PrtKind::None, |prt| prt.kind_at(vertex));
    let slot = match kind {
        Gen3PrtKind::Ambient => Some(0),
        Gen3PrtKind::Linear => Some(1),
        Gen3PrtKind::Quadratic => Some(2),
        Gen3PrtKind::None => None,
    };
    slot.and_then(|slot| material.prt[slot].as_ref())
        .unwrap_or(&material.lit)
}
