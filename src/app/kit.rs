//! One loaded editing kit (tag source) and the state scoped to it.
//! It owns kit identity and per-source state; global preferences, dialogs, and process-level services belong on [`Baboon`].

use super::*;

/// Stable, never-reused identity for a loaded kit.
///
/// Allocated from a monotonic counter on [`Baboon`], never from a position in
/// `kits`. This is the load-bearing invariant of the multi-kit model: a stale
/// id left behind by a background job or a layout reference resolves to `None`
/// after its kit closes, where a positional index would silently retarget
/// whichever kit slid into that slot.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(super) struct KitId(pub(super) u64);

/// Which kit a background job ran for, and against which revision of it.
///
/// Every kit-scoped [`WorkerMessage`] carries one. Validating it answers both
/// staleness questions at once — did the kit close while the job ran, and was
/// its source replaced underneath it — so no handler has to remember to check
/// them separately. A single global generation could not do this: reloading
/// one kit would have invalidated every other kit's in-flight work.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(super) struct KitStamp {
    pub(super) kit: KitId,
    pub(super) generation: u64,
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
pub(super) struct Kit {
    pub(super) id: KitId,
    /// The loaded source and its source-relative browser/index state, or
    /// `None` for an empty workspace.
    pub(super) source: Option<LoadedSourceData>,
    /// This kit's tag-name index: the source's own names merged over the
    /// application defaults. Per-kit because group and field naming differs
    /// between games, and split view renders two kits in the same frame.
    pub(super) names: TagNameIndex,

    // --- Open documents ---
    /// Parsed documents keyed by the stable [`TagEntry::key`] identity.
    pub(super) parsed_tags: HashMap<String, TagDocument>,
    /// Keys with an outstanding background load, preventing duplicate jobs.
    pub(super) loading_tags: HashSet<String>,
    /// Active document key. Selection may temporarily precede parsing while a
    /// matching key is present in `loading_tags`.
    pub(super) selected_key: Option<String>,
    /// Docked and floating tabs share this ordered set of open document keys.
    pub(super) open_tabs: Vec<String>,
    /// Layout of this kit's open tags: which panes exist, how they are split,
    /// and which is active in each tab group. The tree is authoritative —
    /// `open_tabs` is re-derived from it, never the other way round — so a
    /// split or a drag survives instead of being overwritten by a mirror.
    pub(super) tag_tree: egui_tiles::Tree<String>,
    /// In-progress text edits keyed by stable widget/edit identifiers. Drafts
    /// rather than bare strings so a value the user is still typing is not
    /// overwritten by the document underneath it.
    pub(super) edit_buffers: EditDrafts,

    // --- Per-document derived caches ---
    pub(super) bitmap_previews: HashMap<String, BitmapPreviewState>,
    pub(super) model_previews: HashMap<String, ModelPreviewState>,
    /// Source-local render-method definition cache; `None` is a cached miss.
    pub(super) rmdf_cache: HashMap<String, Option<RenderMethodDefinition>>,
    /// Source-local render-method option cache; `None` is a cached miss.
    pub(super) rmop_cache: HashMap<String, Option<RenderMethodOption>>,
    /// Campaign Evolved Wwise bindings, cached per tag key because resolving
    /// one walks several packages.
    pub(super) ce_sound_bindings: HashMap<String, Arc<crate::source::ce_audio::CeSoundBinding>>,

    /// Pending expand/collapse-all requests, keyed by tag. Raised from the tag
    /// tab's menu and consumed by the next draw of that tag's pane.
    pub(super) pending_expand: HashMap<String, bool>,

    // --- Per-tag "Search fields" state ---
    pub(super) field_search: HashMap<String, String>,
    /// The last query actually applied per tag, so the collapse is a one-shot
    /// on change rather than a per-frame override the user can't fight.
    pub(super) field_search_applied: HashMap<String, String>,

    // --- Browser and index state ---
    /// How this kit's browser lists tags, and in what order. Per kit because
    /// the useful view differs by game — a folder-organized editing kit reads
    /// best as Folders while a container source reads best as Groups — and two
    /// browsers are on screen at once in a split. New kits start from the
    /// saved [`Baboon::default_browser_mode`], so a single workspace behaves
    /// exactly as it did when this was one application-wide setting.
    pub(super) browser_mode: BrowserMode,
    pub(super) browser_sort: BrowserSort,
    pub(super) filter: String,
    pub(super) filter_cache: FilterCache,
    /// Which tags the browser should mark as modified, and the signature the
    /// set was built from. Rebuilt only when that signature changes: resolving
    /// a tag key to its entry is a linear scan of the source, so doing it for
    /// every dirty tag every frame would cost far more than the handful of
    /// lookups it represents.
    pub(super) modified_tags: std::sync::Arc<ModifiedTags>,
    pub(super) modified_signature: Vec<String>,
    /// Bumped whenever this kit's source or its `all_entries` set is replaced,
    /// so caches and in-flight async results know to recompute or drop against
    /// fresh data. Per kit, so reloading one kit cannot invalidate another's.
    pub(super) generation: u64,
    pub(super) field_index: FieldValueIndex,
    /// Browser keys this workspace may delete, and the generation they were
    /// resolved at. Recomputed only when the generation moves: answering it
    /// walks every entry and stats each container's backups, which is far too
    /// much to repeat for every frame the browser draws.
    pub(super) deletable_keys: std::sync::Arc<HashSet<String>>,
    pub(super) deletable_keys_generation: Option<u64>,
    pub(super) keywords: KeywordStore,
    pub(super) active_favorite_entries: Vec<TagEntry>,
    /// True while a background full-scan of this loose-folder source is running.
    pub(super) scanning_entries: bool,

    // --- Per-kit terminal placement ---
    pub(super) terminal_open: bool,
    /// Working directory for terminal commands (game kit root, parent of tags/).
    pub(super) terminal_work_dir: Option<PathBuf>,

    /// The path the user chose when opening this kit, as typed into the file
    /// dialog or the recents list — not the resolved scan root, which can
    /// differ (picking an editing-kit root scans its `tags/` subdirectory).
    /// Matching on the requested path is what lets a repeat open of the same
    /// folder focus this kit instead of loading a duplicate.
    pub(super) requested_path: Option<PathBuf>,
    /// First-class custom profile associated with this workspace, if any.
    pub(super) profile: Option<EditingKitProfileIdentity>,

    /// This kit's Campaign Evolved recovery/project database, if its source
    /// has one. Per kit because a project belongs to a source — two Campaign
    /// Evolved kits are two projects, and one application-wide slot would let
    /// either checkpoint over the other's tags.
    pub(super) campaign_project: Option<ActiveCampaignProject>,
    /// Project contents staged until this kit's source finishes mounting.
    pub(super) pending_campaign_project: Option<PendingCampaignProject>,

    /// Folders the user made in a container source that no tag has landed in
    /// yet, as `/`-separated `display_path`-cased paths.
    ///
    /// Held here rather than on `LoadedSourceData` because that is rebuilt on
    /// every reload and snapshotted into worker threads, and a folder made to
    /// organise work into has to outlive both. `Baboon::rebuild_kit_tree` is the
    /// only place this is applied — every other tree rebuild routes through it,
    /// because a site that forgets it silently deletes the user's folders.
    pub(super) pending_container_folders: std::collections::BTreeSet<String>,

    /// CE-only nested surface. Chimp is not an editing kit and does not enter
    /// the top-level kit registry; it lives beside the Tags surface here.
    pub(super) surface: KitSurface,
    pub(super) chimp: ChimpState,

    /// Tags staged by a session restore, drained once this kit's source
    /// finishes loading. Held per kit rather than in one shared slot so
    /// several kits can restore concurrently and finish in any order.
    pub(super) pending_restore_tags: Vec<LastSessionTag>,
    /// Undo/redo stacks a restored project brought back, by document key, held
    /// until the document they belong to exists. A restored tab is loaded
    /// asynchronously, so the history almost always arrives before the tag it
    /// applies to.
    pub(super) pending_history: HashMap<String, TagHistory>,
    /// Chimp packages staged by session restore until the Unreal container
    /// world has mounted. Kept separate from tag restoration so Tags remains
    /// the initial surface.
    pub(super) pending_restore_chimp_packages: Vec<String>,
    pub(super) pending_restore_active_chimp_package: Option<String>,
    /// Loose tag paths requested on the command line, drained after this kit's
    /// editing-kit source finishes loading.
    pub(super) pending_launch_tags: Option<Vec<PathBuf>>,
}

impl Kit {
    /// An empty workspace: no source, default names, nothing open.
    pub(super) fn empty(id: KitId, names: TagNameIndex) -> Self {
        Self {
            id,
            source: None,
            names,
            parsed_tags: HashMap::new(),
            loading_tags: HashSet::new(),
            selected_key: None,
            open_tabs: Vec::new(),
            tag_tree: egui_tiles::Tree::empty(tag_tree_id(id)),
            edit_buffers: EditDrafts::default(),
            bitmap_previews: HashMap::new(),
            model_previews: HashMap::new(),
            rmdf_cache: HashMap::new(),
            rmop_cache: HashMap::new(),
            ce_sound_bindings: HashMap::new(),
            pending_expand: HashMap::new(),
            field_search: HashMap::new(),
            field_search_applied: HashMap::new(),
            browser_mode: BrowserMode::default(),
            browser_sort: BrowserSort::default(),
            filter: String::new(),
            filter_cache: FilterCache::default(),
            modified_tags: std::sync::Arc::new(ModifiedTags::default()),
            modified_signature: Vec::new(),
            generation: 0,
            field_index: FieldValueIndex::default(),
            deletable_keys: std::sync::Arc::new(HashSet::new()),
            deletable_keys_generation: None,
            keywords: KeywordStore::default(),
            active_favorite_entries: Vec::new(),
            scanning_entries: false,
            terminal_open: false,
            terminal_work_dir: None,
            requested_path: None,
            profile: None,
            campaign_project: None,
            pending_campaign_project: None,
            pending_container_folders: std::collections::BTreeSet::new(),
            surface: KitSurface::Tags,
            chimp: ChimpState::default(),
            pending_restore_tags: Vec::new(),
            pending_history: HashMap::new(),
            pending_restore_chimp_packages: Vec::new(),
            pending_restore_active_chimp_package: None,
            pending_launch_tags: None,
        }
    }

    /// The pending-folder set in the shape `build_tree_with_folders` wants.
    pub(super) fn folder_seeds(&self) -> Vec<String> {
        self.pending_container_folders.iter().cloned().collect()
    }

    /// Whether this workspace holds edits that are not written into the game:
    /// unsaved open documents, or tags stashed in its project.
    ///
    /// One predicate for both, because the tab's dot and its colour were
    /// computed separately and drifted apart — closing the last stashed tag
    /// left the dot showing while the colour went back to clean.
    pub(super) fn has_unwritten_modifications(&self) -> bool {
        self.parsed_tags
            .values()
            .any(|document| document.dirty.is_set())
            || self.chimp.documents.values().any(|document| document.dirty)
            || self
                .campaign_project
                .as_ref()
                .is_some_and(|project| !project.overlays.is_empty())
    }

    /// Whether this kit is an empty workspace rather than a loaded source.
    /// A lone empty kit is hidden by the kit strip, so an unloaded Baboon
    /// looks exactly as it did when it was single-source.
    #[allow(dead_code)]
    pub(super) fn is_empty_workspace(&self) -> bool {
        self.source.is_none()
    }

    /// Whether this workspace is available to receive a new source load.
    ///
    /// Source loading reserves an empty kit by recording the requested path
    /// before the worker starts. It is still visually empty while that work
    /// is in flight, but it must not be reused for a different source.
    pub(super) fn can_accept_source_load(&self) -> bool {
        self.source.is_none() && self.requested_path.is_none()
    }

    /// Clear state staged for a source load that failed before installation.
    /// The source-less kit can then be reused without carrying the failed
    /// source's profile, restore windows, project target, or launch request.
    pub(super) fn release_source_load(&mut self) {
        if self.source.is_some() {
            return;
        }
        self.requested_path = None;
        self.profile = None;
        self.pending_restore_tags.clear();
        self.pending_restore_chimp_packages.clear();
        self.pending_restore_active_chimp_package = None;
        self.pending_launch_tags = None;
        self.pending_campaign_project = None;
        // Folders belong to the source that was being loaded, so a reused kit
        // must not seed them into whatever mounts here next.
        self.pending_container_folders.clear();
    }
}

impl Baboon {
    /// Look a kit up by its stable id. Unused while `kits` holds a single
    /// workspace; these are the entry points the kit strip, per-kit worker
    /// routing, and the layout trees resolve through once more than one kit
    /// can be resident.
    #[allow(dead_code)]
    pub(super) fn kit_index(&self, id: KitId) -> Option<usize> {
        self.kits.iter().position(|kit| kit.id == id)
    }

    #[allow(dead_code)]
    pub(super) fn kit(&self, id: KitId) -> Option<&Kit> {
        self.kits.iter().find(|kit| kit.id == id)
    }

    #[allow(dead_code)]
    pub(super) fn kit_mut(&mut self, id: KitId) -> Option<&mut Kit> {
        self.kits.iter_mut().find(|kit| kit.id == id)
    }

    /// The active kit. Infallible: `kits` is never empty and `active` is
    /// always a valid index into it.
    #[allow(dead_code)]
    pub(super) fn active_kit(&self) -> &Kit {
        &self.kits[self.active]
    }

    #[allow(dead_code)]
    pub(super) fn active_kit_mut(&mut self) -> &mut Kit {
        &mut self.kits[self.active]
    }

    pub(super) fn active_kit_id(&self) -> KitId {
        self.kits[self.active].id
    }

    /// Stamp identifying the active kit and its current revision, to be
    /// attached to a background job so its result can be routed back.
    pub(super) fn kit_stamp(&self) -> KitStamp {
        let kit = &self.kits[self.active];
        KitStamp {
            kit: kit.id,
            generation: kit.generation,
        }
    }

    /// Resolve a stamp to the kit index it still refers to, or `None` if the
    /// kit has closed or its source was replaced while the job was running.
    /// Ids are never reused, so a closed kit cannot alias a live one.
    pub(super) fn resolve_stamp(&self, stamp: KitStamp) -> Option<usize> {
        let index = self.kit_index(stamp.kit)?;
        (self.kits[index].generation == stamp.generation).then_some(index)
    }

    /// Focus the kit a piece of navigation state belongs to, before acting on
    /// it. Navigation names tags by key, and a key only means something within
    /// its own kit — so a jump, reveal, or results row has to return to the kit
    /// it came from. Returns false when that kit has closed, in which case the
    /// navigation is dropped rather than applied to whichever kit is active.
    pub(super) fn focus_navigation_kit(&mut self, kit: KitId) -> bool {
        match self.kit_index(kit) {
            Some(index) => {
                self.active = index;
                // Bring its workspace tab to the front too. `active` alone only
                // decides where the action lands; if that workspace is a
                // background tab the user is still looking at another game, and
                // a jump or a confirmed dialog reads as having done nothing.
                // A split needs no help here — both panes are already visible,
                // and `make_active` leaves a pane that is not in a tab group
                // alone.
                self.kit_tree.make_active(
                    |_, tile| matches!(tile, egui_tiles::Tile::Pane(id) if *id == kit),
                );
                true
            }
            None => false,
        }
    }

    /// Resolve a kit id to its index, ignoring generation. For results that
    /// stay valid across a source reload, such as a parsed document.
    pub(super) fn resolve_kit(&self, kit: KitId) -> Option<usize> {
        self.kit_index(kit)
    }

    /// The active kit's source, or `None` for an empty workspace.
    pub(super) fn source(&self) -> Option<&LoadedSourceData> {
        self.kits[self.active].source.as_ref()
    }

    pub(super) fn source_mut(&mut self) -> Option<&mut LoadedSourceData> {
        self.kits[self.active].source.as_mut()
    }

    pub(super) fn names(&self) -> &TagNameIndex {
        &self.kits[self.active].names
    }

    /// Allocate the next never-reused kit id.
    #[allow(dead_code)]
    pub(super) fn next_kit_id(&mut self) -> KitId {
        let id = KitId(self.next_kit_id);
        self.next_kit_id = self.next_kit_id.wrapping_add(1);
        id
    }

    /// Build an empty kit carrying a fresh id and the application defaults,
    /// including the browser view a new workspace opens in. Every kit is made
    /// here so no path can miss the seeding and open in the wrong view.
    fn empty_kit(&mut self) -> Kit {
        let id = self.next_kit_id();
        Kit {
            browser_mode: self.default_browser_mode,
            browser_sort: self.default_browser_sort,
            ..Kit::empty(id, self.default_names.clone())
        }
    }

    /// Add an empty kit and make it active. The next load installs into it,
    /// so "open another game" is add-then-load rather than a separate path.
    pub(super) fn add_kit(&mut self) -> KitId {
        let kit = self.empty_kit();
        let id = kit.id;
        self.kits.push(kit);
        self.active = self.kits.len() - 1;
        id
    }

    /// Remove a kit, dropping its documents and caches. `kits` is never left
    /// empty — closing the last one leaves a fresh empty workspace, which is
    /// the same state Baboon starts in.
    pub(super) fn remove_kit(&mut self, id: KitId) {
        let Some(index) = self.kit_index(id) else {
            return;
        };
        let closing_campaign_evolved = self.kits[index]
            .source
            .as_ref()
            .is_some_and(|source| matches!(&source.source, TagSource::IoStoreContainerSet { .. }));
        if closing_campaign_evolved {
            self.reset_runtime_poke_source_state();
        }
        self.kits.remove(index);
        if self.kits.is_empty() {
            let kit = self.empty_kit();
            self.kits.push(kit);
        }
        self.active = active_after_removal(self.active, index, self.kits.len());
    }

    /// Whether any kit holds unsaved edits.
    pub(super) fn any_kit_dirty(&self) -> bool {
        self.kits.iter().any(kit_has_dirty_documents)
    }

    /// Index of the first kit holding unsaved edits.
    pub(super) fn first_dirty_kit(&self) -> Option<usize> {
        self.kits.iter().position(kit_has_dirty_documents)
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
    pub(super) fn open_kit_for(&mut self, path: &Path) -> bool {
        let path = clean_recent_path(path.to_path_buf());
        if let Some(index) = self
            .kits
            .iter()
            .position(|kit| requested_path_matches(kit, &path))
        {
            self.active = index;
            return true;
        }
        if !self.kits[self.active].can_accept_source_load() {
            self.add_kit();
        }
        self.kits[self.active].requested_path = Some(path);
        false
    }

    /// Release a source-load reservation after the worker or synchronous
    /// preflight fails. The kit may then be reused for a later open request.
    /// All state staged specifically for that failed load is discarded with
    /// the reservation so it cannot leak into the next source.
    pub(super) fn release_source_load(&mut self, kit: KitId) {
        let Some(index) = self.kit_index(kit) else {
            return;
        };
        self.kits[index].release_source_load();
    }

    /// Install a freshly loaded source into the active kit, replacing whatever
    /// it held. Multi-kit loading (add-a-kit rather than replace) arrives with
    /// the kit strip; until then this preserves single-source behavior.
    pub(super) fn install_loaded_source(&mut self, source: LoadedSourceData) {
        let mut names = source.names.clone();
        names.merge_missing(self.default_names.clone());
        let id = self.active_kit_id();
        let index = self.active;
        // The requested path outlives the load it started, so a later open of
        // the same folder can find this kit.
        let requested_path = self.kits[index].requested_path.clone();
        let profile = self.kits[index].profile.clone();
        let pending_restore_tags = std::mem::take(&mut self.kits[index].pending_restore_tags);
        let pending_restore_chimp_packages =
            std::mem::take(&mut self.kits[index].pending_restore_chimp_packages);
        let pending_restore_active_chimp_package =
            self.kits[index].pending_restore_active_chimp_package.take();
        let pending_launch_tags = self.kits[index].pending_launch_tags.take();
        let pending_campaign_project =
            std::mem::take(&mut self.kits[index].pending_campaign_project);
        // The browser view belongs to the workspace, not to the source in it:
        // reloading a kit — or restoring one, which stages the saved view
        // before the load lands — must not snap it back to the default.
        let browser_mode = self.kits[index].browser_mode;
        let browser_sort = self.kits[index].browser_sort;
        self.kits[index] = Kit {
            source: Some(source),
            names,
            requested_path,
            profile,
            browser_mode,
            browser_sort,
            pending_restore_tags,
            pending_restore_chimp_packages,
            pending_restore_active_chimp_package,
            pending_launch_tags,
            pending_campaign_project,
            ..Kit::empty(id, self.default_names.clone())
        };
    }
}

fn requested_path_matches(kit: &Kit, path: &Path) -> bool {
    kit.requested_path
        .as_deref()
        .is_some_and(|open| same_recent_path(open, path))
}

fn kit_has_dirty_documents(kit: &Kit) -> bool {
    kit.parsed_tags
        .iter()
        .any(|(key, document)| {
            document.dirty.is_set() && document_edits_are_saveable(kit, key, document)
        })
        || kit.chimp.documents.values().any(|document| document.dirty)
}

/// Whether this kit's edits to `key` could be written back at all.
///
/// A monolithic build (and any other tag with no writer) is editable but never
/// saveable, so its dirty flag is not unsaved *work*: counting it as such would
/// raise a save prompt on every close whose only honest answer is Discard.
pub(super) fn document_edits_are_saveable(kit: &Kit, key: &str, document: &TagDocument) -> bool {
    // An entry the kit can no longer find is not one this can rule out.
    kit.entry_for_key(key)
        .is_none_or(|entry| is_saveable_tag(entry, &document.tag))
}

/// Label for a kit in the kit strip: the game's display name where one was
/// detected, otherwise the source label, otherwise an empty workspace.
pub(super) fn kit_strip_label(kit: &Kit) -> String {
    let Some(source) = kit.source.as_ref() else {
        return "New workspace".to_owned();
    };
    if let Some(profile) = &kit.profile {
        return profile.name.clone();
    }
    match source.game.as_deref() {
        Some(game) => game_display_name(game).to_owned(),
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

#[cfg(test)]
mod tests {
    use super::{Kit, KitId, TagDocument, active_after_removal, kit_has_dirty_documents};
    use super::EditingKitProfileIdentity;
    use crate::app::test_definition_path;
    use crate::source::{LoadedSourceData, TagEntry, TagEntryLocation, TagSource, build_tree};
    use blam_tags::TagFile;
    use std::collections::HashMap;
    use std::path::Path;
    use std::path::PathBuf;

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
        kit.open_tag_pane("file:/tags/objects/a.weapon");
        kit.open_tag_pane("file:/tags/objects/b.weapon");
        kit.selected_key = Some("file:/tags/objects/a.weapon".to_owned());

        let mut map = HashMap::new();
        map.insert(
            "file:/tags/objects/a.weapon".to_owned(),
            "file:/tags/moved/a.weapon".to_owned(),
        );
        kit.remap_tag_keys(&map);

        // Read back through the tree, not the cached list, so the assertion
        // fails if only the list was rewritten.
        let panes = kit.tabs_from_tree();
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
        kit.pending_launch_tags = Some(vec![PathBuf::from("objects/example.weapon")]);

        kit.release_source_load();

        assert!(kit.can_accept_source_load());
        assert!(kit.profile.is_none());
        assert!(kit.pending_launch_tags.is_none());
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
}

/// egui id for a kit's tag layout tree. Distinct per kit so two trees rendered
/// in the same frame keep separate drag state.
pub(super) fn tag_tree_id(id: KitId) -> egui::Id {
    egui::Id::new(("kit_tag_tree", id.0))
}

impl Kit {
    /// This kit's browser entry for `key`, wherever it is listed: the visible
    /// entries, the full set a filtered browser hides, or a favorite pulled in
    /// from elsewhere.
    pub(super) fn entry_for_key(&self, key: &str) -> Option<&TagEntry> {
        let source = self.source.as_ref()?;
        source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .chain(self.active_favorite_entries.iter())
            .find(|entry| entry.key == key)
    }

    /// Tag keys currently laid out, in tab order. Derived from the tree, which
    /// owns the layout; callers treat `open_tabs` as a read-only view.
    pub(super) fn tabs_from_tree(&self) -> Vec<String> {
        self.tag_tree
            .tiles
            .tiles()
            .filter_map(|tile| match tile {
                egui_tiles::Tile::Pane(key) => Some(key.clone()),
                egui_tiles::Tile::Container(_) => None,
            })
            .collect()
    }

    /// Rewrite every open tag key through `map`, after a move or rename has
    /// changed the keys underneath them.
    ///
    /// The tree is where this has to land: `open_tabs` is re-derived from it
    /// every frame, so remapping only the list is overwritten immediately and
    /// the panes keep pointing at keys their source no longer has.
    pub(super) fn remap_tag_keys(&mut self, map: &HashMap<String, String>) {
        for (_, tile) in self.tag_tree.tiles.iter_mut() {
            if let egui_tiles::Tile::Pane(key) = tile
                && let Some(new_key) = map.get(key)
            {
                *key = new_key.clone();
            }
        }
        if let Some(selected) = self.selected_key.as_ref()
            && let Some(new_key) = map.get(selected)
        {
            self.selected_key = Some(new_key.clone());
        }
        self.sync_open_tabs();
    }

    /// Re-derive `open_tabs` from the tree. Called after anything that can
    /// change the layout: a frame of `tree.ui`, an open, or a close.
    pub(super) fn sync_open_tabs(&mut self) {
        self.open_tabs = self.tabs_from_tree();
        if self
            .selected_key
            .as_ref()
            .is_some_and(|key| !self.open_tabs.contains(key))
        {
            self.selected_key = self.open_tabs.first().cloned();
        }
    }

    /// Add `key` as a pane if it is not already laid out, and select it.
    pub(super) fn open_tag_pane(&mut self, key: &str) {
        if let Some(tile_id) = self.tile_for_key(key) {
            self.tag_tree.make_active(|id, _| id == tile_id);
        } else {
            let tile_id = self.tag_tree.tiles.insert_pane(key.to_owned());
            match self.tag_tree.root() {
                Some(root) => {
                    if let Some(egui_tiles::Tile::Container(container)) =
                        self.tag_tree.tiles.get_mut(root)
                    {
                        container.add_child(tile_id);
                    } else {
                        // A bare pane at the root: wrap both in a tab group.
                        let tabs = self.tag_tree.tiles.insert_tab_tile(vec![root, tile_id]);
                        self.tag_tree.root = Some(tabs);
                    }
                }
                None => self.tag_tree.root = Some(tile_id),
            }
            self.tag_tree.make_active(|id, _| id == tile_id);
        }
        self.selected_key = Some(key.to_owned());
        self.sync_open_tabs();
    }

    /// Open `key` as a pane split beside the existing layout, rather than as
    /// another tab in the same group. This is what alt-click does — the
    /// successor to tearing a tag out into its own window.
    pub(super) fn open_tag_pane_beside(&mut self, key: &str) {
        if self.tile_for_key(key).is_some() {
            self.open_tag_pane(key);
            return;
        }
        let tile_id = self.tag_tree.tiles.insert_pane(key.to_owned());
        match self.tag_tree.root() {
            Some(root) => {
                let split = self
                    .tag_tree
                    .tiles
                    .insert_horizontal_tile(vec![root, tile_id]);
                self.tag_tree.root = Some(split);
            }
            None => self.tag_tree.root = Some(tile_id),
        }
        self.selected_key = Some(key.to_owned());
        self.sync_open_tabs();
    }

    /// Remove `key`'s pane from the layout.
    pub(super) fn close_tag_pane(&mut self, key: &str) {
        if let Some(tile_id) = self.tile_for_key(key) {
            self.tag_tree.remove_recursively(tile_id);
        }
        self.sync_open_tabs();
    }

    fn tile_for_key(&self, key: &str) -> Option<egui_tiles::TileId> {
        self.tag_tree
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(pane) if pane == key => Some(*id),
                _ => None,
            })
    }
}
