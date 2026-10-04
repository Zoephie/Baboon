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
            let frame_count = pose.frames.len();
            let mut position = (state.animation.time * ANIMATION_FRAME_RATE).max(0.0);
            if state.animation.looped && frame_count > 1 {
                position %= frame_count as f32;
            } else {
                position = position.min((frame_count - 1) as f32);
            }
            let a = (position.floor() as usize).min(frame_count - 1);
            let b = if state.animation.looped {
                (a + 1) % frame_count
            } else {
                (a + 1).min(frame_count - 1)
            };
            Some((&pose.frames[a], &pose.frames[b], position - a as f32))
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
        let Some(kit_index) = self.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.resolve_stamp(stamp).is_none();
        let Some(state) = self.kits[kit_index].model_previews.get_mut(&key) else {
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
        let Some(kit_index) = self.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.resolve_stamp(stamp).is_none();
        let Some(state) = self.kits[kit_index].model_previews.get_mut(&key) else {
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
    let skeleton = Skeleton::from_tag(&antr);
    let gbxmodel = halo1_object_reference(object, "model").and_then(|reference| {
        load_referenced_tag_from_source(source, &reference, "gbxmodel", b"mod2").ok()
    });
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
