//! A horizontal slider whose rail Baboon paints: egui fills its rail with
//! the handle's own colour, which in light mode is the panel's, so the rail
//! vanished.

use super::style::{foundation_group_bg, foundation_input, foundation_input_edge, is_dark_mode, slider_rail_dark};
use eframe::egui;
use eframe::egui::{Color32, Ui};

/// Adds `slider` (horizontal, `ui.spacing().slider_width` long) with its
/// rail painted under the handle: in light mode a sand well edged like an
/// input, in dark mode egui's own fill. `marks` adds shapes over the rail
/// and under the handle, given the slider's own rect, its value box left out.
///
/// Disabled, here or by its `ui`, it fades as one piece: egui fades each
/// shape on its own, so the rail would show through a see-through handle.
/// Each colour is mixed with the card's background instead, and drawn solid.
pub(in crate::app) fn railed_slider(
    ui: &mut Ui,
    slider: egui::Slider<'_>,
    enabled: bool,
    marks: impl FnOnce(egui::Rect) -> Vec<egui::Shape>,
) -> egui::Response {
    railed_slider_over(ui, slider, enabled, foundation_group_bg(), marks)
}

/// A [`railed_slider`] with no marks, for any slider in the app.
/// `background` is what it sits on, which a disabled slider fades toward.
pub(in crate::app) fn plain_railed_slider(
    ui: &mut Ui,
    slider: egui::Slider<'_>,
    enabled: bool,
    background: Color32,
) -> egui::Response {
    railed_slider_over(ui, slider, enabled, background, |_| Vec::new())
}

fn railed_slider_over(
    ui: &mut Ui,
    slider: egui::Slider<'_>,
    enabled: bool,
    background: Color32,
    marks: impl FnOnce(egui::Rect) -> Vec<egui::Shape>,
) -> egui::Response {
    ui.scope(|ui| {
        // How much of each colour shows: the opacity egui would have drawn at.
        let opacity = if enabled && ui.is_enabled() {
            1.0
        } else if ui.is_enabled() {
            ui.opacity() * ui.visuals().disabled_alpha()
        } else {
            ui.opacity()
        };
        let fade = move |color: Color32| {
            if opacity >= 1.0 {
                color
            } else {
                background.lerp_to_gamma(background.blend(color), opacity)
            }
        };
        if opacity < 1.0 {
            ui.disable();
            ui.set_opacity(1.0);
            // The value box's text, which Baboon sets for every widget.
            let text = &mut ui.visuals_mut().override_text_color;
            *text = text.map(fade);
            let widgets = &mut ui.visuals_mut().widgets;
            for visuals in [
                &mut widgets.noninteractive,
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
                &mut widgets.open,
            ] {
                visuals.bg_fill = fade(visuals.bg_fill);
                visuals.weak_bg_fill = fade(visuals.weak_bg_fill);
                visuals.bg_stroke.color = fade(visuals.bg_stroke.color);
                visuals.fg_stroke.color = fade(visuals.fg_stroke.color);
            }
        }
        let rail_height = ui.spacing().slider_rail_height;
        let rail_length = ui.spacing().slider_width;
        let (fill, edge) = if is_dark_mode() {
            (slider_rail_dark(), egui::Stroke::NONE)
        } else {
            (foundation_input(), egui::Stroke::new(1.0, foundation_input_edge()))
        };
        let corner = ui.visuals().widgets.inactive.corner_radius;
        // Reserved before the slider, so what goes here lies under its handle.
        let under = ui.painter().add(egui::Shape::Noop);
        ui.spacing_mut().slider_rail_height = 0.0;
        let response = ui.add(slider);
        // The rail is the slider's first `slider_width`; a value box follows.
        let span = egui::Rangef::new(
            response.rect.left(),
            (response.rect.left() + rail_length).min(response.rect.right()),
        );
        let middle = response.rect.center().y;
        let own = egui::Rect::from_x_y_ranges(span, response.rect.y_range());
        let rail = egui::Rect::from_x_y_ranges(span, egui::Rangef::point(middle).expand(rail_height / 2.0));
        let mut shapes = vec![
            egui::Shape::rect_filled(rail, corner, fade(fill)),
            egui::Shape::rect_stroke(
                rail,
                corner,
                egui::Stroke::new(edge.width, fade(edge.color)),
                egui::StrokeKind::Inside,
            ),
        ];
        shapes.extend(marks(own).into_iter().map(|shape| match shape {
            egui::Shape::LineSegment { points, stroke } => egui::Shape::LineSegment {
                points,
                stroke: egui::Stroke::new(stroke.width, fade(stroke.color)),
            },
            other => other,
        }));
        ui.painter().set(under, egui::Shape::Vec(shapes));
        response
    })
    .inner
}
