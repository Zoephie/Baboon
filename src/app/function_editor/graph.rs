//! Function sampling, gradients, and graph painting.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

/// The function editor's graph. It renders the primary and ranged graphs
/// independently and drags their control points: the H3+ multi-part curve
/// (which can also add, remove and retype) and Halo 2's fixed points (moved
/// within the engine's x rules).
pub(super) fn draw_foundation_graph(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
) -> bool {
    let size = Vec2::new(465.0, 225.0);
    let sense = if editable && editor.master_type() == EngineMasterType::Curve {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let plot = rect.shrink2(Vec2::new(30.0, 20.0));
    let to_screen = |(x, y): (f32, f32)| {
        egui::pos2(
            egui::lerp(plot.left()..=plot.right(), x.clamp(0.0, 1.0)),
            egui::lerp(plot.bottom()..=plot.top(), y.clamp(0.0, 1.0)),
        )
    };
    let to_graph = |pos: egui::Pos2| {
        (
            egui::remap_clamp(pos.x, plot.left()..=plot.right(), 0.0..=1.0),
            egui::remap_clamp(pos.y, plot.bottom()..=plot.top(), 0.0..=1.0),
        )
    };

    let curve_points = |editor: &TagFunctionEditor| {
        let mut points = Vec::new();
        if editor.master_type() == EngineMasterType::Curve {
            for graph in 0..editor.graph_count() {
                for point in 0..editor.curve_control_point_count(graph).unwrap_or(0) {
                    if let Some(value) = editor.curve_control_point(graph, point) {
                        points.push((graph, point, value));
                    }
                }
            }
        }
        points
    };

    let mut changed = false;
    if editable && editor.master_type() == EngineMasterType::Curve {
        let hit_points = curve_points(editor);
        let nearest = |pos: egui::Pos2| {
            hit_points
                .iter()
                .map(|&(graph, point, value)| (graph, point, to_screen(value).distance(pos)))
                .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        };
        if let Some(pos) = response.interact_pointer_pos() {
            if response.drag_started() || response.clicked() {
                // Pick where the button went down: egui reports the drag only
                // once the pointer has moved past its threshold, by which point
                // a quick drag has already left the point it started on.
                let press = ui.input(|input| input.pointer.press_origin()).unwrap_or(pos);
                let mut grabbed = false;
                if let Some((graph, point, distance)) = nearest(press)
                    && distance <= 13.0
                {
                    *selected_graph = graph;
                    *selected_point = point;
                    grabbed = true;
                } else if editor.curve_points_are_editable_structure() {
                    let (x, _) = to_graph(press);
                    if editor.insert_curve_point(*selected_graph, x).is_ok() {
                        grabbed = true;
                        let count = editor
                            .curve_control_point_count(*selected_graph)
                            .unwrap_or(1);
                        *selected_point = (0..count)
                            .filter_map(|point| {
                                editor
                                    .curve_control_point(*selected_graph, point)
                                    .map(|value| (point, (value.0 - x).abs()))
                            })
                            .min_by(|a, b| {
                                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
                            })
                            .map(|(point, _)| point)
                            .unwrap_or(0);
                        changed = true;
                    }
                }
                // A press that grabbed nothing must not drag the point that
                // happened to be selected before it.
                ui.data_mut(|data| data.insert_temp(response.id, grabbed));
            }
            if response.dragged() && ui.data(|data| data.get_temp::<bool>(response.id)).unwrap_or(false) {
                let value = to_graph(pos);
                if editor
                    .set_curve_control_point(*selected_graph, *selected_point, value)
                    .is_ok()
                {
                    changed = true;
                }
            }
        }
        if response.hovered()
            && editor.curve_points_are_editable_structure()
            && ui.input(|input| {
                input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
            })
            && editor
                .curve_is_graph_point(*selected_graph, *selected_point)
                .unwrap_or(false)
            && editor
                .delete_curve_point(*selected_graph, *selected_point)
                .is_ok()
        {
            let count = editor
                .curve_control_point_count(*selected_graph)
                .unwrap_or(1);
            *selected_point = (*selected_point).min(count.saturating_sub(1));
            changed = true;
        }

        let menu_position = response.interact_pointer_pos().unwrap_or(plot.center());
        if editor.curve_points_are_editable_structure() {
        response.context_menu(|ui| {
            let (x, _) = to_graph(menu_position);
            if ui.button("Add point").clicked() {
                if editor.insert_curve_point(*selected_graph, x).is_ok() {
                    changed = true;
                }
                ui.close_menu();
            }
            let is_graph_point = editor
                .curve_is_graph_point(*selected_graph, *selected_point)
                .unwrap_or(false);
            if ui
                .add_enabled(is_graph_point, egui::Button::new("Delete point"))
                .clicked()
            {
                if editor
                    .delete_curve_point(*selected_graph, *selected_point)
                    .is_ok()
                {
                    *selected_point = (*selected_point).saturating_sub(1);
                    changed = true;
                }
                ui.close_menu();
            }
            ui.separator();
            let segment_count = editor.curve_segment_count(*selected_graph).unwrap_or(0);
            let mut start = 0usize;
            let mut starts = Vec::with_capacity(segment_count);
            for segment in 0..segment_count {
                starts.push(start);
                start += match editor.curve_segment_type(*selected_graph, segment) {
                    Some(CurveSegmentType::Linear) => 1,
                    Some(CurveSegmentType::Spline | CurveSegmentType::Spline2) => 3,
                    None => 1,
                };
            }
            let selected_segment = starts
                .iter()
                .rposition(|start| *start <= *selected_point)
                .unwrap_or(0);
            for (kind, label) in [
                (CurveSegmentType::Linear, "Linear segment"),
                (CurveSegmentType::Spline, "Spline segment"),
                (CurveSegmentType::Spline2, "Spline2 segment"),
            ] {
                if ui.button(label).clicked() {
                    if editor
                        .set_curve_segment_type(*selected_graph, selected_segment, kind)
                        .is_ok()
                    {
                        changed = true;
                    }
                    ui.close_menu();
                }
            }
            if selected_segment > 0 {
                ui.separator();
                for (mode, label) in [
                    (CurvePointMode::Corner, "Corner point"),
                    (CurvePointMode::Smooth, "Smooth point"),
                ] {
                    if ui.button(label).clicked() {
                        if editor
                            .set_curve_join_mode(*selected_graph, selected_segment, mode)
                            .is_ok()
                        {
                            changed = true;
                        }
                        ui.close_menu();
                    }
                }
            }
        });
        }
    }

    let function = editor.function();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    if function.color_graph_type() == ColorGraphType::Scalar {
        painter.rect_filled(plot, 0.0, function_plot_bg());
    } else {
        draw_function_color_gradient_vertical(painter, plot, &function_color_stops(function));
    }
    painter.rect_stroke(plot, 0.0, Stroke::new(1.0, grid_line()));
    for i in 1..10 {
        let x = egui::lerp(plot.left()..=plot.right(), i as f32 / 10.0);
        let y = egui::lerp(plot.bottom()..=plot.top(), i as f32 / 10.0);
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            Stroke::new(1.0, function_grid_line()),
        );
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            Stroke::new(1.0, function_grid_line()),
        );
    }
    for graph in 0..editor.graph_count() {
        let range = graph as f32;
        let samples = (0..=128)
            .map(|i| {
                let x = i as f32 / 128.0;
                to_screen((x, function.evaluate_shape(x, range).clamp(0.0, 1.0)))
            })
            .collect::<Vec<_>>();
        let color = if graph == 0 {
            Color32::from_rgb(42, 190, 72)
        } else {
            Color32::from_rgb(220, 55, 55)
        };
        painter.add(egui::Shape::line(samples, Stroke::new(2.0, color)));
    }

    // Guerilla draws a spline's end tangents: p0 to p1 and p3 to p2.
    if editor.function().as_h2().is_some()
        && matches!(editor.function_type(), FunctionType::Spline | FunctionType::Spline2)
    {
        for graph in 0..editor.graph_count() {
            let point = |i: usize| editor.curve_control_point(graph, i).map(to_screen);
            if let (Some(p0), Some(p1), Some(p2), Some(p3)) = (point(0), point(1), point(2), point(3)) {
                let stroke = Stroke::new(1.0, Color32::from_gray(150));
                painter.line_segment([p0, p1], stroke);
                painter.line_segment([p3, p2], stroke);
            }
        }
    }

    for (graph, point, value) in curve_points(editor) {
        let graph_point = editor.curve_is_graph_point(graph, point).unwrap_or(true);
        let selected = graph == *selected_graph && point == *selected_point;
        let center = to_screen(value);
        let radius = if selected { 5.0 } else { 3.5 };
        let fill = if graph == 0 {
            Color32::from_rgb(80, 230, 105)
        } else {
            Color32::from_rgb(245, 90, 90)
        };
        if graph_point {
            painter.rect_filled(
                egui::Rect::from_center_size(center, Vec2::splat(radius * 2.0)),
                0.0,
                fill,
            );
        } else {
            painter.circle_filled(center, radius, fill);
        }
        painter.circle_stroke(center, radius, Stroke::new(1.0, Color32::BLACK));
    }

    if function.color_graph_type() != ColorGraphType::Scalar {
        let bar = egui::Rect::from_min_size(
            rect.left_bottom() + Vec2::new(28.0, 7.0),
            Vec2::new(390.0, 16.0),
        );
        draw_function_color_gradient_horizontal(painter, bar, &function_color_stops(function));
    }
    changed
}

/// The engine stores color stops at non-contiguous slots in the header
/// colors[4] array, defined by the IDA remap table `byte_140CDE670`:
///   0:[0,0,0,0]  1:[0,3,0,0]  2:[0,1,3,0]  3:[0,1,2,3]
/// Empirically verified from real tag data: TwoColor uses slots [0,3]
/// (colors[1] and colors[2] are always zero for TwoColor).
pub(in crate::app) fn color_graph_slots(cgt: ColorGraphType) -> &'static [usize] {
    match cgt {
        ColorGraphType::Scalar => &[],
        ColorGraphType::OneColor => &[0],
        ColorGraphType::TwoColor => &[0, 3],
        ColorGraphType::ThreeColor => &[0, 1, 3],
        ColorGraphType::FourColor => &[0, 1, 2, 3],
    }
}

pub(in crate::app) fn function_color_stops(function: &TagFunction) -> Vec<Color32> {
    let mut stops: Vec<Color32> = match function {
        TagFunction::Blob(blob) => {
            let header = blob.header();
            color_graph_slots(header.color_graph_type)
                .iter()
                .map(|&i| color32_from_argb(header.colors[i]))
                .collect()
        }
        TagFunction::H2(f) => (0..function.color_count())
            .filter_map(|i| f.color(i))
            .map(color32_from_argb)
            .collect(),
    };
    if stops.is_empty() {
        let color = function.evaluate_color(0.0, 0.0);
        stops.push(Color32::from_rgb(
            float_channel_to_u8(color.red),
            float_channel_to_u8(color.green),
            float_channel_to_u8(color.blue),
        ));
    }
    if stops.len() == 1 {
        stops.push(stops[0]);
    }
    stops
}

pub(in crate::app) fn color32_from_argb(argb: u32) -> Color32 {
    // The alpha byte in Halo function ARGB color fields is typically 0
    // (unused/unset), not a transparency value. Force opaque for display.
    Color32::from_rgb(
        ((argb >> 16) & 0xFF) as u8,
        ((argb >> 8) & 0xFF) as u8,
        (argb & 0xFF) as u8,
    )
}

pub(in crate::app) fn draw_function_color_gradient_vertical(
    painter: &egui::Painter,
    rect: egui::Rect,
    stops: &[Color32],
) {
    // Reverse so stop[0] renders at the bottom (y=0, low output) and
    // stop[last] at the top (y=1, high output), matching Guerilla's layout.
    let reversed: Vec<Color32> = stops.iter().rev().cloned().collect();
    draw_function_color_gradient(painter, rect, &reversed, true);
}

pub(in crate::app) fn draw_function_color_gradient_horizontal(
    painter: &egui::Painter,
    rect: egui::Rect,
    stops: &[Color32],
) {
    draw_function_color_gradient(painter, rect, stops, false);
}

pub(in crate::app) fn draw_function_color_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    stops: &[Color32],
    vertical: bool,
) {
    let stops = if stops.is_empty() {
        &[Color32::BLACK, Color32::BLACK][..]
    } else {
        stops
    };
    let steps = if vertical {
        rect.height().round().max(1.0) as usize
    } else {
        rect.width().round().max(1.0) as usize
    }
    .min(256);
    for step in 0..steps {
        let t0 = step as f32 / steps as f32;
        let t1 = (step + 1) as f32 / steps as f32;
        let color = sample_color_stops(stops, t0);
        let strip = if vertical {
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), egui::lerp(rect.top()..=rect.bottom(), t0)),
                egui::pos2(rect.right(), egui::lerp(rect.top()..=rect.bottom(), t1)),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2(egui::lerp(rect.left()..=rect.right(), t0), rect.top()),
                egui::pos2(egui::lerp(rect.left()..=rect.right(), t1), rect.bottom()),
            )
        };
        painter.rect_filled(strip, 0.0, color);
    }
}

pub(in crate::app) fn sample_color_stops(stops: &[Color32], t: f32) -> Color32 {
    if stops.len() == 1 {
        return stops[0];
    }
    let scaled = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let index = scaled.floor() as usize;
    let next = (index + 1).min(stops.len() - 1);
    let local = scaled - index as f32;
    lerp_color(stops[index], stops[next], local)
}

pub(in crate::app) fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |a: u8, b: u8| -> u8 {
        (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)).round() as u8
    };
    Color32::from_rgba_unmultiplied(
        lerp(a.r(), b.r()),
        lerp(a.g(), b.g()),
        lerp(a.b(), b.b()),
        lerp(a.a(), b.a()),
    )
}

#[derive(Clone)]
/// Physical storage used by a function view.
/// Halo 2 byte blocks cannot be edited as ordinary data-field paths and must be
/// replaced through [`FunctionDataOp`] to preserve their surrounding structure.
pub(in crate::app) enum FunctionDataStorage {
    DataField(String),
    Halo2ByteBlock(String),
}

impl FunctionDataStorage {
    pub(in crate::app) fn data_field_path(&self) -> Option<&str> {
        match self {
            Self::DataField(path) => Some(path),
            Self::Halo2ByteBlock(_) => None,
        }
    }
}

#[derive(Clone)]
/// All write targets captured alongside a decoded mapping function.
/// Keeping wrapper fields with the data target lets one popup emit a coherent
/// edit batch without rediscovering paths after the tag borrow ends.
pub(in crate::app) struct FunctionEditPaths {
    /// Backing storage for the raw `mapping_function` blob.
    pub(in crate::app) data: FunctionDataStorage,
    /// `type` — the Output enum (`RenderMethodAnimatedParameterType`).
    pub(in crate::app) parameter_type: String,
    /// `input name` — string_id.
    pub(in crate::app) input_name: String,
    /// `range name` — string_id.
    pub(in crate::app) range_name: String,
    /// `time period` — real (seconds).
    pub(in crate::app) time_period: String,
    /// Parent `animated parameters` block path — used to push a delete op.
    pub(in crate::app) block_path: String,
    /// Index of this animated parameter within `block_path`.
    pub(in crate::app) block_index: usize,
}

#[derive(Clone)]
/// Decoded function plus optional exact write-back information.
/// A missing `edit` target is intentionally read-only; callers must not guess a
/// path from the display label.
pub(in crate::app) struct FunctionView {
    pub(in crate::app) function: TagFunction,
    pub(in crate::app) input_name: String,
    pub(in crate::app) range_name: String,
    /// Output enum index (`RenderMethodAnimatedParameterType`), when the
    /// view came from an animated parameter. Drives the Output dropdown
    /// and the wrapper write-back.
    pub(in crate::app) output_index: Option<i32>,
    pub(in crate::app) time_period_in_seconds: f32,
    /// Tag write targets. `None` when the function has no resolvable
    /// path (material parameter blocks, template summaries) → the editor
    /// renders read-only.
    pub(in crate::app) edit: Option<FunctionEditPaths>,
    /// Which color graph types the field offers.
    pub(in crate::app) color_types: ColorTypeChoices,
}
