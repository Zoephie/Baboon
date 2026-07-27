//! documents application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;

/// Whether a document diverges from its last save, and how many times it has
/// been changed.
///
/// The count exists so anything that caches a document's serialized bytes can
/// tell "still the same tag" from "edited again" without serializing it. The
/// Campaign Evolved autosave used to re-serialize every dirty document twice a
/// second purely to discover nothing had changed — 100 ms a tick with a 105 MiB
/// animation graph open.
#[derive(Default)]
pub(in crate::app) struct Dirty {
    set: bool,
    revision: u64,
}

impl Dirty {
    pub(in crate::app) fn is_set(&self) -> bool {
        self.set
    }

    /// Monotonic across saves: clearing the flag must not let a later edit
    /// reuse a revision some cache has already seen.
    pub(in crate::app) fn revision(&self) -> u64 {
        self.revision
    }

    pub(in crate::app) fn touch(&mut self) {
        self.set = true;
        self.revision = self.revision.wrapping_add(1);
    }

    pub(in crate::app) fn clear(&mut self) {
        self.set = false;
    }
}

/// Parsed tag plus its unsaved-state and byte-snapshot edit history.
/// `dirty` reflects divergence from the last successful save, while journal
/// entries may still exist after saving to support later undo operations.
pub(in crate::app) struct TagDocument {
    pub(in crate::app) tag: TagFile,
    pub(in crate::app) dirty: Dirty,
    pub(in crate::app) journal: EditJournal,
}

impl TagDocument {
    pub(in crate::app) fn clean(tag: TagFile) -> Self {
        Self {
            tag,
            dirty: Dirty::default(),
            journal: EditJournal::default(),
        }
    }

    /// A document that starts life already modified — used for tags that exist
    /// only in memory (a newly-created or imported container tag with no backing
    /// payload on disk or in a pak), so closing it prompts to save and it feeds
    /// Export Mod as a modified tag.
    pub(in crate::app) fn modified(tag: TagFile) -> Self {
        let mut dirty = Dirty::default();
        dirty.touch();
        Self {
            tag,
            dirty,
            journal: EditJournal::default(),
        }
    }
}

#[derive(Clone, Debug)]
/// Close transaction retained while the save/discard prompt spans UI frames.
/// Key-bearing variants use the same stable keys as `parsed_tags` and tabs.
pub(in crate::app) enum PendingCloseAction {
    CloseApp,
    CloseTab(String),
    CloseAllTabs,
    CloseAllButThis(String),
    /// Close a whole kit, discarding its documents and caches.
    CloseKit(KitId),
}

pub(in crate::app) struct DirtyTagEntry {
    pub(in crate::app) path: String,
    pub(in crate::app) tag_id: String,
    pub(in crate::app) checked: bool,
}

/// Foundation-style confirmation shown when a close action would discard
/// edited tags. `allow_app_close_once` is set only after the user confirms an
/// app exit; the next native close request is then allowed through instead of
/// being vetoed and prompting again.
pub(in crate::app) struct SaveChangesPrompt {
    pub(in crate::app) visible: bool,
    /// Whether this workspace can hold edits in a Baboon project rather than
    /// writing them into the game. Container sources can; a loose kit has
    /// nowhere to stash to, so it is offered Save or nothing.
    pub(in crate::app) can_stash: bool,
    pub(in crate::app) dirty_tags: Vec<DirtyTagEntry>,
    pub(in crate::app) pending_action: PendingCloseAction,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) allow_app_close_once: bool,
}

impl Default for SaveChangesPrompt {
    fn default() -> Self {
        Self {
            visible: false,
            can_stash: false,
            dirty_tags: Vec::new(),
            pending_action: PendingCloseAction::CloseApp,
            error: None,
            allow_app_close_once: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum LastSessionSourceKind {
    SingleFile,
    LooseFolder,
    MonolithicCache,
    IoStoreContainerSet,
}

impl LastSessionSourceKind {
    pub(in crate::app) fn as_str(self) -> &'static str {
        match self {
            LastSessionSourceKind::SingleFile => "single_file",
            LastSessionSourceKind::LooseFolder => "loose_folder",
            LastSessionSourceKind::MonolithicCache => "monolithic_cache",
            LastSessionSourceKind::IoStoreContainerSet => "iostore_container_set",
        }
    }

    pub(in crate::app) fn from_str(value: &str) -> Option<Self> {
        match value {
            "single_file" => Some(Self::SingleFile),
            "loose_folder" => Some(Self::LooseFolder),
            "monolithic_cache" => Some(Self::MonolithicCache),
            "iostore_container_set" => Some(Self::IoStoreContainerSet),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionTag {
    pub(in crate::app) key: String,
    pub(in crate::app) label: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) path: Option<PathBuf>,
}

/// One kit's worth of saved session: its source and the tags it had open.
#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionKit {
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) game: Option<String>,
    pub(in crate::app) project_path: Option<PathBuf>,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) tags: Vec<LastSessionTag>,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionState {
    pub(in crate::app) kits: Vec<LastSessionKit>,
}

/// One kit to reopen during a session restore.
pub(in crate::app) struct RestoreKit {
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) project_path: Option<PathBuf>,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) tags: Vec<LastSessionTag>,
}

pub(in crate::app) struct LastOpenedWindowEntry {
    pub(in crate::app) tag: LastSessionTag,
    pub(in crate::app) checked: bool,
    pub(in crate::app) available: bool,
}

/// One kit's section of the restore prompt.
pub(in crate::app) struct LastOpenedWindowsKit {
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) game: Option<String>,
    pub(in crate::app) source_available: bool,
    pub(in crate::app) project_path: Option<PathBuf>,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) entries: Vec<LastOpenedWindowEntry>,
}

/// Launch-time restore prompt backed by `last_session.json`. OK reloads each
/// saved kit's source; as each async load completes, that kit's queued tag
/// keys are reopened through the normal `select_entry` path. Restores are
/// independent, so the kits can finish loading in any order.
pub(in crate::app) struct LastOpenedWindowsPrompt {
    pub(in crate::app) visible: bool,
    pub(in crate::app) kits: Vec<LastOpenedWindowsKit>,
    /// "Don't ask again": on OK, remember as Always; on Cancel, as Never.
    pub(in crate::app) dont_ask_again: bool,
}

impl LastOpenedWindowsKit {
    fn from_saved(saved: LastSessionKit) -> Option<Self> {
        let source_available = match saved.source_kind {
            LastSessionSourceKind::SingleFile => saved.source_path.is_file(),
            LastSessionSourceKind::LooseFolder => saved.source_path.is_dir(),
            LastSessionSourceKind::MonolithicCache => {
                if saved.source_path.is_dir() {
                    saved.source_path.join("blob_index.dat").is_file()
                } else {
                    saved.source_path.is_file()
                        && saved
                            .source_path
                            .file_name()
                            .is_some_and(|name| name.eq_ignore_ascii_case("blob_index.dat"))
                }
            }
            LastSessionSourceKind::IoStoreContainerSet => {
                crate::source::find_paks_dir(&saved.source_path).is_some()
            }
        };
        let entries = saved
            .tags
            .into_iter()
            .map(|tag| {
                let tag_available = tag.path.as_ref().map(|path| path.exists()).unwrap_or(true);
                let available = source_available && tag_available;
                LastOpenedWindowEntry {
                    tag,
                    checked: available,
                    available,
                }
            })
            .collect::<Vec<_>>();
        if entries.is_empty() && saved.project_path.is_none() {
            return None;
        }
        Some(Self {
            source_kind: saved.source_kind,
            source_path: saved.source_path,
            game: saved.game,
            source_available,
            project_path: saved.project_path,
            browser_mode: saved.browser_mode,
            browser_sort: saved.browser_sort,
            entries,
        })
    }

    pub(in crate::app) fn checked_tags(&self) -> Vec<LastSessionTag> {
        self.entries
            .iter()
            .filter(|entry| entry.available && entry.checked)
            .map(|entry| entry.tag.clone())
            .collect()
    }
}

impl LastOpenedWindowsPrompt {
    pub(in crate::app) fn from_session(session: LastSessionState) -> Option<Self> {
        let kits = session
            .kits
            .into_iter()
            .filter_map(LastOpenedWindowsKit::from_saved)
            .collect::<Vec<_>>();
        if kits.is_empty() {
            return None;
        }
        Some(Self {
            visible: true,
            kits,
            dont_ask_again: false,
        })
    }

    /// Every kit that still has something checked, paired with those tags.
    /// Every kit worth reopening, with the tags checked for it. A kit with no
    /// checked tags is still restored when it carries a project, since the
    /// project is the session.
    pub(in crate::app) fn checked_kits(&self) -> Vec<RestoreKit> {
        self.kits
            .iter()
            .filter_map(|kit| {
                let tags = kit.checked_tags();
                (!tags.is_empty() || kit.project_path.is_some()).then(|| RestoreKit {
                    source_kind: kit.source_kind,
                    source_path: kit.source_path.clone(),
                    project_path: kit.project_path.clone(),
                    browser_mode: kit.browser_mode,
                    browser_sort: kit.browser_sort,
                    tags,
                })
            })
            .collect()
    }

    pub(in crate::app) fn has_checked_tags(&self) -> bool {
        self.kits
            .iter()
            .any(|kit| !kit.checked_tags().is_empty() || kit.project_path.is_some())
    }
}


/// Import-a-tag-file dialog for a Campaign Evolved container source. Owns the
/// parsed imported `TagFile` (moved out on confirm) and the schema-comparison
/// result against our shipped JSON. Not `Clone` — `TagFile` isn't cloneable.
pub(in crate::app) struct ImportTagDialog {
    /// Workspace this was raised from. The confirm applies against the active
    /// kit, and a modeless dialog outlives the frame that opened it, so the
    /// user can focus another game in between; resolving this first is what
    /// keeps the action on the workspace it was started in.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) source_path: PathBuf,
    /// Pre-filled container folder (empty for the root); the leaf name is `name`.
    pub(in crate::app) folder_rel: String,
    pub(in crate::app) name: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) group_name: String,
    pub(in crate::app) extension: String,
    /// The parsed imported tag; `take()`n when the user confirms.
    pub(in crate::app) tag: Option<TagFile>,
    /// Structural comparison of the imported tag against our JSON definition.
    pub(in crate::app) comparison: Option<blam_tags::LayoutComparison>,
    /// User override for benign (field-count) schema drift.
    pub(in crate::app) import_anyway: bool,
    pub(in crate::app) error: Option<String>,
}

/// A parsed imported tag awaiting a "discard unsaved edits?" confirmation before
/// it overwrites an already-open, dirty document at `target_key`.
/// Pending "throw away everything this workspace has not written into the
/// game" confirmation, listing what it is about to drop.
/// One tag the export is about to write, as reviewed before writing.
pub(in crate::app) struct ModExportRow {
    pub(in crate::app) identity: String,
    pub(in crate::app) display_path: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) kind: ModExportChange,
    pub(in crate::app) include: bool,
    pub(in crate::app) bytes: usize,
    /// Why this tag cannot be exported, when it cannot.
    pub(in crate::app) reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ModExportChange {
    /// A tag this workspace created, with no counterpart in the game.
    New,
    /// An edit to a tag the game ships.
    Modified,
    /// In the workspace's project, but no longer resolvable in this source.
    Unresolved,
}

/// A modified tag's field-level differences, computed when its row is first
/// expanded and kept for as long as the review is open.
pub(in crate::app) struct ModRowDiff {
    pub(in crate::app) rows: Vec<TagFieldDiff>,
    /// The two tags the rows were computed from, kept so each change can be
    /// rendered through the real field editor rather than described.
    pub(in crate::app) base: Option<blam_tags::TagFile>,
    pub(in crate::app) edited: Option<blam_tags::TagFile>,
    pub(in crate::app) truncated: bool,
    pub(in crate::app) error: Option<String>,
}

/// Review of what Export Mod is about to write, shown before anything is
/// written and before a destination is chosen.
///
/// Holds the captured snapshot rather than re-deriving one on confirm, so what
/// was reviewed and what is written cannot disagree.
pub(in crate::app) struct ModExportDialog {
    pub(in crate::app) kit: KitId,
    /// Opened to look rather than to export: the same review without a
    /// destination or an Export button.
    pub(in crate::app) review_only: bool,
    pub(in crate::app) snapshot: CampaignProjectSnapshot,
    pub(in crate::app) rows: Vec<ModExportRow>,
    pub(in crate::app) name: String,
    pub(in crate::app) folder: PathBuf,
    /// True once the user has accepted overwriting the files already there.
    pub(in crate::app) overwrite_acknowledged: bool,
    /// Rows the user has opened. Diffs are computed on first expansion rather
    /// than up front: each one costs a container read and two parses, which
    /// would make opening the review scale with how much is stashed.
    pub(in crate::app) expanded: HashSet<String>,
    pub(in crate::app) diffs: HashMap<String, ModRowDiff>,
}

/// Fold a mod name into a file-safe stem, keeping its capitalisation.
///
/// The name becomes three file names in a folder the user never types, so
/// spaces and punctuation are separators to normalise rather than characters
/// to carry through. Anything that is not a letter, digit, hyphen or
/// underscore becomes a hyphen, runs collapse, and the ends are trimmed --
/// kebab case, with the user's own casing left alone.
///
/// Underscores survive deliberately: `_P` marks a mod's priority, and folding
/// it to `-P` would leave a second one appended.
pub(in crate::app) fn sanitize_mod_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

impl ModExportDialog {
    /// The file stem the mod will be written under.
    ///
    /// `_P` is what gives an override container priority over the game's own,
    /// so it is part of the name rather than something the user can leave off.
    /// The game folds case before comparing, so a name already ending `_p` is
    /// left as it is.
    pub(in crate::app) fn stem(&self) -> String {
        let name = sanitize_mod_name(&self.name);
        if name.len() >= 2 && name[name.len() - 2..].eq_ignore_ascii_case("_p") {
            name
        } else {
            format!("{name}_P")
        }
    }

    pub(in crate::app) fn included(&self) -> impl Iterator<Item = &ModExportRow> {
        self.rows.iter().filter(|row| row.include)
    }

    /// Files that already exist where this would be written. A mod is three
    /// files plus its project sidecar, and only the container was ever guarded.
    pub(in crate::app) fn existing_files(&self) -> Vec<String> {
        let stem = self.stem();
        ["utoc", "ucas", "pak", "baboon"]
            .into_iter()
            .map(|extension| format!("{stem}.{extension}"))
            .filter(|name| self.folder.join(name).exists())
            .collect()
    }
}

/// What Export Mod just wrote, so the app can say what to do with it.
///
/// A mod is three files, and only the `.pak` looks like one. The status line
/// used to carry the instruction and no longer did; it also clears itself after
/// a few seconds, which is not long enough to act on.
pub(in crate::app) struct ExportedMod {
    pub(in crate::app) stem: String,
    pub(in crate::app) directory: PathBuf,
    pub(in crate::app) count: usize,
    pub(in crate::app) skipped: usize,
}

pub(in crate::app) struct ClearStashConfirm {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) stashed: Vec<String>,
    pub(in crate::app) unsaved: usize,
}

/// Pending "Save will overwrite the game's paks in place" confirmation.
///
/// Carries its workspace for the same reason [`PendingImport`] does: the
/// confirm is modeless, and an in-place container overwrite is the last thing
/// that should land on whichever game happens to be focused when it is
/// answered.
pub(in crate::app) struct OverwriteConfirm {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
}

pub(in crate::app) struct PendingImport {
    /// Workspace this was raised from. The confirm applies against the active
    /// kit, and a modeless dialog outlives the frame that opened it, so the
    /// user can focus another game in between; resolving this first is what
    /// keeps the action on the workspace it was started in.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) tag: TagFile,
    pub(in crate::app) target_key: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct NewTagGroup {
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) name: String,
    pub(in crate::app) schema_path: PathBuf,
    pub(in crate::app) extension: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct NewTagDialog {
    /// Workspace the dialog was opened for. `None` before it is first
    /// opened, since this dialog is always resident rather than optional.
    pub(in crate::app) kit: Option<KitId>,
    pub(in crate::app) game: String,
    pub(in crate::app) rel_path: String,
    pub(in crate::app) output_path: Option<PathBuf>,
    pub(in crate::app) groups: Vec<NewTagGroup>,
    pub(in crate::app) selected_group: usize,
    pub(in crate::app) error: Option<String>,
}

impl Default for NewTagDialog {
    fn default() -> Self {
        Self {
            kit: None,
            game: "halo3_mcc".to_owned(),
            rel_path: String::new(),
            output_path: None,
            groups: Vec::new(),
            selected_group: 0,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::app) struct TagFieldDiff {
    /// Path in the edited tag.
    pub(in crate::app) path: String,
    /// Path in the tag as shipped, when it differs -- deleting an element
    /// shifts every index below it, so the same field lives at two paths and a
    /// side-by-side view needs both to find it.
    pub(in crate::app) base_path: Option<String>,
    pub(in crate::app) a: String,
    pub(in crate::app) b: String,
}

/// State for the "Compare Tags" window: tag A (fixed to the launch tag), the
/// chosen tag B, and the computed diff (once "Compare" is clicked).
pub(in crate::app) struct TagDiffState {
    /// The kit both tags are read from.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) a_key: String,
    /// Open-tab key of tag B (when B is an open tag); `None` when B was picked
    /// from disk (then `results`/`b_display` are set directly).
    pub(in crate::app) b_key: Option<String>,
    /// Display label for tag B (open key or picked disk path).
    pub(in crate::app) b_display: Option<String>,
    pub(in crate::app) results: Option<TagDiffResults>,
}

pub(in crate::app) struct TagDiffResults {
    pub(in crate::app) diffs: Vec<TagFieldDiff>,
    /// True when the diff hit the cap and more differences exist.
    pub(in crate::app) truncated: bool,
}
