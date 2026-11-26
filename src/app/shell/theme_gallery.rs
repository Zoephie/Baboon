//! The Theme Gallery: one of every kind of widget Baboon draws, the text
//! colours, and a swatch of every token by group, all in the theme drawn now.
//! The Theme Editor shows it beside its rows, so an edit can be judged on
//! every widget as it is made.

use super::*;
use crate::app::ui_kit::themes::{self, Token};

/// The gallery, and the state of its sample widgets.
pub(in crate::app) struct ThemeGallery {
    checked: bool,
    choice: usize,
    amount: f32,
    text: String,
    selected: usize,
}

impl Default for ThemeGallery {
    fn default() -> Self {
        Self {
            checked: true,
            choice: 0,
            amount: 0.4,
            text: "sample text".to_owned(),
            selected: 1,
        }
    }
}

impl ThemeGallery {
    /// Draws the gallery, scrolling, in all of `ui`.
    pub(in crate::app) fn draw(&mut self, ui: &mut Ui) {
        egui::ScrollArea::both()
            .id_salt("theme_gallery_body")
            .auto_shrink([false, false])
            .show(ui, |ui| self.draw_body(ui));
    }

    fn draw_body(&mut self, ui: &mut Ui) {
        let colors = themes::installed();
        heading(ui, "Widgets");
        ui.horizontal_wrapped(|ui| {
            let _ = ui.button("Button");
            ui.add_enabled(false, egui::Button::new("Disabled"));
            for (index, label) in ["One", "Two", "Three"].into_iter().enumerate() {
                if ui.selectable_label(self.selected == index, label).clicked() {
                    self.selected = index;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.baboon_checkbox(&mut self.checked, "Checkbox");
            ui.radio_value(&mut self.choice, 0, "Radio");
            ui.radio_value(&mut self.choice, 1, "Other");
            ui.add(egui::Slider::new(&mut self.amount, 0.0..=1.0).text("Slider"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.text).desired_width(160.0));
            let mut empty = String::new();
            ui.add(
                egui::TextEdit::singleline(&mut empty)
                    .hint_text("hint text")
                    .desired_width(120.0),
            );
            crate::app::ui_kit::ValueComboBox::from_id_salt("theme_gallery_combo")
                .width(120.0)
                .selected_text(["One", "Two", "Three"][self.selected])
                .show_ui(ui, |ui| {
                    for (index, label) in ["One", "Two", "Three"].into_iter().enumerate() {
                        ui.selectable_value(&mut self.selected, index, label);
                    }
                });
        });
        ui.add(egui::ProgressBar::new(self.amount).show_percentage());
        egui::CollapsingHeader::new("Collapsing header")
            .id_salt("theme_gallery_header")
            .default_open(true)
            .show(ui, |ui| {
                ui.label("Inside a collapsing header.");
            });
        ui.separator();

        heading(ui, "Text");
        for token in [
            Token::Text,
            Token::TextStrong,
            Token::TextMuted,
            Token::AccentText,
            Token::LinkText,
            Token::FieldHelpText,
            Token::StatusError,
            Token::StatusWarning,
            Token::StatusSuccess,
        ] {
            ui.label(
                RichText::new(format!("{}  {}", token.id(), token.description()))
                    .color(colors.get(token)),
            );
        }

        let mut group = "";
        for token in Token::ALL {
            if token.group() != group {
                group = token.group();
                heading(ui, group);
            }
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(56.0, 20.0), egui::Sense::hover());
                // Over the page, as most tokens are drawn, with sample text
                // in the body colour.
                ui.painter()
                    .rect_filled(rect, 3.0, colors.get(Token::Background));
                ui.painter().rect_filled(rect, 3.0, colors.get(*token));
                ui.painter().rect_stroke(
                    rect,
                    3.0,
                    ui.visuals().widgets.inactive.fg_stroke,
                    egui::StrokeKind::Inside,
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Aa",
                    egui::FontId::proportional(12.0),
                    colors.get(Token::Text),
                );
                ui.label(RichText::new(token.id()).monospace());
                ui.label(RichText::new(token.description()).color(subtle_dark()));
            });
        }
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).strong());
    ui.add_space(2.0);
}
