//! What hangs on a Halo 3 or ODST object attached to another.

use std::sync::Arc;
use blam_tags::TagFile;
use super::super::attachment::MarkerFrame;
use super::super::cloth::ClothGame;
use super::super::*;
use crate::core::source::TagSource;
use crate::app::model_preview::common::attached::named;

/// What hangs on a Halo 3 or ODST object brought into the preview: its
/// lights, flares, light volumes, particle systems and the strips they carry,
/// strips, cloth and screen effects, at `markers` as `frame` places them.
pub(in crate::app::model_preview) fn read_h3_attached_features<F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize)>(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: &F,
    name: Option<&str>,
    preview: &mut RenderModelPreview,
) {
    let lights = super::super::h3::lights::read_h3_lights(source, object, owner, markers, frame);
    let flares = super::super::h3::lights::read_h3_flares(source, object, owner, markers, frame);
    let volumes = super::super::gen3::light_volumes::read_h3_light_volumes(source, object, owner, markers, frame);
    preview.h3_light_volumes.extend(named(volumes, name));
    let particles = super::super::particles::read_h3_particles(source, object, owner, markers, frame);
    let particles: Vec<_> = named(particles, name).map(Arc::new).collect();
    let carried = super::super::particles::carried_strips(&particles);
    preview.h3_particles.extend(particles);
    preview.h3_strips.extend(carried);
    let strips = super::super::gen3::strips::read_h3_strips(source, object, owner, markers, frame);
    preview.h3_strips.extend(named(strips, name).map(Arc::new));
    super::super::cloth::read_h3_cloth(source, object, owner, markers, frame, preview, ClothGame::Halo3, [0.0; 3]);
    let screens = super::super::gen3::screen_effects::read_h3_screen_effects(source, object, owner, markers, frame);
    preview.h3_screen_effects.extend(screens);
    super::super::gen3::screen_effects::add_screen_shader_materials(preview);
    let lights: Vec<_> = named(lights, name).collect();
    preview.h3_flares.extend(named(flares, name));
    preview.h3_flares.extend(super::super::h3::lights::light_flares(&lights));
    preview.h3_lights.extend(lights);
}
