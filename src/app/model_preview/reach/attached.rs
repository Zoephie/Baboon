//! What hangs on a Reach object attached to another.

use std::sync::Arc;
use blam_tags::TagFile;
use super::super::attachment::MarkerFrame;
use super::super::cloth::ClothGame;
use super::super::*;
use crate::core::source::TagSource;
use crate::app::model_preview::common::attached::named;

/// What hangs on a Reach object brought into the preview, or on the one
/// opened (`name` none): its flares, lights and cheap lights, and the
/// `light_apply` passes they are drawn with, its effects' volumes,
/// particles, cheap particles, strips and screen effects, and its antennas
/// and cloth. `center` is the middle of its bounding sphere, in its own
/// space.
#[allow(clippy::too_many_arguments)]
pub(in crate::app::model_preview) fn read_reach_attached_features<F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize)>(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: &F,
    name: Option<&str>,
    center: [f32; 3],
    preview: &mut RenderModelPreview,
) {
    let flares = super::super::h3::lights::read_h3_flares(source, object, owner, markers, frame);
    preview.h3_flares.extend(named(flares, name));
    let (lights, cheap) = super::super::reach::lights::read_reach_lights(source, object, owner, markers, frame);
    preview.reach_lights.extend(named(lights, name));
    preview.reach_cheap_lights.extend(named(cheap, name));
    let volumes = super::super::gen3::light_volumes::read_h3_light_volumes(source, object, owner, markers, frame);
    preview.h3_light_volumes.extend(named(volumes, name));
    let cheap = super::super::particles::cheap::read_reach_cheap_emitters(source, object, owner, markers, frame);
    preview.reach_cheap_emitters.extend(named(cheap, name));
    let particles = super::super::particles::read_h3_particles(source, object, owner, markers, frame);
    let particles: Vec<_> = named(particles, name).map(Arc::new).collect();
    let carried = super::super::particles::carried_strips(&particles);
    preview.h3_particles.extend(particles);
    preview.h3_strips.extend(carried);
    let strips = super::super::gen3::strips::read_h3_strips(source, object, owner, markers, frame);
    preview.h3_strips.extend(named(strips, name).map(Arc::new));
    let screens = super::super::gen3::screen_effects::read_h3_screen_effects(source, object, owner, markers, frame);
    preview.h3_screen_effects.extend(screens);
    super::super::gen3::screen_effects::add_screen_shader_materials(preview);
    preview.h2_antennas.extend(super::super::h2::lights::read_reach_antennas(source, object, markers, frame, name));
    super::super::cloth::read_h3_cloth(source, object, owner, markers, frame, preview, ClothGame::Reach, center);
    // `render_screenspace_light_callback`'s `99 + combo`, gels aside.
    let lit = !(preview.reach_lights.is_empty() && preview.reach_cheap_lights.is_empty());
    if lit && preview.reach_light_passes.is_empty() {
        let mut sources = super::super::gen3::shaders::H3Sources::new(source);
        let files = ["light_apply", "light_apply_frustum", "light_apply_specular", "light_apply_frustum_specular"];
        preview.reach_light_passes = files.iter().map(|file| sources.reach_light_pass(file).ok()).collect();
    }
}
