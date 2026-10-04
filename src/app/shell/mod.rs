//! The application shell: the frame loop and the worker messages it applies,
//! the session saved and restored, update checks, and the windows and bars
//! around the features.

use super::*;

pub(in crate::app) mod updates;
pub(in crate::app) mod jobs;
pub(in crate::app) mod session;
pub(in crate::app) mod frame;
pub(in crate::app) mod workspace;
pub(in crate::app) mod welcome;
pub(in crate::app) mod first_run;
pub(in crate::app) mod settings;
pub(in crate::app) mod kit_tiles;
pub(in crate::app) mod tag_tiles;
pub(in crate::app) mod loading;
pub(in crate::app) use loading::centered_loading_state;
pub(in crate::app) mod recents;
pub(in crate::app) mod operation_notice;
