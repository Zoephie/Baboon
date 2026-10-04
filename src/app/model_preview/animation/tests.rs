//! The animation playback math: skinning matrices, frame blending, and the
//! skeleton-to-preview mapping. The one property everything hangs on: a pose
//! identical to the bind pose must skin every vertex to exactly where the tag
//! put it — any drift there deforms the model just by pressing play.

use super::*;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
use blam_tags::render_model::Node;

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
                i: 0.2588190,
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
        key: format!("file:{}", tags_root.join(rel).display()),
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
        key: format!("file:{}", model_path.display()),
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
    let dot = a.iter().zip(b).map(|(a, b)| a * b).sum::<f32>().abs().min(1.0);
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
    let names = TagNameIndex::load_game(crate::test_kits::definitions(), GameId::from_id(game).unwrap()).expect("tag names");
    let data = crate::app::model_preview::loading::load_model_preview(
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
        .unwrap_or_else(|| panic!("{}: no '{idle}' in {} animations", entry.display_path, list.len()));
    assert!(list[index].playable);
    let decoded = decode_model_animation(&source, &entry, index).expect("decode");
    assert_eq!(decoded.frames.len(), list[index].frame_count as usize);

    let by_name: HashMap<&str, &RenderModelPreviewNode> =
        nodes.iter().map(|node| (node.name.as_str(), node)).collect();
    let (mut chosen, mut opposite) = (Vec::new(), Vec::new());
    for (name, transform) in decoded.skeleton_names.iter().zip(&decoded.frames[0]) {
        let node = by_name
            .get(name.as_str())
            .unwrap_or_else(|| panic!("{}: animated node '{name}' is not in the preview", entry.display_path));
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
    let tags = std::path::PathBuf::from(crate::test_kits::tag_path("haloce_mcc", ""));
    let rel = "characters/cyborg/cyborg.biped";
    if !tags.join(rel).is_file() {
        eprintln!("skipping: set BLAM_TEST_HCEEK to a Halo CE kit's tags folder");
        return;
    }
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(GameId::HaloCe),
        definitions_root: crate::test_kits::definitions().to_path_buf(),
    };
    let entry = TagEntry {
        key: format!("file:{}", tags.join(rel).display()),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"bipd"),
        group_name: Some("biped".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    };
    plays_a_classic_idle(source, entry, "haloce_mcc", "stand rifle idle");
}

/// `BLAM_TEST_H2EK` names a Halo 2 kit's `tags` folder.
#[test]
fn a_halo_2_model_plays_its_idle() {
    let tags = crate::test_kits::h2ek_tags();
    let rel = "objects/characters/masterchief/masterchief.model";
    if !tags.join(rel).is_file() {
        eprintln!("skipping: set BLAM_TEST_H2EK to a Halo 2 kit's tags folder");
        return;
    }
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(GameId::Halo2),
        definitions_root: crate::test_kits::definitions().to_path_buf(),
    };
    let entry = TagEntry {
        key: format!("file:{}", tags.join(rel).display()),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    };
    plays_a_classic_idle(source, entry, "halo2_mcc", "combat:rifle:idle");
}

/// Only Halo CE's objects stand in for a `.model`. Campaign Evolved's game id
/// also starts with `haloce`, but its tags are Reach's.
#[test]
fn only_halo_ce_is_halo1_for_animation_lists() {
    let folder = |game: &str| TagSource::LooseFolder {
        root: std::path::PathBuf::from("/tags"),
        game: GameId::from_id(game),
        definitions_root: std::path::PathBuf::new(),
    };
    assert!(source_is_halo1(&folder("haloce_mcc")));
    assert!(!source_is_halo1(&folder("haloce_evolved")));
    assert!(!source_is_halo1(&folder("halo2_mcc")));
}
