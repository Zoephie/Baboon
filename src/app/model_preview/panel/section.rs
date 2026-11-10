//! The section chrome the panel cards share with the bitmap editor: headers, sizing and the loading shells.

use super::super::super::ui_kit::style::{
    bold_font, foundation_block_text, foundation_group_bg, foundation_group_edge,
    foundation_section_bar,
};
use super::layout::{PREVIEW_HEADER_HEIGHT, PanelRects, in_rect};
use eframe::egui;
use eframe::egui::{RichText, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ModelPreviewSectionPart {
    Header,
    Body,
}

pub(in crate::app::model_preview) const MODEL_PREVIEW_STATS_FOOTER_HEIGHT: f32 = 28.0;

pub(in crate::app::model_preview) const MODEL_PREVIEW_SECTION_GAP: f32 = 8.0;

pub(in crate::app::model_preview) fn draw_loading_section_body(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(1.0), height.max(1.0)),
        Sense::hover(),
    );
    crate::app::shell::loading::paint_loading_rings(ui, rect);
}

/// Draw the panel's cards, and loading rings where the sidebar's will be,
/// before any model parsing
/// begins. The post-pane worker hook starts the actual build after this
/// frame, so a tab switch never has to wait for a complex render model or BSP
/// to be walked.
pub(in crate::app::model_preview) fn draw_model_preview_loading_shells(
    ui: &mut Ui,
    rects: &PanelRects,
) {
    in_rect(ui, rects.setup, "model_setup_loading", |ui| {
        draw_loading_section_body(ui, rects.setup.height());
    });
    let cards = [
        (rects.preview, "Model Preview", PREVIEW_HEADER_HEIGHT),
        (
            rects.animation,
            "Animation Player",
            PREVIEW_HEADER_HEIGHT + animation_extra_header_height(rects.animation.width()) + 16.0,
        ),
    ];
    for (rect, title, chrome) in cards {
        in_rect(ui, rect, title, |ui| {
            let body = (rect.height() - chrome).max(1.0);
            draw_model_preview_section(ui, title, Some(body), |ui, part| {
                if part == ModelPreviewSectionPart::Body {
                    draw_loading_section_body(ui, body);
                }
            });
        });
    }
}

/// The Animation Player header's extra rows at `width`: its picker and
/// transport wrap under the title as the page narrows.
pub(in crate::app::model_preview) fn animation_extra_header_height(width: f32) -> f32 {
    if width >= 800.0 {
        0.0
    } else if width < 320.0 {
        96.0
    } else if width < 560.0 {
        64.0
    } else {
        32.0
    }
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
        let animation_header = title == "Animation Player";
        let bitmap_header = title == "Bitmap Preview";
        let wrapped_bitmap_header = bitmap_header
            && header_wrap_width
                .map(|wrap_width| width < wrap_width)
                .unwrap_or(false);
        let edge_to_edge = matches!(title, "Model Preview" | "Bitmap Preview");
        let extra_header_height = if wrapped_bitmap_header {
            40.0
        } else if animation_header {
            animation_extra_header_height(width)
        } else {
            0.0
        };
        let header_height = 40.0 + extra_header_height;
        let (header_rect, _) =
            ui.allocate_exact_size(Vec2::new(width, header_height), Sense::hover());
        ui.painter().rect_filled(
            header_rect,
            egui::CornerRadius {
                nw: (RADIUS) as u8,
                ne: (RADIUS) as u8,
                sw: 0,
                se: 0,
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
        } else if animation_header {
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
        } else if animation_header || bitmap_header {
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
        let body = egui::Frame::NONE
            .fill(foundation_group_bg())
            .corner_radius(egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: (RADIUS) as u8,
                se: (RADIUS) as u8,
            })
            .inner_margin(egui::Margin::same(if edge_to_edge { 0 } else { 8 }))
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
            Stroke::new(1.0_f32, foundation_group_edge()),
            egui::StrokeKind::Middle,
        );
        container_rect
    })
    .inner
}
