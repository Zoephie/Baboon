//! Animation playback for the `.model` preview: listing the linked graph's
//! animations, decoding a selection into per-node frames off the UI thread,
//! and sampling the current time into GPU skinning matrices. Geometry,
//! rendering, and the panel presentation belong elsewhere.

use super::*;
use blam_tags::math::Matrix4;
use blam_tags::animation::classic::{CeAnimation, CeAnimations};
use blam_tags::{Animation, AnimationGraph, JmaKind, NodeTransform, Skeleton};

/// Halo's animation clock. No tag carries a rate; the engine (and the JMA
/// header the extractor writes) is a fixed 30 Hz.
pub(crate) const ANIMATION_FRAME_RATE: f32 = 30.0;

/// One row of the preview's animation list.
#[derive(Debug, Clone)]
pub(crate) struct PreviewAnimationEntry {
    pub name: String,
    pub frame_count: u16,
    /// A JMA-family label (`jma`, `jmo`, `jmr`, …) for the row.
    pub kind: &'static str,
    /// False when the build kept no payload for this animation (a monolithic
    /// capture's unpaged resource, or a runtime-blend composite) — listed,
    /// but not selectable.
    pub playable: bool,
}

/// One node's transform at one frame, parent-local.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PreviewNodeTransform {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    pub scale: f32,
}

impl PreviewNodeTransform {
    fn from_node_transform(transform: &NodeTransform) -> Self {
        Self {
            rotation: transform.rotation.normalized().to_array(),
            translation: [
                transform.translation.x,
                transform.translation.y,
                transform.translation.z,
            ],
            scale: if transform.scale.is_finite() && transform.scale != 0.0 {
                transform.scale
            } else {
                1.0
            },
        }
    }
}

/// A decoded animation in SKELETON node order, straight off the worker. The
/// handler maps it onto the preview's node order by name.
#[derive(Debug, Clone)]
pub(crate) struct DecodedAnimationPose {
    pub skeleton_names: Vec<String>,
    pub frames: Vec<Vec<PreviewNodeTransform>>,
}

impl DecodedAnimationPose {
    fn new(skeleton: &Skeleton, pose: &blam_tags::Pose) -> Self {
        Self {
            skeleton_names: skeleton
                .nodes
                .iter()
                .map(|node| node.name.clone())
                .collect(),
            frames: pose
                .frames
                .iter()
                .map(|frame| {
                    frame
                        .iter()
                        .map(PreviewNodeTransform::from_node_transform)
                        .collect()
                })
                .collect(),
        }
    }
}

/// A decoded animation mapped onto `RenderModelPreview::nodes` order, ready
/// to sample every draw frame.
#[derive(Debug, Clone)]
pub(crate) struct PreviewAnimationPose {
    /// Which list entry this is, so a stale selection change re-decodes.
    pub animation_index: usize,
    /// `frames[frame][preview_node]`, parent-local.
    pub frames: Vec<Vec<PreviewNodeTransform>>,
    /// How far any frame can carry a node from the model origin; see
    /// [`PreviewAnimationPose::new`]. Worked out once here: the camera needs
    /// it every frame, and it walks every node of every frame.
    pub reach: f32,
}

impl PreviewAnimationPose {
    pub fn new(animation_index: usize, frames: Vec<Vec<PreviewNodeTransform>>) -> Self {
        let reach = pose_reach_bound(&frames);
        Self {
            animation_index,
            frames,
            reach,
        }
    }
}

/// How far a decoded pose can carry any node from the model origin, as the
/// largest per-frame sum of local translation norms (rotations preserve
/// norms, so a chain can never reach further than its links laid end to end),
/// stretched by the frame's largest scale. Loose on purpose: it only sizes
/// the depth window, where slack costs precision and a tight miss costs
/// geometry.
fn pose_reach_bound(frames: &[Vec<PreviewNodeTransform>]) -> f32 {
    let mut reach = 0.0f32;
    for frame in frames {
        let mut total = 0.0f32;
        let mut max_scale = 1.0f32;
        for transform in frame {
            let [x, y, z] = transform.translation;
            total += (x * x + y * y + z * z).sqrt();
            max_scale = max_scale.max(transform.scale.abs());
        }
        reach = reach.max(total * max_scale);
    }
    if reach.is_finite() { reach } else { 0.0 }
}

/// Cross-frame playback state, one per previewed document.
pub(crate) struct PreviewAnimationPlayback {
    pub selected: Option<usize>,
    pub playing: bool,
    /// Stop is distinct from pause: it shows the bind pose while retaining
    /// the selected and decoded animation.
    pub stopped: bool,
    pub looped: bool,
    /// Blend between frames. Off, the pose holds each frame whole, which is
    /// how an overlay animation is read frame by frame.
    pub interpolate: bool,
    pub speed: f32,
    /// Seconds into the animation.
    pub time: f32,
    pub pose: Option<std::sync::Arc<PreviewAnimationPose>>,
    /// The animation index a decode worker is running for.
    pub decoding: Option<usize>,
    pub error: Option<String>,
    /// The one-shot guard on the list request for this load.
    pub requested_list: bool,
    /// Case-insensitive substring filter for the animation picker; a graph
    /// can list a thousand animations.
    pub filter: String,
    /// The egui pass the clock last advanced in. Two panes showing the same
    /// tag share this state, and each advanced the clock, so playback ran at
    /// twice the speed.
    pub advanced_in_pass: Option<u64>,
}

impl Default for PreviewAnimationPlayback {
    fn default() -> Self {
        Self {
            selected: None,
            playing: false,
            stopped: false,
            looped: true,
            interpolate: true,
            speed: 1.0,
            time: 0.0,
            pose: None,
            decoding: None,
            error: None,
            requested_list: false,
            filter: String::new(),
            advanced_in_pass: None,
        }
    }
}

/// Where playback stands within `frame_count` frames, in frames: wrapped when
/// looping, held at the last frame when not, and on a whole frame when not
/// interpolating.
pub(super) fn playback_frame_position(playback: &PreviewAnimationPlayback, frame_count: usize) -> f32 {
    let mut position = (playback.time * ANIMATION_FRAME_RATE).max(0.0);
    if playback.looped && frame_count > 1 {
        position %= frame_count as f32;
    } else {
        position = position.min(frame_count.saturating_sub(1) as f32);
    }
    if playback.interpolate { position } else { position.floor() }
}

/// The two frames playback lies between and how far it is from the first to
/// the second. `frame_count` must not be 0.
fn sampled_frames(playback: &PreviewAnimationPlayback, frame_count: usize) -> (usize, usize, f32) {
    let position = playback_frame_position(playback, frame_count);
    let a = (position.floor() as usize).min(frame_count - 1);
    let b = if playback.looped {
        (a + 1) % frame_count
    } else {
        (a + 1).min(frame_count - 1)
    };
    (a, b, position - a as f32)
}

/// Sample the playback state into skinning-matrix rows for this draw frame —
/// three vec4 rows per preview node, `world × inverse_bind`, so an
/// unanimated node lands exactly on its bind pose. `None` draws the plain
/// bind pose.
pub(super) fn animation_skinning_rows(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Option<Vec<[f32; 4]>> {
    let playback = &state.animation;
    if playback.stopped {
        return None;
    }
    let pose = playback.pose.as_ref()?;
    let nodes = &data.preview.nodes;
    if nodes.is_empty() || nodes.len() > MAX_PREVIEW_BONES || pose.frames.is_empty() {
        return None;
    }

    let (frame_a, frame_b, blend) = sampled_frames(playback, pose.frames.len());
    let (frame_a, frame_b) = (&pose.frames[frame_a], &pose.frames[frame_b]);

    let mut world: Vec<(RealQuaternion, RealVector3d, f32)> = Vec::with_capacity(nodes.len());
    let mut rows: Vec<[f32; 4]> = Vec::with_capacity(nodes.len() * 3);
    for (index, node) in nodes.iter().enumerate() {
        let local = blend_transforms(frame_a.get(index), frame_b.get(index), blend, node);
        let (rotation, translation, scale) = if node.parent >= 0 {
            let (parent_rotation, parent_translation, parent_scale) = world
                .get(node.parent as usize)
                .copied()
                .unwrap_or((RealQuaternion::IDENTITY, RealVector3d::ZERO, 1.0));
            (
                (parent_rotation * local.0).normalized(),
                parent_translation + (parent_rotation * (local.1 * parent_scale)),
                parent_scale * local.2,
            )
        } else {
            local
        };
        world.push((rotation, translation, scale));

        let world_matrix = Matrix4::from_loc_rot_scale(
            RealPoint3d {
                x: translation.i,
                y: translation.j,
                z: translation.k,
            },
            rotation,
            scale,
        );
        let inverse_bind = Matrix4 {
            m: [
                node.inverse_bind[0],
                node.inverse_bind[1],
                node.inverse_bind[2],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        let skin = world_matrix * inverse_bind;
        rows.push(skin.m[0]);
        rows.push(skin.m[1]);
        rows.push(skin.m[2]);
    }
    Some(rows)
}

/// World-space joint positions for the armature overlay. This follows the
/// same sampled parent-local pose as GPU skinning, and falls back to the bind
/// pose when no animation is active.
pub(super) fn armature_node_positions(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Vec<[f32; 3]> {
    let nodes = &data.preview.nodes;
    if nodes.is_empty() || nodes.len() > MAX_PREVIEW_BONES {
        return Vec::new();
    }

    let sampled_frames = (!state.animation.stopped)
        .then_some(state.animation.pose.as_ref())
        .flatten()
        .and_then(|pose| {
            if pose.frames.is_empty() {
                return None;
            }
            let (a, b, blend) = sampled_frames(&state.animation, pose.frames.len());
            Some((&pose.frames[a], &pose.frames[b], blend))
        });

    let mut world: Vec<(RealQuaternion, RealVector3d, f32)> = Vec::with_capacity(nodes.len());
    let mut positions = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let local = if let Some((a, b, blend)) = sampled_frames {
            blend_transforms(a.get(index), b.get(index), blend, node)
        } else {
            (
                quat(node.bind_rotation),
                RealVector3d {
                    i: node.bind_translation[0],
                    j: node.bind_translation[1],
                    k: node.bind_translation[2],
                },
                1.0,
            )
        };
        let transform = if node.parent >= 0 {
            let (parent_rotation, parent_translation, parent_scale) = world
                .get(node.parent as usize)
                .copied()
                .unwrap_or((RealQuaternion::IDENTITY, RealVector3d::ZERO, 1.0));
            (
                (parent_rotation * local.0).normalized(),
                parent_translation + (parent_rotation * (local.1 * parent_scale)),
                parent_scale * local.2,
            )
        } else {
            local
        };
        positions.push([transform.1.i, transform.1.j, transform.1.k]);
        world.push(transform);
    }
    positions
}

/// Blend one node between two frames — nlerp on the shorter arc for the
/// rotation, lerp for translation and scale — falling back to the node's own
/// bind pose when the animation carries no entry for it.
fn blend_transforms(
    a: Option<&PreviewNodeTransform>,
    b: Option<&PreviewNodeTransform>,
    blend: f32,
    node: &RenderModelPreviewNode,
) -> (RealQuaternion, RealVector3d, f32) {
    let bind = PreviewNodeTransform {
        rotation: node.bind_rotation,
        translation: node.bind_translation,
        scale: 1.0,
    };
    let a = a.unwrap_or(&bind);
    let b = b.unwrap_or(&bind);
    let qa = quat(a.rotation);
    let mut qb = quat(b.rotation);
    if qa.dot(qb) < 0.0 {
        qb = -qb;
    }
    let rotation = qa.nlerp(qb, blend).normalized();
    let translation = RealVector3d {
        i: a.translation[0] + (b.translation[0] - a.translation[0]) * blend,
        j: a.translation[1] + (b.translation[1] - a.translation[1]) * blend,
        k: a.translation[2] + (b.translation[2] - a.translation[2]) * blend,
    };
    let scale = a.scale + (b.scale - a.scale) * blend;
    (rotation, translation, scale)
}

fn quat(values: [f32; 4]) -> RealQuaternion {
    RealQuaternion {
        i: values[0],
        j: values[1],
        k: values[2],
        w: values[3],
    }
}

#[cfg(test)]
mod tests {
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

    /// Halfway between two frames, the pose is a blend of them; with
    /// interpolation off it is the first frame exactly, which is how an
    /// overlay animation is read frame by frame.
    #[test]
    fn without_interpolation_the_pose_holds_each_frame() {
        let nodes = test_nodes();
        let data = preview_with_nodes(nodes.clone());
        let bind = armature_node_positions(&data, &ModelPreviewState::default());
        let first = bind_pose_frame(&nodes);
        let mut second = bind_pose_frame(&nodes);
        second[0].translation[0] += 1.0;
        let mut state = ModelPreviewState::default();
        state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(0, vec![first, second])));
        state.animation.looped = false;
        state.animation.time = 0.5 / ANIMATION_FRAME_RATE;

        let blended = armature_node_positions(&data, &state);
        assert!((blended[0][0] - bind[0][0] - 0.5).abs() < 1e-4, "halfway between the frames");

        state.animation.interpolate = false;
        let held = armature_node_positions(&data, &state);
        assert!((held[0][0] - bind[0][0]).abs() < 1e-4, "still on the first frame");
        assert_eq!(playback_frame_position(&state.animation, 2), 0.0);

        state.animation.time = 1.0 / ANIMATION_FRAME_RATE;
        let next = armature_node_positions(&data, &state);
        assert!((next[0][0] - bind[0][0] - 1.0).abs() < 1e-4, "on the second frame");
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
        let names = TagNameIndex::load_game(crate::core::test_kits::definitions(), GameId::from_id(game).unwrap()).expect("tag names");
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
        let tags = std::path::PathBuf::from(crate::core::test_kits::tag_path("haloce_mcc", ""));
        let rel = "characters/cyborg/cyborg.biped";
        if !tags.join(rel).is_file() {
            eprintln!("skipping: set BLAM_TEST_HCEEK to a Halo CE kit's tags folder");
            return;
        }
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
        let tags = std::path::PathBuf::from(crate::core::test_kits::tag_path("haloce_mcc", ""));
        let rel = "vehicles/warthog/warthog.vehicle";
        if !tags.join(rel).is_file() {
            eprintln!("skipping: set BLAM_TEST_HCEEK to a Halo CE kit's tags folder");
            return;
        }
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

    /// `BLAM_TEST_H2EK` names a Halo 2 kit's `tags` folder.
    #[test]
    fn a_halo_2_model_plays_its_idle() {
        let tags = crate::core::test_kits::h2ek_tags();
        let rel = "objects/characters/masterchief/masterchief.model";
        if !tags.join(rel).is_file() {
            eprintln!("skipping: set BLAM_TEST_H2EK to a Halo 2 kit's tags folder");
            return;
        }
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
}

impl Baboon {
    /// Start listing the animations in a loaded `.model` preview's linked
    /// graph, once per load, on a worker. Rides the same per-frame hook as
    /// the texture and overlay requests.
    pub(in crate::app) fn maybe_request_model_animations(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) {
        let kit = &self.model.kits[kit_index];
        let view = &self.views[kit.id];
        let Some(state) = view.caches.model_previews.get(key) else {
            return;
        };
        if state.animation.requested_list {
            return;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return;
        };
        // No skeleton (or one past the GPU bone budget) means nothing could
        // play; don't spend a worker discovering that.
        if data.preview.nodes.is_empty() || data.preview.nodes.len() > MAX_PREVIEW_BONES {
            return;
        }
        let Some(entry) = kit.entry_for_key(key).cloned() else {
            return;
        };
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return;
        };
        // A `.model` names its graph; a Halo CE object has no `.model` and
        // names its `model_animations` itself, the way it names its gbxmodel.
        let names_a_graph = entry.group_tag == u32::from_be_bytes(*b"hlmt")
            || (is_object_family_group(entry.group_tag) && source_is_halo1(&source));
        if !names_a_graph {
            return;
        }
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        if let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(key) {
            state.animation.requested_list = true;
        }

        let (key, panic_key) = (key.to_owned(), key.to_owned());
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::ModelAnimationsListed {
                stamp,
                key,
                result: list_model_animations(&source, &entry),
            },
            move |_| WorkerMessage::ModelAnimationsListed {
                stamp,
                key: panic_key,
                result: Err("the animation graph crashed the reader".to_owned()),
            },
        );
    }

    /// Start decoding the animation the panel selected, if it is not the one
    /// already decoded or being decoded.
    pub(in crate::app) fn maybe_request_model_animation_decode(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) {
        let kit = &self.model.kits[kit_index];
        let view = &self.views[kit.id];
        let Some(state) = view.caches.model_previews.get(key) else {
            return;
        };
        let Some(selected) = state.animation.selected else {
            return;
        };
        if state.animation.decoding.is_some()
            || state
                .animation
                .pose
                .as_ref()
                .is_some_and(|pose| pose.animation_index == selected)
        {
            return;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return;
        };
        if !data
            .animations
            .as_ref()
            .and_then(|animations| animations.get(selected))
            .is_some_and(|entry| entry.playable)
        {
            return;
        }
        let Some(entry) = kit.entry_for_key(key).cloned() else {
            return;
        };
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return;
        };
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        if let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(key) {
            state.animation.decoding = Some(selected);
            state.animation.error = None;
        }

        let (key, panic_key) = (key.to_owned(), key.to_owned());
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::ModelAnimationDecoded {
                stamp,
                key,
                animation_index: selected,
                result: decode_model_animation(&source, &entry, selected),
            },
            move |_| WorkerMessage::ModelAnimationDecoded {
                stamp,
                key: panic_key,
                animation_index: selected,
                result: Err("this animation crashed the decoder".to_owned()),
            },
        );
    }

    pub(in crate::app) fn handle_model_animations_listed(
        &mut self,
        stamp: KitStamp,
        key: String,
        result: Result<Vec<PreviewAnimationEntry>, String>,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.model.resolve_stamp(stamp).is_none();
        let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(&key) else {
            return true;
        };
        if stale {
            // Asked for once per preview, so a list dropped as stale has to be
            // asked for again, or it never arrives.
            state.animation.requested_list = false;
            return true;
        }
        match result {
            Ok(entries) => {
                if let Some(Ok(data)) = state.data.as_mut() {
                    data.animations = Some(std::sync::Arc::new(entries));
                }
            }
            Err(error) => state.animation.error = Some(error),
        }
        false
    }

    pub(in crate::app) fn handle_model_animation_decoded(
        &mut self,
        stamp: KitStamp,
        key: String,
        animation_index: usize,
        result: Result<DecodedAnimationPose, String>,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.model.resolve_stamp(stamp).is_none();
        let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(&key) else {
            return true;
        };
        // Cleared before the staleness check, so a decode dropped for a
        // generation bump does not leave the clip "decoding" for good.
        if state.animation.decoding == Some(animation_index) {
            state.animation.decoding = None;
        }
        if stale {
            return true;
        }
        // The selection moved on while this decoded; the per-frame hook will
        // have started (or will start) the right one.
        if state.animation.selected != Some(animation_index) {
            return true;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return true;
        };
        match result {
            Ok(decoded) => {
                // Skeleton order → preview node order, matched by name — the
                // only mapping the engine itself uses.
                let skeleton_index: HashMap<&str, usize> = decoded
                    .skeleton_names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| (name.as_str(), index))
                    .collect();
                let node_sources: Vec<Option<usize>> = data
                    .preview
                    .nodes
                    .iter()
                    .map(|node| skeleton_index.get(node.name.as_str()).copied())
                    .collect();
                let frames = decoded
                    .frames
                    .iter()
                    .map(|frame| {
                        data.preview
                            .nodes
                            .iter()
                            .zip(&node_sources)
                            .map(|(node, source)| {
                                source
                                    .and_then(|index| frame.get(index).copied())
                                    .unwrap_or(PreviewNodeTransform {
                                        rotation: node.bind_rotation,
                                        translation: node.bind_translation,
                                        scale: 1.0,
                                    })
                            })
                            .collect()
                    })
                    .collect();
                state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(
                    animation_index,
                    frames,
                )));
                state.animation.time = 0.0;
                state.animation.playing = true;
                state.animation.stopped = false;
            }
            Err(error) => {
                state.animation.error = Some(error);
                state.animation.playing = false;
            }
        }
        false
    }
}

/// Whether a kit is Halo CE, whose objects stand in for the `.model`.
///
/// Exactly `haloce_mcc`: a prefix test also took Campaign Evolved
/// (`haloce_evolved`), whose tags are Reach's and have a `.model`.
fn source_is_halo1(source: &TagSource) -> bool {
    matches!(source, TagSource::LooseFolder { game: Some(GameId::HaloCe), .. })
}

/// Worker half of the list request.
fn list_model_animations(
    source: &TagSource,
    entry: &TagEntry,
) -> Result<Vec<PreviewAnimationEntry>, String> {
    let model = crate::core::source::read_entry(source, entry).map_err(|error| error.to_string())?;
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
            let jmad_ref = tag_ref_path(&model.root(), "animation")
                .ok_or("This model references no animation graph.")?;
            let jmad =
                load_referenced_tag_from_source(source, &jmad_ref, "model_animation_graph", b"jmad")
                    .map_err(|error| error.to_string())?;
            let animation = Animation::new(&jmad).map_err(|error| error.to_string())?;
            Ok(animation
                .iter()
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

/// A Halo CE object's `model_animations`.
fn load_object_animations(source: &TagSource, object: &TagFile) -> Result<TagFile, String> {
    let reference = halo1_object_reference(object, "animation graph")
        .ok_or("This object references no model_animations.")?;
    load_referenced_tag_from_source(source, &reference, "model_animations", b"antr")
        .map_err(|error| error.to_string())
}

fn ce_jma_kind(animation: &CeAnimation<'_>) -> JmaKind {
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
fn ce_rest_pose(skeleton: &Skeleton, gbxmodel: Option<&TagFile>) -> Vec<NodeTransform> {
    let mut by_name: HashMap<String, NodeTransform> = HashMap::new();
    if let Some(nodes) = gbxmodel
        .and_then(|tag| tag.root().field("nodes"))
        .and_then(|field| field.as_block())
    {
        for node in (0..nodes.len()).filter_map(|index| nodes.element(index)) {
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
        .map(|node| by_name.get(&node.name).copied().unwrap_or(NodeTransform::IDENTITY))
        .collect()
}

/// Worker half of the decode request: the extractor's exact composition
/// recipe (`write_group_jma`), minus the file write.
fn decode_model_animation(
    source: &TagSource,
    entry: &TagEntry,
    animation_index: usize,
) -> Result<DecodedAnimationPose, String> {
    let model = crate::core::source::read_entry(source, entry).map_err(|error| error.to_string())?;
    if blam_tags::game::Game::of(&model) == blam_tags::game::Game::Halo1 {
        return decode_ce_animation(source, &model, animation_index);
    }
    let root = model.root();
    let jmad_ref =
        tag_ref_path(&root, "animation").ok_or("This model references no animation graph.")?;
    let jmad = load_referenced_tag_from_source(source, &jmad_ref, "model_animation_graph", b"jmad")
        .map_err(|error| error.to_string())?;
    let animation = Animation::new(&jmad).map_err(|error| error.to_string())?;
    let skeleton = Skeleton::from_tag(&jmad);
    let render_tag = tag_ref_path(&root, "render model").and_then(|reference| {
        load_referenced_tag_from_source(source, &reference, "render_model", b"mode").ok()
    });
    let object_space =
        blam_tags::extract::animation::additional_node_data_is_object_space(&animation);
    let defaults = blam_tags::extract::animation::build_defaults(
        &skeleton,
        &jmad,
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
            let graph = AnimationGraph::from_tag(&jmad);
            animation
                .overlay_base_pose(&graph, group, &skeleton, &defaults)
                .unwrap_or_else(|| defaults.clone())
        }
        _ => defaults.clone(),
    };
    let pose = match kind {
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

    Ok(DecodedAnimationPose::new(&skeleton, &pose))
}

/// The Halo CE half of [`decode_model_animation`]: the extractor's recipe
/// (`write_ce_group_jma`), on the gbxmodel's rest pose.
fn decode_ce_animation(
    source: &TagSource,
    object: &TagFile,
    animation_index: usize,
) -> Result<DecodedAnimationPose, String> {
    let antr = load_object_animations(source, object)?;
    let animations = CeAnimations::new(&antr);
    let animation = animations
        .get(animation_index)
        .ok_or("The graph no longer lists this animation.")?;
    let gbxmodel = halo1_object_reference(object, "model").and_then(|reference| {
        load_referenced_tag_from_source(source, &reference, "gbxmodel", b"mod2").ok()
    });
    // The extractor's skeleton: a graph that lists no nodes of its own
    // animates its gbxmodel's, when their node list checksums agree.
    let skeleton = blam_tags::extract::animation::ce_skeleton(&animations, &antr, gbxmodel.as_ref());
    let rest = ce_rest_pose(&skeleton, gbxmodel.as_ref());
    let clip = animation.decode();
    let mut pose = match ce_jma_kind(animation) {
        JmaKind::Jmo => clip.overlay_pose(&skeleton, &rest).1,
        JmaKind::Jmr => clip.replacement_pose(&skeleton, &rest),
        _ => clip.pose(&skeleton, Some(&rest)),
    };
    // CE rotations, rest pose and animation alike, are stored inverted
    // relative to the forward chaining the preview runs: the engine feeds
    // both into the same orientations (`model_get_node_orientations`, CE
    // Anniversary X360), and `RenderModel` conjugates the gbxmodel's. Over
    // the standing animations of six characters, frame 0 sits a median 20-36°
    // from the bind pose conjugated and 49-102° as stored; Halo 2 is the
    // mirror image, closer as stored.
    for frame in &mut pose.frames {
        for transform in frame {
            let q = transform.rotation;
            transform.rotation = RealQuaternion { i: -q.i, j: -q.j, k: -q.k, w: q.w };
        }
    }
    Ok(DecodedAnimationPose::new(&skeleton, &pose))
}
