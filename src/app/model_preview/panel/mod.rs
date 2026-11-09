//! The model preview side panel: its layout, the setup card, the
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
mod weapons;
pub(in crate::app) use viewport_size::*;
mod standalone;
pub(in crate::app) use standalone::*;
#[cfg(test)]
mod tests;
mod variants;
