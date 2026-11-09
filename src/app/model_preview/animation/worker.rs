//! The background animation worker that decodes a preview's animations.

use super::*;

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
        if let Some(state) = self.views[self.model.kits[kit_index].id]
            .caches
            .model_previews
            .get_mut(key)
        {
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
        if let Some(state) = self.views[self.model.kits[kit_index].id]
            .caches
            .model_previews
            .get_mut(key)
        {
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
        let Some(state) = self.views[self.model.kits[kit_index].id]
            .caches
            .model_previews
            .get_mut(&key)
        else {
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
                    let ce_clip = data
                        .ce_weapons
                        .as_ref()
                        .and_then(|weapons| Some((state.clip_after_load.take()?, weapons)));
                    if let Some((clip, weapons)) = ce_clip {
                        let held = weapons
                            .equippable
                            .iter()
                            .find(|weapon| Some(&weapon.path) == state.weapon.as_ref());
                        let class = held.map_or("unarmed", |weapon| weapon.class.as_str());
                        state.animation.selected =
                            super::super::ce::weapons::clip_after_weapon_change(
                                &entries,
                                &weapons.hidden_clips,
                                &clip,
                                class,
                            );
                    }
                    if let Some(weapons) = data.weapons.as_mut() {
                        let (class, kinds) = weapons
                            .held(state.weapon.as_deref(), state.secondary_weapon.as_deref());
                        let names: Vec<_> =
                            entries.iter().map(|entry| entry.name.as_str()).collect();
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
                            state.animation.selected =
                                shown(clip.split('%').next().unwrap_or_default())
                                    .or_else(|| shown(&format!("{mode}:{class}:idle")));
                        }
                    }
                    // Any other reload: the same clip, if the geometry still has it.
                    if let Some(clip) = state.clip_after_load.take() {
                        state.animation.selected =
                            entries.iter().position(|entry| entry.name == clip);
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
        let Some(state) = self.views[self.model.kits[kit_index].id]
            .caches
            .model_previews
            .get_mut(&key)
        else {
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
