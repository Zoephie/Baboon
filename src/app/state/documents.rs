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

/// Confirmation shared by the Chimp toolbar discard action and the close
/// transaction. `pending_action` is set only when discarding is part of an
/// app/kit close; a toolbar discard has no continuation.
pub(in crate::app) struct ChimpDiscardPrompt {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) packages: Vec<String>,
    pub(in crate::app) pending_action: Option<PendingCloseAction>,
    pub(in crate::app) error: Option<String>,
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
    /// Where discarding would delete from, and how many of the listed tags have
    /// a stashed copy there. Discarding on a stashing workspace is not "close
    /// without writing anything" — it deletes rows out of a file that persists
    /// across sessions — so the prompt has to be able to name both.
    pub(in crate::app) stash_file: Option<PathBuf>,
    pub(in crate::app) stashed: usize,
    /// Set by the first click on Discard. The second click is the one that acts,
    /// which is what keeps a one-click "no thanks" on a quit dialog from
    /// deleting work the user believed was already exported.
    pub(in crate::app) confirm_discard: bool,
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
            stash_file: None,
            stashed: 0,
            confirm_discard: false,
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
    pub(in crate::app) profile_id: Option<String>,
    /// The `.baboon` this kit had open, to reattach as its save target. `None`
    /// for a workspace whose edits only ever lived in its recovery file.
    pub(in crate::app) project_path: Option<PathBuf>,
    /// Whether this kit carried a Baboon project at all. A workspace with a
    /// stash but no named project file is still worth reopening — the project
    /// *is* the session — and that no longer follows from `project_path`.
    pub(in crate::app) has_project: bool,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) tags: Vec<LastSessionTag>,
    pub(in crate::app) chimp_packages: Vec<String>,
    pub(in crate::app) active_chimp_package: Option<String>,
    /// Whether this workspace had the Bitmap Library tab open.
    ///
    /// Carried as a flag rather than as a `tags` entry: it is not a tag, has no
    /// document behind it, and nothing in the source resolves its pane key — the
    /// tag loop would drop it on the way out and again on the way back in.
    pub(in crate::app) bitmap_library_open: bool,
    /// Whether this was the kit the user was looking at. Carried per kit rather
    /// than as an index into the list so that dropping a kit — which the
    /// restore prompt lets the user do — cannot silently re-point it at
    /// whichever workspace slid into that slot.
    pub(in crate::app) was_active: bool,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionState {
    pub(in crate::app) kits: Vec<LastSessionKit>,
}

/// One kit to reopen during a session restore.
pub(in crate::app) struct RestoreKit {
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) profile_id: Option<String>,
    pub(in crate::app) project_path: Option<PathBuf>,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) tags: Vec<LastSessionTag>,
    pub(in crate::app) chimp_packages: Vec<String>,
    pub(in crate::app) active_chimp_package: Option<String>,
    /// Whether this workspace had the Bitmap Library tab open.
    ///
    /// Carried as a flag rather than as a `tags` entry: it is not a tag, has no
    /// document behind it, and nothing in the source resolves its pane key — the
    /// tag loop would drop it on the way out and again on the way back in.
    pub(in crate::app) bitmap_library_open: bool,
    /// Whether this kit was the focused one when the session was saved.
    pub(in crate::app) was_active: bool,
}

pub(in crate::app) struct LastOpenedWindowEntry {
    pub(in crate::app) tag: LastSessionTag,
    pub(in crate::app) checked: bool,
    pub(in crate::app) available: bool,
}

pub(in crate::app) struct LastOpenedChimpEntry {
    pub(in crate::app) package: String,
    pub(in crate::app) checked: bool,
    pub(in crate::app) available: bool,
}

/// One kit's section of the restore prompt.
pub(in crate::app) struct LastOpenedWindowsKit {
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) game: Option<String>,
    pub(in crate::app) profile_id: Option<String>,
    pub(in crate::app) source_available: bool,
    pub(in crate::app) project_path: Option<PathBuf>,
    pub(in crate::app) has_project: bool,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) entries: Vec<LastOpenedWindowEntry>,
    pub(in crate::app) chimp_entries: Vec<LastOpenedChimpEntry>,
    /// Whether this workspace had the Bitmap Library tab open.
    ///
    /// Carried as a flag rather than as a `tags` entry: it is not a tag, has no
    /// document behind it, and nothing in the source resolves its pane key — the
    /// tag loop would drop it on the way out and again on the way back in.
    pub(in crate::app) bitmap_library_open: bool,
    pub(in crate::app) active_chimp_package: Option<String>,
    /// Whether this kit was the focused one when the session was saved.
    pub(in crate::app) was_active: bool,
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
        let chimp_entries = saved
            .chimp_packages
            .into_iter()
            .map(|package| LastOpenedChimpEntry {
                package,
                checked: source_available,
                available: source_available,
            })
            .collect::<Vec<_>>();
        Some(Self {
            source_kind: saved.source_kind,
            source_path: saved.source_path,
            game: saved.game,
            profile_id: saved.profile_id,
            source_available,
            project_path: saved.project_path,
            has_project: saved.has_project,
            browser_mode: saved.browser_mode,
            browser_sort: saved.browser_sort,
            entries,
            chimp_entries,
            active_chimp_package: saved.active_chimp_package,
            // Restored with the workspace rather than offered as a checkbox,
            // like the browser view beside it: the library is a view onto the
            // kit, not a document that could be missing or unsaved.
            bitmap_library_open: saved.bitmap_library_open && source_available,
            was_active: saved.was_active,
        })
    }

    pub(in crate::app) fn checked_tags(&self) -> Vec<LastSessionTag> {
        self.entries
            .iter()
            .filter(|entry| entry.available && entry.checked)
            .map(|entry| entry.tag.clone())
            .collect()
    }

    pub(in crate::app) fn checked_chimp_packages(&self) -> Vec<String> {
        self.chimp_entries
            .iter()
            .filter(|entry| entry.available && entry.checked)
            .map(|entry| entry.package.clone())
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

    /// Every saved workspace, paired with the tags checked for it. A source is
    /// worth reopening even when it has no checked tags or project state: the
    /// workspace itself is part of the user's last session.
    pub(in crate::app) fn checked_kits(&self) -> Vec<RestoreKit> {
        self.kits
            .iter()
            .map(|kit| {
                let tags = kit.checked_tags();
                let chimp_packages = kit.checked_chimp_packages();
                let active_chimp_package = kit
                    .active_chimp_package
                    .clone()
                    .filter(|active| chimp_packages.contains(active));
                RestoreKit {
                    source_kind: kit.source_kind,
                    source_path: kit.source_path.clone(),
                    profile_id: kit.profile_id.clone(),
                    project_path: kit.project_path.clone(),
                    browser_mode: kit.browser_mode,
                    browser_sort: kit.browser_sort,
                    tags,
                    chimp_packages,
                    active_chimp_package,
                    bitmap_library_open: kit.bitmap_library_open,
                    was_active: kit.was_active,
                }
            })
            .collect()
    }

    pub(in crate::app) fn has_reopenable_kits(&self) -> bool {
        !self.kits.is_empty()
    }
}

/// How a picked file has to be turned into a Campaign Evolved tag.
///
/// Import used to ask one question — "does this match the definition we ship?"
/// — and treat every answer short of yes as drift the user could wave through.
/// That conflates two unrelated situations. A tag saved by an older toolset
/// against a drifted Campaign Evolved layout really is safe to wave through.
/// A tag authored for *another game* is not: its root struct can agree while
/// nested structs disagree, so copying the bytes lands a tag the simulation
/// will read at the wrong offsets. Naming which of the two we are looking at
/// is what lets the gate refuse only the second.
pub(in crate::app) enum ImportMode {
    /// The file already carries Campaign Evolved's layout: copy its bytes.
    Native {
        comparison: Option<blam_tags::LayoutComparison>,
        /// Wave through benign field-metadata drift. Offered only when no
        /// other profile claims the file either — it is an answer to "this
        /// toolset wrote the struct slightly differently", never to "this is a
        /// Halo Reach tag".
        import_anyway: bool,
    },
    /// The file matches another game's profile, so its bytes have to be
    /// converted before they can land. `draft` is the analyzed conversion once
    /// one exists; `None` means it has not been run, or cannot be.
    Convert {
        source_game: String,
        draft: Option<TagConversionDraft>,
    },
}

/// How well an imported tag's own layout fits one game's definition of its
/// group.
///
/// Deliberately not `blam_tags::LayoutSeverity`, which compares root structs
/// only. That is the right cost for asking "roughly, is this our group?" and the
/// wrong answer for "can these bytes be copied?" — a Halo Reach animation graph
/// earns a clean `Match` against Campaign Evolved because their root structs are
/// identical, while four nested structs are the wrong size.
#[derive(Debug)]
pub(in crate::app) enum ProfileFit {
    /// Wire-identical all the way down. Bytes written against this profile can
    /// be read as this game's version of the group without reinterpretation.
    Identical,
    /// The same group, but the shapes diverge somewhere below the root. Carries
    /// where, because "differs" without a location is not actionable on a group
    /// the size of `scenario`.
    Diverges(String),
    /// Not this game's version of this group at all.
    WrongGroup,
}

impl ProfileFit {
    pub(in crate::app) fn is_identical(&self) -> bool {
        matches!(self, ProfileFit::Identical)
    }
}

/// Import-a-tag-file dialog for a Campaign Evolved container source. Owns the
/// parsed imported `TagFile` (moved out on confirm) and how it has to be
/// landed. Not `Clone` — `TagFile` isn't cloneable.
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
    /// How this file has to be landed.
    pub(in crate::app) mode: ImportMode,
    /// Every profile that defines this group, with how the imported tag's own
    /// layout fits it. Evidence rather than a decision: it seeds `mode` and
    /// lets the user correct the guess when the file is unusual.
    pub(in crate::app) profile_verdicts: Vec<(String, ProfileFit)>,
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
    /// The mounted mod currently serving this tag, if one is. The comparison
    /// below the row is against the game's own pack either way, so without this
    /// there is nothing to explain why the editor is showing the mod's values.
    pub(in crate::app) overridden_by: Option<String>,
}

impl ModExportDialog {
    /// The container this export would write, which may be one this workspace has
    /// mounted — that is what makes a re-export of an installed mod impossible
    /// while it is open.
    pub(in crate::app) fn output_utoc(&self) -> PathBuf {
        self.destination().join(format!("{}.utoc", self.stem()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ModExportChange {
    /// A tag this workspace created, with no counterpart in the game.
    New,
    /// An edit to a tag the game ships.
    Modified,
    /// In the workspace's stash, but byte-identical to what the game ships.
    ///
    /// Reaching this state does not require the user to have undone anything
    /// deliberately: a value nudged and put back, or an edit that re-encodes to
    /// the same bytes, leaves the document flagged as modified with nothing to
    /// show for it. Listing that as an unexported change is how a review ends up
    /// asserting the user made a change they did not.
    Unchanged,
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
    /// Height of everything drawn below the list, measured on the previous
    /// frame.
    ///
    /// The list is sized as "the window, less this", so the figure has to be the
    /// real one: a hardcoded guess that came out too small grew the window by
    /// the difference every frame, because a resizable egui window expands to
    /// fit its contents and never shrinks back. The naming lines and the
    /// overwrite warning appear conditionally, so there is no one number to
    /// guess -- measuring settles in a frame and cannot run away.
    pub(in crate::app) controls_height: f32,
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
    ///
    /// A name that sanitizes to nothing still gets a stem: the field is shown
    /// live beside the file names it produces, and a half-typed name that
    /// previewed as a bare `_P.utoc` read like a bug. Export stays disabled
    /// until the name is real.
    pub(in crate::app) fn stem(&self) -> String {
        let name = sanitize_mod_name(&self.name);
        let name = if name.is_empty() {
            "mod".to_owned()
        } else {
            name
        };
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
        let destination = self.destination();
        MOD_FILE_EXTENSIONS
            .into_iter()
            .map(|extension| format!("{stem}.{extension}"))
            .filter(|name| destination.join(name).exists())
            .collect()
    }

    /// The directory the mod's files land in: the folder that was chosen, and
    /// nothing appended to it.
    ///
    /// The mod name names the *files*, never a directory. Deriving a folder
    /// from it too meant picking `Paks/~mods` and typing `mymod` wrote to
    /// `Paks/~mods/mymod/`, which is a level of nesting nobody asked for and
    /// which the engine's loader does not require — `~mods` is scanned
    /// recursively, so a triplet sitting directly in it is found either way.
    /// The default folder is already `Paks/~mods` (see `open_mod_review`), so
    /// the obvious export lands in the obvious place with no guessing here.
    pub(in crate::app) fn destination(&self) -> PathBuf {
        self.folder.clone()
    }
}

/// The directory mods are grouped under, and the export's default destination
/// inside the game's own `Paks`.
pub(in crate::app) const MODS_DIR: &str = "~mods";

/// Everything one exported mod is made of, in the order the dialog lists them.
/// The three the engine loads, plus the project sidecar that travels with them.
pub(in crate::app) const MOD_FILE_EXTENSIONS: [&str; 4] = ["utoc", "ucas", "pak", "baboon"];

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

/// Mandatory confirmation for an in-place Campaign Evolved package duplicate.
/// The destination is deliberately only a leaf: its parent, group, extension,
/// and providing container are all captured from the source entry.
pub(in crate::app) struct ContainerDuplicateConfirm {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
    pub(in crate::app) destination_leaf: String,
}

/// Mandatory confirmation for extracting shipped tags out of a container set.
///
/// Not destructive, but expensive enough to be worth a deliberate answer: it is
/// thousands of decompressed reads and file writes, and the user has to know
/// that before it starts rather than three minutes into a frozen-looking
/// window. Carries its workspace like the other modeless confirms.
pub(in crate::app) struct ContainerDumpConfirm {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) output: PathBuf,
    pub(in crate::app) total: usize,
    pub(in crate::app) scope: ContainerDumpScope,
}

/// Which shipped tags an extraction covers.
///
/// The scope is captured when the confirmation is raised rather than re-derived
/// when it is accepted. A modeless confirm outlives the frame that opened it, and
/// a folder's membership is a snapshot of the tree the user right-clicked: making
/// the run re-walk the workspace on accept would let an import or a delete in
/// between silently change what gets written.
pub(in crate::app) enum ContainerDumpScope {
    /// Every `Container` entry in the workspace — the File menu action.
    AllShipped,
    /// Just the tags beneath one browser folder, captured at right-click.
    /// `label` is the folder's display path, for the confirmation wording.
    Folder {
        label: String,
        keys: Vec<String>,
    },
}

/// The outcome of a container write, kept on screen until dismissed.
///
/// These operations rewrite the game's own pak and take long enough that the
/// user has looked away, so their result cannot live in the status bar: it is
/// gone before it can be read, and a failure that scrolls past is a failure that
/// gets reported as "nothing happened". The message is selectable and copyable
/// because the useful ones are too long to retype.
pub(in crate::app) struct OperationNotice {
    pub(in crate::app) title: String,
    pub(in crate::app) message: String,
    pub(in crate::app) failed: bool,
}

/// Which kind of storage a pending delete will act on. The wording and the
/// warnings differ completely: one moves a file, the other rewrites a pak.
pub(in crate::app) enum DeleteKind {
    Loose,
    Container {
        /// The exact pack that will be rewritten, mod or shipped, with its path.
        target_label: String,
    },
}

/// Mandatory confirmation for deleting a tag.
pub(in crate::app) struct DeleteConfirm {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
    pub(in crate::app) display_path: String,
    pub(in crate::app) kind: DeleteKind,
    /// Display paths of the tags that point at this one and will be left
    /// dangling.
    pub(in crate::app) referrers: Vec<String>,
    /// True when no reverse-dependency index was available, so the referrer list
    /// says nothing either way.
    pub(in crate::app) referrers_unavailable: bool,
    pub(in crate::app) has_unsaved_edits: bool,
}

/// Paths of the immutable pre-mutation backup kept beside the target UTOC.
#[derive(Clone, Debug)]
pub(in crate::app) struct DuplicateBackupPaths {
    pub(in crate::app) utoc: PathBuf,
    pub(in crate::app) manifest: PathBuf,
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
    /// Whether the selected group can actually be created, and why not when it
    /// cannot. `None` for games other than Campaign Evolved, where the question
    /// does not arise: a loose tag is a file, not a package with a native class
    /// behind it.
    ///
    /// Cached rather than computed per frame because answering it parses the
    /// game's whole mapping table; it is refreshed when the group or the game
    /// changes, which is the only time it can move.
    pub(in crate::app) authorability: Option<(bool, String)>,
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
            authorability: None,
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

#[cfg(test)]
mod mod_export_tests {
    use super::*;

    fn dialog(name: &str, folder: &str) -> ModExportDialog {
        ModExportDialog {
            kit: KitId(1),
            review_only: false,
            snapshot: CampaignProjectSnapshot {
                game: "haloce_evolved".to_owned(),
                source_path: PathBuf::new(),
                selected_identity: None,
                tabs: Vec::new(),
                overlays: HashMap::new(),
                history: Default::default(),
                folders: Default::default(),
            },
            rows: Vec::new(),
            name: name.to_owned(),
            folder: PathBuf::from(folder),
            overwrite_acknowledged: false,
            expanded: HashSet::new(),
            diffs: HashMap::new(),
            controls_height: 0.0,
        }
    }

    #[test]
    fn the_mod_name_names_the_files_and_not_a_folder() {
        let dialog = dialog("Cool Mod", "D:/Game/Paks/~mods");
        // `_P` belongs to the files, because it is what gives the container
        // priority rather than part of the name the user typed.
        assert_eq!(dialog.destination(), PathBuf::from("D:/Game/Paks/~mods"));
        assert_eq!(dialog.stem(), "Cool-Mod_P");
        assert_eq!(
            dialog.output_utoc(),
            PathBuf::from("D:/Game/Paks/~mods/Cool-Mod_P.utoc")
        );
    }

    #[test]
    fn a_chosen_folder_is_the_folder_written_to() {
        // Every shape of picked folder, including one that is not `~mods` at
        // all: what was picked is where the files go. Anything else made
        // "Browse..." mean "browse to the parent of where I want this".
        for folder in [
            "D:/Game/Paks",
            "D:/Game/Paks/~mods",
            "D:/Game/Paks/~MODS",
            "D:/Somewhere/Else",
        ] {
            let dialog = dialog("coolmod", folder);
            assert_eq!(dialog.destination(), PathBuf::from(folder));
            assert_eq!(
                dialog.output_utoc(),
                PathBuf::from(folder).join("coolmod_P.utoc")
            );
        }
    }

    #[test]
    fn a_name_that_sanitizes_to_nothing_still_names_a_file() {
        // The export button is disabled for an empty name, but the file names
        // are shown while it is being typed and a bare `_P.utoc` reads like a
        // bug rather than like an unfinished name.
        assert_eq!(dialog("///", "D:/Game/Paks").stem(), "mod_P");
    }
}
