//! Halo 4's lens flares on objects: each `lens_flare` an object attaches,
//! tested against the depth by a world-space patch (`lens_flare_render_occlusion`)
//! and then drawn one screen-facing quad per reflection through
//! `rasterizer_globals`' explicit shader 46 `lens_flare` (`lens_flare_render_internal`,
//! X360 0x83AE8330, MCC 0x1408CF390). See `docs/h4-preview-research/lights.md` §4.

use std::f32::consts::PI;
use std::sync::Arc;

use blam_tags::tag_function::transition_function_evaluate;
use blam_tags::{TagFile, TagFunction, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::common::fields::{elements, has_flag};
use super::super::gen3::effects::flares::{OcclusionOffset, rotation_function};
use super::super::common::view::posed_on;
use super::super::functions::FunctionValues;
use super::super::common::image::H2Image;
use super::super::common::named_function::tag_function_at;
use super::super::effects::part_markers;
use super::super::gen3::materials::{H3Bitmap, read_h3_bitmap};
use super::super::reach::lights::object_value;
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::model_preview::common::math::{cross, dot, unit_unless_under_micro};
use crate::app::model_preview::common::attached::attached;
use crate::app::model_preview::common::attached::{ObjectAttachment, object_attachments};

mod read;
pub(in crate::app::model_preview) use read::*;
mod parameters;
mod compile;
pub(in crate::app::model_preview) use compile::*;
#[cfg(test)]
mod tests;
pub(in crate::app::model_preview) mod pass;
