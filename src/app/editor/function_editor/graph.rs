//! Function sampling, gradients, and graph painting.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

pub(super) const FUNCTION_GRAPH_SIZE: Vec2 = Vec2::new(465.0, 225.0);
pub(super) const FUNCTION_PLOT_INSET: Vec2 = Vec2::splat(10.0);

/// Tile the grid into disjoint rectangles: every pixel belongs to one color
/// and opacity, including crossings. This avoids compositing translucent
/// white rails underneath the dark centers.
fn function_color_grid_mesh(plot: egui::Rect, grid: FunctionGrid) -> egui::Mesh {
    let axis = |min: f32, max: f32| {
        let mut bands = vec![
            (min, min + 1.0, true, 128),
            (min + 1.0, min + 2.0, false, 128),
            (max - 2.0, max - 1.0, false, 128),
            (max - 1.0, max, true, 128),
        ];
        if let Some(divisions) = grid.divisions() {
            for i in 1..divisions {
                let center = egui::lerp(min..=max, i as f32 / divisions as f32);
                let alpha = if grid == FunctionGrid::Twenty && i % 2 == 1 {
                    64
                } else {
                    128
                };
                bands.extend([
                    (center - 1.5, center - 0.5, false, alpha),
                    (center - 0.5, center + 0.5, true, alpha),
                    (center + 0.5, center + 1.5, false, alpha),
                ]);
            }
        }
        let mut cuts = vec![min, max];
        for &(a, b, _, _) in &bands {
            cuts.extend([a.max(min), b.min(max)]);
        }
        cuts.sort_by(f32::total_cmp);
        cuts.dedup();
        (cuts, bands)
    };
    let (xs, xbands) = axis(plot.left(), plot.right());
    let (ys, ybands) = axis(plot.top(), plot.bottom());
    let mut mesh = egui::Mesh::default();
    for x in xs.windows(2) {
        for y in ys.windows(2) {
            let mut dark = false;
            let mut alpha = 0;
            for (middle, bands) in [
                ((x[0] + x[1]) * 0.5, &xbands),
                ((y[0] + y[1]) * 0.5, &ybands),
            ] {
                for &(a, b, black, opacity) in bands {
                    if middle > a && middle < b {
                        dark |= black;
                        alpha = alpha.max(opacity);
                    }
                }
            }
            if alpha == 0 {
                continue;
            }
            let color = if dark {
                Color32::from_black_alpha(alpha)
            } else {
                Color32::from_white_alpha(alpha)
            };
            mesh.add_colored_rect(
                egui::Rect::from_min_max(egui::pos2(x[0], y[0]), egui::pos2(x[1], y[1])),
                color,
            );
        }
    }
    mesh
}

fn pasted_function_shape(
    source: &TagFunction,
    destination: &TagFunctionEditor,
) -> Option<TagFunctionEditor> {
    if source.encoding() != destination.function().encoding() {
        return None;
    }
    // The function payload carries its graphs and typed parameters. Restore
    // the destination's output union after copying it so colors and scalar
    // output bounds never leak between functions.
    let mut pasted = TagFunctionEditor::from_function(source.clone());
    pasted
        .set_color_graph_type(destination.color_graph_type())
        .ok()?;
    if let Some((min, max)) = destination.clamp_range() {
        pasted.set_clamp_range(min, max).ok()?;
    } else {
        // A constant H3+ function exposes only one active color, but its
        // configured output can still hold additional stops used after paste.
        for index in 0..color_graph_slots(destination.color_graph_type()).len() {
            pasted
                .set_color(index, destination.get_color(index)?)
                .ok()?;
        }
    }
    Some(pasted)
}

fn function_curve_color(graph: usize) -> Color32 {
    if graph == 0 {
        Color32::from_rgb(42, 190, 72)
    } else {
        Color32::from_rgb(220, 55, 55)
    }
}

fn function_handle_diamond(center: egui::Pos2, radius: f32) -> Vec<egui::Pos2> {
    vec![
        center + Vec2::new(0.0, -radius),
        center + Vec2::new(radius, 0.0),
        center + Vec2::new(0.0, radius),
        center + Vec2::new(-radius, 0.0),
    ]
}

pub(super) fn function_point_is_on_curve(editor: &TagFunctionEditor, graph: usize, point: usize) -> bool {
    let h2_spline_handle = editor.function().as_h2().is_some()
        && matches!(editor.function_type(), FunctionType::Spline | FunctionType::Spline2)
        && matches!(point, 1 | 2);
    !h2_spline_handle && editor.curve_is_graph_point(graph, point).unwrap_or(true)
}

pub(super) fn draw_function_selection_marker(
    painter: &egui::Painter, center: egui::Pos2,
    editor: &TagFunctionEditor, graph: usize, point: usize,
) {
    let color = function_curve_color(graph);
    let stroke = Stroke::new(1.0, color);
    if function_point_is_on_curve(editor, graph, point) {
        painter.circle(center, 5.0, Color32::WHITE, stroke);
        painter.circle_filled(center, 3.0, color);
    } else {
        painter.add(egui::Shape::convex_polygon(function_handle_diamond(center, 5.0), Color32::WHITE, stroke));
        painter.add(egui::Shape::convex_polygon(function_handle_diamond(center, 2.5), color, Stroke::NONE));
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FunctionPointerAction {
    Add { graph: usize, x: f32 },
    Delete { graph: usize, point: usize },
}

fn function_segment_points(
    editor: &TagFunctionEditor,
    graph: usize,
    segment: usize,
    to_screen: &impl Fn((f32, f32)) -> egui::Pos2,
) -> Option<Vec<egui::Pos2>> {
    let start = (0..segment)
        .map(|index| {
            if editor.curve_segment_type(graph, index) == Some(CurveSegmentType::Linear) {
                1
            } else {
                3
            }
        })
        .sum::<usize>();
    let end = start
        + if editor.curve_segment_type(graph, segment)? == CurveSegmentType::Linear {
            1
        } else {
            3
        };
    let left = editor.curve_control_point(graph, start)?.0;
    let right = editor.curve_control_point(graph, end)?.0;
    Some(
        (0..=64)
            .map(|sample| {
                let x = egui::lerp(left..=right, sample as f32 / 64.0);
                let y = editor.function().evaluate_shape(x, graph as f32);
                to_screen((
                    x,
                    if editor.is_clamped() {
                        y.clamp(0.0, 1.0)
                    } else {
                        y
                    },
                ))
            })
            .collect(),
    )
}

fn hit_function_segment(
    editor: &TagFunctionEditor,
    pos: egui::Pos2,
    to_screen: &impl Fn((f32, f32)) -> egui::Pos2,
) -> Option<(usize, usize)> {
    let mut closest = None;
    for graph in 0..editor.graph_count() {
        for segment in 0..editor.curve_segment_count(graph).unwrap_or(0) {
            let points = function_segment_points(editor, graph, segment, to_screen)?;
            for pair in points.windows(2) {
                let delta = pair[1] - pair[0];
                let t = if delta.length_sq() > 0.0 {
                    ((pos - pair[0]).dot(delta) / delta.length_sq()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let distance = pos.distance(pair[0] + delta * t);
                if distance <= 8.0 && closest.is_none_or(|(_, _, best)| distance < best) {
                    closest = Some((graph, segment, distance));
                }
            }
        }
    }
    closest.map(|(graph, segment, _)| (graph, segment))
}

/// Cursor badges and modifier clicks share this hit test, including endpoint
/// protection, segment capacity, and rejection of out-of-grid insertions.
fn function_pointer_action(
    editor: &TagFunctionEditor,
    pos: egui::Pos2,
    x: f32,
    inside_plot: bool,
    to_screen: impl Fn((f32, f32)) -> egui::Pos2,
    modifiers: egui::Modifiers,
) -> Option<FunctionPointerAction> {
    if !editor.curve_points_are_editable_structure() {
        return None;
    }
    let nearest = (0..editor.graph_count())
        .flat_map(|graph| {
            (0..editor.curve_control_point_count(graph).unwrap_or(0)).filter_map(move |point| {
                editor
                    .curve_control_point(graph, point)
                    .map(|value| (graph, point, value))
            })
        })
        .map(|(graph, point, value)| (graph, point, to_screen(value).distance(pos)))
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .filter(|hit| hit.2 <= 13.0);
    if modifiers.alt {
        let (graph, point, _) = nearest?;
        function_join_segment(editor, graph, point)?;
        if !delete_function_point(&mut editor.clone(), graph, point) {
            return None;
        }
        return Some(FunctionPointerAction::Delete { graph, point });
    }
    if !modifiers.shift || !inside_plot || nearest.is_some() {
        return None;
    }
    let (graph, distance) = (0..editor.graph_count())
        .map(|graph| {
            let y = editor.function().evaluate_shape(x, graph as f32);
            (graph, to_screen((x, y)).distance(pos))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    if distance > 8.0 {
        return None;
    }
    let mut candidate = editor.clone();
    insert_function_point(&mut candidate, graph, x)?;
    Some(FunctionPointerAction::Add { graph, x })
}

pub(in crate::app) fn function_graph_preview_button(
    ui: &mut Ui,
    rect: egui::Rect,
    function: &TagFunction,
    id: egui::Id,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    let painter = ui.painter();
    let visuals = ui.style().interact(&response);
    painter.rect_filled(rect, 3.0, visuals.bg_fill);
    let plot = rect.shrink(3.0);
    if function.color_graph_type() == ColorGraphType::Scalar {
        painter.rect_filled(plot, 0.0, function_plot_bg());
    } else {
        draw_function_color_gradient_vertical(painter, plot, &function_color_stops(function));
    }
    for graph in 0..if function.is_ranged() { 2 } else { 1 } {
        let samples = (0..=64).map(|step| {
            let x = step as f32 / 64.0;
            let y = function.evaluate_shape(x, graph as f32).clamp(0.0, 1.0);
            egui::pos2(
                egui::lerp(plot.left()..=plot.right(), x),
                egui::lerp(plot.bottom()..=plot.top(), y),
            )
        }).collect::<Vec<_>>();
        for (width, color) in [
            (3.0, Color32::BLACK),
            (2.0, Color32::WHITE),
            (1.0, function_curve_color(graph)),
        ] {
            painter.add(egui::Shape::line(samples.clone(), Stroke::new(width, color)));
        }
    }
    painter.rect_stroke(rect, 3.0, visuals.bg_stroke, egui::StrokeKind::Inside);
    response
}

/// The function editor's graph. It renders the primary and ranged graphs
/// independently and drags their control points: the H3+ multi-part curve
/// (which can also add, remove and retype) and Halo 2's fixed points (moved
/// within the engine's x rules).
#[cfg(test)]
pub(super) fn draw_foundation_graph(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
) -> bool {
    draw_foundation_graph_with_view(
        ui,
        editor,
        editable,
        selected_graph,
        selected_point,
        &mut FunctionGraphView::default(),
    )
}

#[cfg(test)]
pub(super) fn draw_foundation_graph_with_view(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
    view: &mut FunctionGraphView,
) -> bool {
    draw_foundation_graph_with_preview(
        ui,
        editor,
        editable,
        selected_graph,
        selected_point,
        view,
        true,
    )
}

pub(super) fn draw_foundation_graph_with_preview(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
    view: &mut FunctionGraphView,
    show_preview: bool,
) -> bool {
    let size = FUNCTION_GRAPH_SIZE;
    let sense = if editable && editor.master_type() == EngineMasterType::Curve {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    // Reserve the preview strip as well so the controls below cannot overlap it.
    let (allocated, allocation_response) = ui.allocate_exact_size(
        size + Vec2::new(0.0, if show_preview { 60.0 } else { 0.0 }),
        Sense::hover(),
    );
    let rect = egui::Rect::from_min_size(allocated.min, size);
    let response = ui.interact(rect, allocation_response.id, sense);
    if ui.rect_contains_pointer(rect.intersect(ui.clip_rect())) {
        let wheel = ui.input_mut(|input| {
            let wheel = input.smooth_scroll_delta.y;
            input.smooth_scroll_delta.y = 0.0;
            wheel
        });
        view.zoom_percent = (view.zoom_percent + wheel * 0.2).clamp(50.0, 100.0);
    }
    let viewport = rect.shrink2(FUNCTION_PLOT_INSET);
    let bounds = function_view_bounds(view.zoom_percent);
    let to_screen = |(x, y): (f32, f32)| {
        egui::pos2(
            egui::remap(x, bounds[0]..=bounds[1], viewport.left()..=viewport.right()),
            egui::remap(y, bounds[2]..=bounds[3], viewport.bottom()..=viewport.top()),
        )
    };
    let plot = egui::Rect::from_two_pos(to_screen((0.0, 0.0)), to_screen((1.0, 1.0)));
    let to_graph = |pos: egui::Pos2| {
        (
            egui::remap(
                pos.x,
                viewport.left()..=viewport.right(),
                bounds[0]..=bounds[1],
            ),
            egui::remap(
                pos.y,
                viewport.bottom()..=viewport.top(),
                bounds[2]..=bounds[3],
            ),
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

    let (mut changed, exclusion_captured, exclusion_markers) =
        function_exclusion_controls(ui, editor, editable, response.id, plot, to_graph, view.grid);
    let segment_id = response.id.with("selected_segment");
    let hovered_segment =
        if !exclusion_captured && ui.rect_contains_pointer(rect.intersect(ui.clip_rect())) {
            ui.input(|input| input.pointer.hover_pos())
                .and_then(|pos| hit_function_segment(editor, pos, &to_screen))
        } else {
            None
        };
    let point_selection = (*selected_graph, *selected_point);
    if response.secondary_clicked() || (response.clicked() && hovered_segment.is_some()) {
        ui.data_mut(|data| data.insert_temp(segment_id, hovered_segment));
        if let Some((graph, _)) = hovered_segment {
            *selected_graph = graph;
            *selected_point = (*selected_point).min(
                editor
                    .curve_control_point_count(graph)
                    .unwrap_or(1)
                    .saturating_sub(1),
            );
        }
    }
    let selected_segment = ui
        .data(|data| data.get_temp::<Option<(usize, usize)>>(segment_id))
        .flatten();
    let modifiers = ui.input(|input| input.modifiers);
    if editable && editor.master_type() == EngineMasterType::Curve && !exclusion_captured {
        let hit_points = curve_points(editor);
        let nearest = |pos: egui::Pos2| {
            let closest = hit_points
                .iter()
                .map(|&(graph, point, value)| (graph, point, to_screen(value).distance(pos)))
                .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))?;
            if closest.2 > 13.0 {
                return Some(closest);
            }
            let center = to_screen(editor.curve_control_point(closest.0, closest.1)?);
            let overlapping: Vec<_> = hit_points
                .iter()
                .filter(|(_, _, value)| to_screen(*value).distance(center) <= 3.0)
                .map(|&(graph, point, value)| (graph, point, to_screen(value).distance(pos)))
                .filter(|hit| hit.2 <= 13.0)
                .collect();
            let selected = overlapping
                .iter()
                .position(|hit| (hit.0, hit.1) == point_selection);
            let cycle = response.clicked() && !modifiers.alt && !modifiers.shift;
            let index = match selected {
                Some(index) if cycle => (index + 1) % overlapping.len(),
                Some(index) => index,
                None => return Some(closest),
            };
            Some(overlapping[index])
        };
        if let Some(pos) = response.interact_pointer_pos() {
            if response.drag_started() || response.clicked() {
                // Pick where the button went down: egui reports the drag only
                // once the pointer has moved past its threshold, by which point
                // a quick drag has already left the point it started on.
                let press = ui
                    .input(|input| input.pointer.press_origin())
                    .unwrap_or(pos);
                let mut grabbed = false;
                if let Some((graph, point, distance)) = nearest(press)
                    && distance <= 13.0
                    && !function_point_is_on_curve(editor, graph, point)
                {
                    // Shift/Alt on a handle control its paired tangent rather
                    // than adding or deleting graph points.
                    *selected_graph = graph;
                    *selected_point = point;
                    grabbed = true;
                } else {
                    match function_pointer_action(
                        editor,
                        press,
                        to_graph(press).0,
                        plot.contains(press),
                        to_screen,
                        modifiers,
                    ) {
                        Some(FunctionPointerAction::Delete { graph, point }) => {
                            if delete_function_point(editor, graph, point) {
                                *selected_graph = graph;
                                *selected_point = point.saturating_sub(1);
                                changed = true;
                            }
                        }
                        Some(FunctionPointerAction::Add { graph, x }) => {
                            if let Some(point) = insert_function_point(editor, graph, x) {
                                *selected_graph = graph;
                                *selected_point = point;
                                changed = true;
                            }
                        }
                        None => {
                            if !modifiers.alt
                                && !modifiers.shift
                                && let Some((graph, point, distance)) = nearest(press)
                                && distance <= 13.0
                            {
                                *selected_graph = graph;
                                *selected_point = point;
                                grabbed = true;
                            }
                        }
                    }
                }
                // A press that grabbed nothing must not drag the point that
                // happened to be selected before it.
                ui.data_mut(|data| data.insert_temp(response.id, grabbed));
            }
            // egui 0.36 calls it a drag as soon as the pointer leaves the
            // graph, even within click distance, so a click that slipped off
            // the edge moved the point it had just added. A point moves once
            // the pointer has gone further than a click may.
            let max_click_dist = ui
                .ctx()
                .options(|options| options.input_options.max_click_dist);
            let past_click_distance = ui.input(|input| {
                input
                    .pointer
                    .press_origin()
                    .is_none_or(|origin| origin.distance(pos) > max_click_dist)
            });
            if response.dragged()
                && past_click_distance
                && ui
                    .data(|data| data.get_temp::<bool>(response.id))
                    .unwrap_or(false)
            {
                let value = to_graph(pos);
                changed |= move_function_point_in_view(
                    editor,
                    *selected_graph,
                    *selected_point,
                    value,
                    modifiers,
                    *view,
                );
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
            && delete_function_point(editor, *selected_graph, *selected_point)
        {
            let count = editor
                .curve_control_point_count(*selected_graph)
                .unwrap_or(1);
            *selected_point = (*selected_point).min(count.saturating_sub(1));
            changed = true;
        }
    }
    let menu_position = response.interact_pointer_pos().unwrap_or(plot.center());
    context_menu(&response, |ui| {
        let clipboard_id = egui::Id::new("function_graph_shape_clipboard");
        if ui.button("Copy").clicked() {
            ui.data_mut(|data| data.insert_temp(clipboard_id, editor.function().clone()));
            close_menu(ui);
        }
        let clipboard = ui.data(|data| data.get_temp::<TagFunction>(clipboard_id));
        let pasted = clipboard
            .as_ref()
            .and_then(|source| pasted_function_shape(source, editor));
        let paste = ui.add_enabled(editable && pasted.is_some(), egui::Button::new("Paste"));
        if paste.on_hover_text(if !editable {
                    "This function is read-only"
                } else if clipboard.is_none() {
                    "Copy a function graph first"
                } else {
                    "Paste the function shape, preserving this output type and its colors or scalar bounds. Halo 2 and H3+ shapes use different formats."
                }).clicked() {
                    if let Some(pasted) = pasted {
                        changed |= editor.to_bytes() != pasted.to_bytes();
                        *editor = pasted;
                        *selected_graph = 0;
                        *selected_point = 0;
                        ui.data_mut(|data| data.insert_temp(segment_id, None::<(usize, usize)>));
                    }
                    close_menu(ui);
                }
        if editable && editor.curve_points_are_editable_structure() {
            ui.separator();
            let (x, _) = to_graph(menu_position);
            if ui.button("Add point").clicked() {
                if let Some(point) = insert_function_point(editor, *selected_graph, x) {
                    *selected_point = point;
                    changed = true;
                }
                close_menu(ui);
            }
            let is_graph_point =
                function_join_segment(editor, *selected_graph, *selected_point).is_some();
            if ui
                .add_enabled(is_graph_point, egui::Button::new("Delete point"))
                .clicked()
            {
                if delete_function_point(editor, *selected_graph, *selected_point) {
                    *selected_point = (*selected_point).saturating_sub(1);
                    changed = true;
                }
                close_menu(ui);
            }
            ui.separator();
            for (kind, label) in [
                (CurveSegmentType::Linear, "Linear segment"),
                (CurveSegmentType::Spline, "Spline segment"),
                (CurveSegmentType::Spline2, "Spline2 segment"),
            ] {
                if ui
                    .add_enabled(selected_segment.is_some(), egui::Button::new(label))
                    .clicked()
                {
                    if let Some((graph, segment)) = selected_segment
                        && set_function_segment_type(editor, graph, segment, kind)
                    {
                        changed = true;
                    }
                    close_menu(ui);
                }
            }
        }
    });

    let function = editor.function();
    let painter = &ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let container_fill = if is_dark_mode() {
        Color32::from_black_alpha(64)
    } else {
        Color32::BLACK
    };
    painter.rect_filled(rect, 4.0, container_fill);
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, foundation_input_edge()),
        egui::StrokeKind::Inside,
    );
    if function.color_graph_type() == ColorGraphType::Scalar {
        painter.rect_filled(plot, 0.0, function_plot_bg());
    } else {
        draw_function_color_gradient_vertical(painter, plot, &function_color_stops(function));
    }
    let color_grid = function.color_graph_type() != ColorGraphType::Scalar;
    if !color_grid {
        painter.rect_stroke(
            plot,
            0.0,
            Stroke::new(1.0_f32, grid_line()),
            egui::StrokeKind::Middle,
        );
    }
    if let Some(blob) = function.as_blob().filter(|blob| blob.is_exclusion()) {
        let top = to_screen((0.0, blob.exclusion_max().clamp(0.0, 1.0))).y;
        let bottom = to_screen((0.0, blob.exclusion_min().clamp(0.0, 1.0))).y;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(plot.left(), top),
                egui::pos2(plot.right(), bottom),
            ),
            0.0,
            Color32::from_white_alpha(28),
        );
        for y in [top, bottom] {
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                Stroke::new(1.0, Color32::from_white_alpha(120)),
            );
        }
    }
    if color_grid {
        painter.add(egui::Shape::mesh(function_color_grid_mesh(plot, view.grid)));
    }
    if !color_grid {
        for i in 1..view.grid.divisions().unwrap_or(0) {
            let divisions = view.grid.divisions().unwrap() as f32;
            let color = if view.grid == FunctionGrid::Twenty && i % 2 == 1 {
                function_grid_line().gamma_multiply(0.5)
            } else {
                function_grid_line()
            };
            let x = egui::lerp(plot.left()..=plot.right(), i as f32 / divisions);
            let y = egui::lerp(plot.bottom()..=plot.top(), i as f32 / divisions);
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                Stroke::new(1.0, color),
            );
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                Stroke::new(1.0, color),
            );
        }
    }
    if let Some((graph, segment)) = selected_segment.or(hovered_segment)
        && let Some(points) = function_segment_points(editor, graph, segment, &to_screen)
    {
        painter.add(egui::Shape::line(
            points,
            Stroke::new(8.0, function_curve_color(graph).gamma_multiply(0.4)),
        ));
    }
    for graph in 0..editor.graph_count() {
        let range = graph as f32;
        let samples = (0..=128)
            .map(|i| {
                let x = i as f32 / 128.0;
                let y = function.evaluate_shape(x, range);
                to_screen((
                    x,
                    if editor.is_clamped() {
                        y.clamp(0.0, 1.0)
                    } else {
                        y
                    },
                ))
            })
            .collect::<Vec<_>>();
        // Paint from the outside in so both outlines remain visible.
        for (width, color) in [
            (3.0, Color32::BLACK),
            (2.0, Color32::WHITE),
            (1.0, function_curve_color(graph)),
        ] {
            painter.add(egui::Shape::line(
                samples.clone(),
                Stroke::new(width, color),
            ));
        }
    }

    // Guerilla draws a spline's end tangents: p0 to p1 and p3 to p2.
    if editor.function().as_h2().is_some()
        && matches!(
            editor.function_type(),
            FunctionType::Spline | FunctionType::Spline2
        )
    {
        for graph in 0..editor.graph_count() {
            let point = |i: usize| editor.curve_control_point(graph, i).map(to_screen);
            if let (Some(p0), Some(p1), Some(p2), Some(p3)) =
                (point(0), point(1), point(2), point(3))
            {
                let stroke = Stroke::new(1.0_f32, function_curve_color(graph));
                painter.line_segment([p0, p1], stroke);
                painter.line_segment([p3, p2], stroke);
            }
        }
    }

    // H3+ curves store shared endpoints between segments. Each spline adds
    // two tangent handles and its end point; linear segments add only an end.
    if editor.function().as_blob().is_some() {
        for graph in 0..editor.graph_count() {
            let mut start = 0;
            for segment in 0..editor.curve_segment_count(graph).unwrap_or(0) {
                match editor.curve_segment_type(graph, segment) {
                    Some(CurveSegmentType::Spline | CurveSegmentType::Spline2) => {
                        for (endpoint, handle) in [(start, start + 1), (start + 3, start + 2)] {
                            if let (Some(endpoint), Some(handle)) = (
                                editor.curve_control_point(graph, endpoint),
                                editor.curve_control_point(graph, handle),
                            ) {
                                painter.line_segment(
                                    [to_screen(endpoint), to_screen(handle)],
                                    Stroke::new(1.0, function_curve_color(graph)),
                                );
                            }
                        }
                        start += 3;
                    }
                    Some(CurveSegmentType::Linear) => start += 1,
                    None => {}
                }
            }
        }
    }

    for (center, left, selected) in exclusion_markers {
        let direction = if left { 1.0 } else { -1.0 };
        let points = [
            (-4.0, -3.0),
            (1.0, -3.0),
            (4.0, 0.0),
            (1.0, 3.0),
            (-4.0, 3.0),
        ]
        .into_iter()
        .map(|(x, y)| center + Vec2::new(x * direction, y))
        .collect();
        painter.add(egui::Shape::convex_polygon(
            points,
            if selected {
                function_curve_color(0)
            } else {
                function_plot_bg()
            },
            Stroke::new(1.0, Color32::WHITE),
        ));
    }
    let mut drawn_points = curve_points(editor);
    drawn_points
        .sort_by_key(|(graph, point, _)| (*graph, *point) == (*selected_graph, *selected_point));
    for (graph, point, value) in drawn_points {
        // The backend classifies all H2 control points as graph points, but
        // spline p1/p2 are the tangent handles drawn above, not endpoints.
        let graph_point = function_point_is_on_curve(editor, graph, point);
        let selected = graph == *selected_graph && point == *selected_point;
        let center = to_screen(value);
        let radius = if selected { 5.0 } else { 3.5 };
        let color = function_curve_color(graph);
        let stroke = Stroke::new(1.0_f32, color);
        if graph_point {
            painter.circle(center, radius, Color32::WHITE, stroke);
            if selected {
                painter.circle_filled(center, radius - 2.0, color);
            }
        } else {
            painter.add(egui::Shape::convex_polygon(
                function_handle_diamond(center, radius),
                Color32::WHITE,
                stroke,
            ));
            if selected {
                painter.add(egui::Shape::convex_polygon(
                    function_handle_diamond(center, radius - 2.5),
                    color,
                    Stroke::NONE,
                ));
            }
        }
    }

    let painter = ui.painter();
    if editable
        && response.hovered()
        && let Some(pos) = ui.input(|input| input.pointer.hover_pos())
        && let Some(action) = function_pointer_action(
            editor,
            pos,
            to_graph(pos).0,
            plot.contains(pos),
            to_screen,
            modifiers,
        )
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
        let center = pos + Vec2::new(14.0, 14.0);
        let badge = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            response.id.with("point_cursor"),
        ));
        badge.circle_filled(center, 7.0, Color32::BLACK);
        badge.line_segment(
            [center - Vec2::new(4.0, 0.0), center + Vec2::new(4.0, 0.0)],
            Stroke::new(1.5, Color32::WHITE),
        );
        if matches!(action, FunctionPointerAction::Add { .. }) {
            badge.line_segment(
                [center - Vec2::new(0.0, 4.0), center + Vec2::new(0.0, 4.0)],
                Stroke::new(1.5, Color32::WHITE),
            );
        }
    }
    if show_preview {
        let bar_container = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.bottom() + 8.0),
            Vec2::new(rect.width(), 52.0),
        );
        draw_function_preview_container(painter, bar_container, plot.width(), function);
    }
    changed
}

pub(super) fn draw_function_preview_container(
    painter: &egui::Painter,
    bar_container: egui::Rect,
    width: f32,
    function: &TagFunction,
) {
    let container_fill = if is_dark_mode() {
        Color32::from_black_alpha(64)
    } else {
        Color32::BLACK
    };
    painter.rect_filled(bar_container, 4.0, container_fill);
    painter.rect_stroke(
        bar_container,
        4.0,
        Stroke::new(1.0, foundation_input_edge()),
        egui::StrokeKind::Inside,
    );
    let preview = egui::Rect::from_center_size(
        bar_container.center(),
        Vec2::new(width, bar_container.height() - 20.0),
    );
    draw_function_output_preview(painter, preview, function);
}

fn function_exclusion_controls(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    id: egui::Id,
    plot: egui::Rect,
    to_graph: impl Fn(egui::Pos2) -> (f32, f32),
    grid: FunctionGrid,
) -> (bool, bool, Vec<(egui::Pos2, bool, bool)>) {
    let Some(blob) = editor
        .function()
        .as_blob()
        .filter(|blob| blob.is_exclusion())
    else {
        return (false, false, Vec::new());
    };
    let (mut min, mut max) = (blob.exclusion_min(), blob.exclusion_max());
    let mut changed = false;
    let mut captured = false;
    let selection_id = id.with("exclusion_selection");
    let mut selected = ui
        .data(|data| data.get_temp::<usize>(selection_id))
        .unwrap_or(1);
    let mut markers = Vec::new();
    for bound in 0..2 {
        for left in [true, false] {
            let value = if bound == 0 { min } else { max }.clamp(0.0, 1.0);
            let center = egui::pos2(
                if left {
                    plot.left() - 6.0
                } else {
                    plot.right() + 6.0
                },
                egui::lerp(plot.bottom()..=plot.top(), value),
            );
            let response = ui.interact(
                egui::Rect::from_center_size(center, Vec2::new(10.0, 10.0)),
                id.with(("exclusion", bound, left)),
                if editable {
                    Sense::click_and_drag()
                } else {
                    Sense::hover()
                },
            );
            captured |=
                response.hovered() || response.is_pointer_button_down_on() || response.dragged();
            if response.clicked() || response.drag_started() {
                selected = bound;
            }
            if editable
                && response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let mut value = to_graph(pos).1.clamp(0.0, 1.0);
                if ui.input(|input| input.modifiers.ctrl)
                    && let Some(divisions) = grid.divisions()
                {
                    let divisions = divisions as f32;
                    value = (value * divisions).round() / divisions;
                }
                if bound == 0 {
                    value = value.min(max.clamp(0.0, 1.0));
                    min = value;
                } else {
                    value = value.max(min.clamp(0.0, 1.0));
                    max = value;
                }
                changed |= set_function_exclusion_bounds(editor, min, max);
            }
            response
                .on_hover_cursor(egui::CursorIcon::ResizeVertical)
                .on_hover_text(format!(
                    "Exclusion {}: {:.3}",
                    if bound == 0 { "minimum" } else { "maximum" },
                    value
                ));
            markers.push((center, left, selected == bound));
        }
    }
    ui.data_mut(|data| data.insert_temp(selection_id, selected));
    (changed, captured, markers)
}

fn draw_function_output_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    function: &TagFunction,
) {
    let pixels_per_point = painter.ctx().pixels_per_point();
    let columns = (rect.width() * pixels_per_point).ceil().clamp(128.0, 4096.0) as u32;
    let rows = if function.is_ranged() {
        (rect.height() * pixels_per_point).ceil().clamp(1.0, 64.0) as u32
    } else {
        1
    };
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices(((columns + 1) * (rows + 1)) as usize);
    mesh.reserve_triangles((columns * rows * 2) as usize);
    for row in 0..=rows {
        // Range increases bottom-to-top, showing both curves and the outputs
        // between them. Non-ranged functions always evaluate the primary curve.
        let range = if function.is_ranged() { row as f32 / rows as f32 } else { 0.0 };
        let y = egui::lerp(rect.bottom()..=rect.top(), row as f32 / rows as f32);
        for column in 0..=columns {
            let input = column as f32 / columns as f32;
            let color = if function.color_graph_type() == ColorGraphType::Scalar {
                // Match Guerilla: preview the curve's intensity before the
                // scalar output Min/Max mapping, retaining the function's own
                // amplitude bounds and the blend between ranged curves.
                Color32::from_gray(float_channel_to_u8(function.evaluate_shape(input, range).clamp(0.0, 1.0)))
            } else {
                let color = function.evaluate_color(input, range);
                Color32::from_rgb(
                    float_channel_to_u8(color.red),
                    float_channel_to_u8(color.green),
                    float_channel_to_u8(color.blue),
                )
            };
            mesh.colored_vertex(egui::pos2(egui::lerp(rect.left()..=rect.right(), input), y), color);
        }
    }
    for row in 0..rows {
        for column in 0..columns {
            let start = row * (columns + 1) + column;
            let next_row = start + columns + 1;
            mesh.add_triangle(start, start + 1, next_row + 1);
            mesh.add_triangle(start, next_row + 1, next_row);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
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

pub(in crate::app) fn draw_function_color_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    stops: &[Color32],
    vertical: bool,
) {
    let stops = if stops.is_empty() {
        &[Color32::BLACK, Color32::BLACK][..]
    } else if stops.len() == 1 {
        painter.rect_filled(rect, 0.0, stops[0]);
        return;
    } else {
        stops
    };
    // Shared vertices at each color stop let the GPU interpolate continuously
    // across the whole interval, rather than painting quantized solid strips.
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices(stops.len() * 2);
    mesh.reserve_triangles((stops.len() - 1) * 2);
    for (index, &color) in stops.iter().enumerate() {
        let t = index as f32 / (stops.len() - 1) as f32;
        if vertical {
            let y = egui::lerp(rect.top()..=rect.bottom(), t);
            mesh.colored_vertex(egui::pos2(rect.left(), y), color);
            mesh.colored_vertex(egui::pos2(rect.right(), y), color);
        } else {
            let x = egui::lerp(rect.left()..=rect.right(), t);
            mesh.colored_vertex(egui::pos2(x, rect.top()), color);
            mesh.colored_vertex(egui::pos2(x, rect.bottom()), color);
        }
    }
    for interval in 0..(stops.len() - 1) as u32 {
        let start = interval * 2;
        mesh.add_triangle(start, start + 1, start + 3);
        mesh.add_triangle(start, start + 3, start + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
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
    /// view came from an animated parameter. Preserves the shader row's
    /// output assignment when writing the function back.
    pub(in crate::app) output_index: Option<i32>,
    pub(in crate::app) time_period_in_seconds: f32,
    /// Tag write targets. `None` when the function has no resolvable
    /// path (material parameter blocks, template summaries) → the editor
    /// renders read-only.
    pub(in crate::app) edit: Option<FunctionEditPaths>,
    /// Which color graph types the field offers.
    pub(in crate::app) color_types: ColorTypeChoices,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interactive_graph_frame(
        ctx: &egui::Context,
        editor: &mut TagFunctionEditor,
        view: &mut FunctionGraphView,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        crate::app::run_ui_test(
            ctx,
            egui::RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 600.0),
                )),
                ..Default::default()
            },
            |_| {
                egui::Area::new(egui::Id::new("interactive_graph_controls_test"))
                    .fixed_pos(egui::Pos2::ZERO)
                    .show(ctx, |ui| {
                        draw_foundation_graph_with_view(ui, editor, true, &mut 0, &mut 0, view);
                    });
            },
        )
    }

    #[test]
    fn selected_point_is_painted_over_an_overlapping_range_point() {
        let mut editor =
            TagFunctionEditor::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        set_function_ranged(&mut editor, true);
        let value = editor.curve_control_point(0, 0).unwrap();
        editor.set_curve_control_point(1, 0, value).unwrap();
        for graph in [0, 1] {
            let output =
                crate::app::run_ui_test(&egui::Context::default(), egui::RawInput::default(), |ui| {
                    draw_foundation_graph(ui, &mut editor, true, &mut graph.clone(), &mut 0);
                });
            let selected_center = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle) if circle.radius == 5.0 => Some(circle.center),
                    _ => None,
                })
                .unwrap();
            let last = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle) if circle.center == selected_center => {
                        Some(circle.fill)
                    }
                    _ => None,
                })
                .last()
                .unwrap();
            assert_eq!(last, function_curve_color(graph));
        }
    }

    #[test]
    fn wheel_zoom_is_limited_to_the_graph_container_and_consumes_scrolling() {
        let ctx = egui::Context::default();
        let mut editor =
            TagFunctionEditor::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        let mut view = FunctionGraphView::default();
        interactive_graph_frame(&ctx, &mut editor, &mut view, vec![]);
        let inside = egui::pos2(200.0, 100.0);
        interactive_graph_frame(
            &ctx,
            &mut editor,
            &mut view,
            vec![egui::Event::PointerMoved(inside)],
        );
        let wheel = || egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, -100.0),
            modifiers: egui::Modifiers::NONE,
        };
        interactive_graph_frame(&ctx, &mut editor, &mut view, vec![wheel()]);
        assert!(view.zoom_percent < 100.0 && view.zoom_percent >= 50.0);
        assert_eq!(ctx.input(|input| input.smooth_scroll_delta.y), 0.0);
        let zoom = view.zoom_percent;
        interactive_graph_frame(
            &ctx,
            &mut editor,
            &mut view,
            vec![egui::Event::PointerMoved(egui::pos2(600.0, 500.0)), wheel()],
        );
        assert_eq!(view.zoom_percent, zoom);
    }

    #[test]
    fn exclusion_side_markers_drag_both_bounds_without_moving_curve_points() {
        let ctx = egui::Context::default();
        let mut editor =
            TagFunctionEditor::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        set_editor_flag(&mut editor, FunctionFlags::EXCLUSION, true);
        set_function_exclusion_bounds(&mut editor, 0.25, 0.75);
        let before = editor.to_bytes()[32..].to_vec();
        let mut view = FunctionGraphView::default();
        interactive_graph_frame(&ctx, &mut editor, &mut view, vec![]);
        let plot = egui::Rect::from_min_size(egui::Pos2::ZERO, FUNCTION_GRAPH_SIZE)
            .shrink2(FUNCTION_PLOT_INSET);
        for (left, from, to) in [(true, 0.75, 0.9), (false, 0.25, 0.4)] {
            let point = |y| {
                egui::pos2(
                    if left {
                        plot.left() - 6.0
                    } else {
                        plot.right() + 6.0
                    },
                    egui::lerp(plot.bottom()..=plot.top(), y),
                )
            };
            let (start, end) = (point(from), point(to));
            interactive_graph_frame(
                &ctx,
                &mut editor,
                &mut view,
                vec![egui::Event::PointerMoved(start)],
            );
            interactive_graph_frame(
                &ctx,
                &mut editor,
                &mut view,
                vec![egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            for _ in 0..2 {
                interactive_graph_frame(
                    &ctx,
                    &mut editor,
                    &mut view,
                    vec![egui::Event::PointerMoved(end)],
                );
            }
            interactive_graph_frame(
                &ctx,
                &mut editor,
                &mut view,
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
        let blob = reparsed.function().as_blob().unwrap();
        assert!(
            (blob.exclusion_min() - 0.4).abs() < 0.001
                && (blob.exclusion_max() - 0.9).abs() < 0.001,
            "{}, {}",
            blob.exclusion_min(),
            blob.exclusion_max()
        );
        assert_eq!(editor.to_bytes()[32..], before);
    }

    #[test]
    fn control_dragging_exclusion_snaps_to_the_selected_grid() {
        for (grid, expected) in [
            (FunctionGrid::Off, 0.86),
            (FunctionGrid::Ten, 0.9),
            (FunctionGrid::Twenty, 0.85),
        ] {
            let ctx = egui::Context::default();
            let mut editor =
                TagFunctionEditor::parse(&decode_hex(&constant_function_hex(0.0)).unwrap())
                    .unwrap();
            editor.set_master_type(EngineMasterType::Curve).unwrap();
            set_editor_flag(&mut editor, FunctionFlags::EXCLUSION, true);
            set_function_exclusion_bounds(&mut editor, 0.25, 0.75);
            let mut view = FunctionGraphView {
                grid,
                ..Default::default()
            };
            let plot = egui::Rect::from_min_size(egui::Pos2::ZERO, FUNCTION_GRAPH_SIZE)
                .shrink2(FUNCTION_PLOT_INSET);
            let point =
                |y| egui::pos2(plot.left() - 6.0, egui::lerp(plot.bottom()..=plot.top(), y));
            let (start, end) = (point(0.75), point(0.86));
            let mut frame = |mut events: Vec<egui::Event>| {
                events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers::CTRL));
                interactive_graph_frame(&ctx, &mut editor, &mut view, events);
            };
            frame(vec![]);
            frame(vec![egui::Event::PointerMoved(start)]);
            frame(vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::CTRL,
            }]);
            for _ in 0..2 {
                frame(vec![egui::Event::PointerMoved(end)]);
            }
            frame(vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::CTRL,
            }]);
            let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
            let value = reparsed.function().as_blob().unwrap().exclusion_max();
            assert!((value - expected).abs() < 0.001, "{grid:?}: {value}");
        }
    }

    #[test]
    fn color_grid_is_a_single_translucent_layer_without_overlaps() {
        let plot = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0));
        for grid in [FunctionGrid::Ten, FunctionGrid::Twenty, FunctionGrid::Off] {
            let mesh = function_color_grid_mesh(plot, grid);
            let cells: Vec<_> = mesh
                .vertices
                .chunks_exact(4)
                .map(|vertices| {
                    let mut rect = egui::Rect::NOTHING;
                    for vertex in vertices {
                        rect.extend_with(vertex.pos);
                    }
                    assert!(
                        vertices
                            .iter()
                            .all(|vertex| vertex.color.a() == 128 || vertex.color.a() == 64)
                    );
                    (rect, vertices[0].color)
                })
                .collect();
            for (index, (rect, _)) in cells.iter().enumerate() {
                for (other, _) in &cells[index + 1..] {
                    let overlap = rect.intersect(*other);
                    assert!(overlap.width() <= 0.0 || overlap.height() <= 0.0);
                }
            }
            let color_at = |point| {
                cells
                    .iter()
                    .find(|(rect, _)| rect.contains(point))
                    .map(|(_, color)| *color)
            };
            assert_eq!(
                color_at(egui::pos2(0.25, 5.25)),
                Some(Color32::from_black_alpha(128))
            );
            assert_eq!(
                color_at(egui::pos2(1.25, 5.25)),
                Some(Color32::from_white_alpha(128))
            );
            if grid != FunctionGrid::Off {
                assert_eq!(
                    color_at(egui::pos2(40.25, 20.25)),
                    Some(Color32::from_black_alpha(128))
                );
                assert_eq!(
                    color_at(egui::pos2(41.25, 21.25)),
                    Some(Color32::from_white_alpha(128))
                );
            }
            if grid == FunctionGrid::Twenty {
                assert_eq!(
                    color_at(egui::pos2(20.25, 5.25)),
                    Some(Color32::from_black_alpha(64))
                );
            }
        }
    }
    #[test]
    fn zoom_keeps_the_graph_centered_and_grid_subdivisions_are_fainter() {
        let function =
            TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        let mut editor = TagFunctionEditor::from_function(function);
        editor.set_master_type(EngineMasterType::Periodic).unwrap();
        let mut params = editor.periodic_params(0).unwrap();
        params.amplitude_min = -1.0;
        params.amplitude_max = 2.0;
        editor.set_periodic_params(0, params).unwrap();
        let mut plots = Vec::new();
        let mut previews = Vec::new();
        for (zoom, grid, expected_major, expected_minor) in [
            (100.0, FunctionGrid::Ten, 18, 0),
            (50.0, FunctionGrid::Twenty, 18, 20),
            (50.0, FunctionGrid::Off, 0, 0),
        ] {
            let ctx = egui::Context::default();
            let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                draw_foundation_graph_with_view(
                    ui,
                    &mut editor,
                    false,
                    &mut 0,
                    &mut 0,
                    &mut FunctionGraphView {
                        zoom_percent: zoom,
                        grid,
                    },
                );
            });
            let plot = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == function_plot_bg() => Some(rect.rect),
                    _ => None,
                })
                .unwrap();
            plots.push(plot);
            let preview = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) => {
                        let min_x = mesh
                            .vertices
                            .iter()
                            .map(|vertex| vertex.pos.x)
                            .min_by(f32::total_cmp)?;
                        let max_x = mesh
                            .vertices
                            .iter()
                            .map(|vertex| vertex.pos.x)
                            .max_by(f32::total_cmp)?;
                        Some((min_x, max_x))
                    }
                    _ => None,
                })
                .unwrap();
            previews.push(preview);
            let major = function_grid_line();
            let minor = major.gamma_multiply(0.5);
            let line_count = |color| {
                output
                    .shapes
                    .iter()
                    .filter(|shape| {
                        matches!(&shape.shape,
                egui::Shape::LineSegment { stroke, .. } if stroke.color == color)
                    })
                    .count()
            };
            assert_eq!(line_count(major), expected_major);
            assert_eq!(line_count(minor), expected_minor);
        }
        assert_eq!(plots[0].center(), plots[1].center());
        assert_eq!(plots[1], plots[2]);
        assert!((plots[0].width() * 0.5 - plots[1].width()).abs() < 0.001);
        assert!((plots[0].height() * 0.5 - plots[1].height()).abs() < 0.001);
        for (plot, (left, right)) in plots.iter().zip(previews) {
            assert!((plot.left() - left).abs() < 0.001 && (plot.right() - right).abs() < 0.001);
        }
    }

    #[test]
    fn segment_hit_testing_uses_the_hovered_curve_and_span() {
        let mut editor =
            TagFunctionEditor::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        editor.set_ranged(true).unwrap();
        editor.insert_curve_point(0, 0.5).unwrap();
        editor.insert_curve_point(1, 0.5).unwrap();
        for (point, value) in [(0, (0.0, 1.0)), (1, (0.5, 0.5)), (2, (1.0, 0.0))] {
            editor.set_curve_control_point(1, point, value).unwrap();
        }
        let screen = |(x, y): (f32, f32)| egui::pos2(x * 400.0, (1.0 - y) * 200.0);
        assert_eq!(
            hit_function_segment(&editor, screen((0.75, 0.75)), &screen),
            Some((0, 1))
        );
        assert_eq!(
            hit_function_segment(&editor, screen((0.25, 0.75)), &screen),
            Some((1, 0))
        );
        assert_eq!(
            hit_function_segment(&editor, screen((0.25, 0.1)), &screen),
            None
        );
    }

    #[test]
    fn context_menu_retypes_and_highlights_the_hovered_segment_instead_of_the_selected_point() {
        let ctx = egui::Context::default();
        let mut editor =
            TagFunctionEditor::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        editor.insert_curve_point(0, 0.5).unwrap();
        let mut view = FunctionGraphView::default();
        interactive_graph_frame(&ctx, &mut editor, &mut view, vec![]);
        let plot = egui::Rect::from_min_size(egui::Pos2::ZERO, FUNCTION_GRAPH_SIZE)
            .shrink2(FUNCTION_PLOT_INSET);
        let pos = egui::pos2(
            egui::lerp(plot.left()..=plot.right(), 0.75),
            egui::lerp(plot.bottom()..=plot.top(), 0.75),
        );
        interactive_graph_frame(
            &ctx,
            &mut editor,
            &mut view,
            vec![egui::Event::PointerMoved(pos)],
        );
        for pressed in [true, false] {
            interactive_graph_frame(
                &ctx,
                &mut editor,
                &mut view,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        let output = interactive_graph_frame(&ctx, &mut editor, &mut view, vec![]);
        assert!(output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Path(path) if path.stroke.width == 8.0)
        ));
        let button = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Spline segment" => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
                }
                _ => None,
            })
            .unwrap();
        interactive_graph_frame(
            &ctx,
            &mut editor,
            &mut view,
            vec![egui::Event::PointerMoved(button)],
        );
        for pressed in [true, false] {
            interactive_graph_frame(
                &ctx,
                &mut editor,
                &mut view,
                vec![egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(
            editor.curve_segment_type(0, 0),
            Some(CurveSegmentType::Linear)
        );
        assert_eq!(
            editor.curve_segment_type(0, 1),
            Some(CurveSegmentType::Spline)
        );
    }

    #[test]
    fn modifier_badges_only_target_available_actions() {
        let function =
            TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        let mut editor = TagFunctionEditor::from_function(function);
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        let screen = |(x, y): (f32, f32)| egui::pos2(x * 400.0, (1.0 - y) * 200.0);
        let action = |editor: &TagFunctionEditor, value: (f32, f32), modifiers| {
            function_pointer_action(
                editor,
                screen(value),
                value.0,
                (0.0..=1.0).contains(&value.0) && (0.0..=1.0).contains(&value.1),
                screen,
                modifiers,
            )
        };
        assert_eq!(
            action(&editor, (0.5, 0.5), egui::Modifiers::SHIFT),
            Some(FunctionPointerAction::Add { graph: 0, x: 0.5 })
        );
        assert_eq!(action(&editor, (0.5, 0.2), egui::Modifiers::SHIFT), None);
        assert_eq!(action(&editor, (0.0, 0.0), egui::Modifiers::SHIFT), None);
        assert_eq!(action(&editor, (0.0, 0.0), egui::Modifiers::ALT), None);
        let join = insert_function_point(&mut editor, 0, 0.5).unwrap();
        assert_eq!(
            action(&editor, (0.5, 0.5), egui::Modifiers::ALT),
            Some(FunctionPointerAction::Delete {
                graph: 0,
                point: join
            })
        );
        assert_eq!(action(&editor, (0.5, 0.2), egui::Modifiers::ALT), None);
        editor
            .set_curve_segment_type(0, 0, CurveSegmentType::Spline)
            .unwrap();
        let handle = editor.curve_control_point(0, 1).unwrap();
        assert_eq!(action(&editor, handle, egui::Modifiers::ALT), None);
        assert_eq!(action(&editor, handle, egui::Modifiers::SHIFT), None);
        editor.insert_curve_point(0, 0.7).unwrap();
        editor.insert_curve_point(0, 0.9).unwrap();
        assert_eq!(action(&editor, (0.6, 0.6), egui::Modifiers::SHIFT), None);
    }

    fn scalar_editor(encoding: FunctionEncoding) -> TagFunctionEditor {
        let function = if encoding == FunctionEncoding::H2 {
            h2_tag_function(&h2_constant_scalar_function_data(1.0, None)).unwrap()
        } else {
            let mut bytes = vec![0u8; 32];
            bytes[0] = FunctionType::Constant as u8;
            bytes[8..12].copy_from_slice(&1.0f32.to_le_bytes());
            TagFunction::parse(&bytes).unwrap()
        };
        TagFunctionEditor::from_function(function)
    }

    #[test]
    fn h3_bezier_handles_connect_to_endpoints_across_mixed_segments_and_ranges() {
        let mut editor = scalar_editor(FunctionEncoding::Blob);
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        editor.set_ranged(true).unwrap();
        for graph in 0..2 {
            editor.insert_curve_point(graph, 0.3).unwrap();
            editor.insert_curve_point(graph, 0.7).unwrap();
            editor.set_curve_segment_type(graph, 1, CurveSegmentType::Spline).unwrap();
            editor.set_curve_segment_type(graph, 2, CurveSegmentType::Spline2).unwrap();
        }
        editor.set_curve_control_point(1, 2, (0.4, 0.2)).unwrap();
        let ctx = egui::Context::default();
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            draw_foundation_graph(ui, &mut editor, false, &mut 0, &mut 0);
        });
        let plot = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == function_plot_bg() => Some(rect.rect), _ => None,
        }).unwrap();
        for graph in 0..2 {
            for (endpoint, handle) in [(1, 2), (4, 3), (4, 5), (7, 6)] {
                let screen = |point| {
                    let (x, y) = editor.curve_control_point(graph, point).unwrap();
                    egui::pos2(egui::lerp(plot.left()..=plot.right(), x.clamp(0.0, 1.0)),
                        egui::lerp(plot.bottom()..=plot.top(), y.clamp(0.0, 1.0)))
                };
                let expected = [screen(endpoint), screen(handle)];
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::LineSegment { points, stroke }
                        if stroke.color == function_curve_color(graph)
                        && points[0].distance(expected[0]) < 0.01
                        && points[1].distance(expected[1]) < 0.01
                )), "graph {graph}: handle {handle} connects to endpoint {endpoint}");
            }
        }
    }

    #[test]
    fn pasting_periodic_shapes_preserves_destination_output_in_both_directions() {
        for encoding in [FunctionEncoding::H2, FunctionEncoding::Blob] {
            let mut source = scalar_editor(encoding);
            source.set_master_type(EngineMasterType::Periodic).unwrap();
            source.set_ranged(true).unwrap();
            for slot in 0..2 {
                source.set_periodic_params(slot, PeriodicParams {
                    function_index: 2, frequency: 3.0 + slot as f32,
                    phase: 0.2, amplitude_min: 0.1, amplitude_max: 0.9,
                }).unwrap();
            }
            let mut color = scalar_editor(encoding);
            color.set_color_graph_type(ColorGraphType::ThreeColor).unwrap();
            let colors = [0x80112233, 0x40223344, 0xff334455];
            for (index, value) in colors.iter().enumerate() { color.set_color(index, *value).unwrap(); }
            let pasted = pasted_function_shape(source.function(), &color).unwrap();
            assert_eq!(pasted.color_graph_type(), ColorGraphType::ThreeColor);
            assert_eq!(pasted.graph_count(), 2);
            for (index, value) in colors.iter().enumerate() { assert_eq!(pasted.get_color(index), Some(*value)); }
            for slot in 0..2 { assert_eq!(pasted.periodic_params(slot), source.periodic_params(slot)); }
            let mut scalar = scalar_editor(encoding);
            scalar.set_clamp_range(-2.0, 5.0).unwrap();
            let reverse = pasted_function_shape(pasted.function(), &scalar).unwrap();
            assert_eq!(reverse.color_graph_type(), ColorGraphType::Scalar);
            assert_eq!(reverse.clamp_range(), Some((-2.0, 5.0)));
            for slot in 0..2 { assert_eq!(reverse.periodic_params(slot), source.periodic_params(slot)); }
        }
    }

    #[test]
    fn pasting_curves_copies_endpoints_and_handles_without_copying_colors() {
        for encoding in [FunctionEncoding::H2, FunctionEncoding::Blob] {
            let mut source = scalar_editor(encoding);
            if encoding == FunctionEncoding::H2 { source.set_function_type(FunctionType::Spline).unwrap(); }
            else {
                source.set_master_type(EngineMasterType::Curve).unwrap();
                source.set_curve_segment_type(0, 0, CurveSegmentType::Spline).unwrap();
            }
            source.set_ranged(true).unwrap();
            for graph in 0..2 {
                for (point, value) in [(0.0, 0.2), (0.3, 0.9), (0.7, 0.1), (1.0, 0.8)].into_iter().enumerate() {
                    source.set_curve_control_point(graph, point, value).unwrap();
                }
            }
            let mut destination = scalar_editor(encoding);
            destination.set_color_graph_type(ColorGraphType::TwoColor).unwrap();
            destination.set_color(0, 0xff010203).unwrap();
            destination.set_color(1, 0xff040506).unwrap();
            let pasted = pasted_function_shape(source.function(), &destination).unwrap();
            assert_eq!(pasted.function_type(), source.function_type());
            assert_eq!(pasted.get_color(0), Some(0xff010203));
            assert_eq!(pasted.get_color(1), Some(0xff040506));
            for graph in 0..2 {
                assert_eq!(pasted.curve_control_point_count(graph), source.curve_control_point_count(graph));
                for point in 0..source.curve_control_point_count(graph).unwrap() {
                    assert_eq!(pasted.curve_control_point(graph, point), source.curve_control_point(graph, point));
                }
            }
        }
        let source = scalar_editor(FunctionEncoding::H2);
        let destination = scalar_editor(FunctionEncoding::Blob);
        assert!(pasted_function_shape(source.function(), &destination).is_none());
    }

    #[test]
    fn graph_context_menu_copies_read_only_periodic_functions_and_pastes_the_shape() {
        let ctx = egui::Context::default();
        let mut source = scalar_editor(FunctionEncoding::H2);
        source.set_master_type(EngineMasterType::Periodic).unwrap();
        let source_function = source.function().clone();
        let editor = std::cell::RefCell::new(source);
        let editable = std::cell::Cell::new(false);
        let changed = std::cell::Cell::new(false);
        let frame = |events| crate::app::run_ui_test(&ctx, egui::RawInput { events, ..Default::default() }, |ui| {
            changed.set(changed.get() | draw_foundation_graph(ui, &mut editor.borrow_mut(), editable.get(), &mut 0, &mut 0));
        });
        let click = |pos, button| {
            frame(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(vec![egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE }]);
            }
        };
        frame(Vec::new());
        let output = frame(Vec::new());
        let plot = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == function_plot_bg() => Some(rect.rect), _ => None,
        }).unwrap();
        for action in ["Copy", "Paste"] {
            click(plot.center(), egui::PointerButton::Secondary);
            let output = frame(Vec::new());
            let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == action => Some(text.pos + text.galley.size() * 0.5), _ => None,
            }).expect("graph menu contains clipboard actions");
            click(pos, egui::PointerButton::Primary);
            if action == "Copy" {
                assert!(!changed.get());
                let copied = ctx.data(|data| data.get_temp::<TagFunction>(egui::Id::new("function_graph_shape_clipboard"))).unwrap();
                assert_eq!(copied.to_bytes(), source_function.to_bytes());
                let mut target = scalar_editor(FunctionEncoding::H2);
                target.set_color_graph_type(ColorGraphType::TwoColor).unwrap();
                target.set_color(0, 0xff112233).unwrap();
                target.set_color(1, 0xff445566).unwrap();
                *editor.borrow_mut() = target;
                editable.set(true);
                frame(Vec::new());
            }
        }
        assert!(changed.get());
        assert_eq!(editor.borrow().master_type(), EngineMasterType::Periodic);
        assert_eq!(editor.borrow().color_graph_type(), ColorGraphType::TwoColor);
        assert_eq!(editor.borrow().get_color(0), Some(0xff112233));
        assert_eq!(editor.borrow().get_color(1), Some(0xff445566));
    }
}
