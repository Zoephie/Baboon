//! SVG checkbox artwork over native layout, input and accessibility.
use super::*;

/// Retains egui's response, keyboard activation, focus, labels and mixed-state semantics.
pub(in crate::app) struct ThemedCheckbox<'a> {
    checked: &'a mut bool,
    atoms: egui::Atoms<'a>,
    indeterminate: bool,
    icon_center_x: Option<f32>,
}
impl<'a> ThemedCheckbox<'a> {
    pub fn new(checked: &'a mut bool, label: impl egui::IntoAtoms<'a>) -> Self {
        Self {
            checked,
            atoms: label.into_atoms(),
            indeterminate: false,
            icon_center_x: None,
        }
    }
    pub fn without_text(checked: &'a mut bool) -> Self {
        Self::new(checked, ())
    }
    /// Align the artwork to a shared table column without changing its hit area.
    pub fn icon_center_x(mut self, x: f32) -> Self {
        self.icon_center_x = Some(x);
        self
    }
    pub fn indeterminate(mut self, mixed: bool) -> Self {
        self.indeterminate = mixed;
        self
    }
}
impl ThemedCheckbox<'_> {
    fn show(self, ui: &mut Ui) -> egui::Response {
        let response = native_selection_response(ui, |ui| {
            ui.add(
                egui::Checkbox::new(&mut *self.checked, self.atoms)
                    .indeterminate(self.indeterminate),
            )
        });
        if ui.is_rect_visible(response.rect) {
            let state = if self.indeterminate {
                SelectionState::Indeterminate
            } else if *self.checked {
                SelectionState::Checked
            } else {
                SelectionState::Unchecked
            };
            let hovered = response.hovered() || response.has_focus();
            let rect = egui::Rect::from_center_size(
                egui::pos2(
                    self.icon_center_x.unwrap_or(response.rect.left() + 8.0),
                    response.rect.center().y,
                ),
                Vec2::splat(16.0),
            );
            paint_selection_control(ui, rect, state, hovered);
        }
        response
    }
}

impl egui::Widget for ThemedCheckbox<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        // Icon-only checkboxes belong to a composed row or a table; that
        // owner supplies the row's padding and preserves its column placement.
        if egui::containers::menu::is_in_menu(ui)
            && self.atoms.text().is_some_and(|text| !text.is_empty())
        {
            Frame::NONE
                .inner_margin(menu_control_margin(ui))
                .show(ui, |ui| self.show(ui))
                .inner
        } else {
            self.show(ui)
        }
    }
}

/// Custom rows supply the glyph rect; their parent UI owns disabled opacity.
pub(in crate::app) fn paint_checkbox_at(ui: &Ui, rect: egui::Rect, checked: bool, hovered: bool) {
    paint_selection_control(
        ui,
        rect,
        if checked {
            SelectionState::Checked
        } else {
            SelectionState::Unchecked
        },
        hovered,
    );
}

pub(in crate::app) trait CheckboxUi {
    fn baboon_checkbox<'a>(
        &mut self,
        checked: &'a mut bool,
        label: impl egui::IntoAtoms<'a>,
    ) -> egui::Response;
}
impl CheckboxUi for Ui {
    fn baboon_checkbox<'a>(
        &mut self,
        checked: &'a mut bool,
        label: impl egui::IntoAtoms<'a>,
    ) -> egui::Response {
        self.add(ThemedCheckbox::new(checked, label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_checkbox_retains_native_layout_click_keyboard_and_disabled_behaviour() {
        fn frame(
            ctx: &egui::Context,
            value: &mut bool,
            custom: bool,
            enabled: bool,
            time: f64,
            events: Vec<egui::Event>,
        ) -> egui::Response {
            let mut response = None;
            let _ = crate::app::run_ui_test(
                ctx,
                crate::app::loose_fixture::screen(events, time),
                |ui| {
                    ui.spacing_mut().icon_width = 16.0;
                    let original = ui.style().clone();
                    response = Some(if custom {
                        ui.add_enabled(enabled, ThemedCheckbox::new(value, "Selection"))
                    } else {
                        ui.add_enabled(enabled, egui::Checkbox::new(value, "Selection"))
                    });
                    assert_eq!(
                        ui.visuals().widgets.inactive.bg_fill,
                        original.visuals.widgets.inactive.bg_fill,
                        "checkbox styling must not leak into neighbouring controls"
                    );
                },
            );
            response.unwrap()
        }
        for enabled in [false, true] {
            let mut results = Vec::new();
            for custom in [false, true] {
                let ctx = egui::Context::default();
                Baboon::configure_context(&ctx);
                let mut value = false;
                let response = frame(&ctx, &mut value, custom, enabled, 0.0, vec![]);
                let rect = response.rect;
                let pointer = |pressed| egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                frame(
                    &ctx,
                    &mut value,
                    custom,
                    enabled,
                    0.1,
                    vec![egui::Event::PointerMoved(rect.center()), pointer(true)],
                );
                let clicked = frame(&ctx, &mut value, custom, enabled, 0.2, vec![pointer(false)]);
                let after_click = value;
                ctx.memory_mut(|memory| memory.request_focus(response.id));
                let mut keyboard_changed = false;
                for (step, pressed) in [true, false].into_iter().enumerate() {
                    keyboard_changed |= frame(
                        &ctx,
                        &mut value,
                        custom,
                        enabled,
                        0.3 + step as f64 * 0.1,
                        vec![egui::Event::Key {
                            key: egui::Key::Space,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: Default::default(),
                        }],
                    )
                    .changed();
                }
                results.push((
                    rect.size(),
                    clicked.changed(),
                    after_click,
                    keyboard_changed,
                    value,
                ));
            }
            assert_eq!(
                results[0], results[1],
                "custom glyph must preserve native interaction, enabled={enabled}"
            );
            assert_eq!(
                results[1].2, enabled,
                "only enabled checkboxes toggle on click"
            );
            assert_eq!(
                results[1].3, enabled,
                "only enabled checkboxes toggle with Space"
            );
        }
    }
}
