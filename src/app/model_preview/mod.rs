//! Model loading, variant selection, and depth-tested preview presentation.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
use blam_tags::render_model::{Marker, Node, RenderMesh};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub(in crate::app) mod animation;
pub(in crate::app) mod derived;
mod errors;
pub(in crate::app) mod loading;
pub(in crate::app) mod materials;
mod renderer;
mod variants;

use animation::*;
use derived::*;
use errors::*;
use loading::*;
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
pub(in crate::app) use loading::build_render_preview;
// Started by the source loader when a Campaign Evolved install mounts.
pub(in crate::app) use loading::prewarm_ce_mesh_sync_index;
pub(in crate::app) use renderer::material_color;
use renderer::*;
use variants::*;

// The material resolver currently understands the render-method shader/bitmap
// formats validated for H3EK and HREK. Other kits may share a tag container
// generation, but that alone does not make their texture path supported.
fn model_preview_supports_textures(game: Option<&str>) -> bool {
    matches!(game, Some("halo3_mcc" | "haloreach_mcc"))
}

#[cfg(test)]
mod texture_availability_tests {
    use super::model_preview_supports_textures;

    #[test]
    fn textured_shading_is_limited_to_supported_editing_kits() {
        assert!(model_preview_supports_textures(Some("halo3_mcc")));
        assert!(model_preview_supports_textures(Some("haloreach_mcc")));
        for game in [
            None,
            Some("haloce_mcc"),
            Some("halo2_mcc"),
            Some("halo3odst_mcc"),
            Some("halo4_mcc"),
            Some("halo2amp_mcc"),
            Some("haloce_evolved"),
        ] {
            assert!(!model_preview_supports_textures(game), "{game:?}");
        }
    }
}

/// Renderer-facing preview geometry derived from a [`RenderModel`]. Lives in
/// Baboon (not blam-tags) since it is purely a GUI concern.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreview {
    pub regions: Vec<RenderModelPreviewRegion>,
    pub vertices: Vec<RenderModelPreviewVertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<RenderModelPreviewBatch>,
    /// One entry per `RenderModelPreviewBatch::material_index`, naming the
    /// shader tag that batch draws with. Kept as the raw reference rather than
    /// resolved textures: resolving reads other tags off disk, which belongs on
    /// a worker rather than in the geometry walk.
    pub materials: Vec<RenderModelPreviewMaterial>,
    pub markers: Vec<RenderModelPreviewMarker>,
    /// The render model's skeleton, for animation playback. Empty on derived
    /// previews (collision, physics, BSPs, particles), which cannot animate.
    pub nodes: Vec<RenderModelPreviewNode>,
    /// Import-tool diagnostics carried by render, collision, and physics
    /// tags. They stay separate from draw batches so the UI can paint them as
    /// an always-legible overlay and attach hover text to each primitive.
    pub errors: Vec<ModelErrorPrimitive>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewRegion {
    pub name: String,
    pub permutations: Vec<String>,
}

/// One vertex as the GL program consumes it. `#[repr(C)]` because the whole
/// buffer is uploaded as raw bytes; the attribute offsets in
/// [`renderer::ModelGlRenderer::new`] are hand-written to match this order and
/// a test pins them.
///
/// `tangent` and `binormal` are carried rather than reconstructed: a normal map
/// needs the handedness the tag authored, and `cross(normal, tangent)` alone
/// cannot recover a mirrored UV island's sign — which is most of a Halo
/// character, since they mirror left to right to halve texture space.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RenderModelPreviewVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub texcoord: [f32; 2],
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
    /// Skeleton node influences, as FLOATS: GLSL 100 (the GLES fallback
    /// dialect) has no integer vertex attributes, so the indices ride as
    /// floats and the shader casts. All zeros — weights included — on
    /// geometry that has no skeleton (derived previews, particles, chimp);
    /// the shader's animated path treats zero total weight as rigid-to-bind.
    pub node_indices: [f32; 4],
    pub node_weights: [f32; 4],
}

/// The shader one material draws with, as named by the render_model.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewMaterial {
    /// Tag-relative path, no extension — e.g.
    /// `objects\characters\masterchief\shaders\masterchief`. Empty when
    /// the tag_ref was null.
    pub shader_path: String,
    /// Group FOURCC of the shader reference (`rmsh`, `rmtr`, `shad`, ...).
    /// Decides which extension the path resolves with, and which resolver
    /// understands the tag behind it.
    pub shader_group: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewBatch {
    pub region_name: String,
    pub permutation_name: String,
    pub material_index: u16,
    pub index_start: u32,
    pub index_count: u32,
    /// A fixed color instead of the cycling per-material palette. Derived
    /// geometry (collision, physics) sets it so an overlay keeps one
    /// recognizable color no matter where its material lands in the list.
    pub flat_color: Option<[u8; 3]>,
    /// Which independently-toggleable model layer owns this batch. Keeping
    /// this separate from `region_name` lets collision geometry retain its
    /// real region/permutation taxonomy for variant selection.
    pub layer: ModelPreviewLayer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ModelPreviewLayer {
    #[default]
    Render,
    Collision,
    Physics,
}

#[derive(Debug, Clone)]
pub(crate) struct ModelErrorPoint {
    pub position: [f32; 3],
    pub node_indices: [i16; 4],
    pub node_weights: [f32; 4],
}

const MODEL_ERROR_FALLBACK_COLOR: [u8; 4] = [255, 55, 45, 255];

#[derive(Debug, Clone)]
pub(crate) struct ModelErrorPrimitive {
    pub label: String,
    pub non_critical: bool,
    /// Authored debug-view color as RGBA bytes.
    pub color: [u8; 4],
    pub layer: ModelPreviewLayer,
    pub shape: ModelErrorShape,
}

#[derive(Debug, Clone)]
pub(crate) enum ModelErrorShape {
    Point(ModelErrorPoint),
    Vector {
        point: ModelErrorPoint,
        normal: [f32; 3],
        length: f32,
    },
    Polyline(Vec<ModelErrorPoint>),
    Face(Vec<ModelErrorPoint>),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewMarker {
    pub name: String,
    pub node_index: i16,
    pub position: [f32; 3],
    pub axes: [[f32; 3]; 3],
}

/// One skeleton node, carried so animation playback can run forward
/// kinematics and skinning without going back to the tag. Parent-before-child
/// order (the tag's own), which the FK pass relies on.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewNode {
    pub name: String,
    pub parent: i16,
    /// Parent-local bind pose.
    pub bind_rotation: [f32; 4],
    pub bind_translation: [f32; 3],
    /// Inverse of the accumulated bind-pose world transform, as three rows of
    /// an affine matrix — what turns a bind-space vertex into node-local
    /// space before the animated world transform takes it back out.
    pub inverse_bind: [[f32; 4]; 3],
}

static NEXT_MODEL_GEOMETRY_ID: AtomicU64 = AtomicU64::new(1);

fn animation_frame_position(playback: &PreviewAnimationPlayback, pose_frames: usize) -> f32 {
    let last_frame = pose_frames.saturating_sub(1) as f32;
    let position = playback.time * ANIMATION_FRAME_RATE;
    if playback.looped && pose_frames > 1 {
        position % pose_frames as f32
    } else {
        position.min(last_frame)
    }
}

fn animation_frame_label(position: f32, pose_frames: usize) -> String {
    let frame = if pose_frames > 0 {
        position.floor() as usize + 1
    } else {
        0
    };
    format!("{frame} / {pose_frames}")
}

fn animation_header_group(ui: &mut Ui, width: f32, add_contents: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, BUTTON_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        add_contents,
    );
}

fn model_preview_data(
    source_key: String,
    render_model_path: String,
    preview: RenderModelPreview,
    variants: Vec<ModelVariantPreview>,
) -> ModelPreviewData {
    let geometry_id = NEXT_MODEL_GEOMETRY_ID.fetch_add(1, Ordering::Relaxed);
    ModelPreviewData {
        source_key,
        render_model_path,
        preview: Arc::new(preview),
        geometry_id,
        textures_id: geometry_id,
        textures: None,
        variants,
        scenario_bsps: Vec::new(),
        animations: None,
    }
}

fn draw_animation_combo(
    ui: &mut Ui,
    entry_key: &str,
    animations: &[PreviewAnimationEntry],
    playback: &mut PreviewAnimationPlayback,
    width: f32,
) {
    let selected_text = playback
        .selected
        .and_then(|index| animations.get(index))
        .map(|entry| entry.name.as_str())
        .unwrap_or("<None>");
    let popup_id = ui.make_persistent_id(("model_animation_popup", entry_key));
    let open = ui.memory(|memory| memory.is_popup_open(popup_id));
    let response = ui
        .scope(|ui| {
            if open {
                ui.visuals_mut().widgets.inactive.weak_bg_fill =
                    ui.visuals().widgets.open.weak_bg_fill;
            }
            ui.add_sized(Vec2::new(width, BUTTON_HEIGHT), egui::Button::new(""))
        })
        .inner;
    let foreground = if ui.is_enabled() {
        text_dark()
    } else {
        ui.visuals().widgets.noninteractive.fg_stroke.color
    };
    ui.painter().text(
        response.rect.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        truncate_for_cell(selected_text, response.rect.width() - 36.0),
        FontId::proportional(12.0),
        foreground,
    );
    let arrow_rect = egui::Rect::from_center_size(
        egui::pos2(response.rect.right() - 12.0, response.rect.center().y),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    paint_button_icon_at(ui, ButtonIcon::Down, arrow_rect, foreground);
    let just_opened = response.clicked() && !open;
    if response.clicked() {
        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
    }
    egui::popup::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::popup::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(width.max(240.0));
            let search = ui.add(
                egui::TextEdit::singleline(&mut playback.filter)
                    .hint_text(placeholder_text("search animations…"))
                    .desired_width(320.0),
            );
            if just_opened {
                search.request_focus();
            }
            ui.separator();
            let filter = playback.filter.trim().to_ascii_lowercase();
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    let mut shown = 0;
                    for (index, row) in animations.iter().enumerate() {
                        if !filter.is_empty() && !row.name.to_ascii_lowercase().contains(&filter) {
                            continue;
                        }
                        shown += 1;
                        let label = if row.playable {
                            format!("{}  ({} · {} frames)", row.name, row.kind, row.frame_count)
                        } else {
                            format!("{}  (no data)", row.name)
                        };
                        if ui
                            .add_enabled(
                                row.playable,
                                egui::SelectableLabel::new(playback.selected == Some(index), label),
                            )
                            .clicked()
                        {
                            playback.selected = Some(index);
                            playback.pose = None;
                            playback.time = 0.0;
                            playback.playing = false;
                            playback.stopped = false;
                            playback.error = None;
                            ui.memory_mut(|memory| memory.close_popup());
                        }
                    }
                    if shown == 0 {
                        ui.label(RichText::new("No animations match.").color(subtle_dark()));
                    }
                });
        },
    );
}

pub(super) fn draw_model_preview_panel(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    source_game: Option<&str>,
    state: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    edit: &mut FieldEditContext<'_>,
) {
    let is_model = is_previewable_geometry_group(entry.group_tag, names);
    if !is_model {
        return;
    }

    let supports_textures = model_preview_supports_textures(source_game);
    if !supports_textures {
        state.render_mode = state.render_mode.without_textures();
    }

    ui.scope(|ui| {
        // The parse is synchronous; on the first frame for a tag, show a
        // spinner, kick the (blocking) parse, and repaint so the decoded
        // model appears next frame instead of a blank panel. (A future
        // change can move the parse to a worker thread — see plan 1.9.)
        let needs_load = state.needs_preview_load(&entry.key);
        if needs_load {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Loading model…").color(subtle_dark()));
            });
            ensure_model_preview_loaded(tag, entry, names, source, state);
            ui.ctx().request_repaint();
            return;
        }

        let is_campaign_evolved = tag.header.group_tag.to_be_bytes() == *b"hlmt"
            && tag
                .root()
                .read_tag_ref_with_group("skeleton model")
                .map(|(_, reference)| !reference.trim().is_empty())
                .unwrap_or(false);

        let Some(data_result) = state.data.take() else {
            ui.label(RichText::new("No preview loaded").color(subtle_dark()));
            return;
        };
        let mut restore_data = Some(data_result);
        let data = match restore_data.as_ref().expect("preview data just set") {
            Ok(data) => data,
            Err(error) => {
                ui.colored_label(Color32::from_rgb(150, 56, 44), error);
                state.data = restore_data.take();
                return;
            }
        };

        // A scenario's per-BSP toggle list. Region toggles cannot serve
        // here: a region only exists once its BSP is loaded, and the point
        // of this list is choosing what to load in the first place.
        let draw_scenario_setup = |ui: &mut Ui, state: &mut ModelPreviewState| {
            if !data.scenario_bsps.is_empty() {
                ui.label(RichText::new("Structure BSPs").strong().color(text_dark()));
                ui.horizontal_wrapped(|ui| {
                    for (index, reference) in data.scenario_bsps.iter().enumerate() {
                        let Some(reference) = reference else {
                            continue;
                        };
                        let mut checked = state.scenario_bsp_selection.contains(&index);
                        if ui
                            .checkbox(&mut checked, bsp_display_name(reference))
                            .on_hover_text(reference.as_str())
                            .changed()
                        {
                            if checked {
                                state.scenario_bsp_selection.insert(index);
                            } else {
                                state.scenario_bsp_selection.remove(&index);
                            }
                        }
                    }
                });
                if state.scenario_bsp_selection.is_empty() {
                    ui.label(
                        RichText::new("Check a structure BSP to load its geometry.")
                            .color(subtle_dark()),
                    );
                }
                ui.separator();
            }
        };

        // Deferred until after the viewport/setup row so the player can use
        // the full page width, matching the layout in the design reference.
        let draw_animation_player = |ui: &mut Ui, state: &mut ModelPreviewState| {
            ui.add_space(8.0);
            draw_model_preview_section(ui, "Animation Player", None, |ui, part| {
                let animations = data.animations.as_deref().map_or(&[][..], Vec::as_slice);
                let pose_frames = state
                    .animation
                    .pose
                    .as_ref()
                    .map(|pose| pose.frames.len())
                    .unwrap_or(0);
                let controls_enabled = state.animation.selected.is_some() && pose_frames > 0;
                let duration = pose_frames as f32 / ANIMATION_FRAME_RATE;
                let last_frame = pose_frames.saturating_sub(1) as f32;
                if part == ModelPreviewSectionPart::Header {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        let loading = state.animation.selected.is_some()
                            && state.animation.pose.is_none()
                            && state.animation.error.is_none();
                        let spinner_width = if loading { 20.0 } else { 0.0 };
                        let combo_width = if ui.available_width() >= 600.0 {
                            340.0
                        } else {
                            240.0_f32.min((ui.available_width() - spinner_width).max(1.0))
                        };
                        animation_header_group(ui, combo_width + spinner_width, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.add_enabled_ui(!animations.is_empty(), |ui| {
                                draw_animation_combo(
                                    ui,
                                    &entry.key,
                                    animations,
                                    &mut state.animation,
                                    combo_width,
                                );
                            });
                            if loading {
                                ui.spinner();
                            }
                        });
                        animation_header_group(ui, 112.0, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if selectable_icon_button(
                                ui,
                                ButtonIcon::Play,
                                "Play",
                                state.animation.playing,
                                controls_enabled,
                            )
                            .clicked()
                            {
                                state.animation.playing = true;
                                state.animation.stopped = false;
                                // Play at the end of a non-looping clip restarts it.
                                if !state.animation.looped
                                    && state.animation.time * ANIMATION_FRAME_RATE >= last_frame
                                {
                                    state.animation.time = 0.0;
                                }
                            }
                            if selectable_icon_button(
                                ui,
                                ButtonIcon::Pause,
                                "Pause",
                                !state.animation.playing && !state.animation.stopped,
                                controls_enabled,
                            )
                            .clicked()
                            {
                                state.animation.playing = false;
                                state.animation.stopped = false;
                            }
                            if selectable_icon_button(
                                ui,
                                ButtonIcon::Stop,
                                "Stop and return to the default pose",
                                state.animation.stopped,
                                controls_enabled,
                            )
                            .clicked()
                            {
                                state.animation.playing = false;
                                state.animation.stopped = true;
                                state.animation.time = 0.0;
                            }
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if selectable_icon_button(
                                ui,
                                ButtonIcon::Loop,
                                "Loop animation",
                                state.animation.looped,
                                controls_enabled,
                            )
                            .clicked()
                            {
                                state.animation.looped = !state.animation.looped;
                            }
                        });
                        let speed_label_width = ui
                            .painter()
                            .layout_no_wrap(
                                "Speed".to_owned(),
                                FontId::proportional(12.0),
                                subtle_dark(),
                            )
                            .size()
                            .x;
                        animation_header_group(ui, speed_label_width + 4.0 + 48.0, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new("Speed").color(subtle_dark()));
                            ui.add_enabled_ui(controls_enabled, |ui| {
                                ui.add_sized(
                                    Vec2::new(48.0, BUTTON_HEIGHT),
                                    egui::DragValue::new(&mut state.animation.speed)
                                        .range(0.05..=4.0)
                                        .speed(0.02)
                                        .max_decimals(2)
                                        .suffix("×"),
                                );
                            });
                        });
                        let frame_position =
                            animation_frame_position(&state.animation, pose_frames);
                        ui.add(
                            egui::Label::new(
                                RichText::new(animation_frame_label(frame_position, pose_frames))
                                    .color(subtle_dark()),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
                } else {
                    ui.scope(|ui| {
                        let mut scrub = animation_frame_position(&state.animation, pose_frames);
                        ui.spacing_mut().slider_width = (ui.available_width() - 2.0).max(1.0);
                        if ui
                            .add_enabled(
                                controls_enabled,
                                egui::Slider::new(&mut scrub, 0.0..=last_frame.max(1.0))
                                    .show_value(false),
                            )
                            .changed()
                        {
                            state.animation.time = scrub / ANIMATION_FRAME_RATE;
                            state.animation.playing = false;
                            state.animation.stopped = false;
                        }
                        if controls_enabled && state.animation.playing {
                            let pass = ui.ctx().cumulative_pass_nr();
                            let dt = ui.input(|input| input.stable_dt).min(0.1);
                            advance_playback_clock(
                                &mut state.animation,
                                pass,
                                dt,
                                duration,
                                last_frame,
                            );
                            ui.ctx().request_repaint();
                        }
                    });
                    if let Some(error) = state.animation.error.clone() {
                        ui.label(RichText::new(error).color(Color32::from_rgb(150, 56, 44)));
                    }
                }
            });
        };

        let mut mutation_requested = false;
        let mut reload_requested = false;
        let page_width = ui.available_width();
        let can_place_controls_beside = page_width >= 780.0;
        if can_place_controls_beside {
            let gap = MODEL_PREVIEW_SECTION_GAP;
            // At wide sizes the persisted preview scale controls the entire
            // preview card. Reserve enough room for Model Setup, then let the
            // card grow around its 470×300 viewport until it reaches that
            // limit. Previously only the image changed size inside a fixed
            // 40% column, which made the control feel disconnected.
            let preview_width = wide_model_preview_section_width(page_width, *model_preview_size);
            let setup_width = (page_width - preview_width - gap).max(WIDE_MODEL_SETUP_MIN_WIDTH);
            let preview_viewport_size = model_viewport_size(preview_width, *model_preview_size);
            let shared_body_height = preview_viewport_size.y + MODEL_PREVIEW_STATS_FOOTER_HEIGHT;
            // Setup's body has 8-point top/bottom margins; Preview's body is
            // edge-to-edge. Match their *outer* card heights, not just content.
            let setup_body_height =
                (shared_body_height - model_setup_extra_header_height(setup_width) - 16.0).max(1.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.allocate_ui(Vec2::new(preview_width, 0.0), |ui| {
                    ui.set_min_width(preview_width);
                    ui.set_max_width(preview_width);
                    draw_model_preview_section(
                        ui,
                        "Model Preview",
                        Some(shared_body_height),
                        |ui, part| match part {
                            ModelPreviewSectionPart::Header => draw_model_view_settings_menu(
                                ui,
                                tag,
                                entry,
                                data,
                                state,
                                model_preview_size,
                                supports_textures,
                                is_campaign_evolved,
                                !data.preview.nodes.is_empty(),
                            ),
                            ModelPreviewSectionPart::Body => {
                                draw_model_viewport_with_stats(
                                    ui,
                                    data,
                                    state,
                                    preview_viewport_size,
                                );
                            }
                        },
                    );
                });
                ui.allocate_ui(Vec2::new(setup_width, 0.0), |ui| {
                    ui.set_min_width(setup_width);
                    ui.set_max_width(setup_width);
                    draw_model_preview_section(
                        ui,
                        "Model Setup",
                        Some(setup_body_height),
                        |ui, part| {
                            if part == ModelPreviewSectionPart::Header {
                                draw_model_setup_header_controls(
                                    ui,
                                    setup_width,
                                    data,
                                    state,
                                    |ui, state| {
                                        if draw_variant_header_actions(ui, data, state, edit) {
                                            mutation_requested = true;
                                        }
                                        if icon_text_button(
                                            ui,
                                            ButtonIcon::Refresh,
                                            "Refresh Model",
                                            true,
                                        )
                                        .clicked()
                                        {
                                            state.loaded_key = None;
                                            state.data = None;
                                            ensure_model_preview_loaded(
                                                tag, entry, names, source, state,
                                            );
                                            reload_requested = true;
                                        }
                                    },
                                );
                            } else {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_height(setup_body_height)
                                    .show(ui, |ui| {
                                        draw_scenario_setup(ui, state);
                                        draw_variant_controls(ui, data, state);
                                    });
                            }
                        },
                    );
                });
            });
        } else {
            draw_model_preview_section(ui, "Model Preview", None, |ui, part| match part {
                ModelPreviewSectionPart::Header => draw_model_view_settings_menu(
                    ui,
                    tag,
                    entry,
                    data,
                    state,
                    model_preview_size,
                    supports_textures,
                    is_campaign_evolved,
                    !data.preview.nodes.is_empty(),
                ),
                ModelPreviewSectionPart::Body => {
                    let size = model_viewport_size(ui.available_width(), *model_preview_size);
                    draw_model_viewport_with_stats(ui, data, state, size);
                }
            });
            ui.add_space(8.0);
            let setup_width = ui.available_width();
            draw_model_preview_section(ui, "Model Setup", None, |ui, part| {
                if part == ModelPreviewSectionPart::Header {
                    draw_model_setup_header_controls(ui, setup_width, data, state, |ui, state| {
                        if draw_variant_header_actions(ui, data, state, edit) {
                            mutation_requested = true;
                        }
                        if icon_text_button(ui, ButtonIcon::Refresh, "Refresh Model", true)
                            .clicked()
                        {
                            state.loaded_key = None;
                            state.data = None;
                            ensure_model_preview_loaded(tag, entry, names, source, state);
                            reload_requested = true;
                        }
                    });
                } else {
                    draw_scenario_setup(ui, state);
                    draw_variant_controls(ui, data, state);
                }
            });
        }
        draw_animation_player(ui, state);
        if mutation_requested {
            state.loaded_key = None;
            state.data = None;
        } else if !reload_requested {
            state.data = restore_data.take();
        }
    });
    ui.add_space(8.0);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ModelPreviewSectionPart {
    Header,
    Body,
}

const MODEL_PREVIEW_STATS_FOOTER_HEIGHT: f32 = 28.0;
const MODEL_SETUP_EXTRA_HEADER_HEIGHT: f32 = 32.0;
const MODEL_SETUP_INLINE_HEADER_MIN_WIDTH: f32 = 680.0;
const MODEL_SETUP_SHARED_CONTROLS_ROW_MIN_WIDTH: f32 = 560.0;
const WIDE_MODEL_SETUP_MIN_WIDTH: f32 = 400.0;
const MODEL_PREVIEW_SECTION_GAP: f32 = 8.0;

fn model_setup_extra_header_height(width: f32) -> f32 {
    if width >= MODEL_SETUP_INLINE_HEADER_MIN_WIDTH {
        0.0
    } else if width >= MODEL_SETUP_SHARED_CONTROLS_ROW_MIN_WIDTH {
        MODEL_SETUP_EXTRA_HEADER_HEIGHT
    } else {
        MODEL_SETUP_EXTRA_HEADER_HEIGHT * 2.0
    }
}

fn draw_model_setup_header_controls(
    ui: &mut Ui,
    section_width: f32,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    mut draw_actions: impl FnMut(&mut Ui, &mut ModelPreviewState),
) {
    let mut draw_action_group = |ui: &mut Ui, state: &mut ModelPreviewState| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            draw_actions(ui, state);
        });
    };
    if section_width < MODEL_SETUP_SHARED_CONTROLS_ROW_MIN_WIDTH {
        // Three intact groups: title above, then variant navigation, then
        // Save/Delete/Refresh. Do not let individual buttons escape the card.
        ui.vertical(|ui| {
            draw_variant_selector(ui, data, state);
            draw_action_group(ui, state);
        });
    } else {
        // The title is either inline or on the row above; the two control
        // groups fit together at this width.
        draw_variant_selector(ui, data, state);
        draw_action_group(ui, state);
    }
}

fn wide_model_preview_section_width(page_width: f32, model_preview_size: f32) -> f32 {
    let desired_width =
        470.0 * model_preview_size.clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE);
    desired_width
        .min((page_width - MODEL_PREVIEW_SECTION_GAP - WIDE_MODEL_SETUP_MIN_WIDTH).max(1.0))
}

/// A fixed, full-width section styled like the Tag Fields group headers, with
/// room for compact controls on the right side of the header bar.
pub(in crate::app) fn draw_model_preview_section(
    ui: &mut Ui,
    title: &str,
    min_body_height: Option<f32>,
    add_contents: impl FnMut(&mut Ui, ModelPreviewSectionPart),
) -> egui::Rect {
    draw_model_preview_section_with_header_wrap(ui, title, min_body_height, None, add_contents)
}

/// Draws a preview section whose header actions can move to a second row once
/// the section is narrower than `header_wrap_width`.
pub(in crate::app) fn draw_model_preview_section_with_header_wrap(
    ui: &mut Ui,
    title: &str,
    min_body_height: Option<f32>,
    header_wrap_width: Option<f32>,
    mut add_contents: impl FnMut(&mut Ui, ModelPreviewSectionPart),
) -> egui::Rect {
    const RADIUS: f32 = 5.0;
    // `allocate_ui` inherits its parent's layout. At wide widths this section
    // lives inside the horizontal Preview/Setup row, so establish a vertical
    // layout here instead of allowing the header, body and footer to become
    // siblings in that outer row.
    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
        let width = ui.available_width().max(1.0);
        let setup_header = title == "Model Setup";
        let animation_header = title == "Animation Player";
        let bitmap_header = title == "Bitmap Preview";
        let wrapped_bitmap_header = bitmap_header
            && header_wrap_width
                .map(|wrap_width| width < wrap_width)
                .unwrap_or(false);
        let edge_to_edge = matches!(title, "Model Preview" | "Bitmap Preview");
        let extra_header_height = if wrapped_bitmap_header {
            40.0
        } else if setup_header {
            model_setup_extra_header_height(width)
        } else if animation_header && width < 800.0 {
            if width < 320.0 {
                96.0
            } else if width < 560.0 {
                64.0
            } else {
                32.0
            }
        } else {
            0.0
        };
        let header_height = 40.0 + extra_header_height;
        let (header_rect, _) =
            ui.allocate_exact_size(Vec2::new(width, header_height), Sense::hover());
        ui.painter().rect_filled(
            header_rect,
            egui::Rounding {
                nw: RADIUS,
                ne: RADIUS,
                sw: 0.0,
                se: 0.0,
            },
            foundation_section_bar(),
        );

        let header_content_rect = header_rect.shrink(8.0);
        let title_rect = if extra_header_height > 0.0 {
            egui::Rect::from_min_max(
                header_content_rect.min,
                egui::pos2(header_content_rect.max.x, header_content_rect.min.y + 24.0),
            )
        } else {
            header_content_rect
        };
        let mut title_ui = ui.new_child(egui::UiBuilder::new().max_rect(title_rect));
        title_ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if !bitmap_header {
                ui.label(
                    RichText::new(title)
                        .font(bold_font(12.5))
                        .color(foundation_block_text()),
                );
            }
        });
        let actions_rect = if bitmap_header {
            header_content_rect
        } else if extra_header_height > 0.0 {
            egui::Rect::from_min_max(
                egui::pos2(
                    header_content_rect.min.x,
                    header_content_rect.max.y - extra_header_height + 2.0,
                ),
                header_content_rect.max,
            )
        } else if setup_header || animation_header {
            let title_width = ui
                .painter()
                .layout_no_wrap(title.to_owned(), bold_font(12.5), foundation_block_text())
                .size()
                .x;
            egui::Rect::from_min_max(
                egui::pos2(
                    header_content_rect.min.x + title_width + 16.0,
                    header_content_rect.min.y,
                ),
                header_content_rect.max,
            )
        } else {
            header_content_rect
        };
        let mut actions_ui = ui.new_child(egui::UiBuilder::new().max_rect(actions_rect));
        if wrapped_bitmap_header {
            actions_ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                add_contents(ui, ModelPreviewSectionPart::Header);
            });
        } else if setup_header || animation_header || bitmap_header {
            actions_ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                add_contents(ui, ModelPreviewSectionPart::Header);
            });
        } else {
            actions_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                add_contents(ui, ModelPreviewSectionPart::Header);
            });
        }

        ui.add_space(-ui.spacing().item_spacing.y);
        let body = egui::Frame::none()
            .fill(foundation_group_bg())
            .rounding(egui::Rounding {
                nw: 0.0,
                ne: 0.0,
                sw: RADIUS,
                se: RADIUS,
            })
            .inner_margin(egui::Margin::same(if edge_to_edge { 0.0 } else { 8.0 }))
            .show(ui, |ui| {
                ui.set_min_width((width - if edge_to_edge { 0.0 } else { 16.0 }).max(1.0));
                if let Some(min_body_height) = min_body_height {
                    ui.set_min_height(min_body_height);
                }
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    if edge_to_edge {
                        ui.spacing_mut().item_spacing.y = 0.0;
                    }
                    add_contents(ui, ModelPreviewSectionPart::Body);
                });
            });
        let container_rect = egui::Rect::from_min_max(header_rect.min, body.response.rect.max);
        ui.painter().rect_stroke(
            container_rect,
            RADIUS,
            Stroke::new(1.0, foundation_group_edge()),
        );
        container_rect
    })
    .inner
}

#[allow(clippy::too_many_arguments)]
fn draw_model_view_settings_menu(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    supports_textures: bool,
    is_campaign_evolved: bool,
    has_armature: bool,
) {
    preview_header_menu(
        ui,
        ButtonIcon::View,
        &format!("View: {}", state.render_mode.label()),
        |ui| {
            const VIEW_SETTINGS_WIDTH: f32 = 280.0;
            // Fix both bounds: a menu's sizing pass can otherwise let the
            // full-width marker filter grow wider than the shading combo.
            ui.set_width(VIEW_SETTINGS_WIDTH);
            ui.scope(|ui| {
                // Menu styling uses a compact 2 px inset; match the variant
                // selector's normal button padding for this combo box.
                ui.spacing_mut().button_padding.x = BUTTON_TEXT_PADDING_X;
                ui.visuals_mut().widgets.inactive.weak_bg_fill =
                    foundation_visuals().widgets.inactive.weak_bg_fill;
                egui::ComboBox::from_id_salt(("model_render_mode", &entry.key))
                    .selected_text(state.render_mode.label())
                    .width(VIEW_SETTINGS_WIDTH)
                    .show_ui(ui, |ui| {
                        for mode in ModelRenderMode::ALL {
                            if supports_textures || !mode.uses_textures() {
                                ui.selectable_value(&mut state.render_mode, mode, mode.label());
                            }
                        }
                    });
            });
            if is_campaign_evolved {
                ui.checkbox(&mut state.high_detail, "High Detail")
                    .on_hover_text(
                        "Decode full-resolution Nanite geometry instead of Unreal's coarse fallback.",
                    );
            }
            ui.separator();

            if tag.header.group_tag.to_be_bytes() == *b"hlmt" && !is_campaign_evolved {
                model_view_icon_checkbox(
                    ui,
                    &mut state.show_render,
                    ModelViewCheckboxIcon::Tag(*b"mode"),
                    "Render Model",
                );
                model_view_icon_checkbox(
                    ui,
                    &mut state.show_collision,
                    ModelViewCheckboxIcon::Tag(*b"coll"),
                    "Collision Model",
                );
                model_view_icon_checkbox(
                    ui,
                    &mut state.show_physics,
                    ModelViewCheckboxIcon::Tag(*b"phmo"),
                    "Physics Model",
                );
                if state.overlays_pending && (state.show_collision || state.show_physics) {
                    ui.spinner();
                }
            }
            ui.add_enabled_ui(has_armature, |ui| {
                model_view_icon_checkbox(
                    ui,
                    &mut state.show_armature,
                    ModelViewCheckboxIcon::Tag(*b"jmad"),
                    "Armature",
                )
                .on_hover_text("Draw the model skeleton; hover a joint to see its name.");
            });

            ui.separator();
            model_view_icon_checkbox(
                ui,
                &mut state.show_markers,
                ModelViewCheckboxIcon::Markers,
                "Show Markers",
            );
            draw_marker_filter_field(ui, &mut state.marker_filter);

            let error_count = data.preview.errors.len();
            ui.add_enabled_ui(error_count > 0, |ui| {
                model_view_icon_checkbox(
                    ui,
                    &mut state.show_errors,
                    ModelViewCheckboxIcon::Errors,
                    "Show Errors",
                )
                .on_hover_text(format!(
                    "Highlight {error_count} error/warning report primitive(s); hover one to see its report."
                ));
                if state.show_errors {
                    ui.indent("model_error_filters", |ui| {
                        ui.checkbox(
                            &mut state.show_non_critical_errors,
                            "Show Non-Critical",
                        )
                        .on_hover_text("Include error and warning reports marked non-critical.");
                    });
                }
            });

            ui.separator();
            ui.checkbox(&mut state.show_grid, "Show Grid")
                .on_hover_text(
                    "Ground-reference grid on the z = 0 plane, spaced to the model's size.",
                );
            ui.checkbox(&mut state.show_backfaces, "Render Backfaces");
        },
    );
    preview_header_menu(ui, ButtonIcon::Find, "Camera", |ui| {
        ui.set_min_width(280.0);
        ui.horizontal(|ui| {
            ui.label("Zoom");
            ui.add(
                egui::Slider::new(&mut state.scale, MIN_PREVIEW_SCALE..=MAX_PREVIEW_SCALE)
                    .logarithmic(true)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            );
            let drag_speed = (state.scale * 0.05).max(0.01) as f64;
            ui.add(
                egui::DragValue::new(&mut state.scale)
                    .range(MIN_PREVIEW_SCALE..=MAX_PREVIEW_SCALE)
                    .speed(drag_speed)
                    .max_decimals(2)
                    .suffix("×"),
            );
        });
        ui.checkbox(&mut state.perspective, "Perspective Projection")
            .on_hover_text(
                "Perspective projection instead of the flat orthographic view. The framing at \
                 the orbit point stays identical, so toggling never jumps.",
            );
        ui.horizontal(|ui| {
            ui.label("Preview Size");
            ui.add(
                egui::Slider::new(
                    model_preview_size,
                    MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
                )
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
            );
            draw_model_viewport_size_input(ui, model_preview_size);
        });
        if ui.button("Reset View").clicked() {
            state.yaw = -0.45;
            state.pitch = 0.25;
            state.focus = [0.0; 3];
            state.scale = 1.0;
        }
    });
}

#[derive(Clone, Copy)]
enum ModelViewCheckboxIcon {
    Tag([u8; 4]),
    Markers,
    Errors,
}

fn model_view_icon_checkbox(
    ui: &mut Ui,
    checked: &mut bool,
    icon: ModelViewCheckboxIcon,
    label: &str,
) -> egui::Response {
    let row = ui.horizontal(|ui| {
        let checkbox_response = ui.checkbox(checked, "");
        // Native checkbox text starts before the end of an empty checkbox's
        // 24-point hitbox. Put the icon at that same text start, without
        // overlapping the two clickable hitboxes.
        let icon_left =
            checkbox_response.rect.left() + ui.spacing().icon_width + ui.spacing().icon_spacing;
        let interaction_left = icon_left.max(checkbox_response.rect.right());
        ui.spacing_mut().item_spacing.x = interaction_left - checkbox_response.rect.right();
        let interaction_width = (icon_left + 16.0 - interaction_left).max(1.0);
        let (icon_hit_rect, icon_response) =
            ui.allocate_exact_size(Vec2::new(interaction_width, BUTTON_HEIGHT), Sense::click());
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(icon_left, icon_hit_rect.center().y - 8.0),
            Vec2::splat(16.0),
        );
        match icon {
            ModelViewCheckboxIcon::Tag(group) => {
                paint_tag_icon_at(ui, Some(u32::from_be_bytes(group)), icon_rect);
            }
            ModelViewCheckboxIcon::Markers => {
                paint_button_icon_at(ui, ButtonIcon::Markers, icon_rect, text_dark());
            }
            ModelViewCheckboxIcon::Errors => {
                paint_button_icon_at(ui, ButtonIcon::Errors, icon_rect, text_dark());
            }
        }
        ui.spacing_mut().item_spacing.x = 4.0;
        let label_response = ui.add(egui::Label::new(label).sense(Sense::click()));
        if icon_response.clicked() || label_response.clicked() {
            *checked = !*checked;
        }
        checkbox_response
    });
    if ui.rect_contains_pointer(row.response.rect) && !row.inner.hovered() {
        paint_checkbox_row_hover(ui, row.inner.rect, *checked);
    }
    row.response
}

fn draw_marker_filter_field(ui: &mut Ui, filter: &mut String) -> egui::Response {
    const HEIGHT: f32 = 24.0;
    const ICON_SIZE: f32 = 16.0;
    let width = ui.available_width().max(HEIGHT);
    let (rect, background_response) =
        ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::hover());
    let rounding = ui.visuals().widgets.inactive.rounding;
    ui.painter()
        .rect_filled(rect, rounding, ui.visuals().extreme_bg_color);

    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 4.0, rect.center().y - ICON_SIZE * 0.5),
        Vec2::splat(ICON_SIZE),
    );
    paint_button_icon_at(ui, ButtonIcon::Filter, icon_rect, text_dark());
    let icon_response = ui.interact(
        icon_rect,
        background_response.id.with("marker_filter_icon"),
        Sense::click(),
    );
    let edit_rect = egui::Rect::from_min_max(
        egui::pos2(icon_rect.right() + 8.0, rect.top()),
        egui::pos2(rect.right() - 8.0, rect.bottom()),
    );
    let edit_response = ui.put(
        edit_rect,
        egui::TextEdit::singleline(filter)
            .hint_text(placeholder_text("Filter Markers…"))
            .text_color(text_dark())
            .frame(false)
            .margin(egui::Margin::same(0.0))
            .vertical_align(egui::Align::Center)
            .min_size(edit_rect.size()),
    );
    if icon_response.clicked() {
        edit_response.request_focus();
    }
    let response = background_response
        .union(icon_response)
        .union(edit_response.clone());
    let stroke = if edit_response.has_focus() {
        ui.visuals().selection.stroke
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_stroke
    } else {
        Stroke::new(1.0, foundation_input_edge())
    };
    ui.painter().rect_stroke(rect, rounding, stroke);
    response
}

fn preview_header_menu(
    ui: &mut Ui,
    icon: ButtonIcon,
    label: &str,
    add_contents: impl FnOnce(&mut Ui),
) {
    icon_text_dropdown_button(ui, icon, label, add_contents);
}

/// What the preview panel and its tab call themselves, by tag group. Only
/// `.model`-family tags actually show a *render model*; the derived previews
/// name what they draw.
pub(in crate::app) fn preview_panel_title(group_tag: u32) -> &'static str {
    match &group_tag.to_be_bytes() {
        b"coll" => "Collision Model",
        b"phmo" => "Physics Model",
        b"sbsp" => "Structure BSP",
        b"scnr" => "Scenario Geometry",
        _ => "Model Preview",
    }
}

pub(in crate::app) fn draw_model_viewport_size_input(ui: &mut Ui, model_preview_size: &mut f32) {
    let mut percent = model_preview_size_percent(*model_preview_size);
    let response = ui.add(
        egui::DragValue::new(&mut percent)
            .range(
                model_preview_size_percent(MIN_MODEL_PREVIEW_SIZE)
                    ..=model_preview_size_percent(MAX_MODEL_PREVIEW_SIZE),
            )
            .speed(1.0)
            .max_decimals(0)
            .suffix("%"),
    );
    if response.changed() {
        *model_preview_size = model_preview_size_from_percent(percent);
    }
}

fn model_preview_size_percent(model_preview_size: f32) -> f32 {
    model_preview_size * 100.0
}

fn model_preview_size_from_percent(percent: f32) -> f32 {
    (percent / 100.0).clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE)
}

fn model_viewport_size(available_width: f32, model_preview_size: f32) -> Vec2 {
    let scale = model_preview_size.clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE);
    let desired = Vec2::new(470.0 * scale, 300.0 * scale);
    // Never force a minimum wider than the actual section. Doing so made the
    // GL callback overflow its panel while egui clipped only the width, which
    // visually stretched the model as the window narrowed.
    let width = desired.x.min(available_width.max(1.0));
    Vec2::new(width, desired.y * (width / desired.x))
}

fn draw_model_viewport_with_stats(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    desired_size: Vec2,
) {
    // Hold the viewport until the textures land, rather than drawing the model
    // untextured and re-shading it a second later — a model that changes
    // appearance under the cursor reads as a glitch, not as progress.
    if state.render_mode.uses_textures() && state.textures_pending && data.textures.is_none() {
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        ui.painter()
            .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.horizontal_centered(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading shaders…").color(subtle_dark()));
                });
            });
        });
        ui.ctx().request_repaint();
    } else {
        draw_model_viewport(ui, data, state, desired_size);
    }
    let footer_width = ui.available_width().max(desired_size.x);
    let (footer_rect, _) = ui.allocate_exact_size(
        Vec2::new(footer_width, MODEL_PREVIEW_STATS_FOOTER_HEIGHT),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(footer_rect, 0.0, foundation_section_bar());
    ui.painter().text(
        footer_rect.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        format!(
            "{} vertices, {} triangles",
            data.preview.vertices.len(),
            data.preview.indices.len() / 3
        ),
        FontId::proportional(10.0),
        foundation_block_text(),
    );
    // Why a material drew untextured, which the resolve records and nothing
    // showed: the model just looked flat.
    if state.render_mode.uses_textures()
        && let Some(textures) = data.textures.as_deref()
        && let Some((summary, detail)) = texture_resolve_note(textures, &data.preview.materials)
    {
        ui.label(RichText::new(summary).small().color(subtle_dark()))
            .on_hover_text(detail);
    }
}

/// A line saying which materials resolved badly, and the reasons, for hover.
/// `None` when every material resolved from its definition.
fn texture_resolve_note(
    textures: &[MaterialTextures],
    materials: &[RenderModelPreviewMaterial],
) -> Option<(String, String)> {
    let name = |index: usize| {
        materials
            .get(index)
            .map(|material| material.shader_path.as_str())
            .filter(|path| !path.is_empty())
            .unwrap_or("(no shader)")
            .to_owned()
    };
    let failed: Vec<String> = textures
        .iter()
        .enumerate()
        .filter_map(|(index, texture)| {
            texture
                .error
                .as_ref()
                .map(|error| format!("{}: {error}", name(index)))
        })
        .collect();
    let partial: Vec<String> = textures
        .iter()
        .enumerate()
        .filter(|(_, texture)| texture.error.is_none() && texture.used_shader_parameters_only)
        .map(|(index, _)| name(index))
        .collect();
    if failed.is_empty() && partial.is_empty() {
        return None;
    }
    let mut summary = Vec::new();
    let mut detail = Vec::new();
    if !failed.is_empty() {
        summary.push(format!(
            "{} of {} materials untextured",
            failed.len(),
            textures.len()
        ));
        detail.extend(failed);
    }
    if !partial.is_empty() {
        summary.push(format!(
            "{} read without their render method definition",
            partial.len()
        ));
        detail.push(format!(
            "Read from the shader's own parameters, without option defaults such as \
             detail-map tiling:\n{}",
            partial.join("\n")
        ));
    }
    Some((
        format!("{} — hover for why", summary.join("; ")),
        detail.join("\n"),
    ))
}

/// Build the renderer's camera-independent data for a standalone mesh, such as
/// a Chimp StaticMesh/SkeletalMesh document.
pub(in crate::app) fn standalone_mesh_preview(
    source_key: String,
    preview: RenderModelPreview,
) -> ModelPreviewData {
    model_preview_data(source_key.clone(), source_key, preview, Vec::new())
}

/// Draw a standalone mesh with the same camera, shading, wireframe and
/// backface controls as Baboon's tag model viewer.
pub(in crate::app) fn draw_standalone_mesh_preview(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    // Chimp presents raw Unreal sections, including intentionally two-sided
    // surfaces. Keep backfaces visible unconditionally; classic tag viewers
    // continue to use their own independent toggle in draw_model_preview_panel.
    state.show_backfaces = true;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Scale").color(subtle_dark()));
        ui.add(
            egui::Slider::new(&mut state.scale, MIN_PREVIEW_SCALE..=MAX_PREVIEW_SCALE)
                .logarithmic(true)
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
        let drag_speed = (state.scale * 0.05).max(0.01) as f64;
        ui.add(
            egui::DragValue::new(&mut state.scale)
                .range(MIN_PREVIEW_SCALE..=MAX_PREVIEW_SCALE)
                .speed(drag_speed)
                .max_decimals(2)
                .suffix("×"),
        );
        if ui.button("Reset view").clicked() {
            state.yaw = -0.45;
            state.pitch = 0.25;
            state.focus = [0.0; 3];
            state.scale = 1.0;
        }
        egui::ComboBox::from_id_salt(("standalone_model_render_mode", &data.source_key))
            .selected_text(state.render_mode.label())
            .show_ui(ui, |ui| {
                for mode in ModelRenderMode::ALL {
                    ui.selectable_value(&mut state.render_mode, mode, mode.label());
                }
            });
        ui.checkbox(&mut state.perspective, "Perspective")
            .on_hover_text("Perspective projection instead of the flat orthographic view.");
        ui.checkbox(&mut state.show_grid, "Grid")
            .on_hover_text("Ground-reference grid on the z = 0 plane.");
        ui.add_enabled(
            false,
            egui::Checkbox::new(&mut state.show_backfaces, "Backfaces"),
        )
        .on_hover_text("Chimp always displays backfaces");
    });
    let size = Vec2::new(
        ui.available_width().max(280.0),
        ui.available_height().max(300.0),
    );
    draw_model_viewport(ui, data, state, size);
    ui.small(
        RichText::new(format!(
            "{} vertices, {} triangles",
            data.preview.vertices.len(),
            data.preview.indices.len() / 3
        ))
        .color(subtle_dark()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap_header_control_rows(screen_width: f32, wrap_width: f32) -> (f32, f32) {
        let context = egui::Context::default();
        let mut left_y = 0.0;
        let mut right_y = 0.0;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(screen_width, 160.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    draw_model_preview_section_with_header_wrap(
                        ui,
                        "Bitmap Preview",
                        Some(1.0),
                        Some(wrap_width),
                        |ui, part| {
                            if part == ModelPreviewSectionPart::Header {
                                left_y = ui.button("Selector").rect.center().y;
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        right_y = ui.button("Actions").rect.center().y;
                                    },
                                );
                            }
                        },
                    );
                });
            },
        );
        (left_y, right_y)
    }

    #[test]
    fn bitmap_header_moves_actions_to_a_second_row_below_its_wrap_width() {
        let (wide_left, wide_right) = bitmap_header_control_rows(640.0, 400.0);
        assert!((wide_left - wide_right).abs() < 1.0);

        let (narrow_left, narrow_right) = bitmap_header_control_rows(320.0, 400.0);
        assert!(narrow_right > narrow_left + BUTTON_HEIGHT);
    }

    #[test]
    fn model_setup_header_wraps_only_when_controls_need_room() {
        assert_eq!(model_setup_extra_header_height(559.0), 64.0);
        assert_eq!(model_setup_extra_header_height(560.0), 32.0);
        assert_eq!(
            model_setup_extra_header_height(679.0),
            MODEL_SETUP_EXTRA_HEADER_HEIGHT
        );
        assert_eq!(model_setup_extra_header_height(680.0), 0.0);
    }

    #[test]
    fn marker_filter_field_matches_button_height() {
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        let mut filter = String::new();
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(320.0, 100.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let available = ui.available_width();
                    let response = draw_marker_filter_field(ui, &mut filter);
                    assert_eq!(response.rect.height(), BUTTON_HEIGHT);
                    assert_eq!(response.rect.width(), available);
                });
            },
        );
    }

    #[test]
    fn model_setup_header_groups_stay_inside_their_card() {
        let data = model_preview_data(
            String::new(),
            String::new(),
            RenderModelPreview::default(),
            Vec::new(),
        );
        for width in [400.0, 559.0, 560.0, 679.0, 680.0, 900.0] {
            let context = egui::Context::default();
            context.set_fonts(foundation_fonts());
            context.set_style(foundation_style());
            let mut state = ModelPreviewState::default();
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width + 16.0, 400.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        ui.set_width(width);
                        let mut actions_bounds = egui::Rect::NOTHING;
                        let mut actions_area = egui::Rect::NOTHING;
                        let card =
                            draw_model_preview_section(ui, "Model Setup", None, |ui, part| {
                                if part == ModelPreviewSectionPart::Header {
                                    actions_area = ui.max_rect();
                                    draw_model_setup_header_controls(
                                        ui,
                                        width,
                                        &data,
                                        &mut state,
                                        |ui, _| {
                                            icon_text_dropdown_button(
                                                ui,
                                                ButtonIcon::Save,
                                                "Save",
                                                |_| {},
                                            );
                                            icon_text_button(
                                                ui,
                                                ButtonIcon::Garbage,
                                                "Delete",
                                                true,
                                            );
                                            icon_text_button(
                                                ui,
                                                ButtonIcon::Refresh,
                                                "Refresh Model",
                                                true,
                                            );
                                        },
                                    );
                                    actions_bounds = ui.min_rect();
                                }
                            });
                        assert!(
                            actions_bounds.right() <= actions_area.right() + 1.0,
                            "width {width}: controls {actions_bounds:?}, area {actions_area:?}"
                        );
                        assert!(
                            actions_bounds.bottom() <= actions_area.bottom() + 1.0,
                            "width {width}: controls {actions_bounds:?}, area {actions_area:?}"
                        );
                        assert!(card.width() <= width + 1.0);
                    });
                },
            );
        }
    }

    #[test]
    fn physics_overlay_is_not_a_model_setup_variant_region() {
        let preview = RenderModelPreview {
            regions: vec![
                RenderModelPreviewRegion {
                    name: "body".into(),
                    permutations: vec!["default".into()],
                },
                RenderModelPreviewRegion {
                    name: PHYSICS_REGION.into(),
                    permutations: vec!["default".into()],
                },
            ],
            batches: vec![
                RenderModelPreviewBatch {
                    region_name: "body".into(),
                    layer: ModelPreviewLayer::Render,
                    ..Default::default()
                },
                RenderModelPreviewBatch {
                    region_name: PHYSICS_REGION.into(),
                    layer: ModelPreviewLayer::Physics,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
        let mut state = ModelPreviewState {
            overlays_loaded: true,
            ..Default::default()
        };
        reset_model_preview_selection(&mut state, &data, None);

        assert!(is_model_physics_overlay_region(
            &data,
            &state,
            &data.preview.regions[1]
        ));
        assert!(state.region_selections[PHYSICS_REGION].enabled);
        assert_eq!(selected_variant_regions(&data, &state).len(), 1);
        assert_eq!(
            selected_variant_regions(&data, &state)[0].region_name,
            "body"
        );

        state.overlays_loaded = false;
        assert!(!is_model_physics_overlay_region(
            &data,
            &state,
            &data.preview.regions[1]
        ));
    }

    #[test]
    fn viewport_percentage_conversion_clamps_to_persisted_range() {
        assert_eq!(model_preview_size_percent(1.25), 125.0);
        assert_eq!(model_preview_size_from_percent(125.0), 1.25);
        assert_eq!(
            model_preview_size_from_percent(20.0),
            MIN_MODEL_PREVIEW_SIZE
        );
        assert_eq!(
            model_preview_size_from_percent(400.0),
            MAX_MODEL_PREVIEW_SIZE
        );
    }

    #[test]
    fn viewport_shrinks_without_changing_aspect_ratio() {
        let size = model_viewport_size(200.0, 1.0);
        assert_eq!(size.x, 200.0);
        assert!((size.x / size.y - 470.0 / 300.0).abs() < 0.000_001);
    }

    #[test]
    fn variant_buttons_wrap_without_widening_a_narrow_setup_card() {
        let preview = RenderModelPreview {
            regions: vec![RenderModelPreviewRegion {
                name: "helmet".into(),
                permutations: vec![
                    "minor".into(),
                    "chiefweapon".into(),
                    "jump_pack".into(),
                    "stalker".into(),
                ],
            }],
            ..Default::default()
        };
        let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
        for width in [280.0, 360.0, 520.0] {
            let context = egui::Context::default();
            context.set_fonts(foundation_fonts());
            let mut state = ModelPreviewState::default();
            let mut card_rect = egui::Rect::NOTHING;
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 600.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let available = ui.available_width();
                        card_rect =
                            draw_model_preview_section(ui, "Model Setup", None, |ui, part| {
                                if part == ModelPreviewSectionPart::Body {
                                    draw_variant_controls(ui, &data, &mut state);
                                }
                            });
                        assert!(
                            card_rect.width() <= available + 1.0,
                            "pane {width}: card {}, available {available}",
                            card_rect.width()
                        );
                    });
                },
            );
        }
    }

    #[test]
    fn variant_rows_are_not_capped_shorter_than_the_setup_card() {
        let preview = RenderModelPreview {
            regions: (0..20)
                .map(|index| RenderModelPreviewRegion {
                    name: format!("region_{index}"),
                    permutations: vec!["default".into()],
                })
                .collect(),
            ..Default::default()
        };
        let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        let mut state = ModelPreviewState::default();
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(500.0, 900.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let top = ui.next_widget_position().y;
                    draw_variant_controls(ui, &data, &mut state);
                    assert!(ui.min_rect().bottom() - top > 500.0);
                });
            },
        );
    }

    #[test]
    fn animation_groups_and_scrubber_fit_narrow_cards() {
        for width in [280.0, 360.0, 520.0, 800.0] {
            let context = egui::Context::default();
            context.set_fonts(foundation_fonts());
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 600.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        let available = ui.available_width();
                        let rect =
                            draw_model_preview_section(ui, "Animation Player", None, |ui, part| {
                                if part == ModelPreviewSectionPart::Header {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing.x = 16.0;
                                        animation_header_group(ui, 240.0, |ui| {
                                            ui.add_sized(
                                                Vec2::new(240.0, BUTTON_HEIGHT),
                                                egui::Button::new("Animation"),
                                            );
                                        });
                                        animation_header_group(ui, 112.0, |ui| {
                                            for _ in 0..4 {
                                                ui.add_sized(
                                                    ICON_BUTTON_SIZE,
                                                    egui::Button::new(""),
                                                );
                                            }
                                        });
                                        animation_header_group(ui, 90.0, |ui| {
                                            ui.label("Speed");
                                            ui.add_sized(
                                                Vec2::new(48.0, BUTTON_HEIGHT),
                                                egui::DragValue::new(&mut 1.0_f32),
                                            );
                                        });
                                        ui.label("2 / 9");
                                    });
                                } else {
                                    let mut frame = 2.0;
                                    ui.spacing_mut().slider_width =
                                        (ui.available_width() - 2.0).max(1.0);
                                    ui.add(
                                        egui::Slider::new(&mut frame, 0.0..=9.0).show_value(false),
                                    );
                                }
                            });
                        assert!(
                            rect.width() <= available + 1.0,
                            "pane {width}: card {}, available {available}",
                            rect.width()
                        );
                    });
                },
            );
        }
    }

    #[test]
    fn preview_scale_resizes_the_wide_section_until_setup_reaches_its_minimum() {
        assert_eq!(wide_model_preview_section_width(1_600.0, 1.0), 470.0);
        assert_eq!(wide_model_preview_section_width(1_600.0, 1.5), 705.0);
        assert_eq!(
            wide_model_preview_section_width(1_000.0, 2.0),
            1_000.0 - MODEL_PREVIEW_SECTION_GAP - WIDE_MODEL_SETUP_MIN_WIDTH
        );
    }

    #[test]
    fn preview_and_setup_cards_match_outer_height_at_both_header_breakpoints() {
        for setup_width in [360.0, 800.0] {
            for scale in [1.0, 1.5] {
                let context = egui::Context::default();
                context.set_fonts(foundation_fonts());
                let viewport = model_viewport_size(470.0 * scale, scale);
                let shared_body_height = viewport.y + MODEL_PREVIEW_STATS_FOOTER_HEIGHT;
                let setup_body_height =
                    shared_body_height - model_setup_extra_header_height(setup_width) - 16.0;
                let mut preview_rect = egui::Rect::NOTHING;
                let mut setup_rect = egui::Rect::NOTHING;
                let _ = context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(2_000.0, 1_200.0),
                        )),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.allocate_ui(Vec2::new(viewport.x, 0.0), |ui| {
                                    ui.set_width(viewport.x);
                                    preview_rect = draw_model_preview_section(
                                        ui,
                                        "Model Preview",
                                        Some(shared_body_height),
                                        |ui, part| {
                                            if part == ModelPreviewSectionPart::Body {
                                                ui.allocate_exact_size(viewport, Sense::hover());
                                                ui.allocate_exact_size(
                                                    Vec2::new(
                                                        viewport.x,
                                                        MODEL_PREVIEW_STATS_FOOTER_HEIGHT,
                                                    ),
                                                    Sense::hover(),
                                                );
                                            }
                                        },
                                    );
                                });
                                ui.allocate_ui(Vec2::new(setup_width, 0.0), |ui| {
                                    ui.set_width(setup_width);
                                    setup_rect = draw_model_preview_section(
                                        ui,
                                        "Model Setup",
                                        Some(setup_body_height),
                                        |ui, part| {
                                            if part == ModelPreviewSectionPart::Body {
                                                egui::ScrollArea::vertical()
                                                    .auto_shrink([false, false])
                                                    .max_height(setup_body_height)
                                                    .show(ui, |ui| {
                                                        ui.set_min_height(setup_body_height * 2.0);
                                                    });
                                            }
                                        },
                                    );
                                });
                            });
                        });
                    },
                );
                assert!(
                    (preview_rect.height() - setup_rect.height()).abs() < 1.0,
                    "scale {scale}, setup width {setup_width}: preview={}, setup={}",
                    preview_rect.height(),
                    setup_rect.height()
                );
            }
        }
    }

    #[test]
    fn section_body_stays_below_header_inside_a_horizontal_row() {
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        let mut header_rect = None;
        let mut body_rect = None;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 600.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.allocate_ui(Vec2::new(360.0, 0.0), |ui| {
                            draw_model_preview_section(ui, "Model Preview", None, |ui, part| {
                                match part {
                                    ModelPreviewSectionPart::Header => {
                                        header_rect = Some(ui.max_rect())
                                    }
                                    ModelPreviewSectionPart::Body => {
                                        body_rect = Some(ui.max_rect())
                                    }
                                }
                            });
                        });
                    });
                });
            },
        );

        let header_rect = header_rect.expect("header was drawn");
        let body_rect = body_rect.expect("body was drawn");
        assert!(body_rect.min.y >= header_rect.max.y);
        // The preview viewport now reaches the card edge; the title retains
        // its 8-point header inset.
        assert!((header_rect.min.x - body_rect.min.x - 8.0).abs() < 1.0);
    }

    #[test]
    fn campaign_evolved_preview_defaults_to_full_detail() {
        assert!(ModelPreviewState::default().high_detail);
    }

    #[test]
    fn model_geometry_uses_the_model_preview_tab_name() {
        assert_eq!(
            preview_panel_title(u32::from_be_bytes(*b"hlmt")),
            "Model Preview"
        );
        assert_eq!(
            preview_panel_title(u32::from_be_bytes(*b"mode")),
            "Model Preview"
        );
        assert_eq!(
            preview_panel_title(u32::from_be_bytes(*b"coll")),
            "Collision Model"
        );
    }
}

/// Move a playing animation's clock on by `dt`, once per egui pass however
/// many panes draw it (they share the state), wrapping or stopping at the end.
fn advance_playback_clock(
    playback: &mut PreviewAnimationPlayback,
    pass: u64,
    dt: f32,
    duration: f32,
    last_frame: f32,
) {
    if playback.advanced_in_pass == Some(pass) {
        return;
    }
    playback.advanced_in_pass = Some(pass);
    playback.time += dt * playback.speed.max(0.0);
    if playback.looped {
        if duration > 0.0 {
            playback.time %= duration;
        }
    } else if playback.time * ANIMATION_FRAME_RATE >= last_frame {
        playback.time = last_frame / ANIMATION_FRAME_RATE;
        playback.playing = false;
    }
}

#[cfg(test)]
mod playback_clock_tests {
    use super::*;

    /// Two panes on the same tag draw the same playback state in one pass; the
    /// clock moves once. Each pane used to move it, so playback ran at 2x.
    #[test]
    fn two_panes_advance_the_clock_once_per_pass() {
        let mut playback = PreviewAnimationPlayback {
            playing: true,
            ..Default::default()
        };
        advance_playback_clock(&mut playback, 7, 0.1, 10.0, 300.0);
        advance_playback_clock(&mut playback, 7, 0.1, 10.0, 300.0);
        assert!((playback.time - 0.1).abs() < 1e-6, "{}", playback.time);
        advance_playback_clock(&mut playback, 8, 0.1, 10.0, 300.0);
        assert!((playback.time - 0.2).abs() < 1e-6, "{}", playback.time);
    }
}

#[cfg(test)]
mod texture_note_tests {
    use super::*;

    /// A material that drew untextured says why, instead of just looking flat.
    #[test]
    fn untextured_materials_say_why() {
        let material = |path: &str| RenderModelPreviewMaterial {
            shader_path: path.to_owned(),
            ..Default::default()
        };
        let materials = [
            material("shaders/a"),
            material("shaders/b"),
            material("shaders/c"),
        ];
        let textures = [
            MaterialTextures {
                error: Some("shader tag not found".to_owned()),
                ..Default::default()
            },
            MaterialTextures {
                used_shader_parameters_only: true,
                ..Default::default()
            },
            MaterialTextures::default(),
        ];
        let (summary, detail) = texture_resolve_note(&textures, &materials).unwrap();
        assert_eq!(
            summary,
            "1 of 3 materials untextured; 1 read without their render method definition \
             — hover for why"
        );
        assert!(detail.contains("shaders/a: shader tag not found"));
        assert!(detail.contains("shaders/b"));
        assert!(!detail.contains("shaders/c"));
        assert_eq!(texture_resolve_note(&textures[2..], &materials[2..]), None);
    }
}
