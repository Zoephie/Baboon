//! The animation playback math: skinning matrices, frame blending, and the
//! skeleton-to-preview mapping. The one property everything hangs on: a pose
//! identical to the bind pose must skin every vertex to exactly where the tag
//! put it — any drift there deforms the model just by pressing play.

use super::super::super::{TagEntry, TagEntryLocation, TagNameIndex, TagSource, file_entry_key};
use super::super::ce::animation::{
    ce_idle_beneath, ce_inverted_clip, ce_jma_kind, ce_rest_pose, load_object_animations,
};
use super::super::{
    MAX_PREVIEW_BONES, ModelPreviewData, ModelPreviewState, ModelVariantPreview,
    RenderModelPreview, RenderModelPreviewNode, build_render_preview, compute_variant_selection,
    load_referenced_tag_from_source, model_preview_data, object_reference, preview_skeleton_nodes,
};
use super::decode::decode_model_animation;
use super::graph::list_model_animations;
use super::*;
use crate::app::model_preview::harness::kit::kit_root;
use crate::core::game::GameId;
use blam_tags::animation::classic::CeAnimations;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
use blam_tags::render_model::Node;
use blam_tags::{Animation, AnimationGraph, JmaKind, Skeleton};
use std::collections::HashMap;

fn preview_node(name: &str, parent: i16) -> RenderModelPreviewNode {
    RenderModelPreviewNode {
        name: name.to_owned(),
        parent,
        bind_rotation: [0.0, 0.0, 0.0, 1.0],
        bind_translation: [0.0; 3],
        inverse_bind: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    }
}

/// A muted node takes its whole subtree; its parent and siblings play on.
#[test]
fn a_muted_node_holds_its_subtree() {
    let nodes = [
        preview_node("root", -1),
        preview_node("b_bay_door", 0),
        preview_node("b_bay_door_hinge", 1),
        preview_node("b_hinge_tip", 2),
        preview_node("b_l_prop", 0),
    ];
    let muted = muted_nodes(&nodes, &["b_bay_door".to_owned()]);
    assert_eq!(muted, [false, true, true, true, false]);
    assert_eq!(muted_nodes(&nodes, &[]), [false; 5]);
}

/// With `BLAM_TEST_H4EK`: the Pelican's `m70_gunship_opaque_cockpit` holds
/// its cockpit, bay door, caches and props.
#[test]
fn halo_4_pelican_variant_reads_its_muted_nodes() {
    let Some(root) = kit_root(GameId::Halo4) else {
        return;
    };
    let path = std::path::Path::new(&root)
        .join("objects/vehicles/human/storm_pelican/storm_pelican.model");
    let variants =
        super::super::load::read_model_variants(&blam_tags::TagFile::read(path).unwrap());
    let cockpit = variants
        .iter()
        .find(|variant| variant.name == "m70_gunship_opaque_cockpit")
        .unwrap();
    assert_eq!(cockpit.muted_nodes.len(), 10);
    assert!(cockpit.muted_nodes.iter().any(|name| name == "b_bay_door"));
    assert!(
        variants
            .iter()
            .find(|variant| variant.name == "default")
            .unwrap()
            .muted_nodes
            .is_empty()
    );
}

fn raw_node(name: &str, parent: i16, translation: RealPoint3d, rotation: RealQuaternion) -> Node {
    Node {
        name: name.to_owned(),
        parent_node: parent,
        first_child_node: -1,
        next_sibling_node: -1,
        default_translation: translation,
        default_rotation: rotation,
        inverse_forward: RealVector3d::ZERO,
        inverse_left: RealVector3d::ZERO,
        inverse_up: RealVector3d::ZERO,
        inverse_position: RealPoint3d::ZERO,
        inverse_scale: 0.0,
        distance_from_parent: 0.0,
    }
}

fn test_nodes() -> Vec<RenderModelPreviewNode> {
    let nodes = vec![
        raw_node(
            "pelvis",
            -1,
            RealPoint3d {
                x: 0.1,
                y: 0.2,
                z: 0.9,
            },
            RealQuaternion {
                i: 0.0,
                j: 0.0,
                k: 0.3826834,
                w: 0.9238795,
            },
        ),
        raw_node(
            "spine",
            0,
            RealPoint3d {
                x: 0.0,
                y: 0.0,
                z: 0.25,
            },
            RealQuaternion {
                i: 0.258819,
                j: 0.0,
                k: 0.0,
                w: 0.9659258,
            },
        ),
    ];
    preview_skeleton_nodes(&nodes)
}

fn bind_pose_frame(nodes: &[RenderModelPreviewNode]) -> Vec<PreviewNodeTransform> {
    nodes
        .iter()
        .map(|node| PreviewNodeTransform {
            rotation: node.bind_rotation,
            translation: node.bind_translation,
            scale: 1.0,
        })
        .collect()
}

fn preview_with_nodes(nodes: Vec<RenderModelPreviewNode>) -> ModelPreviewData {
    let preview = RenderModelPreview {
        nodes,
        ..Default::default()
    };
    model_preview_data("test".to_owned(), "test".to_owned(), preview, Vec::new())
}

/// Playing the bind pose must be a no-op: every skin matrix is identity.
#[test]
fn a_bind_pose_animation_skins_every_node_to_identity() {
    let nodes = test_nodes();
    let frame = bind_pose_frame(&nodes);
    let data = preview_with_nodes(nodes);
    let mut state = ModelPreviewState::default();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(
        0,
        vec![frame],
    )));

    let rows = animation_skinning_rows(&data, &state).expect("skinning rows");
    assert_eq!(rows.len(), 2 * 3);
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    for (node, chunk) in rows.chunks_exact(3).enumerate() {
        for (row, expected) in chunk.iter().zip(identity) {
            for (value, want) in row.iter().zip(expected) {
                assert!(
                    (value - want).abs() < 1e-4,
                    "node {node}: bind-pose skin drifted: {chunk:?}"
                );
            }
        }
    }
}

/// Translating the root in the pose moves the skin transform by exactly that
/// delta — the world × inverse-bind composition points the right way round.
#[test]
fn a_translated_root_moves_the_skin_by_the_delta() {
    let nodes = test_nodes();
    let mut frame = bind_pose_frame(&nodes);
    frame[0].translation[0] += 0.5;
    let data = preview_with_nodes(nodes);
    let mut state = ModelPreviewState::default();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(
        0,
        vec![frame],
    )));

    let rows = animation_skinning_rows(&data, &state).expect("skinning rows");
    // Root: identity rotation part relative to bind, translation +0.5 in x.
    assert!((rows[0][3] - 0.5).abs() < 1e-4, "root x: {:?}", rows[0]);
    assert!(rows[1][3].abs() < 1e-4);
    assert!(rows[2][3].abs() < 1e-4);
    // The child inherits the same rigid shift, nothing else.
    assert!((rows[3][3] - 0.5).abs() < 1e-4, "child x: {:?}", rows[3]);
}

#[test]
fn armature_positions_follow_the_current_animation_pose() {
    let nodes = test_nodes();
    let bind_positions = armature_node_positions(
        &preview_with_nodes(nodes.clone()),
        &ModelPreviewState::default(),
    );
    let mut frame = bind_pose_frame(&nodes);
    frame[0].translation[0] += 0.5;
    let data = preview_with_nodes(nodes);
    let mut state = ModelPreviewState::default();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(
        0,
        vec![frame],
    )));
    let animated = armature_node_positions(&data, &state);

    assert_eq!(animated.len(), 2);
    assert!((animated[0][0] - bind_positions[0][0] - 0.5).abs() < 1e-4);
    assert!((animated[1][0] - bind_positions[1][0] - 0.5).abs() < 1e-4);
}

/// A muted node's joint holds its bind pose under its moving parent, as
/// the skinned mesh does, so the armature stays on the mesh.
#[test]
fn armature_holds_a_muted_node_where_the_skin_does() {
    let nodes = test_nodes();
    let bind = armature_node_positions(
        &preview_with_nodes(nodes.clone()),
        &ModelPreviewState::default(),
    );
    let mut frame = bind_pose_frame(&nodes);
    frame[0].translation[0] += 0.5;
    frame[1].translation[2] += 0.75;
    let mut data = preview_with_nodes(nodes);
    data.variants = vec![ModelVariantPreview {
        name: "muted".to_owned(),
        regions: Default::default(),
        hidden_regions: Default::default(),
        muted_nodes: vec!["spine".to_owned()],
    }];
    let mut state = ModelPreviewState::default();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(
        0,
        vec![frame],
    )));
    let unmuted = armature_node_positions(&data, &state);
    state.selected_variant = Some(0);
    let muted = armature_node_positions(&data, &state);

    assert!(
        (unmuted[1][2] - bind[1][2]).abs() > 0.1,
        "unmuted, the spine plays its own move"
    );
    assert!(
        (muted[1][0] - bind[1][0] - 0.5).abs() < 1e-4,
        "it follows its parent"
    );
    assert!(
        (muted[1][2] - bind[1][2]).abs() < 1e-4,
        "and holds its own bind pose"
    );
    // The skin's translation is world × inverse bind, so the joint is the
    // skin applied to the bind position.
    let rows = animation_skinning_rows(&data, &state).expect("skinning rows");
    let skinned: [f32; 3] = std::array::from_fn(|axis| {
        let row = rows[3 + axis];
        row[0] * bind[1][0] + row[1] * bind[1][1] + row[2] * bind[1][2] + row[3]
    });
    for axis in 0..3 {
        assert!(
            (skinned[axis] - muted[1][axis]).abs() < 1e-4,
            "{skinned:?} {:?}",
            muted[1]
        );
    }
}

/// Halfway between two frames, the pose is a blend of them; with
/// interpolation off it is the first frame exactly, which is how an
/// overlay animation is read frame by frame.
/// The game plays a looping clip to its last frame, holds it, then
/// continues from the loop frame (0 restarts it); the preview blended the
/// last frame into frame 0 and always restarted at 0.
#[test]
fn a_looping_clip_holds_its_last_frame_then_continues_from_the_loop_frame() {
    let nodes = test_nodes();
    let data = preview_with_nodes(nodes.clone());
    let bind = armature_node_positions(&data, &ModelPreviewState::default());
    let frames: Vec<_> = (0..3)
        .map(|frame| {
            let mut pose = bind_pose_frame(&nodes);
            pose[0].translation[0] += frame as f32;
            pose
        })
        .collect();
    let mut state = ModelPreviewState::default();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose {
        loop_frame: 1,
        ..PreviewAnimationPose::new(0, frames)
    }));
    let offset_at = |state: &mut ModelPreviewState, frames: f32| {
        state.animation.time = frames / ANIMATION_FRAME_RATE;
        armature_node_positions(&data, state)[0][0] - bind[0][0]
    };

    assert!(
        (offset_at(&mut state, 2.5) - 2.0).abs() < 1e-4,
        "the last frame holds"
    );
    assert!(
        (offset_at(&mut state, 3.25) - 1.25).abs() < 1e-4,
        "continues from the loop frame"
    );
    assert!(
        (offset_at(&mut state, 5.25) - 1.25).abs() < 1e-4,
        "loops between frame 1 and the end"
    );

    let restarting = state.animation.pose.as_ref().unwrap().frames.clone();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(
        0, restarting,
    )));
    assert!(
        (offset_at(&mut state, 3.25) - 0.25).abs() < 1e-4,
        "a loop frame of 0 restarts"
    );
}

#[test]
fn without_interpolation_the_pose_holds_each_frame() {
    let nodes = test_nodes();
    let data = preview_with_nodes(nodes.clone());
    let bind = armature_node_positions(&data, &ModelPreviewState::default());
    let first = bind_pose_frame(&nodes);
    let mut second = bind_pose_frame(&nodes);
    second[0].translation[0] += 1.0;
    let mut state = ModelPreviewState::default();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(
        0,
        vec![first, second],
    )));
    state.animation.looped = false;
    state.animation.time = 0.5 / ANIMATION_FRAME_RATE;

    let blended = armature_node_positions(&data, &state);
    assert!(
        (blended[0][0] - bind[0][0] - 0.5).abs() < 1e-4,
        "halfway between the frames"
    );

    state.animation.interpolate = false;
    let held = armature_node_positions(&data, &state);
    assert!(
        (held[0][0] - bind[0][0]).abs() < 1e-4,
        "still on the first frame"
    );
    assert_eq!(playback_frame_position(&state.animation, 2), 0.0);

    state.animation.time = 1.0 / ANIMATION_FRAME_RATE;
    let next = armature_node_positions(&data, &state);
    assert!(
        (next[0][0] - bind[0][0] - 1.0).abs() < 1e-4,
        "on the second frame"
    );
}

#[test]
fn stop_restores_bind_pose_without_unloading_the_animation() {
    let nodes = test_nodes();
    let bind_positions = armature_node_positions(
        &preview_with_nodes(nodes.clone()),
        &ModelPreviewState::default(),
    );
    let mut frame = bind_pose_frame(&nodes);
    frame[0].translation[0] += 0.5;
    let data = preview_with_nodes(nodes);
    let mut state = ModelPreviewState::default();
    state.animation.selected = Some(2);
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(
        2,
        vec![frame],
    )));
    state.animation.stopped = true;

    assert!(animation_skinning_rows(&data, &state).is_none());
    assert_eq!(armature_node_positions(&data, &state), bind_positions);
    assert_eq!(state.animation.selected, Some(2));
    assert!(state.animation.pose.is_some());
}

/// A node the animation does not cover falls back to its own bind pose —
/// identity skin — rather than collapsing to the origin.
#[test]
fn a_node_missing_from_the_pose_stays_at_bind() {
    let nodes = test_nodes();
    let frame = vec![bind_pose_frame(&nodes)[0]]; // only the root
    let data = preview_with_nodes(nodes);
    let mut state = ModelPreviewState::default();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(
        0,
        vec![frame],
    )));

    let rows = animation_skinning_rows(&data, &state).expect("skinning rows");
    assert!(
        (rows[3][0] - 1.0).abs() < 1e-4,
        "child rotation row: {:?}",
        &rows[3]
    );
    assert!(rows[3][3].abs() < 1e-4, "child translation: {:?}", &rows[3]);
}

/// Point `BABOON_REACH_KIT` at a Reach kit's `tags` folder to prove big
/// Reach skeletons play: mule (99 nodes) and halsey (163) both blew the old
/// 96-bone budget and silently showed no animation strip at all. Absent, this
/// self-skips.
#[test]
fn a_reach_skeleton_past_the_old_bone_budget_lists_and_decodes() {
    let Some(tags_root) = std::env::var_os("BABOON_REACH_KIT").map(std::path::PathBuf::from) else {
        eprintln!("skipping: set BABOON_REACH_KIT to a Reach editing kit's tags folder");
        return;
    };
    let source = TagSource::LooseFolder {
        root: tags_root.clone(),
        game: Some(GameId::HaloReach),
        definitions_root: std::path::PathBuf::new(),
    };
    let entry_for = |rel: &str| TagEntry {
        key: file_entry_key(&tags_root.join(rel)),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: TagEntryLocation::LooseFile(tags_root.join(rel)),
    };

    for rel in [
        "objects/characters/mule/mule.model",
        "objects/characters/halsey/halsey.model",
    ] {
        if !tags_root.join(rel).is_file() {
            eprintln!("skipping {rel}: not in this kit");
            continue;
        }
        let entry = entry_for(rel);
        let model = crate::core::source::read_entry(&source, &entry).expect("model reads");
        let (_, render_rel) = model
            .root()
            .read_tag_ref_with_group("render model")
            .expect("render model ref");
        let preview =
            load_referenced_tag_from_source(&source, &render_rel, "render_model", b"mode")
                .map_err(|error| error.to_string())
                .and_then(|tag| build_render_preview(&tag))
                .expect("render preview");
        assert!(
            !preview.nodes.is_empty() && preview.nodes.len() <= MAX_PREVIEW_BONES,
            "{rel}: {} nodes outside the bone budget of {MAX_PREVIEW_BONES}",
            preview.nodes.len()
        );
        // Reach meshes store palette-LOCAL blend indices behind a per-mesh
        // node map; blam-tags must hand them out remapped to global. Every
        // weighted influence lands inside the skeleton, and — the regression
        // signal — some influence indexes past any single palette (these
        // skeletons need several), which local indices never could.
        let mut max_weighted = 0usize;
        for vertex in &preview.vertices {
            for (index, weight) in vertex.node_indices.iter().zip(vertex.node_weights) {
                if weight > 0.0 {
                    let index = (*index + 0.5) as usize;
                    assert!(
                        index < preview.nodes.len(),
                        "{rel}: influence on node {index} outside the {}-node skeleton",
                        preview.nodes.len()
                    );
                    max_weighted = max_weighted.max(index);
                }
            }
        }
        assert!(
            max_weighted > 64,
            "{rel}: max weighted node {max_weighted} looks palette-local, not global"
        );
        let list = list_model_animations(&source, &entry).expect("animation list");
        let playable = list
            .iter()
            .position(|entry| entry.playable && entry.frame_count > 1)
            .unwrap_or_else(|| panic!("{rel}: no playable animation listed"));
        let decoded = decode_model_animation(&source, &entry, playable).expect("decode");
        assert!(!decoded.frames.is_empty(), "{rel}: no frames decoded");
        eprintln!(
            "{rel}: {} nodes, {} animations, '{}' decoded to {} frames",
            preview.nodes.len(),
            list.len(),
            list[playable].name,
            decoded.frames.len()
        );
    }
}

/// Point `BABOON_MODEL_KIT` at an H3-family kit's `tags` folder to decode a
/// real animation end to end; absent, this self-skips.
/// The game retargets a Halo 3 replacement's object-space entries onto
/// the named node's animated children only, so every bone the
/// animation doesn't animate stays on the base pose. The export
/// reconstruction re-oriented the pelvis entry's whole subtree, legs
/// included.
#[test]
fn a_halo3_replacement_leaves_unanimated_bones_on_the_base() {
    let Some(tags_root) = std::env::var_os("BABOON_MODEL_KIT").map(std::path::PathBuf::from) else {
        eprintln!("skipping: set BABOON_MODEL_KIT to an editing kit's tags folder");
        return;
    };
    let folder = tags_root.join("objects/characters/masterchief");
    let model_path = folder.join("masterchief.model");
    if !model_path.is_file() {
        eprintln!(
            "skipping: no masterchief.model under {}",
            tags_root.display()
        );
        return;
    }
    let name = "combat:pistol:hp:reload_1";
    let jmad = blam_tags::TagFile::read(folder.join("masterchief.model_animation_graph")).unwrap();
    let render = blam_tags::TagFile::read(folder.join("masterchief.render_model")).ok();
    let animation = Animation::new(&jmad).unwrap();
    let skeleton = Skeleton::from_tag(&jmad);
    let object_space =
        blam_tags::extract::animation::additional_node_data_is_object_space(&animation);
    let defaults = blam_tags::extract::animation::build_defaults(
        &skeleton,
        &jmad,
        render.as_ref(),
        object_space,
    );
    let group = (0..animation.len())
        .filter_map(|index| animation.get(index))
        .find(|group| group.name.as_deref() == Some(name))
        .expect("the chief's graph has no pistol reload");
    assert!(
        !group.object_space_parents.is_empty(),
        "{name} carries no object-space entries"
    );
    let base = animation
        .overlay_base_pose(
            &AnimationGraph::from_tag(&jmad),
            group,
            &skeleton,
            &defaults,
        )
        .unwrap_or_else(|| defaults.clone());
    let flags = group.decode().unwrap().node_flags.expect("node flags");

    let source = TagSource::LooseFolder {
        root: tags_root,
        game: Some(GameId::Halo3),
        definitions_root: std::path::PathBuf::new(),
    };
    let entry = TagEntry {
        key: file_entry_key(&model_path),
        display_path: "objects/characters/masterchief/masterchief.model".to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: TagEntryLocation::LooseFile(model_path),
    };
    let decoded = decode_model_animation(&source, &entry, group.index).expect("decode");

    let mut unanimated = 0;
    for (node, rest) in base.iter().enumerate() {
        if flags.animated_rotation.bit(node) {
            continue;
        }
        unanimated += 1;
        let want = PreviewNodeTransform::from_node_transform(rest).rotation;
        for (index, frame) in decoded.frames.iter().enumerate() {
            let got = frame[node].rotation;
            let dot: f32 = got.iter().zip(want).map(|(a, b)| a * b).sum();
            assert!(
                dot.abs() > 0.9999,
                "{} moved off the base at frame {index}: {got:?} vs {want:?}",
                decoded.skeleton_names[node]
            );
        }
    }
    assert!(unanimated > 0, "{name} animates every bone");
}

#[test]
fn a_real_kits_animation_decodes_into_frames() {
    let Some(tags_root) = std::env::var_os("BABOON_MODEL_KIT").map(std::path::PathBuf::from) else {
        eprintln!("skipping: set BABOON_MODEL_KIT to an editing kit's tags folder");
        return;
    };
    let model_path = tags_root.join("objects/characters/masterchief/masterchief.model");
    if !model_path.is_file() {
        eprintln!(
            "skipping: no masterchief.model under {}",
            tags_root.display()
        );
        return;
    }
    let source = TagSource::LooseFolder {
        root: tags_root,
        game: Some(GameId::Halo3),
        definitions_root: std::path::PathBuf::new(),
    };
    let entry = TagEntry {
        key: file_entry_key(&model_path),
        display_path: "objects/characters/masterchief/masterchief.model".to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: TagEntryLocation::LooseFile(model_path),
    };

    let list = list_model_animations(&source, &entry).expect("animation list");
    assert!(!list.is_empty(), "the chief's graph lists no animations");
    let playable = list
        .iter()
        .position(|entry| entry.playable && entry.frame_count > 1)
        .expect("no playable animation in the graph");
    eprintln!(
        "{} animations; decoding '{}' ({} frames)",
        list.len(),
        list[playable].name,
        list[playable].frame_count
    );

    let decoded = decode_model_animation(&source, &entry, playable).expect("decode");
    assert!(!decoded.frames.is_empty(), "no frames decoded");
    assert!(
        decoded.skeleton_names.iter().any(|name| name == "pelvis"),
        "skeleton names look wrong: {:?}",
        &decoded.skeleton_names[..decoded.skeleton_names.len().min(5)]
    );
    let frame = &decoded.frames[0];
    assert_eq!(frame.len(), decoded.skeleton_names.len());
    assert!(
        frame.iter().all(|transform| {
            transform.rotation.iter().all(|value| value.is_finite())
                && transform.translation.iter().all(|value| value.is_finite())
        }),
        "non-finite transforms in frame 0"
    );
}

/// Angle between two `[i, j, k, w]` rotations, in degrees.
fn rotation_angle(a: [f32; 4], b: [f32; 4]) -> f32 {
    let dot = a
        .iter()
        .zip(b)
        .map(|(a, b)| a * b)
        .sum::<f32>()
        .abs()
        .min(1.0);
    2.0 * dot.acos().to_degrees()
}

/// Drive a real kit's model through the panel's own path — the preview load,
/// the animation list, the decode — and check what playback consumes: the
/// geometry is skinned across the skeleton, every decoded node lands on a
/// preview node, and an idle's first frame sits closer to the bind pose in the
/// rotation convention the decode picked than in the opposite one. That last
/// check is the one a wrong conjugation fails.
fn plays_a_classic_idle(source: TagSource, entry: TagEntry, game: &str, idle: &str) {
    let tag = crate::core::source::read_entry(&source, &entry).expect("tag reads");
    let names = TagNameIndex::load_game(
        crate::core::test_kits::definitions(),
        GameId::from_id(game).unwrap(),
    )
    .expect("tag names");
    let data = crate::app::model_preview::load::load_model_preview(
        &tag,
        &entry,
        &names,
        Some(&source),
        &Default::default(),
    )
    .expect("preview loads");
    let nodes = &data.preview.nodes;

    // Before the engine read skinning for these games, every vertex was on node 0.
    let skinned: std::collections::BTreeSet<usize> = data
        .preview
        .vertices
        .iter()
        .flat_map(|vertex| {
            vertex
                .node_indices
                .iter()
                .zip(vertex.node_weights)
                .filter(|(_, weight)| *weight > 0.0)
                .map(|(index, _)| (*index + 0.5) as usize)
        })
        .collect();
    assert!(
        skinned.len() > 10 && skinned.iter().all(|&node| node < nodes.len()),
        "{}: weighted nodes {skinned:?} of {}",
        entry.display_path,
        nodes.len()
    );

    let list = list_model_animations(&source, &entry).expect("animation list");
    let index = list
        .iter()
        .position(|animation| animation.name == idle)
        .unwrap_or_else(|| {
            panic!(
                "{}: no '{idle}' in {} animations",
                entry.display_path,
                list.len()
            )
        });
    assert!(list[index].playable);
    let decoded = decode_model_animation(&source, &entry, index).expect("decode");
    assert_eq!(decoded.frames.len(), list[index].frame_count as usize);

    let by_name: HashMap<&str, &RenderModelPreviewNode> = nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let (mut chosen, mut opposite) = (Vec::new(), Vec::new());
    for (name, transform) in decoded.skeleton_names.iter().zip(&decoded.frames[0]) {
        let node = by_name.get(name.as_str()).unwrap_or_else(|| {
            panic!(
                "{}: animated node '{name}' is not in the preview",
                entry.display_path
            )
        });
        let [i, j, k, w] = transform.rotation;
        chosen.push(rotation_angle(transform.rotation, node.bind_rotation));
        opposite.push(rotation_angle([-i, -j, -k, w], node.bind_rotation));
    }
    let median = |mut values: Vec<f32>| {
        values.sort_by(f32::total_cmp);
        values[values.len() / 2]
    };
    let (chosen, opposite) = (median(chosen), median(opposite));
    eprintln!(
        "{}: '{idle}' frame 0 median {chosen:.1}° from bind, {opposite:.1}° in the opposite convention",
        entry.display_path
    );
    assert!(
        chosen < opposite,
        "{}: '{idle}' frame 0 is nearer the bind pose with its rotations conjugated \
         ({opposite:.1}° vs {chosen:.1}°)",
        entry.display_path
    );
}

/// A Halo CE object stands in for the `.model`: it names the gbxmodel and the
/// `model_animations` both. `BLAM_TEST_HCEEK` names the kit's `tags` folder.
#[test]
fn a_halo_ce_biped_plays_its_idle() {
    let Some(tags) = kit_root(GameId::HaloCe) else {
        return;
    };
    let rel = "characters/cyborg/cyborg.biped";
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(GameId::HaloCe),
        definitions_root: crate::core::test_kits::definitions().to_path_buf(),
    };
    let entry = TagEntry {
        key: file_entry_key(&tags.join(rel)),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"bipd"),
        group_name: Some("biped".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    };
    plays_a_classic_idle(source, entry, "haloce_mcc", "stand rifle idle");
}

/// A Halo CE graph that lists no nodes of its own animates its gbxmodel's,
/// as the JMA extractor does. Preview built the skeleton from the graph
/// alone, so such a vehicle's animations decoded onto no nodes and the
/// preview stayed in its bind pose. `BLAM_TEST_HCEEK` names the kit's
/// `tags` folder.
#[test]
fn a_halo_ce_vehicle_animates_its_gbxmodels_nodes() {
    let Some(tags) = kit_root(GameId::HaloCe) else {
        return;
    };
    let rel = "vehicles/warthog/warthog.vehicle";
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(GameId::HaloCe),
        definitions_root: crate::core::test_kits::definitions().to_path_buf(),
    };
    let entry = TagEntry {
        key: file_entry_key(&tags.join(rel)),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"vehi"),
        group_name: Some("vehicle".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    };
    let list = list_model_animations(&source, &entry).expect("animation list");
    let index = list
        .iter()
        .position(|animation| animation.playable && animation.frame_count > 1)
        .expect("a playable animation");
    let decoded = decode_model_animation(&source, &entry, index).expect("decode");
    assert!(
        !decoded.skeleton_names.is_empty(),
        "'{}' decoded onto no nodes",
        list[index].name
    );
}

/// The game applies a Halo CE overlay as `delta × base` on the stored
/// rotations (`overlay_animation_apply`); the preview, which inverts
/// every CE rotation, composed `base × delta` and inverted the product,
/// reversing the order. `BLAM_TEST_HCEEK` names the kit's `tags` folder.
#[test]
fn a_halo_ce_overlay_is_applied_in_the_games_order() {
    let Some(tags) = kit_root(GameId::HaloCe) else {
        return;
    };
    let rel = "characters/cyborg/cyborg.biped";
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(GameId::HaloCe),
        definitions_root: crate::core::test_kits::definitions().to_path_buf(),
    };
    let entry = TagEntry {
        key: file_entry_key(&tags.join(rel)),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"bipd"),
        group_name: Some("biped".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    };
    let name = "stand rifle aim-still";
    let list = list_model_animations(&source, &entry).expect("animation list");
    let index = list
        .iter()
        .position(|animation| animation.name == name)
        .expect("the cyborg has no aim-still");
    let decoded = decode_model_animation(&source, &entry, index).expect("decode");

    let object = crate::core::source::read_entry(&source, &entry).unwrap();
    let antr = load_object_animations(&source, &object).unwrap();
    let animations = CeAnimations::new(&antr);
    let animation = animations.get(index).unwrap();
    assert_eq!(
        ce_jma_kind(animation),
        JmaKind::Jmo,
        "{name} is not an overlay"
    );
    let gbxmodel = object_reference(&object, "model").and_then(|reference| {
        load_referenced_tag_from_source(&source, &reference, "gbxmodel", b"mod2").ok()
    });
    let skeleton =
        blam_tags::extract::animation::ce_skeleton(&animations, &antr, gbxmodel.as_ref());
    let mut rest = ce_rest_pose(&skeleton, gbxmodel.as_ref());
    rest.iter_mut()
        .for_each(|node| node.rotation = node.rotation.conjugate());
    let idle = ce_idle_beneath(&animations, animation).expect("the cyborg has no idle");
    let idle = ce_inverted_clip(idle).pose(&skeleton, Some(&rest));
    let clip = animation.decode();
    let flags = clip.node_flags.as_ref().expect("node flags");
    let tracks = clip.animated_tracks.as_ref().expect("animated tracks");

    let (mut track, mut orders_differ) = (0, false);
    for node in 0..skeleton.len() {
        if !flags.animated_rotation.bit(node) {
            continue;
        }
        for (frame, delta) in tracks.rotations[track].iter().enumerate() {
            // The game's order, on the values as the tag stores them.
            let base = idle.frames[frame % idle.frames.len()][node]
                .rotation
                .conjugate();
            let want = (*delta * base).conjugate().normalized();
            let reversed = (base * *delta).conjugate().normalized();
            orders_differ |= want.dot(reversed).abs() < 0.999;
            let got = decoded.frames[frame][node].rotation;
            let dot: f32 = got.iter().zip(want.to_array()).map(|(a, b)| a * b).sum();
            assert!(
                dot.abs() > 0.9999,
                "{} frame {frame}: {got:?} vs {want:?}",
                decoded.skeleton_names[node]
            );
        }
        track += 1;
    }
    assert!(track > 0, "{name} animates no rotation");
    assert!(orders_differ, "{name} can't tell the two orders apart");
}

/// `BLAM_TEST_H2EK` names a Halo 2 kit's `tags` folder.
#[test]
fn a_halo_2_model_plays_its_idle() {
    let Some(tags) = kit_root(GameId::Halo2) else {
        return;
    };
    let rel = "objects/characters/masterchief/masterchief.model";
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(GameId::Halo2),
        definitions_root: crate::core::test_kits::definitions().to_path_buf(),
    };
    let entry = TagEntry {
        key: file_entry_key(&tags.join(rel)),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    };
    plays_a_classic_idle(source, entry, "halo2_mcc", "combat:rifle:idle");
}

/// `BLAM_TEST_H2EK`: Miranda's own graph holds four animations, and
/// everything else she plays comes from the marine's.
#[test]
fn a_model_lists_and_plays_what_its_graph_inherits() {
    let Some(tags) = kit_root(GameId::Halo2) else {
        return;
    };
    let rel = "objects/characters/miranda/miranda.model";
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(GameId::Halo2),
        definitions_root: crate::core::test_kits::definitions().to_path_buf(),
    };
    let entry = TagEntry {
        key: file_entry_key(&tags.join(rel)),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    };
    let list = list_model_animations(&source, &entry).expect("animation list");
    assert_eq!(list.len(), 642);
    let mut names: Vec<_> = list.iter().map(|entry| &entry.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(
        names.len(),
        list.len(),
        "a child's animation should replace its parent's"
    );

    let inherited = list
        .iter()
        .enumerate()
        .skip(4)
        .find(|(_, entry)| entry.playable && entry.frame_count > 1)
        .expect("no inherited animation plays")
        .0;
    let decoded = decode_model_animation(&source, &entry, inherited).expect("decode");
    assert!(!decoded.frames.is_empty());
    assert!(decoded.skeleton_names.iter().any(|name| name == "pelvis"));
}

/// `BLAM_TEST_H2EK`: a vehicle draws, animates and colors through the
/// `.model` it names, and the `.model` is what holds what it draws.
#[test]
fn an_object_previews_through_its_model() {
    let Some(tags) = kit_root(GameId::Halo2) else {
        return;
    };
    let rel = "objects/vehicles/pelican/pelican.vehicle";
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(GameId::Halo2),
        definitions_root: crate::core::test_kits::definitions().to_path_buf(),
    };
    let entry = TagEntry {
        key: file_entry_key(&tags.join(rel)),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"vehi"),
        group_name: Some("vehicle".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    };
    let tag = crate::core::source::read_entry(&source, &entry).unwrap();
    let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = crate::core::format::TagNameIndex::load_from_definitions(&defs);
    let data = super::super::load::load_model_preview(
        &tag,
        &entry,
        &names,
        Some(&source),
        &super::super::load::PreviewLoadSettings::default(),
    )
    .expect("the vehicle's model loads");
    assert!(!data.preview.batches.is_empty());
    assert!(!data.variants.is_empty(), "variants come from the .model");
    let list = list_model_animations(&source, &entry).expect("animation list");
    assert!(!list.is_empty());
}

/// `BLAM_TEST_H2EK`: `ultra_scl` names the base helmet, and a permutation
/// called `ultra` that no variant uses must not take its place.
#[test]
fn a_variant_shows_the_permutation_it_names() {
    let Some(tags) = kit_root(GameId::Halo2) else {
        return;
    };
    let rel = "objects/characters/elite/elite.model";
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(GameId::Halo2),
        definitions_root: crate::core::test_kits::definitions().to_path_buf(),
    };
    let entry = TagEntry {
        key: file_entry_key(&tags.join(rel)),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    };
    let tag = crate::core::source::read_entry(&source, &entry).unwrap();
    let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = crate::core::format::TagNameIndex::load_from_definitions(&defs);
    let data = super::super::load::load_model_preview(
        &tag,
        &entry,
        &names,
        Some(&source),
        &super::super::load::PreviewLoadSettings::default(),
    )
    .unwrap();
    let helmet = |variant: &str| {
        let index = data.variants.iter().position(|entry| entry.name == variant);
        compute_variant_selection(&data, index)["helmet"]
            .permutation
            .clone()
    };
    assert_eq!(helmet("ultra_scl"), "base");
    assert_eq!(helmet("council_scl"), "council");
    // Its honor armor is named by an empty entry on every other variant,
    // inherited through `parent variant` on the `_dog` ones.
    let armor = |variant: &str| {
        let index = data.variants.iter().position(|entry| entry.name == variant);
        compute_variant_selection(&data, index)["hg"].enabled
    };
    assert!(armor("honor_scl"));
    assert!(!armor("minor_scl"));
    assert!(!armor("minor_dog"));
}

/// The depth window must cover an animation's travel: the reach bound is
/// at least the largest per-frame chain length, and an empty pose adds
/// nothing.
#[test]
fn pose_reach_covers_the_travelling_frame() {
    let frame = |x: f32| {
        vec![
            PreviewNodeTransform {
                rotation: [0.0, 0.0, 0.0, 1.0],
                translation: [x, 0.0, 0.9],
                scale: 1.0,
            },
            PreviewNodeTransform {
                rotation: [0.0, 0.0, 0.0, 1.0],
                translation: [0.0, 0.0, 0.25],
                scale: 1.0,
            },
        ]
    };
    let pose = PreviewAnimationPose::new(0, vec![frame(0.0), frame(3.0)]);
    let reach = pose.reach;
    let travelled = (3.0f32 * 3.0 + 0.9 * 0.9).sqrt() + 0.25;
    assert!(
        reach >= travelled - 1e-4,
        "reach {reach} misses the travelled frame {travelled}"
    );
    let empty = PreviewAnimationPose::new(0, Vec::new());
    assert_eq!(empty.reach, 0.0);
}
