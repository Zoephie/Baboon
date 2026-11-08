//! A Halo 2 object's own features, read when it opens.

use super::super::*;
use blam_tags::TagFile;

/// A Halo 2 object's own flares, lights, antennas, particles, light volumes,
/// contrails, liquids, cloth and extension weapons.
pub(in crate::app::model_preview) fn read_h2_object_features(reader: &TagReader, model_tag: &TagFile, preview: &mut RenderModelPreview) {
    let markers = std::mem::take(&mut preview.markers);
    let frame = crate::app::model_preview::common::marker::own_marker_frame;
    let cx = super::super::load::LoadCx { reader, object: model_tag, owner: 0, markers: &markers, frame: &frame, name: None };
    super::attached::read_h2_features(&cx, preview);
    preview.markers = markers;
}
