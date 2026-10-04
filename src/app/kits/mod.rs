//! Editing kits: loading a kit or other tag source and indexing it, the built-
//! in and custom kit profiles and finding them on disk, the kit's tools
//! (Sapien, Guerilla, tag_test, tool commands, the terminal), launching
//! scenarios, and dropping tags on the tools.

use super::*;

pub(in crate::app) mod kit;
pub(in crate::app) use kit::*;
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
pub(in crate::app) mod tools;
pub(in crate::app) mod profiles;
