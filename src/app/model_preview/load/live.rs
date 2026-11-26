//! The Material Editor's edits, drawn as they are made: when a shader the
//! preview's editor holds changes, its materials alone are resolved again
//! from the document, on a worker, and swapped in. A load of the whole model
//! (seconds, for a Spartan) is never what an edit costs.
//!
//! The textures that come back share every image the materials before them
//! had (the resolve's caches live across these resolves, and the materials
//! not edited are the same ones), so the renderer uploads only what the edit
//! changed (`TexturePool`).

use super::super::super::Baboon;
use super::super::material_editor::editor_materials;
use super::super::materials::{MaterialTextures, resolve_caches, resolve_model_materials};
use super::super::{KitStamp, WorkerMessage, egui, spawn_worker};
use std::sync::{Arc, Mutex};

impl Baboon {
    /// Start resolving again the materials of a shader the preview of `key`
    /// holds, if one has changed since its materials were resolved. One at a
    /// time; cheap to call every frame.
    pub(in crate::app) fn maybe_request_material_resolve(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) -> bool {
        let kit = &self.model.kits[kit_index];
        let Some(state) = self.views[kit.id].caches.model_previews.get(key) else {
            return false;
        };
        let live = &state.material_live;
        if live.in_flight || live.documents.is_empty() || state.textures_pending || state.needs_preview_load(key) {
            return false;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return false;
        };
        let Some(textures) = data.textures.clone() else {
            return false;
        };
        // The first held document whose document is not what was drawn.
        let mut clean = Vec::new();
        let mut changed = None;
        for (material, document) in &live.documents {
            let Ok(document) = document else { continue };
            let held = kit.held_documents.get(document).is_some_and(|holders| holders.contains(key));
            if !held || kit.loading_tags.contains(document) {
                continue;
            }
            let now = kit.parsed_tags.get(document);
            let stamp = now.map(|document| document.content_stamp());
            match live.drawn.get(document) {
                Some(drawn) if *drawn == stamp => {}
                // Drawn from its file, which a clean document is.
                None if now.is_some_and(|document| !document.dirty.is_set()) => clean.push((document.clone(), stamp)),
                _ => {
                    if changed.is_none() {
                        changed = Some((material.clone(), document.clone(), stamp));
                    }
                }
            }
        }
        let Some((material, document, stamp)) = changed else {
            if !clean.is_empty()
                && let Some(state) = self.views[kit.id].caches.model_previews.get_mut(key)
            {
                state.material_live.drawn.extend(clean);
            }
            return false;
        };
        let Some(indices) = editor_materials(&data.preview)
            .into_iter()
            .find(|listed| listed.key == material)
            .map(|listed| listed.indices)
        else {
            return false;
        };
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return false;
        };
        // A clean document is its file; edits go to the worker as bytes.
        let bytes = kit
            .parsed_tags
            .get(&document)
            .filter(|document| document.dirty.is_set())
            .and_then(|document| document.tag.write_to_bytes().ok());
        let group = material.group;
        let preview = Arc::clone(&data.preview);
        let textures_id = data.textures_id;
        let game_shaders = data.textures_game_shaders;
        let reads = state.reads.clone();
        let stamp_of_kit = KitStamp { kit: kit.id, generation: kit.generation };
        let kit_id = kit.id;
        let state = self.views[kit_id].caches.model_previews.get_mut(key).expect("checked above");
        let caches = match &state.material_live.caches {
            Some((basis, caches)) if *basis == data_basis(state) => Arc::clone(caches),
            _ => {
                let caches = Arc::new(Mutex::new(resolve_caches(&source)));
                state.material_live.caches = Some((data_basis(state), Arc::clone(&caches)));
                caches
            }
        };
        state.material_live.in_flight = true;
        let (key, panic_key, panic_document) = (key.to_owned(), key.to_owned(), document.clone());
        spawn_worker(
            &self.tx,
            ctx,
            move || {
                let _recording = crate::core::source::reads::record_reads(&reads);
                let definitions_root = match &source {
                    crate::core::source::TagSource::LooseFolder { definitions_root, .. } => definitions_root.clone(),
                    _ => crate::core::bundled::locate_definitions_root(),
                };
                let shader = bytes.and_then(|bytes| {
                    crate::core::source::read_tag_from_bytes(&bytes, source.game(), Some(&definitions_root), group)
                        .ok()
                        .map(Arc::new)
                });
                let mut caches = caches.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let textures = resolve_model_materials(
                    &source,
                    &preview.materials,
                    &textures,
                    &indices,
                    shader,
                    game_shaders,
                    &mut caches,
                );
                WorkerMessage::MaterialsResolved {
                    stamp: stamp_of_kit,
                    key,
                    textures_id,
                    document,
                    drawn: stamp,
                    textures: Some(textures),
                }
            },
            move |_| WorkerMessage::MaterialsResolved {
                stamp: stamp_of_kit,
                key: panic_key,
                textures_id,
                document: panic_document,
                drawn: stamp,
                textures: None,
            },
        );
        true
    }

    /// Applies `WorkerMessage::MaterialsResolved`: the materials of
    /// `document`'s shader, resolved from it at `drawn`, in place of those
    /// drawn, unless the model loaded again meanwhile.
    pub(in crate::app) fn handle_materials_resolved(
        &mut self,
        stamp: KitStamp,
        key: String,
        textures_id: u64,
        document: String,
        drawn: Option<(u64, u64)>,
        textures: Option<Vec<MaterialTextures>>,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.model.resolve_stamp(stamp).is_none();
        let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(&key) else {
            return true;
        };
        state.material_live.in_flight = false;
        let Some(Ok(data)) = state.data.as_mut() else {
            return true;
        };
        if stale || data.textures_id != textures_id {
            return true;
        }
        // A resolve that failed outright leaves the materials as they were,
        // noted as drawn so it isn't tried again until the shader changes.
        if let Some(textures) = textures {
            let game_shaders = data.textures_game_shaders;
            data.set_textures(textures, game_shaders);
        }
        state.material_live.drawn.insert(document, drawn);
        true
    }
}

/// What the editor's caches belong to: the load the drawn textures came
/// from. A new load starts them over.
fn data_basis(state: &super::super::ModelPreviewState) -> u64 {
    match &state.data {
        Some(Ok(data)) => data.geometry_id,
        _ => 0,
    }
}
