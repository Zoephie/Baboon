//! Halo 3's cloth: a sheet of points kept apart by links, pinned to markers and
//! blown by the wind. The tag holds only the sheet's size; `tool.exe` works the
//! rest out, so it is worked out here the same way.

use blam_tags::{TagFile, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::common::fields::{elements, has_flag, object_block};
use super::common::view::posed_on;
use super::effects::marker_name;
use super::*;
use crate::app::model_preview::common::math::{cross, dot, minus, unit_or_unchanged};
use crate::app::model_preview::common::random::{high_bits, lcg_step};
use super::common::geometry::{Overflow, VertexBase};

mod rules;
pub(in crate::app::model_preview) use rules::*;
mod wind;
pub(in crate::app::model_preview) use wind::*;
mod links;
pub(in crate::app::model_preview) use links::*;
mod read;
pub(in crate::app::model_preview) use read::*;
mod sim;
pub(in crate::app::model_preview) use sim::*;
#[cfg(test)]
mod tests;
