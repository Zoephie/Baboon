//! A Halo 3 or ODST object's own features, read when it opens.

use super::super::*;
use crate::core::source::TagSource;
use blam_tags::TagFile;
use super::super::cloth::ClothGame;

/// A Halo 3 or ODST object's own shield, lights, flares, light volumes,
/// particles, strips, screen effects, antennas, cloth, extension weapons and
/// leaves; `model` is its `.model`.
pub(in crate::app::model_preview) fn read_h3_object_features(source: &TagSource, model_tag: &TagFile, model: &TagFile, preview: &mut RenderModelPreview) {
    let frame = own_marker_frame;
    preview.h3.shield = model
        .root()
        .read_tag_ref_path("shield impact parameter override")
        .filter(|path| !path.is_empty())
        .and_then(|path| super::super::gen3::materials::read_h3_shield_settings(source, &path));
    preview.h3.lights = super::super::h3::lights::read_h3_lights(source, model_tag, 0, &preview.markers, frame);
    preview.gen3.flares = super::super::gen3::flares::read_h3_flares(source, model_tag, 0, &preview.markers, frame);
    preview.gen3.flares.extend(super::super::gen3::flares::light_flares(&preview.h3.lights));
    preview.gen3.light_volumes =
        super::super::gen3::light_volumes::read_h3_light_volumes(source, model_tag, 0, &preview.markers, frame);
    let systems = super::super::particles::read_h3_particles(source, model_tag, 0, &preview.markers, frame);
    preview.particles = systems.into_iter().map(Arc::new).collect();
    let strips = super::super::gen3::strips::read_h3_strips(source, model_tag, 0, &preview.markers, frame);
    preview.gen3.strips = strips.into_iter().map(Arc::new).collect();
    let carried = super::super::particles::carried_strips(&preview.particles);
    preview.gen3.strips.extend(carried);
    preview.gen3.screen_effects =
        super::super::gen3::screen_effects::read_h3_screen_effects(source, model_tag, 0, &preview.markers, frame);
    super::super::gen3::screen_effects::add_screen_shader_materials(preview);
    if preview.gen3.screen_effects.iter().any(|effect| effect.turns_on_vision()) {
        preview.h3.vision = super::super::h3::vision::read_h3_vision(source, model_tag).map(Arc::new);
    }
    if preview.gen3.screen_effects.iter().any(|effect| effect.sucks()) {
        preview.h3.motion_blur = super::super::h3::vision::read_h3_motion_blur(source);
    }
    if preview.gen3.screen_effects.iter().any(|effect| effect.adds_noise()) {
        preview.reach.screen_noise = super::super::gen3::screen_effects::read_reach_screen_noise(source);
    }
    preview.antennas = super::super::antennas::read_antennas(source, model_tag, &preview.markers);
    let markers = preview.markers.clone();
    super::super::cloth::read_cloth(source, model_tag, 0, &markers, frame, preview, ClothGame::Halo3, [0.0; 3]);
    super::super::weapons::attached::read_extension_weapons(source, model_tag, 0, &markers, &frame, preview, None);
    let middle = std::array::from_fn(|axis| (preview.bounds_min[axis] + preview.bounds_max[axis]) * 0.5);
    let leaves = super::super::h3::leaves::read_h3_leaves(source, model_tag, &markers, middle);
    // Framed by a null model alone, the eye sits among the leaves and
    // one passing the lens fills the view.
    for corner in leaves.iter().flat_map(|system| system.emission_corners()) {
        expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, corner);
    }
    preview.h3.leaves = leaves.into_iter().map(Arc::new).collect();
}
