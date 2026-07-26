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
    /// Subset of `open_tabs` currently rendered as independent windows.
    pub(super) floating_tabs: HashSet<String>,
    /// Transient text-entry buffers keyed by stable widget/edit identifiers.
    pub(super) edit_buffers: HashMap<String, String>,

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

    // --- Per-tag "Search fields" state ---
    pub(super) field_search: HashMap<String, String>,
    /// The last query actually applied per tag, so the collapse is a one-shot
    /// on change rather than a per-frame override the user can't fight.
    pub(super) field_search_applied: HashMap<String, String>,

    // --- Browser and index state ---
    pub(super) filter: String,
    pub(super) filter_cache: FilterCache,
    /// Bumped whenever this kit's source or its `all_entries` set is replaced,
    /// so caches and in-flight async results know to recompute or drop against
    /// fresh data. Per kit, so reloading one kit cannot invalidate another's.
    pub(super) generation: u64,
    pub(super) field_index: FieldValueIndex,
    pub(super) keywords: KeywordStore,
    pub(super) active_favorite_entries: Vec<TagEntry>,
    /// True while a background full-scan of this loose-folder source is running.
    pub(super) scanning_entries: bool,

    // --- Per-kit terminal placement ---
    pub(super) terminal_open: bool,
    /// Working directory for terminal commands (game kit root, parent of tags/).
    pub(super) terminal_work_dir: Option<PathBuf>,
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
            floating_tabs: HashSet::new(),
            edit_buffers: HashMap::new(),
            bitmap_previews: HashMap::new(),
            model_previews: HashMap::new(),
            rmdf_cache: HashMap::new(),
            rmop_cache: HashMap::new(),
            ce_sound_bindings: HashMap::new(),
            field_search: HashMap::new(),
            field_search_applied: HashMap::new(),
            filter: String::new(),
            filter_cache: FilterCache::default(),
            generation: 0,
            field_index: FieldValueIndex::default(),
            keywords: KeywordStore::default(),
            active_favorite_entries: Vec::new(),
            scanning_entries: false,
            terminal_open: false,
            terminal_work_dir: None,
        }
    }

    /// Whether this kit is an empty workspace rather than a loaded source.
    /// A lone empty kit is hidden by the kit strip, so an unloaded Baboon
    /// looks exactly as it did when it was single-source.
    #[allow(dead_code)]
    pub(super) fn is_empty_workspace(&self) -> bool {
        self.source.is_none()
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

    /// Install a freshly loaded source into the active kit, replacing whatever
    /// it held. Multi-kit loading (add-a-kit rather than replace) arrives with
    /// the kit strip; until then this preserves single-source behavior.
    pub(super) fn install_loaded_source(&mut self, source: LoadedSourceData) {
        let mut names = source.names.clone();
        names.merge_missing(self.default_names.clone());
        let id = self.active_kit_id();
        let index = self.active;
        self.kits[index] = Kit {
            source: Some(source),
            names,
            ..Kit::empty(id, self.default_names.clone())
        };
    }
}
