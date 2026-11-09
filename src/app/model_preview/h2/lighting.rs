//! Halo 2's own lights over its passes: `render_lights`' sphere lights and
//! its gel lights, each drawn over the layers it lights.

use eframe::glow::{self, HasContext as _};

use super::super::*;
use super::super::common::features::paths;
use super::super::dx9::passes::{H2DefaultLight, H2Layer};

/// Draw the object's sphere lights over `order`, each over its diffuse and
/// specular layers.
pub(in crate::app::model_preview) unsafe fn draw_h2_point_lights(
    core: &mut RenderCore,
    gl: &glow::Context,
    frame: &ModelGpuFrame,
    slot: usize,
    order: &[usize],
) {
    unsafe {
        // `render_lights`: real lights draw whichever rig stands behind them.
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        for (index, point) in frame.preview.h2.point_lights.iter().enumerate() {
            if !frame.shown.is(paths::H2_POINT_LIGHTS, index) {
                continue;
            }
            let values = frame.function_values_of_object(point.object);
            let onsets = &mut core.slots[slot].simulations.effect_onsets;
            let onset = onsets.of(point.object, point.scale_name(), values, frame.time);
            let Some(light) = point.lit(values, bones, frame.time, onset) else {
                continue;
            };
            let (light, draws) = super::lights::overbrighten(&light);
            let specular = H2DefaultLight { color: light.specular, ..light };
            for _ in 0..draws {
                for (layer, light) in
                    [(point.diffuse_layer(), &light), (H2Layer::LightSpecular, &specular)]
                {
                    core.draw_h2_passes(gl, frame, slot, order, layer, Some(light));
                }
            }
        }
    }
}

/// Draw the object's gel lights over `order`, with their `gels` maps, by
/// `h2.gel_lights`: a specular one masks first, and the mask is cleared after.
pub(in crate::app::model_preview) unsafe fn draw_h2_gel_lights(
    core: &mut RenderCore,
    gl: &glow::Context,
    frame: &ModelGpuFrame,
    slot: usize,
    order: &[usize],
    gels: &[Option<glow::NativeTexture>],
) {
    unsafe {
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let mut masked = false;
        for (index, gel) in frame.preview.h2.gel_lights.iter().enumerate() {
            if !frame.shown.is(paths::H2_GEL_LIGHTS, index) {
                continue;
            }
            let values = frame.function_values_of_object(gel.object);
            let onsets = &mut core.slots[slot].simulations.effect_onsets;
            let onset = onsets.of(gel.object, gel.scale_name(), values, frame.time);
            let Some((light, rows)) = gel.lit(values, bones, frame.time, onset) else {
                continue;
            };
            let specular = super::lights::has_specular(&light);
            let (light, draws) = super::lights::overbrighten(&light);
            core.dx9.h2_gel = Some((gels.get(index).copied().flatten(), rows.rows));
            if specular {
                core.draw_h2_passes(gl, frame, slot, order, H2Layer::GelMask, Some(&light));
                masked = true;
            }
            let layer = if specular { H2Layer::LightFrustum } else { H2Layer::LightGel };
            for _ in 0..draws {
                core.draw_h2_passes(gl, frame, slot, order, layer, Some(&light));
            }
            core.dx9.h2_gel = None;
        }
        if masked {
            gl.color_mask(false, false, false, true);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.color_mask(true, true, true, true);
        }
    }
}
