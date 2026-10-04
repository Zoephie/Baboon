//! What the menus, the toolbar and shortcuts can ask the application to do.
//!
//! One list of actions with one handler, so a menu item, a toolbar button and
//! (later) a shortcut or palette entry that do the same thing are the same
//! value. Each runs the operation it names as it ran when clicked; ones that
//! must see an edit the click committed first go through
//! [`AppAction::Defer`].

use super::*;
use super::recents::RecentAction;

/// An application-level action, carried out once the frame's drawing is over.
pub(in crate::app) enum AppAction {
    /// Make `kit`'s workspace the active one, for the actions sent after it.
    FocusKit(KitId),
    /// Run a file action on the next frame, once any edit that was focused
    /// when the menu took focus has been committed.
    Defer(DeferredFileAction),
    NewTag,
    /// Import a tag or folder: into the containers for a container source, or
    /// converted from another game's kit for a loose one.
    ImportTags,
    LoadTag,
    LoadFolder,
    LoadMonolithic,
    LoadContainer,
    OpenProject,
    OpenTagsFolder,
    OpenDataFolder,
    Recent(RecentAction),
    SaveCurrentTagAs,
    UndoLastPoke,
    ReviewChanges,
    Undo,
    Redo,
    /// Return the tag at `key` in `kit` to the way its source has it.
    DiscardChanges { kit: KitId, key: String },
    /// Ask before clearing every unsaved modification in `kit`.
    ConfirmClearModifications {
        kit: KitId,
        stashed: Vec<String>,
        unsaved: usize,
    },
    /// Open Settings, on `tab` if given.
    OpenSettings(Option<SettingsTab>),
    OpenToolCommands,
    FindReferences(String),
    ExploreReferences(String),
    /// Open Compare Tags on the tag at `key` in `kit`.
    CompareTags { kit: KitId, key: String },
    FixDependencies,
    OpenFieldValueSearch,
    OpenKeywordChooser,
    FindUnreferencedTags,
    ListMapIds,
    ListSoundsByClass,
    ListUncompressedSounds,
    BuildReferenceIndex,
    /// Rescan the active loose folder from disk, dropping the cached index.
    RegenerateTagIndex,
    RefreshTagBrowser,
    ToggleTerminal,
    CheckForUpdates,
    LoadEditingKit(CustomEditingKitProfile),
    LoadBuiltInEditingKit(EditingKitShortcut),
    LaunchBlender,
    LaunchTagTest,
    LaunchSapien,
    OpenBitmapLibrary,
    OpenModelLibrary,
    OpenBlamPane,
}

impl Baboon {
    pub(in crate::app) fn apply_app_action(&mut self, action: AppAction, ctx: &egui::Context) {
        let active = self.model.active;
        match action {
            AppAction::FocusKit(kit) => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.model.active = index;
                }
            }
            AppAction::Defer(action) => self.defer_file_action(action, ctx),
            AppAction::NewTag => self.open_new_tag_dialog(),
            AppAction::ImportTags => {
                if self.model.current_source_is_container() {
                    self.begin_import_tag(None);
                } else {
                    self.open_tag_import_dialog(None);
                }
            }
            AppAction::LoadTag => self.begin_load_single(ctx.clone()),
            AppAction::LoadFolder => self.begin_load_folder(ctx.clone()),
            AppAction::LoadMonolithic => self.begin_load_monolithic(ctx.clone()),
            AppAction::LoadContainer => self.begin_load_iostore_container(ctx.clone()),
            AppAction::OpenProject => self.begin_open_campaign_project(ctx.clone()),
            AppAction::OpenTagsFolder => self.open_loaded_tags_folder(),
            AppAction::OpenDataFolder => self.open_loaded_data_folder(),
            AppAction::Recent(action) => self.apply_recent_action(action, ctx),
            AppAction::SaveCurrentTagAs => self.save_current_tag_as(),
            AppAction::UndoLastPoke => self.begin_undo_last_poke(ctx.clone()),
            AppAction::ReviewChanges => self.review_changes(),
            AppAction::Undo => self.undo_current_tag(),
            AppAction::Redo => self.redo_current_tag(),
            AppAction::DiscardChanges { kit, key } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.discard_tag_changes(index, &key, ctx);
                }
            }
            AppAction::ConfirmClearModifications { kit, stashed, unsaved } => {
                self.mods.clear_stash_confirm = Some(ClearStashConfirm { kit, stashed, unsaved });
            }
            AppAction::OpenSettings(tab) => {
                if let Some(tab) = tab {
                    self.shell.settings_tab = tab;
                }
                self.shell.settings_open = true;
            }
            AppAction::OpenToolCommands => self.kit_tools.tool_commands.open = true,
            AppAction::FindReferences(key) => self.show_references_for(&key),
            AppAction::ExploreReferences(key) => self.open_content_explorer(&key),
            AppAction::CompareTags { kit, key } => {
                self.compare.tag_diff = Some(TagDiffState {
                    kit,
                    a_key: key,
                    source: TagCompareSource::OpenTag,
                    b_kit: None,
                    b_key: None,
                    b_path: None,
                    comparison_kit_root: None,
                    git_history: GitHistoryState::default(),
                    error: None,
                    filters: TagDiffFilters::default(),
                    swapped: false,
                    results: None,
                    git_pending: None,
                });
            }
            AppAction::FixDependencies => self.fix_current_tag_dependencies(),
            AppAction::OpenFieldValueSearch => self.search.field_value_search_open = true,
            AppAction::OpenKeywordChooser => self.browser.keyword_chooser_open = true,
            AppAction::FindUnreferencedTags => self.show_unreferenced_tags(),
            AppAction::ListMapIds => self.show_map_ids(ctx),
            AppAction::ListSoundsByClass => self.show_sounds_by_class(ctx),
            AppAction::ListUncompressedSounds => self.show_uncompressed_sounds(ctx),
            AppAction::BuildReferenceIndex => self.begin_build_reverse_dependencies(ctx.clone(), true),
            AppAction::RegenerateTagIndex => {
                // Clear cached entries so the scan runs fresh.
                if let Some(source) = self.source_mut() {
                    source.all_entries.clear();
                    source.group_tree = crate::core::source::build_group_tree(&[]);
                    source.reverse_dependencies = None;
                }
                self.model.kits[active].field_index.invalidate();
                self.begin_scan_all_entries_with_label(ctx.clone(), "Rebuilding index...");
            }
            AppAction::RefreshTagBrowser => self.refresh_tag_browser(ctx.clone()),
            AppAction::ToggleTerminal => {
                let terminal = &mut self.views[self.model.kits[active].id].terminal;
                terminal.open = !terminal.open;
                self.remember_terminal_open_for_game();
            }
            AppAction::CheckForUpdates => self.begin_check_for_updates(ctx.clone(), false),
            AppAction::LoadEditingKit(profile) => {
                self.load_custom_editing_kit_profile(profile, ctx.clone());
            }
            AppAction::LoadBuiltInEditingKit(shortcut) => self.load_editing_kit_shortcut(shortcut, ctx.clone()),
            AppAction::LaunchBlender => self.launch_blender(),
            AppAction::LaunchTagTest => self.launch_tag_test(),
            AppAction::LaunchSapien => self.launch_sapien(),
            AppAction::OpenBitmapLibrary => self.open_bitmap_library(),
            AppAction::OpenModelLibrary => self.open_model_library(),
            AppAction::OpenBlamPane => {
                // Re-detect on every open: the data folder may have changed
                // since the pane was last shown.
                self.views[self.model.kits[active].id].blam.scanned_path = None;
                self.kit_and_view(active).open_tag_pane(BLAM_KEY);
            }
        }
    }
}
