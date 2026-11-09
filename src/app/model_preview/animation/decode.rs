//! Decoding an animation into poses: held layers over a base, and Halo CE's own clips.

use super::super::super::{TagEntry, TagSource};
use super::super::ce::animation::decode_ce_animation;
use super::super::{load_referenced_tag_from_source, preview_model_tag};
use super::graph::{inherited_animation_order, load_graph_chain};
use super::pose::DecodedAnimationPose;
use blam_tags::paths::tag_ref_path;
use blam_tags::{Animation, AnimationGraph, JmaKind, Skeleton, TagFile};

/// [`decode_held_animation`] with no weapon held, for the tests.
#[cfg(test)]
pub(super) fn decode_model_animation(
    source: &TagSource,
    entry: &TagEntry,
    animation_index: usize,
) -> Result<DecodedAnimationPose, String> {
    decode_held_animation(source, entry, animation_index, &[])
}

/// Lay `layer` over every frame of `pose`, leaving the arm it does not reach.
fn lay_over(
    pose: &mut blam_tags::Pose,
    animation: &Animation<'_>,
    skeleton: &Skeleton,
    left_arm: &[bool],
    layer: &super::super::weapons::PoseLayer,
) {
    use super::super::weapons::Arm;
    let Some(group) = animation.find(&layer.clip) else {
        return;
    };
    let Ok(clip) = group.decode() else {
        return;
    };
    let kind = blam_tags::extract::animation::jma_kind_for(group);
    for row in &mut pose.frames {
        let composed = match kind {
            JmaKind::Jmo => clip.runtime_overlay_pose(skeleton, row),
            JmaKind::Jmr => {
                clip.retargeted_replacement_pose(skeleton, row, &group.object_space_parents)
            }
            _ => clip.replacement_pose(skeleton, row),
        };
        let Some(composed) = composed.frames.into_iter().next() else {
            continue;
        };
        for (node, (old, new)) in row.iter_mut().zip(composed).enumerate() {
            let on_left = left_arm.get(node).copied().unwrap_or(false);
            let reached = match layer.arm {
                None => true,
                Some(Arm::Left) => on_left,
                Some(Arm::NotLeft) => !on_left,
            };
            if reached {
                *old = new;
            }
        }
    }
}

/// Worker half of the decode request: the extractor's exact composition
/// recipe (`write_group_jma`), minus the file write, with what a held weapon
/// lays over the clip.
pub(super) fn decode_held_animation(
    source: &TagSource,
    entry: &TagEntry,
    animation_index: usize,
    layers: &[super::super::weapons::PoseLayer],
) -> Result<DecodedAnimationPose, String> {
    let model =
        crate::core::source::read_entry(source, entry).map_err(|error| error.to_string())?;
    if blam_tags::game::Game::of(&model) == blam_tags::game::Game::Halo1 {
        return decode_ce_animation(source, &model, animation_index);
    }
    let model = preview_model_tag(source, &model)?.unwrap_or(model);
    let root = model.root();
    let jmad_ref =
        tag_ref_path(&root, "animation").ok_or("This model references no animation graph.")?;
    let chain = load_graph_chain(source, &jmad_ref)?;
    let animations = chain
        .iter()
        .map(|jmad| Animation::new(jmad).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let (graph, animation_index) = inherited_animation_order(&animations)
        .get(animation_index)
        .copied()
        .ok_or("The graph no longer lists this animation.")?;
    // An inherited clip decodes against the graph that holds it, and reaches
    // the model by node name.
    let (jmad, animation) = (&chain[graph], &animations[graph]);
    let skeleton = Skeleton::from_tag(jmad);
    let render_tag = tag_ref_path(&root, "render model").and_then(|reference| {
        load_referenced_tag_from_source(source, &reference, "render_model", b"mode").ok()
    });
    let object_space =
        blam_tags::extract::animation::additional_node_data_is_object_space(animation);
    let defaults = blam_tags::extract::animation::build_defaults(
        &skeleton,
        jmad,
        render_tag.as_ref(),
        object_space,
    );
    let group = animation
        .get(animation_index)
        .ok_or("The graph no longer lists this animation.")?;
    if group.blob.is_empty() {
        return Err(
            "This animation has no payload — a composite/runtime blend, or data the build \
             never kept."
                .to_owned(),
        );
    }
    let clip = group.decode().map_err(|error| error.to_string())?;

    let kind = blam_tags::extract::animation::jma_kind_for(group);
    let base = match kind {
        JmaKind::Jmo | JmaKind::Jmr => {
            let graph = AnimationGraph::from_tag(jmad);
            animation
                .overlay_base_pose(&graph, group, &skeleton, &defaults)
                .unwrap_or_else(|| defaults.clone())
        }
        _ => defaults.clone(),
    };
    // Halo 2, Halo 3 and ODST compose as their solver does. Graphs with
    // Reach's data-driven blocks (Reach, Halo 4, H2A, Campaign Evolved) keep
    // the export composition until their solver's rules are checked.
    let solver_rules = jmad
        .root()
        .field_path("definitions/NEW blend screens")
        .is_none();
    let mut pose = match kind {
        JmaKind::Jmo if solver_rules => clip.runtime_overlay_pose(&skeleton, &base),
        JmaKind::Jmr if solver_rules => {
            clip.retargeted_replacement_pose(&skeleton, &base, &group.object_space_parents)
        }
        JmaKind::Jmo => {
            let (mut reference, mut body) = clip.overlay_pose(&skeleton, &base);
            body.apply_object_space_corrections(
                &mut reference,
                &skeleton,
                &base,
                &group.object_space_parents,
            );
            body
        }
        JmaKind::Jmr => {
            let mut body = clip.replacement_pose(&skeleton, &base);
            let mut reference = base.clone();
            body.apply_object_space_corrections(
                &mut reference,
                &skeleton,
                &base,
                &group.object_space_parents,
            );
            body
        }
        _ => clip.pose(&skeleton, Some(&defaults)),
    };
    // Reach's data-driven board lays the state's blend screens over an
    // action: at rest, aiming and looking ahead.
    if !matches!(kind, JmaKind::Jmo | JmaKind::Jmr) {
        super::super::animation::blend_screens::lay_blend_screens(
            jmad,
            animation,
            &skeleton,
            animation_index,
            &mut pose,
        );
    }
    if !layers.is_empty() {
        let left_arm = super::super::weapons::left_arm_nodes(jmad);
        for layer in layers {
            lay_over(&mut pose, animation, &skeleton, &left_arm, layer);
        }
    }

    let loop_frame = usize::try_from(group.loop_frame_index).unwrap_or(0);
    let graphs: Vec<&TagFile> = chain[graph..].iter().collect();
    Ok(DecodedAnimationPose {
        reach_ik: super::super::animation::ik::reach::read_reach_ik_tracks(&graphs, group),
        h4_ik_set: graphs.first().map_or_else(Vec::new, |graph| {
            super::super::animation::ik::h4::read_h4_ik_set(graph, group.index)
        }),
        ..DecodedAnimationPose::new(&skeleton, &pose, loop_frame)
    })
}
