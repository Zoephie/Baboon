//! The preview and overlay workers: the requests the UI starts and the handlers for what they send back.

use super::super::super::Baboon;
use super::super::{
    KitStamp, ModelPreviewData, ModelPreviewState, ModelTagPanelTab, NEXT_MODEL_GEOMETRY_ID,
    PreviewAnimationPlayback, RenderModelPreview, TagEntry, TagFile, TagSource, WorkerMessage,
    compute_variant_selection, default_variant_index, egui, is_object_family_group, read_entry,
    reset_model_preview_selection, spawn_worker,
};
use super::load_model_preview;
use super::merge::merge_preview_append;
use super::overlays::{Overlay, hlmt_collision_overlay, hlmt_physics_overlay};
use std::sync::Arc;
use std::sync::atomic::Ordering;

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
            && !is_object_family_group(
                entry.group_tag,
                kit.source.as_ref().and_then(|source| source.game),
            )
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
        if let Some(state) = self.views[self.model.kits[kit_index].id]
            .caches
            .model_previews
            .get_mut(key)
        {
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
            if let Some(overlays) = super::games::own_object_overlays(&model, &reader) {
                return Some(overlays);
            }
            let model = super::super::load::preview_model_tag(&source, &model)
                .ok()?
                .unwrap_or(model);
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
                WorkerMessage::ModelOverlaysBuilt {
                    stamp,
                    key,
                    geometry_id,
                    collision,
                    physics,
                    notes,
                }
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
        let Some(state) = self.views[self.model.kits[kit_index].id]
            .caches
            .model_previews
            .get_mut(&key)
        else {
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

/// The load-affecting slice of [`ModelPreviewState`], snapshotted before the
/// parse so the loader never holds the mutable state alongside it.
#[derive(Clone)]
pub(in crate::app::model_preview) struct PreviewLoadSettings {
    pub(in crate::app::model_preview) campaign_evolved:
        super::super::campaign_evolved::CampaignEvolvedLoad,
    pub(in crate::app::model_preview) scenario_selection: std::collections::BTreeSet<usize>,
    /// The weapon a unit holds, by path.
    pub(in crate::app::model_preview) weapon: Option<String>,
    /// Halo CE's.
    pub(in crate::app::model_preview) ce: super::super::ce::weapons::CeLoadSettings,
    /// Halo 2 and later units: what the left hand holds, by path.
    pub(in crate::app::model_preview) secondary_weapon: Option<String>,
    /// Halo 2 and later units: their weapons, when an earlier load found them.
    pub(in crate::app::model_preview) equippable: Option<Vec<super::super::weapons::Equippable>>,
    /// What the load reads for: [`super::TagReader::game_shaders`].
    pub(in crate::app::model_preview) game_shaders: bool,
}

/// A load of everything, Game Shaders' share too, as a test reads it.
impl Default for PreviewLoadSettings {
    fn default() -> Self {
        Self {
            campaign_evolved: Default::default(),
            scenario_selection: Default::default(),
            weapon: None,
            ce: Default::default(),
            secondary_weapon: None,
            equippable: None,
            game_shaders: true,
        }
    }
}

impl PreviewLoadSettings {
    fn of(state: &ModelPreviewState) -> Self {
        Self {
            campaign_evolved: super::super::campaign_evolved::CampaignEvolvedLoad::of(
                &state.campaign_evolved,
            ),
            scenario_selection: state.scenario_bsp_selection.clone(),
            weapon: state.weapon.clone(),
            secondary_weapon: state.secondary_weapon.clone(),
            equippable: match &state.data {
                Some(Ok(data)) => data
                    .weapons
                    .as_ref()
                    .and_then(|weapons| weapons.equippable.clone()),
                _ => None,
            },
            ce: super::super::ce::weapons::CeLoadSettings::of(state.data.as_ref()),
            game_shaders: state.game_shaders,
        }
    }
}

pub(super) static NEXT_MODEL_PREVIEW_LOAD_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

impl Baboon {
    /// Start the expensive base geometry + variant build after the loading
    /// cards have had a frame to appear. Re-reading the tag on the worker is
    /// intentional: `TagFile` is not cloneable, and moving the open document
    /// away would make its Fields tab unavailable while the preview builds.
    pub(in crate::app) fn maybe_request_model_preview(
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
        if state.active_tab != ModelTagPanelTab::ModelPreview {
            return false;
        }
        let desired_matches = state.loaded_key.as_deref() == Some(key)
            && state.campaign_evolved.loaded_as_set()
            && state.loaded_scenario_selection == state.scenario_bsp_selection
            && state.loaded_weapon == state.weapon
            && state.loaded_secondary_weapon == state.secondary_weapon
            && state.loaded_game_shaders == state.game_shaders;
        if desired_matches && (state.data.is_some() || state.preview_load_id.is_some()) {
            return false;
        }

        let Some(entry) = kit.entry_for_key(key).cloned() else {
            return false;
        };
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return false;
        };
        let edits = snapshot_edits(kit.parsed_tags.get(key));
        let names = kit.names.clone();
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        let settings = PreviewLoadSettings::of(state);
        let request_id =
            NEXT_MODEL_PREVIEW_LOAD_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let state = self.views[self.model.kits[kit_index].id]
            .caches
            .model_previews
            .get_mut(key)
            .expect("preview state checked above");
        state.loaded_key = Some(key.to_owned());
        settings
            .campaign_evolved
            .loading_into(&mut state.campaign_evolved);
        state.loaded_scenario_selection = settings.scenario_selection.clone();
        state.loaded_weapon = settings.weapon.clone();
        state.loaded_secondary_weapon = settings.secondary_weapon.clone();
        state.loaded_game_shaders = settings.game_shaders;
        state.preview_load_id = Some(request_id);
        state.remember_selection();
        state.data = None;
        state.render_model_path = None;
        // A fresh base load orphans any overlay, material, or animation work
        // keyed to the old geometry. Their request hooks re-arm after this
        // worker result is accepted.
        state.overlays_pending = false;
        state.overlays_loaded = false;
        state.textures_pending = false;
        state.weapons_pending = false;
        // How the clip plays outlives the reload; the clip itself is picked
        // again by name once the new geometry lists its own.
        let kept = std::mem::take(&mut state.animation);
        state.animation = PreviewAnimationPlayback {
            playing: kept.playing,
            stopped: kept.stopped,
            looped: kept.looped,
            interpolate: kept.interpolate,
            speed: kept.speed,
            time: kept.time,
            filter: kept.filter,
            ..PreviewAnimationPlayback::default()
        };

        let (worker_key, panic_key) = (key.to_owned(), key.to_owned());
        let load = move || -> Result<_, String> {
            let model_tag = read_opened_tag(&source, &entry, edits)?;
            load_model_preview(&model_tag, &entry, &names, Some(&source), &settings)
        };
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::ModelPreviewLoaded {
                stamp,
                key: worker_key,
                request_id,
                result: load(),
            },
            move |_| WorkerMessage::ModelPreviewLoaded {
                stamp,
                key: panic_key,
                request_id,
                result: Err("Render model preview crashed while parsing this tag.".to_owned()),
            },
        );
        true
    }

    pub(in crate::app) fn handle_model_preview_loaded(
        &mut self,
        stamp: KitStamp,
        key: String,
        request_id: u64,
        result: Result<ModelPreviewData, String>,
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
        if state.preview_load_id != Some(request_id) {
            return true;
        }
        // The request is answered before the staleness check, so a result
        // dropped for a generation bump still lets the pane ask again, rather
        // than sit on its loading shells, repainting every frame, with a
        // request "in flight" that never lands.
        state.preview_load_id = None;
        if stale {
            return true;
        }
        if let Ok(data) = &result {
            state.render_model_path = Some(data.render_model_path.clone());
            // The variant chosen before the reload, by name, else the canonical
            // one (named `default`, else the first), so both cards arrive in
            // one consistent state.
            let remembered = state.variant_after_load.take();
            let remembered = remembered.and_then(|name| {
                data.variants
                    .iter()
                    .position(|variant| variant.name == name)
            });
            let variant = remembered.or_else(|| default_variant_index(data));
            reset_model_preview_selection(state, data, variant);
            // A weapon change only swaps what the unit holds; Game Shaders
            // only what it reads.
            if let Some((geometry_id, regions)) = state.reload_carry.take() {
                for (region, selection) in regions {
                    if let Some(kept) = state.region_selections.get_mut(&region) {
                        *kept = selection;
                    }
                }
                state
                    .ce
                    .carry_function_inputs(geometry_id, data.geometry_id);
                // Halo 2's, by name. A name only the old weapon had is
                // carried too, and reads nothing.
                let carried: Vec<_> = state
                    .function_inputs
                    .iter()
                    .filter(|((id, _), _)| *id == geometry_id)
                    .map(|((_, name), value)| ((data.geometry_id, name.clone()), *value))
                    .collect();
                state.function_inputs.extend(carried);
            }
        }
        state.data = Some(result);
        false
    }
}

impl Baboon {
    /// List a unit's weapons on a worker, once its picker asked
    /// (`ModelPreviewState::list_weapons`): listing reads every weapon in the
    /// kit, which no load waits for. Cheap to call every frame.
    pub(in crate::app) fn maybe_request_unit_weapons(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) -> bool {
        let kit = &self.model.kits[kit_index];
        let Some(state) = self.views[kit.id].caches.model_previews.get(key) else {
            return false;
        };
        if !state.list_weapons || state.weapons_pending {
            return false;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return false;
        };
        let geometry_id = data.geometry_id;
        let (Some(entry), Some(source)) = (
            kit.entry_for_key(key).cloned(),
            kit.source.as_ref().map(|source| source.source.clone()),
        ) else {
            return false;
        };
        // Read as the base load reads it, unsaved edits and all.
        let edits = snapshot_edits(kit.parsed_tags.get(key));
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        if let Some(state) = self.views[kit.id].caches.model_previews.get_mut(key) {
            state.list_weapons = false;
            state.weapons_pending = true;
        }
        let (key, panic_key) = (key.to_owned(), key.to_owned());
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::UnitWeaponsListed {
                stamp,
                key,
                geometry_id,
                listed: Some(super::super::weapons::list_weapons(&source, &entry, edits)),
            },
            // The picker stops waiting, and lists nothing.
            move |_| WorkerMessage::UnitWeaponsListed {
                stamp,
                key: panic_key,
                geometry_id,
                listed: None,
            },
        );
        true
    }

    /// Put a worker's weapon list in the preview it was listed for: none,
    /// when the worker crashed.
    pub(in crate::app) fn handle_unit_weapons_listed(
        &mut self,
        stamp: KitStamp,
        key: String,
        geometry_id: u64,
        listed: Option<super::super::weapons::ListedWeapons>,
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
        // Answered before the staleness check, so the picker can ask again.
        state.weapons_pending = false;
        if stale {
            return true;
        }
        let Some(Ok(data)) = state.data.as_mut() else {
            return true;
        };
        // The preview reloaded while this ran; its picker asks again.
        if data.geometry_id != geometry_id {
            return true;
        }
        use super::super::weapons::ListedWeapons;
        match listed {
            Some(ListedWeapons::Unit(listed)) => {
                if let Some(weapons) = data.weapons.as_mut() {
                    weapons.equippable = Some(listed);
                }
            }
            Some(ListedWeapons::HaloCe(listed)) => {
                if let Some(weapons) = data.ce.weapons.as_mut() {
                    weapons.equippable = Some(listed);
                }
            }
            None => {
                if let Some(weapons) = data.weapons.as_mut() {
                    weapons.equippable.get_or_insert_with(Vec::new);
                }
                if let Some(weapons) = data.ce.weapons.as_mut() {
                    weapons.equippable.get_or_insert_with(Vec::new);
                }
            }
        }
        false
    }
}

/// The opened document's unsaved edits as bytes, taken on the UI thread for a
/// worker to read with [`read_opened_tag`]; none for a clean document, which
/// the worker re-reads from disk, so a large BSP does not serialize on the UI
/// thread merely to hand the same bytes back to the parser.
pub(in crate::app::model_preview) fn snapshot_edits(
    document: Option<&crate::app::TagDocument>,
) -> Option<Result<Vec<u8>, String>> {
    let document = document.filter(|document| document.dirty.is_set())?;
    Some(
        document
            .tag
            .write_to_bytes()
            .map_err(|error| format!("Could not snapshot edited model: {error}")),
    )
}

/// The opened tag as the editor holds it: `edits` from [`snapshot_edits`],
/// or the file when there are none.
pub(in crate::app::model_preview) fn read_opened_tag(
    source: &TagSource,
    entry: &TagEntry,
    edits: Option<Result<Vec<u8>, String>>,
) -> Result<TagFile, String> {
    match edits {
        // Edited bytes re-parse the way the kit reads them from disk: a
        // classic (H2/CE) tag has no self-describing layout for
        // `TagFile::read_from_bytes` to find.
        Some(Ok(bytes)) => {
            let (game, definitions_root) = match source {
                TagSource::LooseFolder {
                    game,
                    definitions_root,
                    ..
                } => (*game, Some(definitions_root.as_path())),
                _ => (None, None),
            };
            crate::core::source::read_tag_from_bytes(
                &bytes,
                game,
                definitions_root,
                entry.group_tag,
            )
            .map_err(|error| error.to_string())
        }
        Some(Err(error)) => Err(error),
        None => read_entry(source, entry).map_err(|error| error.to_string()),
    }
}
