//! Function sampling, gradients, and graph painting.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

/// Editable color swatches for the N populated color slots of a
/// color-graph function. Returns whether any color changed.
pub(super) fn draw_function_color_stop_editors(
    ui: &mut Ui,
    function: &mut TagFunction,
    editable: bool,
) -> bool {
    let slots = color_graph_slots(function.color_graph_type());
    if slots.is_empty() {
        return false;
    }
    let mut changed = false;
    ui.vertical(|ui| {
        // Render high-end color at top (last slot) and low-end at bottom
        // (first slot), matching Guerilla's layout (top = y=1, bottom = y=0).
        for &slot in slots.iter().rev() {
            let argb = function.header().colors[slot];
            let orig_alpha = (argb >> 24) as u8;
            let mut color = color32_from_argb(argb);
            ui.horizontal(|ui| {
                // Swatch: always use color_edit_button so it's clickable even
                // for non-key curve types — color stops are always editable.
                let resp = if editable {
                    ui.color_edit_button_srgba(&mut color)
                } else {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, color);
                    ui.painter()
                        .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
                    resp
                };
                // Hex code label (#RRGGBB)
                ui.label(
                    RichText::new(format!(
                        "#{:02X}{:02X}{:02X}",
                        color.r(),
                        color.g(),
                        color.b()
                    ))
                    .color(subtle_dark())
                    .small()
                    .monospace(),
                );
                if resp.changed() {
                    // Preserve the original alpha byte; Halo function headers
                    // store these as "opaque ARGB" with alpha=0 meaning unused.
                    let new_argb = ((orig_alpha as u32) << 24)
                        | ((color.r() as u32) << 16)
                        | ((color.g() as u32) << 8)
                        | color.b() as u32;
                    function.set_color(slot, new_argb);
                    changed = true;
                }
            });
        }
    });
    changed
}

/// Draw the function curve and, for any editable function, allow
/// dragging the control points. Non-key functions become an editable
/// key curve on the first drag (seeded from their current shape).
/// Returns whether the function changed.
pub(in crate::app) fn draw_function_graph_preview(
    ui: &mut Ui,
    function: &mut TagFunction,
    editable: bool,
    selected_point: &mut usize,
) -> bool {
    let size = Vec2::new(440.0, 190.0);
    let sense = if editable {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let plot = rect.shrink2(Vec2::new(22.0, 18.0));
    let point_screen = |x: f32, y: f32| {
        egui::pos2(
            egui::lerp(plot.left()..=plot.right(), x.clamp(0.0, 1.0)),
            egui::lerp(plot.bottom()..=plot.top(), y.clamp(0.0, 1.0)),
        )
    };

    // --- Interaction first, so handles/line reflect this frame's edit. ---
    // Within HANDLE_HIT pixels of an existing handle: select/drag it.
    // Outside: add a new point (click) or add-and-drag (drag).
    const HANDLE_HIT: f32 = 14.0;

    let mut changed = false;
    if editable {
        // Snapshot handles before any mutation this frame.
        let hit_pts = function_control_points(function);

        let nearest_handle = |pos: egui::Pos2| -> Option<(usize, f32)> {
            hit_pts
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| (i, point_screen(x, y).distance(pos)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        };

        if let Some(pos) = response.interact_pointer_pos() {
            // First frame of a drag gesture.
            if response.drag_started() {
                match nearest_handle(pos) {
                    Some((i, d)) if d < HANDLE_HIT => {
                        *selected_point = i;
                    }
                    _ => {
                        // Empty area drag: convert and insert, then drag it.
                        ensure_editable_curve(function);
                        let nx = egui::remap_clamp(pos.x, plot.left()..=plot.right(), 0.0..=1.0);
                        let ny = egui::remap_clamp(pos.y, plot.bottom()..=plot.top(), 0.0..=1.0);
                        if let Some(idx) = function.insert_linear_key_point(nx, ny) {
                            *selected_point = idx;
                            changed = true;
                        }
                    }
                }
            }

            // Drag in progress: move the selected handle.
            if response.dragged() {
                let n = function.active_linear_key_point_count().max(1);
                *selected_point = (*selected_point).min(n - 1);
                let nx = egui::remap_clamp(pos.x, plot.left()..=plot.right(), 0.0..=1.0);
                let ny = egui::remap_clamp(pos.y, plot.bottom()..=plot.top(), 0.0..=1.0);
                function.set_linear_key_point(*selected_point, nx, ny);
                changed = true;
            }

            // Pure click (no drag): select near handle, or insert a new point.
            if response.clicked() {
                match nearest_handle(pos) {
                    Some((i, d)) if d < HANDLE_HIT => {
                        *selected_point = i;
                    }
                    _ => {
                        ensure_editable_curve(function);
                        let nx = egui::remap_clamp(pos.x, plot.left()..=plot.right(), 0.0..=1.0);
                        let ny = egui::remap_clamp(pos.y, plot.bottom()..=plot.top(), 0.0..=1.0);
                        if let Some(idx) = function.insert_linear_key_point(nx, ny) {
                            *selected_point = idx;
                            changed = true;
                        }
                    }
                }
            }
        }

        // Delete / Backspace while the pointer is over the graph removes
        // the currently selected handle (minimum 2 points kept).
        if response.hovered()
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            let n = function.active_linear_key_point_count();
            let i = (*selected_point).min(n.saturating_sub(1));
            if function.delete_linear_key_point(i) {
                let new_n = function.active_linear_key_point_count();
                if *selected_point >= new_n {
                    *selected_point = new_n.saturating_sub(1);
                }
                changed = true;
            }
        }
    }

    // --- Background, grid, normalized-shape curve. ---
    {
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
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                Stroke::new(1.0, function_grid_line()),
            );
            let y = egui::lerp(plot.bottom()..=plot.top(), i as f32 / 10.0);
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                Stroke::new(1.0, function_grid_line()),
            );
        }
        // Plot the normalized curve SHAPE (0..1), not the output-mapped
        // value — so curves with output ranges outside [0,1] (or
        // inverted, like high=-1/low=0) still show their real shape.
        let samples = (0..=80)
            .map(|i| {
                let x = i as f32 / 80.0;
                let y = function.evaluate_shape(x, x).clamp(0.0, 1.0);
                egui::pos2(
                    egui::lerp(plot.left()..=plot.right(), x),
                    egui::lerp(plot.bottom()..=plot.top(), y),
                )
            })
            .collect::<Vec<_>>();
        painter.add(egui::Shape::line(
            samples,
            Stroke::new(2.0, Color32::from_rgb(54, 132, 58)),
        ));
    }

    // --- Handles (recomputed after any edit). ---
    {
        let control_points = function_control_points(function);
        let painter = ui.painter();
        for (i, (x, y)) in control_points.iter().enumerate() {
            let point = point_screen(*x, *y);
            let selected = editable && i == *selected_point;
            let handle =
                egui::Rect::from_center_size(point, Vec2::splat(if selected { 9.0 } else { 7.0 }));
            painter.rect_filled(
                handle,
                0.0,
                if selected {
                    Color32::from_rgb(120, 220, 120)
                } else {
                    Color32::from_rgb(240, 240, 238)
                },
            );
            painter.rect_stroke(handle, 0.0, Stroke::new(1.0, Color32::BLACK));
        }
        painter.text(
            rect.left_bottom() + Vec2::new(6.0, -4.0),
            Align2::LEFT_BOTTOM,
            "0",
            FontId::proportional(11.0),
            text_dark(),
        );
        painter.text(
            rect.right_top() + Vec2::new(-6.0, 4.0),
            Align2::RIGHT_TOP,
            "1",
            FontId::proportional(11.0),
            text_dark(),
        );
    }

    if function.color_graph_type() != ColorGraphType::Scalar {
        let bar = egui::Rect::from_min_size(
            rect.left_bottom() + Vec2::new(28.0, 10.0),
            Vec2::new(330.0, 24.0),
        );
        draw_function_color_gradient_horizontal(ui.painter(), bar, &function_color_stops(function));
        ui.allocate_space(Vec2::new(0.0, 36.0));
    }

    changed
}

pub(in crate::app) fn function_control_points(function: &TagFunction) -> Vec<(f32, f32)> {
    match function.kind() {
        FunctionKind::LinearKey { .. } | FunctionKind::MultiLinearKey { .. } => {
            // Only return the active (non-padding) points. Trailing slots
            // that are bit-identical to the preceding slot are padding.
            let pts = function.linear_key_points().unwrap();
            let n = function.active_linear_key_point_count();
            pts[..n].to_vec()
        }
        FunctionKind::MultiSpline { compact, .. } => {
            // Expose the segment join points (the visible kinks) so each
            // one can be clicked and inspected.
            let mut result = vec![(0.0_f32, function.evaluate_shape(0.0, 0.0))];
            for part in &compact.parts {
                let x = part.ending_x.clamp(0.0, 1.0);
                result.push((x, function.evaluate_shape(x, x)));
            }
            result
        }
        _ => vec![
            (0.0, function.evaluate_shape(0.0, 0.0)),
            (1.0, function.evaluate_shape(1.0, 1.0)),
        ],
    }
}

/// Convert any non-key function into a 2-point LinearKey curve, seeding
/// the endpoints from the current normalised shape so the curve doesn't
/// visually jump. No-op if it's already a key curve. Slots 2 and 3 are
/// set to bit-identical copies of slot 1 so `active_lk_count` treats
/// them as padding.
pub(in crate::app) fn ensure_editable_curve(function: &mut TagFunction) {
    if function.linear_key_points().is_some() {
        return;
    }
    let y0 = function.evaluate_shape(0.0, 0.0).clamp(0.0, 1.0);
    let y1 = function.evaluate_shape(1.0, 1.0).clamp(0.0, 1.0);
    function.set_function_type(FunctionType::LinearKey);
    function.set_linear_key_point(0, 0.0, y0);
    function.set_linear_key_point(1, 1.0, y1);
    function.set_linear_key_point(2, 1.0, y1); // padding
    function.set_linear_key_point(3, 1.0, y1); // padding
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
    let header = function.header();
    let slots = color_graph_slots(header.color_graph_type);
    let mut stops: Vec<Color32> = slots
        .iter()
        .map(|&i| color32_from_argb(header.colors[i]))
        .collect();
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
    pub(in crate::app) h2_legacy: Option<H2LegacyFunctionView>,
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
    pub(in crate::app) hide_scalar_color_controls: bool,
}
