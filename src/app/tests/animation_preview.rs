//! The animation playback math: skinning matrices, frame blending, and the
//! skeleton-to-preview mapping. The one property everything hangs on: a pose
//! identical to the bind pose must skin every vertex to exactly where the tag
//! put it — any drift there deforms the model just by pressing play.

use super::*;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
use blam_tags::render_model::Node;

fn raw_node(
    name: &str,
    parent: i16,
    translation: RealPoint3d,
    rotation: RealQuaternion,
) -> Node {
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
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose {
        animation_index: 0,
        frames: vec![frame],
    }));

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
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose {
        animation_index: 0,
        frames: vec![frame],
    }));

    let rows = animation_skinning_rows(&data, &state).expect("skinning rows");
    // Root: identity rotation part relative to bind, translation +0.5 in x.
    assert!((rows[0][3] - 0.5).abs() < 1e-4, "root x: {:?}", rows[0]);
    assert!(rows[1][3].abs() < 1e-4);
    assert!(rows[2][3].abs() < 1e-4);
    // The child inherits the same rigid shift, nothing else.
    assert!((rows[3][3] - 0.5).abs() < 1e-4, "child x: {:?}", rows[3]);
}

/// A node the animation does not cover falls back to its own bind pose —
/// identity skin — rather than collapsing to the origin.
#[test]
fn a_node_missing_from_the_pose_stays_at_bind() {
    let nodes = test_nodes();
    let frame = vec![bind_pose_frame(&nodes)[0]]; // only the root
    let data = preview_with_nodes(nodes);
    let mut state = ModelPreviewState::default();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose {
        animation_index: 0,
        frames: vec![frame],
    }));

    let rows = animation_skinning_rows(&data, &state).expect("skinning rows");
    assert!((rows[3][0] - 1.0).abs() < 1e-4, "child rotation row: {:?}", &rows[3]);
    assert!(rows[3][3].abs() < 1e-4, "child translation: {:?}", &rows[3]);
}

/// Point `BABOON_REACH_KIT` at a Reach kit's `tags` folder to prove big
/// Reach skeletons play: mule (99 nodes) and halsey (163) both blew the old
/// 96-bone budget and silently showed no animation strip at all. Absent, this
/// self-skips.
#[test]
fn a_reach_skeleton_past_the_old_bone_budget_lists_and_decodes() {
    let Some(tags_root) = std::env::var_os("BABOON_REACH_KIT").map(std::path::PathBuf::from)
    else {
        eprintln!("skipping: set BABOON_REACH_KIT to a Reach editing kit's tags folder");
        return;
    };
    let source = TagSource::LooseFolder {
        root: tags_root.clone(),
        game: Some("haloreach_mcc".to_owned()),
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
        let model = crate::source::read_entry(&source, &entry).expect("model reads");
        let (_, render_rel) = model
            .root()
            .read_tag_ref_with_group("render model")
            .expect("render model ref");
        let preview = load_referenced_tag_from_source(&source, &render_rel, "render_model", b"mode")
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
    let Some(tags_root) = std::env::var_os("BABOON_MODEL_KIT").map(std::path::PathBuf::from)
    else {
        eprintln!("skipping: set BABOON_MODEL_KIT to an editing kit's tags folder");
        return;
    };
    let model_path = tags_root.join("objects/characters/masterchief/masterchief.model");
    if !model_path.is_file() {
        eprintln!("skipping: no masterchief.model under {}", tags_root.display());
        return;
    }
    let source = TagSource::LooseFolder {
        root: tags_root,
        game: Some("halo3_mcc".to_owned()),
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
