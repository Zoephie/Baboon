//! Preview tag loading and render-model resolution.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::common::geometry::{Overflow, VertexBase};
use super::common::marker::{MarkerFrame, attach_point, attach_vector};
use super::{ModelPreviewData, TagEntry, TagFile, TagNameIndex, TagSource};
use model::load_model;
use std::sync::Arc;

pub(in crate::app::model_preview) mod reader;
pub(in crate::app) use reader::TagReader;
mod variants;
pub(in crate::app::model_preview) use variants::*;
pub(in crate::app::model_preview) mod derived;
mod errors;
pub(in crate::app::model_preview) use derived::*;
mod merge;
pub(in crate::app::model_preview) use merge::*;
mod scenario;
pub(in crate::app::model_preview) use scenario::*;
mod jobs;
pub(in crate::app::model_preview) mod overlays;
pub(in crate::app::model_preview) use jobs::*;
mod guard;
pub(in crate::app::model_preview) use guard::{feature, feature_into};
mod render_model;
pub(in crate::app) use render_model::build_render_preview;
pub(in crate::app::model_preview) use render_model::*;
mod games;
use games::{add_overlay_shaders, load_own_object, load_unreal_model};
pub(in crate::app::model_preview) use games::{rules, type_exports};
mod rules;
pub(in crate::app::model_preview) use rules::GameRules;
mod object;
pub(in crate::app::model_preview) use object::*;
mod model;
use super::common::geometry::expand_preview_bounds_local;
mod cx;
pub(in crate::app::model_preview) use cx::*;

pub(super) fn load_model_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    settings: &PreviewLoadSettings,
) -> Result<ModelPreviewData, String> {
    // One load, one reader: what it reads is read once, and dropped with it.
    let reader = source.map(TagReader::new);
    let reader = reader.as_ref();
    let mut data = load_tag_preview(model_tag, entry, names, reader, settings)?;
    let generation = blam_tags::game::Game::of(model_tag);
    let game = games::game_of(generation, source);
    // Just built: no other handle shares it, so this does not copy.
    let preview = Arc::make_mut(&mut data.preview);
    preview.game = game;
    preview.generation = Some(generation);
    games::read_model_features(reader, &mut data);
    Ok(data)
}

/// The preview of `model_tag`, by its group: a particle model or a render
/// tag is its own geometry; an object reads its model and then its own
/// features; collision, physics and structure BSPs derive theirs; a scenario
/// composes the BSPs chosen; a Campaign Evolved model reads its Unreal meshes;
/// and any other `.model` previews the render model it names.
fn load_tag_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    reader: Option<&TagReader>,
    settings: &PreviewLoadSettings,
) -> Result<ModelPreviewData, String> {
    if blam_tags::is_particle_model_group(model_tag.header.group_tag) {
        return load_particle_model(model_tag, entry);
    }
    let group = model_tag.header.group_tag.to_be_bytes();
    if matches!(&group, b"mode" | b"mod2") {
        return load_render_tag(model_tag, entry);
    }
    if let Some(result) = load_own_object(model_tag, entry, reader, settings) {
        return result;
    }
    if super::is_object_family_group(
        model_tag.header.group_tag,
        reader.and_then(|reader| reader.source.game()),
    ) {
        return load_object(model_tag, entry, names, reader, settings);
    }
    if matches!(&group, b"coll" | b"phmo") {
        return load_collision_or_physics(model_tag, entry, reader);
    }
    if &group == b"sbsp" {
        return load_structure_bsp(model_tag, entry);
    }
    if &group == b"scnr" {
        return load_scenario(model_tag, entry, reader, settings);
    }
    if let Some(result) = load_unreal_model(model_tag, entry, reader, settings) {
        return result;
    }
    load_model(model_tag, entry, names, reader)
}

#[cfg(test)]
mod tests;
