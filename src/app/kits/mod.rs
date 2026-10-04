//! Editing kits: loading a kit or other tag source and indexing it, the built-
//! in and custom kit profiles and finding them on disk, the kit's tools
//! (Sapien, Guerilla, tag_test, tool commands, the terminal), launching
//! scenarios, and dropping tags on the tools.

use super::*;

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
    pub(in crate::app) custom_editing_kit_draft: Option<CustomEditingKitDraft>,
    pub(in crate::app) custom_editing_kit_removal: Option<CustomEditingKitRemoval>,
    pub(in crate::app) blender_path_input: String,
    pub(in crate::app) editing_kit_path_inputs: HashMap<String, String>,
    pub(in crate::app) editing_kit_path_attention: Option<String>,
    pub(in crate::app) tool_commands: ToolCommandsUiState,
    /// A browser drag hovering Sapien's or Guerilla's window, if one is.
    pub(in crate::app) kit_tool_drag: KitToolDragState,
    pub(in crate::app) terminal: TerminalState,
    /// Game ids (`halo3_mcc`, saved as written) for which the user has chosen to
    /// keep the terminal open. Persisted in prefs.json and restored per kit.
    pub(in crate::app) terminal_open_games: HashSet<String>,
    pub(in crate::app) saved_terminal_open_games: HashSet<String>,
    pub(in crate::app) show_entry_index_wait_notice: bool,
    /// Pending "import geometry via tool" request from an Import button.
    pub(in crate::app) pending_tool_import: Option<ToolImportRequest>,
}
