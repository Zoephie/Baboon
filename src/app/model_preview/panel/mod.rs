//! The model preview side panel: its layout, the setup cards, the
//! animation player, the view menu and the viewport footer.

mod layout;
pub(in crate::app) use layout::*;
mod animation;
mod section;
mod setup;
pub(in crate::app) use section::*;
mod functions;
mod view_menu;
mod viewport_size;
#[cfg(test)]
pub(in crate::app::model_preview) use viewport_size::texture_resolve_note;
mod weapons;
pub(in crate::app::model_preview) use weapons::{carry_through_weapon_change, weapon_dropdown};
mod standalone;
pub(in crate::app) use standalone::*;
#[cfg(test)]
mod tests;
mod variants;
