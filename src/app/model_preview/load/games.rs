//! The one place the loader names a game: what each game reads of an
//! object, a model and a Halo CE object of its own.

use super::*;

/// The object's change colors, functions and its game's own features, each
/// noted if its reader crashes, by the kit's game.
pub(super) fn read_object_features(object: &ObjectLoad<'_>, data: &mut ModelPreviewData, notes: &mut Vec<String>) {
    let ObjectLoad { reader, object: model_tag, model } = *object;
    let game = match &reader.source {
        TagSource::LooseFolder { game, .. } => *game,
        _ => None,
    };
    match game {
        Some(GameId::Halo3 | GameId::Halo3Odst) => {
            object.read_colors_and_functions(data, notes);
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            feature_into(notes, "its lights and effects", preview, |preview| {
                super::super::h3::object::read_h3_object_features(reader, model_tag, model, preview);
            });
        }
        // Reach's attachments, lights and effects read differently, and wait
        // on their own readers; what the object's shaders read does not.
        Some(GameId::HaloReach) => {
            object.read_colors_and_functions(data, notes);
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            feature_into(notes, "its flares and lights", preview, |preview| {
                super::super::reach::object::read_reach_object_features(reader, model_tag, model, preview);
            });
        }
        // Halo 4's functions and switches feed its materials' functions; its
        // attachments, lights and effects wait on their own readers.
        Some(GameId::Halo4) => {
            object.read_colors_and_functions(data, notes);
            super::super::h4::object::read_h4_object_features(reader, model_tag, model, data, notes);
        }
        Some(GameId::Halo2) => {
            object.read_colors_and_functions(data, notes);
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            feature_into(notes, "its lights and effects", preview, |preview| {
                super::super::h2::attached::read_h2_object_features(reader, model_tag, preview);
            });
        }
        _ => {}
    }
}

/// What a model's game reads of its own once its preview is built: Halo
/// 4's colour grading.
pub(super) fn read_model_features(reader: Option<&TagReader>, data: &mut ModelPreviewData) {
    if let Some(reader) = reader.filter(|reader| reader.source.game() == Some(GameId::Halo4)) {
        // Just built: no other handle shares it, so this does not copy.
        Arc::make_mut(&mut data.preview).h4.color_grading = super::super::h4::lighting::read_h4_color_grading(reader);
    }
}

/// The shaders a `.model` lays over its render model: Halo 2's hologram and
/// shield shaders, which no other game's `.model` names.
pub(super) fn add_overlay_shaders(model_tag: &TagFile, preview: &mut RenderModelPreview) {
    super::super::h2::overlays::add_overlay_shaders(model_tag, preview);
}

/// A Halo CE object, which names its gbxmodel itself.
pub(super) fn load_ce_object(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: Option<&TagReader>,
    settings: &PreviewLoadSettings,
) -> Result<ModelPreviewData, String> {
    super::super::ce::object::load_ce_object(model_tag, entry, reader, settings)
}
