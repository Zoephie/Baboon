//! Application state, subsystem composition, and top-level eframe integration.
//! It owns composition of long-lived application state; subsystem-specific behavior and widget implementation belong in child modules.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender},
};

use blam_tags::bitmap::decode::decode_to_rgba8;
use blam_tags::paths::{derive_tags_root, group_tag_to_extension, resolve_tag_path, tag_ref_path};
use blam_tags::render_method::{
    GlobalRenderMethodFlags, Halo2ShaderAnimationType, RenderMethod, RenderMethodAnimatedParameter,
    RenderMethodAnimatedParameterType, RenderMethodDefinition, RenderMethodOption,
    RenderMethodOptionParameter, RenderMethodParameter, RenderMethodParameterType,
    compile_real_constant,
};
use blam_tags::{
    AssFile, Bitmap, BlobFunction, ColorGraphType, CurvePointMode, CurveSegmentType, Endian,
    FoundationMasterType as EngineMasterType, FunctionEncoding, FunctionFlags, FunctionKind,
    FunctionType, H2Function, JmsFile, PeriodicParams, RenderModel, SchemaEnum, StringIdData,
    TagBlock, TagField, TagFieldData, TagFieldType, TagFile, TagFunction, TagFunctionEditor,
    TagReferenceData, TagResource, TagResourceKind, TagStruct, TransitionParams, format_group_tag,
    parse_group_tag,
};
use eframe::egui::{
    self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, RichText,
    ScrollArea, Sense, Stroke, TextStyle, Ui, Vec2,
};
use serde_json::{Value, json};

use crate::core::bundled::{
    definitions_missing_message, locate_definitions_root, locate_help_docs_root,
};
use crate::core::tag_key::{file_entry_key, file_key_path, key_label, new_tag_entry_key};
use crate::core::game::{GameFacts, GameId, game_for_launch_flag, game_for_saved_id};
use crate::core::format::{TagNameIndex, format_value, group_label};
use crate::core::process::background_command;
use crate::core::source::{
    DependencyRef, EkFolderAlias, EntryIndexRefresh, KitLayout, LoadedSourceData,
    NewContainerTemplate, ReverseDependencyIndex, TagEntry, TagEntryLocation,
    TagSource, TagTree, TagTreeNode, load_editing_kit_layout, load_folder,
    load_folder_node_entries, load_iostore_container, load_iostore_container_set,
    load_monolithic_blob_index, load_single_file, loose_file_entry, read_entry,
    resolve_folder_root, scan_folder_subtree_entries, scan_folder_subtree_entries_with_progress,
};

pub(super) const BABOON_GITHUB_URL: &str = "https://github.com/Zoephie/Baboon";
/// The newest `v*` release. GitHub never returns a prerelease here, so this
/// endpoint is the stable channel by definition.
pub(super) const BABOON_STABLE_RELEASE_API: &str =
    "https://api.github.com/repos/Zoephie/Baboon/releases/latest";
/// The rolling development build. `release.yml` deletes and recreates this
/// prerelease on every push to `main`, always under the same `dev` tag.
pub(super) const BABOON_DEV_RELEASE_API: &str =
    "https://api.github.com/repos/Zoephie/Baboon/releases/tags/dev";
pub(super) const BABOON_RELEASES_URL: &str = "https://github.com/Zoephie/Baboon/releases";

/// The commit this binary was built from, baked in by `build.rs`. Empty when
/// the build happened outside a git checkout; suffixed `-dirty` when the
/// working tree had uncommitted changes.
pub(super) const BABOON_BUILD_COMMIT: &str = env!("BABOON_BUILD_COMMIT");

use crate::core::document::journal::*;
use crate::core::document::ops::*;
use crate::core::document::TagDocument;
#[cfg(test)]
use crate::core::document::Dirty;
use crate::core::keywords::*;
mod prefs;
use prefs::*;
pub(in crate::app) mod browser;
use browser::*;
mod export;
use export::*;
pub(in crate::app) mod model_preview;
use model_preview::*;
mod editor;
use editor::*;
mod audio;
mod runtime_poke;
use runtime_poke::*;
mod chimp;
use chimp::*;
#[cfg(test)]
mod loose_fixture;
use crate::core::created_tags::{CreatedTagLedger, CreatedTagRecord};
use mods::container_write::{ContainerLeaseId, ContainerWriteLease};
mod help;
use help::*;
mod compare;
use compare::*;
mod search;
use search::*;
mod references;
use references::*;
pub(in crate::app) mod import;
use import::*;
pub(in crate::app) mod mods;
use mods::*;
pub(in crate::app) mod tag_ops;
use tag_ops::*;
pub(in crate::app) mod kits;
use kits::*;
pub(in crate::app) mod documents;
use documents::*;
pub(in crate::app) mod shell;
use shell::*;
mod ui_kit;
use ui_kit::*;
pub(crate) use shell::{StartupArguments, parse_startup_arguments};

/// One headless egui pass for a test. egui 0.36 debug-panics when a
/// `FullOutput` with unapplied texture deltas is dropped, and a test has no
/// renderer to apply them to, so they are cleared here.
#[cfg(test)]
pub(crate) fn run_ui_test(
    ctx: &egui::Context,
    input: egui::RawInput,
    run_ui: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut output = ctx.run_ui(input, run_ui);
    output.textures_delta.clear();
    output
}

/// The text a pass put on the clipboard, or empty when it copied nothing.
/// egui 0.36 reports a copy as an output command rather than a field.
#[cfg(test)]
pub(crate) fn copied_text(output: &egui::PlatformOutput) -> String {
    output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(super) fn test_definition_path(rel: &str) -> PathBuf {
    locate_definitions_root().join(rel)
}

/// Long-lived application state shared by Baboon's immediate-mode UI.
///
/// Subsystem modules implement focused operations on this state; this type is
/// intentionally the composition root rather than a domain model.
pub struct Baboon {
    window_state: crate::window_state::WindowStateTracker,
    /// The clock eframe stamped on the latest input, kept for
    /// [`Baboon::run_logic`]: while the window is hidden eframe runs no egui
    /// pass, so egui's own clock stays at the last frame shown.
    native_clock: Option<f64>,
    default_names: TagNameIndex,
    /// Cloneable sender given to background jobs; every completion is funneled
    /// back through the receive loop so UI state mutates only on the UI thread.
    tx: Sender<WorkerMessage>,
    /// Single UI-thread receiver. Messages are applied in arrival order and
    /// generation-tagged results are discarded when their source is stale.
    rx: Receiver<WorkerMessage>,
    /// The context every frame runs in, kept so a job can be started from
    /// code that has no frame's context to hand: it wakes the UI when the
    /// job answers.
    egui_ctx: egui::Context,
    /// Every open kit: the content store of the multi-kit model, each owning
    /// its source and all state scoped to it. **Never empty** — an unloaded
    /// Baboon holds one empty workspace kit, so readers of per-kit state need
    /// no "nothing loaded" special case. Cross-frame references use [`KitId`],
    /// never a position, since positions shift when a kit closes.
    kits: Vec<Kit>,
    /// Layout of the open kits: which workspaces are visible and how they are
    /// split. References kits by [`KitId`]; `kits` remains the content store,
    /// so repairing the tree never creates or destroys a kit.
    kit_tree: egui_tiles::Tree<KitId>,
    /// Index into `kits` of the kit the browser, tabs, and editor act on.
    /// Always a valid index; kept in range whenever `kits` changes.
    active: usize,
    /// Monotonic [`KitId`] allocator; ids are never reused.
    next_kit_id: u64,
    /// The most recent check's result, kept only while it is actually an
    /// update. The status line expires on a timer, so this is what keeps the
    /// news reachable after a silent startup check.
    available_update: Option<UpdateCheckResult>,
    /// The most recent successful check, update or not, so Settings can report
    /// the outcome after the status line has expired.
    last_update_check: Option<UpdateCheckResult>,
    chimp_usmap_path_input: String,
    pending_ui_scale: f32,
    editing_kit_validation: EditingKitValidationCache,
    custom_editing_kit_draft: Option<CustomEditingKitDraft>,
    custom_editing_kit_removal: Option<CustomEditingKitRemoval>,
    /// The live preferences: what Settings edits and every reader consults.
    /// `browser_mode` / `browser_sort` here are only the seed a new workspace
    /// starts from — each kit keeps its own — and [`Baboon::current_prefs`]
    /// takes the focused kit's when it writes them out.
    prefs: GuiPrefs,
    /// What was last written to disk, so an unchanged frame writes nothing.
    saved_prefs: GuiPrefs,
    first_run_wizard: Option<FirstRunWizardState>,
    settings_open: bool,
    settings_tab: SettingsTab,
    /// Result of the last container write, shown until dismissed.
    operation_notice: Option<OperationNotice>,
    /// A Chimp mesh export waiting on the choice to export its textures too.
    chimp_mesh_texture_prompt: Option<ChimpMeshTexturePrompt>,
    /// A Chimp texture export waiting on the choice of image format.
    chimp_texture_export_prompt: Option<ChimpTextureExportPrompt>,
    chimp_level_export_prompt: Option<ChimpLevelExportPrompt>,
    chimp_level_job: Option<ChimpLevelJob>,
    /// Kits with a Chimp save running, and the close to run once it lands.
    chimp_writes: HashMap<KitId, Option<PendingCloseAction>>,
    /// Pending workspace-wide Chimp discard, optionally continuing a close
    /// transaction after the packages have been restored.
    chimp_discard_prompt: Option<ChimpDiscardPrompt>,
    tool_commands: ToolCommandsUiState,
    blender_path_input: String,
    editing_kit_path_inputs: HashMap<String, String>,
    editing_kit_path_attention: Option<String>,
    /// One cross-frame edit popup at a time; its embedded tag/path identity
    /// prevents applying a delayed confirmation to the newly selected tag.
    /// File-menu actions run after the editor has rendered, so an edit being
    /// committed by focus loss is applied before its save/export snapshot.
    deferred_file_action: Option<DeferredFileAction>,
    /// Kits whose session-restore load has not landed yet, and the one the
    /// session named as focused. Every load ends by making its own kit active,
    /// so the focus can only be honoured once none are outstanding.
    restoring_kits: HashSet<KitId>,
    restored_active_kit: Option<KitId>,
    color_popup: Option<MaterialColorPopup>,
    /// Kits owning the editing popups below. Each outlives the frame that
    /// opened it and applies an edit addressed by tag key — and a tag key is
    /// only unique within a kit, so applying against whichever kit happens to
    /// be active when the user confirms could edit another game's document, or
    /// silently drop the edit when no such key exists there.
    color_popup_kit: Option<KitId>,
    function_popup_kit: Option<KitId>,
    tag_reference_picker_kit: Option<KitId>,
    /// Function editor snapshot and write targets captured when the popup opens.
    function_popup: Option<FunctionPopup>,
    /// "Compare Tags" (Tag Diff) window state.
    tag_diff: Option<TagDiffState>,
    content_explorer: Option<ContentExplorer>,
    keyword_chooser_open: bool,
    reveal_target: Option<RevealRequest>,
    tsv_paste: Option<TsvPasteState>,
    /// A browser drag hovering Sapien's or Guerilla's window, if one is.
    kit_tool_drag: KitToolDragState,
    status: String,
    /// Mirror of `status` as of the last frame, and when it changed. `status`
    /// is assigned from well over a hundred places, so rather than route them
    /// all through a setter, the change is detected by comparison — which
    /// cannot be bypassed by a new assignment site.
    status_shown: String,
    status_changed_at: f64,
    show_entry_index_wait_notice: bool,
    terminal: TerminalState,
    /// Game ids (`halo3_mcc`, saved as written) for which the user has chosen to
    /// keep the terminal open. Persisted in prefs.json and restored per kit.
    terminal_open_games: HashSet<String>,
    saved_terminal_open_games: HashSet<String>,
    /// When the per-frame prefs check next runs (egui time).
    prefs_next_check_at: f64,
    /// Modal close transaction; the pending action is executed only after every
    /// selected dirty document has been saved or discard is confirmed.
    save_changes_prompt: SaveChangesPrompt,
    /// Startup-only prompt reconstructed from the prior session file.
    last_opened_windows: Option<LastOpenedWindowsPrompt>,
    /// Pending destructive block op (delete / delete all) awaiting confirm.
    block_confirm: Option<BlockConfirm>,
    /// Sound-tag audition: FMOD bank playback (rodio output + bank cache).
    audio: audio::AudioState,
    /// The bundled UE reflection mappings, parsed once on first use — needed to
    /// decode a cooked `AkAudioEvent`.
    ce_usmap: Option<Arc<blam_tags::iostore::usmap::Usmap>>,
    /// Pending play/extract of a `.sound` a container-source tag only refers to,
    /// stamped with the kit that raised it. Resolved after rendering, since the
    /// referenced tag's audio has to be walked out to Wwise first.
    /// A referenced sound to resolve, with the kit whose containers resolve it
    /// and the tab whose player asked (which owns the playback).
    pending_ce_sound_ref: Option<(KitId, String, CeSoundRefRequest)>,
    /// Pending "open referenced tag in a new tab" request.
    pending_open: Option<OpenTagRequest>,
    /// Movable Campaign Evolved tag-reference picker, when one is open.
    tag_reference_picker: Option<TagReferencePickerState>,
    /// Pending "import geometry via tool" request from an Import button.
    pending_tool_import: Option<ToolImportRequest>,
    /// Toolbar launcher icons (decoded from embedded .ico at startup).
    blender_icon: Option<egui::TextureHandle>,
    sapien_icon: Option<egui::TextureHandle>,
    tag_test_icon: Option<egui::TextureHandle>,
    game_banner_textures: HashMap<Option<GameId>, egui::TextureHandle>,
    game_emblem_textures: HashMap<GameId, egui::TextureHandle>,
    custom_editing_kit_textures: HashMap<String, egui::TextureHandle>,
    custom_editing_kit_texture_failures: HashSet<String>,
    last_pixels_per_point: f32,
    /// Clipboard for copy/paste of a block element between identical tags.
    block_clipboard: Option<BlockClipboard>,
    /// A reference-jump awaiting its referrer tag to finish loading before we
    /// can walk it to locate the exact referencing field. Set from the
    /// "References to X" popup; drained by `apply_field_nav`.
    pending_ref_jump: Option<PendingRefJump>,
    /// Active reference-jump navigation: force ancestor blocks open and glow the
    /// exact referencing field until its glow window expires.
    field_nav: Option<FieldNav>,
    /// Which referrer rows in the "References to X" popup are expanded to show
    /// their per-occurrence list. Keyed by row index; reset per references query.
    ref_jump_expanded: HashSet<usize>,
    /// Lazily-computed occurrences per expanded referrer row. A present-but-empty
    /// vec means "walked, none found"; absence means "not yet walked (loading)".
    ref_jump_occurrences: HashMap<usize, Vec<RefOccurrence>>,
    /// Referrer rows whose occurrences a worker is computing. The tag is read
    /// and walked off the UI thread and never cached as a document: it is not
    /// open, so there is no tab to keep it for.
    ref_jump_loading: HashSet<usize>,
    /// Memory poking: the poke dialog, the record that undoes the last poke,
    /// and whether a poke or its undo is running.
    pub(in crate::app) poke: PokeFeature,
    /// Search: the Find dialog, tag query results, the field-value search and a
    /// Find hit waiting to be opened.
    pub(in crate::app) search: SearchFeature,
    /// Help: the About and help windows, tutorials, HaloScript and field docs,
    /// tag compatibility and map names.
    pub(in crate::app) help: HelpFeature,
    /// Import: the Import Tags and cache import windows, single-tag import and
    /// its discard prompt, and the template cache conversions share.
    pub(in crate::app) import: ImportFeature,
    /// Tag operations: New Tag, rename, folder rename and refactor, container
    /// folders, delete and duplicate, the operations running per workspace, and
    /// the ledger of tags Baboon created.
    pub(in crate::app) tag_ops: TagOpsFeature,
    /// Campaign Evolved mods: the overwrite and clear-stash prompts, container
    /// write leases and the remounts they leave, and Export Mod with its
    /// review.
    pub(in crate::app) mods: ModsFeature,
    /// Export: the container dump and its confirmation, the extract target
    /// window, and a sound extraction waiting to start.
    pub(in crate::app) export: ExportFeature,
}

impl Baboon {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        window_state: crate::window_state::WindowStateTracker,
        startup_arguments: StartupArguments,
    ) -> Self {
        let storage = crate::core::storage::initialize();
        Self::configure_context(&cc.egui_ctx);
        let prefs = load_gui_prefs();
        let terminal_open_games = load_terminal_open_games();
        let suppress_startup_popups = startup_arguments.suppresses_startup_popups();
        let first_run_wizard = (!suppress_startup_popups && !load_first_run_complete())
            .then(|| FirstRunWizardState::new(storage.mode));
        set_dark_mode(prefs.dark_mode);
        cc.egui_ctx.set_visuals(foundation_visuals());
        let names = TagNameIndex::load_from_definitions(&locate_definitions_root());
        names.publish_as_process_group_names();
        let last_session = (!suppress_startup_popups && first_run_wizard.is_none())
            .then(load_last_session)
            .flatten()
            .and_then(|session| {
                LastOpenedWindowsPrompt::from_session(session, &prefs.custom_editing_kit_profiles)
            });
        let (last_opened_windows, auto_restore_session) = match prefs.session_restore {
            SessionRestore::Always => match last_session {
                Some(prompt) if prompt.has_reopenable_kits() => (None, Some(prompt.checked_kits())),
                _ => (None, None),
            },
            // Show the prompt.
            SessionRestore::Ask => (last_session, None),
            // Start fresh — never reopen, never ask.
            SessionRestore::Never => (None, None),
        };
        let mut app = Self::assemble(
            &cc.egui_ctx,
            window_state,
            prefs,
            terminal_open_games,
            first_run_wizard,
            names,
            last_opened_windows,
        );
        if let Some(kits) = auto_restore_session {
            app.begin_last_session_restore(kits, cc.egui_ctx.clone());
        }
        match startup_arguments {
            StartupArguments::Normal => {}
            StartupArguments::Launch(launch) => {
                app.begin_command_line_launch(launch, cc.egui_ctx.clone())
            }
            StartupArguments::Invalid(error) => {
                app.status = format!("Command line: {error}");
            }
        }
        if app.should_check_updates_on_startup() {
            app.begin_check_for_updates(cc.egui_ctx.clone(), true);
        }
        app
    }

    /// Fonts, style, image loaders and end-of-pass hooks: everything the app
    /// installs on its egui context before the first frame. Split from
    /// [`Baboon::new`] so a headless context is set up exactly as the
    /// window's is.
    pub(crate) fn configure_context(ctx: &egui::Context) {
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        egui_extras::install_image_loaders(ctx);
        // A drag hovering Sapien's window asks for a copy or not-allowed
        // cursor (see `track_kit_tool_drop`). It is applied at the end of the
        // pass, over whatever cursor a widget under the pointer chose. egui's
        // drag-and-drop shows its grabbing hand after this hook, and only
        // when no cursor was chosen, so the request stands.
        ctx.on_end_pass(
            "kit_tool_drop_cursor",
            Arc::new(|ctx| {
                let cursor = ctx.data(|data| {
                    data.get_temp::<egui::CursorIcon>(egui::Id::new(
                        kits::tool_drop::KIT_TOOL_DROP_CURSOR,
                    ))
                });
                if let Some(cursor) = cursor {
                    ctx.set_cursor_icon(cursor);
                }
            }),
        );
    }

    /// The app built from state already loaded. Split from [`Baboon::new`],
    /// which owns the side effects — context setup, reading prefs and the last
    /// session off disk, startup restores and update checks — so tests can
    /// build an app without any of them.
    #[allow(clippy::too_many_arguments)]
    fn assemble(
        ctx: &egui::Context,
        window_state: crate::window_state::WindowStateTracker,
        prefs: GuiPrefs,
        terminal_open_games: HashSet<String>,
        first_run_wizard: Option<FirstRunWizardState>,
        names: TagNameIndex,
        last_opened_windows: Option<LastOpenedWindowsPrompt>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let editing_kit_validation = EditingKitValidationCache::new(
            &prefs.editing_kit_paths,
            &prefs.custom_editing_kit_profiles,
        );
        // What the file holds, brought into range. `saved_prefs` keeps the
        // file's own values, so a correction here is written back once.
        let mut live_prefs = prefs.clone();
        live_prefs
            .tool_commands_window_size
            .get_or_insert(DEFAULT_TOOL_COMMANDS_WINDOW_SIZE);
        live_prefs.tool_commands_left_width = live_prefs
            .tool_commands_left_width
            .max(MIN_TOOL_COMMANDS_LEFT_WIDTH);
        Self {
            window_state,
            native_clock: None,
            default_names: names.clone(),
            tx,
            rx,
            egui_ctx: ctx.clone(),
            // The startup workspace is seeded like any other new kit; every
            // later one goes through `Baboon::empty_kit`.
            kits: vec![Kit {
                browser_mode: prefs.browser_mode,
                browser_sort: prefs.browser_sort,
                ..Kit::empty(KitId(0), names.clone())
            }],
            kit_tree: egui_tiles::Tree::empty(egui::Id::new("kit_tree")),
            active: 0,
            next_kit_id: 1,
            available_update: None,
            last_update_check: None,
            chimp_usmap_path_input: prefs
                .chimp_usmap_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            pending_ui_scale: prefs.ui_scale,
            editing_kit_validation,
            custom_editing_kit_draft: None,
            custom_editing_kit_removal: None,
            saved_prefs: prefs.clone(),
            prefs: live_prefs,
            first_run_wizard,
            settings_open: false,
            settings_tab: SettingsTab::Startup,
            operation_notice: None,
            chimp_mesh_texture_prompt: None,
            chimp_texture_export_prompt: None,
            chimp_level_export_prompt: None,
            chimp_level_job: None,
            chimp_writes: HashMap::new(),
            chimp_discard_prompt: None,
            tool_commands: ToolCommandsUiState::default(),
            editing_kit_path_inputs: editing_kit_path_inputs(&prefs.editing_kit_paths),
            editing_kit_path_attention: None,
            blender_path_input: prefs
                .blender_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            deferred_file_action: None,
            restoring_kits: HashSet::new(),
            restored_active_kit: None,
            color_popup: None,
            color_popup_kit: None,
            function_popup_kit: None,
            tag_reference_picker_kit: None,
            function_popup: None,
            pending_ref_jump: None,
            field_nav: None,
            ref_jump_expanded: HashSet::new(),
            ref_jump_occurrences: HashMap::new(),
            ref_jump_loading: HashSet::new(),
            tag_diff: None,
            content_explorer: None,
            keyword_chooser_open: false,
            reveal_target: None,
            tsv_paste: None,
            kit_tool_drag: KitToolDragState::default(),
            status: "Ready".to_owned(),
            status_shown: String::new(),
            status_changed_at: 0.0,
            show_entry_index_wait_notice: false,
            terminal: TerminalState {
                input: String::new(),
                lines: Vec::new(),
                history: Vec::new(),
                history_cursor: None,
                refocus_input: false,
                running: false,
                running_id: None,
                next_run_id: 1,
                running_command: None,
                last_log_path: None,
                process: None,
                scroll_to_bottom: false,
            },
            saved_terminal_open_games: terminal_open_games.clone(),
            prefs_next_check_at: 0.0,
            terminal_open_games,
            save_changes_prompt: SaveChangesPrompt::default(),
            last_opened_windows,
            block_confirm: None,
            audio: audio::AudioState::default(),
            ce_usmap: None,
            pending_ce_sound_ref: None,
            pending_open: None,
            tag_reference_picker: None,
            pending_tool_import: None,
            blender_icon: load_ico_texture(
                &ctx,
                "blender_icon",
                include_root_bytes!("assets/Quick access/blender.ico"),
            ),
            sapien_icon: load_ico_texture(
                &ctx,
                "sapien_icon",
                include_root_bytes!("assets/Quick access/sapien.ico"),
            ),
            tag_test_icon: load_ico_texture(
                &ctx,
                "tag_test_icon",
                include_root_bytes!("assets/Quick access/tag_test.ico"),
            ),
            game_banner_textures: HashMap::new(),
            game_emblem_textures: HashMap::new(),
            custom_editing_kit_textures: HashMap::new(),
            custom_editing_kit_texture_failures: HashSet::new(),
            last_pixels_per_point: ctx.pixels_per_point(),
            block_clipboard: None,
            poke: PokeFeature {
                poke_dialog: None,
                last_poke: None,
                poke_direct_running: false,
                poke_undo_running: false,
            },
            search: SearchFeature {
                find: FindDialogState::default(),
                query_results: None,
                pending_find_jump: None,
                field_value_search_open: false,
                field_value_query: String::new(),
                field_value_group: String::new(),
                field_value_searching: false,
            },
            help: HelpFeature {
                about_open: false,
                help_panel_tab: HelpPanelTab::About,
                help_docs: HelpDocsState::load(),
                tutorials: TutorialsState::load(&ctx),
                tutorials_game: GameId::CampaignEvolved.as_str().to_owned(),
                tutorials_category: TutorialCategory::ThreeD,
                script_docs: ScriptDocsUiState::default(),
                tag_compat: TagCompatUiState::default(),
                map_names_game_tab: MapNamesGameTab::HaloCe,
                def_docs_cache: HashMap::new(),
            },
            import: ImportFeature {
                tag_import_dialog: None,
                cache_import_dialog: None,
                native_template_cache: None,
                import_tag_dialog: None,
                import_discard_confirm: None,
            },
            tag_ops: TagOpsFeature {
                new_tag_open: false,
                new_tag_dialog: NewTagDialog::default(),
                container_duplicate_confirm: None,
                container_duplicate_running: HashSet::new(),
                container_rename_running: HashSet::new(),
                delete_confirm: None,
                container_delete_running: HashSet::new(),
                created_tags: CreatedTagLedger::load(),
                rename_tag: None,
                loose_folder_rename: None,
                container_folder_dialog: None,
                folder_refactor: None,
            },
            mods: ModsFeature {
                overwrite_confirm: None,
                container_write_leases: HashMap::new(),
                next_container_lease: 0,
                pending_chimp_remounts: Vec::new(),
                last_mod_export_name: None,
                clear_stash_confirm: None,
                exported_mod: None,
                mod_export: None,
            },
            export: ExportFeature {
                container_dump_confirm: None,
                container_dump_job: None,
                extract_target: None,
                pending_sound_extract: None,
            },
        }
    }

    /// An app with default prefs and no kits, for tests. Prefs and the last
    /// session are not read, and building it writes nothing.
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self::assemble(
            &egui::Context::default(),
            crate::window_state::WindowStateTracker::for_test(),
            GuiPrefs::default(),
            HashSet::new(),
            None,
            TagNameIndex::default(),
            None,
        )
    }

    fn game_banner_texture(
        &mut self,
        ctx: &egui::Context,
        game: Option<GameId>,
    ) -> Option<&egui::TextureHandle> {
        if !self.game_banner_textures.contains_key(&game) {
            let name = game.map_or("unknown", GameId::as_str);
            let texture = load_png_texture(
                ctx,
                &format!("game_banner_{name}"),
                get_game_banner_bytes(game),
            )?;
            self.game_banner_textures.insert(game, texture);
        }
        self.game_banner_textures.get(&game)
    }

    fn game_emblem_texture(
        &mut self,
        ctx: &egui::Context,
        game: GameId,
    ) -> Option<&egui::TextureHandle> {
        if !self.game_emblem_textures.contains_key(&game) {
            let bytes = get_game_emblem_bytes(game);
            let texture = load_png_texture(ctx, &format!("game_emblem_{game}"), bytes)?;
            self.game_emblem_textures.insert(game, texture);
        }
        self.game_emblem_textures.get(&game)
    }

    fn custom_editing_kit_texture(
        &mut self,
        ctx: &egui::Context,
        profile: &CustomEditingKitProfile,
    ) -> Option<&egui::TextureHandle> {
        let relative = profile.icon.as_deref()?;
        if self
            .custom_editing_kit_texture_failures
            .contains(&profile.id)
        {
            return None;
        }
        if !self.custom_editing_kit_textures.contains_key(&profile.id) {
            let texture = resolve_custom_icon_path(relative)
                .ok()
                .and_then(|absolute| fs::read(absolute).ok())
                .and_then(|bytes| {
                    load_png_texture(ctx, &format!("custom_editing_kit_{}", profile.id), &bytes)
                });
            let Some(texture) = texture else {
                self.custom_editing_kit_texture_failures
                    .insert(profile.id.clone());
                return None;
            };
            self.custom_editing_kit_textures
                .insert(profile.id.clone(), texture);
        }
        self.custom_editing_kit_textures.get(&profile.id)
    }

    /// Resolve the image shown in a loaded workspace's browser header.
    ///
    /// A custom profile's selected image takes precedence over the built-in
    /// engine artwork. Looking the profile up by its stable ID keeps restored
    /// workspaces connected to later name/icon edits without copying a
    /// potentially stale icon path into session state.
    fn workspace_banner_texture(
        &mut self,
        ctx: &egui::Context,
        game: Option<GameId>,
        profile_id: Option<&str>,
    ) -> Option<egui::TextureHandle> {
        let profile = profile_id.and_then(|profile_id| {
            self.prefs
                .custom_editing_kit_profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .cloned()
        });
        if let Some(profile) = profile
            && let Some(texture) = self.custom_editing_kit_texture(ctx, &profile).cloned()
        {
            return Some(texture);
        }
        self.game_banner_texture(ctx, game).cloned()
    }

    fn handle_pixels_per_point_change(&mut self, ctx: &egui::Context) {
        let pixels_per_point = ctx.pixels_per_point();
        if (pixels_per_point - self.last_pixels_per_point).abs() < 0.01 {
            return;
        }
        self.last_pixels_per_point = pixels_per_point;
        self.blender_icon = load_ico_texture(
            ctx,
            "blender_icon",
            include_root_bytes!("assets/Quick access/blender.ico"),
        );
        self.sapien_icon = load_ico_texture(
            ctx,
            "sapien_icon",
            include_root_bytes!("assets/Quick access/sapien.ico"),
        );
        self.tag_test_icon = load_ico_texture(
            ctx,
            "tag_test_icon",
            include_root_bytes!("assets/Quick access/tag_test.ico"),
        );
        self.game_banner_textures.clear();
        self.game_emblem_textures.clear();
        self.custom_editing_kit_textures.clear();
        self.custom_editing_kit_texture_failures.clear();
        ctx.request_repaint();
    }
}

/// Decode an embedded `.ico` into an egui texture for a toolbar button.
fn load_ico_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Ico).ok()?;
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

fn load_png_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture(
        name,
        color,
        egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear)),
    ))
}

fn editing_kit_path_inputs(paths: &HashMap<String, PathBuf>) -> HashMap<String, String> {
    EDITING_KIT_SHORTCUTS
        .into_iter()
        .map(|shortcut| {
            (
                shortcut.game.as_str().to_owned(),
                paths
                    .get(shortcut.game.as_str())
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests;

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
