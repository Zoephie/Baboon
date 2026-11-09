//! Halo 3's and Reach's shaders, as their tools compile them.
//!
//! A `render_method_definition` lists categories, each naming a function that
//! the chosen option's function stands in for, and one source file is
//! compiled per entry point with those as defines: `shaders\<name>.fx` in
//! Halo 3, the definition's own `location` in Reach. The sources live in
//! `.hlsl_include` tags, read from the user's kit.

use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::render_method::{BLEND_MODES, REACH_BLEND_MODES};
use super::super::common::image::{Sampler, ImageTexels};

use blam_tags::TagStruct;
use blam_tags::tag_function::FunctionType;

use super::super::common::fields::{elements, has_flag, has_flag_labelled};
use super::super::functions::FunctionValues;
use super::super::common::image::Image;
use super::super::*;
use crate::core::hlsl::emit::{Stage, Translated, translate};
use crate::core::hlsl::parse::{TextureKind, Type, parse};
use crate::core::hlsl::pre::Preprocessor;
use crate::core::source::TagSource;

mod definition;
pub(in crate::app::model_preview) use definition::*;
mod defines;
pub(in crate::app::model_preview) use defines::*;
mod dialect;
pub(in crate::app::model_preview) use dialect::*;
mod sources;
pub(in crate::app::model_preview) use sources::*;
mod pass;
pub(in crate::app::model_preview) use pass::*;
mod material;
pub(in crate::app::model_preview) use material::*;
mod animation;
pub(in crate::app::model_preview) use animation::*;
