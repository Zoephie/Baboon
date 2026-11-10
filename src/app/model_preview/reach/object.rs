//! A Reach object's own features, read when it opens.

use super::super::common::marker::own_marker_frame;
use super::super::{RenderModelPreview, RenderModelPreviewMarker, TagReader};
use blam_tags::TagFile;

/// A Reach object's own shield, camo, shadow caster and IK markers, then
/// what hangs on it: its lights, flares, light volumes, effects, antennas
/// and cloth (`read_reach_attached_features`).
pub(in crate::app::model_preview) fn read_reach_object_features(
    reader: &TagReader,
    model_tag: &TagFile,
    model: &TagFile,
    preview: &mut RenderModelPreview,
) {
    // Game Shaders' alone (`TagReader::game_shaders`).
    if reader.game_shaders() {
        preview.reach.shield = super::super::reach::shield::read_reach_shield(reader, model);
        preview.reach.camo =
            super::super::reach::camo::ReachCamo::read(reader).map(std::sync::Arc::new);
        preview.reach.shadow_caster =
            super::super::reach::shadow::reach_shadow_caster(model_tag, model);
    }
    let own = |frame: &crate::app::model_preview::common::marker::MarkerFrame| {
        (frame.axes, frame.position)
    };
    preview.gen3.ik_markers = super::super::animation::ik::solve::ik_markers(
        preview,
        super::super::animation::ik::solve::IkHolder::Unit,
        own,
        None,
    );
    let markers = preview.markers.clone();
    let frame = |marker: &RenderModelPreviewMarker| own_marker_frame(marker);
    // `object_get_bounding_sphere`'s middle, as the leaves take it.
    let center =
        std::array::from_fn(|axis| (preview.bounds_min[axis] + preview.bounds_max[axis]) * 0.5);
    let cx = super::super::load::LoadCx {
        reader,
        object: model_tag,
        owner: 0,
        markers: &markers,
        frame: &frame,
        name: None,
    };
    super::super::reach::attached::read_reach_attached_features(&cx, center, preview);
    // The final composite's noise map, which only an area screen effect
    // that adds bright or dark noise reads.
    if preview
        .gen3
        .screen_effects
        .iter()
        .any(|effect| effect.adds_noise())
    {
        preview.reach.screen_noise = super::composite::read_reach_screen_noise(reader);
    }
}
