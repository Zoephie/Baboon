//! SVG checkbox artwork over egui's native layout, input and accessibility.
use super::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum State {
    Unchecked,
    Checked,
    Indeterminate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Palette {
    state: State,
    dark: bool,
    hovered: bool,
    foreground: Color32,
    accent: Color32,
}

fn svg(palette: Palette) -> String {
    let source = match palette.state {
        State::Unchecked => include_root_str!("assets/Button Icons/Checkbox-Unchecked.svg"),
        State::Checked => include_root_str!("assets/Button Icons/Checkbox-Checked.svg"),
        State::Indeterminate => include_root_str!("assets/Button Icons/Checkbox-Indeterminate.svg"),
    };
    let colour = |c: Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
    let mut source = source
        .replace("currentColor", &colour(palette.foreground))
        .replace("#47AEEB", &colour(palette.accent));
    if !palette.dark {
        source = source.replace("fill=\"black\"", "fill=\"white\"");
    }
    if palette.hovered {
        source = source
            .replace("opacity=\"0.2\"", "opacity=\"0.5\"")
            .replace("opacity=\"0.4\"", "opacity=\"0.7\"")
            .replace("opacity=\"0.8\"", "opacity=\"1\"");
    } else if palette.state == State::Checked {
        // Only the outer border softens; the checkmark stays fully legible.
        source = source.replacen("<path ", "<path opacity=\"0.5\" ", 1);
    }
    source
}

fn image(palette: Palette) -> egui::Image<'static> {
    thread_local! {
        static SVGS: RefCell<HashMap<Palette, (String, Arc<[u8]>)>> = RefCell::new(HashMap::new());
    }
    let (uri, bytes) = SVGS.with(|cache| {
        cache
            .borrow_mut()
            .entry(palette)
            .or_insert_with(|| {
                (
                    format!(
                        "bytes://baboon_checkboxes/{:?}-{}-{}-{:08x}-{:08x}.svg",
                        palette.state,
                        palette.dark,
                        palette.hovered,
                        u32::from_le_bytes(palette.foreground.to_array()),
                        u32::from_le_bytes(palette.accent.to_array())
                    ),
                    svg(palette).into_bytes().into(),
                )
            })
            .clone()
    });
    egui::Image::from_bytes(uri, egui::load::Bytes::Shared(bytes))
        .fit_to_exact_size(Vec2::splat(16.0))
}

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
        let original = ui.style().clone();
        // Leave the native glyph's space in the layout and suppress its paint.
        // Preserve text colour explicitly while hiding only the glyph strokes.
        {
            let style = ui.style_mut();
            style.spacing.icon_width = 16.0;
            style.visuals.override_text_color = Some(original.visuals.text_color());
            for widget in [
                &mut style.visuals.widgets.noninteractive,
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
                &mut style.visuals.widgets.open,
            ] {
                widget.bg_fill = Color32::TRANSPARENT;
                widget.bg_stroke = Stroke::NONE;
                widget.fg_stroke = Stroke::NONE;
            }
        }
        let response = ui.add(
            egui::Checkbox::new(&mut *self.checked, self.atoms).indeterminate(self.indeterminate),
        );
        ui.set_style(original);
        if ui.is_rect_visible(response.rect) {
            let state = if self.indeterminate {
                State::Indeterminate
            } else if *self.checked {
                State::Checked
            } else {
                State::Unchecked
            };
            let hovered = response.hovered() || response.has_focus();
            let rect = egui::Rect::from_center_size(
                egui::pos2(
                    self.icon_center_x.unwrap_or(response.rect.left() + 8.0),
                    response.rect.center().y,
                ),
                Vec2::splat(16.0),
            );
            paint_themed_checkbox(ui, rect, state, hovered);
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

/// Shared artwork for native-layout widgets and custom allocated checkbox rows.
fn paint_themed_checkbox(ui: &Ui, rect: egui::Rect, state: State, hovered: bool) {
    image(Palette {
        state,
        dark: ui.visuals().dark_mode,
        hovered,
        foreground: ui.ctx().global_style().visuals.text_color(),
        accent: ui.ctx().global_style().visuals.selection.bg_fill,
    })
    .paint_at(ui, rect);
}

/// Custom rows supply the glyph rect; their parent UI owns disabled opacity.
pub(in crate::app) fn paint_checkbox_at(ui: &Ui, rect: egui::Rect, checked: bool, hovered: bool) {
    paint_themed_checkbox(
        ui,
        rect,
        if checked {
            State::Checked
        } else {
            State::Unchecked
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

    fn palette(state: State, dark: bool, hovered: bool) -> Palette {
        Palette {
            state,
            dark,
            hovered,
            foreground: if dark { Color32::WHITE } else { Color32::BLACK },
            accent: Color32::from_rgb(64, 108, 134),
        }
    }
    fn raster(palette: Palette) -> egui::ColorImage {
        egui_extras::image::load_svg_bytes(svg(palette).as_bytes(), &Default::default()).unwrap()
    }

    #[test]
    fn checkbox_art_rasterizes_all_states_and_hover_strengthens_the_border() {
        for dark in [false, true] {
            for state in [State::Unchecked, State::Checked, State::Indeterminate] {
                let normal = raster(palette(state, dark, false));
                let hover = raster(palette(state, dark, true));
                assert_eq!(normal.size, [16, 16]);
                assert!(normal.pixels.iter().any(|pixel| pixel.a() > 0));
                assert!(
                    hover[(0, 8)].a() > normal[(0, 8)].a(),
                    "hover border: {state:?}, dark={dark}"
                );
            }
        }
        let light = raster(palette(State::Checked, false, false));
        let dark = raster(palette(State::Checked, true, false));
        assert!(
            light[(1, 8)].r() > dark[(1, 8)].r(),
            "the translucent well reverses for light mode"
        );
    }

    #[test]
    fn checkbox_images_load_through_the_apps_svg_loader() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        for dark in [false, true] {
            for hovered in [false, true] {
                for state in [State::Unchecked, State::Checked, State::Indeterminate] {
                    let image = image(palette(state, dark, hovered));
                    let loaded = image.load_for_size(&ctx, Vec2::splat(16.0));
                    assert!(
                        matches!(loaded, Ok(egui::load::TexturePoll::Ready { .. })),
                        "checkbox image must load in the app: {:?}, URI={:?}",
                        loaded.as_ref().err(),
                        image.uri()
                    );
                }
            }
        }
    }

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
