//! One loaded editing kit (tag source) and the state scoped to it.
//! It owns kit identity and per-source state; global preferences, dialogs, and process-level services belong on [`Baboon`].

use super::*;
use crate::app::browser::KitBrowser;
use crate::app::mods::project::KitProject;
use crate::app::shell::session::RestorePlan;

/// Stable, never-reused identity for a loaded kit.
///
/// Allocated from a monotonic counter on [`Baboon`], never from a position in
/// `kits`. This is the load-bearing invariant of the multi-kit model: a stale
/// id left behind by a background job or a layout reference resolves to `None`
/// after its kit closes, where a positional index would silently retarget
/// whichever kit slid into that slot.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(in crate::app) struct KitId(pub(in crate::app) u64);

/// Which kit a background job ran for, and against which revision of it.
///
/// Every kit-scoped [`WorkerMessage`] carries one. Validating it answers both
/// staleness questions at once — did the kit close while the job ran, and was
/// its source replaced underneath it — so no handler has to remember to check
/// them separately. A single global generation could not do this: reloading
/// one kit would have invalidated every other kit's in-flight work.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(in crate::app) struct KitStamp {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) generation: u64,
}

/// One editing kit: a tag source plus every piece of state scoped to it.
///
/// Baboon was historically single-source, with all of this living flat on
/// [`Baboon`] and keyed by a bare tag-key string. Holding it per kit is what
/// lets several kits be resident at once without their documents, caches, and
/// indices aliasing one another — which they genuinely would, since monolithic
/// (`cache:{group}:{name}`) and container (`ublock:{chunk}:{path}`) tag keys
/// are not unique across sources.
///
/// A kit with `source: None` is an empty workspace. [`Baboon::kits`] always
/// holds at least one kit, so an unloaded application is one empty kit rather
/// than an absent one; its empty maps then give every reader the right
/// behavior with no special-casing at the call site.
pub(in crate::app) struct Kit {
    pub(in crate::app) id: KitId,
    /// The loaded source and its source-relative browser/index state, or
    /// `None` for an empty workspace.
    pub(in crate::app) source: Option<LoadedSourceData>,
    /// This kit's tag-name index: the source's own names merged over the
    /// application defaults. Per-kit because group and field naming differs
    /// between games, and split view renders two kits in the same frame.
    pub(in crate::app) names: TagNameIndex,

    // --- Open documents ---
    /// Parsed documents keyed by the stable [`TagEntry::key`] identity.
    pub(in crate::app) parsed_tags: HashMap<String, TagDocument>,
    /// Keys with an outstanding background load, preventing duplicate jobs.
    pub(in crate::app) loading_tags: HashSet<String>,
    /// Active document key. Selection may temporarily precede parsing while a
    /// matching key is present in `loading_tags`.
    pub(in crate::app) selected_key: Option<String>,
    /// The open document keys in tab order: re-derived from the view's
    /// `tag_tree`, which owns the layout, by [`Baboon::sync_open_tabs`].
    pub(in crate::app) open_tabs: Vec<String>,

    // --- Source generation and index state ---
    /// This kit's background index work. It lived on the app, shared by every
    /// kit: loading one kit reset another's in-flight reference build, and one
    /// kit's build or refresh blocked every other kit's.
    pub(in crate::app) index_jobs: IndexJobs,
    /// Bumped whenever this kit's source or its `all_entries` set is replaced,
    /// so caches and in-flight async results know to recompute or drop against
    /// fresh data. Per kit, so reloading one kit cannot invalidate another's.
    pub(in crate::app) generation: u64,
    pub(in crate::app) field_index: FieldValueIndex,
    pub(in crate::app) keywords: KeywordStore,
    /// The favourites the browser lists, resolved against this kit's source:
    /// tags as entries, folders as paths.
    pub(in crate::app) active_favorite_entries: Vec<TagEntry>,
    pub(in crate::app) active_favorite_folders: Vec<PathBuf>,
    /// True while a background full-scan of this loose-folder source is running.
    pub(in crate::app) scanning_entries: bool,

    /// The path the user chose when opening this kit, as typed into the file
    /// dialog or the recents list — not the resolved scan root, which can
    /// differ (picking an editing-kit root scans its `tags/` subdirectory).
    /// Matching on the requested path is what lets a repeat open of the same
    /// folder focus this kit instead of loading a duplicate.
    pub(in crate::app) requested_path: Option<PathBuf>,
    /// First-class custom profile associated with this workspace, if any.
    pub(in crate::app) profile: Option<EditingKitProfileIdentity>,

    /// Folders the user made in a container source that no tag has landed in
    /// yet, as `/`-separated `display_path`-cased paths.
    ///
    /// Held here rather than on `LoadedSourceData` because that is rebuilt on
    /// every reload and snapshotted into worker threads, and a folder made to
    /// organise work into has to outlive both. `Baboon::rebuild_kit_tree` is the
    /// only place this is applied — every other tree rebuild routes through it,
    /// because a site that forgets it silently deletes the user's folders.
    pub(in crate::app) pending_container_folders: std::collections::BTreeSet<String>,

    /// Campaign Evolved only: Chimp, which edits the same containers' Unreal
    /// packages on a surface beside the tags (the view's `surface`).
    pub(in crate::app) chimp: ChimpState,

    /// What a restored session still has to put back once the kit's source
    /// loads: tags and folders, undo histories, Chimp packages, the libraries,
    /// and tags named on the command line.
    pub(in crate::app) restore: RestorePlan,
    /// This kit's Campaign Evolved recovery/project database, and project
    /// contents staged until its source finishes mounting.
    pub(in crate::app) project: KitProject,
}

impl Kit {
    /// An empty workspace: no source, default names, nothing open.
    pub(in crate::app) fn empty(id: KitId, names: TagNameIndex) -> Self {
        Self {
            id,
            source: None,
            names,
            parsed_tags: HashMap::new(),
            loading_tags: HashSet::new(),
            selected_key: None,
            open_tabs: Vec::new(),
            index_jobs: IndexJobs::default(),
            generation: 0,
            field_index: FieldValueIndex::default(),
            keywords: KeywordStore::default(),
            active_favorite_entries: Vec::new(),
            active_favorite_folders: Vec::new(),
            scanning_entries: false,
            requested_path: None,
            profile: None,
            pending_container_folders: std::collections::BTreeSet::new(),
            chimp: ChimpState::default(),
            restore: RestorePlan {
                pending_restore_tags: Vec::new(),
                pending_restore_folders: Vec::new(),
                pending_history: HashMap::new(),
                pending_restore_chimp_packages: Vec::new(),
                pending_restore_bitmap_library: false,
                pending_restore_model_library: false,
                pending_restore_active_chimp_package: None,
                pending_launch_tags: None,
            },
            project: KitProject::default(),
        }
    }

    /// The pending-folder set in the shape `build_tree_with_folders` wants.
    pub(in crate::app) fn folder_seeds(&self) -> Vec<String> {
        self.pending_container_folders.iter().cloned().collect()
    }

    /// Whether this workspace holds edits that are not written into the game:
    /// unsaved open documents, or tags stashed in its project.
    ///
    /// One predicate for both, because the tab's dot and its colour were
    /// computed separately and drifted apart — closing the last stashed tag
    /// left the dot showing while the colour went back to clean.
    pub(in crate::app) fn has_unwritten_modifications(&self) -> bool {
        self.parsed_tags
            .values()
            .any(|document| document.dirty.is_set())
            || self.chimp.documents.values().any(|document| document.dirty)
            || self
                .project.active
                .as_ref()
                .is_some_and(|project| !project.overlays.is_empty())
    }

    /// Whether this kit is an empty workspace rather than a loaded source.
    /// A lone empty kit is hidden by the kit strip, so an unloaded Baboon
    /// looks exactly as it did when it was single-source.
    #[allow(dead_code)]
    pub(in crate::app) fn is_empty_workspace(&self) -> bool {
        self.source.is_none()
    }

    /// Whether this workspace is available to receive a new source load.
    ///
    /// Source loading reserves an empty kit by recording the requested path
    /// before the worker starts. It is still visually empty while that work
    /// is in flight, but it must not be reused for a different source.
    pub(in crate::app) fn can_accept_source_load(&self) -> bool {
        self.source.is_none() && self.requested_path.is_none()
    }

    /// Clear state staged for a source load that failed before installation.
    /// The source-less kit can then be reused without carrying the failed
    /// source's profile, restore windows, project target, or launch request.
    pub(in crate::app) fn release_source_load(&mut self) {
        if self.source.is_some() {
            return;
        }
        self.requested_path = None;
        self.profile = None;
        self.restore.pending_restore_tags.clear();
        self.restore.pending_restore_folders.clear();
        self.restore.pending_restore_chimp_packages.clear();
        self.restore.pending_restore_bitmap_library = false;
        self.restore.pending_restore_model_library = false;
        self.restore.pending_restore_active_chimp_package = None;
        self.restore.pending_launch_tags = None;
        self.project.pending = None;
        // Folders belong to the source that was being loaded, so a reused kit
        // must not seed them into whatever mounts here next.
        self.pending_container_folders.clear();
    }
}

impl Baboon {

    #[allow(dead_code)]
    pub(in crate::app) fn kit_mut(&mut self, id: KitId) -> Option<&mut Kit> {
        self.model.kits.iter_mut().find(|kit| kit.id == id)
    }

    #[allow(dead_code)]
    pub(in crate::app) fn active_kit_mut(&mut self) -> &mut Kit {
        &mut self.model.kits[self.model.active]
    }

    /// Focus the kit a piece of navigation state belongs to, before acting on
    /// it. Navigation names tags by key, and a key only means something within
    /// its own kit — so a jump, reveal, or results row has to return to the kit
    /// it came from. Returns false when that kit has closed, in which case the
    /// navigation is dropped rather than applied to whichever kit is active.
    pub(in crate::app) fn focus_navigation_kit(&mut self, kit: KitId) -> bool {
        match self.model.kit_index(kit) {
            Some(index) => {
                self.focus_kit(index);
                true
            }
            None => false,
        }
    }

    /// Make the kit at `index` the one actions land on, and bring its
    /// workspace tab to the front.
    ///
    /// `active` decides where the File menu, Ctrl+S and the save prompt act;
    /// the kit tab bar decides which game the user is looking at. Setting one
    /// without the other sends an import or a save into a game in a background
    /// tab. A split needs no help — both panes are already visible, and
    /// `make_active` leaves a pane that is not in a tab group alone.
    pub(in crate::app) fn focus_kit(&mut self, index: usize) {
        self.model.active = index;
        let kit = self.model.kits[index].id;
        self.kit_tree
            .make_active(|_, tile| matches!(tile, egui_tiles::Tile::Pane(id) if *id == kit));
    }

    pub(in crate::app) fn source_mut(&mut self) -> Option<&mut LoadedSourceData> {
        self.model.kits[self.model.active].source.as_mut()
    }

    /// Allocate the next never-reused kit id.
    #[allow(dead_code)]
    pub(in crate::app) fn next_kit_id(&mut self) -> KitId {
        let id = KitId(self.model.next_kit_id);
        self.model.next_kit_id = self.model.next_kit_id.wrapping_add(1);
        id
    }

    /// Add an empty kit carrying a fresh id and the application defaults,
    /// with the view a new workspace opens in. Every kit is made here so no
    /// path can miss the seeding and open in the wrong view.
    fn push_empty_kit(&mut self) -> KitId {
        let id = self.next_kit_id();
        self.model.kits.push(Kit::empty(id, self.model.default_names.clone()));
        self.views.insert(
            id,
            KitView::new(
                id,
                KitBrowser::new(
                    self.model.prefs.browser_mode,
                    self.model.prefs.browser_sort,
                    self.model.prefs.browser_search_scope,
                ),
            ),
        );
        id
    }

    /// Add an empty kit and make it active. The next load installs into it,
    /// so "open another game" is add-then-load rather than a separate path.
    pub(in crate::app) fn add_kit(&mut self) -> KitId {
        let id = self.push_empty_kit();
        self.model.active = self.model.kits.len() - 1;
        id
    }

    /// Add `kit` as it is, with the view a new workspace opens in, for tests
    /// that build a kit by hand.
    #[cfg(test)]
    pub(in crate::app) fn push_kit(&mut self, kit: Kit) {
        self.views.insert(kit.id, KitView::new(kit.id, KitBrowser::default()));
        self.model.kits.push(kit);
    }

    /// Remove a kit, dropping its documents and caches. `kits` is never left
    /// empty — closing the last one leaves a fresh empty workspace, which is
    /// the same state Baboon starts in.
    pub(in crate::app) fn remove_kit(&mut self, id: KitId) {
        let Some(index) = self.model.kit_index(id) else {
            return;
        };
        let closing_campaign_evolved = self.model.kits[index]
            .source
            .as_ref()
            .is_some_and(|source| matches!(&source.source, TagSource::IoStoreContainerSet { .. }));
        if closing_campaign_evolved {
            self.reset_runtime_poke_source_state();
        }
        self.model.kits.remove(index);
        self.views.remove(id);
        if self.model.kits.is_empty() {
            self.push_empty_kit();
        }
        // The tab bar would otherwise fall back to its first tab while the
        // actions went to the kit that slid into the closed one's place.
        self.focus_kit(active_after_removal(self.model.active, index, self.model.kits.len()));
    }

    /// Route an open request for `path` to a kit.
    ///
    /// Opening a source that is already open focuses its kit rather than
    /// loading a second copy of it — the same gesture that switches to an
    /// already-open tab in an editor. Otherwise the current kit is reused only
    /// when it is an idle empty workspace, and a new kit is added if it is
    /// loaded or already reserved for another load, so opening a second game
    /// never silently discards the first.
    ///
    /// Returns `true` when the source was already open and no load is needed.
    pub(in crate::app) fn open_kit_for(&mut self, path: &Path) -> bool {
        let path = clean_recent_path(path.to_path_buf());
        if let Some(index) = self
            .model.kits
            .iter()
            .position(|kit| requested_path_matches(kit, &path))
        {
            self.focus_kit(index);
            return true;
        }
        if !self.model.kits[self.model.active].can_accept_source_load() {
            self.add_kit();
        }
        self.model.kits[self.model.active].requested_path = Some(path);
        false
    }

    /// Release a source-load reservation after the worker or synchronous
    /// preflight fails. The kit may then be reused for a later open request.
    /// All state staged specifically for that failed load is discarded with
    /// the reservation so it cannot leak into the next source.
    pub(in crate::app) fn release_source_load(&mut self, kit: KitId) {
        let Some(index) = self.model.kit_index(kit) else {
            return;
        };
        self.model.kits[index].release_source_load();
    }

    /// Install a freshly loaded source into the active kit, replacing whatever
    /// it held. Multi-kit loading (add-a-kit rather than replace) arrives with
    /// the kit strip; until then this preserves single-source behavior.
    pub(in crate::app) fn install_loaded_source(&mut self, source: LoadedSourceData) {
        let mut names = source.names.clone();
        names.merge_missing(self.model.default_names.clone());
        let id = self.model.active_kit_id();
        let index = self.model.active;
        // The requested path outlives the load it started, so a later open of
        // the same folder can find this kit.
        let requested_path = self.model.kits[index].requested_path.clone();
        let profile = self.model.kits[index].profile.clone();
        // The whole restore plan, staged before the load, carries over.
        let restore = std::mem::take(&mut self.model.kits[index].restore);
        // A project staged for this source is still waiting for it; the
        // previous source's open project is not carried.
        let pending_project = std::mem::take(&mut self.model.kits[index].project.pending);
        // The browser view belongs to the workspace, not to the source in it:
        // reloading a kit — or restoring one, which stages the saved view
        // before the load lands — must not snap it back to the default.
        // The rest of the view starts over with the source.
        let browser = &self.views[id].browser;
        let browser = KitBrowser::new(browser.mode, browser.sort, browser.search_scope);
        self.views.insert(id, KitView::new(id, browser));
        // Carried, then moved on, never reset: a job stamped by the source
        // being replaced must not resolve against the new one. Rebuilding
        // from `Kit::empty` reset it to 0, and the load handler's bump then
        // gave every source in the kit generation 1, so a stale result from
        // the previous source passed `resolve_stamp`.
        let generation = self.model.kits[index].generation.wrapping_add(1);
        self.model.kits[index] = Kit {
            source: Some(source),
            generation,
            names,
            requested_path,
            profile,
            restore,
            project: KitProject {
                active: None,
                pending: pending_project,
            },
            ..Kit::empty(id, self.model.default_names.clone())
        };
    }
}

/// The groups whose tags the shader grid reads through `rmdf_cache` and
/// `rmop_cache`.
pub(in crate::app) fn is_render_method_layout_group(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"rmdf") || group_tag == u32::from_be_bytes(*b"rmop")
}

impl Kit {
    /// Set one tag's references in the loaded reference index, or with `None`
    /// drop the tag from it. Every change made during a session goes through
    /// here, so a rebuild running at the time can be told about it.
    pub(in crate::app) fn set_tag_references(&mut self, key: &str, references: Option<Vec<DependencyRef>>) {
        if let Some(index) = self
            .source
            .as_mut()
            .and_then(|source| source.reverse_dependencies.as_mut())
        {
            match &references {
                Some(references) => index.set_tag_dependencies(key.to_owned(), references.clone()),
                None => index.clear_tag(key),
            }
        }
        if self.index_jobs.building_references {
            self.index_jobs
                .references_changed_during_build
                .insert(key.to_owned(), references);
        }
    }
}

fn requested_path_matches(kit: &Kit, path: &Path) -> bool {
    kit.requested_path
        .as_deref()
        .is_some_and(|open| same_recent_path(open, path))
}

fn kit_has_dirty_documents(kit: &Kit) -> bool {
    kit.parsed_tags.iter().any(|(key, document)| {
        document.dirty.is_set() && document_edits_are_saveable(kit, key, document)
    }) || kit.chimp.documents.values().any(|document| document.dirty)
}

/// Whether this kit's edits to `key` could be written back at all.
///
/// A monolithic build (and any other tag with no writer) is editable but never
/// saveable, so its dirty flag is not unsaved *work*: counting it as such would
/// raise a save prompt on every close whose only honest answer is Discard.
pub(in crate::app) fn document_edits_are_saveable(kit: &Kit, key: &str, document: &TagDocument) -> bool {
    // An entry the kit can no longer find is not one this can rule out.
    kit.entry_for_key(key)
        .is_none_or(|entry| is_saveable_tag(entry, &document.tag))
}

/// Label for a kit in the kit strip: the game's display name where one was
/// detected, otherwise the source label, otherwise an empty workspace.
pub(in crate::app) fn kit_strip_label(kit: &Kit) -> String {
    let Some(source) = kit.source.as_ref() else {
        return "New workspace".to_owned();
    };
    if let Some(profile) = &kit.profile {
        return profile.name.clone();
    }
    match source.game {
        Some(game) => game.display_name().to_owned(),
        None => source.label.clone(),
    }
}

/// Where the active selection lands after the kit at `removed` is taken out of
/// a list that now has `new_len` entries.
///
/// Kits before the removed one keep their index; kits after it shift down by
/// one, so the active selection has to shift with them or it silently
/// retargets a neighbour. Removing the active kit itself falls through to the
/// clamp, which selects the kit that slid into its place (or the new last one).
fn active_after_removal(active: usize, removed: usize, new_len: usize) -> usize {
    let shifted = if active > removed { active - 1 } else { active };
    shifted.min(new_len.saturating_sub(1))
}

/// egui id for a kit's tag layout tree. Distinct per kit so two trees rendered
/// in the same frame keep separate drag state.
pub(in crate::app) fn tag_tree_id(id: KitId) -> egui::Id {
    egui::Id::new(("kit_tag_tree", id.0))
}

impl Kit {
    /// This kit's browser entry for `key`, wherever it is listed: the visible
    /// entries, the full set a filtered browser hides, or a favorite pulled in
    /// from elsewhere.
    pub(in crate::app) fn entry_for_key(&self, key: &str) -> Option<&TagEntry> {
        let source = self.source.as_ref()?;
        source.entry_for_key(key).or_else(|| {
            self.active_favorite_entries
                .iter()
                .find(|entry| entry.key == key)
        })
    }
}

/// A kit's background tag-index and reference-index jobs.
#[derive(Default)]
pub(in crate::app) struct IndexJobs {
    /// Checking the cached loose-folder index for file changes.
    pub(in crate::app) refreshing: bool,
    /// When the next periodic refresh is due, in egui time.
    pub(in crate::app) next_refresh_at: f64,
    /// A reference-index build is running.
    pub(in crate::app) building_references: bool,
    /// That build was started by a tag-index build, which reports them as one.
    pub(in crate::app) references_for_entry_index: bool,
    pub(in crate::app) reference_progress: Option<ReferenceIndexProgressState>,
    pub(in crate::app) entry_progress: Option<EntryIndexProgressState>,
    /// Tags whose references changed (a save, a refresh, a new or deleted tag)
    /// while a reference-index build was running, with what they are now;
    /// `None` for a tag that is gone. The build read those tags before the
    /// change, so its result is patched with these before it replaces the
    /// index, rather than reverting them. See [`Kit::set_tag_references`].
    pub(in crate::app) references_changed_during_build: HashMap<String, Option<Vec<DependencyRef>>>,
}

impl Model {
    /// Look a kit up by its stable id. Unused while `kits` holds a single
    /// workspace; these are the entry points the kit strip, per-kit worker
    /// routing, and the layout trees resolve through once more than one kit
    /// can be resident.
    #[allow(dead_code)]
    pub(in crate::app) fn kit_index(&self, id: KitId) -> Option<usize> {
        self.kits.iter().position(|kit| kit.id == id)
    }

    #[allow(dead_code)]
    pub(in crate::app) fn kit(&self, id: KitId) -> Option<&Kit> {
        self.kits.iter().find(|kit| kit.id == id)
    }

    /// The active kit. Infallible: `kits` is never empty and `active` is
    /// always a valid index into it.
    #[allow(dead_code)]
    pub(in crate::app) fn active_kit(&self) -> &Kit {
        &self.kits[self.active]
    }

    pub(in crate::app) fn active_kit_id(&self) -> KitId {
        self.kits[self.active].id
    }

    /// Stamp identifying the active kit and its current revision, to be
    /// attached to a background job so its result can be routed back.
    pub(in crate::app) fn kit_stamp(&self) -> KitStamp {
        let kit = &self.kits[self.active];
        KitStamp {
            kit: kit.id,
            generation: kit.generation,
        }
    }

    /// Resolve a stamp to the kit index it still refers to, or `None` if the
    /// kit has closed or its source was replaced while the job was running.
    /// Ids are never reused, so a closed kit cannot alias a live one.
    pub(in crate::app) fn resolve_stamp(&self, stamp: KitStamp) -> Option<usize> {
        let index = self.kit_index(stamp.kit)?;
        (self.kits[index].generation == stamp.generation).then_some(index)
    }

    /// Resolve a kit id to its index, ignoring generation. For results that
    /// stay valid across a source reload, such as a parsed document.
    pub(in crate::app) fn resolve_kit(&self, kit: KitId) -> Option<usize> {
        self.kit_index(kit)
    }

    /// The active kit's source, or `None` for an empty workspace.
    pub(in crate::app) fn source(&self) -> Option<&LoadedSourceData> {
        self.kits[self.active].source.as_ref()
    }

    pub(in crate::app) fn names(&self) -> &TagNameIndex {
        &self.kits[self.active].names
    }

    /// Whether any kit holds unsaved edits.
    pub(in crate::app) fn any_kit_dirty(&self) -> bool {
        self.kits.iter().any(kit_has_dirty_documents)
    }

    /// Index of the first kit holding unsaved edits.
    pub(in crate::app) fn first_dirty_kit(&self) -> Option<usize> {
        self.kits.iter().position(kit_has_dirty_documents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::TagFile;
    use crate::app::kits::{KitMut, KitView};
    use crate::app::test_definition_path;
    use crate::core::source::{LoadedSourceData, TagEntry, TagEntryLocation, TagSource, build_tree};
    use std::collections::HashMap;
    use std::path::Path;
    use std::path::PathBuf;
    use super::EditingKitProfileIdentity;
    use super::{Kit, KitId, TagDocument, active_after_removal, kit_has_dirty_documents};

    fn kit_holding(location: TagEntryLocation, endian: blam_tags::Endian) -> Kit {
        let mut tag = TagFile::new(test_definition_path("halo4_mcc/camera_track.json")).unwrap();
        tag.endian = endian;
        let entry = TagEntry {
            key: "tag".to_owned(),
            display_path: "test/example.camera_track".to_owned(),
            group_tag: tag.header.group_tag,
            group_name: Some("camera_track".to_owned()),
            location,
        };
        let entries = vec![entry];
        let mut kit = Kit::empty(KitId(0), Default::default());
        kit.source = Some(LoadedSourceData {
            label: "test".to_owned(),
            // Irrelevant to the question asked: what decides an edit's fate is
            // the entry's location, not how the browser was opened.
            source: TagSource::SingleFile {
                path: PathBuf::from("example.camera_track"),
            },
            names: Default::default(),
            game: None,
            tree: build_tree(&entries),
            group_tree: build_tree(&entries),
            entries,
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });
        kit.parsed_tags
            .insert("tag".to_owned(), TagDocument::modified(tag));
        kit
    }

    /// Edits to a tag with nowhere to be written are not unsaved *work*, and
    /// must not raise the close prompt.
    ///
    /// The prompt's only outcomes are Save and Discard. Save on a monolithic
    /// build fails by construction, and `CloseApp` re-checks for dirty
    /// documents after the prompt closes — so counting these would put a quit
    /// behind a dialog whose Save button can never clear it.
    #[test]
    fn a_dirty_tag_that_can_never_be_saved_is_not_unsaved_work() {
        let monolithic = kit_holding(
            TagEntryLocation::Monolithic {
                name: "test\\example".to_owned(),
                group_tag: u32::from_be_bytes(*b"trak"),
            },
            blam_tags::Endian::Be,
        );
        assert!(
            !kit_has_dirty_documents(&monolithic),
            "a monolithic build's edits are session-scratch, not unsaved work"
        );

        // The same document from somewhere it can be written back to still
        // stops a close, which is the whole point of the flag.
        let loose = kit_holding(
            TagEntryLocation::LooseFile(PathBuf::from("example.camera_track")),
            blam_tags::Endian::Le,
        );
        assert!(kit_has_dirty_documents(&loose));
    }

    /// A folder move rewrites tag keys underneath the open tabs. The tree is
    /// what has to be rewritten: `open_tabs` is re-derived from it every frame,
    /// so a remap that touched only the list was overwritten immediately and
    /// left the panes pointing at keys the source no longer had.
    #[test]
    fn remapping_tag_keys_rewrites_the_layout_tree_itself() {
        let mut kit = Kit::empty(KitId(0), Default::default());
        let mut view = KitView::for_test(&kit);
        let mut both = KitMut::new(&mut kit, &mut view);
        both.open_tag_pane("file:/tags/objects/a.weapon");
        both.open_tag_pane("file:/tags/objects/b.weapon");
        both.kit.selected_key = Some("file:/tags/objects/a.weapon".to_owned());

        let mut map = HashMap::new();
        map.insert(
            "file:/tags/objects/a.weapon".to_owned(),
            "file:/tags/moved/a.weapon".to_owned(),
        );
        both.remap_tag_keys(&map);

        // Read back through the tree, not the cached list, so the assertion
        // fails if only the list was rewritten.
        let panes = view.tabs_from_tree();
        assert!(panes.contains(&"file:/tags/moved/a.weapon".to_owned()));
        assert!(!panes.contains(&"file:/tags/objects/a.weapon".to_owned()));
        assert!(panes.contains(&"file:/tags/objects/b.weapon".to_owned()));
        assert_eq!(kit.open_tabs, panes);
        assert_eq!(
            kit.selected_key.as_deref(),
            Some("file:/tags/moved/a.weapon")
        );
    }

    #[test]
    fn removing_a_kit_before_the_active_one_shifts_the_selection_down() {
        // [a b *c] -> remove a -> [b *c]: the active kit moved from 2 to 1.
        assert_eq!(active_after_removal(2, 0, 2), 1);
        assert_eq!(active_after_removal(1, 0, 2), 0);
    }

    #[test]
    fn removing_a_kit_after_the_active_one_leaves_the_selection_alone() {
        // [*a b c] -> remove c -> [*a b]: still index 0.
        assert_eq!(active_after_removal(0, 2, 2), 0);
        assert_eq!(active_after_removal(1, 2, 2), 1);
    }

    #[test]
    fn removing_the_active_kit_selects_the_one_that_took_its_place() {
        // [a *b c] -> remove b -> [a c]: index 1 is now the former c.
        assert_eq!(active_after_removal(1, 1, 2), 1);
        // Removing the last kit clamps back onto the new last kit.
        assert_eq!(active_after_removal(2, 2, 2), 1);
    }

    #[test]
    fn the_selection_never_points_past_the_end() {
        // Closing the only kit leaves one fresh empty workspace behind it.
        assert_eq!(active_after_removal(0, 0, 1), 0);
        assert_eq!(active_after_removal(5, 0, 1), 0);
    }

    #[test]
    fn an_inflight_source_reserves_an_empty_workspace() {
        let mut kit = Kit::empty(KitId(0), Default::default());
        assert!(kit.is_empty_workspace());
        assert!(kit.can_accept_source_load());

        kit.requested_path = Some(PathBuf::from("reach"));

        assert!(kit.is_empty_workspace());
        assert!(!kit.can_accept_source_load());
    }

    #[test]
    fn releasing_a_failed_source_load_makes_the_workspace_reusable() {
        let mut kit = Kit::empty(KitId(0), Default::default());
        kit.requested_path = Some(PathBuf::from("reach"));
        kit.profile = Some(EditingKitProfileIdentity {
            id: "reach-profile".to_owned(),
            name: "Reach".to_owned(),
        });
        kit.restore.pending_launch_tags = Some(vec![PathBuf::from("objects/example.weapon")]);

        kit.release_source_load();

        assert!(kit.can_accept_source_load());
        assert!(kit.profile.is_none());
        assert!(kit.restore.pending_launch_tags.is_none());
    }

    #[test]
    fn a_duplicate_pending_source_matches_its_reserved_workspace() {
        let mut kit = Kit::empty(KitId(0), Default::default());
        kit.requested_path = Some(PathBuf::from("reach"));

        assert!(super::requested_path_matches(&kit, Path::new("reach")));
        assert!(!super::requested_path_matches(
            &kit,
            Path::new("campaign-evolved")
        ));
    }

    /// Two sources loaded one after another into the same kit must not share a
    /// generation, or a job stamped against the first resolves against the
    /// second.
    #[test]
    fn a_second_source_in_a_kit_never_reuses_a_generation() {
        let mut app = crate::app::Baboon::for_test();
        let source = |label: &str| LoadedSourceData {
            label: label.to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from(format!("/{label}")),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        };
        let mut seen = Vec::new();
        let mut stamps = Vec::new();
        for label in ["first", "second", "third"] {
            app.install_loaded_source(source(label));
            // The load handler moves the generation on once more after
            // installing; mirror it so the test sees what jobs see.
            app.model.kits[app.model.active].generation = app.model.kits[app.model.active].generation.wrapping_add(1);
            seen.push(app.model.kits[app.model.active].generation);
            stamps.push(app.model.kit_stamp());
        }
        let mut unique = seen.clone();
        unique.dedup();
        assert_eq!(unique, seen, "generations {seen:?} repeat");
        assert!(app.model.resolve_stamp(stamps[0]).is_none(), "a stale stamp is refused");
        assert!(app.model.resolve_stamp(stamps[2]).is_some());
    }

    /// Closing tabs drops every cache kept for them, model previews included.
    /// Three of the four close paths kept the model preview (its geometry and
    /// textures) for the rest of the session.
    #[test]
    fn closing_tabs_drops_everything_kept_for_them() {
        let mut kit = Kit::empty(KitId(0), TagNameIndex::default());
        let mut view = KitView::for_test(&kit);
        for key in ["kept", "closed"] {
            view.caches.model_previews
                .insert(key.to_owned(), ModelPreviewState::default());
            view.caches.bitmap_previews
                .insert(key.to_owned(), BitmapPreviewState::default());
            kit.loading_tags.insert(key.to_owned());
            view.edit_buffers
                .insert_clean(format!("{key}|name"), "x".to_owned());
        }

        KitMut::new(&mut kit, &mut view).drop_documents_except(Some("kept"));
        assert_eq!(view.caches.model_previews.keys().collect::<Vec<_>>(), ["kept"]);
        assert_eq!(view.caches.bitmap_previews.keys().collect::<Vec<_>>(), ["kept"]);
        assert_eq!(kit.loading_tags.iter().collect::<Vec<_>>(), ["kept"]);

        KitMut::new(&mut kit, &mut view).drop_document("kept");
        assert!(view.caches.model_previews.is_empty());
        assert!(view.caches.bitmap_previews.is_empty());
        assert!(kit.loading_tags.is_empty());
    }
}
