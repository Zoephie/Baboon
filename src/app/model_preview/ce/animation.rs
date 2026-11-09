//! Halo CE's animations: the object's `model_animations`, decoded as tool's
//! JMA export would, with the rest pose from its gbxmodel.

use super::super::super::TagSource;
use super::super::animation::DecodedAnimationPose;
use super::super::{load_referenced_tag_from_source, object_reference};
use blam_tags::animation::classic::{CeAnimation, CeAnimations};
use blam_tags::{JmaKind, NodeTransform, Skeleton, TagFile};
use std::collections::HashMap;

/// A Halo CE object's `model_animations`.
pub(in crate::app::model_preview) fn load_object_animations(
    source: &TagSource,
    object: &TagFile,
) -> Result<TagFile, String> {
    let reference = object_reference(object, "animation graph")
        .ok_or("This object references no model_animations.")?;
    load_referenced_tag_from_source(source, &reference, "model_animations", b"antr")
        .map_err(|error| error.to_string())
}

pub(in crate::app::model_preview) fn ce_jma_kind(animation: &CeAnimation<'_>) -> JmaKind {
    JmaKind::from_metadata(
        animation.animation_type.as_deref(),
        animation.frame_info_type.as_deref(),
        animation.world_relative,
    )
}

/// Halo CE's rest pose for an animation skeleton: the gbxmodel's node
/// defaults matched by name, in the tag's own rotation convention (the one the
/// animation decodes in). A `model_animations` carries no rest pose of its
/// own, so a node the gbxmodel lacks rests at identity.
pub(in crate::app::model_preview) fn ce_rest_pose(
    skeleton: &Skeleton,
    gbxmodel: Option<&TagFile>,
) -> Vec<NodeTransform> {
    let mut by_name: HashMap<String, NodeTransform> = HashMap::new();
    if let Some(nodes) = gbxmodel
        .and_then(|tag| tag.root().field("nodes"))
        .and_then(|field| field.as_block())
    {
        for node in nodes.iter() {
            let Some(name) = node.read_string("name") else {
                continue;
            };
            by_name.insert(
                name,
                NodeTransform {
                    translation: node.read_point3d("default translation"),
                    rotation: node.read_quat("default rotation"),
                    scale: 1.0,
                },
            );
        }
    }
    skeleton
        .nodes
        .iter()
        .map(|node| {
            by_name
                .get(&node.name)
                .copied()
                .unwrap_or(NodeTransform::IDENTITY)
        })
        .collect()
}

/// A clip with its rotations turned to run forward; see [`decode_ce_animation`].
pub(in crate::app::model_preview) fn ce_inverted_clip(
    animation: &CeAnimation<'_>,
) -> blam_tags::AnimationClip {
    let mut clip = animation.decode();
    for tracks in std::iter::once(&mut clip.static_tracks).chain(clip.animated_tracks.as_mut()) {
        for rotation in tracks.rotations.iter_mut().flatten() {
            *rotation = rotation.conjugate();
        }
    }
    clip
}

/// The idle of a clip's seat and weapon class, which an action plays over in
/// the preview. A clip with permutations is named for its first, `idle%0`.
pub(in crate::app::model_preview) fn ce_idle_beneath<'a, 'tag>(
    animations: &'a CeAnimations<'tag>,
    animation: &CeAnimation<'_>,
) -> Option<&'a CeAnimation<'tag>> {
    let name = animation.name.as_deref()?;
    let mut words = name.split(' ');
    let idle = format!("{} {} idle", words.next()?, words.next()?);
    let idle = [idle.clone(), format!("{idle}%0")];
    animations.iter().find(|clip| {
        clip.frame_count > 0 && clip.name.as_ref().is_some_and(|name| idle.contains(name))
    })
}

/// The Halo CE half of [`decode_held_animation`]: the extractor's skeleton
/// and the gbxmodel's rest pose, with overlays applied as the game applies
/// them.
pub(in crate::app::model_preview) fn decode_ce_animation(
    source: &TagSource,
    object: &TagFile,
    animation_index: usize,
) -> Result<DecodedAnimationPose, String> {
    let antr = load_object_animations(source, object)?;
    let animations = CeAnimations::new(&antr);
    let animation = animations
        .get(animation_index)
        .ok_or("The graph no longer lists this animation.")?;
    let gbxmodel = object_reference(object, "model").and_then(|reference| {
        load_referenced_tag_from_source(source, &reference, "gbxmodel", b"mod2").ok()
    });
    // The extractor's skeleton: a graph that lists no nodes of its own
    // animates its gbxmodel's, when their node list checksums agree.
    let skeleton =
        blam_tags::extract::animation::ce_skeleton(&animations, &antr, gbxmodel.as_ref());
    // CE rotations, rest pose and animation alike, are stored inverted
    // relative to the forward chaining the preview runs: the engine feeds
    // both into the same orientations (`model_get_node_orientations`, CE
    // Anniversary X360), and `RenderModel` conjugates the gbxmodel's. Over
    // the standing animations of six characters, frame 0 sits a median 20-36°
    // from the bind pose conjugated and 49-102° as stored; Halo 2 is the
    // mirror image, closer as stored. Both are inverted before composing: the
    // game applies an overlay as `delta × base` on the stored values
    // (`overlay_animation_apply`), which is `base × delta` once inverted.
    let mut rest = ce_rest_pose(&skeleton, gbxmodel.as_ref());
    for transform in &mut rest {
        transform.rotation = transform.rotation.conjugate();
    }
    let clip = ce_inverted_clip(animation);
    // `unit_preprocess_node_orientations` applies an action or overlay to the
    // pose the unit's state already holds: `replacement_animation_apply` and
    // `overlay_animation_apply` write only the nodes the clip animates.
    let beneath = ce_idle_beneath(&animations, animation)
        .map(|idle| ce_inverted_clip(idle).pose(&skeleton, Some(&rest)));
    let over = |compose: &dyn Fn(&[NodeTransform]) -> blam_tags::Pose| match &beneath {
        Some(idle) => {
            let frames = (0..clip.frame_count.max(1) as usize).map(|frame| {
                compose(&idle.frames[frame % idle.frames.len()])
                    .frames
                    .swap_remove(frame)
            });
            blam_tags::Pose {
                frames: frames.collect(),
            }
        }
        None => compose(&rest),
    };
    let mut pose = match ce_jma_kind(animation) {
        JmaKind::Jmo => over(&|base| clip.runtime_overlay_pose(&skeleton, base)),
        JmaKind::Jmr => over(&|base| clip.replacement_pose(&skeleton, base)),
        _ => clip.pose(&skeleton, Some(&rest)),
    };
    // Then the unit aims and looks straight ahead.
    let name = animation.name.as_deref().unwrap_or_default();
    for (index, frame) in super::weapons::resting_aim(&antr, name) {
        let Some(aim) = animations
            .get(index)
            .filter(|aim| frame < aim.frame_count as usize)
        else {
            continue;
        };
        let aim = ce_inverted_clip(aim);
        for row in &mut pose.frames {
            *row = aim
                .runtime_overlay_pose(&skeleton, row)
                .frames
                .swap_remove(frame);
        }
    }
    Ok(DecodedAnimationPose::new(
        &skeleton,
        &pose,
        animation.loop_frame_index as usize,
    ))
}

#[cfg(test)]
mod tests {

    use super::super::super::load_referenced_tag_from_source;
    use super::{decode_ce_animation, load_object_animations};
    use crate::app::model_preview::harness::kit::kit_source;
    use crate::core::game::GameId;
    use blam_tags::animation::classic::CeAnimations;

    /// With `BLAM_TEST_HCEEK`: an action's clip plays over the idle, so the
    /// nodes it leaves alone stand where the idle puts them, not at bind.
    #[test]
    fn a_ce_replacement_clip_leaves_its_unanimated_nodes_to_the_idle() {
        let Some(source) = kit_source(GameId::HaloCe) else {
            return;
        };
        let path = "characters\\cyborg\\cyborg";
        let object = load_referenced_tag_from_source(&source, path, "biped", b"bipd").unwrap();
        let antr = load_object_animations(&source, &object).unwrap();
        let animations = CeAnimations::new(&antr);
        let index = |name: &str| {
            animations
                .iter()
                .position(|clip| clip.name.as_deref() == Some(name))
                .unwrap()
        };
        let pose = |name| decode_ce_animation(&source, &object, index(name)).unwrap();
        let melee = pose("stand pistol b melee");
        let idle = pose("stand pistol idle");
        let flags = |name: &str| {
            animations
                .get(index(name))
                .unwrap()
                .decode()
                .node_flags
                .unwrap()
        };
        let (clip, aim, look) = (
            flags("stand pistol b melee"),
            flags("stand pistol aim-still"),
            flags("stand look"),
        );
        // The idle also takes the aiming screens, which an action shuts off.
        let left_alone: Vec<_> = (0..melee.skeleton_names.len())
            .filter(|node| {
                ![&clip, &aim, &look]
                    .iter()
                    .any(|flags| flags.animated_rotation.bit(*node))
            })
            .collect();
        assert!(!left_alone.is_empty());
        for node in left_alone {
            assert_eq!(
                melee.frames[8][node].rotation,
                idle.frames[8][node].rotation
            );
        }
    }
}
