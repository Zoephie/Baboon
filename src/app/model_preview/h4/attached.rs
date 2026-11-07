//! What hangs on a Halo 4 object attached to another.

use std::sync::Arc;
use blam_tags::TagFile;
use super::super::attachment::MarkerFrame;
use super::super::cloth::ClothGame;
use super::super::*;
use crate::core::source::TagSource;
use crate::app::model_preview::common::attached::named;

/// What a Halo 4 object brings of its own so far: its lights, cheap lights
/// and lens flares and the passes they draw with
/// (`docs/h4-preview-research/lights.md`), its particles, light volumes and
/// tracers, which draw through their materials (`particles.md`,
/// `tracers.md`), its cheap particles, and antennas and cloth, whose
/// definitions are Reach's (`objects.md` §3.10).
#[allow(clippy::too_many_arguments)]
pub(in crate::app::model_preview) fn read_h4_attached_features<F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize)>(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: &F,
    name: Option<&str>,
    center: [f32; 3],
    preview: &mut RenderModelPreview,
) {
    // A weapon opened on its own has no parent to hold it.
    let lone_weapon = name.is_none() && object.group().tag == u32::from_be_bytes(*b"weap");
    let (lights, cheap) = super::super::h4::lights::read_h4_lights(source, object, owner, markers, frame, lone_weapon);
    preview.h4_lights.extend(named(lights, name));
    preview.h4_cheap_lights.extend(named(cheap, name));
    super::super::h4::lights::read_h4_light_passes(source, preview);
    let particles = super::super::particles::read_h3_particles(source, object, owner, markers, frame);
    preview.h3_particles.extend(named(particles, name).map(Arc::new));
    let volumes = super::super::gen3::light_volumes::read_h3_light_volumes(source, object, owner, markers, frame);
    preview.h3_light_volumes.extend(named(volumes, name));
    let tracers = super::super::h4::tracers::read_h4_tracers(source, object, owner, markers, frame);
    preview.h4_tracers.extend(named(tracers, name));
    let cheap = super::super::particles::cheap::read_reach_cheap_emitters(source, object, owner, markers, frame);
    preview.reach_cheap_emitters.extend(named(cheap, name));
    let flares = super::super::h4::flares::read_h4_flares(source, object, owner, markers, frame);
    preview.h4_flares.extend(named(flares, name));
    if !preview.h4_flares.is_empty() && preview.h4_flare_pass.is_none() {
        preview.h4_flare_pass = super::super::h4::flares::h4_flare_pass(source).ok().map(Arc::new);
    }
    preview.h2_antennas.extend(super::super::h2::lights::read_reach_antennas(source, object, markers, frame, name));
    super::super::cloth::read_h3_cloth(source, object, owner, markers, frame, preview, ClothGame::Reach, center);
}
