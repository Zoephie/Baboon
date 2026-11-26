//! The model preview side panel: its layout, the setup cards, the
//! animation player, the view menu and the viewport footer.

mod layout;
pub(in crate::app) use layout::*;
mod animation;
mod section;
mod setup;
mod snapshot;
pub(in crate::app) use section::*;
mod functions;
mod rows;
pub(in crate::app::model_preview) use rows::{CardRows, name_cell, name_column_width, play_row, value_slider};
mod view_menu;
mod viewport_size;
#[cfg(test)]
pub(in crate::app::model_preview) use viewport_size::{game_shaders_note, texture_resolve_note};
mod weapons;
pub(in crate::app::model_preview) use weapons::{carry_through_reload, draw_held_regions, weapon_dropdown};
mod standalone;
pub(in crate::app) use standalone::*;
#[cfg(test)]
mod tests;
mod variants;
