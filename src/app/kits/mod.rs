//! Editing kits: loading a kit or other tag source and indexing it, the built-
//! in and custom kit profiles and finding them on disk, the kit's tools
//! (Sapien, Guerilla, tag_test, tool commands, the terminal), launching
//! scenarios, and dropping tags on the tools.

use super::*;
use crate::app::shell::{
    CommandLineLaunch, EntryIndexProgressState, IndexingNotice, ReferenceIndexProgressState,
    SettingsWindow, WorkerMessage, resolve_launch_tag_entries, spawn_worker,
};
use crate::app::import::BlamUiState;
use crate::app::references::read_entry_dependencies;
use crate::app::search::FieldValueIndex;
use crate::app::compare::GitReviewState;
use crate::app::chimp::{ChimpState, ChimpView, KitSurface};
use crate::core::document::value::is_saveable_tag;
use crate::app::editor::{
    AppliedFindFilter, EditDrafts, EditorCaches,
    ToolImportRequest, bitmap_reimport_data_path, combo_box_with_scroll, combo_scroll_next_index,
    geometry_import_verb, model_source_dir,
};
use crate::app::browser::{
    Bitmaps, DraggedTagRef, FilterCache, KitBrowser, KitToolDragState, Models, PaletteTable,
    ThumbnailLibrary, disclosure_triangle_icon, entry_rel_path, is_folder_pane_key,
};

pub(in crate::app) mod kit;
pub(in crate::app) use kit::*;
pub(in crate::app) mod view;
pub(in crate::app) use view::{KitMut, KitView, KitViews};
pub(in crate::app) mod editing_kits;
pub(in crate::app) use editing_kits::*;
pub(in crate::app) mod tool_drop_delivery;
pub(in crate::app) use tool_drop_delivery::*;
pub(in crate::app) mod tool_commands;
pub(in crate::app) use tool_commands::*;
pub(in crate::app) mod scenario_palettes;
pub(in crate::app) use scenario_palettes::*;
pub(in crate::app) mod terminal;
pub(in crate::app) use terminal::{
    create_terminal_log_file, run_terminal_command_for_reimport, trim_terminal_lines,
};
pub(in crate::app) mod detect;
pub(in crate::app) use detect::*;
pub(in crate::app) mod kit_tool_options;
pub(in crate::app) use kit_tool_options::*;
pub(in crate::app) mod scenario_launch;
pub(in crate::app) use scenario_launch::*;
pub(in crate::app) mod tool_drop;
pub(in crate::app) mod loading;
pub(in crate::app) mod terminal_state;
pub(in crate::app) use terminal_state::*;
pub(in crate::app) mod tool_commands_window;
pub(in crate::app) use tool_commands_window::pick_tool_command_path;
pub(in crate::app) mod tools;
pub(in crate::app) mod profiles;

/// Editing kits beyond any one workspace: their validation, the profile being
/// edited or removed, paths being typed, tool commands, dragging a tag to a
/// tool, the terminal, and a tool import waiting to start.
pub(in crate::app) struct KitsFeature {
    pub(in crate::app) editing_kit_validation: EditingKitValidationCache,
    pub(in crate::app) editing_kit_path_inputs: HashMap<String, String>,
    pub(in crate::app) editing_kit_path_attention: Option<String>,
    /// A browser drag hovering Sapien's or Guerilla's window, if one is.
    pub(in crate::app) kit_tool_drag: KitToolDragState,
    pub(in crate::app) terminal: TerminalState,
    /// Game ids (`halo3_mcc`, saved as written) for which the user has chosen to
    /// keep the terminal open. Persisted in prefs.json and restored per kit.
    pub(in crate::app) terminal_open_games: HashSet<String>,
    pub(in crate::app) saved_terminal_open_games: HashSet<String>,
    /// Pending "import geometry via tool" request from an Import button.
    pub(in crate::app) pending_tool_import: Option<ToolImportRequest>,
}

/// Which tool a scenario opens in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ScenarioTool {
    TagTest,
    Sapien,
}

/// What the editing kits can be asked to do.
pub(in crate::app) enum KitsCommand {
    /// Save the scenario at `key` in `kit` if needed and open it in `tool`.
    LaunchScenario {
        kit: KitId,
        key: String,
        tool: ScenarioTool,
    },
    /// Run a tool import a field asked for, once it can start.
    QueueToolImport(ToolImportRequest),
    /// Run a tool command line in the terminal.
    RunToolCommand(String),
    /// Run what is typed into the terminal's input line.
    RunTerminalInput,
    /// Stop the terminal's running command.
    StopTerminal,
    /// Close the focused kit's terminal, remembering that for its game.
    CloseTerminal,
}

impl Baboon {
    pub(in crate::app) fn apply_kits_command(&mut self, command: KitsCommand, ctx: &egui::Context) {
        match command {
            KitsCommand::LaunchScenario { kit, key, tool } => {
                let Some(index) = self.model.kit_index(kit) else {
                    return;
                };
                self.model.active = index;
                match tool {
                    ScenarioTool::TagTest => self.launch_scenario_in_tag_test(&key),
                    ScenarioTool::Sapien => self.launch_scenario_in_sapien(&key),
                }
            }
            KitsCommand::QueueToolImport(request) => self.kit_tools.pending_tool_import = Some(request),
            KitsCommand::RunToolCommand(command) => self.submit_terminal_command(command, ctx.clone()),
            KitsCommand::RunTerminalInput => self.begin_terminal_command(ctx.clone()),
            KitsCommand::StopTerminal => self.stop_terminal_command(),
            KitsCommand::CloseTerminal => {
                let active = self.model.kits[self.model.active].id;
                self.views[active].terminal.open = false;
                self.remember_terminal_open_for_game();
            }
        }
    }
}
