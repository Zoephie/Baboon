//! The UI kit: the theme and its colours, button and tag-group icons, and game
//! artwork. It knows nothing about tags or features.

use super::*;

pub(in crate::app) mod style;
pub(in crate::app) use style::*;
pub(in crate::app) mod button_icons;
pub(in crate::app) use button_icons::*;
pub(in crate::app) mod tag_icons;
pub(in crate::app) use tag_icons::*;
pub(in crate::app) mod game_assets;
pub(in crate::app) use game_assets::*;
