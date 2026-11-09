//! Model loading, variant selection, and depth-tested preview presentation.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;
use crate::app::browser::decode_thumbnail;
use crate::app::editor::{
    BlockConfirm, FieldEditContext, cached_render_method_definition, cached_render_method_option,
    combo_scroll_next_index, dropdown_wheel_delta, is_object_family_group,
    is_previewable_geometry_group_for_game, object_default_variant, object_reference,
    picker_button, picker_popup_width, picker_results, picker_search_field, viewport_wheel_zoom,
};
use crate::app::export::{
    collision_jms_for_game, load_referenced_tag_from_source, model_skeleton, owning_model_skeleton,
    physics_jms_for_game, render_model_skeleton,
};
use crate::app::kits::KitStamp;
use crate::app::shell::{WorkerMessage, spawn_background, spawn_worker};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub(in crate::app) mod animation;
mod antennas;
pub(in crate::app) mod campaign_evolved;
mod ce;
pub(crate) mod cloth;
mod common;
mod data;
pub(in crate::app) mod dx9;
pub(in crate::app) mod effects;
mod frame;
pub(in crate::app) mod functions;
#[cfg(test)]
mod gates;
pub(crate) mod gen3;
mod gl;
pub(in crate::app) mod h2;
pub(crate) mod h3;
pub(crate) mod h4;
#[cfg(test)]
mod harness;
pub(in crate::app) mod load;
pub(in crate::app) mod materials;
mod panel;
pub(crate) mod particles;
mod reach;
mod renderer;
mod selection;
#[cfg(test)]
mod tests;
mod viewport;
pub(in crate::app) mod weapons;

// Re-exported up to `crate::app` for the worker messages and the playback
// state that lives on `ModelPreviewState`.
pub(in crate::app) use animation::{
    DecodedAnimationPose, PreviewAnimationEntry, PreviewAnimationPlayback,
};
// Re-exported up to `crate::app`: the preview state and the worker message
// both name these, and neither lives under this module.
// Re-exported for the Model Library (`model_browser`), whose worker rasterizes
// the same geometry with the same flat palette into grid thumbnails.
pub(in crate::app) use load::build_render_preview;
// Started by the source loader when a Campaign Evolved install mounts.
pub(in crate::app) use campaign_evolved::prewarm_ce_mesh_sync_index;
use common::camera::{
    DEPTH_UNITS, MAX_PREVIEW_SCALE, ModelGpuCamera, PreviewCamera, camera_light, ce_flare_clip,
    ce_flare_pixels, eye_position, min_preview_scale, preview_center_radius, rotate_view_vector,
    scene_depth_terms, unrotate_view_vector,
};
use common::clock::{frame_steps, resume_point};
pub(in crate::app) use common::palette::material_color;
use dx9::passes::{ChangeColors, read_change_colors};
pub(in crate::app) use panel::{
    ModelPreviewSectionPart, draw_model_preview_panel, draw_model_preview_section_with_header_wrap,
    draw_model_viewport_size_input, draw_standalone_mesh_preview, preview_panel_title,
    standalone_mesh_preview,
};

#[cfg(test)]
use animation::PreviewAnimationPose;
use animation::{ANIMATION_FRAME_RATE, animation_skinning_rows, armature_node_positions};
use campaign_evolved::load_campaign_evolved_preview;
use data::{MODEL_ERROR_FALLBACK_COLOR, NEXT_MODEL_GEOMETRY_ID, model_preview_data};
pub(in crate::app) use data::{
    ModelErrorPoint, ModelErrorPrimitive, ModelErrorShape, ModelPreviewData, ModelPreviewLayer,
    ModelVariantPreview, RenderModelPreview, RenderModelPreviewBatch, RenderModelPreviewMarker,
    RenderModelPreviewMaterial, RenderModelPreviewNode, RenderModelPreviewRegion,
    RenderModelPreviewVertex,
};
use frame::{ModelGpuFrame, build_frame, model_layer_visible, preview_clock};
#[cfg(test)]
use frame::{object_states, visible_batch_indices};
use load::{
    PHYSICS_REGION, PreviewLoadSettings, TagReader, bsp_display_name, loose_weapon_paths,
    preview_mesh_order, preview_model_tag, read_model_variants, render_model_to_preview,
};
#[cfg(test)]
use load::{
    halo1_object_collision_overlay, load_model_preview, merge_preview_append,
    preview_skeleton_nodes,
};
pub(in crate::app) use materials::{
    ALL_SLOTS, DetailComposition, DetailFunction, MaterialTextures, SLOT_COUNT, TextureImage,
    TextureSlot, resolve_model_textures,
};
use materials::{ResolveCaches, Values, decode_image, parameter_named};
#[cfg(test)]
use panel::texture_resolve_note;
#[cfg(test)]
use renderer::ModelGlRenderer;
use renderer::{
    FrameCx, MAX_PREVIEW_BONES, ModelGpuSlot, RenderCore, link_h2_program, model_gl_failure,
    paint_model_gl, preview_background,
};
use selection::{
    compute_variant_selection, default_permutation, default_variant_index, detect_active_variant,
    is_model_physics_overlay_region, normalized_new_variant_name, reset_model_preview_selection,
    selected_variant_regions,
};
use viewport::draw_model_viewport;
pub(in crate::app) mod state;
pub(in crate::app) use state::{
    DEFAULT_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE, MIN_MODEL_PREVIEW_SIZE, ModelPreviewState,
    ModelRegionSelection, ModelRenderMode, ModelTagPanelTab,
};
