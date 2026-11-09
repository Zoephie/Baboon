//! A classic Halo CE object's own features, read when it opens.

use super::super::load::{build_render_preview, feature, feature_into};
use super::super::object_reference;
use super::super::{
    ModelPreviewData, PreviewLoadSettings, RenderModelPreview, RenderModelPreviewMarker,
    RenderModelPreviewMaterial, TagEntry, TagReader, model_preview_data, read_change_colors,
};
use blam_tags::TagFile;

/// A Halo CE object's own lights, antennas, effects, contrails, light volumes,
/// lightning, glows, particle systems, flags and the effects it names for its
/// events, then those of the weapon it holds, as object 1 on its markers in
/// the hand.
pub(in crate::app::model_preview) fn read_ce_object_features(
    reader: &TagReader,
    model_tag: &TagFile,
    attached: Option<&super::super::ce::weapons::CeAttachedObject>,
    preview: &mut RenderModelPreview,
) {
    let own = super::super::ce::lights::read_ce_lights(
        reader,
        model_tag,
        0,
        &preview.markers,
        |marker| {
            let frame = crate::app::model_preview::common::marker::MarkerFrame {
                axes: marker.axes,
                position: marker.position,
                node: marker.node_index.max(0),
            };
            (frame, marker.node_index.max(0) as usize)
        },
    );
    // The opened object's own lights come first, then the held weapon's
    // that `attach_ce_weapon` added, as every other feature lists them.
    let held_lights = std::mem::replace(&mut preview.ce.lights, own);
    preview.ce.lights.extend(held_lights);
    // Empty until now: only the held weapon's lights come before.
    read_ce_features(reader, model_tag, 0, &preview.markers, &mut preview.ce);
    // What hangs on the held weapon, as object 1, on its markers in the hand.
    let held_tag = attached.and_then(|held| {
        reader
            .load(&held.path, "weapon", b"weap")
            .ok()
            .map(|tag| (tag, held))
    });
    if let Some((weapon, held)) = &held_tag {
        read_ce_features(reader, weapon, 1, &held.markers, &mut preview.ce);
    }
    // What each names for its events, which the panel plays: the weapon's on
    // the weapon, as `weapon_effect_new` makes them.
    let events =
        super::super::ce::effects::read_ce_event_effects(reader, model_tag, 0, &preview.markers);
    preview.ce.effects.extend(events);
    if let Some((weapon, held)) = &held_tag {
        preview
            .ce
            .effects
            .extend(super::super::ce::effects::read_ce_event_effects(
                reader,
                weapon,
                1,
                &held.markers,
            ));
    }
    let own = super::super::ce::firing::read_ce_weapon_firing(model_tag, 0, &preview.ce.effects);
    let held = held_tag.as_ref().and_then(|(weapon, _)| {
        super::super::ce::firing::read_ce_weapon_firing(weapon, 1, &preview.ce.effects)
    });
    preview.ce.firing = own.into_iter().chain(held).collect();
    let markers = preview.markers.clone();
    preview.ce.flags =
        super::super::ce::flags::read_ce_flags(reader, model_tag, 0, &markers, preview);
    if let Some((weapon, held)) = &held_tag {
        let flags =
            super::super::ce::flags::read_ce_flags(reader, weapon, 1, &held.markers, preview);
        preview.ce.flags.extend(flags);
    }
}

/// What hangs on a Halo CE object, the opened one or the weapon it holds, as
/// `owner` at `markers`: its antennas, effects, contrails, light volumes,
/// lightning, glows and particle systems, added after what `ce` holds.
fn read_ce_features(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    ce: &mut super::super::ce::CeFeatures,
) {
    ce.antennas
        .extend(super::super::antennas::ce::read_ce_antennas(
            reader, object, owner, markers,
        ));
    ce.effects
        .extend(super::super::ce::effects::read_ce_effects(
            reader, object, owner, markers,
        ));
    ce.contrails
        .extend(super::super::ce::contrails::read_ce_contrails(
            reader, object, owner, markers,
        ));
    ce.light_volumes
        .extend(super::super::ce::widgets::read_ce_light_volumes(
            reader, object, owner, markers,
        ));
    ce.lightning
        .extend(super::super::ce::widgets::read_ce_lightning(
            reader, object, owner, markers,
        ));
    ce.glows.extend(super::super::ce::widgets::read_ce_glows(
        reader, object, owner, markers,
    ));
    ce.particle_systems.extend(
        super::super::ce::particle_systems::read_ce_particle_systems(
            reader, object, owner, markers,
        ),
    );
}

/// Halo CE object-family tags are the equivalent of later engines' model
/// wrapper: the object directly names its gbxmodel, collision, animation,
/// and legacy physics tags. Start with the render reference here; collision
/// is added by the background overlay path.
pub(in crate::app::model_preview) fn load_ce_object(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: Option<&TagReader>,
    settings: &PreviewLoadSettings,
) -> Result<ModelPreviewData, String> {
    let Some(reader) = reader else {
        return Err("Halo CE object preview requires a loaded source.".to_owned());
    };
    let reference =
        object_reference(model_tag, "model").ok_or("This object references no gbxmodel.")?;
    let render = reader
        .load(&reference, "gbxmodel", b"mod2")
        .map_err(|error| format!("Could not load {reference}.gbxmodel: {error}"))?;
    let mut preview = build_render_preview(&render)?;
    if preview.batches.is_empty() {
        return Err("Referenced gbxmodel has no previewable draw batches.".to_owned());
    }
    if let Some((shader_group, shader_path)) = super::functions::read_ce_modifier_shader(model_tag)
    {
        preview.materials.push(RenderModelPreviewMaterial {
            shader_path,
            shader_group,
            overlay: true,
            shield: None,
            permutation: 0,
            object: 0,
        });
    }
    // `render_model_parts`: an object's own index wins over each shader's.
    let forced = super::functions::read_ce_forced_permutation(model_tag);
    if forced != 0 {
        preview
            .materials
            .iter_mut()
            .for_each(|material| material.permutation = forced);
    }
    let mut notes = Vec::new();
    // A vehicle's guns are part of it. A biped holds what it is handed.
    let is_biped = model_tag.header.group_tag == u32::from_be_bytes(*b"bipd");
    let chosen = settings.weapon.as_deref();
    let weapons = feature(&mut notes, "the biped's weapons", || {
        let known = settings.ce_equippable.as_deref();
        is_biped.then(|| super::weapons::biped_weapons(reader, model_tag, chosen, known))
    });
    let held = match &weapons {
        Some(weapons) => chosen
            .filter(|path| weapons.equippable.iter().any(|weapon| weapon.path == *path))
            .map(str::to_owned),
        None => super::weapons::first_weapon(model_tag),
    };
    let attached = held.and_then(|path| {
        let attach = |preview: &mut RenderModelPreview| {
            super::weapons::attach_ce_weapon(reader, model_tag, &path, preview)
        };
        let attached = feature_into(&mut notes, "the held weapon", &mut preview, attach);
        if attached.is_none() {
            notes.push(format!("the weapon {path} could not be attached"));
        }
        attached
    });
    feature_into(
        &mut notes,
        "its lights and effects",
        &mut preview,
        |preview| {
            super::object::read_ce_object_features(reader, model_tag, attached.as_ref(), preview);
        },
    );
    let mut data = model_preview_data(entry.key.clone(), reference, preview, Vec::new());
    data.change_colors = feature(&mut notes, "its change colors", || {
        read_change_colors(model_tag)
    });
    data.ce_object_functions = feature(&mut notes, "its functions", || {
        super::functions::read_ce_object_functions(model_tag)
    });
    data.ce_attached.extend(attached);
    data.ce_weapons = weapons;
    data.notes = notes;
    Ok(data)
}
