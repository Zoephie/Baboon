//! prefs application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum FirstRunPage {
    Storage,
    Interface,
    EditingKits,
}

pub(in crate::app) struct FirstRunWizardState {
    pub(in crate::app) page: FirstRunPage,
    pub(in crate::app) selected_storage: Option<crate::storage::StorageMode>,
    pub(in crate::app) committed_storage: Option<crate::storage::StorageMode>,
    pub(in crate::app) editing_kit_detection_ran: bool,
    pub(in crate::app) validation_error: Option<String>,
}

impl FirstRunWizardState {
    pub(in crate::app) fn new(existing_mode: Option<crate::storage::StorageMode>) -> Self {
        Self {
            page: if existing_mode.is_some() {
                FirstRunPage::Interface
            } else {
                FirstRunPage::Storage
            },
            selected_storage: existing_mode,
            committed_storage: existing_mode,
            editing_kit_detection_ran: false,
            validation_error: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum HelpPanelTab {
    About,
    Doc,
    Tutorials,
    ScriptDoc,
    TagCompat,
    MapNames,
}

/// How nested containers in the tag editor start out.
///
/// `Schema` keeps each container's own judgement — top-level groups and
/// priority sections open, deeply nested blocks closed — which suits browsing
/// an unfamiliar tag. The other two override it outright for people who would
/// rather always start from one end.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(in crate::app) enum NestedDefault {
    #[default]
    Schema,
    Collapsed,
    Expanded,
}

impl NestedDefault {
    pub(in crate::app) const ALL: [NestedDefault; 3] = [
        NestedDefault::Schema,
        NestedDefault::Collapsed,
        NestedDefault::Expanded,
    ];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            NestedDefault::Schema => "Default",
            NestedDefault::Collapsed => "Collapsed",
            NestedDefault::Expanded => "Expanded",
        }
    }

    pub(in crate::app) fn help(self) -> &'static str {
        match self {
            NestedDefault::Schema => {
                "Each group, struct and block decides for itself — top-level \
                 sections open, deeply nested ones closed"
            }
            NestedDefault::Collapsed => "Every nested group, struct and block starts closed",
            NestedDefault::Expanded => "Every nested group, struct and block starts open",
        }
    }

    /// The starting open state for a container whose schema-derived default is
    /// `schema_default`.
    pub(in crate::app) fn applies_to(self, schema_default: bool) -> bool {
        match self {
            NestedDefault::Schema => schema_default,
            NestedDefault::Collapsed => false,
            NestedDefault::Expanded => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum SettingsTab {
    Startup,
    Browser,
    EditingKits,
    Appearance,
    Tools,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct EditingKitFavorites {
    pub(in crate::app) tags_root: PathBuf,
    pub(in crate::app) tags: Vec<PathBuf>,
    pub(in crate::app) folders: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) struct EditingKitShortcut {
    pub(in crate::app) label: &'static str,
    pub(in crate::app) game: &'static str,
    pub(in crate::app) fallback: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct CustomEditingKitProfile {
    pub(in crate::app) read_only: bool,
    pub(in crate::app) git_tracked: bool,
    pub(in crate::app) id: String,
    pub(in crate::app) name: String,
    pub(in crate::app) game: String,
    pub(in crate::app) root: PathBuf,
    /// Relative to the active data directory, with legacy executable-relative lookup.
    /// `None` uses the bundled engine artwork.
    pub(in crate::app) icon: Option<PathBuf>,
    /// The kit's tags and data folders, relative to `root` or absolute; `None`
    /// is the root's own `tags`/`data`. Only kits whose tools accept
    /// `-tags_dir`/`-data_dir` can set them (see [`kit_folders_are_choosable`]).
    pub(in crate::app) tags_folder: Option<PathBuf>,
    pub(in crate::app) data_folder: Option<PathBuf>,
}

/// Whether a kit of `game` can use tags and data folders other than its root's
/// own `tags` and `data`.
///
/// Only the Halo CE and Halo 2 MCC tools take `-tags_dir`/`-data_dir`. The
/// Halo 3-era tools open `tags\` and `data\` relative to the folder they run
/// in and accept no option to change it (checked in IDA for the Halo 3 and
/// Reach `tool.exe`), so a kit editing another folder would have its tools
/// working on a different one.
pub(in crate::app) fn kit_folders_are_choosable(game: &str) -> bool {
    matches!(game, "haloce_mcc" | "halo2_mcc")
}

impl CustomEditingKitProfile {
    /// Whether this profile names a tags or data folder of its own.
    pub(in crate::app) fn has_chosen_folders(&self) -> bool {
        kit_folders_are_choosable(&self.game)
            && (self.tags_folder.is_some() || self.data_folder.is_some())
    }
}

impl CustomEditingKitProfile {
    pub(in crate::app) fn is_read_only_for(
        &self,
        identity: Option<&EditingKitProfileIdentity>,
        root: Option<&Path>,
    ) -> bool {
        let scope = self.read_only_scope();
        self.read_only
            && self.game != "haloce_evolved"
            && (identity.is_some_and(|identity| identity.id == self.id)
                || root.is_some_and(|root| {
                    root.ancestors()
                        .any(|ancestor| same_recent_path(ancestor, &scope))
                }))
    }

    /// The folder a read-only profile protects for tags not opened through it.
    ///
    /// Its root, except for an engine whose kits may share a root: there it is
    /// its tags folder, so a read-only kit doesn't make a writable kit beside
    /// it read-only too. Worked out without touching the disk, since the
    /// banner asks every frame; a default tags folder is taken to be `tags`.
    fn read_only_scope(&self) -> PathBuf {
        if kit_folders_are_choosable(&self.game) {
            self.root
                .join(self.tags_folder.as_deref().unwrap_or(Path::new("tags")))
        } else {
            self.root.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct EditingKitProfileIdentity {
    pub(in crate::app) id: String,
    pub(in crate::app) name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum CustomEditingKitIconDraft {
    Default,
    Existing(PathBuf),
    Selected(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct CustomEditingKitDraft {
    pub(in crate::app) read_only: bool,
    pub(in crate::app) git_tracked: bool,
    pub(in crate::app) editing_id: Option<String>,
    pub(in crate::app) name: String,
    pub(in crate::app) game: String,
    pub(in crate::app) root_input: String,
    /// The tags and data folder inputs, shown only for engines that can use
    /// them. Empty is the root's own folder.
    pub(in crate::app) tags_folder_input: String,
    pub(in crate::app) data_folder_input: String,
    /// Whether each folder input still holds what choosing the root filled in.
    /// Changing the root refills only these, never a folder the user picked.
    pub(in crate::app) tags_folder_auto: bool,
    pub(in crate::app) data_folder_auto: bool,
    pub(in crate::app) icon: CustomEditingKitIconDraft,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) icon_warning: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct CustomEditingKitRemoval {
    pub(in crate::app) id: String,
    pub(in crate::app) name: String,
}

impl CustomEditingKitDraft {
    pub(in crate::app) fn new() -> Self {
        Self {
            read_only: false,
            git_tracked: false,
            editing_id: None,
            name: String::new(),
            game: "halo2_mcc".to_owned(),
            root_input: String::new(),
            tags_folder_input: String::new(),
            data_folder_input: String::new(),
            tags_folder_auto: true,
            data_folder_auto: true,
            icon: CustomEditingKitIconDraft::Default,
            error: None,
            icon_warning: None,
        }
    }

    pub(in crate::app) fn from_profile(profile: &CustomEditingKitProfile) -> Self {
        Self {
            read_only: profile.read_only,
            git_tracked: profile.git_tracked,
            editing_id: Some(profile.id.clone()),
            name: profile.name.clone(),
            game: profile.game.clone(),
            root_input: profile.root.display().to_string(),
            tags_folder_input: folder_input(profile.tags_folder.as_deref()),
            data_folder_input: folder_input(profile.data_folder.as_deref()),
            tags_folder_auto: profile.tags_folder.is_none(),
            data_folder_auto: profile.data_folder.is_none(),
            icon: profile
                .icon
                .clone()
                .map(CustomEditingKitIconDraft::Existing)
                .unwrap_or(CustomEditingKitIconDraft::Default),
            error: None,
            icon_warning: None,
        }
    }
}

fn folder_input(folder: Option<&Path>) -> String {
    folder
        .map(|folder| folder.display().to_string())
        .unwrap_or_default()
}

pub(in crate::app) const EDITING_KIT_SHORTCUTS: [EditingKitShortcut; 8] = [
    EditingKitShortcut {
        label: "HCEEK",
        game: "haloce_mcc",
        fallback: "CE",
    },
    EditingKitShortcut {
        label: "H2EK",
        game: "halo2_mcc",
        fallback: "H2",
    },
    EditingKitShortcut {
        label: "H3EK",
        game: "halo3_mcc",
        fallback: "H3",
    },
    EditingKitShortcut {
        label: "H3ODSTEK",
        game: "halo3odst_mcc",
        fallback: "ODST",
    },
    EditingKitShortcut {
        label: "HREK",
        game: "haloreach_mcc",
        fallback: "R",
    },
    EditingKitShortcut {
        label: "H4EK",
        game: "halo4_mcc",
        fallback: "H4",
    },
    EditingKitShortcut {
        label: "H2AMPEK",
        game: "halo2amp_mcc",
        fallback: "H2A",
    },
    EditingKitShortcut {
        label: "Campaign Evolved",
        game: "haloce_evolved",
        fallback: "HCE",
    },
];

/// What Baboon does with the previous session's open windows on startup.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum SessionRestore {
    /// Ask which windows to reopen (shows the "Last Opened Windows" prompt).
    Ask,
    /// Silently reopen the last session.
    Always,
    /// Start fresh — never reopen and never ask.
    Never,
}

impl SessionRestore {
    pub(in crate::app) fn as_str(self) -> &'static str {
        match self {
            SessionRestore::Ask => "ask",
            SessionRestore::Always => "always",
            SessionRestore::Never => "never",
        }
    }

    pub(in crate::app) fn from_str(value: &str) -> Option<Self> {
        match value {
            "ask" => Some(SessionRestore::Ask),
            "always" => Some(SessionRestore::Always),
            "never" => Some(SessionRestore::Never),
            _ => None,
        }
    }
}

/// Which build track Baboon checks against and points people at.
///
/// The two tracks are what `release.yml` publishes: `v*` tags, and a `dev`
/// prerelease that is deleted and recreated on every push to `main`. Because
/// the development release always carries the same tag, the two are compared
/// against different things — a version number for [`UpdateChannel::Stable`],
/// the build commit for [`UpdateChannel::Development`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(in crate::app) enum UpdateChannel {
    #[default]
    Stable,
    Development,
}

impl UpdateChannel {
    pub(in crate::app) const ALL: [UpdateChannel; 2] =
        [UpdateChannel::Stable, UpdateChannel::Development];

    pub(in crate::app) fn as_str(self) -> &'static str {
        match self {
            UpdateChannel::Stable => "stable",
            UpdateChannel::Development => "development",
        }
    }

    pub(in crate::app) fn from_str(value: &str) -> Option<Self> {
        match value {
            "stable" => Some(UpdateChannel::Stable),
            "development" => Some(UpdateChannel::Development),
            _ => None,
        }
    }

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            UpdateChannel::Stable => "Stable releases",
            UpdateChannel::Development => "Development builds",
        }
    }

    pub(in crate::app) fn help(self) -> &'static str {
        match self {
            UpdateChannel::Stable => {
                "Only published, versioned releases — the tested build most people want"
            }
            UpdateChannel::Development => {
                "The rolling build from the latest commit, rebuilt on every change — \
                 newer features, less testing"
            }
        }
    }
}

/// Memoized search results for the tag browser.
///
/// Filtering the full tag set (100k+ entries) and lowercasing each name is far
/// too expensive to redo every frame while the user types or scrolls. This
/// caches a *pruned* tree containing only the matching tags (in folder- or
/// group-hierarchy form) and only rebuilds it when the query, the source
/// generation, the entry universe (`all_entries` vs `entries`), or the browser
/// mode actually changes — see [`FilterCache::refresh`].
///
/// The pruned tree is rendered with folders collapsed, so the user drills down
/// the same way as the unfiltered tree; collapsed headers don't build their
/// children, which keeps per-frame cost bounded to what's actually expanded.

#[derive(Clone, PartialEq)]
pub(in crate::app) struct GuiPrefs {
    pub(in crate::app) browser_mode: BrowserMode,
    pub(in crate::app) browser_sort: BrowserSort,
    pub(in crate::app) nested_default: NestedDefault,
    pub(in crate::app) show_browser_prefixes: bool,
    pub(in crate::app) folders_before_tags: bool,
    pub(in crate::app) double_click_to_open_tags: bool,
    pub(in crate::app) session_restore: SessionRestore,
    pub(in crate::app) update_channel: UpdateChannel,
    pub(in crate::app) check_updates_on_startup: bool,
    pub(in crate::app) show_block_sizes: bool,
    /// Show angle fields in degrees, as Guerilla does, rather than the
    /// radians they hold on disk. Default on: it is what every other Halo
    /// tool shows, and what the field names themselves claim.
    pub(in crate::app) angles_in_degrees: bool,
    pub(in crate::app) scroll_to_cycle_dropdowns: bool,
    /// Warn before Save overwrites Campaign Evolved pak files in place.
    pub(in crate::app) confirm_container_overwrite: bool,
    /// Show the preflight plan and wait for confirmation before a runtime poke.
    /// Cleared, a poke writes to the running game as soon as it is requested.
    pub(in crate::app) confirm_runtime_poke: bool,
    pub(in crate::app) enable_chimp: bool,
    pub(in crate::app) chimp_output_dir: Option<PathBuf>,
    /// Optional external Unreal mappings used by Chimp instead of the bundled
    /// Campaign Evolved mappings.
    pub(in crate::app) chimp_usmap_path: Option<PathBuf>,
    pub(in crate::app) expert_mode: bool,
    pub(in crate::app) dark_mode: bool,
    pub(in crate::app) ui_scale: f32,
    /// Multiplier on mouse-wheel and trackpad scrolling in scroll areas.
    /// 1.0 is egui's own speed, which is what Baboon always scrolled at.
    pub(in crate::app) scroll_speed: f32,
    /// Multiplier on mouse-wheel zoom in the model and bitmap viewports.
    pub(in crate::app) zoom_speed: f32,
    pub(in crate::app) model_preview_size: f32,
    /// The model preview's projection: perspective, or orthographic when
    /// cleared. Shared by every tag pane and remembered across sessions.
    pub(in crate::app) model_preview_perspective: bool,
    pub(in crate::app) bitmap_preview_view: BitmapPreviewViewSettings,
    pub(in crate::app) blender_path: Option<PathBuf>,
    pub(in crate::app) editing_kit_paths: HashMap<String, PathBuf>,
    pub(in crate::app) ek_folder_aliases: Vec<EkFolderAlias>,
    pub(in crate::app) custom_editing_kit_profiles: Vec<CustomEditingKitProfile>,
    pub(in crate::app) tool_commands_window_pos: Option<egui::Pos2>,
    pub(in crate::app) tool_commands_window_size: Option<Vec2>,
    pub(in crate::app) tool_commands_left_width: f32,
    pub(in crate::app) tool_commands_collapsed_categories: HashSet<String>,
    pub(in crate::app) recent_folders: Vec<PathBuf>,
    pub(in crate::app) editing_kit_favorites: Vec<EditingKitFavorites>,
    pub(in crate::app) custom_color_swatches: Vec<Option<ColorPaletteSwatch>>,
    pub(in crate::app) palette_last_dir: Option<PathBuf>,
    /// Editing-kit profiles and folder aliases naming a game this build does
    /// not support (a newer Baboon's, or none at all), kept as they were read
    /// so saving preferences writes them back. They are never offered as kits.
    pub(in crate::app) unusable_kit_entries: UnusableKitEntries,
}

/// See [`GuiPrefs::unusable_kit_entries`].
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::app) struct UnusableKitEntries {
    pub(in crate::app) profiles: Vec<serde_json::Value>,
    pub(in crate::app) aliases: Vec<serde_json::Value>,
}

impl Default for GuiPrefs {
    fn default() -> Self {
        Self {
            browser_mode: BrowserMode::default(),
            browser_sort: BrowserSort::default(),
            nested_default: NestedDefault::default(),
            show_browser_prefixes: false,
            folders_before_tags: false,
            double_click_to_open_tags: false,
            show_block_sizes: false,
            angles_in_degrees: true,
            scroll_to_cycle_dropdowns: true,
            confirm_container_overwrite: true,
            confirm_runtime_poke: true,
            enable_chimp: true,
            chimp_output_dir: None,
            chimp_usmap_path: None,
            expert_mode: false,
            dark_mode: false,
            ui_scale: DEFAULT_UI_SCALE,
            scroll_speed: DEFAULT_SCROLL_SPEED,
            zoom_speed: DEFAULT_ZOOM_SPEED,
            model_preview_size: DEFAULT_MODEL_PREVIEW_SIZE,
            model_preview_perspective: true,
            bitmap_preview_view: BitmapPreviewViewSettings::default(),
            blender_path: None,
            editing_kit_paths: HashMap::new(),
            session_restore: SessionRestore::Ask,
            update_channel: UpdateChannel::default(),
            check_updates_on_startup: true,
            ek_folder_aliases: Vec::new(),
            custom_editing_kit_profiles: Vec::new(),
            tool_commands_window_pos: None,
            tool_commands_window_size: None,
            tool_commands_left_width: DEFAULT_TOOL_COMMANDS_LEFT_WIDTH,
            tool_commands_collapsed_categories: HashSet::new(),
            recent_folders: Vec::new(),
            editing_kit_favorites: Vec::new(),
            custom_color_swatches: default_color_swatches(),
            palette_last_dir: None,
            unusable_kit_entries: UnusableKitEntries::default(),
        }
    }
}

pub(in crate::app) const DEFAULT_UI_SCALE: f32 = 1.0;
pub(in crate::app) const MIN_UI_SCALE: f32 = 0.6;
pub(in crate::app) const MAX_UI_SCALE: f32 = 1.5;

pub(in crate::app) const DEFAULT_SCROLL_SPEED: f32 = 1.0;
pub(in crate::app) const MIN_SCROLL_SPEED: f32 = 0.5;
pub(in crate::app) const MAX_SCROLL_SPEED: f32 = 4.0;

pub(in crate::app) const DEFAULT_ZOOM_SPEED: f32 = 1.0;
pub(in crate::app) const MIN_ZOOM_SPEED: f32 = 0.25;
pub(in crate::app) const MAX_ZOOM_SPEED: f32 = 4.0;

#[cfg(test)]
mod tests;

pub(in crate::app) const DEFAULT_TOOL_COMMANDS_WINDOW_SIZE: Vec2 = Vec2::new(800.0, 600.0);
pub(in crate::app) const MIN_TOOL_COMMANDS_WINDOW_SIZE: Vec2 = Vec2::new(600.0, 400.0);
pub(in crate::app) const DEFAULT_TOOL_COMMANDS_LEFT_WIDTH: f32 = 280.0;
pub(in crate::app) const MIN_TOOL_COMMANDS_LEFT_WIDTH: f32 = 200.0;
pub(in crate::app) const MAX_RECENT_FOLDERS: usize = 10;
pub(in crate::app) const CUSTOM_COLOR_SWATCH_COUNT: usize = 64;
pub(in crate::app) const LEGACY_CUSTOM_COLOR_SWATCH_COUNT: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct ColorPaletteSwatch {
    pub(in crate::app) rgba: [u8; 4],
    pub(in crate::app) name: Option<String>,
}

impl ColorPaletteSwatch {
    pub(in crate::app) fn unnamed(rgba: [u8; 4]) -> Self {
        Self { rgba, name: None }
    }

    pub(in crate::app) fn named(rgba: [u8; 4], name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            rgba,
            name: (!name.trim().is_empty()).then(|| name.trim().to_owned()),
        }
    }
}
