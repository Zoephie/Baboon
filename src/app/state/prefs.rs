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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) struct EditingKitShortcut {
    pub(in crate::app) label: &'static str,
    pub(in crate::app) game: &'static str,
    pub(in crate::app) fallback: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct CustomEditingKitProfile {
    pub(in crate::app) id: String,
    pub(in crate::app) name: String,
    pub(in crate::app) game: String,
    pub(in crate::app) root: PathBuf,
    /// Relative to the executable directory. `None` uses the bundled folder icon.
    pub(in crate::app) icon: Option<PathBuf>,
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
    pub(in crate::app) editing_id: Option<String>,
    pub(in crate::app) name: String,
    pub(in crate::app) game: String,
    pub(in crate::app) root_input: String,
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
            editing_id: None,
            name: String::new(),
            game: "halo2_mcc".to_owned(),
            root_input: String::new(),
            icon: CustomEditingKitIconDraft::Default,
            error: None,
            icon_warning: None,
        }
    }

    pub(in crate::app) fn from_profile(profile: &CustomEditingKitProfile) -> Self {
        Self {
            editing_id: Some(profile.id.clone()),
            name: profile.name.clone(),
            game: profile.game.clone(),
            root_input: profile.root.display().to_string(),
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
    pub(in crate::app) model_preview_size: f32,
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
    pub(in crate::app) custom_color_swatches: Vec<Option<[u8; 4]>>,
    pub(in crate::app) palette_last_dir: Option<PathBuf>,
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
            scroll_to_cycle_dropdowns: true,
            confirm_container_overwrite: true,
            confirm_runtime_poke: true,
            enable_chimp: true,
            chimp_output_dir: None,
            chimp_usmap_path: None,
            expert_mode: false,
            dark_mode: false,
            ui_scale: DEFAULT_UI_SCALE,
            model_preview_size: DEFAULT_MODEL_PREVIEW_SIZE,
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
            custom_color_swatches: vec![None; CUSTOM_COLOR_SWATCH_COUNT],
            palette_last_dir: None,
        }
    }
}

pub(in crate::app) const DEFAULT_UI_SCALE: f32 = 1.0;
pub(in crate::app) const MIN_UI_SCALE: f32 = 0.6;
pub(in crate::app) const MAX_UI_SCALE: f32 = 1.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_kit_shortcuts_include_expected_profiles() {
        let pairs: Vec<(&str, &str)> = EDITING_KIT_SHORTCUTS
            .iter()
            .map(|shortcut| (shortcut.label, shortcut.game))
            .collect();

        assert_eq!(
            pairs,
            vec![
                ("HCEEK", "haloce_mcc"),
                ("H2EK", "halo2_mcc"),
                ("H3EK", "halo3_mcc"),
                ("H3ODSTEK", "halo3odst_mcc"),
                ("HREK", "haloreach_mcc"),
                ("H4EK", "halo4_mcc"),
                ("H2AMPEK", "halo2amp_mcc"),
                ("Campaign Evolved", "haloce_evolved"),
            ]
        );
    }

    #[test]
    fn update_channel_round_trips_through_its_stored_string() {
        for channel in UpdateChannel::ALL {
            assert_eq!(UpdateChannel::from_str(channel.as_str()), Some(channel));
        }
        assert_eq!(UpdateChannel::from_str("nightly"), None);
        assert_eq!(UpdateChannel::default(), UpdateChannel::Stable);
    }
}

pub(in crate::app) const DEFAULT_TOOL_COMMANDS_WINDOW_SIZE: Vec2 = Vec2::new(800.0, 600.0);
pub(in crate::app) const MIN_TOOL_COMMANDS_WINDOW_SIZE: Vec2 = Vec2::new(600.0, 400.0);
pub(in crate::app) const DEFAULT_TOOL_COMMANDS_LEFT_WIDTH: f32 = 280.0;
pub(in crate::app) const MIN_TOOL_COMMANDS_LEFT_WIDTH: f32 = 200.0;
pub(in crate::app) const MAX_RECENT_FOLDERS: usize = 10;
pub(in crate::app) const CUSTOM_COLOR_SWATCH_COUNT: usize = 16;
