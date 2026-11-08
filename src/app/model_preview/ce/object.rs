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
    let held_lights = std::mem::replace(&mut preview.ce.lights, own);
    preview.ce.lights.extend(held_lights);
    // Empty until now: only the held weapon's lights come before.
    read_ce_features(source, model_tag, 0, &preview.markers, &mut preview.ce);
    // What hangs on the held weapon, as object 1, on its markers in the hand.
    let held_tag = attached
        .and_then(|held| load_referenced_tag_from_source(source, &held.path, "weapon", b"weap").ok().map(|tag| (tag, held)));
    if let Some((weapon, held)) = &held_tag {
        read_ce_features(source, weapon, 1, &held.markers, &mut preview.ce);
    }
    // What each names for its events, which the panel plays: the weapon's on
    // the weapon, as `weapon_effect_new` makes them.
    let events = super::super::ce::particles::read_ce_event_effects(source, model_tag, 0, &preview.markers);
    preview.ce.effects.extend(events);
    if let Some((weapon, held)) = &held_tag {
        preview.ce.effects.extend(super::super::ce::particles::read_ce_event_effects(source, weapon, 1, &held.markers));
    }
    let own = super::super::ce::firing::read_ce_weapon_firing(model_tag, 0, &preview.ce.effects);
    let held = held_tag.as_ref().and_then(|(weapon, _)| super::super::ce::firing::read_ce_weapon_firing(weapon, 1, &preview.ce.effects));
    preview.ce.firing = own.into_iter().chain(held).collect();
    let markers = preview.markers.clone();
    preview.ce.flags = super::super::ce::flags::read_ce_flags(source, model_tag, 0, &markers, preview);
    if let Some((weapon, held)) = &held_tag {
        let flags = super::super::ce::flags::read_ce_flags(source, weapon, 1, &held.markers, preview);
        preview.ce.flags.extend(flags);
    }
}

/// What hangs on a Halo CE object, the opened one or the weapon it holds, as
/// `owner` at `markers`: its antennas, effects, contrails, light volumes,
/// lightning, glows and particle systems, added after what `ce` holds.
fn read_ce_features(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    ce: &mut super::super::ce::CeFeatures,
) {
    ce.antennas.extend(super::super::antennas::ce::read_ce_antennas(source, object, owner, markers));
    ce.effects.extend(super::super::ce::particles::read_ce_effects(source, object, owner, markers));
    ce.contrails.extend(super::super::ce::contrails::read_ce_contrails(source, object, owner, markers));
    ce.light_volumes.extend(super::super::ce::widgets::read_ce_light_volumes(source, object, owner, markers));
    ce.lightning.extend(super::super::ce::widgets::read_ce_lightning(source, object, owner, markers));
    ce.glows.extend(super::super::ce::widgets::read_ce_glows(source, object, owner, markers));
    ce.particle_systems.extend(super::super::ce::particle_systems::read_ce_particle_systems(source, object, owner, markers));
}
