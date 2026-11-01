//! SVG radio artwork over native input, selection and accessibility.
use super::*;

pub(in crate::app) trait RadioUi {
    fn baboon_radio_value<'a, Value: PartialEq>(
        &mut self,
        current: &mut Value,
        alternative: Value,
        label: impl egui::IntoAtoms<'a>,
    ) -> egui::Response;
}

impl RadioUi for Ui {
    fn baboon_radio_value<'a, Value: PartialEq>(
        &mut self,
        current: &mut Value,
        alternative: Value,
        label: impl egui::IntoAtoms<'a>,
    ) -> egui::Response {
        if egui::containers::menu::is_in_menu(self) {
            Frame::NONE
                .inner_margin(menu_control_margin(self))
                .show(self, |ui| show_radio_value(ui, current, alternative, label))
                .inner
        } else {
            show_radio_value(self, current, alternative, label)
        }
    }
}

fn show_radio_value<'a, Value: PartialEq>(
    ui: &mut Ui,
    current: &mut Value,
    alternative: Value,
    label: impl egui::IntoAtoms<'a>,
) -> egui::Response {
    let selected = *current == alternative;
    let mut response =
        native_selection_response(ui, |ui| ui.add(egui::RadioButton::new(selected, label)));
    let selected_after = selected || response.clicked();
    if response.clicked() && !selected {
        *current = alternative;
        response.mark_changed();
    }
    if ui.is_rect_visible(response.rect) {
        let rect = egui::Rect::from_center_size(
            egui::pos2(response.rect.left() + 8.0, response.rect.center().y),
            Vec2::splat(16.0),
        );
        paint_selection_control(
            ui,
            rect,
            if selected_after {
                SelectionState::RadioChecked
            } else {
                SelectionState::RadioUnchecked
            },
            response.hovered() || response.has_focus(),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_radio_retains_native_click_keyboard_disabled_and_selected_behavior() {
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
                    response = Some(
                        ui.add_enabled_ui(enabled, |ui| {
                            if custom {
                                ui.baboon_radio_value(value, true, "Choice")
                            } else {
                                ui.radio_value(value, true, "Choice")
                            }
                        })
                        .inner,
                    );
                    assert_eq!(
                        ui.visuals().widgets.inactive.fg_stroke,
                        original.visuals.widgets.inactive.fg_stroke
                    );
                    assert_eq!(
                        ui.visuals().override_text_color,
                        original.visuals.override_text_color
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
                let initial = frame(&ctx, &mut value, custom, enabled, 0.0, vec![]);
                let at = initial.rect.center();
                let pointer = |pressed| egui::Event::PointerButton {
                    pos: at,
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
                    vec![egui::Event::PointerMoved(at), pointer(true)],
                );
                let click = frame(&ctx, &mut value, custom, enabled, 0.2, vec![pointer(false)]);
                let after_click = value;
                frame(&ctx, &mut value, custom, enabled, 0.3, vec![pointer(true)]);
                let repeat = frame(&ctx, &mut value, custom, enabled, 0.4, vec![pointer(false)]);
                let after_repeat = value;
                value = false;
                ctx.memory_mut(|memory| memory.request_focus(initial.id));
                let mut key_changed = false;
                for (step, pressed) in [true, false].into_iter().enumerate() {
                    key_changed |= frame(
                        &ctx,
                        &mut value,
                        custom,
                        enabled,
                        0.5 + step as f64 * 0.1,
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
                    initial.rect.size(),
                    click.changed(),
                    after_click,
                    repeat.changed(),
                    after_repeat,
                    key_changed,
                    value,
                ));
            }
            assert_eq!(results[0], results[1], "native parity, enabled={enabled}");
            assert_eq!(
                results[1].2, enabled,
                "only an enabled radio selects on click"
            );
            assert!(
                !results[1].3,
                "clicking a selected radio does not change it"
            );
            assert_eq!(results[1].4, enabled, "selected radios stay selected");
            assert_eq!(
                results[1].5, enabled,
                "only an enabled radio selects with Space"
            );
        }
    }
}
