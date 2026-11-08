//! The preview and overlay workers: the requests the UI starts and the handlers for what they send back.

use super::*;

impl Baboon {
    /// Start building a loaded `.model` preview's collision/physics overlays
    /// on a worker, once per load.
    ///
    /// Building them inline froze the frame: every toggle re-read the
    /// collision and physics tags, re-walked the collision BSP, and
    /// re-tessellated the shapes — on the UI thread, in both directions.
    /// Built here instead, the merged geometry sits in memory for the
    /// document's lifetime and the toggles become draw-time filters.
    /// Idempotent and cheap to call every frame, like the texture request
    /// beside it.
    pub(in crate::app) fn maybe_request_model_overlays(
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
        if state.overlays_loaded || state.overlays_pending {
            return false;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return false;
        };
        let geometry_id = data.geometry_id;
        let Some(entry) = kit.entry_for_key(key).cloned() else {
            return false;
        };
        if entry.group_tag != u32::from_be_bytes(*b"hlmt")
            && !is_object_family_group(entry.group_tag, kit.source.as_ref().and_then(|source| source.game))
        {
            return false;
        }
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return false;
        };
        // Read as the base load reads it, unsaved edits and all.
        let edits = super::super::load::snapshot_edits(kit.parsed_tags.get(key));
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        if let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(key) {
            state.overlays_pending = true;
        }

        // Through `spawn_worker` like every preview worker: a panicking tag
        // must still send its message or the pending flag sticks and the
        // overlays never arrive.
        let (key, panic_key) = (key.to_owned(), key.to_owned());
        let build = move || -> Option<(Overlay, Overlay)> {
                let model = super::super::load::read_opened_tag(&source, &entry, edits).ok()?;
                // The overlays are a load of their own.
                let reader = super::super::load::TagReader::new(&source);
                if blam_tags::game::Game::of(&model) == blam_tags::game::Game::Halo1
                    && is_object_family_group(model.header.group_tag, source.game())
                {
                    return Some((halo1_object_collision_overlay(&model, &reader), Ok(None)));
                }
                let model = super::super::load::preview_model_tag(&source, &model).ok()?.unwrap_or(model);
                // Classic models only: a Campaign Evolved hlmt previews
                // through Unreal geometry and hides the overlay toggles.
                model.root().read_tag_ref_with_group("render model")?;
                Some((
                    hlmt_collision_overlay(&model, &reader),
                    hlmt_physics_overlay(&model, &reader),
                ))
        };
        spawn_worker(
            &self.tx,
            ctx,
            move || {
                let (collision, physics) = build().unwrap_or((Ok(None), Ok(None)));
                let mut notes = Vec::new();
                let mut kept = |overlay: Overlay| {
                    overlay.unwrap_or_else(|error| {
                        notes.push(error);
                        None
                    })
                };
                let (collision, physics) = (kept(collision), kept(physics));
                WorkerMessage::ModelOverlaysBuilt { stamp, key, geometry_id, collision, physics, notes }
            },
            move |_| WorkerMessage::ModelOverlaysBuilt {
                stamp,
                key: panic_key,
                geometry_id,
                collision: None,
                physics: None,
                notes: vec!["the collision and physics overlays: the builder crashed".to_owned()],
            },
        );
        true
    }

    /// Merge a worker's overlay geometry into the preview it was built for.
    pub(in crate::app) fn handle_model_overlays_built(
        &mut self,
        stamp: KitStamp,
        key: String,
        geometry_id: u64,
        collision: Option<RenderModelPreview>,
        physics: Option<RenderModelPreview>,
        notes: Vec<String>,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.model.resolve_stamp(stamp).is_none();
        let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(&key) else {
            return true;
        };
        // The in-flight marker is cleared before the staleness check, so a
        // result dropped for a generation bump still lets the overlays be
        // asked for again rather than waited on for good.
        state.overlays_pending = false;
        if stale {
            return true;
        }
        {
            let Some(Ok(data)) = state.data.as_mut() else {
                return true;
            };
            if data.geometry_id != geometry_id {
                // The preview reloaded while this built; the reload re-armed
                // the request, so a fresh build is already on its way.
                return true;
            }
            state.overlays_loaded = true;
            data.notes.extend(notes);
            if collision.is_none() && physics.is_none() {
                return false;
            }
            let mut merged = (*data.preview).clone();
            if let Some(overlay) = &collision {
                merge_preview_append(&mut merged, overlay);
            }
            if let Some(overlay) = &physics {
                merge_preview_append(&mut merged, overlay);
            }
            data.preview = Arc::new(merged);
            data.mesh_id = NEXT_MODEL_GEOMETRY_ID.fetch_add(1, Ordering::Relaxed);
        }
        // The layer filter also consults region selection, so collision-only
        // regions need entries: what the selected variant gives them, so the
        // variant still reads as chosen. Shared names keep the render model's.
        if let Some(Ok(data)) = &state.data {
            for (region, selection) in compute_variant_selection(data, state.selected_variant) {
                state.region_selections.entry(region).or_insert(selection);
            }
        }
        // Textures are keyed by `textures_id`, which the merge leaves alone:
        // a resolve in flight still lands, and resolved ones stay uploaded.
        false
    }
}
