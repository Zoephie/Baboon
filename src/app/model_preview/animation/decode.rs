//! Decoding an animation into poses: held layers over a base, and Halo CE's own clips.

use super::*;

pub(super) fn ce_jma_kind(animation: &CeAnimation<'_>) -> JmaKind {
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
pub(super) fn ce_rest_pose(skeleton: &Skeleton, gbxmodel: Option<&TagFile>) -> Vec<NodeTransform> {
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

/// A clip with its rotations turned to run forward; see [`decode_ce_animation`].
pub(super) fn ce_inverted_clip(animation: &CeAnimation<'_>) -> blam_tags::AnimationClip {
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
pub(super) fn ce_idle_beneath<'a, 'tag>(
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
pub(super) fn decode_ce_animation(
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
    for (index, frame) in super::super::ce::weapons::resting_aim(&antr, name) {
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
mod ce_replacement_tests {
    use super::super::*;
    use crate::app::model_preview::harness::kit::kit_source;
    use crate::core::game::GameId;

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
