//! What hangs on a Halo 2 object attached to another.

use std::sync::Arc;
use blam_tags::TagFile;
use super::super::attachment::MarkerFrame;
use super::super::cloth::ClothGame;
use super::super::*;
use crate::core::source::TagSource;
use crate::app::model_preview::common::attached::Attached;

/// What hangs on a Halo 2 object brought into the preview: its flares and
/// lights, particle systems, light volumes, contrails, liquids, cloth and
/// extension weapons, at `markers` as `frame` places them, named `name`.
pub(in crate::app::model_preview) fn read_h2_attached_features<F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize)>(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: &F,
    name: &str,
    preview: &mut RenderModelPreview,
) {
    let (flares, lights, gels) =
        super::lights::read_h2_flares(source, object, owner, markers, frame);
    preview.h2.flares.extend(flares.into_iter().map(|flare| flare.on_attached(name)));
    preview.h2.point_lights.extend(lights.into_iter().map(|light| light.on_attached(name)));
    preview.h2.gel_lights.extend(gels.into_iter().map(|light| light.on_attached(name)));
    // Particle systems carry the held object's event effects (`firing
    // effect` and the rest), which its owner's panel plays by name.
    let particles = super::super::particles::read_h3_particles(source, object, owner, markers, frame);
    let particles: Vec<_> = particles.into_iter().map(|system| Arc::new(system.on_attached(name))).collect();
    preview.particles.extend(particles);
    let volumes = super::light_volumes::read_h2_light_volumes(source, object, owner, markers, frame);
    preview.h2.light_volumes.extend(volumes.into_iter().map(|volume| volume.on_attached(name)));
    let contrails = super::contrails::read_h2_contrails(source, object, owner, markers, frame);
    preview.h2.contrails.extend(contrails.into_iter().map(|contrail| contrail.on_attached(name)));
    let liquids = super::liquid::read_h2_liquids(source, object, owner, markers, frame);
    preview.h2.liquids.extend(liquids.into_iter().map(|liquid| liquid.on_attached(name)));
    super::super::cloth::read_h3_cloth(source, object, owner, markers, frame, preview, ClothGame::Halo2, [0.0; 3]);
    super::super::weapons::attached::read_extension_weapons(source, object, owner, markers, frame, preview, Some(name));
}
