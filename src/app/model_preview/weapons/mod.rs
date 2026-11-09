//! What a unit of Halo 2 or a later game can hold, and how it holds it. A
//! weapon names its own animation class and type, and `unit_can_use_weapon`
//! lets it in when the unit's graph has an idle for them. Dual wielding is a
//! class of its own, `dual`, once a second weapon is in the left hand. What
//! an attached object brings with it is in [`attached`], and a variant's
//! child objects in [`variants`].

use std::collections::BTreeSet;

use blam_tags::{TagFile, TagStruct};

use super::common::fields::{elements, has_flag, holder_of};
use super::load::build_render_preview;
use super::*;
use crate::app::model_preview::animation::ik::weapon::CeWeaponIk;
use crate::app::model_preview::animation::ik::weapon::weapon_ik;
use crate::app::model_preview::common::marker::MarkerFrame;
use crate::app::model_preview::common::marker::{attach_point, attach_vector};
use crate::app::model_preview::load::loose_weapon_paths;
use crate::core::source::TagSource;

pub(in crate::app::model_preview) mod attached;
pub(in crate::app::model_preview) mod variants;

use attached::{AttachedObject, fold_attached};
use variants::object_variant;

mod graph;
pub(in crate::app::model_preview) use graph::*;
mod unit;
pub(in crate::app::model_preview) use unit::*;
mod attach;
pub(in crate::app::model_preview) use attach::*;
#[cfg(test)]
mod tests;
