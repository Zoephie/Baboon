//! The panel's function controls and event effects.

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

/// A button for each effect an event would play. An effect starts when its
/// function comes on, so a press turns it off for a frame and then on.
pub(in crate::app::model_preview) fn draw_event_effects(
    ui: &mut Ui,
    events: &[&str],
    key: u64,
    set: &mut FunctionInputs,
) {
    for name in events {
        if ui.small_button(format!("Play {name}")).clicked() {
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
    for &name in names {
        let at_rest = functions.idle(name);
        let mut value = set.get(&(key, name.to_owned())).copied().unwrap_or(at_rest);
        let slider = egui::Slider::new(&mut value, 0.0..=1.0).text(name);
        if ui
            .add_enabled(drawn(name), slider)
            .on_disabled_hover_text(UNREAD)
            .changed()
        {
            set.insert((key, name.to_owned()), value);
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for function in &functions.functions {
        let export = function.export.as_str();
        // One the type exports never answers for its name; the slider above
        // does.
        if export.is_empty() || seen.contains(&export) || functions.type_exports(export) {
            continue;
        }
        seen.push(export);
        let computed = values.get(export).copied().unwrap_or(0.0);
        let held = (key, export.to_owned());
        ui.add_enabled_ui(drawn(export), |ui| {
            ui.horizontal(|ui| {
                let mut scripted = set.contains_key(&held);
                let toggle = ui
                    .add(ThemedCheckbox::without_text(&mut scripted))
                    .on_hover_text(SCRIPTED);
                if toggle.changed() {
                    if scripted {
                        set.insert(held.clone(), computed);
                    } else {
                        set.remove(&held);
                    }
                }
                match set.get_mut(&held) {
                    Some(value) => {
                        ui.add(egui::Slider::new(value, 0.0..=1.0).text(export));
                    }
                    None => {
                        ui.label(format!("{export}: {computed:.2}"));
                    }
                }
            });
        })
        .response
        .on_disabled_hover_text(UNREAD);
    }
}
