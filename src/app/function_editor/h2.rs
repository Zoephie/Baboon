//! The Halo 2 function editor: the byte-block `c_function_definition` edited
//! through the engine's own setters ([`H2Function`]), with the engine's option
//! tables. Applying edits to documents and unrelated shader layout belong
//! elsewhere.

use super::*;
use blam_tags::tag_function::h2::{
    COLOR_GRAPH_TYPE_NAMES, FUNCTION_TYPE_NAMES, PERIODIC_FUNCTION_NAMES, TRANSITION_FUNCTION_NAMES,
};

/// A Halo 2 `data` byte-block, read the way the H2 engine reads it.
pub(in crate::app) fn h2_tag_function(bytes: &[u8]) -> Option<TagFunction> {
    TagFunction::parse_encoded(FunctionEncoding::H2, bytes).ok()
}

/// A combo over `names` by index. Returns the picked index when it changed.
fn h2_index_combo(ui: &mut Ui, id: &str, current: usize, names: &[&str], editable: bool, width: f32) -> Option<usize> {
    let label = names.get(current).copied().unwrap_or("unknown");
    if !editable {
        foundation_input_cell(ui, label, width);
        return None;
    }
    let mut picked = None;
    let (_, wheel_delta) = combo_box_with_scroll(
        ui,
        egui::ComboBox::from_id_salt(id).selected_text(label).width(width),
        |ui| {
            for (index, name) in names.iter().enumerate() {
                if ui.selectable_label(index == current, *name).clicked() && index != current {
                    picked = Some(index);
                }
            }
        },
    );
    if let Some(delta) = wheel_delta
        && let Some(next) = combo_scroll_next_index(current.min(names.len() - 1), names.len(), delta)
        && next != current
    {
        picked = Some(next);
    }
    picked
}

fn h2_drag(ui: &mut Ui, label: &str, value: f32, speed: f64, editable: bool) -> Option<f32> {
    let mut v = value;
    ui.label(RichText::new(label).color(text_dark()).small());
    let changed = ui
        .add_enabled(editable, egui::DragValue::new(&mut v).speed(speed).max_decimals(6))
        .changed();
    (changed && v.to_bits() != value.to_bits()).then_some(v)
}

pub(in crate::app) fn draw_h2_function_editor_contents(
    ui: &mut Ui,
    view: &mut FunctionView,
    editable: bool,
    color_popup: Option<&mut Option<MaterialColorPopup>>,
) -> bool {
    let mut changed = false;
    let input_editable = editable && view.edit.as_ref().is_some_and(|paths| !paths.input_name.is_empty());
    let range_editable = editable && view.edit.as_ref().is_some_and(|paths| !paths.range_name.is_empty());
    let time_editable = editable && view.edit.as_ref().is_some_and(|paths| !paths.time_period.is_empty());
    let Some(f) = view.function.as_h2_mut() else {
        return false;
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new("Function type:").color(text_dark()).small());
        let current = f.function_type() as usize;
        if let Some(index) = h2_index_combo(ui, "h2_fn_type", current, &FUNCTION_TYPE_NAMES, editable, 130.0)
            && let Some(kind) = FunctionType::from_byte(index as u8)
        {
            f.set_function_type(kind);
            changed = true;
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Color:").color(text_dark()).small());
        let current = f.color_graph_type() as usize;
        if let Some(index) = h2_index_combo(ui, "h2_fn_color", current, &COLOR_GRAPH_TYPE_NAMES, editable, 130.0) {
            changed |= f.set_color_graph_type(index as u8).is_ok();
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Input:").color(text_dark()).small());
        changed |= seeded_name_combo(ui, "h2_fn_input", &mut view.input_name, input_editable);

        // The RANGE flag is what the engine blends on; the range name is the
        // input feeding it. Both follow the checkbox.
        let mut ranged = f.is_ranged();
        if ui.add_enabled(editable, egui::Checkbox::new(&mut ranged, "")).changed() {
            f.set_ranged(ranged);
            if !ranged {
                view.range_name.clear();
            }
            changed = true;
        }
        ui.label(RichText::new("Range:").color(text_dark()).small());
        if ranged {
            changed |= seeded_name_combo(ui, "h2_fn_range", &mut view.range_name, range_editable);
        } else {
            foundation_input_cell(ui, "", 120.0);
        }
    });
    ui.add_space(6.0);

    if f.color_graph_type() == 0 {
        ui.horizontal(|ui| {
            let (min, max) = (f.clamp_range_min(), f.clamp_range_max());
            let new_min = h2_drag(ui, "Min:", min, 0.01, editable);
            let new_max = h2_drag(ui, "Max:", max, 0.01, editable);
            if new_min.is_some() || new_max.is_some() {
                changed |= f.set_clamp_range(new_min.unwrap_or(min), new_max.unwrap_or(max)).is_ok();
            }
        });
    }
    for graph in 0..(1 + f.is_ranged() as usize) {
        changed |= draw_h2_graph_parameters(ui, f, graph, editable);
    }
    ui.add_space(8.0);

    ui.horizontal_top(|ui| {
        draw_h2_graph_preview(ui, &view.function);
        if view.function.color_count() > 0 {
            ui.add_space(8.0);
            changed |= draw_h2_color_editors(ui, &mut view.function, editable, color_popup);
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("time period").color(text_dark()).small());
        changed |= ui
            .add_enabled(
                time_editable,
                egui::DragValue::new(&mut view.time_period_in_seconds)
                    .speed(0.1)
                    .range(0.0..=f32::MAX),
            )
            .changed();
        ui.label(RichText::new("seconds").color(subtle_dark()).small());
    });
    changed
}

/// The per-type parameters of `graph` (0 = green, 1 = the ranged red graph).
fn draw_h2_graph_parameters(ui: &mut Ui, f: &mut H2Function, graph: usize, editable: bool) -> bool {
    let mut changed = false;
    let tag = if graph == 0 { "" } else { " (range)" };
    ui.horizontal(|ui| {
        ui.push_id(("h2_graph", graph), |ui| match f.function_type() {
            FunctionType::Transition | FunctionType::Periodic => {
                let periodic = f.function_type() == FunctionType::Periodic;
                let names: &[&str] = if periodic { &PERIODIC_FUNCTION_NAMES } else { &TRANSITION_FUNCTION_NAMES };
                ui.label(RichText::new(format!("Function{tag}:")).color(text_dark()).small());
                let current = f.function_index(graph) as usize;
                if let Some(index) = h2_index_combo(ui, "h2_fn_index", current, names, editable, 170.0) {
                    changed |= f.set_function_index(graph, index as u8).is_ok();
                }
                if let Some((frequency, phase)) = f.periodic_frequency_phase(graph) {
                    let new_frequency = h2_drag(ui, "Frequency:", frequency, 0.05, editable);
                    let new_phase = h2_drag(ui, "Phase:", phase, 0.05, editable);
                    if new_frequency.is_some() || new_phase.is_some() {
                        changed |= f
                            .set_periodic_frequency_phase(graph, new_frequency.unwrap_or(frequency), new_phase.unwrap_or(phase))
                            .is_ok();
                    }
                }
                changed |= draw_h2_amplitude(ui, f, graph, editable);
            }
            FunctionType::Exponent => {
                if let Some(exponent) = f.exponent(graph)
                    && let Some(v) = h2_drag(ui, &format!("Exponent{tag}:"), exponent, 0.05, editable)
                {
                    changed |= f.set_exponent(graph, v).is_ok();
                }
                changed |= draw_h2_amplitude(ui, f, graph, editable);
            }
            FunctionType::Linear | FunctionType::LinearKey | FunctionType::Spline | FunctionType::Spline2 => {
                ui.label(RichText::new(format!("Points{tag}:")).color(text_dark()).small());
                for point in 0..f.control_point_count(graph) {
                    let Some((x, y)) = f.control_point(graph, point) else { continue };
                    ui.push_id(point, |ui| {
                        ui.label(RichText::new(format!("x {x:.2}")).color(subtle_dark()).small());
                        if let Some(v) = h2_drag(ui, "y", y, 0.01, editable) {
                            changed |= f.set_control_point_y(graph, point, v).is_ok();
                        }
                    });
                }
            }
            FunctionType::MultiLinearKey | FunctionType::MultiSpline => {
                ui.label(
                    RichText::new("the Halo 2 engine evaluates this type as 0")
                        .color(subtle_dark())
                        .small(),
                );
            }
            FunctionType::Identity | FunctionType::Constant => {}
        });
    });
    changed
}

fn draw_h2_amplitude(ui: &mut Ui, f: &mut H2Function, graph: usize, editable: bool) -> bool {
    let Some((min, max)) = f.amplitude_range(graph) else { return false };
    let new_min = h2_drag(ui, "Amp min:", min, 0.01, editable);
    let new_max = h2_drag(ui, "Amp max:", max, 0.01, editable);
    (new_min.is_some() || new_max.is_some())
        && f.set_amplitude_range(graph, new_min.unwrap_or(min), new_max.unwrap_or(max)).is_ok()
}

/// Swatches for the populated colors, top = last. A dedicated color popup
/// (when given) edits through [`FunctionDraftColorTarget::H2Logical`].
fn draw_h2_color_editors(
    ui: &mut Ui,
    function: &mut TagFunction,
    editable: bool,
    mut color_popup: Option<&mut Option<MaterialColorPopup>>,
) -> bool {
    let mut changed = false;
    let count = function.color_count();
    ui.vertical(|ui| {
        for index in (0..count).rev() {
            let Some(argb) = function.as_h2().and_then(|f| f.color(index)) else { continue };
            let mut color = color32_from_argb(argb);
            ui.horizontal(|ui| {
                let dedicated = color_popup.as_deref_mut();
                let resp = if dedicated.is_none() && editable {
                    ui.color_edit_button_srgba(&mut color)
                } else {
                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::splat(24.0),
                        if editable { Sense::click() } else { Sense::hover() },
                    );
                    ui.painter().rect_filled(rect, 0.0, color);
                    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
                    resp
                };
                ui.label(
                    RichText::new(format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b()))
                        .color(subtle_dark())
                        .small()
                        .monospace(),
                );
                match dedicated {
                    Some(popup) if resp.clicked() => {
                        *popup = Some(
                            MaterialColorPopup::new(
                                &format!("Function color {}", index + 1),
                                color.r() as f32 / 255.0,
                                color.g() as f32 / 255.0,
                                color.b() as f32 / 255.0,
                                1.0,
                            )
                            .with_function_draft_color(FunctionDraftColorTarget::H2Logical(index), 0),
                        );
                    }
                    None if resp.changed() => {
                        // Keep the stored alpha; the swatch edits RGB.
                        let rgb = (color.r() as u32) << 16 | (color.g() as u32) << 8 | color.b() as u32;
                        if let Some(f) = function.as_h2_mut() {
                            changed |= f.set_color(index, (argb & 0xFF00_0000) | rgb).is_ok();
                        }
                    }
                    _ => {}
                }
            });
            if index > 0 {
                ui.add_space(if count <= 2 { 90.0 } else { 18.0 });
            }
        }
    });
    changed
}

/// The normalized curve(s) the engine evaluates: green = graph 0 (range 0),
/// red = graph 1 (range 1) when ranged; over the color gradient for color
/// functions.
fn draw_h2_graph_preview(ui: &mut Ui, function: &TagFunction) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(360.0, 120.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    let plot = rect.shrink(12.0);
    if function.color_count() > 0 {
        draw_function_color_gradient_vertical(&painter, plot, &function_color_stops(function));
    } else {
        painter.rect_filled(plot, 0.0, Color32::from_gray(180));
    }
    painter.rect_stroke(plot, 0.0, Stroke::new(1.0, Color32::from_gray(80)));
    for i in 1..10 {
        let x = egui::lerp(plot.left()..=plot.right(), i as f32 / 10.0);
        painter.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], Stroke::new(1.0, Color32::from_gray(135)));
        let y = egui::lerp(plot.bottom()..=plot.top(), i as f32 / 10.0);
        painter.line_segment([egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)], Stroke::new(1.0, Color32::from_gray(135)));
    }
    let graphs: &[(f32, Color32)] = if function.is_ranged() {
        &[(1.0, Color32::RED), (0.0, Color32::GREEN)]
    } else {
        &[(0.0, Color32::GREEN)]
    };
    for &(range, stroke) in graphs {
        let points = (0..96)
            .map(|i| {
                let t = i as f32 / 95.0;
                let y = function.evaluate_shape(t, range).clamp(0.0, 1.0);
                egui::pos2(egui::lerp(plot.left()..=plot.right(), t), egui::lerp(plot.bottom()..=plot.top(), y))
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(2.0, stroke)));
    }
}

#[cfg(test)]
#[path = "../tests/function_editor_h2.rs"]
mod tests;

impl FunctionView {
    pub(in crate::app) fn from_function(function: TagFunction) -> Self {
        Self {
            function,
            input_name: String::new(),
            range_name: String::new(),
            output_index: None,
            time_period_in_seconds: 0.0,
            edit: None,
        }
    }

    pub(in crate::app) fn from_animated(
        animated: &RenderMethodAnimatedParameter,
        function: TagFunction,
    ) -> Self {
        Self {
            function,
            input_name: animated.input_name.clone(),
            range_name: animated.range_name.clone(),
            output_index: animated.parameter_type.and_then(|kind| {
                OUTPUT_TYPE_OPTIONS
                    .iter()
                    .find(|(_, name)| name.eq_ignore_ascii_case(kind.name()))
                    .map(|(value, _)| *value)
            }),
            time_period_in_seconds: animated.time_period_in_seconds,
            edit: None,
        }
    }

    pub(in crate::app) fn with_edit(mut self, paths: FunctionEditPaths) -> Self {
        self.edit = Some(paths);
        self
    }

    pub(in crate::app) fn data_bytes(&self) -> Vec<u8> {
        self.function.to_bytes()
    }
}

#[derive(Clone)]
/// Cross-frame function editor state bound to one tag and one captured view.
/// The original write targets and last-applied snapshot prevent selection or
/// presentation-setting changes from redirecting an in-progress edit.
pub(in crate::app) struct FunctionPopup {
    /// The tag the function belongs to — edits target this tag's doc.
    pub(super) tag_key: String,
    pub(super) title: String,
    pub(super) view: FunctionView,
    /// Whether the owning tag is writable (LE loose file). Read-only
    /// tags still open the dialog but disable the controls.
    pub(super) editable: bool,
    /// Snapshot of the values last pushed as edits; lets us emit a
    /// `PendingFieldEdit` only when something actually changed.
    pub(super) last_applied: FunctionSnapshot,
    /// Currently selected LinearKey control point (drag/x-y target).
    pub(super) selected_point: usize,
    /// Selected graph slot in the Foundation H3+ editor (green=0, red=1).
    pub(super) selected_graph: usize,
}

impl FunctionPopup {
    pub(in crate::app) fn new(
        tag_key: String,
        title: String,
        view: FunctionView,
        editable: bool,
    ) -> Self {
        let last_applied = FunctionSnapshot::from_view(&view);
        Self {
            tag_key,
            title,
            view,
            editable,
            last_applied,
            selected_point: 0,
            selected_graph: 0,
        }
    }

    pub(in crate::app) fn apply_draft_color(
        &mut self,
        target: FunctionDraftColorTarget,
        argb: u32,
    ) {
        // The target names which editor opened the picker; both encodings take
        // the logical color through the engine's setter.
        let (FunctionDraftColorTarget::H3Logical(index) | FunctionDraftColorTarget::H2Logical(index)) = target;
        if self.view.function.as_h2().is_some() {
            // The H2 swatches edit RGB; keep the stored alpha.
            let keep = self.view.function.as_h2().and_then(|f| f.color(index)).unwrap_or(0xFF00_0000);
            let argb = (keep & 0xFF00_0000) | (argb & 0x00FF_FFFF);
            if let Some(f) = self.view.function.as_h2_mut() {
                let _ = f.set_color(index, argb);
            }
            return;
        }
        let mut editor = TagFunctionEditor::from_function(self.view.function.clone());
        if editor.set_color(index, argb).is_ok() {
            self.view.function = editor.into_function();
        }
    }
}

/// Values that map to writable tag fields. Compared frame-to-frame to
/// decide which `PendingFieldEdit`s to emit.
/// Raw function bytes are compared in full so changes never silently discard
/// unrecognized classic H2 data.
#[derive(Clone, PartialEq)]
pub(in crate::app) struct FunctionSnapshot {
    pub(super) data: Vec<u8>,
    pub(super) output_index: Option<i32>,
    pub(super) input_name: String,
    pub(super) range_name: String,
    pub(super) time_period: f32,
}

impl FunctionSnapshot {
    pub(in crate::app) fn from_view(view: &FunctionView) -> Self {
        Self {
            data: view.data_bytes(),
            output_index: view.output_index,
            input_name: view.input_name.clone(),
            range_name: view.range_name.clone(),
            time_period: view.time_period_in_seconds,
        }
    }
}

/// Edits produced by the function dialog this frame, plus the tag they
/// belong to.
pub(in crate::app) struct FunctionEditBatch {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) edits: Vec<PendingFieldEdit>,
    pub(in crate::app) data_ops: Vec<FunctionDataOp>,
}
