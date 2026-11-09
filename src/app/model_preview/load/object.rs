//! Object tags: the model an object names, its features, its colours and functions, and the weapons a unit holds.

use super::*;

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
    let object = ObjectLoad {
        reader,
        object: model_tag,
        model,
    };
    object.read_features(&mut data, &mut notes);
    let weapons_family = matches!(
        &reader.source,
        TagSource::LooseFolder {
            game: Some(
                GameId::Halo2
                    | GameId::Halo3
                    | GameId::Halo3Odst
                    | GameId::HaloReach
                    | GameId::Halo4
            ),
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
            super::super::weapons::variants::attach_variant_objects(
                reader, model, variants, preview,
            )
        };
        data.attached.extend(feature_into(
            &mut notes,
            "the variants' children",
            preview,
            attach,
        ));
    }
    data.notes = notes;
    Ok(data)
}

/// A later object being previewed: the kit, the object's own tag, and the
/// `.model` it names.
pub(super) struct ObjectLoad<'a> {
    pub(super) reader: &'a TagReader,
    pub(super) object: &'a TagFile,
    pub(super) model: &'a TagFile,
}

impl ObjectLoad<'_> {
    /// The object's change colors, functions and its game's own features,
    /// each noted if its reader crashes.
    fn read_features(&self, data: &mut ModelPreviewData, notes: &mut Vec<String>) {
        super::games::read_object_features(self, data, notes);
    }

    /// The object's change colors, then its functions.
    pub(super) fn read_colors_and_functions(
        &self,
        data: &mut ModelPreviewData,
        notes: &mut Vec<String>,
    ) {
        let Self {
            object: model_tag,
            model,
            ..
        } = *self;
        data.change_colors = feature(notes, "its change colors", || read_change_colors(model_tag));
        let functions = || super::super::functions::read_object_functions(model_tag, Some(model));
        data.object_functions = feature(notes, "its functions", functions);
    }

    /// What a biped holds: the settings' primary in its right hand and
    /// secondary in its left, each noted if it will not attach.
    fn attach_unit_weapons(
        &self,
        data: &mut ModelPreviewData,
        notes: &mut Vec<String>,
        settings: &PreviewLoadSettings,
    ) {
        let Self {
            reader,
            object: model_tag,
            ..
        } = *self;
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
                    super::super::weapons::attach_weapon(
                        reader, model_tag, path, left, owner, ik, preview,
                    )
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
        for path in std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
        {
            if path.is_dir() {
                folders.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "weapon")
            {
                let inner = path.strip_prefix(root).unwrap_or(&path).with_extension("");
                found.push(inner.to_string_lossy().replace('/', "\\"));
            }
        }
    }
    found
}
