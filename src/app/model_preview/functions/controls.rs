//! The panel's function controls and event effects.

use super::super::super::ui_kit::button_icons::ButtonIcon;
use super::super::panel::{CardRows, name_cell, name_column_width, play_row, value_slider};
use super::ThemedCheckbox;
use super::read::{FunctionInputs, FunctionValues, ObjectFunctions};
use eframe::egui;
use eframe::egui::Ui;

/// An event effect's function the frame it is pressed: off, so it is seen
/// coming on the next.
const STARTING: f32 = -1.0;

/// What an object's functions take from outside, split into the event
/// effects it can play and the inputs a slider sets.
pub(in crate::app::model_preview) fn function_names<'a>(
    functions: &'a ObjectFunctions,
    shader_inputs: &'a [String],
) -> (Vec<&'a str>, Vec<&'a str>) {
    let mut names = functions.imports();
    for name in shader_inputs {
        if !functions.makes(name) && !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .partition(|name| super::super::effects::is_event_function(name))
}

/// Turns on each event effect pressed last frame. Run every frame, whether
/// or not its buttons are shown, or a press would hold it off.
pub(in crate::app::model_preview) fn settle_event_effects(
    events: &[&str],
    key: u64,
    set: &mut FunctionInputs,
) {
    for name in events {
        if let Some(value) = set.get_mut(&(key, (*name).to_owned()))
            && *value == STARTING
        {
            *value = 1.0;
        }
    }
}

/// A row for each effect an event would play, with a Play button. An effect
/// starts when its function comes on, so a press turns it off for a frame
/// and then on.
pub(in crate::app::model_preview) fn draw_event_effects(
    ui: &mut Ui,
    rows: &mut CardRows,
    events: &[&str],
    key: u64,
    set: &mut FunctionInputs,
) {
    for name in events {
        if play_row(ui, rows, ButtonIcon::Play, name, "Play this effect") {
            set.insert((key, (*name).to_owned()), STARTING);
            // It has to be seen off for a frame before it can come on.
            ui.ctx().request_repaint();
        }
    }
}

/// Sliders for what an object imports, `names` (the inputs of
/// [`function_names`]), and what its functions come to.
pub(in crate::app::model_preview) fn draw_function_controls(
    ui: &mut Ui,
    functions: &ObjectFunctions,
    names: &[&str],
    key: u64,
    set: &mut FunctionInputs,
    values: &FunctionValues,
    shader_inputs: &[String],
) {
    const SCRIPTED: &str = "Set it as a script would, with object_set_function_variable: \
                            the game reads that before what the function makes.";
    const UNREAD: &str = "Nothing this preview draws reads this. It may drive something \
                          the preview does not have, such as an effect's particles.";
    let reached = functions.reaching(shader_inputs);
    let drawn = |name: &str| reached.iter().any(|known| known == name);
    // What the functions make, each once, but those the type exports: their
    // names answer to the sliders for the inputs.
    let mut exports: Vec<&str> = Vec::new();
    for function in &functions.functions {
        let export = function.export.as_str();
        if !export.is_empty() && !exports.contains(&export) && !functions.type_exports(export) {
            exports.push(export);
        }
    }
    let mut rows = CardRows::default();
    let row_width = ui.available_width().max(1.0);
    // Names are long here: up to the column's widest, leaving the slider
    // some room rather than a share of the row.
    let name_width = name_column_width(ui, names.iter().chain(&exports).copied(), f32::MAX)
        .min((row_width - LEAD_WIDTH - 130.0).max(40.0));
    for &name in names {
        let at_rest = functions.idle(name);
        let mut value = set.get(&(key, name.to_owned())).copied().unwrap_or(at_rest);
        let row = rows.row(ui, |ui| {
            ui.add_enabled_ui(drawn(name), |ui| {
                lead_cell(ui, |_| ());
                name_cell(ui, name, name_width);
                value_slider(ui, &mut value, true).changed()
            })
        });
        row.response.on_disabled_hover_text(UNREAD);
        if row.inner {
            set.insert((key, name.to_owned()), value);
        }
    }
    for export in exports {
        let computed = values.get(export).copied().unwrap_or(0.0);
        let held = (key, export.to_owned());
        rows.row(ui, |ui| {
            ui.add_enabled_ui(drawn(export), |ui| {
                let mut scripted = set.contains_key(&held);
                // Set as a script would, its slider moves; otherwise it shows
                // what the function makes.
                let toggle = lead_cell(ui, |ui| {
                    ui.add(ThemedCheckbox::without_text(&mut scripted))
                        .on_hover_text(SCRIPTED)
                });
                if toggle.changed() {
                    if scripted {
                        set.insert(held.clone(), computed);
                    } else {
                        set.remove(&held);
                    }
                }
                name_cell(ui, export, name_width);
                match set.get_mut(&held) {
                    Some(value) => {
                        value_slider(ui, value, true);
                    }
                    None => {
                        let mut shown = computed;
                        value_slider(ui, &mut shown, false);
                    }
                }
            })
            .response
            .on_disabled_hover_text(UNREAD);
        });
    }
}

/// The room before a function's name: its Scripted checkbox, or space for
/// one, so every name starts in one column.
const LEAD_WIDTH: f32 = 24.0;

/// A row's lead, [`LEAD_WIDTH`] wide whatever `add` puts in it.
fn lead_cell<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let size = egui::vec2(LEAD_WIDTH, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(size);
        add(ui)
    })
    .inner
}
