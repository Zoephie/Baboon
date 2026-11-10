//! What hangs on a Halo 4 object attached to another.

use super::super::cloth::ClothGame;
use super::super::load::LoadCx;
use super::super::{RenderModelPreview, RenderModelPreviewMarker};
use crate::app::model_preview::common::attached::named;
use crate::app::model_preview::common::marker::MarkerFrame;
use std::sync::Arc;

/// What a Halo 4 object brings of its own so far: its lights, cheap lights
/// and lens flares and the passes they draw with
/// (`docs/h4-preview-research/lights.md`), its particles, light volumes and
/// tracers, which draw through their materials (`particles.md`,
/// `tracers.md`), its cheap particles, and antennas and cloth, whose
/// definitions are Reach's (`objects.md` §3.10).
pub(in crate::app::model_preview) fn read_h4_attached_features<
    F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
>(
    cx: &LoadCx<'_, F>,
    center: [f32; 3],
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
        super::super::cloth::read_cloth(cx, preview, ClothGame::Reach, center);
        return;
    }
    // A weapon opened on its own has no parent to hold it.
    let lone_weapon = name.is_none() && object.group().tag == u32::from_be_bytes(*b"weap");
    let (lights, cheap) = super::super::h4::lights::read_h4_lights(
        reader,
        object,
        owner,
        markers,
        frame,
        lone_weapon,
    );
    preview.h4.lights.extend(named(lights, name));
    preview.h4.cheap_lights.extend(named(cheap, name));
    super::super::h4::light_pass::read_h4_light_passes(reader, preview);
    let particles = super::super::particles::read_particles(reader, object, owner, markers, frame);
    preview
        .particles
        .extend(named(particles, name).map(Arc::new));
    let volumes = super::super::gen3::effects::light_volumes::read_h3_light_volumes(
        reader, object, owner, markers, frame,
    );
    preview.gen3.light_volumes.extend(named(volumes, name));
    let tracers = super::super::h4::tracers::read_h4_tracers(reader, object, owner, markers, frame);
    preview.h4.tracers.extend(named(tracers, name));
    let cheap = super::super::particles::cheap::read_reach_cheap_emitters(
        reader, object, owner, markers, frame,
    );
    preview.gen3.cheap_emitters.extend(named(cheap, name));
    let flares = super::super::h4::flares::read_h4_flares(reader, object, owner, markers, frame);
    preview.h4.flares.extend(named(flares, name));
    if !preview.h4.flares.is_empty() && preview.h4.flare_pass.is_none() {
        preview.h4.flare_pass = super::super::h4::flares::h4_flare_pass(reader)
            .ok()
            .map(Arc::new);
    }
    preview
        .antennas
        .extend(super::super::reach::antennas::read_reach_antennas(
            reader, object, markers, frame, name,
        ));
    super::super::cloth::read_cloth(cx, preview, ClothGame::Reach, center);
}
