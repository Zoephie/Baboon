//! Shared keyboard and middle-click scrolling for scroll panes.
use super::*;

#[derive(Clone, Copy)]
struct ScrollOwner { id: egui::Id, rect: egui::Rect, clicked_pass: u64 }

pub(in crate::app) fn scroll_key(ui: &Ui) -> Option<egui::Key> {
    if ui.ctx().text_edit_focused() || egui::Popup::is_any_open(ui.ctx()) { return None; }
    ui.input_mut(|input| {
        [egui::Key::Home, egui::Key::End, egui::Key::PageUp, egui::Key::PageDown]
            .into_iter().find(|key| input.consume_key(egui::Modifiers::NONE, *key))
    })
}

fn keyboard_scroll(ui: &Ui, id: egui::Id, viewport: egui::Rect, content_height: f32, rect: egui::Rect) {
    let limit = (content_height - viewport.height()).max(0.0);
    if limit < 0.5 { return; }
    let owner_id = egui::Id::new("keyboard_scroll_owner");
    let pass = ui.ctx().cumulative_pass_nr();
    if ui.input(|input| input.pointer.any_pressed()) && ui.rect_contains_pointer(rect) {
        ui.data_mut(|data| {
            let previous = data.get_temp::<ScrollOwner>(owner_id);
            // Inner scroll panes draw first; an enclosing pane must not take
            // ownership of the same click away from a smaller viewport.
            if previous.is_none_or(|owner| owner.clicked_pass != pass || rect.area() < owner.rect.area()) {
                data.insert_temp(owner_id, ScrollOwner { id, rect, clicked_pass: pass });
            }
        });
    }
    if !ui.data(|data| data.get_temp::<ScrollOwner>(owner_id)).is_some_and(|owner| owner.id == id) { return; }
    let Some(key) = scroll_key(ui) else { return; };
    let page = viewport.height() * 0.9;
    let offset = viewport.min.y;
    let next = match key {
        egui::Key::Home => 0.0,
        egui::Key::End => limit,
        egui::Key::PageUp => (offset - page).max(0.0),
        _ => (offset + page).min(limit),
    };
    ui.scroll_with_delta(Vec2::new(0.0, offset - next));
    ui.ctx().request_repaint();
}

/// Keep egui's sizing and virtualization, adding scoped keyboard scrolling
/// and registering each viewport with the shared middle-click controller.
pub(in crate::app) struct KeyboardScrollArea { inner: egui::ScrollArea, axes: egui::Vec2b, salt: egui::IdSalt }

impl KeyboardScrollArea {
    pub fn vertical() -> Self { Self { inner: egui::ScrollArea::vertical(), axes: egui::Vec2b::new(false, true), salt: egui::IdSalt::new("scroll_area") } }
    pub fn horizontal() -> Self { Self { inner: egui::ScrollArea::horizontal(), axes: egui::Vec2b::new(true, false), salt: egui::IdSalt::new("scroll_area") } }
    pub fn both() -> Self { Self { inner: egui::ScrollArea::both(), axes: egui::Vec2b::TRUE, salt: egui::IdSalt::new("scroll_area") } }
    pub fn id_salt(mut self, salt: impl egui::AsIdSalt) -> Self { self.salt = egui::IdSalt::new(salt); self.inner = self.inner.id_salt(self.salt); self }
    pub fn auto_shrink(mut self, value: impl Into<egui::Vec2b>) -> Self { self.inner = self.inner.auto_shrink(value); self }
    pub fn max_height(mut self, value: f32) -> Self { self.inner = self.inner.max_height(value); self }
    pub fn min_scrolled_height(mut self, value: f32) -> Self { self.inner = self.inner.min_scrolled_height(value); self }
    pub fn vertical_scroll_offset(mut self, value: f32) -> Self { self.inner = self.inner.vertical_scroll_offset(value); self }
    pub fn stick_to_bottom(mut self, value: bool) -> Self { self.inner = self.inner.stick_to_bottom(value); self }
    #[cfg(test)]
    pub fn animated(mut self, value: bool) -> Self { self.inner = self.inner.animated(value); self }

    pub fn show<R>(self, ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
        self.show_viewport(ui, |ui, _| contents(ui))
    }

    pub fn show_viewport<R>(self, ui: &mut Ui, contents: impl FnOnce(&mut Ui, egui::Rect) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
        let id = ui.make_persistent_id(self.salt);
        let area_bounds = ui.available_rect_before_wrap();
        let output = self.inner.show_viewport(ui, |ui, viewport| {
            let mut bounds = egui::Rect::from_min_size(ui.max_rect().min + viewport.min.to_vec2(), viewport.size()).intersect(ui.clip_rect());
            bounds.max.x = (bounds.max.x + ui.spacing().scroll.allocated_width()).min(area_bounds.right());
            let result = contents(ui, viewport);
            if self.axes.y { keyboard_scroll(ui, id, viewport, ui.min_size().y, bounds); }
            result
        });
        super::autoscroll::register(ui, &output, self.axes);
        output
    }

    pub fn show_rows<R>(self, ui: &mut Ui, row_height: f32, total: usize,
        contents: impl FnOnce(&mut Ui, std::ops::Range<usize>) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
        let spacing = ui.spacing().item_spacing.y;
        let id = ui.make_persistent_id(self.salt);
        let area_bounds = ui.available_rect_before_wrap();
        let stride = row_height + spacing;
        let output = self.inner.show_rows(ui, row_height, total, |ui, rows| {
            let origin = ui.cursor().top() - rows.start as f32 * stride;
            let clip = ui.clip_rect();
            let viewport = egui::Rect::from_min_size(egui::pos2(0.0, clip.top() - origin), clip.size());
            let result = contents(ui, rows);
            if self.axes.y { keyboard_scroll(ui, id, viewport, (total as f32 * stride - spacing).max(0.0), clip.intersect(area_bounds)); }
            result
        });
        super::autoscroll::register(ui, &output, self.axes);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_scroll_targets_the_clicked_pane_and_keeps_virtualized_rows_working() {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut frame = |events| {
            time += 0.1;
            let mut panes = Vec::new();
            let _ = crate::app::run_ui_test(&ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 200.0))),
                events, time: Some(time), ..Default::default()
            }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.columns(2, |columns| {
                        for (index, ui) in columns.iter_mut().enumerate() {
                            let mut visible = Vec::new();
                            let output = KeyboardScrollArea::vertical().id_salt(index).max_height(150.0)
                                .show_rows(ui, 20.0, 1000, |ui, rows| {
                                    for row in rows { visible.push(row); ui.label(format!("Row {row}")); }
                                });
                            panes.push((output.inner_rect, output.state.offset.y, visible));
                        }
                    });
                });
            });
            panes
        };
        let panes = frame(Vec::new());
        let at = panes[1].0.center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(at), button(true)]);
        frame(vec![button(false)]);
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(vec![key(egui::Key::End)]);
        for _ in 0..10 { frame(Vec::new()); }
        let panes = frame(Vec::new());
        assert_eq!(panes[0].1, 0.0, "the other pane must stay put");
        assert!(panes[1].2.contains(&999), "End reaches the final virtualized row");
        frame(vec![key(egui::Key::Home)]);
        for _ in 0..10 { frame(Vec::new()); }
        let panes = frame(Vec::new());
        assert!(panes[1].2.contains(&0));
        frame(vec![key(egui::Key::PageDown)]);
        for _ in 0..10 { frame(Vec::new()); }
        let panes = frame(Vec::new());
        assert!(panes[1].1 > 50.0 && panes[1].1 < 160.0);
        frame(vec![key(egui::Key::PageUp)]);
        for _ in 0..10 { frame(Vec::new()); }
        assert!(frame(Vec::new())[1].1 < 1.0);
    }

    #[test]
    fn nested_scrolling_and_text_fields_keep_their_own_keys() {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut text = "example text".to_owned();
        let mut frame = |events| {
            time += 0.1;
            let mut inner_offset = 0.0;
            let mut field = egui::Rect::NOTHING;
            let mut inner_rect = egui::Rect::NOTHING;
            let mut outer_offset = 0.0;
            let _ = crate::app::run_ui_test(&ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(500.0, 350.0))),
                events, time: Some(time), ..Default::default()
            }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let outer = KeyboardScrollArea::vertical().id_salt("outer").max_height(300.0).show(ui, |ui| {
                        let inner = KeyboardScrollArea::vertical().id_salt("inner").max_height(120.0).show(ui, |ui| {
                            field = ui.text_edit_singleline(&mut text).rect;
                            for index in 0..100 { ui.label(format!("Inner {index}")); }
                        });
                        inner_offset = inner.state.offset.y;
                        inner_rect = inner.inner_rect;
                        for index in 0..100 { ui.label(format!("Outer {index}")); }
                    });
                    outer_offset = outer.state.offset.y;
                });
            });
            (field, inner_rect, inner_offset, outer_offset)
        };
        let (field, inner, _, _) = frame(Vec::new());
        let click = |at, pressed| egui::Event::PointerButton {
            pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(field.center()), click(field.center(), true)]);
        frame(vec![click(field.center(), false)]);
        frame(vec![egui::Event::Key { key: egui::Key::End, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
        assert_eq!(frame(Vec::new()).2, 0.0, "End edits text instead of scrolling its pane");
        let at = egui::pos2(inner.right() - 20.0, inner.bottom() - 15.0);
        frame(vec![egui::Event::PointerMoved(at), click(at, true)]);
        frame(vec![click(at, false)]);
        frame(vec![egui::Event::Key { key: egui::Key::End, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
        for _ in 0..10 { frame(Vec::new()); }
        let (_, _, inner_offset, outer_offset) = frame(Vec::new());
        assert!(inner_offset > 1000.0);
        assert_eq!(outer_offset, 0.0, "the enclosing scroll pane must not also move");
    }
}
