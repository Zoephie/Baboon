//! A Halo 2 object's own features, read when it opens.

use super::super::*;
use crate::core::source::TagSource;
use blam_tags::TagFile;
use super::super::cloth::ClothGame;

/// A Halo 2 object's own flares, lights, antennas, particles, light volumes,
/// contrails, liquids, cloth and extension weapons.
pub(in crate::app::model_preview) fn read_h2_object_features(source: &TagSource, model_tag: &TagFile, preview: &mut RenderModelPreview) {
    let markers = std::mem::take(&mut preview.markers);
    let found = super::super::h2::lights::read_h2_flares(source, model_tag, 0, &markers, |marker| {
        let place = super::super::attachment::MarkerFrame {
            axes: marker.axes,
            position: marker.position,
            node: marker.node_index,
        };
        (place, marker.node_index.max(0) as usize)
    });
    (preview.h2_flares, preview.h2_point_lights, preview.h2_gel_lights) = found;
    preview.h2_antennas = super::super::h2::lights::read_h2_antennas(source, model_tag, &markers);
    let systems = super::super::particles::read_h3_particles(source, model_tag, 0, &markers, |marker| {
        let place = super::super::attachment::MarkerFrame {
            axes: marker.axes,
            position: marker.position,
            node: marker.node_index,
        };
        (place, marker.node_index.max(0) as usize)
    });
    preview.h3_particles = systems.into_iter().map(Arc::new).collect();
    preview.h2_light_volumes = super::super::h2::light_volumes::read_h2_light_volumes(source, model_tag, 0, &markers, |marker| {
        let place = super::super::attachment::MarkerFrame {
            axes: marker.axes,
            position: marker.position,
            node: marker.node_index,
        };
        (place, marker.node_index.max(0) as usize)
    });
    preview.h2_contrails = super::super::h2::contrails::read_h2_contrails(source, model_tag, 0, &markers, |marker| {
        let place = super::super::attachment::MarkerFrame {
            axes: marker.axes,
            position: marker.position,
            node: marker.node_index,
        };
        (place, marker.node_index.max(0) as usize)
    });
    let frame = |marker: &RenderModelPreviewMarker| {
        let place = super::super::attachment::MarkerFrame {
            axes: marker.axes,
            position: marker.position,
            node: marker.node_index,
        };
        (place, marker.node_index.max(0) as usize)
    };
    let frame = &frame;
    preview.h2_liquids = super::super::h2::liquid::read_h2_liquids(source, model_tag, 0, &markers, frame);
    super::super::cloth::read_h3_cloth(source, model_tag, 0, &markers, frame, preview, ClothGame::Halo2, [0.0; 3]);
    super::super::weapons::attached::read_extension_weapons(source, model_tag, 0, &markers, frame, preview, None);
    preview.markers = markers;
}
