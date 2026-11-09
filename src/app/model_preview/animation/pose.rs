//! An animation pose: node transforms, decoded and preview poses, and the blends between them.

#[cfg(test)]
use super::super::super::{TagEntry, TagSource};
use super::super::RenderModelPreviewNode;
#[cfg(test)]
use super::super::{ModelPreviewData, ModelPreviewState};
#[cfg(test)]
use super::decode::decode_held_animation;
#[cfg(test)]
use super::graph::list_model_animations;
#[cfg(test)]
use super::skinning::h2_held_layers;
use blam_tags::math::{RealQuaternion, RealVector3d};
use blam_tags::{NodeTransform, Skeleton};

/// Halo's animation clock. No tag carries a rate; the engine (and the JMA
/// header the extractor writes) is a fixed 30 Hz.
pub(crate) const ANIMATION_FRAME_RATE: f32 = 30.0;

/// One node's transform at one frame, parent-local.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PreviewNodeTransform {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    pub scale: f32,
}

impl PreviewNodeTransform {
    pub(super) fn from_node_transform(transform: &NodeTransform) -> Self {
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
    pub reach_ik: Vec<super::super::animation::ik::reach::ReachIkTrack>,
    /// Halo 4: the clip's IK set, solved before the chains.
    pub h4_ik_set: Vec<super::super::animation::ik::h4::H4IkPoint>,
}

impl DecodedAnimationPose {
    pub(super) fn new(skeleton: &Skeleton, pose: &blam_tags::Pose, loop_frame: usize) -> Self {
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
    pub reach_ik: std::sync::Arc<Vec<super::super::animation::ik::reach::ReachIkTrack>>,
    /// Halo 4's IK set for the clip, solved before the chains.
    pub h4_ik_set: std::sync::Arc<Vec<super::super::animation::ik::h4::H4IkPoint>>,
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

/// Hold a preview at `H2_SHOT_FRAME` of the clip named `clip`, for the headless shot.
#[cfg(test)]
pub(in crate::app::model_preview) fn pose_for_shot(
    source: &TagSource,
    entry: &TagEntry,
    data: &mut ModelPreviewData,
    state: &mut ModelPreviewState,
    clip: &str,
) {
    let list = list_model_animations(source, entry).unwrap();
    let index = list
        .iter()
        .position(|animation| animation.name == clip)
        .unwrap_or_else(|| {
            let names: Vec<&str> = list
                .iter()
                .map(|animation| animation.name.as_str())
                .collect();
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
            let source = decoded
                .skeleton_names
                .iter()
                .position(|name| *name == node.name);
            source
                .and_then(|source| decoded.frames[at].get(source).copied())
                .unwrap_or(PreviewNodeTransform {
                    rotation: node.bind_rotation,
                    translation: node.bind_translation,
                    scale: 1.0,
                })
        })
        .collect();
    data.animations = Some(std::sync::Arc::new(list));
    state.animation.selected = Some(index);
    // The one frame held is the clip's `at`th; its IK reads that frame.
    let reach_ik = decoded
        .reach_ik
        .into_iter()
        .map(|track| super::super::animation::ik::reach::ReachIkTrack {
            effector_frames: track.effector_frames.get(at).copied().into_iter().collect(),
            weights: track.weights.get(at).copied().into_iter().collect(),
            poles: track
                .poles
                .map(|poles| poles.get(at).copied().into_iter().collect()),
            ..track
        })
        .collect();
    state.animation.pose = Some(std::sync::Arc::new(PreviewAnimationPose {
        reach_ik: std::sync::Arc::new(reach_ik),
        h4_ik_set: std::sync::Arc::new(decoded.h4_ik_set),
        ..PreviewAnimationPose::new(index, vec![frame])
    }));
}

/// Blend one node between two frames — nlerp on the shorter arc for the
/// rotation, lerp for translation and scale — falling back to the node's own
/// bind pose when the animation carries no entry for it.
pub(super) fn blend_transforms(
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

pub(super) fn quat(values: [f32; 4]) -> RealQuaternion {
    RealQuaternion {
        i: values[0],
        j: values[1],
        k: values[2],
        w: values[3],
    }
}
