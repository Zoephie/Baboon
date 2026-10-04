//! The application shell: the frame loop and the worker messages it applies,
//! the session saved and restored, update checks, and the windows and bars
//! around the features.

use super::*;

pub(in crate::app) mod updates;
pub(in crate::app) mod jobs;
pub(in crate::app) mod session;
