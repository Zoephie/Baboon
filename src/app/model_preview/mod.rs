//! Model loading, variant selection, and depth-tested preview presentation.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;
use crate::app::shell::{WorkerMessage, spawn_background, spawn_worker};
use crate::app::kits::KitStamp;
use crate::app::editor::{
    BlockConfirm, FieldEditContext, cached_render_method_definition, cached_render_method_option,
    combo_scroll_next_index, dropdown_wheel_delta,
    object_default_variant, object_reference, is_object_family_group, is_previewable_geometry_group_for_game,
    picker_button, picker_popup_width, picker_results, picker_search_field, viewport_wheel_zoom,
};
use crate::app::export::{
    collision_jms_for_game, load_referenced_tag_from_source, model_skeleton, owning_model_skeleton,
    physics_jms_for_game, render_model_skeleton,
};
use crate::app::browser::decode_thumbnail;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
use blam_tags::render_model::{Marker, Node, RenderMesh};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

mod data;
pub(in crate::app) mod animation;
mod ce;
pub(in crate::app) mod dx9;
pub(in crate::app) mod h2;
pub(crate) mod cloth;
pub(in crate::app) mod functions;
pub(crate) mod gen3;
pub(crate) mod h3;
pub(crate) mod h4;
mod reach;
pub(crate) mod particles;
pub(in crate::app) mod load;
pub(in crate::app) mod campaign_evolved;
pub(in crate::app) mod materials;
mod antennas;
mod common;
mod gl;
mod renderer;
mod viewport;
mod frame;
#[cfg(test)]
mod gates;
#[cfg(test)]
mod harness;
mod selection;
mod panel;
pub(in crate::app) mod effects;
pub(in crate::app) mod weapons;

pub(in crate::app) use data::*;
use animation::*;
use load::*;
use campaign_evolved::*;
// Re-exported up to `crate::app` for the worker messages and the playback
// state that lives on `ModelPreviewState`.
pub(in crate::app) use animation::{
    DecodedAnimationPose, PreviewAnimationEntry, PreviewAnimationPlayback,
};
// Re-exported up to `crate::app`: the preview state and the worker message
// both name these, and neither lives under this module.
pub(in crate::app) use materials::*;
// Re-exported for the Model Library (`model_browser`), whose worker rasterizes
// the same geometry with the same flat palette into grid thumbnails.
pub(in crate::app) use load::build_render_preview;
// Started by the source loader when a Campaign Evolved install mounts.
pub(in crate::app) use campaign_evolved::prewarm_ce_mesh_sync_index;
pub(in crate::app) use common::palette::material_color;
use renderer::*;
use viewport::*;
use frame::*;
use common::camera::*;
use common::clock::*;
use dx9::passes::{H2ChangeColors, read_h2_change_colors};
use selection::*;
#[cfg(test)]
use panel::*;
pub(in crate::app) use panel::{
    ModelPreviewSectionPart, draw_model_preview_panel,
    draw_model_preview_section_with_header_wrap, draw_model_viewport_size_input, draw_standalone_mesh_preview,
    preview_panel_title, standalone_mesh_preview,
};

pub(in crate::app) mod state;
pub(in crate::app) use state::*;
