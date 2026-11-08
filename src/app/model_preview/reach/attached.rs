//! What hangs on a Reach object attached to another.

use std::sync::Arc;
use super::super::attachment::MarkerFrame;
use super::super::cloth::ClothGame;
use super::super::*;
use super::super::load::LoadCx;
use crate::app::model_preview::common::attached::named;

/// What hangs on a Reach object brought into the preview, or on the one
/// opened (`name` none): its flares, lights and cheap lights, and the
/// `light_apply` passes they are drawn with, its effects' volumes,
/// particles, cheap particles, strips and screen effects, and its antennas
/// and cloth. `center` is the middle of its bounding sphere, in its own
/// space.
pub(in crate::app::model_preview) fn read_reach_attached_features<F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize)>(
    cx: &LoadCx<'_, F>,
    center: [f32; 3],
    preview: &mut RenderModelPreview,
) {
    let LoadCx { reader, object, owner, markers, frame, name } = *cx;
    let flares = super::super::gen3::flares::read_h3_flares(reader, object, owner, markers, frame);
    preview.gen3.flares.extend(named(flares, name));
    let (lights, cheap) = super::super::reach::lights::read_reach_lights(reader, object, owner, markers, frame);
    preview.reach.lights.extend(named(lights, name));
    preview.reach.cheap_lights.extend(named(cheap, name));
    let volumes = super::super::gen3::light_volumes::read_h3_light_volumes(reader, object, owner, markers, frame);
    preview.gen3.light_volumes.extend(named(volumes, name));
    let cheap = super::super::particles::cheap::read_reach_cheap_emitters(reader, object, owner, markers, frame);
    preview.gen3.cheap_emitters.extend(named(cheap, name));
    let particles = super::super::particles::read_h3_particles(reader, object, owner, markers, frame);
    let particles: Vec<_> = named(particles, name).map(Arc::new).collect();
    let carried = super::super::particles::carried_strips(&particles);
    preview.particles.extend(particles);
    preview.gen3.strips.extend(carried);
    let strips = super::super::gen3::strips::read_h3_strips(reader, object, owner, markers, frame);
    preview.gen3.strips.extend(named(strips, name).map(Arc::new));
    let screens = super::super::gen3::screen_effects::read_h3_screen_effects(reader, object, owner, markers, frame);
    preview.gen3.screen_effects.extend(screens);
    super::super::gen3::screen_effects::add_screen_shader_materials(preview);
    preview.antennas.extend(super::super::antennas::read_reach_antennas(reader, object, markers, frame, name));
    super::super::cloth::read_cloth(cx, preview, ClothGame::Reach, center);
    // `render_screenspace_light_callback`'s `99 + combo`, gels aside.
    let lit = !(preview.reach.lights.is_empty() && preview.reach.cheap_lights.is_empty());
    if lit && preview.reach.light_passes.is_empty() {
        let mut sources = super::super::gen3::shaders::H3Sources::new(&reader.source);
        let files = ["light_apply", "light_apply_frustum", "light_apply_specular", "light_apply_frustum_specular"];
        preview.reach.light_passes = files.iter().map(|file| sources.reach_light_pass(file).ok()).collect();
    }
}
