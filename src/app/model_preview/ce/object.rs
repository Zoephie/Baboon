//! A classic Halo CE object's own features, read when it opens.

use super::super::*;
use crate::core::source::TagSource;
use blam_tags::TagFile;

/// A Halo CE object's own lights, antennas, effects, contrails, light volumes,
/// lightning, glows, particle systems, flags and the effects it names for its
/// events, then those of the weapon it holds, as object 1 on its markers in
/// the hand.
pub(in crate::app::model_preview) fn read_ce_object_features(
    source: &TagSource,
    model_tag: &TagFile,
    attached: Option<&super::super::ce::attachments::CeAttachedObject>,
    preview: &mut RenderModelPreview,
) {
    let own = super::super::ce::lights::read_ce_lights(source, model_tag, 0, &preview.markers, |marker| {
        let frame = super::super::attachment::MarkerFrame {
            axes: marker.axes,
            position: marker.position,
            node: marker.node_index.max(0),
        };
        (frame, marker.node_index.max(0) as usize)
    });
    // The opened object's own lights come first, then the held weapon's
    // that `attach_ce_weapon` added, as every other feature lists them.
    let held_lights = std::mem::replace(&mut preview.ce_lights, own);
    preview.ce_lights.extend(held_lights);
    preview.ce_antennas = super::super::ce::lights::read_ce_antennas(source, model_tag, 0, &preview.markers);
    preview.ce_effects = super::super::ce::particles::read_ce_effects(source, model_tag, 0, &preview.markers);
    preview.ce_contrails = super::super::ce::contrails::read_ce_contrails(source, model_tag, 0, &preview.markers);
    preview.ce_light_volumes = super::super::ce::widgets::read_ce_light_volumes(source, model_tag, 0, &preview.markers);
    preview.ce_lightning = super::super::ce::widgets::read_ce_lightning(source, model_tag, 0, &preview.markers);
    preview.ce_glows = super::super::ce::widgets::read_ce_glows(source, model_tag, 0, &preview.markers);
    preview.ce_particle_systems = super::super::ce::particle_systems::read_ce_particle_systems(source, model_tag, 0, &preview.markers);
    // What hangs on the held weapon, as object 1, on its markers in the hand.
    let held_tag = attached
        .and_then(|held| load_referenced_tag_from_source(source, &held.path, "weapon", b"weap").ok().map(|tag| (tag, held)));
    if let Some((weapon, held)) = &held_tag {
        let markers = &held.markers;
        preview.ce_antennas.extend(super::super::ce::lights::read_ce_antennas(source, weapon, 1, markers));
        preview.ce_effects.extend(super::super::ce::particles::read_ce_effects(source, weapon, 1, markers));
        preview.ce_contrails.extend(super::super::ce::contrails::read_ce_contrails(source, weapon, 1, markers));
        preview.ce_light_volumes.extend(super::super::ce::widgets::read_ce_light_volumes(source, weapon, 1, markers));
        preview.ce_lightning.extend(super::super::ce::widgets::read_ce_lightning(source, weapon, 1, markers));
        preview.ce_glows.extend(super::super::ce::widgets::read_ce_glows(source, weapon, 1, markers));
        preview.ce_particle_systems.extend(super::super::ce::particle_systems::read_ce_particle_systems(source, weapon, 1, markers));
    }
    // What each names for its events, which the panel plays: the weapon's on
    // the weapon, as `weapon_effect_new` makes them.
    let events = super::super::ce::particles::read_ce_event_effects(source, model_tag, 0, &preview.markers);
    preview.ce_effects.extend(events);
    if let Some((weapon, held)) = &held_tag {
        preview.ce_effects.extend(super::super::ce::particles::read_ce_event_effects(source, weapon, 1, &held.markers));
    }
    let own = super::super::ce::firing::read_ce_weapon_firing(model_tag, 0, &preview.ce_effects);
    let held = held_tag.as_ref().and_then(|(weapon, _)| super::super::ce::firing::read_ce_weapon_firing(weapon, 1, &preview.ce_effects));
    preview.ce_firing = own.into_iter().chain(held).collect();
    let markers = preview.markers.clone();
    preview.ce_flags = super::super::ce::flags::read_ce_flags(source, model_tag, 0, &markers, preview);
    if let Some((weapon, held)) = &held_tag {
        let flags = super::super::ce::flags::read_ce_flags(source, weapon, 1, &held.markers, preview);
        preview.ce_flags.extend(flags);
    }
}
