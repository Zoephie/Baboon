//! Open tags as documents: selecting and loading them, saving, undo and redo,
//! and closing tabs or the app with the save-changes prompt.

use super::*;

pub(in crate::app) mod selection;
pub(in crate::app) mod saving;
pub(in crate::app) use saving::ordered_unique_keys;
pub(in crate::app) mod close;
pub(in crate::app) mod undo;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;
