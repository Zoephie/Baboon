//! Animation playback for the `.model` preview: listing the linked graph's
//! animations, decoding a selection into per-node frames off the UI thread,
//! and sampling the current time into GPU skinning matrices. Geometry,
//! rendering, and the panel presentation belong elsewhere.

use super::*;
use blam_tags::math::Matrix4;
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

/// A decoded animation mapped onto `RenderModelPreview::nodes` order, ready
/// to sample every draw frame.
#[derive(Debug, Clone)]
pub(crate) struct PreviewAnimationPose {
    /// Which list entry this is, so a stale selection change re-decodes.
    pub animation_index: usize,
    /// `frames[frame][preview_node]`, parent-local.
    pub frames: Vec<Vec<PreviewNodeTransform>>,
}

/// Cross-frame playback state, one per previewed document.
pub(crate) struct PreviewAnimationPlayback {
    pub selected: Option<usize>,
    pub playing: bool,
    pub looped: bool,
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
}

impl Default for PreviewAnimationPlayback {
    fn default() -> Self {
        Self {
            selected: None,
            playing: false,
            looped: true,
            speed: 1.0,
            time: 0.0,
            pose: None,
            decoding: None,
            error: None,
            requested_list: false,
            filter: String::new(),
        }
    }
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
    let pose = playback.pose.as_ref()?;
    let nodes = &data.preview.nodes;
    if nodes.is_empty() || nodes.len() > MAX_PREVIEW_BONES || pose.frames.is_empty() {
        return None;
    }

    let frame_count = pose.frames.len();
    let mut frame_position = (playback.time * ANIMATION_FRAME_RATE).max(0.0);
    if playback.looped && frame_count > 1 {
        frame_position %= frame_count as f32;
    } else {
        frame_position = frame_position.min((frame_count - 1) as f32);
    }
    let frame_a = (frame_position.floor() as usize).min(frame_count - 1);
    let frame_b = if playback.looped {
        (frame_a + 1) % frame_count
    } else {
        (frame_a + 1).min(frame_count - 1)
    };
    let blend = frame_position - frame_a as f32;
    let (frame_a, frame_b) = (&pose.frames[frame_a], &pose.frames[frame_b]);

    let mut world: Vec<(RealQuaternion, RealVector3d, f32)> = Vec::with_capacity(nodes.len());
    let mut rows: Vec<[f32; 4]> = Vec::with_capacity(nodes.len() * 3);
    for (index, node) in nodes.iter().enumerate() {
        let local = blend_transforms(
            frame_a.get(index),
            frame_b.get(index),
            blend,
            node,
        );
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
#[path = "../tests/animation_preview.rs"]
mod tests;

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
        let kit = &self.kits[kit_index];
        let Some(state) = kit.model_previews.get(key) else {
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
        if entry.group_tag != u32::from_be_bytes(*b"hlmt") {
            return;
        }
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return;
        };
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        if let Some(state) = self.kits[kit_index].model_previews.get_mut(key) {
            state.animation.requested_list = true;
        }

        let (tx, ctx, key) = (self.tx.clone(), ctx.clone(), key.to_owned());
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                list_model_animations(&source, &entry)
            }))
            .unwrap_or_else(|_| Err("the animation graph crashed the reader".to_owned()));
            let _ = tx.send(WorkerMessage::ModelAnimationsListed { stamp, key, result });
            ctx.request_repaint();
        });
    }

    /// Start decoding the animation the panel selected, if it is not the one
    /// already decoded or being decoded.
    pub(in crate::app) fn maybe_request_model_animation_decode(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) {
        let kit = &self.kits[kit_index];
        let Some(state) = kit.model_previews.get(key) else {
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
        if let Some(state) = self.kits[kit_index].model_previews.get_mut(key) {
            state.animation.decoding = Some(selected);
            state.animation.error = None;
        }

        let (tx, ctx, key) = (self.tx.clone(), ctx.clone(), key.to_owned());
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                decode_model_animation(&source, &entry, selected)
            }))
            .unwrap_or_else(|_| Err("this animation crashed the decoder".to_owned()));
            let _ = tx.send(WorkerMessage::ModelAnimationDecoded {
                stamp,
                key,
                animation_index: selected,
                result,
            });
            ctx.request_repaint();
        });
    }

    pub(in crate::app) fn handle_model_animations_listed(
        &mut self,
        stamp: KitStamp,
        key: String,
        result: Result<Vec<PreviewAnimationEntry>, String>,
    ) -> bool {
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            return true;
        };
        let Some(state) = self.kits[kit_index].model_previews.get_mut(&key) else {
            return true;
        };
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
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            return true;
        };
        let Some(state) = self.kits[kit_index].model_previews.get_mut(&key) else {
            return true;
        };
        if state.animation.decoding == Some(animation_index) {
            state.animation.decoding = None;
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
                    animation_index,
                    frames,
                }));
                state.animation.time = 0.0;
                state.animation.playing = true;
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
    let model = crate::source::read_entry(source, entry).map_err(|error| error.to_string())?;
    // Classic models only: Campaign Evolved previews reconstruct from Unreal
    // and its animations live in Unreal assets; H1/H2 geometry carries no
    // per-vertex skinning through this pipeline.
    if blam_tags::game::Game::of(&model) != blam_tags::game::Game::Halo3 {
        return Err("Animation playback needs a Halo 3-family model.".to_owned());
    }
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

/// Worker half of the decode request: the extractor's exact composition
/// recipe (`write_group_jma`), minus the file write.
fn decode_model_animation(
    source: &TagSource,
    entry: &TagEntry,
    animation_index: usize,
) -> Result<DecodedAnimationPose, String> {
    let model = crate::source::read_entry(source, entry).map_err(|error| error.to_string())?;
    let root = model.root();
    let jmad_ref =
        tag_ref_path(&root, "animation").ok_or("This model references no animation graph.")?;
    let jmad =
        load_referenced_tag_from_source(source, &jmad_ref, "model_animation_graph", b"jmad")
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

    Ok(DecodedAnimationPose {
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
    })
}
