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
use browser::{BrowserFeature, KitBrowser, KitToolDragState};
mod export;
use export::*;
pub(in crate::app) mod model_preview;
use model_preview::*;
mod editor;
use editor::*;
mod audio;
use audio::AudioCommand;
mod runtime_poke;
use runtime_poke::*;
mod chimp;
use chimp::*;
#[cfg(test)]
mod loose_fixture;
use crate::core::created_tags::{CreatedTagLedger, CreatedTagRecord};
use mods::container_write::ContainerLeaseId;
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
mod model;
use model::Model;
mod context;
use context::{CommandQueue, Ctx, cx};
mod dialogs;
use dialogs::{AppReads, Dialog, DialogHost, app_reads};
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
    /// Layout of the open kits: which workspaces are visible and how they are
    /// split. References kits by [`KitId`]; `kits` remains the content store,
    /// so repairing the tree never creates or destroys a kit.
    kit_tree: egui_tiles::Tree<KitId>,
    /// What was last written to disk, so an unchanged frame writes nothing.
    saved_prefs: GuiPrefs,
    /// Mirror of `status` as of the last frame, and when it changed. `status`
    /// is assigned from well over a hundred places, so rather than route them
    /// all through a setter, the change is detected by comparison — which
    /// cannot be bypassed by a new assignment site.
    status_shown: String,
    status_changed_at: f64,
    /// Sound-tag audition: FMOD bank playback (rodio output + bank cache).
    audio: audio::AudioState,
    /// The bundled UE reflection mappings, parsed once on first use — needed to
    /// decode a cooked `AkAudioEvent`.
    ce_usmap: Option<Arc<blam_tags::iostore::usmap::Usmap>>,
    /// Memory poking: the record that undoes the last poke, and whether a
    /// poke or its undo is running.
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
    /// Chimp's app-wide prompts and jobs: mesh texture, texture export and
    /// level export prompts, the level job, writes in flight, the discard
    /// prompt and the usmap path being typed.
    pub(in crate::app) chimp: ChimpFeature,
    /// Editing kits beyond any one workspace: their validation, the profile
    /// being edited or removed, paths being typed, tool commands, dragging a
    /// tag to a tool, the terminal, and a tool import waiting to start.
    pub(in crate::app) kit_tools: KitsFeature,
    /// The tag editor's windows and requests: the colour and function popups,
    /// the reference picker, TSV paste, block confirmation and clipboard, a
    /// deferred file action and a Campaign Evolved sound reference.
    pub(in crate::app) editor: EditorFeature,
    /// References: a reference jump waiting for its referrer, field
    /// navigation, and a referenced tag waiting to open.
    pub(in crate::app) references: ReferencesFeature,
    /// The browser: the keyword chooser and a tag waiting to be revealed.
    pub(in crate::app) browser: BrowserFeature,
    /// Documents: the save-changes prompt.
    pub(in crate::app) documents: DocumentsFeature,
    /// The shell: settings and first run, update checks, the session being
    /// restored, the operation notice, toolbar icons and game artwork, and when
    /// prefs are next checked.
    pub(in crate::app) shell: ShellFeature,
    /// The application model: open kits and the active one, the live
    /// preferences, the default tag names and the status line.
    pub(in crate::app) model: Model,
    /// Each open kit's view state, apart from the model so a draw can change
    /// its kit's view while it reads the model.
    pub(in crate::app) views: KitViews,
    /// What this frame's draws have asked for, applied once drawing is over.
    commands: CommandQueue,
    /// The windows open over the workspaces.
    pub(in crate::app) dialogs: DialogHost,
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
            .then(|| FirstRunWizardState::new(storage.mode, &prefs));
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
                app.model.status = format!("Command line: {error}");
            }
        }
        if app.model.should_check_updates_on_startup() {
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
        // What the application opens on: setup, or the last session's windows.
        let mut dialogs = DialogHost::default();
        if let Some(wizard) = first_run_wizard {
            dialogs.open(wizard);
        }
        if let Some(prompt) = last_opened_windows {
            dialogs.open(prompt);
        }
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
            tx,
            rx,
            egui_ctx: ctx.clone(),
            kit_tree: egui_tiles::Tree::empty(egui::Id::new("kit_tree")),
            saved_prefs: prefs.clone(),
            status_shown: String::new(),
            status_changed_at: 0.0,
            audio: audio::AudioState::default(),
            ce_usmap: None,
            poke: PokeFeature {
                last_poke: None,
                poke_direct_running: false,
                poke_undo_running: false,
            },
            search: SearchFeature {
                find: FindDialogState::default(),
                pending_find_jump: None,
                field_value_searching: false,
            },
            help: HelpFeature {
                docs: Rc::new(HelpDocsState::load()),
                tutorials: Rc::new(TutorialsState::load(&ctx)),
                def_docs_cache: HashMap::new(),
            },
            import: ImportFeature {
                native_template_cache: None,
            },
            tag_ops: TagOpsFeature {
                container_duplicate_running: HashSet::new(),
                container_rename_running: HashSet::new(),
                container_delete_running: HashSet::new(),
                created_tags: CreatedTagLedger::load(),
                folder_refactor: None,
            },
            mods: ModsFeature {
                container_write_leases: HashMap::new(),
                next_container_lease: 0,
                pending_chimp_remounts: Vec::new(),
                last_mod_export_name: None,
            },
            export: ExportFeature {
                container_dump_job: None,
                pending_sound_extract: None,
            },
            chimp: ChimpFeature {
                chimp_level_job: None,
                chimp_writes: HashMap::new(),
            },
            kit_tools: KitsFeature {
                editing_kit_validation,
                editing_kit_path_inputs: editing_kit_path_inputs(&prefs.editing_kit_paths),
                editing_kit_path_attention: None,
                kit_tool_drag: KitToolDragState::default(),
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
                terminal_open_games,
                pending_tool_import: None,
            },
            editor: EditorFeature {
                deferred_file_action: None,
                pending_ce_sound_ref: None,
                block_clipboard: None,
            },
            references: ReferencesFeature {
                pending_ref_jump: None,
                field_nav: None,
                pending_open: None,
            },
            browser: BrowserFeature {
                reveal_target: None,
            },
            documents: DocumentsFeature {
                allow_app_close_once: false,
            },
            shell: ShellFeature {
                available_update: None,
                last_update_check: None,
                restoring_kits: HashSet::new(),
                restored_active_kit: None,
                prefs_next_check_at: 0.0,
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
                artwork: ArtworkCache::default(),
                last_pixels_per_point: ctx.pixels_per_point(),
            },
            commands: CommandQueue::default(),
            dialogs,
            // The startup workspace is seeded like any other new kit; every
            // later one goes through `Baboon::empty_kit`.
            views: KitViews::startup(KitView::new(
                KitId(0),
                KitBrowser::new(prefs.browser_mode, prefs.browser_sort),
            )),
            model: Model {
                default_names: names.clone(),
                kits: vec![Kit::empty(KitId(0), names.clone())],
                active: 0,
                next_kit_id: 1,
                prefs: live_prefs,
                status: "Ready".to_owned(),
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

    fn handle_pixels_per_point_change(&mut self, ctx: &egui::Context) {
        let pixels_per_point = ctx.pixels_per_point();
        if (pixels_per_point - self.shell.last_pixels_per_point).abs() < 0.01 {
            return;
        }
        self.shell.last_pixels_per_point = pixels_per_point;
        self.shell.blender_icon = load_ico_texture(
            ctx,
            "blender_icon",
            include_root_bytes!("assets/Quick access/blender.ico"),
        );
        self.shell.sapien_icon = load_ico_texture(
            ctx,
            "sapien_icon",
            include_root_bytes!("assets/Quick access/sapien.ico"),
        );
        self.shell.tag_test_icon = load_ico_texture(
            ctx,
            "tag_test_icon",
            include_root_bytes!("assets/Quick access/tag_test.ico"),
        );
        self.shell.artwork.clear();
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
