//! Halo 3's and ODST's object lights, each drawn on its own over the
//! object's `dynamic_light` passes after the static lighting.

use super::super::common::features::paths::H3_LIGHTS;
use super::super::gen3::render::{Gen3Batch, Gen3Cx};
use super::super::gen3::state::PassLight;

/// `submit_visibility_and_render`: each shown light the frame lights, its
/// passes drawn by it.
pub(super) unsafe fn draw_dynamic_lights(cx: &mut Gen3Cx<'_>, batches: &[Gen3Batch]) {
    let (gl, frame, slot) = (cx.gl, cx.frame, cx.slot);
    unsafe {
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        for (index, light) in frame.preview.h3.lights.iter().enumerate() {
            if !frame.shown.is(H3_LIGHTS, index) {
                continue;
            }
            let values = frame.function_values_of_object(light.object);
            let colors = frame.change_colors_of_object(light.object);
            let onset = cx.core.slots[slot].simulations.effect_onsets.of(
                light.object,
                light.scale_name(),
                values,
                frame.time,
            );
            let Some(lit) = light.lit(values, &colors, bones, frame.time, onset) else {
                continue;
            };
            let gel = cx.core.slots[slot]
                .h3_textures
                .gels
                .get(index)
                .copied()
                .flatten();
            cx.state.light = Some(PassLight {
                rows: lit.rows,
                projection: lit.projection,
                gel,
            });
            for (batch, material) in batches {
                if let Some(pass) = material.dynamic_light.as_ref() {
                    cx.state
                        .draw_pass(cx.core, gl, frame, slot, batch, material, pass, None);
                }
            }
            cx.state.light = None;
        }
    }
}
