//! The animation player: the clip picker, the transport, the scrubber and its clock.

use super::super::super::ui_kit::button_icons::{ButtonIcon, selectable_icon_button};
use super::super::super::ui_kit::style::{BUTTON_HEIGHT, subtle_dark, text_dark};
use super::super::{
    ANIMATION_FRAME_RATE, ModelPreviewState, PreviewAnimationEntry, PreviewAnimationPlayback,
    picker_button, picker_popup_width, picker_results, picker_search_field,
};
use super::layout::PreviewPanel;
use super::section::{
    ModelPreviewSectionPart, animation_extra_header_height, draw_model_preview_section,
};
use eframe::egui;
use eframe::egui::{Color32, FontId, RichText, Ui, Vec2};

pub(in crate::app::model_preview) fn animation_frame_position(
    playback: &PreviewAnimationPlayback,
    pose_frames: usize,
) -> f32 {
    super::super::animation::playback_frame_position(playback, pose_frames)
}

pub(in crate::app::model_preview) fn animation_frame_label(
    position: f32,
    pose_frames: usize,
) -> String {
    let frame = if pose_frames > 0 {
        position.floor() as usize + 1
    } else {
        0
    };
    format!("{frame} / {pose_frames}")
}

/// Where each frame lands on the timeline slider in `rect`, whose value runs
/// 0..=`range_end`: along the handle's own travel, which egui insets from the
/// rect by the handle's radius at each end. Ticks closer than 3 points would
/// merge into a bar, so a long animation is ticked every 5th, 10th, 30th…
/// frame instead.
pub(in crate::app::model_preview) fn frame_tick_xs(
    rect: egui::Rect,
    handle_shape: egui::style::HandleShape,
    range_end: f32,
    pose_frames: usize,
) -> Vec<f32> {
    if pose_frames < 2 || range_end <= 0.0 {
        return Vec::new();
    }
    let radius = rect.height() / 2.5;
    let radius = match handle_shape {
        egui::style::HandleShape::Circle => radius,
        egui::style::HandleShape::Rect { aspect_ratio } => radius * aspect_ratio,
    };
    let travel = rect.x_range().shrink(radius);
    let per_frame = travel.span() / range_end;
    let Some(every) = [1usize, 5, 10, 30, 60, 150, 300, 600, 1500]
        .into_iter()
        .find(|&every| per_frame * every as f32 >= 3.0)
    else {
        return Vec::new();
    };
    (0..pose_frames)
        .step_by(every)
        .map(|frame| travel.min + per_frame * frame as f32)
        .collect()
}

pub(in crate::app::model_preview) fn animation_header_group(
    ui: &mut Ui,
    width: f32,
    add_contents: impl FnOnce(&mut Ui),
) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, BUTTON_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        add_contents,
    );
}

pub(in crate::app::model_preview) fn draw_animation_combo(
    ui: &mut Ui,
    entry_key: &str,
    animations: &[PreviewAnimationEntry],
    hidden: &std::collections::BTreeSet<usize>,
    playback: &mut PreviewAnimationPlayback,
    width: f32,
) {
    let selected_text = playback
        .selected
        .and_then(|index| animations.get(index))
        .map(|entry| entry.name.as_str())
        .unwrap_or("<None>");
    let popup_id = ui.make_persistent_id(("model_animation_popup", entry_key));
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    // The hidden set comes from the graph, read apart from this list.
    let shown = animations.len() - hidden.range(..animations.len()).count();
    let response = picker_button(
        ui,
        popup_id,
        selected_text,
        shown,
        width,
        text_dark(),
        shown > 0,
    );
    let just_opened = response.clicked() && !open;
    if response.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }
    // Open while its id is open in memory, as the button above toggles it;
    // left alone, a popup built from a response is always open.
    egui::Popup::from_response(&response)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            picker_popup_width(ui, &response);
            let filter_changed =
                picker_search_field(ui, &mut playback.filter, "search animations…", just_opened);
            ui.separator();
            let filter = playback.filter.trim().to_ascii_lowercase();
            picker_results(ui, 300.0, filter_changed, |ui| {
                let mut shown = 0;
                for (index, row) in animations.iter().enumerate() {
                    if hidden.contains(&index)
                        || (!filter.is_empty() && !row.name.to_ascii_lowercase().contains(&filter))
                    {
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
                            egui::Button::selectable(playback.selected == Some(index), label),
                        )
                        .clicked()
                    {
                        playback.selected = Some(index);
                        playback.pose = None;
                        playback.time = 0.0;
                        playback.playing = false;
                        playback.stopped = false;
                        playback.error = None;
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
                if shown == 0 {
                    ui.label(RichText::new("No animations match.").color(subtle_dark()));
                }
            });
        });
}

/// How far the selected clip runs, for the player's controls.
pub(in crate::app::model_preview) struct ClipFrames {
    pose_frames: usize,
    controls_enabled: bool,
    duration: f32,
    last_frame: f32,
}

/// The Animation Player body's inner height: the scrubber, and a line for a
/// clip's error under it.
pub(in crate::app::model_preview) const ANIMATION_BODY_HEIGHT: f32 = 44.0;

/// The Animation Player card's fixed height on a page `width` wide: its
/// header, with the rows it wraps to, over its body and the body's margins.
pub(in crate::app::model_preview) fn animation_card_height(width: f32) -> f32 {
    40.0 + animation_extra_header_height(width) + ANIMATION_BODY_HEIGHT + 16.0
}

/// The Animation Player card, docked along the bottom of the panel at a
/// fixed height, the full page width.
pub(in crate::app::model_preview) fn draw_animation_player(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
) {
    let body = Some(ANIMATION_BODY_HEIGHT);
    draw_model_preview_section(ui, "Animation Player", body, |ui, part| {
        let animations = panel
            .data
            .animations
            .as_deref()
            .map_or(&[][..], Vec::as_slice);
        let pose_frames = state
            .animation
            .pose
            .as_ref()
            .map(|pose| pose.frames.len())
            .unwrap_or(0);
        let controls_enabled = state.animation.selected.is_some() && pose_frames > 0;
        let duration = pose_frames as f32 / ANIMATION_FRAME_RATE;
        let last_frame = pose_frames.saturating_sub(1) as f32;
        let frames = ClipFrames {
            pose_frames,
            controls_enabled,
            duration,
            last_frame,
        };
        if part == ModelPreviewSectionPart::Header {
            draw_animation_header(ui, panel, animations, state, &frames);
        } else {
            draw_animation_scrubber(ui, state, &frames);
        }
    });
}

/// The Animation Player's header: the clip picker, the transport buttons,
/// the speed and the frame position.
pub(in crate::app::model_preview) fn draw_animation_header(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    animations: &[PreviewAnimationEntry],
    state: &mut ModelPreviewState,
    frames: &ClipFrames,
) {
    let ClipFrames {
        pose_frames,
        controls_enabled,
        last_frame,
        ..
    } = *frames;
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
                let no_clips = std::collections::BTreeSet::new();
                let hidden = panel
                    .data
                    .own_hidden_clips()
                    .or_else(|| {
                        panel
                            .data
                            .weapons
                            .as_ref()
                            .map(|weapons| &weapons.hidden_clips)
                    })
                    .unwrap_or(&no_clips);
                draw_animation_combo(
                    ui,
                    &panel.entry.key,
                    animations,
                    hidden,
                    &mut state.animation,
                    combo_width,
                );
            });
            if loading {
                ui.spinner();
            }
        });
        animation_header_group(ui, 140.0, |ui| {
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
            if selectable_icon_button(
                ui,
                ButtonIcon::Function,
                "Interpolate between frames (off: step frame by frame)",
                state.animation.interpolate,
                controls_enabled,
            )
            .clicked()
            {
                state.animation.interpolate = !state.animation.interpolate;
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
        let frame_position = animation_frame_position(&state.animation, pose_frames);
        ui.add(
            egui::Label::new(
                RichText::new(animation_frame_label(frame_position, pose_frames))
                    .color(subtle_dark()),
            )
            .wrap_mode(egui::TextWrapMode::Extend),
        );
    });
}

/// The Animation Player's body: the frame scrubber with its ticks, the
/// playback clock, and the clip's error.
pub(in crate::app::model_preview) fn draw_animation_scrubber(
    ui: &mut Ui,
    state: &mut ModelPreviewState,
    frames: &ClipFrames,
) {
    let ClipFrames {
        pose_frames,
        controls_enabled,
        duration,
        last_frame,
    } = *frames;
    ui.scope(|ui| {
        let mut scrub = animation_frame_position(&state.animation, pose_frames);
        ui.spacing_mut().slider_width = (ui.available_width() - 2.0).max(1.0);
        // Reserved before the slider so the ticks lie under its handle.
        let ticks = ui.painter().add(egui::Shape::Noop);
        let mut slider = egui::Slider::new(&mut scrub, 0.0..=last_frame.max(1.0)).show_value(false);
        if !state.animation.interpolate {
            slider = slider.step_by(1.0);
        }
        let response = ui.add_enabled(controls_enabled, slider);
        let tick_xs = frame_tick_xs(
            response.rect,
            ui.style().visuals.handle_shape,
            last_frame.max(1.0),
            pose_frames,
        );
        let rail_y = response.rect.center().y;
        let stroke = egui::Stroke::new(1.0, subtle_dark());
        ui.painter().set(
            ticks,
            egui::Shape::Vec(
                tick_xs
                    .into_iter()
                    .map(|x| {
                        egui::Shape::line_segment(
                            [egui::pos2(x, rail_y - 5.0), egui::pos2(x, rail_y + 5.0)],
                            stroke,
                        )
                    })
                    .collect(),
            ),
        );
        if response.changed() {
            state.animation.time = scrub / ANIMATION_FRAME_RATE;
            state.animation.playing = false;
            state.animation.stopped = false;
        }
        if controls_enabled && state.animation.playing {
            let pass = ui.ctx().cumulative_pass_nr();
            let dt = ui.input(|input| input.stable_dt).min(0.1);
            advance_playback_clock(&mut state.animation, pass, dt, duration, last_frame);
            ui.ctx().request_repaint();
        }
    });
    if let Some(error) = state.animation.error.clone() {
        ui.label(RichText::new(error).color(Color32::from_rgb(150, 56, 44)));
    }
}

/// Move a playing animation's clock on by `dt`, once per egui pass however
/// many panes draw it (they share the state), wrapping or stopping at the end.
pub(in crate::app::model_preview) fn advance_playback_clock(
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
            let frames = (duration * ANIMATION_FRAME_RATE).round() as usize;
            let position = playback.time * ANIMATION_FRAME_RATE;
            playback.time =
                super::super::animation::looped_frame_position(playback, position, frames)
                    / ANIMATION_FRAME_RATE;
        }
    } else if playback.time * ANIMATION_FRAME_RATE >= last_frame {
        playback.time = last_frame / ANIMATION_FRAME_RATE;
        playback.playing = false;
    }
}
