//! What a session saves, and the Last Opened Windows prompt built from it.

use super::*;

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

/// One kit's worth of saved session: its source and the tag/folder panes it had open.
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
    pub(in crate::app) folders: Vec<LastSessionFolder>,
    pub(in crate::app) chimp_packages: Vec<String>,
    pub(in crate::app) active_chimp_package: Option<String>,
    /// Whether this workspace had the Bitmap Library tab open.
    ///
    /// Carried as a flag rather than as a `tags` entry: it is not a tag, has no
    /// document behind it, and nothing in the source resolves its pane key — the
    /// tag loop would drop it on the way out and again on the way back in.
    pub(in crate::app) bitmap_library_open: bool,
    /// Whether this workspace had the Model Library tab open, carried the same
    /// way for the same reason.
    pub(in crate::app) model_library_open: bool,
    /// Whether this was the kit the user was looking at. Carried per kit rather
    /// than as an index into the list so that dropping a kit — which the
    /// restore prompt lets the user do — cannot silently re-point it at
    /// whichever workspace slid into that slot.
    pub(in crate::app) was_active: bool,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionFolder {
    pub(in crate::app) rel_path: PathBuf,
    pub(in crate::app) label: String,
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
    pub(in crate::app) folders: Vec<LastSessionFolder>,
    pub(in crate::app) chimp_packages: Vec<String>,
    pub(in crate::app) active_chimp_package: Option<String>,
    /// Whether this workspace had the Bitmap Library tab open.
    ///
    /// Carried as a flag rather than as a `tags` entry: it is not a tag, has no
    /// document behind it, and nothing in the source resolves its pane key — the
    /// tag loop would drop it on the way out and again on the way back in.
    pub(in crate::app) bitmap_library_open: bool,
    /// Whether this workspace had the Model Library tab open, carried the same
    /// way for the same reason.
    pub(in crate::app) model_library_open: bool,
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

pub(in crate::app) struct LastOpenedFolderEntry {
    pub(in crate::app) folder: LastSessionFolder,
    pub(in crate::app) checked: bool,
    pub(in crate::app) available: bool,
}

/// One kit's section of the restore prompt.
pub(in crate::app) struct LastOpenedWindowsKit {
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) game: Option<String>,
    pub(in crate::app) profile_id: Option<String>,
    /// Current display identity resolved from Editing Kits settings. Kept out
    /// of the session file because the stable profile ID lets renames appear
    /// here immediately without rewriting the saved session.
    pub(in crate::app) profile_name: Option<String>,
    pub(in crate::app) profile_root: Option<PathBuf>,
    pub(in crate::app) source_available: bool,
    pub(in crate::app) project_path: Option<PathBuf>,
    pub(in crate::app) has_project: bool,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) entries: Vec<LastOpenedWindowEntry>,
    pub(in crate::app) folder_entries: Vec<LastOpenedFolderEntry>,
    pub(in crate::app) chimp_entries: Vec<LastOpenedChimpEntry>,
    /// Whether this workspace had the Bitmap Library tab open.
    ///
    /// Carried as a flag rather than as a `tags` entry: it is not a tag, has no
    /// document behind it, and nothing in the source resolves its pane key — the
    /// tag loop would drop it on the way out and again on the way back in.
    pub(in crate::app) bitmap_library_open: bool,
    /// Whether this workspace had the Model Library tab open, carried the same
    /// way for the same reason.
    pub(in crate::app) model_library_open: bool,
    pub(in crate::app) active_chimp_package: Option<String>,
    /// Whether this kit was the focused one when the session was saved.
    pub(in crate::app) was_active: bool,
}

/// Launch-time restore prompt backed by `last_session.json`. OK reloads each
/// saved kit's source; as each async load completes, that kit's queued tag and
/// folder panes are reopened through their normal paths. Restores are independent,
/// so the kits can finish loading in any order.
pub(in crate::app) struct LastOpenedWindowsPrompt {
    pub(in crate::app) kits: Vec<LastOpenedWindowsKit>,
    /// "Don't ask again": on OK, remember as Always; on Cancel, as Never.
    pub(in crate::app) dont_ask_again: bool,
}

impl LastOpenedWindowsKit {
    fn from_saved(
        saved: LastSessionKit,
        profile: Option<&CustomEditingKitProfile>,
    ) -> Option<Self> {
        let availability_path = profile
            .map(|profile| profile.root.as_path())
            .unwrap_or(&saved.source_path);
        let source_available = match saved.source_kind {
            LastSessionSourceKind::SingleFile => availability_path.is_file(),
            LastSessionSourceKind::LooseFolder => availability_path.is_dir(),
            LastSessionSourceKind::MonolithicCache => {
                if availability_path.is_dir() {
                    availability_path.join("blob_index.dat").is_file()
                } else {
                    availability_path.is_file()
                        && availability_path
                            .file_name()
                            .is_some_and(|name| name.eq_ignore_ascii_case("blob_index.dat"))
                }
            }
            LastSessionSourceKind::IoStoreContainerSet => {
                crate::core::source::find_paks_dir(availability_path).is_some()
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
        let folder_entries = saved
            .folders
            .into_iter()
            .map(|folder| LastOpenedFolderEntry {
                folder,
                checked: source_available,
                available: source_available,
            })
            .collect::<Vec<_>>();
        Some(Self {
            source_kind: saved.source_kind,
            source_path: saved.source_path,
            game: saved.game,
            profile_id: saved.profile_id,
            profile_name: profile.map(|profile| profile.name.clone()),
            profile_root: profile.map(|profile| profile.root.clone()),
            source_available,
            project_path: saved.project_path,
            has_project: saved.has_project,
            browser_mode: saved.browser_mode,
            browser_sort: saved.browser_sort,
            entries,
            folder_entries,
            chimp_entries,
            active_chimp_package: saved.active_chimp_package,
            // Restored with the workspace rather than offered as a checkbox,
            // like the browser view beside it: the library is a view onto the
            // kit, not a document that could be missing or unsaved.
            bitmap_library_open: saved.bitmap_library_open && source_available,
            model_library_open: saved.model_library_open && source_available,
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

    pub(in crate::app) fn checked_folders(&self) -> Vec<LastSessionFolder> {
        self.folder_entries
            .iter()
            .filter(|entry| entry.available && entry.checked)
            .map(|entry| entry.folder.clone())
            .collect()
    }
}

impl LastOpenedWindowsPrompt {
    pub(in crate::app) fn from_session(
        session: LastSessionState,
        profiles: &[CustomEditingKitProfile],
    ) -> Option<Self> {
        let kits = session
            .kits
            .into_iter()
            .filter_map(|saved| {
                let profile = saved
                    .profile_id
                    .as_deref()
                    .and_then(|id| profiles.iter().find(|profile| profile.id == id));
                LastOpenedWindowsKit::from_saved(saved, profile)
            })
            .collect::<Vec<_>>();
        if kits.is_empty() {
            return None;
        }
        Some(Self {
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
                let folders = kit.checked_folders();
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
                    folders,
                    chimp_packages,
                    active_chimp_package,
                    bitmap_library_open: kit.bitmap_library_open,
                    model_library_open: kit.model_library_open,
                    was_active: kit.was_active,
                }
            })
            .collect()
    }

    pub(in crate::app) fn has_reopenable_kits(&self) -> bool {
        !self.kits.is_empty()
    }
}
