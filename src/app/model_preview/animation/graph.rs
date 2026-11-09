//! An object's animation graphs: their chain, inheritance and listed animations.

use super::super::super::{TagEntry, TagSource};
use super::super::{load_referenced_tag_from_source, object_reference, preview_model_tag};
use super::decode::ce_jma_kind;
use super::playback::PreviewAnimationEntry;
use blam_tags::animation::classic::CeAnimations;
use blam_tags::paths::tag_ref_path;
use blam_tags::{Animation, TagFile};

/// Worker half of the list request.
pub(super) fn list_model_animations(
    source: &TagSource,
    entry: &TagEntry,
) -> Result<Vec<PreviewAnimationEntry>, String> {
    let model =
        crate::core::source::read_entry(source, entry).map_err(|error| error.to_string())?;
    // A Halo CE object names its `model_animations`; Halo 2 and the Halo 3
    // family name a `model_animation_graph` from the `.model`.
    match blam_tags::game::Game::of(&model) {
        blam_tags::game::Game::Halo1 => {
            let antr = load_object_animations(source, &model)?;
            Ok(CeAnimations::new(&antr)
                .iter()
                .map(|animation| PreviewAnimationEntry {
                    name: animation
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("animation {}", animation.index)),
                    frame_count: animation.frame_count,
                    kind: ce_jma_kind(animation).extension(),
                    playable: animation.frame_count > 0,
                })
                .collect())
        }
        blam_tags::game::Game::Halo2 | blam_tags::game::Game::Halo3 => {
            let model = preview_model_tag(source, &model)?.unwrap_or(model);
            let jmad_ref = tag_ref_path(&model.root(), "animation")
                .ok_or("This model references no animation graph.")?;
            let chain = load_graph_chain(source, &jmad_ref)?;
            let animations = chain
                .iter()
                .map(|jmad| Animation::new(jmad).map_err(|error| error.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(inherited_animation_order(&animations)
                .into_iter()
                .filter_map(|(graph, index)| animations[graph].get(index))
                .map(|group| PreviewAnimationEntry {
                    name: group
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("animation {}", group.index)),
                    frame_count: group.frame_count.max(0) as u16,
                    kind: blam_tags::extract::animation::jma_kind_for(group).extension(),
                    playable: !group.blob.is_empty(),
                })
                .collect())
        }
    }
}

/// A model's graph, then each `parent animation graph` above it. A parent
/// that will not load ends the chain rather than losing the graph's own list.
pub(super) fn load_graph_chain(source: &TagSource, first: &str) -> Result<Vec<TagFile>, String> {
    let load = |path: &str| {
        load_referenced_tag_from_source(source, path, "model_animation_graph", b"jmad")
    };
    let mut seen = vec![first.to_owned()];
    let mut chain = vec![load(first).map_err(|error| error.to_string())?];
    loop {
        let parent = chain.last().and_then(|jmad| {
            let parent = Animation::new(jmad).ok()?.parent()?.to_owned();
            (!seen.contains(&parent)).then_some(parent)
        });
        let Some(jmad) = parent.and_then(|parent| {
            let jmad = load(&parent).ok()?;
            seen.push(parent);
            Some(jmad)
        }) else {
            return Ok(chain);
        };
        chain.push(jmad);
    }
}

/// Every animation of a graph chain as (graph, index): a graph's own, then
/// what it inherits. A name a nearer graph holds replaces the one above it.
pub(super) fn inherited_animation_order(animations: &[Animation<'_>]) -> Vec<(usize, usize)> {
    let mut names = std::collections::HashSet::new();
    let mut order = Vec::new();
    for (graph, animation) in animations.iter().enumerate() {
        let start = order.len();
        for group in animation.iter() {
            if group
                .name
                .as_ref()
                .is_none_or(|name| !names.contains(name.as_str()))
            {
                order.push((graph, group.index));
            }
        }
        names.extend(
            order[start..]
                .iter()
                .filter_map(|&(graph, index)| animations[graph].get(index)?.name.as_deref()),
        );
    }
    order
}

/// A Halo CE object's `model_animations`.
pub(super) fn load_object_animations(
    source: &TagSource,
    object: &TagFile,
) -> Result<TagFile, String> {
    let reference = object_reference(object, "animation graph")
        .ok_or("This object references no model_animations.")?;
    load_referenced_tag_from_source(source, &reference, "model_animations", b"antr")
        .map_err(|error| error.to_string())
}
