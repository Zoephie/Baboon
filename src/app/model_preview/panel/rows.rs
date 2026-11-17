//! The setup cards' rows: each a button's height and some, a hairline
//! between them, a name column at their left and controls filling the rest,
//! so every card's rows line up.

use super::super::super::ui_kit::button_icons::{ButtonIcon, icon_button};
use super::super::super::ui_kit::railed_slider;
use super::super::super::ui_kit::style::{BUTTON_HEIGHT, grid_line, text_dark};
use eframe::egui;
use eframe::egui::{FontId, RichText, Ui, Vec2};

/// A row's height: a button and 4 points above and below it.
pub(in crate::app::model_preview) const ROW_HEIGHT: f32 = BUTTON_HEIGHT + 8.0;

/// The width of a function's value box.
const VALUE_WIDTH: f32 = 56.0;

/// The rows' text.
pub(in crate::app::model_preview) fn row_font() -> FontId {
    FontId::proportional(13.0)
}

/// A card's rows, each under a hairline but the first.
#[derive(Default)]
pub(in crate::app::model_preview) struct CardRows {
    drawn: bool,
}

impl CardRows {
    /// A row of `add`, laid out left to right and centred on the row.
    pub(in crate::app::model_preview) fn row<R>(
        &mut self,
        ui: &mut Ui,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        if self.drawn {
            // Rows sit flush, the hairline between them.
            ui.add_space(-ui.spacing().item_spacing.y);
            let y = ui.cursor().top();
            let x = ui.max_rect().x_range();
            ui.painter().hline(x, y, egui::Stroke::new(1.0, grid_line()));
        }
        self.drawn = true;
        let width = ui.available_width().max(1.0);
        ui.allocate_ui_with_layout(
            Vec2::new(width, ROW_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_size(Vec2::new(width, ROW_HEIGHT));
                ui.spacing_mut().item_spacing.x = 6.0;
                add(ui)
            },
        )
        .inner
    }
}

/// The name column's width for `names`: the widest, but at least 60 points,
/// and never more than 180 or two fifths of `row_width`.
pub(in crate::app::model_preview) fn name_column_width<'a>(
    ui: &Ui,
    names: impl IntoIterator<Item = &'a str>,
    row_width: f32,
) -> f32 {
    names
        .into_iter()
        .map(|name| {
            ui.painter()
                .layout_no_wrap(name.to_owned(), row_font(), text_dark())
                .size()
                .x
        })
        .fold(60.0_f32, f32::max)
        .min(180.0)
        .min(row_width * 0.4)
}

/// `name` in a column `width` wide, left-aligned and cut short with its
/// whole self on hover.
pub(in crate::app::model_preview) fn name_cell(ui: &mut Ui, name: &str, width: f32) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, BUTTON_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(Vec2::new(width, BUTTON_HEIGHT));
            ui.add(
                egui::Label::new(RichText::new(name).color(text_dark()).font(row_font()))
                    .truncate(),
            )
            .on_hover_text(name);
        },
    );
}

/// A row with a Play button and `name`: whether it was pressed.
pub(in crate::app::model_preview) fn play_row(
    ui: &mut Ui,
    rows: &mut CardRows,
    icon: ButtonIcon,
    name: &str,
    tooltip: &str,
) -> bool {
    rows.row(ui, |ui| {
        let pressed = icon_button(ui, icon, tooltip, true, text_dark()).clicked();
        let width = ui.available_width().max(1.0);
        name_cell(ui, name, width);
        pressed
    })
}

/// A 0 to 1 slider filling what is left of the row, its value in a box at
/// the right to three places.
pub(in crate::app::model_preview) fn value_slider(
    ui: &mut Ui,
    value: &mut f32,
    enabled: bool,
) -> egui::Response {
    ui.scope(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        ui.spacing_mut().interact_size.x = VALUE_WIDTH;
        ui.spacing_mut().slider_width = (ui.available_width() - VALUE_WIDTH - spacing).max(40.0);
        let slider = egui::Slider::new(value, 0.0..=1.0).fixed_decimals(3);
        railed_slider(ui, slider, enabled, |_| Vec::new())
    })
    .inner
}
