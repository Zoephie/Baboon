//! The model preview side panel: its layout, the setup card, the
//! animation player, the view menu and the viewport footer.

use super::*;

mod layout;
pub(in crate::app) use layout::*;
mod setup;
pub(in crate::app::model_preview) use setup::*;
mod animation;
pub(in crate::app::model_preview) use animation::*;
mod section;
pub(in crate::app) use section::*;
mod weapons;
pub(in crate::app::model_preview) use weapons::*;
mod functions;
pub(in crate::app::model_preview) use functions::*;
mod view_menu;
pub(in crate::app::model_preview) use view_menu::*;
mod viewport_size;
pub(in crate::app) use viewport_size::*;
mod standalone;
pub(in crate::app) use standalone::*;
mod variants;
pub(in crate::app::model_preview) use variants::*;
#[cfg(test)]
mod tests;
