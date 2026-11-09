//! Shared column-heading typography, bottom rule, and resize dividers.
use super::*;

pub(in crate::app) const TABLE_HEADER_HEIGHT: f32 = 26.0;

pub(in crate::app) fn table_header_font(ui: &Ui) -> FontId {
    TextStyle::Body.resolve(ui.style())
}

pub(in crate::app) fn table_header_label(ui: &mut Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text).font(table_header_font(ui)).strong());
}

pub(in crate::app) fn table_header_background(rect: egui::Rect) -> egui::Shape {
    egui::Shape::rect_filled(rect, 0.0, editor_bg())
}

pub(in crate::app) fn paint_table_header_rule(ui: &Ui, rect: egui::Rect) {
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(2.0, foundation_block_edge()));
}

pub(in crate::app) fn table_header_resize_divider(
    ui: &mut Ui, header: egui::Rect, x: f32, id: egui::Id,
) -> egui::Response {
    let handle = egui::Rect::from_center_size(
        egui::pos2(x, header.center().y), Vec2::new(8.0, header.height()),
    );
    let response = ui.interact(handle, id, Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let color = if response.dragged() || response.hovered() {
        ui.visuals().selection.stroke.color
    } else { foundation_input_edge() };
    ui.painter().vline(x, header.y_range(), Stroke::new(1.0, color));
    response
}
