//! The UI kit: the theme and its colours, button and tag-group icons, and game
//! artwork. It knows nothing about tags or features.

use super::*;
pub(in crate::app) mod scroll;

pub(in crate::app) mod style;
pub(in crate::app) use style::*;
mod input;
pub(in crate::app) use input::*;
mod dropdown;
pub(in crate::app) use dropdown::{ValueComboBox, ValueComboUi, value_dropdown_contents};
pub(in crate::app) mod button_icons;
pub(in crate::app) use button_icons::*;
mod close;
pub(in crate::app) use close::paint_close_control;
pub(in crate::app) mod tag_icons;
pub(in crate::app) use tag_icons::*;
pub(in crate::app) mod game_assets;
pub(in crate::app) use game_assets::*;

pub(in crate::app) mod checkbox;
pub(in crate::app) use checkbox::*;

pub(in crate::app) mod radio;
pub(in crate::app) use radio::*;

mod selection_art;
pub(in crate::app) use selection_art::{SelectionState, native_selection_response, paint_selection_control};
