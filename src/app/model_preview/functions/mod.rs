//! Object functions from Halo 2 on: what shaders, attachments and effects
//! read by name. Halo 2 and the gen3 games differ only in [`FunctionRules`].
//!
//! An object's `functions` block turns the values its type exports into named
//! outputs. In a preview nothing runs, so the panel sets those inputs, and
//! each starts where an idle object leaves it.

use crate::app::ui_kit::ThemedCheckbox;

use super::common::fields::{elements, has_flag, object_block};
use super::common::named_function::tag_function_at;
use crate::app::model_preview::common::random::{high16_over_65535, lcg_step};

mod read;
pub(in crate::app::model_preview) use read::*;
mod controls;
mod evaluate;
mod outputs;
pub(in crate::app::model_preview) use controls::*;
#[cfg(test)]
mod tests;
