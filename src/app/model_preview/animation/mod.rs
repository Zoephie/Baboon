//! Animation playback for the `.model` preview: listing the linked graph's
//! animations, decoding a selection into per-node frames off the UI thread,
//! and sampling the current time into GPU skinning matrices. Geometry,
//! rendering, and the panel presentation belong elsewhere.

/// Reach's blend screens: the aim, look and other pose overlays laid over an
/// action.
pub(super) mod blend_screens;
/// Inverse kinematics: a held weapon's hand, Reach's chains and Halo 4's
/// IK sets.
pub(super) mod ik;

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
    /// The animation's `loop frame index`.
    pub loop_frame: usize,
    /// Reach: what its IK chains reach for, frame by frame.
    pub reach_ik: Vec<super::animation::ik::reach::ReachIkTrack>,
    /// Halo 4: the clip's IK set, solved before the chains.
    pub h4_ik_set: Vec<super::animation::ik::h4::H4IkPoint>,
}

impl DecodedAnimationPose {
    fn new(skeleton: &Skeleton, pose: &blam_tags::Pose, loop_frame: usize) -> Self {
        Self {
            loop_frame,
            reach_ik: Vec::new(),
            h4_ik_set: Vec::new(),
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
    /// The frame looping playback continues from after the last; 0 restarts.
    pub loop_frame: usize,
    /// Reach's IK chain targets, solved after each sampled pose.
    pub reach_ik: std::sync::Arc<Vec<super::animation::ik::reach::ReachIkTrack>>,
    /// Halo 4's IK set for the clip, solved before the chains.
    pub h4_ik_set: std::sync::Arc<Vec<super::animation::ik::h4::H4IkPoint>>,
}

impl PreviewAnimationPose {
    pub fn new(animation_index: usize, frames: Vec<Vec<PreviewNodeTransform>>) -> Self {
        let reach = pose_reach_bound(&frames);
        Self {
            animation_index,
            frames,
            reach,
            loop_frame: 0,
            reach_ik: std::sync::Arc::default(),
            h4_ik_set: std::sync::Arc::default(),
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

/// Where playback stands within `frame_count` frames: past the end of a
/// looping clip it continues from the loop frame, and a clip that doesn't
/// loop holds its last frame. On a whole frame when not interpolating.
pub(super) fn playback_frame_position(playback: &PreviewAnimationPlayback, frame_count: usize) -> f32 {
    let mut position = (playback.time * ANIMATION_FRAME_RATE).max(0.0);
    if playback.looped && frame_count > 1 {
        position = looped_frame_position(playback, position, frame_count);
    } else {
        position = position.min(frame_count.saturating_sub(1) as f32);
    }
    if playback.interpolate { position } else { position.floor() }
}

/// `position` frames into a looping clip of `frame_count` frames. The game
/// plays to the end, then continues from the animation's loop frame, or
/// restarts at frame 0 when that is 0; nothing blends the last frame back
/// into the loop frame.
pub(super) fn looped_frame_position(
    playback: &PreviewAnimationPlayback,
    position: f32,
    frame_count: usize,
) -> f32 {
    let count = frame_count as f32;
    if position < count {
        return position;
    }
    let start = playback
        .pose
        .as_ref()
        .map_or(0, |pose| pose.loop_frame)
        .min(frame_count.saturating_sub(1)) as f32;
    start + (position - start) % (count - start)
}

/// The two frames playback lies between and how far it is from the first to
/// the second. The last frame is held rather than blended into the next
/// loop. `frame_count` must not be 0.
fn sampled_frames(playback: &PreviewAnimationPlayback, frame_count: usize) -> (usize, usize, f32) {
    let position = playback_frame_position(playback, frame_count);
    let a = (position.floor() as usize).min(frame_count - 1);
    let b = (a + 1).min(frame_count - 1);
    (a, b, position - a as f32)
}

/// Which nodes a variant's `muted nodes` hold: each named node and its whole
/// subtree (`model_postprocess_muted_nodes`, X360 0x82D96890, through
/// `model_set_muted_flag_subtree`). Nodes come parents first.
pub(super) fn muted_nodes(nodes: &[RenderModelPreviewNode], names: &[String]) -> Vec<bool> {
    let mut muted = vec![false; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        let parent_muted = usize::try_from(node.parent).ok().and_then(|parent| muted.get(parent)).copied().unwrap_or(false);
        muted[index] = parent_muted || names.contains(&node.name);
    }
    muted
}

/// The selected variant's [`muted_nodes`]; none with no variant selected.
fn variant_muted_nodes(data: &ModelPreviewData, state: &ModelPreviewState) -> Vec<bool> {
    let nodes = &data.preview.nodes;
    state
        .selected_variant
        .and_then(|index| data.variants.get(index))
        .map_or_else(|| vec![false; nodes.len()], |variant| muted_nodes(nodes, &variant.muted_nodes))
}

/// Each node's world rotation, translation and scale, from the local
/// transform `local` gives it: parents first, as the nodes come.
fn world_pose(
    nodes: &[RenderModelPreviewNode],
    mut local: impl FnMut(usize, &RenderModelPreviewNode) -> (RealQuaternion, RealVector3d, f32),
) -> Vec<(RealQuaternion, RealVector3d, f32)> {
    let mut world: Vec<(RealQuaternion, RealVector3d, f32)> = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let local = local(index, node);
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
        world.push(transform);
    }
    world
}

/// A muted node's local transform (`object_handle_muted_nodes`, X360
/// 0x84A476D0): the render model node's default rotation and translation,
/// scale 1.
fn bind_transform(node: &RenderModelPreviewNode) -> (RealQuaternion, RealVector3d, f32) {
    // With no frame either side, the blend is the bind pose.
    blend_transforms(None, None, 0.0, node)
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

    let muted = variant_muted_nodes(data, state);
    let world = world_pose(nodes, |index, node| {
        if muted[index] {
            bind_transform(node)
        } else {
            blend_transforms(frame_a.get(index), frame_b.get(index), blend, node)
        }
    });
    let mut matrices: Vec<Matrix4> = world.iter().copied().map(world_matrix).collect();
    let inverse_binds: Vec<Matrix4> = nodes.iter().map(inverse_bind_matrix).collect();
    apply_weapon_ik(data, state, &mut matrices, &inverse_binds);
    // `biped_postprocess_node_matrices` (X360 0x84B1A738): the IK set, then
    // the chains.
    super::animation::ik::h4::apply_h4_ik_set(&pose.h4_ik_set, &data.preview.gen3.ik_markers, nodes, &inverse_binds, &mut matrices);
    super::animation::ik::reach::apply_reach_ik(
        &pose.reach_ik,
        &data.preview.gen3.ik_markers,
        nodes,
        &inverse_binds,
        &mut matrices,
        sampled_frames(playback, pose.frames.len()),
    );
    let mut rows: Vec<[f32; 4]> = Vec::with_capacity(nodes.len() * 3);
    for (matrix, inverse_bind) in matrices.iter().zip(&inverse_binds) {
        let skin = *matrix * *inverse_bind;
        rows.push(skin.m[0]);
        rows.push(skin.m[1]);
        rows.push(skin.m[2]);
    }
    Some(rows)
}

/// What a Halo 2 unit's weapons lay over the clip at `index` of `clips`.
fn h2_held_layers(
    clips: &[PreviewAnimationEntry],
    index: usize,
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Vec<super::weapons::PoseLayer> {
    let (Some(weapons), Some(clip)) = (data.weapons.as_ref(), clips.get(index)) else {
        return Vec::new();
    };
    let names: Vec<_> = clips.iter().map(|clip| clip.name.as_str()).collect();
    let (primary, secondary) = (state.weapon.as_deref(), state.secondary_weapon.as_deref());
    weapons.pose_layers(&names, &clip.name, primary, secondary)
}

/// Hold a preview at `H2_SHOT_FRAME` of the clip named `clip`, for the headless shot.
#[cfg(test)]
pub(super) fn pose_for_shot(
    source: &TagSource,
    entry: &TagEntry,
    data: &mut ModelPreviewData,
    state: &mut ModelPreviewState,
    clip: &str,
) {
    let list = list_model_animations(source, entry).unwrap();
    let index = list.iter().position(|animation| animation.name == clip).unwrap_or_else(|| {
        let names: Vec<&str> = list.iter().map(|animation| animation.name.as_str()).collect();
        panic!("H2_SHOT_CLIP: no clip {clip:?}; these are: {names:?}")
    });
    let layers = h2_held_layers(&list, index, data, state);
    let decoded = decode_held_animation(source, entry, index, &layers).unwrap();
    let at = std::env::var("H2_SHOT_FRAME").map_or(0, |frame| frame.parse().unwrap());
    let frame = data
        .preview
        .nodes
        .iter()
        .map(|node| {
            let source = decoded.skeleton_names.iter().position(|name| *name == node.name);
            source.and_then(|source| decoded.frames[at].get(source).copied()).unwrap_or(
                PreviewNodeTransform {
                    rotation: node.bind_rotation,
                    translation: node.bind_translation,
                    scale: 1.0,
                },
            )
        })
        .collect();
    data.animations = Some(std::sync::Arc::new(list));
    state.animation.selected = Some(index);
    // The one frame held is the clip's `at`th; its IK reads that frame.
    let reach_ik = decoded
        .reach_ik
        .into_iter()
        .map(|track| super::animation::ik::reach::ReachIkTrack {
            effector_frames: track.effector_frames.get(at).copied().into_iter().collect(),
            weights: track.weights.get(at).copied().into_iter().collect(),
            poles: track.poles.map(|poles| poles.get(at).copied().into_iter().collect()),
            ..track
        })
        .collect();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose {
        reach_ik: std::sync::Arc::new(reach_ik),
        h4_ik_set: std::sync::Arc::new(decoded.h4_ik_set),
        ..PreviewAnimationPose::new(index, vec![frame])
    }));
}

fn world_matrix((rotation, translation, scale): (RealQuaternion, RealVector3d, f32)) -> Matrix4 {
    Matrix4::from_loc_rot_scale(
        RealPoint3d {
            x: translation.i,
            y: translation.j,
            z: translation.k,
        },
        rotation,
        scale,
    )
}

fn inverse_bind_matrix(node: &RenderModelPreviewNode) -> Matrix4 {
    Matrix4 {
        m: [
            node.inverse_bind[0],
            node.inverse_bind[1],
            node.inverse_bind[2],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

/// Hold a unit's hands to its weapon, for the clips the game does.
fn apply_weapon_ik(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
    matrices: &mut [Matrix4],
    inverse_binds: &[Matrix4],
) {
    let clip = state
        .animation
        .selected
        .and_then(|index| data.animations.as_ref()?.get(index))
        .map(|entry| entry.name.as_str());
    let Some(clip) = clip else {
        return;
    };
    for point in &data.preview.weapon_ik {
        let runs = match &data.weapons {
            Some(weapons) => weapons.ik_runs_through(clip, &point.scope),
            None => point.runs_through(clip),
        };
        if runs {
            point.apply(matrices, inverse_binds);
        }
    }
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

    // A muted node holds its bind pose, as the skinned mesh does.
    let muted = variant_muted_nodes(data, state);
    let world = world_pose(nodes, |index, node| match sampled_frames {
        Some(_) if muted[index] => bind_transform(node),
        Some((a, b, blend)) => blend_transforms(a.get(index), b.get(index), blend, node),
        None => (
            quat(node.bind_rotation),
            RealVector3d {
                i: node.bind_translation[0],
                j: node.bind_translation[1],
                k: node.bind_translation[2],
            },
            1.0,
        ),
    });
    let mut matrices: Vec<Matrix4> = world.iter().copied().map(world_matrix).collect();
    if sampled_frames.is_some() {
        let inverse_binds: Vec<_> = nodes.iter().map(inverse_bind_matrix).collect();
        apply_weapon_ik(data, state, &mut matrices, &inverse_binds);
        if let Some(pose) = state.animation.pose.as_ref() {
            let frames = self::sampled_frames(&state.animation, pose.frames.len());
            let markers = &data.preview.gen3.ik_markers;
            super::animation::ik::h4::apply_h4_ik_set(&pose.h4_ik_set, markers, nodes, &inverse_binds, &mut matrices);
            super::animation::ik::reach::apply_reach_ik(&pose.reach_ik, markers, nodes, &inverse_binds, &mut matrices, frames);
        }
    }
    matrices.iter().map(Matrix4::translation).collect()
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
    use crate::app::model_preview::harness::kit::kit_root;
    use crate::core::game::GameId;
    use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
    use blam_tags::render_model::Node;

    fn preview_node(name: &str, parent: i16) -> RenderModelPreviewNode {
        RenderModelPreviewNode {
            name: name.to_owned(),
            parent,
            bind_rotation: [0.0, 0.0, 0.0, 1.0],
            bind_translation: [0.0; 3],
            inverse_bind: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
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
        let Some(root) = kit_root(GameId::Halo4) else { return };
        let path = std::path::Path::new(&root).join("objects/vehicles/human/storm_pelican/storm_pelican.model");
        let variants = super::super::load::read_model_variants(&blam_tags::TagFile::read(path).unwrap());
        let cockpit = variants.iter().find(|variant| variant.name == "m70_gunship_opaque_cockpit").unwrap();
        assert_eq!(cockpit.muted_nodes.len(), 10);
        assert!(cockpit.muted_nodes.iter().any(|name| name == "b_bay_door"));
        assert!(variants.iter().find(|variant| variant.name == "default").unwrap().muted_nodes.is_empty());
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
        let bind = armature_node_positions(&preview_with_nodes(nodes.clone()), &ModelPreviewState::default());
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
        state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(0, vec![frame])));
        let unmuted = armature_node_positions(&data, &state);
        state.selected_variant = Some(0);
        let muted = armature_node_positions(&data, &state);

        assert!((unmuted[1][2] - bind[1][2]).abs() > 0.1, "unmuted, the spine plays its own move");
        assert!((muted[1][0] - bind[1][0] - 0.5).abs() < 1e-4, "it follows its parent");
        assert!((muted[1][2] - bind[1][2]).abs() < 1e-4, "and holds its own bind pose");
        // The skin's translation is world × inverse bind, so the joint is the
        // skin applied to the bind position.
        let rows = animation_skinning_rows(&data, &state).expect("skinning rows");
        let skinned: [f32; 3] = std::array::from_fn(|axis| {
            let row = rows[3 + axis];
            row[0] * bind[1][0] + row[1] * bind[1][1] + row[2] * bind[1][2] + row[3]
        });
        for axis in 0..3 {
            assert!((skinned[axis] - muted[1][axis]).abs() < 1e-4, "{skinned:?} {:?}", muted[1]);
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

        assert!((offset_at(&mut state, 2.5) - 2.0).abs() < 1e-4, "the last frame holds");
        assert!((offset_at(&mut state, 3.25) - 1.25).abs() < 1e-4, "continues from the loop frame");
        assert!((offset_at(&mut state, 5.25) - 1.25).abs() < 1e-4, "loops between frame 1 and the end");

        let restarting = state.animation.pose.as_ref().unwrap().frames.clone();
        state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose::new(0, restarting)));
        assert!((offset_at(&mut state, 3.25) - 0.25).abs() < 1e-4, "a loop frame of 0 restarts");
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
            eprintln!("skipping: no masterchief.model under {}", tags_root.display());
            return;
        }
        let name = "combat:pistol:hp:reload_1";
        let jmad = blam_tags::TagFile::read(folder.join("masterchief.model_animation_graph")).unwrap();
        let render = blam_tags::TagFile::read(folder.join("masterchief.render_model")).ok();
        let animation = Animation::new(&jmad).unwrap();
        let skeleton = Skeleton::from_tag(&jmad);
        let object_space = blam_tags::extract::animation::additional_node_data_is_object_space(&animation);
        let defaults = blam_tags::extract::animation::build_defaults(&skeleton, &jmad, render.as_ref(), object_space);
        let group = (0..animation.len())
            .filter_map(|index| animation.get(index))
            .find(|group| group.name.as_deref() == Some(name))
            .expect("the chief's graph has no pistol reload");
        assert!(!group.object_space_parents.is_empty(), "{name} carries no object-space entries");
        let base = animation
            .overlay_base_pose(&AnimationGraph::from_tag(&jmad), group, &skeleton, &defaults)
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
        let Some(tags) = kit_root(GameId::HaloCe) else { return };
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
        let Some(tags) = kit_root(GameId::HaloCe) else { return };
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
        let Some(tags) = kit_root(GameId::HaloCe) else { return };
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
        assert_eq!(ce_jma_kind(animation), JmaKind::Jmo, "{name} is not an overlay");
        let gbxmodel = object_reference(&object, "model").and_then(|reference| {
            load_referenced_tag_from_source(&source, &reference, "gbxmodel", b"mod2").ok()
        });
        let skeleton = blam_tags::extract::animation::ce_skeleton(&animations, &antr, gbxmodel.as_ref());
        let mut rest = ce_rest_pose(&skeleton, gbxmodel.as_ref());
        rest.iter_mut().for_each(|node| node.rotation = node.rotation.conjugate());
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
                let base = idle.frames[frame % idle.frames.len()][node].rotation.conjugate();
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
        let Some(tags) = kit_root(GameId::Halo2) else { return };
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
        let Some(tags) = kit_root(GameId::Halo2) else { return };
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
        assert_eq!(names.len(), list.len(), "a child's animation should replace its parent's");

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
        let Some(tags) = kit_root(GameId::Halo2) else { return };
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
        let Some(tags) = kit_root(GameId::Halo2) else { return };
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
            compute_variant_selection(&data, index)["helmet"].permutation.clone()
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
}

/// Whether the preview loaded now is the geometry `geometry_id`.
fn is_current_geometry(state: &ModelPreviewState, geometry_id: u64) -> bool {
    matches!(&state.data, Some(Ok(data)) if data.geometry_id == geometry_id)
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
    ) -> bool {
        let kit = &self.model.kits[kit_index];
        let view = &self.views[kit.id];
        let Some(state) = view.caches.model_previews.get(key) else {
            return false;
        };
        if state.animation.requested_list {
            return false;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return false;
        };
        // No skeleton (or one past the GPU bone budget) means nothing could
        // play; don't spend a worker discovering that.
        if data.preview.nodes.is_empty() || data.preview.nodes.len() > MAX_PREVIEW_BONES {
            return false;
        }
        let Some(entry) = kit.entry_for_key(key).cloned() else {
            return false;
        };
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return false;
        };
        // A `.model` names its graph; a Halo CE object has no `.model` and
        // names its `model_animations` itself, the way it names its gbxmodel.
        // Any other object reaches a graph through its `.model`.
        let names_a_graph = entry.group_tag == u32::from_be_bytes(*b"hlmt")
            || is_object_family_group(entry.group_tag, source.game());
        if !names_a_graph {
            return false;
        }
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        let geometry_id = data.geometry_id;
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
                geometry_id,
                result: list_model_animations(&source, &entry),
            },
            move |_| WorkerMessage::ModelAnimationsListed {
                stamp,
                key: panic_key,
                geometry_id,
                result: Err("the animation graph crashed the reader".to_owned()),
            },
        );
        true
    }

    /// Start decoding the animation the panel selected, if it is not the one
    /// already decoded or being decoded. Returns whether it started.
    pub(in crate::app) fn maybe_request_model_animation_decode(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) -> bool {
        let kit = &self.model.kits[kit_index];
        let view = &self.views[kit.id];
        let Some(state) = view.caches.model_previews.get(key) else {
            return false;
        };
        let Some(selected) = state.animation.selected else {
            return false;
        };
        if state.animation.decoding.is_some()
            || state
                .animation
                .pose
                .as_ref()
                .is_some_and(|pose| pose.animation_index == selected)
        {
            return false;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return false;
        };
        if !data
            .animations
            .as_ref()
            .and_then(|animations| animations.get(selected))
            .is_some_and(|entry| entry.playable)
        {
            return false;
        }
        let Some(entry) = kit.entry_for_key(key).cloned() else {
            return false;
        };
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return false;
        };
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        let geometry_id = data.geometry_id;
        let mut layers = Vec::new();
        if let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(key) {
            state.animation.decoding = Some(selected);
            if let Some(Ok(data)) = &state.data
                && let Some(clips) = data.animations.as_deref()
            {
                layers = h2_held_layers(clips, selected, data, state);
            }
            state.animation.error = None;
        }

        let (key, panic_key) = (key.to_owned(), key.to_owned());
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::ModelAnimationDecoded {
                stamp,
                key,
                geometry_id,
                animation_index: selected,
                result: decode_held_animation(&source, &entry, selected, &layers),
            },
            move |_| WorkerMessage::ModelAnimationDecoded {
                stamp,
                key: panic_key,
                geometry_id,
                animation_index: selected,
                result: Err("this animation crashed the decoder".to_owned()),
            },
        );
        true
    }

    pub(in crate::app) fn handle_model_animations_listed(
        &mut self,
        stamp: KitStamp,
        key: String,
        geometry_id: u64,
        result: Result<Vec<PreviewAnimationEntry>, String>,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.model.resolve_stamp(stamp).is_none();
        let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(&key) else {
            return true;
        };
        // Listed for geometry a reload has since replaced, which lists its own.
        if !is_current_geometry(state, geometry_id) {
            return true;
        }
        if stale {
            // Asked for once per preview, so a list dropped as stale has to be
            // asked for again, or it never arrives.
            state.animation.requested_list = false;
            return true;
        }
        match result {
            Ok(entries) => {
                if let Some(Ok(data)) = state.data.as_mut() {
                    // Taken only by the game it is for: H2 reads it below.
                    let ce_clip =
                        data.ce_weapons.as_ref().and_then(|weapons| Some((state.clip_after_load.take()?, weapons)));
                    if let Some((clip, weapons)) = ce_clip {
                        let held = weapons.equippable.iter().find(|weapon| Some(&weapon.path) == state.weapon.as_ref());
                        let class = held.map_or("unarmed", |weapon| weapon.class.as_str());
                        state.animation.selected = super::ce::weapons::clip_after_weapon_change(
                            &entries,
                            &weapons.hidden_clips,
                            &clip,
                            class,
                        );
                    }
                    if let Some(weapons) = data.weapons.as_mut() {
                        let (class, kinds) = weapons
                            .held(state.weapon.as_deref(), state.secondary_weapon.as_deref());
                        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
                        weapons.hidden_clips = weapons.hidden(&names, &class, &kinds);
                        if let Some(clip) = state.clip_after_load.take() {
                            let hidden = &weapons.hidden_clips;
                            let shown = |name: &str| {
                                (0..entries.len()).find(|index| {
                                    entries[*index].name.split('%').next() == Some(name)
                                        && !hidden.contains(index)
                                })
                            };
                            let mode = clip.split(':').next().unwrap_or_default();
                            state.animation.selected = shown(clip.split('%').next().unwrap_or_default())
                                .or_else(|| shown(&format!("{mode}:{class}:idle")));
                        }
                    }
                    // Any other reload: the same clip, if the geometry still has it.
                    if let Some(clip) = state.clip_after_load.take() {
                        state.animation.selected = entries.iter().position(|entry| entry.name == clip);
                    }
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
        geometry_id: u64,
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
        // Decoded for geometry a reload has since replaced: what it holds,
        // and its index, belong to the old clip list, and the reload's own
        // decode may be running for the same index.
        if !is_current_geometry(state, geometry_id) {
            return true;
        }
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
                state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose {
                    loop_frame: decoded.loop_frame,
                    reach_ik: std::sync::Arc::new(decoded.reach_ik),
                    h4_ik_set: std::sync::Arc::new(decoded.h4_ik_set),
                    ..PreviewAnimationPose::new(animation_index, frames)
                }));
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
fn load_graph_chain(source: &TagSource, first: &str) -> Result<Vec<TagFile>, String> {
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
fn inherited_animation_order(animations: &[Animation<'_>]) -> Vec<(usize, usize)> {
    let mut names = std::collections::HashSet::new();
    let mut order = Vec::new();
    for (graph, animation) in animations.iter().enumerate() {
        let start = order.len();
        for group in animation.iter() {
            if group.name.as_ref().is_none_or(|name| !names.contains(name.as_str())) {
                order.push((graph, group.index));
            }
        }
        names.extend(order[start..].iter().filter_map(|&(graph, index)| {
            animations[graph].get(index)?.name.as_deref()
        }));
    }
    order
}

/// A Halo CE object's `model_animations`.
fn load_object_animations(source: &TagSource, object: &TagFile) -> Result<TagFile, String> {
    let reference = object_reference(object, "animation graph")
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
        .map(|node| by_name.get(&node.name).copied().unwrap_or(NodeTransform::IDENTITY))
        .collect()
}

/// [`decode_held_animation`] with no weapon held, for the tests.
#[cfg(test)]
fn decode_model_animation(
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
    layer: &super::weapons::PoseLayer,
) {
    use super::weapons::Arm;
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
fn decode_held_animation(
    source: &TagSource,
    entry: &TagEntry,
    animation_index: usize,
    layers: &[super::weapons::PoseLayer],
) -> Result<DecodedAnimationPose, String> {
    let model = crate::core::source::read_entry(source, entry).map_err(|error| error.to_string())?;
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
        super::animation::blend_screens::lay_blend_screens(jmad, animation, &skeleton, animation_index, &mut pose);
    }
    if !layers.is_empty() {
        let left_arm = super::weapons::left_arm_nodes(jmad);
        for layer in layers {
            lay_over(&mut pose, animation, &skeleton, &left_arm, layer);
        }
    }

    let loop_frame = usize::try_from(group.loop_frame_index).unwrap_or(0);
    let graphs: Vec<&TagFile> = chain[graph..].iter().collect();
    Ok(DecodedAnimationPose {
        reach_ik: super::animation::ik::reach::read_reach_ik_tracks(&graphs, group),
        h4_ik_set: graphs.first().map_or_else(Vec::new, |graph| super::animation::ik::h4::read_h4_ik_set(graph, group.index)),
        ..DecodedAnimationPose::new(&skeleton, &pose, loop_frame)
    })
}

/// A clip with its rotations turned to run forward; see [`decode_ce_animation`].
fn ce_inverted_clip(animation: &CeAnimation<'_>) -> blam_tags::AnimationClip {
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
fn ce_idle_beneath<'a, 'tag>(
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
    let gbxmodel = object_reference(object, "model").and_then(|reference| {
        load_referenced_tag_from_source(source, &reference, "gbxmodel", b"mod2").ok()
    });
    // The extractor's skeleton: a graph that lists no nodes of its own
    // animates its gbxmodel's, when their node list checksums agree.
    let skeleton = blam_tags::extract::animation::ce_skeleton(&animations, &antr, gbxmodel.as_ref());
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
                compose(&idle.frames[frame % idle.frames.len()]).frames.swap_remove(frame)
            });
            blam_tags::Pose { frames: frames.collect() }
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
    for (index, frame) in super::ce::weapons::resting_aim(&antr, name) {
        let Some(aim) = animations.get(index).filter(|aim| frame < aim.frame_count as usize) else {
            continue;
        };
        let aim = ce_inverted_clip(aim);
        for row in &mut pose.frames {
            *row = aim.runtime_overlay_pose(&skeleton, row).frames.swap_remove(frame);
        }
    }
    Ok(DecodedAnimationPose::new(&skeleton, &pose, animation.loop_frame_index as usize))
}

#[cfg(test)]
mod ce_replacement_tests {
    use super::*;
    use crate::app::model_preview::harness::kit::kit_source;
    use crate::core::game::GameId;

    /// With `BLAM_TEST_HCEEK`: an action's clip plays over the idle, so the
    /// nodes it leaves alone stand where the idle puts them, not at bind.
    #[test]
    fn a_ce_replacement_clip_leaves_its_unanimated_nodes_to_the_idle() {
        let Some(source) = kit_source(GameId::HaloCe) else { return };
        let path = "characters\\cyborg\\cyborg";
        let object = load_referenced_tag_from_source(&source, path, "biped", b"bipd").unwrap();
        let antr = load_object_animations(&source, &object).unwrap();
        let animations = CeAnimations::new(&antr);
        let index = |name: &str| {
            animations.iter().position(|clip| clip.name.as_deref() == Some(name)).unwrap()
        };
        let pose = |name| decode_ce_animation(&source, &object, index(name)).unwrap();
        let melee = pose("stand pistol b melee");
        let idle = pose("stand pistol idle");
        let flags = |name: &str| animations.get(index(name)).unwrap().decode().node_flags.unwrap();
        let (clip, aim, look) =
            (flags("stand pistol b melee"), flags("stand pistol aim-still"), flags("stand look"));
        // The idle also takes the aiming screens, which an action shuts off.
        let left_alone: Vec<_> = (0..melee.skeleton_names.len())
            .filter(|node| {
                ![&clip, &aim, &look].iter().any(|flags| flags.animated_rotation.bit(*node))
            })
            .collect();
        assert!(!left_alone.is_empty());
        for node in left_alone {
            assert_eq!(melee.frames[8][node].rotation, idle.frames[8][node].rotation);
        }
    }
}
