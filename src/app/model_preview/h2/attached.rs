//! What hangs on a Halo 2 object, its own or one attached to another.

use std::sync::Arc;
use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::cloth::ClothGame;
use super::super::*;
use super::super::load::LoadCx;
use crate::app::model_preview::common::attached::named;

/// What hangs on a Halo 2 object, its own or one attached to another: its
/// flares and lights, antennas (its own only), particle systems, light
/// volumes, contrails, liquids, cloth and extension weapons, at `cx`'s markers
/// as its frame places them. An attached object's carry its name, which its
/// owner's panel plays its event effects by.
pub(in crate::app::model_preview) fn read_h2_features<F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize)>(
    cx: &LoadCx<'_, F>,
    preview: &mut RenderModelPreview,
) {
    let LoadCx { reader, object, owner, markers, frame, name } = *cx;
    let (flares, lights, gels) = super::lights::read_h2_flares(reader, object, owner, markers, frame);
    preview.h2.flares.extend(named(flares, name));
    preview.h2.point_lights.extend(named(lights, name));
    preview.h2.gel_lights.extend(named(gels, name));
    // `object_attach_antennas` runs for the object itself; what it holds
    // brings none.
    if name.is_none() {
        preview.antennas = super::super::antennas::read_antennas(reader, object, markers);
    }
    // Particle systems carry the held object's event effects (`firing
    // effect` and the rest), which its owner's panel plays by name.
    let particles = super::super::particles::read_h3_particles(reader, object, owner, markers, frame);
    preview.particles.extend(named(particles, name).map(Arc::new));
    let volumes = super::light_volumes::read_h2_light_volumes(reader, object, owner, markers, frame);
    preview.h2.light_volumes.extend(named(volumes, name));
    let contrails = super::contrails::read_h2_contrails(reader, object, owner, markers, frame);
    preview.h2.contrails.extend(named(contrails, name));
    let liquids = super::liquid::read_h2_liquids(reader, object, owner, markers, frame);
    preview.h2.liquids.extend(named(liquids, name));
    super::super::cloth::read_cloth(cx, preview, ClothGame::Halo2, [0.0; 3]);
    super::super::weapons::attached::read_extension_weapons(cx, preview);
}
