//! A classic Halo CE object's own features, read when it opens.

use super::super::load::{build_render_preview_at, feature, feature_into};
use super::super::object_reference;
use super::super::{
    ModelPreviewData, ModelRegionSelection, PreviewLoadSettings, RenderModelPreview,
    RenderModelPreviewMaterial, TagEntry, TagReader, model_preview_data, read_change_colors,
};
use super::markers::{CeMarker, CeObjectMarkers};
use blam_tags::TagFile;
use std::collections::HashMap;

/// What an opened Halo CE object loads beside its geometry.
#[derive(Clone, Default)]
pub(in crate::app) struct CeObjectData {
    /// The object's functions.
    pub(in crate::app) functions: Option<super::functions::CeObjectFunctions>,
    /// What the object brings with it.
    pub(in crate::app) attached: Vec<super::weapons::CeAttachedObject>,
    /// Bipeds: what the unit can hold.
    pub(in crate::app) weapons: Option<super::weapons::CeBipedWeapons>,
    /// The region picks its markers were found under, given back to the
    /// panel when the load lands.
    pub(in crate::app) picks: Option<HashMap<String, ModelRegionSelection>>,
    /// Those of them a pick could move a marker in: another pick there
    /// loads it again (`markers::picks_match`).
    pub(in crate::app) marker_picks: Option<super::markers::CeMarkerPicks>,
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

/// A Halo CE object's own lights, antennas, effects, contrails, light volumes,
/// lightning, glows, particle systems, flags and the effects it names for its
/// events, then those of the weapon it holds, as object 1: each at its
/// markers under the permutations `picks` shows, a held weapon's in the hand.
/// A held weapon with no model hangs all it has at its origin, and its event
/// effects go on the opened object.
pub(in crate::app::model_preview) fn read_ce_object_features(
    reader: &TagReader,
    model_tag: &TagFile,
    attached: Option<&super::super::ce::weapons::CeAttachedObject>,
    picks: &HashMap<String, ModelRegionSelection>,
    preview: &mut RenderModelPreview,
) {
    // Each object's markers as `object_get_marker_by_name` finds them under
    // the permutations shown: the opened one's, and the held weapon's in the
    // hand, as object 1.
    let own =
        CeObjectMarkers::of_markers(&preview.markers, &preview.regions, picks, CeMarker::ORIGIN);
    let held = attached.and_then(|held| {
        let weapon = reader.load(&held.path, "weapon", b"weap").ok()?;
        let markers =
            CeObjectMarkers::of_markers(&held.markers, &preview.regions, picks, held.origin);
        Some((weapon, markers, held.hidden))
    });
    let objects = std::iter::once((model_tag, 0, &own)).chain(
        held.iter()
            .map(|(weapon, markers, _)| (weapon.as_ref(), 1, markers)),
    );
    // Without Game Shaders, the flags alone (`TagReader::game_shaders`).
    if !reader.game_shaders() {
        read_flags(reader, objects, preview);
        return;
    }
    // Whatever hangs on each, the opened object's first: `render_object_list`
    // skips a hidden object's model alone, so a held weapon with none still
    // lights and draws its widgets.
    for (object, owner, markers) in objects.clone() {
        let lights = super::super::ce::lights::read_ce_lights(reader, object, owner, markers);
        preview.ce.lights.extend(lights);
        read_ce_features(reader, object, owner, markers, &mut preview.ce);
    }
    // What each names for its events, which the panel plays: the weapon's on
    // the weapon, as `weapon_effect_new` makes them, or on the unit when the
    // weapon is hidden.
    let events = super::super::ce::effects::read_ce_event_effects(reader, model_tag, 0, &own);
    preview.ce.effects.extend(events);
    if let Some((weapon, markers, hidden)) = &held {
        let (on, at) = if *hidden { (0, &own) } else { (1, markers) };
        preview
            .ce
            .effects
            .extend(super::super::ce::effects::read_ce_event_effects(
                reader, weapon, on, at,
            ));
    }
    let own_firing =
        super::super::ce::firing::read_ce_weapon_firing(model_tag, 0, 0, &preview.ce.effects);
    let held_firing = held.as_ref().and_then(|(weapon, _, hidden)| {
        let effects_on = if *hidden { 0 } else { 1 };
        super::super::ce::firing::read_ce_weapon_firing(weapon, 1, effects_on, &preview.ce.effects)
    });
    preview.ce.firing = own_firing.into_iter().chain(held_firing).collect();
    read_flags(reader, objects, preview);
}

/// The flags each object hangs: the opened one's, then its held weapon's.
fn read_flags<'a>(
    reader: &TagReader,
    objects: impl Iterator<Item = (&'a TagFile, u8, &'a CeObjectMarkers)>,
    preview: &mut RenderModelPreview,
) {
    preview.ce.flags.clear();
    for (object, owner, markers) in objects {
        let flags = super::super::ce::flags::read_ce_flags(reader, object, owner, markers, preview);
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
    markers: &CeObjectMarkers,
    ce: &mut super::super::ce::CeFeatures,
) {
    ce.antennas
        .extend(super::antennas::read_ce_antennas(reader, object, markers));
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
    let mut preview = build_render_preview_at(&render, settings.detail_level)?;
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
    // The permutations its markers are found under: the panel's, once a load
    // gave it some, else those it starts in.
    let mut picks = super::super::selection::base_selection(&preview);
    if let Some(chosen) = &settings.ce.picks {
        for (region, selection) in chosen {
            if let Some(kept) = picks.get_mut(region) {
                *kept = selection.clone();
            }
        }
    }
    let every_marker: Vec<_> = preview
        .markers
        .iter()
        .chain(attached.iter().flat_map(|held| &held.markers))
        .cloned()
        .collect();
    let marker_picks = super::markers::marker_picks(&every_marker, &preview.regions, &picks);
    feature_into(
        &mut notes,
        "its lights and effects",
        &mut preview,
        |preview| {
            super::object::read_ce_object_features(
                reader,
                model_tag,
                attached.as_ref(),
                &picks,
                preview,
            );
        },
    );
    // Named with its extension: a bare path is a later game's `render_model`
    // to the Material Editor, which holds it to switch the shaders it names.
    let mut data = model_preview_data(
        entry.key.clone(),
        format!("{reference}.gbxmodel"),
        preview,
        Vec::new(),
    );
    data.ce.picks = Some(picks);
    data.ce.marker_picks = Some(marker_picks);
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
