//! What a Halo 4 object reads of itself when it opens.

use super::super::common::marker::own_marker_frame;
use super::super::load::{LoadCx, feature_into};
use super::super::*;
use blam_tags::TagFile;

/// A Halo 4 object's `bounding offset` and `bounding radius`
/// (`object_compute_bounding_sphere_recursive`, X360 0x84A489C0, reads both).
pub(in crate::app::model_preview) fn read_h4_bounding_sphere(
    object: &TagFile,
) -> Option<([f32; 3], f32)> {
    let holder = super::super::common::fields::object_block(object, "bounding radius")?;
    let radius = holder.read_real("bounding radius")?;
    let offset = match holder
        .field("bounding offset")
        .and_then(|field| field.value())
    {
        Some(blam_tags::TagFieldData::RealPoint3d(point)) => [point.x, point.y, point.z],
        _ => [0.0; 3],
    };
    (radius > 0.0).then_some((offset, radius))
}

/// How many of a Halo 4 object's change colours reach its materials:
/// `render_object_update_change_colors` (X360 0x82A80E18) packs the first
/// four the object has (`object_get_change_color` answers for each index
/// below the block's count).
pub(in crate::app::model_preview) fn read_h4_change_color_count(object: &TagFile) -> u8 {
    let holder = super::super::common::fields::object_block(object, "change colors");
    let count = holder.map_or(0, |holder| {
        super::super::common::fields::elements(holder, "change colors").len()
    });
    count.min(4) as u8
}

/// Halo 4's object reads: its bounds, change color count, camo, shield,
/// shadow caster, IK markers, then its antennas and cloth.
pub(in crate::app::model_preview) fn read_h4_object_features(
    reader: &TagReader,
    model_tag: &TagFile,
    model: &TagFile,
    data: &mut ModelPreviewData,
    notes: &mut Vec<String>,
) {
    let preview = std::sync::Arc::make_mut(&mut data.preview);
    preview.h4.bounding_sphere = super::object::read_h4_bounding_sphere(model_tag);
    preview.h4.change_color_count = super::object::read_h4_change_color_count(model_tag);
    preview.h4.camo = super::camo::H4Camo::read(reader).map(std::sync::Arc::new);
    preview.h4.shield = super::shield::read_h4_shield(reader, model);
    preview.h4.shadow_caster = super::shadow::h4_shadow_caster(model_tag);
    // The unit's markers its IK sets and chains place.
    let own = |frame: &crate::app::model_preview::common::marker::MarkerFrame| {
        (frame.axes, frame.position)
    };
    preview.gen3.ik_markers = super::super::animation::ik::reach::reach_ik_markers(
        preview,
        super::super::animation::ik::reach::ReachIkHolder::Unit,
        own,
        None,
    );
    let markers = preview.markers.clone();
    let frame = |marker: &RenderModelPreviewMarker| own_marker_frame(marker);
    let center =
        std::array::from_fn(|axis| (preview.bounds_min[axis] + preview.bounds_max[axis]) * 0.5);
    feature_into(notes, "its antennas and cloth", preview, |preview| {
        let cx = LoadCx {
            reader,
            object: model_tag,
            owner: 0,
            markers: &markers,
            frame: &frame,
            name: None,
        };
        super::attached::read_h4_attached_features(&cx, center, preview);
    });
}
