//! The menu bar and the toolbar's launcher buttons.
//!
//! Drawn from a [`Ctx`] and a [`MenuState`]: what the menus show that is not
//! in the model, gathered before they draw. A click sends an [`AppAction`].

use super::*;
use super::recents::draw_recent_folders_menu;
use crate::app::shell::frame::{
    EDITING_KIT_MENU_MIN_WIDTH, EditingKitMenuEntry, editing_kit_menu_row_with_read_only,
    is_file_cached, launcher_button, monitor_commands_for_game, visible_editing_kit_menu_entries,
};

/// What the menus show that the model does not hold, gathered before they
/// draw: undo availability, the poke record, a running extraction, the
/// update link, the editing kits with their banners, and the toolbar icons.
pub(in crate::app) struct MenuState {
    can_undo: bool,
    can_redo: bool,
    last_poke: bool,
    poke_undo_running: bool,
    container_dump_running: bool,
    /// Whether the focused workspace shows Chimp, whose save the File
    /// menu's Save then is.
    chimp_surface: bool,
    /// The menu label and release page of an available update.
    available_update: Option<(String, String)>,
    editing_kits: Vec<EditingKitRow>,
    blender_icon: Option<egui::TextureHandle>,
    tag_test_icon: Option<egui::TextureHandle>,
    sapien_icon: Option<egui::TextureHandle>,
}

/// One row of the Editing Kits menu, ready to draw.
struct EditingKitRow {
    kit: EditingKitMenuEntry,
    name: String,
    fallback: &'static str,
    texture: Option<egui::TextureHandle>,
    show_fallback: bool,
    enabled: bool,
    read_only: bool,
    tooltip: String,
}

impl Baboon {
    /// Gather what the menus show beyond the model. Resolving the editing
    /// kits' banners can load them, which is why this runs before the draw.
    pub(in crate::app) fn menu_state(&mut self, ctx: &egui::Context) -> MenuState {
        let entries = visible_editing_kit_menu_entries(
            &self.model.prefs.custom_editing_kit_profiles,
            &self.kit_tools.editing_kit_validation,
        );
        let editing_kits = entries
            .into_iter()
            .map(|entry| match entry {
                EditingKitMenuEntry::Custom(profile) => {
                    let validation = self.kit_tools.editing_kit_validation.custom(&profile.id);
                    let enabled = validation.is_ok();
                    let tooltip = validation
                        .as_ref()
                        .map(|layout| {
                            format!(
                                "Load {} from {}",
                                profile.name,
                                profile_location(&profile, Some(layout)).display()
                            )
                        })
                        .unwrap_or_else(|error| format!("{} is unavailable: {error}", profile.name));
                    let texture = self.workspace_banner_texture(ctx, profile.game_id(), Some(&profile.id));
                    EditingKitRow {
                        name: profile.name.clone(),
                        fallback: "EK",
                        show_fallback: texture.is_none(),
                        texture,
                        enabled,
                        read_only: profile.read_only && !profile.is_campaign_evolved(),
                        tooltip,
                        kit: EditingKitMenuEntry::Custom(profile),
                    }
                }
                EditingKitMenuEntry::BuiltIn(shortcut) => {
                    let texture = self.game_banner_texture(ctx, Some(shortcut.game)).cloned();
                    let configured_path = self
                        .model
                        .prefs
                        .editing_kit_paths
                        .get(shortcut.game.as_str())
                        .expect("validated built-in path");
                    EditingKitRow {
                        name: shortcut.game.display_name().to_owned(),
                        fallback: shortcut.fallback,
                        texture,
                        show_fallback: false,
                        enabled: true,
                        read_only: false,
                        tooltip: format!("Load {} from {}", shortcut.label, configured_path.display()),
                        kit: EditingKitMenuEntry::BuiltIn(shortcut),
                    }
                }
            })
            .collect();
        MenuState {
            can_undo: self.can_undo_current(),
            can_redo: self.can_redo_current(),
            last_poke: self.poke.last_poke.is_some(),
            poke_undo_running: self.poke.poke_undo_running,
            container_dump_running: self.export.container_dump_job.is_some(),
            chimp_surface: self.views[self.model.kits[self.model.active].id].surface == KitSurface::Chimp,
            available_update: self.shell.available_update.as_ref().map(|update| {
                (
                    format!("Update available: {}...", update.short_name()),
                    update.release_url.clone(),
                )
            }),
            editing_kits,
            blender_icon: self.shell.blender_icon.clone(),
            tag_test_icon: self.shell.tag_test_icon.clone(),
            sapien_icon: self.shell.sapien_icon.clone(),
        }
    }
}

/// The top menu bar: the File, Edit, Tools, View, Help and Editing Kits
/// menus, then the tool launcher buttons.
pub(in crate::app) fn draw_menu_bar(cx: &Ctx, ui: &mut egui::Ui, menu: &MenuState, view: &mut KitView) {
    egui::Panel::top("menu")
        .frame(Frame::NONE.fill(menu_bar()).inner_margin(egui::Margin {
            left: 6,
            right: 6,
            top: 2,
            bottom: 2,
        }))
        .show(ui, |ui| {
            egui::MenuBar::new().config(menu_config()).ui(ui, |ui| {
                aligned_menu_button(ui, "File", |ui| {
                    draw_file_menu(cx, ui, menu);
                });
                aligned_menu_button(ui, "Edit", |ui| {
                    draw_edit_menu(cx, ui, menu);
                });
                aligned_menu_button(ui, "Tools", |ui| {
                    draw_tools_menu(cx, ui);
                });
                aligned_menu_button(ui, "View", |ui| {
                    draw_view_menu(cx, ui, view);
                });
                aligned_menu_button(ui, "Help", |ui| {
                    draw_help_menu(cx, ui, menu);
                });
                aligned_menu_button(ui, "Editing Kits", |ui| {
                    draw_editing_kits_menu(cx, ui, menu);
                });
                draw_tool_launcher_buttons(cx, ui, menu);
            });
        });
}

/// The File menu: loading sources, saving tags and projects, and closing.
fn draw_file_menu(cx: &Ctx, ui: &mut Ui, menu: &MenuState) {
    style_list_menu(ui);
    if ui
        .add_enabled(
            !cx.model.editing_kit_is_read_only(cx.model.active),
            egui::Button::new("New Tag..."),
        )
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::NewTag);
    }
    // One entry, two implementations. A container tag is a
    // package rather than a file, so it lands through the
    // Campaign Evolved path; a loose kit converts from
    // another game. Splitting them in the menu would make
    // the user answer a question about Baboon's internals
    // to do the same thing.
    let can_import = cx.model.current_source_is_container() || cx.model.can_import_tags();
    if ui
        .add_enabled(can_import, egui::Button::new("Import Tags..."))
        .on_hover_text(if cx.model.current_source_is_container() {
            "Bring a tag file into these containers"
        } else {
            "Bring a tag, or a whole folder of them, in from another game's editing kit"
        })
        .on_disabled_hover_text("Load an editing kit or a Campaign Evolved container first")
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::ImportTags);
    }
    if ui.button("Load Tag...").clicked() {
        close_menu(ui);
        cx.send(AppAction::LoadTag);
    }
    if ui.button("Load Folder...").clicked() {
        close_menu(ui);
        cx.send(AppAction::LoadFolder);
    }
    if ui.button("Load Monolithic blob_index.dat...").clicked() {
        close_menu(ui);
        cx.send(AppAction::LoadMonolithic);
    }
    if ui
        .button("Open Campaign Evolved container (.utoc)...")
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::LoadContainer);
    }
    if ui.button("Open Baboon Project...").clicked() {
        close_menu(ui);
        cx.send(AppAction::OpenProject);
    }
    // A workspace's edits are autosaved to a recovery file
    // whether or not they are ever saved anywhere else, so
    // these write a copy the user owns and can move, back up
    // or hand to someone. Before them, the only way to get a
    // `.baboon` out of Baboon was to export a mod.
    let can_save_project = cx.model.current_source_is_campaign_project_capable(cx.model.active);
    let project_target = cx.model.kits[cx.model.active]
        .project.active
        .as_ref()
        .and_then(|project| project.project_path.clone());
    if ui
        .add_enabled(can_save_project, egui::Button::new("Save Baboon Project"))
        .on_hover_text(match project_target.as_deref() {
            Some(path) => format!("Write this workspace's changes to {}", path.display()),
            None => "Choose a .baboon file to keep this workspace's changes in".to_owned(),
        })
        .on_disabled_hover_text("Baboon projects hold changes to Campaign Evolved containers")
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::Defer(DeferredFileAction::SaveProject));
    }
    if ui
        .add_enabled(
            can_save_project,
            egui::Button::new("Save Baboon Project As..."),
        )
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::Defer(DeferredFileAction::SaveProjectAs));
    }
    ui.separator();
    let has_loaded_folder = cx.model.loaded_tags_root().is_some();
    if ui
        .add_enabled(has_loaded_folder, egui::Button::new("Open Tags Folder"))
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::OpenTagsFolder);
    }
    if ui
        .add_enabled(has_loaded_folder, egui::Button::new("Open Data Folder"))
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::OpenDataFolder);
    }
    let recent_action = right_opening_menu_button(ui, "Recent Folders", 280.0, |ui| {
        style_list_menu(ui);
        draw_recent_folders_menu(ui, &cx.model.prefs.recent_folders)
    })
    .inner
    .flatten();
    if let Some(action) = recent_action {
        close_menu(ui);
        cx.send(AppAction::Recent(action));
    }
    ui.separator();
    let save_label =
        if cx.model.prefs.enable_chimp && menu.chimp_surface {
            "Save Chimp Changes...    Ctrl+S"
        } else {
            "Save Current Tag    Ctrl+S"
        };
    if icon_text_button(
        ui,
        ButtonIcon::Save,
        save_label,
        !cx.model.editing_kit_is_read_only(cx.model.active),
    )
    .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::Defer(DeferredFileAction::SaveCurrentTag));
    }
    if ui
        .add_enabled(
            cx.model.kits[cx.model.active].selected_key.is_some()
                && !cx.model.editing_kit_is_read_only(cx.model.active),
            egui::Button::new("Save Current Tag As..."),
        )
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::SaveCurrentTagAs);
    }
    if cx.model.current_source_is_container() {
        if ui
            .add_enabled(
                cx.model.can_poke_current_tag(),
                egui::Button::new("Poke Current Tag...    Ctrl+P"),
            )
            .on_hover_text(
                "Apply supported changes to this already-loaded tag in the verified Campaign Evolved process",
            )
            .clicked()
        {
            close_menu(ui);
            cx.send(AppAction::Defer(DeferredFileAction::PokeCurrentTag));
        }
        if menu.last_poke
            && ui
                .add_enabled(!menu.poke_undo_running, egui::Button::new("Undo Last Poke"))
                .on_hover_text("Restore the bytes from Baboon's last verified runtime poke")
                .clicked()
        {
            close_menu(ui);
            cx.send(AppAction::UndoLastPoke);
        }
        if ui
            .add_enabled(
                cx.model.kits[cx.model.active].parsed_tags.values().any(|d| d.dirty.is_set())
                    || cx.model.kits[cx.model.active]
                        .project.active
                        .as_ref()
                        .is_some_and(|project| !project.overlays.is_empty()),
                egui::Button::new("Export Mod..."),
            )
            .on_hover_text(
                "Bundle every modified project tag into one portable mod overlay, with a copy of this project saved beside it",
            )
            .clicked()
        {
            close_menu(ui);
            cx.send(AppAction::Defer(DeferredFileAction::ExportMod));
        }
        // The same review, opened to look rather than to
        // export -- which is how you check what a workspace
        // is carrying before quitting.
        if ui
            .add_enabled(
                cx.model.kits[cx.model.active].has_unwritten_modifications(),
                egui::Button::new("Review Changes..."),
            )
            .on_hover_text(
                "See every edit this workspace is holding that is not written into the game",
            )
            .clicked()
        {
            close_menu(ui);
            cx.send(AppAction::ReviewChanges);
        }
        // Expert-gated because it is the one action here
        // that writes tens of thousands of files: useful
        // for getting the tag set out to diff or grep, and
        // not something to trip over while editing.
        if cx.model.prefs.expert_mode
            && ui
                .add_enabled(
                    !menu.container_dump_running,
                    egui::Button::new("Extract All Tags to Folder\u{2026}"),
                )
                .on_hover_text(
                    "Expert feature: write every tag these containers ship to a folder laid out like an editing kit. Tens of thousands of files \u{2014} this takes a while",
                )
                .on_disabled_hover_text(
                    "An extraction is already running",
                )
                .clicked()
        {
            close_menu(ui);
            cx.send(AppAction::Defer(DeferredFileAction::ExtractAllContainerTags));
        }
    }
    ui.separator();
    if ui
        .add_enabled(
            cx.model.kits[cx.model.active].selected_key.is_some(),
            egui::Button::new("Close Current Tag    Ctrl+W"),
        )
        .clicked()
    {
        // Deferred, per upstream: the close runs after the
        // editor renders, so an edit committed by the menu
        // taking focus is applied before the dirty check.
        if let Some(key) = cx.model.kits[cx.model.active].selected_key.clone() {
            cx.send(AppAction::Defer(DeferredFileAction::Close(PendingCloseAction::CloseTab(key))));
        }
        close_menu(ui);
    }
    if ui
        .add_enabled(
            !cx.model.kits[cx.model.active].open_tabs.is_empty(),
            egui::Button::new("Close All Tags"),
        )
        .clicked()
    {
        cx.send(AppAction::Defer(DeferredFileAction::Close(PendingCloseAction::CloseAllTabs)));
        close_menu(ui);
    }
    ui.separator();
    // Goes through the same close request as the window's own close
    // button, so unsaved tags are still offered for saving first.
    if ui.button("Exit").clicked() {
        close_menu(ui);
        cx.send(AppAction::Defer(DeferredFileAction::Close(PendingCloseAction::CloseApp)));
    }
}

/// The Edit menu: undo and redo, and discarding unsaved changes.
fn draw_edit_menu(cx: &Ctx, ui: &mut Ui, menu: &MenuState) {
    style_list_menu(ui);
    if ui
        .add_enabled(menu.can_undo, egui::Button::new("Undo    Ctrl+Z"))
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::Undo);
    }
    if ui
        .add_enabled(
            menu.can_redo,
            egui::Button::new("Redo    Ctrl+Shift+Z"),
        )
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::Redo);
    }
    ui.separator();
    // The same two actions as the tab context menu and the
    // toolbar, spelled out. An unlabelled trash icon among
    // the tool launchers is not where anyone looks for this.
    let selected = cx.model.kits[cx.model.active].selected_key.clone();
    let discardable = selected
        .as_deref()
        .is_some_and(|key| cx.model.tag_has_discardable_changes(cx.model.active, key));
    if ui
        .add_enabled(discardable, egui::Button::new("Discard Unsaved Changes"))
        .on_hover_text("Return the current tag to the way its source has it")
        .clicked()
    {
        close_menu(ui);
        if let Some(key) = selected {
            cx.send(AppAction::DiscardChanges {
                kit: cx.model.active_kit_id(),
                key,
            });
        }
    }
    if cx.model.current_source_is_campaign_project_capable(cx.model.active) {
        let stashed = cx.model.stashed_campaign_tags(cx.model.active);
        let unsaved = cx.model.kits[cx.model.active]
            .parsed_tags
            .values()
            .filter(|document| document.dirty.is_set())
            .count();
        if ui
            .add_enabled(
                !stashed.is_empty() || unsaved > 0,
                egui::Button::new("Clear All Unsaved Modifications..."),
            )
            .on_hover_text(
                "Return every tag in this workspace to the way the game \
                 ships it, including edits stashed in earlier sessions",
            )
            .on_disabled_hover_text("This workspace has no unsaved modifications")
            .clicked()
        {
            close_menu(ui);
            cx.send(AppAction::ConfirmClearModifications {
                kit: cx.model.active_kit_id(),
                stashed,
                unsaved,
            });
        }
    }

    ui.separator();
    if icon_text_button(ui, ButtonIcon::Settings, "Settings...", true).clicked() {
        cx.send(AppAction::OpenSettings(None));
        close_menu(ui);
    }
}

/// The Tools menu, in four sections: the tool and asset launchers, actions
/// on the current tag, searches and listings across the workspace, and the
/// indexes those searches run on.
fn draw_tools_menu(cx: &Ctx, ui: &mut Ui) {
    style_list_menu(ui);
    if ui.button("Run Tool...").clicked() {
        close_menu(ui);
        cx.send(AppAction::OpenToolCommands);
    }
    draw_monitor_tools_menu(cx, ui);
    draw_assets_tools_menu(cx, ui);

    ui.separator();
    let has_current = cx.model.kits[cx.model.active].selected_key.is_some();
    if ui
        .add_enabled(
            has_current,
            egui::Button::new("Find References to Current Tag"),
        )
        .clicked()
    {
        close_menu(ui);
        if let Some(key) = cx.model.kits[cx.model.active].selected_key.clone() {
            cx.send(AppAction::FindReferences(key));
        }
    }
    if ui
        .add_enabled(
            has_current,
            egui::Button::new("Explore References to Current Tag..."),
        )
        .clicked()
    {
        close_menu(ui);
        if let Some(key) = cx.model.kits[cx.model.active].selected_key.clone() {
            cx.send(AppAction::ExploreReferences(key));
        }
    }
    if icon_text_button(ui, ButtonIcon::Compare, "Compare Tags...", has_current).clicked() {
        close_menu(ui);
        if let Some(key) = cx.model.kits[cx.model.active].selected_key.clone() {
            cx.send(AppAction::CompareTags {
                kit: cx.model.active_kit_id(),
                key,
            });
        }
    }
    let can_fix_dependencies = has_current
        && cx
            .model.source()
            .is_some_and(|source| matches!(source.source, TagSource::LooseFolder { .. }));
    if ui
        .add_enabled(
            can_fix_dependencies,
            egui::Button::new("Fix Tag Dependencies"),
        )
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::FixDependencies);
    }

    ui.separator();
    if ui.button("Search Field Values...").clicked() {
        close_menu(ui);
        cx.send(AppAction::OpenFieldValueSearch);
    }
    if ui.button("Browse Keywords...").clicked() {
        close_menu(ui);
        cx.send(AppAction::OpenKeywordChooser);
    }
    if ui.button("Find Unreferenced Tags...").clicked() {
        close_menu(ui);
        cx.send(AppAction::FindUnreferencedTags);
    }
    if ui.button("List Scenario Map IDs...").clicked() {
        close_menu(ui);
        cx.send(AppAction::ListMapIds);
    }
    if ui.button("List Sounds by Class...").clicked() {
        close_menu(ui);
        cx.send(AppAction::ListSoundsByClass);
    }
    if ui.button("List Uncompressed Sounds...").clicked() {
        close_menu(ui);
        cx.send(AppAction::ListUncompressedSounds);
    }

    ui.separator();
    {
        // Loose folders and Campaign Evolved containers can
        // both be indexed; cache sources cannot.
        let indexable = cx.model.source().is_some_and(|source| {
            matches!(
                source.source,
                TagSource::LooseFolder { .. } | TagSource::IoStoreContainerSet { .. }
            )
        });
        let has_index = cx
            .model.source()
            .is_some_and(|source| source.reverse_dependencies.is_some());
        let label = if cx.model.kits[cx.model.active].index_jobs.building_references {
            "Building Reference Index…"
        } else if has_index {
            "Rebuild Reference Index"
        } else {
            "Build Reference Index"
        };
        if ui
            .add_enabled(
                indexable && !cx.model.kits[cx.model.active].index_jobs.building_references,
                egui::Button::new(label),
            )
            .on_hover_text("Which tags reference which, for the reference searches above")
            .clicked()
        {
            close_menu(ui);
            cx.send(AppAction::BuildReferenceIndex);
        }
    }
    // Regenerate the tag index: force a fresh full scan and overwrite the
    // cached index file.
    let can_regen = cx
        .model.source()
        .map(|s| matches!(s.source, TagSource::LooseFolder { .. }) && s.game.is_some())
        .unwrap_or(false);
    if ui
        .add_enabled(
            can_regen && !cx.model.kits[cx.model.active].scanning_entries,
            egui::Button::new("Regenerate Tag Index"),
        )
        .on_hover_text("Rescan every tag in the folder from disk")
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::RegenerateTagIndex);
    }
    let can_refresh_browser = cx.model.source().is_some_and(|source| {
        matches!(source.source, TagSource::LooseFolder { .. }) && source.game.is_some()
    });
    if ui
        .add_enabled(
            can_refresh_browser
                && !cx.model.kits[cx.model.active].scanning_entries
                && !cx.model.kits[cx.model.active].index_jobs.refreshing,
            egui::Button::new("Refresh Tag Browser"),
        )
        .clicked()
    {
        close_menu(ui);
        cx.send(AppAction::RefreshTagBrowser);
    }
}

/// The View menu: browser mode and sort, display toggles, expert mode, and
/// the terminal.
fn draw_view_menu(cx: &Ctx, ui: &mut Ui, view: &mut KitView) {
    style_list_menu(ui);
    // The browser view belongs to a workspace, so this
    // menu shows and sets the focused kit's — matching the
    // Folders/Groups buttons in that kit's own toolbar.
    if ui
        .selectable_label(view.browser.mode == BrowserMode::Folders, "Folders")
        .clicked()
    {
        view.browser.mode = BrowserMode::Folders;
        close_menu(ui);
    }
    if ui
        .selectable_label(view.browser.mode == BrowserMode::Groups, "Tag Groups")
        .clicked()
    {
        view.browser.mode = BrowserMode::Groups;
        close_menu(ui);
    }
    ui.separator();
    let selected_sort = right_opening_menu_button(
        ui,
        format!("Sort by: {}", view.browser.sort.label()),
        220.0,
        |ui| {
            style_list_menu(ui);
            for option in BrowserSort::ALL {
                if ui
                    .selectable_label(view.browser.sort == option, option.label())
                    .clicked()
                {
                    return Some(option);
                }
            }
            None
        },
    )
    .inner
    .flatten();
    if let Some(option) = selected_sort {
        view.browser.sort = option;
        close_menu(ui);
    }
    ui.separator();
    // Toggles on copies of the preferences, sent as a change once drawn.
    let prefs = &cx.model.prefs;
    let mut show_browser_prefixes = prefs.show_browser_prefixes;
    let mut show_block_sizes = prefs.show_block_sizes;
    let mut angles_in_degrees = prefs.angles_in_degrees;
    let mut scroll_to_cycle_dropdowns = prefs.scroll_to_cycle_dropdowns;
    let mut expert_mode = prefs.expert_mode;
    ui.checkbox(&mut show_browser_prefixes, "Show [tag]/[folder]");
    ui.checkbox(&mut show_block_sizes, "Show block sizes");
    ui.checkbox(&mut angles_in_degrees, "Angles in degrees")
        .on_hover_text(
            "Angle fields hold radians on disk. Guerilla and the other Halo \
             tools show them in degrees, and so does Baboon — turn this off to \
             read and type the stored radians instead.",
        );
    ui.checkbox(&mut scroll_to_cycle_dropdowns, "Scroll wheel cycles dropdowns");
    ui.checkbox(&mut expert_mode, "Expert mode");
    if (show_browser_prefixes, show_block_sizes, angles_in_degrees, scroll_to_cycle_dropdowns, expert_mode)
        != (
            prefs.show_browser_prefixes,
            prefs.show_block_sizes,
            prefs.angles_in_degrees,
            prefs.scroll_to_cycle_dropdowns,
            prefs.expert_mode,
        )
    {
        cx.edit_prefs(move |prefs| {
            prefs.show_browser_prefixes = show_browser_prefixes;
            prefs.show_block_sizes = show_block_sizes;
            prefs.angles_in_degrees = angles_in_degrees;
            prefs.scroll_to_cycle_dropdowns = scroll_to_cycle_dropdowns;
            prefs.expert_mode = expert_mode;
        });
    }
    ui.separator();
    let terminal_enabled = view.terminal.work_dir.is_some();
    if ui
        .add_enabled(terminal_enabled, egui::Button::selectable(view.terminal.open, "Terminal"))
        .clicked()
    {
        cx.send(AppAction::ToggleTerminal);
        close_menu(ui);
    }
}

/// The Help menu: About, documentation, tutorials, and the update check.
fn draw_help_menu(cx: &Ctx, ui: &mut Ui, menu: &MenuState) {
    style_list_menu(ui);
    if ui.button("About...").clicked() {
        cx.send(HelpCommand::Open(HelpPanelTab::About));
        close_menu(ui);
    }
    if icon_text_button(ui, ButtonIcon::Doc, "Doc...", true).clicked() {
        cx.send(HelpCommand::Open(HelpPanelTab::Doc));
        close_menu(ui);
    }
    if ui.button("Tutorials...").clicked() {
        cx.send(HelpCommand::Open(HelpPanelTab::Tutorials));
        close_menu(ui);
    }
    if ui.button("Tag Compatibility...").clicked() {
        cx.send(HelpCommand::Open(HelpPanelTab::TagCompat));
        close_menu(ui);
    }
    if ui.button("Map Names...").clicked() {
        cx.send(HelpCommand::Open(HelpPanelTab::MapNames));
        close_menu(ui);
    }
    if ui.button("Check for updates").clicked() {
        cx.send(AppAction::CheckForUpdates);
        close_menu(ui);
    }
    if let Some((label, url)) = menu.available_update.as_ref() {
        if ui.button(label).clicked() {
            cx.egui.open_url(egui::OpenUrl::new_tab(url));
            close_menu(ui);
        }
    }
}

/// The Editing Kits menu: one entry per configured editing kit.
fn draw_editing_kits_menu(cx: &Ctx, ui: &mut Ui, menu: &MenuState) {
    ui.set_min_width(EDITING_KIT_MENU_MIN_WIDTH);
    let total_rows = menu.editing_kits.len();
    for (index, row) in menu.editing_kits.iter().enumerate() {
        let response = editing_kit_menu_row_with_read_only(
            ui,
            &row.name,
            row.fallback,
            row.texture.as_ref(),
            row.show_fallback,
            row.enabled,
            row.read_only,
        );
        let response = if row.enabled {
            response.on_hover_text(&row.tooltip)
        } else {
            response.on_disabled_hover_text(&row.tooltip)
        };
        if response.clicked() {
            close_menu(ui);
            cx.send(match &row.kit {
                EditingKitMenuEntry::Custom(profile) => AppAction::LoadEditingKit(profile.clone()),
                EditingKitMenuEntry::BuiltIn(shortcut) => AppAction::LoadBuiltInEditingKit(*shortcut),
            });
        }
        if index + 1 < total_rows {
            ui.separator();
        }
    }
    if total_rows == 0 {
        ui.add_enabled(false, egui::Button::new("No configured editing kits"));
    }
    ui.separator();
    if icon_text_button(ui, ButtonIcon::Settings, "Editing Kit Settings...", true).clicked() {
        cx.send(AppAction::OpenSettings(Some(SettingsTab::EditingKits)));
        close_menu(ui);
    }
}

fn draw_tool_launcher_buttons(cx: &Ctx, ui: &mut Ui, menu: &MenuState) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if launcher_button(ui, menu.blender_icon.as_ref(), "B", true)
            .on_hover_text("Launch Blender")
            .clicked()
        {
            cx.send(AppAction::LaunchBlender);
        }

        let tag_test_ready = cx
            .model.kit_tool_path(cx.model.tag_test_executable())
            .is_some_and(|path| is_file_cached(ui.ctx(), &path));
        if launcher_button(ui, menu.tag_test_icon.as_ref(), "T", tag_test_ready)
            .on_hover_text("Launch tag_test without an auto-start scenario")
            .clicked()
        {
            cx.send(AppAction::LaunchTagTest);
        }

        let sapien_ready = cx
            .model.kit_tool_path("sapien.exe")
            .is_some_and(|path| is_file_cached(ui.ctx(), &path));
        if launcher_button(ui, menu.sapien_icon.as_ref(), "S", sapien_ready)
            .on_hover_text("Launch Sapien without an auto-start scenario")
            .clicked()
        {
            cx.send(AppAction::LaunchSapien);
        }

        // Campaign Evolved holds unsaved edits in a project rather than in
        // the game's files, so a workspace accumulates stashed
        // modifications across sessions. This is the way back to the
        // shipped tags; it is drawn here, outside the workspace tree, so it
        // always acts on the focused kit.
        if cx.model.current_source_is_campaign_project_capable(cx.model.active) {
            let stashed = cx.model.stashed_campaign_tags(cx.model.active);
            let unsaved = cx.model.kits[cx.model.active]
                .parsed_tags
                .values()
                .filter(|document| document.dirty.is_set())
                .count();
            let anything = !stashed.is_empty() || unsaved > 0;
            let icon = button_icon_image(ui, ButtonIcon::Garbage, text_dark(), 16.0);
            let response = ui.add_enabled(anything, egui::Button::image(icon));
            if response
                .on_hover_text(
                    "Clear this workspace's unsaved modifications, returning every tag to \
                     the way the game ships it",
                )
                .on_disabled_hover_text("This workspace has no unsaved modifications")
                .clicked()
            {
                cx.send(AppAction::ConfirmClearModifications {
                    kit: cx.model.active_kit_id(),
                    stashed,
                    unsaved,
                });
            }
        }
    });
}

fn draw_monitor_tools_menu(cx: &Ctx, ui: &mut Ui) {
    let game = cx.model.source_game();
    let commands = monitor_commands_for_game(game);
    let enabled = !commands.is_empty();
    let menu = ui
        .add_enabled_ui(enabled, |ui| {
            right_opening_menu_button(ui, "Monitor", 222.0, |ui| {
                style_list_menu(ui);
                ui.set_min_width(210.0);
                for command in commands {
                    if ui.button(*command).clicked() {
                        return Some(*command);
                    }
                }
                None
            })
        })
        .inner;
    if let Some(command) = menu.inner.flatten() {
        cx.send(KitsCommand::RunToolCommand(format!("tool {command}")));
        close_menu(ui);
    }
    let response = menu.response;
    if enabled {
        response.on_hover_text("Run monitor command");
    } else {
        response.on_disabled_hover_text("No monitor commands available for this game");
    }
}

/// Tools ▸ Assets: the asset libraries, browsed across the whole kit rather
/// than one tag at a time.
fn draw_assets_tools_menu(cx: &Ctx, ui: &mut Ui) {
    let enabled = cx.model.source().is_some();
    let menu = ui
        .add_enabled_ui(enabled, |ui| {
            right_opening_menu_button(ui, "Assets", 222.0, |ui| {
                style_list_menu(ui);
                ui.set_min_width(210.0);
                if ui.button("Bitmap Browser").clicked() {
                    return Some("bitmap");
                }
                if ui.button("Model Browser").clicked() {
                    return Some("model");
                }
                // Baboon's own import pipelines only cover Halo 3 so far,
                // so the entry only appears there.
                if cx.model.active_kit_is_halo3() && ui.button("Blam!").clicked() {
                    return Some("blam");
                }
                None
            })
        })
        .inner;
    if let Some(asset) = menu.inner.flatten() {
        match asset {
            "bitmap" => cx.send(AppAction::OpenBitmapLibrary),
            "model" => cx.send(AppAction::OpenModelLibrary),
            "blam" => cx.send(AppAction::OpenBlamPane),
            _ => {}
        }
        close_menu(ui);
    }
    let response = menu.response;
    if !enabled {
        response.on_disabled_hover_text("Load an editing kit to browse its assets");
    }
}
