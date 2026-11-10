//! What hangs on a Halo 3 or ODST object attached to another.

use super::super::cloth::ClothGame;
use super::super::load::LoadCx;
use super::super::{RenderModelPreview, RenderModelPreviewMarker};
use crate::app::model_preview::common::attached::named;
use crate::app::model_preview::common::marker::MarkerFrame;
use std::sync::Arc;

/// What hangs on a Halo 3 or ODST object brought into the preview: its
/// lights, flares, light volumes, particle systems and the strips they carry,
/// strips, cloth and screen effects, at `markers` as `frame` places them.
pub(in crate::app::model_preview) fn read_h3_attached_features<
    F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
>(
    cx: &LoadCx<'_, F>,
    preview: &mut RenderModelPreview,
) {
    let LoadCx {
        reader,
        object,
        owner,
        markers,
        frame,
        name,
    } = *cx;
    // Without Game Shaders, the cloth alone (`TagReader::game_shaders`).
    if !reader.game_shaders() {
        super::super::cloth::read_cloth(cx, preview, ClothGame::Halo3, [0.0; 3]);
        return;
    }
    let lights = super::super::h3::lights::read_h3_lights(reader, object, owner, markers, frame);
    let flares =
        super::super::gen3::effects::flares::read_h3_flares(reader, object, owner, markers, frame);
    let volumes = super::super::gen3::effects::light_volumes::read_h3_light_volumes(
        reader, object, owner, markers, frame,
    );
    preview.gen3.light_volumes.extend(named(volumes, name));
    let particles = super::super::particles::read_particles(reader, object, owner, markers, frame);
    let particles: Vec<_> = named(particles, name).map(Arc::new).collect();
    let carried = super::super::particles::carried_strips(&particles);
    preview.particles.extend(particles);
    preview.gen3.strips.extend(carried);
    let strips =
        super::super::gen3::effects::strips::read_h3_strips(reader, object, owner, markers, frame);
    preview
        .gen3
        .strips
        .extend(named(strips, name).map(Arc::new));
    super::super::cloth::read_cloth(cx, preview, ClothGame::Halo3, [0.0; 3]);
    let screens = super::super::gen3::effects::screen::read_h3_screen_effects(
        reader, object, owner, markers, frame,
    );
    preview.gen3.screen_effects.extend(screens);
    super::super::gen3::effects::screen::add_screen_shader_materials(preview);
    let lights: Vec<_> = named(lights, name).collect();
    preview.gen3.flares.extend(named(flares, name));
    preview
        .gen3
        .flares
        .extend(super::lights::light_flares(&lights));
    preview.h3.lights.extend(lights);
}
