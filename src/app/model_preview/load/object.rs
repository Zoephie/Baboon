//! Object tags: the model an object names, its features, its colours and functions, and the weapons a unit holds.

use super::*;

/// Halo CE object-family tags are the equivalent of later engines' model
/// wrapper: the object directly names its gbxmodel, collision, animation,
/// and legacy physics tags. Start with the render reference here; collision
/// is added by the background overlay path.
pub(super) fn load_ce_object(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: Option<&TagReader>,
    settings: &PreviewLoadSettings,
) -> Result<ModelPreviewData, String> {
    let Some(reader) = reader else {
        return Err("Halo CE object preview requires a loaded source.".to_owned());
    };
    let reference = object_reference(model_tag, "model")
        .ok_or("This object references no gbxmodel.")?;
    let render = reader.load(&reference, "gbxmodel", b"mod2")
        .map_err(|error| format!("Could not load {reference}.gbxmodel: {error}"))?;
    let mut preview = build_render_preview(&render)?;
    if preview.batches.is_empty() {
        return Err("Referenced gbxmodel has no previewable draw batches.".to_owned());
    }
    if let Some((shader_group, shader_path)) =
        super::super::ce::functions::read_ce_modifier_shader(model_tag)
    {
        preview.materials.push(RenderModelPreviewMaterial {
            shader_path,
            shader_group,
            overlay: true, shield: None,
            permutation: 0,
            object: 0,
        });
    }
    // `render_model_parts`: an object's own index wins over each shader's.
    let forced = super::super::ce::functions::read_ce_forced_permutation(model_tag);
    if forced != 0 {
        preview.materials.iter_mut().for_each(|material| material.permutation = forced);
    }
    let mut notes = Vec::new();
    // A vehicle's guns are part of it. A biped holds what it is handed.
    let is_biped = model_tag.header.group_tag == u32::from_be_bytes(*b"bipd");
    let chosen = settings.weapon.as_deref();
    let weapons = feature(&mut notes, "the biped's weapons", || {
        let known = settings.ce_equippable.as_deref();
        is_biped.then(|| super::super::ce::weapons::biped_weapons(reader, model_tag, chosen, known))
    });
    let held = match &weapons {
        Some(weapons) => chosen
            .filter(|path| weapons.equippable.iter().any(|weapon| weapon.path == *path))
            .map(str::to_owned),
        None => super::super::ce::weapons::first_weapon(model_tag),
    };
    let attached = held.and_then(|path| {
        let attach = |preview: &mut RenderModelPreview| {
            super::super::ce::weapons::attach_ce_weapon(reader, model_tag, &path, preview)
        };
        let attached = feature_into(&mut notes, "the held weapon", &mut preview, attach);
        if attached.is_none() {
            notes.push(format!("the weapon {path} could not be attached"));
        }
        attached
    });
    feature_into(&mut notes, "its lights and effects", &mut preview, |preview| {
        super::super::ce::object::read_ce_object_features(reader, model_tag, attached.as_ref(), preview);
    });
    let mut data = model_preview_data(entry.key.clone(), reference, preview, Vec::new());
    data.change_colors = feature(&mut notes, "its change colors", || read_h2_change_colors(model_tag));
    data.ce_object_functions =
        feature(&mut notes, "its functions", || super::super::ce::functions::read_ce_object_functions(model_tag));
    data.ce_attached.extend(attached);
    data.ce_weapons = weapons;
    data.notes = notes;
    Ok(data)
}

/// Every later object names a `.model`, which previews as itself. The
/// object has the change colors, so nothing needs to go looking for it: its
/// own features, then what its unit holds, then what its variants attach.
pub(super) fn load_object(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    reader: Option<&TagReader>,
    settings: &PreviewLoadSettings,
) -> Result<ModelPreviewData, String> {
    let Some(reader) = reader else {
        return Err("Object preview requires a loaded source.".to_owned());
    };
    // Previewing the object itself again would come straight back here.
    let Some(model) = preview_model_tag(&reader.source, model_tag)? else {
        return Err("This object has no model to preview.".to_owned());
    };
    let model = &model;
    // `load_tag_preview`, not `load_model_preview`: the object's own load
    // already adds what that adds (the H4 colour grading, read once).
    let mut data = load_tag_preview(model, entry, names, Some(reader), settings)?;
    data.default_variant = object_default_variant(model_tag);
    let mut notes = std::mem::take(&mut data.notes);
    let object = ObjectLoad { reader, object: model_tag, model };
    object.read_features(&mut data, &mut notes);
    let weapons_family = matches!(
        &reader.source,
        TagSource::LooseFolder {
            game: Some(GameId::Halo2 | GameId::Halo3 | GameId::Halo3Odst | GameId::HaloReach | GameId::Halo4),
            ..
        }
    );
    if weapons_family {
        object.attach_unit_weapons(&mut data, &mut notes, settings);
    }
    if weapons_family {
        let preview = std::sync::Arc::make_mut(&mut data.preview);
        let variants = &mut data.variants;
        let attach = |preview: &mut RenderModelPreview| {
            super::super::weapons::variants::attach_variant_objects(reader, model, variants, preview)
        };
        data.attached.extend(feature_into(&mut notes, "the variants' children", preview, attach));
    }
    data.notes = notes;
    Ok(data)
}

/// A later object being previewed: the kit, the object's own tag, and the
/// `.model` it names.
pub(super) struct ObjectLoad<'a> {
    reader: &'a TagReader,
    object: &'a TagFile,
    model: &'a TagFile,
}

impl ObjectLoad<'_> {
    /// The object's change colors, functions and its game's own features,
    /// each noted if its reader crashes.
    fn read_features(&self, data: &mut ModelPreviewData, notes: &mut Vec<String>) {
        let Self { reader, object: model_tag, model } = *self;
        if matches!(&reader.source, TagSource::LooseFolder { game: Some(GameId::Halo3 | GameId::Halo3Odst), .. }) {
            self.read_colors_and_functions(data, notes);
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            feature_into(notes, "its lights and effects", preview, |preview| {
                super::super::h3::object::read_h3_object_features(reader, model_tag, model, preview);
            });
        }
        // Reach's attachments, lights and effects read differently, and wait
        // on their own readers; what the object's shaders read does not.
        if matches!(&reader.source, TagSource::LooseFolder { game: Some(GameId::HaloReach), .. }) {
            self.read_colors_and_functions(data, notes);
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            feature_into(notes, "its flares and lights", preview, |preview| {
                super::super::reach::object::read_reach_object_features(reader, model_tag, model, preview);
            });
        }
        // Halo 4's functions and switches feed its materials' functions; its
        // attachments, lights and effects wait on their own readers.
        if matches!(&reader.source, TagSource::LooseFolder { game: Some(GameId::Halo4), .. }) {
            self.read_colors_and_functions(data, notes);
            self.read_h4_features(data, notes);
        }
        if matches!(&reader.source, TagSource::LooseFolder { game: Some(GameId::Halo2), .. }) {
            self.read_colors_and_functions(data, notes);
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            feature_into(notes, "its lights and effects", preview, |preview| {
                super::super::h2::object::read_h2_object_features(reader, model_tag, preview);
            });
        }
    }

    /// The object's change colors, then its functions.
    fn read_colors_and_functions(&self, data: &mut ModelPreviewData, notes: &mut Vec<String>) {
        let Self { object: model_tag, model, .. } = *self;
        data.change_colors = feature(notes, "its change colors", || read_h2_change_colors(model_tag));
        let functions = || super::super::functions::read_object_functions(model_tag, Some(model));
        data.object_functions = feature(notes, "its functions", functions);
    }

    /// Halo 4's object reads: its bounds, change color count, camo, shield,
    /// shadow caster, IK markers, then its antennas and cloth.
    fn read_h4_features(&self, data: &mut ModelPreviewData, notes: &mut Vec<String>) {
        let Self { reader, object: model_tag, model } = *self;
        let preview = std::sync::Arc::make_mut(&mut data.preview);
        preview.h4.bounding_sphere = super::super::h4::object::read_h4_bounding_sphere(model_tag);
        preview.h4.change_color_count = super::super::h4::object::read_h4_change_color_count(model_tag);
        preview.h4.camo = super::super::h4::camo::H4Camo::read(reader).map(std::sync::Arc::new);
        preview.h4.shield = super::super::h4::shield::read_h4_shield(reader, model);
        preview.h4.shadow_caster = super::super::h4::shadow::h4_shadow_caster(model_tag);
        // The unit's markers its IK sets and chains place.
        let own = |frame: &crate::app::model_preview::common::marker::MarkerFrame| (frame.axes, frame.position);
        preview.gen3.ik_markers = super::super::animation::ik::reach::reach_ik_markers(preview, super::super::animation::ik::reach::ReachIkHolder::Unit, own, None);
        let markers = preview.markers.clone();
        let frame = |marker: &RenderModelPreviewMarker| own_marker_frame(marker);
        let center = std::array::from_fn(|axis| (preview.bounds_min[axis] + preview.bounds_max[axis]) * 0.5);
        feature_into(notes, "its antennas and cloth", preview, |preview| {
            let cx = LoadCx { reader, object: model_tag, owner: 0, markers: &markers, frame: &frame, name: None };
            super::super::h4::attached::read_h4_attached_features(&cx, center, preview);
        });
    }

    /// What a biped holds: the settings' primary in its right hand and
    /// secondary in its left, each noted if it will not attach.
    fn attach_unit_weapons(&self, data: &mut ModelPreviewData, notes: &mut Vec<String>, settings: &PreviewLoadSettings) {
        let Self { reader, object: model_tag, .. } = *self;
        // Halo 4's kit graphs all use data-driven animation, which places the
        // hands by IK sets and chains (`biped_postprocess_node_matrices`,
        // X360 0x84B1A738), never the legacy `weapon ik`.
        let legacy_weapon_ik = reader.source.game() != Some(GameId::Halo4);
        let known = settings.h2_equippable.as_deref();
        let is_biped = model_tag.header.group_tag == u32::from_be_bytes(*b"bipd");
        let weapons = feature(notes, "the unit's weapons", || {
            is_biped.then(|| super::super::weapons::unit_weapons(reader, model_tag, known))
        });
        if let Some(weapons) = weapons {
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            let primary = settings.weapon.as_deref();
            let secondary = settings.secondary_weapon.as_deref();
            for (path, left, owner) in [(primary, false, 1), (secondary, true, 2)] {
                let Some(path) = path else {
                    continue;
                };
                // `unit_apply_ik` leaves two weapons in their own hands.
                let ik = (secondary.is_none() && legacy_weapon_ik).then_some(&weapons);
                let attach = |preview: &mut RenderModelPreview| {
                    super::super::weapons::attach_weapon(reader, model_tag, path, left, owner, ik, preview)
                };
                let attached = feature_into(notes, "a held weapon", preview, attach);
                if attached.is_none() {
                    notes.push(format!("the weapon {path} could not be attached"));
                }
                data.attached.extend(attached);
            }
            data.weapons = Some(weapons);
        }
    }
}

/// `.weapon` tags of a loose folder, by path without the extension.
pub(in crate::app::model_preview) fn loose_weapon_paths(root: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for path in std::fs::read_dir(folder).into_iter().flatten().flatten().map(|entry| entry.path()) {
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|extension| extension == "weapon") {
                let inner = path.strip_prefix(root).unwrap_or(&path).with_extension("");
                found.push(inner.to_string_lossy().replace('/', "\\"));
            }
        }
    }
    found
}
