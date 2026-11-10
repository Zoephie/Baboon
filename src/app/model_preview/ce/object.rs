//! A classic Halo CE object's own features, read when it opens.

use super::super::load::{build_render_preview, feature, feature_into};
use super::super::object_reference;
use super::super::{
    ModelPreviewData, PreviewLoadSettings, RenderModelPreview, RenderModelPreviewMarker,
    RenderModelPreviewMaterial, TagEntry, TagReader, model_preview_data, read_change_colors,
};
use blam_tags::TagFile;
use std::sync::Arc;

/// A Halo CE object's own lights, antennas, effects, contrails, light volumes,
/// lightning, glows, particle systems, flags and the effects it names for its
/// events, then those of the weapon it holds, as object 1 on its markers in
/// the hand.
/// What an opened Halo CE object loads beside its geometry.
#[derive(Clone, Default)]
pub(in crate::app) struct CeObjectData {
    /// The object's functions.
    pub(in crate::app) functions: Option<super::functions::CeObjectFunctions>,
    /// What the object brings with it.
    pub(in crate::app) attached: Vec<super::weapons::CeAttachedObject>,
    /// Bipeds: what the unit can hold.
    pub(in crate::app) weapons: Option<super::weapons::CeBipedWeapons>,
}

impl super::super::ModelPreviewData {
    /// The clips the unit's own held weapon hides, where its game picks them
    /// itself: Halo CE's, by the graph's weapon labels.
    pub(in crate::app::model_preview) fn own_hidden_clips(
        &self,
    ) -> Option<&std::collections::BTreeSet<usize>> {
        self.ce
            .weapons
            .as_ref()
            .map(|weapons| &weapons.hidden_clips)
    }
}

pub(in crate::app::model_preview) fn read_ce_object_features(
    reader: &TagReader,
    model_tag: &TagFile,
    attached: Option<&super::super::ce::weapons::CeAttachedObject>,
    preview: &mut RenderModelPreview,
) {
    // The held weapon, whose features hang on it as object 1.
    let held_tag = attached.and_then(|held| {
        reader
            .load(&held.path, "weapon", b"weap")
            .ok()
            .map(|tag| (tag, held))
    });
    // Without Game Shaders, the flags alone (`TagReader::game_shaders`).
    if !reader.game_shaders() {
        read_flags(reader, model_tag, held_tag.as_ref(), preview);
        return;
    }
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
    read_flags(reader, model_tag, held_tag.as_ref(), preview);
}

/// The object's flags, then its held weapon's, as object 1.
fn read_flags(
    reader: &TagReader,
    model_tag: &TagFile,
    held: Option<&(Arc<TagFile>, &super::super::ce::weapons::CeAttachedObject)>,
    preview: &mut RenderModelPreview,
) {
    let markers = preview.markers.clone();
    preview.ce.flags =
        super::super::ce::flags::read_ce_flags(reader, model_tag, 0, &markers, preview);
    if let Some((weapon, held)) = held {
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
    ce.antennas.extend(super::antennas::read_ce_antennas(
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
            ce: super::CeMaterialLook {
                overlay: true,
                permutation: 0,
            },
            h2: Default::default(),
            object: 0,
        });
    }
    // `render_model_parts`: an object's own index wins over each shader's.
    let forced = super::functions::read_ce_forced_permutation(model_tag);
    if forced != 0 {
        preview
            .materials
            .iter_mut()
            .for_each(|material| material.ce.permutation = forced);
    }
    let mut notes = Vec::new();
    // A vehicle's guns are part of it. A biped holds what it is handed.
    let is_biped = model_tag.header.group_tag == u32::from_be_bytes(*b"bipd");
    let chosen = settings.weapon.as_deref();
    let weapons = feature(&mut notes, "the biped's weapons", || {
        let known = settings.ce.equippable.as_deref();
        is_biped
            .then(|| super::weapons::biped_weapons(reader, model_tag, chosen, known))
            .flatten()
    });
    // A biped holds what was chosen: one its list has, or, before the list,
    // any it was handed.
    let held = match (is_biped, &weapons) {
        (true, Some(weapons)) => chosen
            .filter(|path| {
                weapons.equippable.as_ref().is_none_or(|listed| {
                    listed.iter().any(|weapon| weapon.path == *path)
                })
            })
            .map(str::to_owned),
        (true, None) => None,
        (false, _) => super::weapons::first_weapon(model_tag),
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
    // Game Shaders' alone (`TagReader::game_shaders`).
    if reader.game_shaders() {
        data.change_colors = feature(&mut notes, "its change colors", || {
            read_change_colors(model_tag)
        });
        data.ce.functions = feature(&mut notes, "its functions", || {
            super::functions::read_ce_object_functions(model_tag)
        });
    }
    data.ce.attached.extend(attached);
    data.ce.weapons = weapons;
    data.notes = notes;
    Ok(data)
}
