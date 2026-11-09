//! Attached effects, as Halo 3's `effect_new_looping` runs them on an object at
//! rest: what each event's parts are, and where they go. Halo 2, Halo 3, ODST,
//! Reach and Halo 4 read and time their effect parts through these.

use std::sync::Arc;

use blam_tags::{TagFieldData, TagFile, TagStruct};

use crate::app::model_preview::common::math::perpendicular;

use super::common::fields::{elements, function_slot_of, has_flag, object_block};
use super::functions::FunctionValues;
use super::*;
use crate::app::model_preview::common::marker::MarkerFrame;
use crate::app::model_preview::common::math::{cross, dot, unit_by_reciprocal};

mod times;
pub(in crate::app::model_preview) use times::*;
mod parts;
pub(in crate::app::model_preview) use parts::*;
mod object;
pub(in crate::app::model_preview) use object::*;
mod events;
pub(in crate::app::model_preview) use events::*;
mod placement;
pub(in crate::app::model_preview) use placement::*;
#[cfg(test)]
mod tests;
