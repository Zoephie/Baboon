//! Help: the in-app documentation window, tutorials, HaloScript docs, field docs, tag compatibility and map names.

use super::*;

pub(in crate::app) mod docs;
pub(in crate::app) use docs::*;
pub(in crate::app) mod script_docs;
pub(in crate::app) use script_docs::*;
pub(in crate::app) mod field_docs;
pub(in crate::app) use field_docs::*;
pub(in crate::app) mod tutorials;
pub(in crate::app) use tutorials::*;
pub(in crate::app) mod tag_compat;
pub(in crate::app) use tag_compat::*;
pub(in crate::app) mod map_names;
pub(in crate::app) use map_names::*;
pub(in crate::app) mod window;
