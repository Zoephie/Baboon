//! What a Halo 4 object reads of itself when it opens.

use blam_tags::TagFile;

/// A Halo 4 object's `bounding offset` and `bounding radius`
/// (`object_compute_bounding_sphere_recursive`, X360 0x84A489C0, reads both).
pub(in crate::app::model_preview) fn read_h4_bounding_sphere(object: &TagFile) -> Option<([f32; 3], f32)> {
    let holder = super::super::common::fields::object_block(object, "bounding radius")?;
    let radius = holder.read_real("bounding radius")?;
    let offset = match holder.field("bounding offset").and_then(|field| field.value()) {
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
    let count = holder.map_or(0, |holder| super::super::common::fields::elements(holder, "change colors").len());
    count.min(4) as u8
}
