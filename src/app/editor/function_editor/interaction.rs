//! Editing rules shared by pointer interaction and the coordinate fields.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum FunctionGrid {
    Off,
    #[default]
    Ten,
    Twenty,
}

impl FunctionGrid {
    pub(super) fn divisions(self) -> Option<u32> {
        match self {
            Self::Off => None,
            Self::Ten => Some(10),
            Self::Twenty => Some(20),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FunctionGraphView {
    pub(super) zoom_percent: f32,
    pub(super) grid: FunctionGrid,
}

impl Default for FunctionGraphView {
    fn default() -> Self {
        Self {
            zoom_percent: 100.0,
            grid: FunctionGrid::Ten,
        }
    }
}

pub(super) fn set_function_exclusion_bounds(
    editor: &mut TagFunctionEditor,
    min: f32,
    max: f32,
) -> bool {
    if editor.function().as_blob().is_none() || !min.is_finite() || !max.is_finite() {
        return false;
    }
    let mut bytes = editor.to_bytes();
    let min = min.clamp(0.0, 1.0);
    let max = max.clamp(min, 1.0);
    bytes[20..24].copy_from_slice(&min.to_le_bytes());
    bytes[24..28].copy_from_slice(&max.to_le_bytes());
    let Ok(draft) = TagFunctionEditor::parse(&bytes) else {
        return false;
    };
    let changed = bytes != editor.to_bytes();
    *editor = draft;
    changed
}

/// Toggle the wrapper without rebuilding the authored primary graph. The
/// engine supplies the default second graph; existing compact/editor bytes
/// are retained verbatim, including variable-length multipart compacts.
pub(in crate::app) fn set_function_ranged(editor: &mut TagFunctionEditor, ranged: bool) -> bool {
    if editor.is_ranged() == ranged {
        return false;
    }
    let mut draft = editor.clone();
    if draft.set_ranged(ranged).is_err() {
        return false;
    }
    if editor.function().as_h2().is_some()
        || matches!(
            editor.function_type(),
            FunctionType::Constant | FunctionType::Identity
        )
    {
        *editor = draft;
        return true;
    }
    let source = editor.to_bytes();
    let defaults = draft.to_bytes();
    if source[0] != defaults[0] {
        return false;
    }
    let total = i32::from_le_bytes(source[28..32].try_into().unwrap());
    let Ok(total) = usize::try_from(total) else {
        return false;
    };
    let first = if !editor.is_ranged() {
        total
    } else if editor.function_type() == FunctionType::MultiSpline {
        let Some(count_bytes) = source.get(32..36) else {
            return false;
        };
        let count = i32::from_le_bytes(count_bytes.try_into().unwrap());
        if !(1..=4).contains(&count) {
            return false;
        }
        let mut offset = 4;
        for _ in 0..count {
            offset += match source.get(32 + offset) {
                Some(4) => 16,
                Some(7) => 24,
                Some(10) => 36,
                _ => return false,
            };
        }
        offset
    } else {
        total / 2
    };
    let Some(primary) = source.get(32..32 + first) else {
        return false;
    };
    let Some(trailer) = source.get(32 + total..) else {
        return false;
    };
    let primary_trailer = if editor.is_ranged() {
        &trailer[..trailer.len() / 2]
    } else {
        trailer
    };
    let default_total = i32::from_le_bytes(defaults[28..32].try_into().unwrap());
    let Ok(default_total) = usize::try_from(default_total) else {
        return false;
    };
    let secondary_size = if ranged { default_total / 2 } else { 0 };
    let Some(secondary) = defaults.get(32 + secondary_size..32 + secondary_size * 2) else {
        return false;
    };
    let mut bytes = source[..32].to_vec();
    if ranged {
        bytes[1] |= FunctionFlags::RANGE;
    } else {
        bytes[1] &= !FunctionFlags::RANGE;
    }
    bytes[28..32].copy_from_slice(&((first + secondary.len()) as i32).to_le_bytes());
    bytes.extend_from_slice(primary);
    bytes.extend_from_slice(secondary);
    if !primary_trailer.is_empty() {
        bytes[1] &= !FunctionFlags::OPTIMIZED;
        bytes.extend_from_slice(primary_trailer);
        if ranged {
            let Some(default_trailer) = defaults.get(32 + default_total..) else {
                return false;
            };
            bytes.extend_from_slice(&default_trailer[default_trailer.len() / 2..]);
        }
    } else {
        bytes[1] |= FunctionFlags::OPTIMIZED;
    }
    let Ok(mut rebuilt) = TagFunctionEditor::parse(&bytes) else {
        return false;
    };
    if ranged
        && rebuilt.is_cyclic()
        && let Some(count) = rebuilt.curve_control_point_count(1)
        && count >= 2
    {
        let (_, y) = rebuilt.curve_control_point(1, 0).unwrap();
        let (x, _) = rebuilt.curve_control_point(1, count - 1).unwrap();
        if rebuilt
            .set_curve_control_point(1, count - 1, (x, y))
            .is_err()
        {
            return false;
        }
    }
    *editor = rebuilt;
    true
}

pub(super) fn set_function_cyclic(editor: &mut TagFunctionEditor, cyclic: bool) {
    if editor.function().as_blob().is_none() {
        return;
    }
    set_editor_flag(editor, FunctionFlags::CYCLIC, cyclic);
    if cyclic {
        for graph in 0..editor.graph_count() {
            let Some(count) = editor.curve_control_point_count(graph) else {
                continue;
            };
            if count < 2 {
                continue;
            }
            let Some((_, y)) = editor.curve_control_point(graph, 0) else {
                continue;
            };
            let Some((x, _)) = editor.curve_control_point(graph, count - 1) else {
                continue;
            };
            let _ = editor.set_curve_control_point(graph, count - 1, (x, y));
        }
    }
}

/// Index of the segment to the right of an interior on-curve point.
pub(super) fn function_join_segment(
    editor: &TagFunctionEditor,
    graph: usize,
    point: usize,
) -> Option<usize> {
    if !editor.curve_points_are_editable_structure() {
        return None;
    }
    let mut start = 0;
    for segment in 0..editor.curve_segment_count(graph)? {
        if segment > 0 && start == point {
            return Some(segment);
        }
        start += if editor.curve_segment_type(graph, segment)? == CurveSegmentType::Linear {
            1
        } else {
            3
        };
    }
    None
}

/// Merge adjacent curved segments by fitting the original shape across their
/// combined span. Mixed linear/curved joins keep the engine's linear merge.
pub(super) fn delete_function_point(
    editor: &mut TagFunctionEditor,
    graph: usize,
    point: usize,
) -> bool {
    let Some(right_segment) = function_join_segment(editor, graph, point) else {
        return false;
    };
    let left_kind = editor.curve_segment_type(graph, right_segment - 1).unwrap();
    let right_kind = editor.curve_segment_type(graph, right_segment).unwrap();
    let source = editor.function().clone();
    let mut draft = editor.clone();
    if draft.delete_curve_point(graph, point).is_err() {
        return false;
    }
    if left_kind != CurveSegmentType::Linear && right_kind != CurveSegmentType::Linear {
        let start = point - 3;
        let a = editor.curve_control_point(graph, start).unwrap();
        let b = editor.curve_control_point(graph, point + 3).unwrap();
        let kind = if left_kind == right_kind {
            left_kind
        } else {
            CurveSegmentType::Spline
        };
        let Some(points) = fit_function_span(&source, graph, a, b, kind) else {
            return false;
        };
        if draft
            .set_curve_segment_type(graph, right_segment - 1, kind)
            .is_err()
        {
            return false;
        }
        for (local, value) in points.into_iter().enumerate() {
            if draft
                .set_curve_control_point(graph, start + local, value)
                .is_err()
            {
                return false;
            }
        }
        for (new_join, old_join) in [
            (right_segment - 1, right_segment - 1),
            (right_segment, right_segment + 1),
        ] {
            if new_join > 0
                && let Some(mode) = editor.curve_join_mode(graph, old_join)
            {
                if draft.set_curve_join_mode(graph, new_join, mode).is_err() {
                    return false;
                }
            }
        }
    }
    *editor = draft;
    true
}

fn fit_function_span(
    source: &TagFunction,
    graph: usize,
    a: (f32, f32),
    b: (f32, f32),
    kind: CurveSegmentType,
) -> Option<[(f32, f32); 4]> {
    let width = b.0 - a.0;
    if width < 0.0001 {
        return None;
    }
    let (mut aa, mut ab, mut bb, mut ay, mut by) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for sample in 1..32 {
        let t = sample as f32 / 32.0;
        let h0 = t * (1.0 - t) * (1.0 - t);
        let h1 = t * t * (t - 1.0);
        let base =
            (2.0 * t * t * t - 3.0 * t * t + 1.0) * a.1 + (-2.0 * t * t * t + 3.0 * t * t) * b.1;
        let residual = source.evaluate_shape(a.0 + width * t, graph as f32) - base;
        aa += h0 * h0;
        ab += h0 * h1;
        bb += h1 * h1;
        ay += h0 * residual;
        by += h1 * residual;
    }
    let determinant = aa * bb - ab * ab;
    let m0 = (ay * bb - by * ab) / determinant / width;
    let m1 = (by * aa - ay * ab) / determinant / width;
    // Equal handle lengths give Spline2 neutral input bias while retaining
    // the fitted endpoint slopes.
    let length = width / 3.0;
    let dx0 = if kind == CurveSegmentType::Spline2 {
        length / (1.0 + m0 * m0).sqrt()
    } else {
        length
    };
    let dx1 = if kind == CurveSegmentType::Spline2 {
        length / (1.0 + m1 * m1).sqrt()
    } else {
        length
    };
    let points = [
        a,
        (a.0 + dx0, a.1 + dx0 * m0),
        (b.0 - dx1, b.1 - dx1 * m1),
        b,
    ];
    points
        .iter()
        .all(|p| p.0.is_finite() && p.1.is_finite())
        .then_some(points)
}

pub(super) fn function_view_bounds(zoom_percent: f32) -> [f32; 4] {
    let zoom = if zoom_percent.is_finite() {
        zoom_percent.clamp(50.0, 100.0)
    } else {
        100.0
    };
    let half_span = 50.0 / zoom;
    [
        0.5 - half_span,
        0.5 + half_span,
        0.5 - half_span,
        0.5 + half_span,
    ]
}

fn attached_handles(editor: &TagFunctionEditor, graph: usize, point: usize) -> Vec<usize> {
    if !function_point_is_on_curve(editor, graph, point) {
        return Vec::new();
    }
    if editor.function().as_h2().is_some() {
        return if matches!(
            editor.function_type(),
            FunctionType::Spline | FunctionType::Spline2
        ) {
            match point {
                0 => vec![1],
                3 => vec![2],
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
    }
    let mut handles = Vec::new();
    let mut start = 0;
    for segment in 0..editor.curve_segment_count(graph).unwrap_or(0) {
        match editor.curve_segment_type(graph, segment) {
            Some(CurveSegmentType::Spline | CurveSegmentType::Spline2) => {
                if point == start {
                    handles.push(start + 1);
                }
                if point == start + 3 {
                    handles.push(start + 2);
                }
                start += 3;
            }
            _ => start += 1,
        }
    }
    handles
}

#[cfg(test)]
fn move_function_point(
    editor: &mut TagFunctionEditor,
    graph: usize,
    point: usize,
    value: (f32, f32),
    snap: bool,
) -> bool {
    move_function_point_with_modifiers(
        editor,
        graph,
        point,
        value,
        if snap {
            egui::Modifiers::CTRL
        } else {
            egui::Modifiers::NONE
        },
    )
}

pub(super) fn set_function_segment_type(
    editor: &mut TagFunctionEditor,
    graph: usize,
    segment: usize,
    kind: CurveSegmentType,
) -> bool {
    let curved_join = |editor: &TagFunctionEditor, join: usize| {
        join > 0
            && [join - 1, join].into_iter().all(|index| {
                editor
                    .curve_segment_type(graph, index)
                    .is_some_and(|kind| kind != CurveSegmentType::Linear)
            })
    };
    let joins = [segment, segment + 1].map(|join| (join, curved_join(editor, join)));
    let old_kind = editor.curve_segment_type(graph, segment);
    let retained_points = if old_kind.is_some_and(|old| old != CurveSegmentType::Linear)
        && kind != CurveSegmentType::Linear
    {
        let start = (0..segment)
            .map(|index| {
                if editor.curve_segment_type(graph, index) == Some(CurveSegmentType::Linear) {
                    1
                } else {
                    3
                }
            })
            .sum::<usize>();
        Some((
            start,
            (0..4)
                .filter_map(|local| editor.curve_control_point(graph, start + local))
                .collect::<Vec<_>>(),
        ))
    } else {
        None
    };
    let mut draft = editor.clone();
    if draft.set_curve_segment_type(graph, segment, kind).is_err() {
        return false;
    }
    if let Some((start, points)) = retained_points {
        for (local, point) in points.into_iter().enumerate() {
            if draft
                .set_curve_control_point(graph, start + local, point)
                .is_err()
            {
                return false;
            }
        }
    }
    for (join, was_curved) in joins {
        if !was_curved
            && curved_join(&draft, join)
            && draft
                .set_curve_join_mode(graph, join, CurvePointMode::Smooth)
                .is_err()
        {
            return false;
        }
    }
    let changed = draft.to_bytes() != editor.to_bytes();
    *editor = draft;
    changed
}

/// Paired handles belong to a shared spline join. The tag's Smooth/Corner
/// mode persists whether their directions are linked or independent.
fn paired_function_handle(
    editor: &TagFunctionEditor,
    graph: usize,
    point: usize,
) -> Option<(usize, usize, usize)> {
    if function_point_is_on_curve(editor, graph, point) {
        return None;
    }
    for anchor in 0..editor.curve_control_point_count(graph)? {
        let handles = attached_handles(editor, graph, anchor);
        if handles.len() == 2 && handles.contains(&point) {
            let other = handles.into_iter().find(|&handle| handle != point)?;
            return Some((anchor, other, function_join_segment(editor, graph, anchor)?));
        }
    }
    None
}

pub(super) fn function_join_mode(
    editor: &TagFunctionEditor,
    graph: usize,
    segment: usize,
) -> CurvePointMode {
    let mode = editor
        .curve_join_mode(graph, segment)
        .unwrap_or(CurvePointMode::Corner);
    if mode != CurvePointMode::Smooth {
        return mode;
    }
    for anchor in 0..editor.curve_control_point_count(graph).unwrap_or(0) {
        if function_join_segment(editor, graph, anchor) != Some(segment) {
            continue;
        }
        let handles = attached_handles(editor, graph, anchor);
        if handles.len() != 2 {
            return mode;
        }
        let origin = editor.curve_control_point(graph, anchor).unwrap();
        let a = editor.curve_control_point(graph, handles[0]).unwrap();
        let b = editor.curve_control_point(graph, handles[1]).unwrap();
        let a = (a.0 - origin.0, a.1 - origin.1);
        let b = (b.0 - origin.0, b.1 - origin.1);
        let lengths = a.0.hypot(a.1) * b.0.hypot(b.1);
        if lengths > 0.00000001
            && ((a.0 * b.1 - a.1 * b.0).abs() > lengths * 0.001 || a.0 * b.0 + a.1 * b.1 >= 0.0)
        {
            return CurvePointMode::Corner;
        }
    }
    mode
}

pub(super) fn set_function_join_mode(
    editor: &mut TagFunctionEditor,
    graph: usize,
    segment: usize,
    mode: CurvePointMode,
    view: FunctionGraphView,
) -> bool {
    let mut draft = editor.clone();
    if mode == CurvePointMode::Smooth {
        let Some(anchor) = (0..editor.curve_control_point_count(graph).unwrap_or(0))
            .find(|&point| function_join_segment(editor, graph, point) == Some(segment))
        else {
            return false;
        };
        let handles = attached_handles(editor, graph, anchor);
        if handles.len() != 2 {
            return false;
        }
        let origin = editor.curve_control_point(graph, anchor).unwrap();
        let incoming = editor.curve_control_point(graph, handles[0]).unwrap();
        let outgoing = editor.curve_control_point(graph, handles[1]).unwrap();
        let delta = (incoming.0 - origin.0, incoming.1 - origin.1);
        let length = delta.0.hypot(delta.1);
        if length < 0.000001 {
            return false;
        }
        let scale = (outgoing.0 - origin.0).hypot(outgoing.1 - origin.1) / length;
        let target = (origin.0 - delta.0 * scale, origin.1 - delta.1 * scale);
        let bounds = if editor.is_clamped() {
            [0.0, 1.0, 0.0, 1.0]
        } else {
            visible_handle_bounds(view)
        };
        let scale = visible_handle_scale(origin, target, bounds);
        let target = (
            origin.0 + (target.0 - origin.0) * scale,
            origin.1 + (target.1 - origin.1) * scale,
        );
        if target.0 - origin.0 < MINIMUM_TANGENT_RUN - 0.000001 {
            return false;
        }
        if draft
            .set_curve_control_point(graph, handles[1], target)
            .is_err()
        {
            return false;
        }
    }
    if draft.set_curve_join_mode(graph, segment, mode).is_err() {
        return false;
    }
    let changed = draft.to_bytes() != editor.to_bytes();
    *editor = draft;
    changed
}

#[cfg(test)]
pub(super) fn move_function_point_with_modifiers(
    editor: &mut TagFunctionEditor,
    graph: usize,
    point: usize,
    value: (f32, f32),
    modifiers: egui::Modifiers,
) -> bool {
    move_function_point_with_grid(editor, graph, point, value, modifiers, FunctionGrid::Ten)
}

pub(super) fn move_function_point_with_grid(
    editor: &mut TagFunctionEditor,
    graph: usize,
    point: usize,
    mut value: (f32, f32),
    modifiers: egui::Modifiers,
    grid: FunctionGrid,
) -> bool {
    if !value.0.is_finite() || !value.1.is_finite() {
        return false;
    }
    let Some(before) = editor.curve_control_point(graph, point) else {
        return false;
    };
    if modifiers.ctrl
        && let Some(divisions) = grid.divisions()
    {
        let divisions = divisions as f32;
        value = (
            (value.0 * divisions).round() / divisions,
            (value.1 * divisions).round() / divisions,
        );
    }
    if function_point_is_on_curve(editor, graph, point) || editor.is_clamped() {
        value = (value.0.clamp(0.0, 1.0), value.1.clamp(0.0, 1.0));
    }
    if editor.function().as_blob().is_some() && function_point_is_on_curve(editor, graph, point) {
        let count = editor.curve_control_point_count(graph).unwrap_or(0);
        let previous = (0..point)
            .rev()
            .find(|&index| function_point_is_on_curve(editor, graph, index));
        let next =
            (point + 1..count).find(|&index| function_point_is_on_curve(editor, graph, index));
        let left = previous
            .and_then(|index| editor.curve_control_point(graph, index))
            .map_or(0.0, |p| p.0);
        let right = next
            .and_then(|index| editor.curve_control_point(graph, index))
            .map_or(1.0, |p| p.0);
        value.0 = value.0.clamp(left, right.max(left));
    }
    let handles = attached_handles(editor, graph, point)
        .into_iter()
        .filter_map(|handle| {
            editor
                .curve_control_point(graph, handle)
                .map(|value| (handle, value))
        })
        .collect::<Vec<_>>();
    let mut draft = editor.clone();
    if draft.set_curve_control_point(graph, point, value).is_err() {
        return false;
    }
    let Some(after) = draft.curve_control_point(graph, point) else {
        return false;
    };
    if editor.is_cyclic() && function_point_is_on_curve(editor, graph, point) {
        let last = editor
            .curve_control_point_count(graph)
            .unwrap_or(1)
            .saturating_sub(1);
        let opposite = if point == 0 {
            Some(last)
        } else if point == last {
            Some(0)
        } else {
            None
        };
        if let Some(opposite) = opposite
            && opposite != point
        {
            let (x, _) = draft.curve_control_point(graph, opposite).unwrap();
            // Link only endpoint Y; the opposite tangent stays independent.
            if draft
                .set_curve_control_point(graph, opposite, (x, after.1))
                .is_err()
            {
                return false;
            }
        }
    }
    let delta = (after.0 - before.0, after.1 - before.1);
    for (handle, old) in handles {
        if draft
            .set_curve_control_point(graph, handle, (old.0 + delta.0, old.1 + delta.1))
            .is_err()
        {
            return false;
        }
    }
    if let Some((anchor, other, segment)) = paired_function_handle(editor, graph, point) {
        if modifiers.alt
            || (!modifiers.shift
                && function_join_mode(editor, graph, segment) == CurvePointMode::Corner)
        {
            if draft
                .set_curve_join_mode(graph, segment, CurvePointMode::Corner)
                .is_err()
            {
                return false;
            }
        } else if modifiers.shift
            || function_join_mode(editor, graph, segment) == CurvePointMode::Smooth
        {
            let origin = draft.curve_control_point(graph, anchor).unwrap();
            let opposite = editor.curve_control_point(graph, other).unwrap();
            let delta = (after.0 - origin.0, after.1 - origin.1);
            let length = delta.0.hypot(delta.1);
            let opposite_length = (opposite.0 - origin.0).hypot(opposite.1 - origin.1);
            let scale = if modifiers.shift {
                Some(1.0)
            } else if length > 0.000001 {
                Some(opposite_length / length)
            } else {
                None
            };
            if let Some(scale) = scale {
                if draft
                    .set_curve_control_point(
                        graph,
                        other,
                        (origin.0 - delta.0 * scale, origin.1 - delta.1 * scale),
                    )
                    .is_err()
                {
                    return false;
                }
            }
            if modifiers.shift
                && draft
                    .set_curve_join_mode(graph, segment, CurvePointMode::Smooth)
                    .is_err()
            {
                return false;
            }
        }
    }
    let changed = draft.to_bytes() != editor.to_bytes();
    *editor = draft;
    changed
}

const MINIMUM_TANGENT_RUN: f32 = 0.001;

fn handle_anchor(editor: &TagFunctionEditor, graph: usize, handle: usize) -> Option<usize> {
    (0..editor.curve_control_point_count(graph)?)
        .find(|&anchor| attached_handles(editor, graph, anchor).contains(&handle))
}

fn visible_handle_bounds(view: FunctionGraphView) -> [f32; 4] {
    let mut bounds = function_view_bounds(view.zoom_percent);
    // Leave room for the selected handle's 5pt diamond and the frame border.
    let span = bounds[1] - bounds[0];
    let margin = (FUNCTION_PLOT_INSET - Vec2::splat(6.0))
        / (FUNCTION_GRAPH_SIZE - FUNCTION_PLOT_INSET * 2.0)
        * span;
    bounds[0] -= margin.x;
    bounds[1] += margin.x;
    bounds[2] -= margin.y;
    bounds[3] += margin.y;
    bounds
}

fn visible_handle_scale(origin: (f32, f32), value: (f32, f32), bounds: [f32; 4]) -> f32 {
    let mut scale = 1.0_f32;
    for (center, target, min, max) in [
        (origin.0, value.0, bounds[0], bounds[1]),
        (origin.1, value.1, bounds[2], bounds[3]),
    ] {
        let delta = target - center;
        if target > max && delta > 0.0 {
            scale = scale.min((max - center) / delta);
        }
        if target < min && delta < 0.0 {
            scale = scale.min((min - center) / delta);
        }
    }
    scale.clamp(0.0, 1.0)
}

/// Keep every handle moved by this edit visible, including translated and
/// linked counterparts. Shorten tangents along their rays so the boundary
/// does not break their straight-line relationship.
pub(super) fn move_function_point_in_view(
    editor: &mut TagFunctionEditor,
    graph: usize,
    point: usize,
    mut value: (f32, f32),
    modifiers: egui::Modifiers,
    view: FunctionGraphView,
) -> bool {
    if !value.0.is_finite() || !value.1.is_finite() {
        return false;
    }
    let bounds = visible_handle_bounds(view);
    if !function_point_is_on_curve(editor, graph, point) {
        let mut limits = bounds;
        if let Some(anchor) = handle_anchor(editor, graph, point) {
            let origin = editor.curve_control_point(graph, anchor).unwrap();
            // The serialized spline uses dy/dx tangents. Keep incoming and
            // outgoing handles on their own sides, away from a vertical run.
            if point > anchor {
                limits[0] = limits[0].max(origin.0 + MINIMUM_TANGENT_RUN);
            } else {
                limits[1] = limits[1].min(origin.0 - MINIMUM_TANGENT_RUN);
            }
        }
        if modifiers.shift
            && !modifiers.alt
            && let Some((anchor, _, _)) = paired_function_handle(editor, graph, point)
        {
            let origin = editor.curve_control_point(graph, anchor).unwrap();
            limits = [
                limits[0].max(2.0 * origin.0 - bounds[1]),
                limits[1].min(2.0 * origin.0 - bounds[0]),
                limits[2].max(2.0 * origin.1 - bounds[3]),
                limits[3].min(2.0 * origin.1 - bounds[2]),
            ];
        }
        if modifiers.ctrl
            && let Some(divisions) = view.grid.divisions()
        {
            let divisions = divisions as f32;
            limits = [
                (limits[0] * divisions).ceil() / divisions,
                (limits[1] * divisions).floor() / divisions,
                (limits[2] * divisions).ceil() / divisions,
                (limits[3] * divisions).floor() / divisions,
            ];
        }
        if limits[0] > limits[1] || limits[2] > limits[3] {
            return false;
        }
        value = (
            value.0.clamp(limits[0], limits[1]),
            value.1.clamp(limits[2], limits[3]),
        );
    }
    let mut draft = editor.clone();
    move_function_point_with_grid(&mut draft, graph, point, value, modifiers, view.grid);
    if modifiers.shift
        && !modifiers.alt
        && let Some((anchor, other, _)) = paired_function_handle(&draft, graph, point)
    {
        let origin = draft.curve_control_point(graph, anchor).unwrap();
        let selected = draft.curve_control_point(graph, point).unwrap();
        let opposite = draft.curve_control_point(graph, other).unwrap();
        let scale = visible_handle_scale(origin, selected, bounds)
            .min(visible_handle_scale(origin, opposite, bounds));
        for (handle, target) in [(point, selected), (other, opposite)] {
            let target = (
                origin.0 + (target.0 - origin.0) * scale,
                origin.1 + (target.1 - origin.1) * scale,
            );
            if draft
                .set_curve_control_point(graph, handle, target)
                .is_err()
            {
                return false;
            }
        }
    }
    for handle in 0..draft.curve_control_point_count(graph).unwrap_or(0) {
        if function_point_is_on_curve(&draft, graph, handle) {
            continue;
        }
        let Some(target) = draft.curve_control_point(graph, handle) else {
            continue;
        };
        if handle != point && editor.curve_control_point(graph, handle) == Some(target) {
            continue;
        }
        let anchor = (0..draft.curve_control_point_count(graph).unwrap_or(0))
            .find(|&anchor| attached_handles(&draft, graph, anchor).contains(&handle));
        let Some(origin) = anchor.and_then(|anchor| draft.curve_control_point(graph, anchor))
        else {
            continue;
        };
        let scale = visible_handle_scale(origin, target, bounds);
        if scale < 1.0 {
            if draft
                .set_curve_control_point(
                    graph,
                    handle,
                    (
                        origin.0 + (target.0 - origin.0) * scale,
                        origin.1 + (target.1 - origin.1) * scale,
                    ),
                )
                .is_err()
            {
                return false;
            }
        }
    }
    for handle in 0..draft.curve_control_point_count(graph).unwrap_or(0) {
        if function_point_is_on_curve(&draft, graph, handle) {
            continue;
        }
        let Some((x, y)) = draft.curve_control_point(graph, handle) else {
            continue;
        };
        if handle != point && editor.curve_control_point(graph, handle) == Some((x, y)) {
            continue;
        }
        // An imported off-screen anchor can make a linked tangent impossible
        // to keep visible. Reject that edit instead of moving its peer out.
        if x < bounds[0] - 0.000001
            || x > bounds[1] + 0.000001
            || y < bounds[2] - 0.000001
            || y > bounds[3] + 0.000001
        {
            return false;
        }
        if let Some(anchor) = handle_anchor(&draft, graph, handle) {
            let origin = draft.curve_control_point(graph, anchor).unwrap();
            let run = if handle > anchor {
                x - origin.0
            } else {
                origin.0 - x
            };
            if run < MINIMUM_TANGENT_RUN - 0.000001 {
                return false;
            }
        }
    }
    let changed = draft.to_bytes() != editor.to_bytes();
    *editor = draft;
    changed
}

/// Fit two Hermite pieces to the original curve. Ordinary splines are cubic,
/// so subdivision preserves them; Spline2's input bias is approximated by
/// least squares on each half instead of replacing it with straight lines.
pub(super) fn insert_function_point(
    editor: &mut TagFunctionEditor,
    graph: usize,
    x: f32,
) -> Option<usize> {
    if !x.is_finite() || !(0.0..=1.0).contains(&x) {
        return None;
    }
    let mut start = 0;
    let mut found = None;
    for segment in 0..editor.curve_segment_count(graph)? {
        let kind = editor.curve_segment_type(graph, segment)?;
        let end = start
            + if kind == CurveSegmentType::Linear {
                1
            } else {
                3
            };
        let left = editor.curve_control_point(graph, start)?;
        let right = editor.curve_control_point(graph, end)?;
        if x > left.0 && x < right.0 {
            found = Some((segment, start, kind, left, right));
            break;
        }
        start = end;
    }
    let (segment, start, kind, left, right) = found?;
    if x - left.0 < 0.0001 || right.0 - x < 0.0001 {
        return None;
    }
    let source = editor.function().clone();
    let mut draft = editor.clone();
    draft.insert_curve_point(graph, x).ok()?;
    if kind == CurveSegmentType::Linear {
        let inserted = draft.curve_control_point(graph, start + 1)?;
        if !(0.0..=1.0).contains(&inserted.1) {
            return None;
        }
        *editor = draft;
        return Some(start + 1);
    }
    let middle = (x, source.evaluate_shape(x, graph as f32));
    // A graph point cannot be placed beyond the unit grid, even if its
    // unclamped spline overshoots between endpoints.
    if !(0.0..=1.0).contains(&middle.1) {
        return None;
    }
    draft.set_curve_segment_type(graph, segment, kind).ok()?;
    draft
        .set_curve_segment_type(graph, segment + 1, kind)
        .ok()?;
    for (offset, a, b) in [(0, left, middle), (3, middle, right)] {
        for (local, value) in fit_function_span(&source, graph, a, b, kind)?
            .into_iter()
            .enumerate()
        {
            draft
                .set_curve_control_point(graph, start + offset + local, value)
                .ok()?;
        }
    }
    draft
        .set_curve_join_mode(graph, segment + 1, CurvePointMode::Smooth)
        .ok()?;
    *editor = draft;
    Some(start + 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(kind: CurveSegmentType) -> TagFunctionEditor {
        let function =
            TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
        let mut editor = TagFunctionEditor::from_function(function);
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        editor.set_curve_segment_type(0, 0, kind).unwrap();
        editor
    }

    fn near(a: (f32, f32), b: (f32, f32)) {
        assert!(
            (a.0 - b.0).abs() < 0.0001 && (a.1 - b.1).abs() < 0.0001,
            "{a:?} != {b:?}"
        );
    }

    #[test]
    fn tangent_handles_keep_a_minimum_horizontal_run() {
        for kind in [CurveSegmentType::Spline, CurveSegmentType::Spline2] {
            let mut editor = curve(kind);
            let view = FunctionGraphView {
                grid: FunctionGrid::Off,
                ..Default::default()
            };
            for (handle, target, expected) in [(1, (-0.1, 0.8), 0.001), (2, (1.1, 0.2), 0.999)] {
                assert!(move_function_point_in_view(
                    &mut editor,
                    0,
                    handle,
                    target,
                    egui::Modifiers::ALT,
                    view
                ));
                assert!(
                    (editor.curve_control_point(0, handle).unwrap().0 - expected).abs() < 0.000001
                );
            }
            let mut editor = curve(kind);
            let anchor = insert_function_point(&mut editor, 0, 0.5).unwrap();
            let origin = editor.curve_control_point(0, anchor).unwrap();
            for modifiers in [egui::Modifiers::ALT, egui::Modifiers::SHIFT] {
                move_function_point_in_view(
                    &mut editor,
                    0,
                    anchor + 1,
                    (origin.0 - 0.1, origin.1 + 0.1),
                    modifiers,
                    view,
                );
                assert!(
                    editor.curve_control_point(0, anchor + 1).unwrap().0 >= origin.0 + 0.000999
                );
                assert!(
                    editor.curve_control_point(0, anchor - 1).unwrap().0 <= origin.0 - 0.000999
                );
            }
            let saved = TagFunction::parse(&editor.to_bytes()).unwrap();
            for sample in 0..=100 {
                assert!(saved.evaluate_shape(sample as f32 / 100.0, 0.0).is_finite());
            }
        }
    }

    #[test]
    fn tangent_horizontal_limits_respect_grid_snapping() {
        let mut editor = curve(CurveSegmentType::Spline);
        for (handle, target, expected) in [(1, (0.0, 0.8), 0.1), (2, (1.0, 0.2), 0.9)] {
            assert!(move_function_point_in_view(
                &mut editor,
                0,
                handle,
                target,
                egui::Modifiers::CTRL,
                FunctionGraphView::default()
            ));
            assert!((editor.curve_control_point(0, handle).unwrap().0 - expected).abs() < 0.000001);
        }
    }

    #[test]
    fn unclamped_handles_stop_at_visible_bounds_for_current_zoom() {
        for zoom in [50.0, 100.0] {
            let mut editor = curve(CurveSegmentType::Spline);
            set_editor_flag(&mut editor, FunctionFlags::CLAMPED, false);
            let view = FunctionGraphView {
                zoom_percent: zoom,
                grid: FunctionGrid::Off,
            };
            let bounds = visible_handle_bounds(view);
            assert!(move_function_point_in_view(
                &mut editor,
                0,
                2,
                (-5.0, 5.0),
                egui::Modifiers::ALT,
                view
            ));
            let value = editor.curve_control_point(0, 2).unwrap();
            near(value, (bounds[0], bounds[3]));
            assert!(
                value.0 < 0.0 && value.1 > 1.0,
                "padding/outside-grid space remains usable"
            );
        }
    }

    #[test]
    fn mirrored_handles_and_point_translations_cannot_push_handles_outside_view() {
        let mut editor = curve(CurveSegmentType::Spline);
        let anchor = insert_function_point(&mut editor, 0, 0.9).unwrap();
        set_editor_flag(&mut editor, FunctionFlags::CLAMPED, false);
        let view = FunctionGraphView::default();
        let bounds = visible_handle_bounds(view);
        move_function_point_in_view(
            &mut editor,
            0,
            anchor - 1,
            (0.2, 0.2),
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            view,
        );
        let a = editor.curve_control_point(0, anchor - 1).unwrap();
        let b = editor.curve_control_point(0, anchor + 1).unwrap();
        let origin = editor.curve_control_point(0, anchor).unwrap();
        near((a.0 + b.0, a.1 + b.1), (2.0 * origin.0, 2.0 * origin.1));
        assert!((a.0 * 10.0 - (a.0 * 10.0).round()).abs() < 0.00001);
        move_function_point_in_view(
            &mut editor,
            0,
            anchor,
            (0.9, 1.0),
            egui::Modifiers::NONE,
            view,
        );
        for point in 0..editor.curve_control_point_count(0).unwrap() {
            if function_point_is_on_curve(&editor, 0, point) {
                continue;
            }
            let (x, y) = editor.curve_control_point(0, point).unwrap();
            assert!(
                x >= bounds[0] - 0.000001
                    && x <= bounds[1] + 0.000001
                    && y >= bounds[2] - 0.000001
                    && y <= bounds[3] + 0.000001,
                "{x}, {y}"
            );
        }
    }

    #[test]
    fn an_imported_offscreen_anchor_cannot_push_a_linked_handle_out_of_view() {
        let mut editor = curve(CurveSegmentType::Spline);
        let anchor = insert_function_point(&mut editor, 0, 0.5).unwrap();
        editor
            .set_curve_control_point(0, anchor, (0.5, 2.0))
            .unwrap();
        editor
            .set_curve_control_point(0, anchor - 1, (0.4, 0.5))
            .unwrap();
        editor
            .set_curve_control_point(0, anchor + 1, (0.6, 0.5))
            .unwrap();
        let before = editor.to_bytes();
        assert!(!move_function_point_in_view(
            &mut editor,
            0,
            anchor + 1,
            (0.7, 0.7),
            egui::Modifiers::SHIFT,
            FunctionGraphView::default()
        ));
        assert_eq!(editor.to_bytes(), before);
    }

    #[test]
    fn changing_spline_variants_preserves_all_handles_and_join_modes() {
        let mut editor = curve(CurveSegmentType::Spline);
        editor.set_ranged(true).unwrap();
        editor.insert_curve_point(0, 0.4).unwrap();
        set_function_segment_type(&mut editor, 0, 0, CurveSegmentType::Spline);
        set_function_segment_type(&mut editor, 0, 1, CurveSegmentType::Spline);
        editor.set_curve_control_point(0, 1, (0.12, 0.63)).unwrap();
        editor.set_curve_control_point(0, 2, (0.27, 0.17)).unwrap();
        editor.set_curve_control_point(0, 4, (0.6, 0.85)).unwrap();
        editor.set_curve_control_point(0, 5, (0.8, 0.3)).unwrap();
        editor
            .set_curve_join_mode(0, 1, CurvePointMode::Corner)
            .unwrap();
        let points = (0..7)
            .map(|point| editor.curve_control_point(0, point).unwrap())
            .collect::<Vec<_>>();
        for target in [CurveSegmentType::Spline2, CurveSegmentType::Spline] {
            assert!(set_function_segment_type(&mut editor, 0, 0, target));
            assert!(set_function_segment_type(&mut editor, 0, 1, target));
            let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
            assert_eq!(reparsed.curve_join_mode(0, 1), Some(CurvePointMode::Corner));
            for (point, value) in points.iter().enumerate() {
                near(reparsed.curve_control_point(0, point).unwrap(), *value);
            }
        }
    }

    #[test]
    fn toggling_range_preserves_the_primary_curve_and_adds_only_default_range_data() {
        let mut editor = curve(CurveSegmentType::Spline2);
        insert_function_point(&mut editor, 0, 0.35).unwrap();
        editor.set_curve_control_point(0, 1, (0.1, 0.7)).unwrap();
        editor
            .set_curve_join_mode(0, 1, CurvePointMode::Corner)
            .unwrap();
        editor
            .set_color_graph_type(ColorGraphType::ThreeColor)
            .unwrap();
        editor.set_color(0, 0x00FF_0000).unwrap();
        let before = editor.clone();
        assert!(set_function_ranged(&mut editor, true));
        assert_eq!(editor.curve_segment_count(1), Some(1));
        near(editor.curve_control_point(1, 0).unwrap(), (0.0, 0.0));
        near(editor.curve_control_point(1, 1).unwrap(), (1.0, 1.0));
        for point in 0..before.curve_control_point_count(0).unwrap() {
            near(
                editor.curve_control_point(0, point).unwrap(),
                before.curve_control_point(0, point).unwrap(),
            );
        }
        assert_eq!(editor.curve_join_mode(0, 1), Some(CurvePointMode::Corner));
        for step in 0..=100 {
            let x = step as f32 / 100.0;
            assert!(
                (editor.function().evaluate_shape(x, 0.0)
                    - before.function().evaluate_shape(x, 0.0))
                .abs()
                    < 0.000001
            );
        }
        assert!(set_function_ranged(&mut editor, false));
        assert_eq!(editor.to_bytes(), before.to_bytes());
    }

    #[test]
    fn toggling_range_preserves_typed_parameters() {
        let mut editor = curve(CurveSegmentType::Linear);
        editor.set_master_type(EngineMasterType::Periodic).unwrap();
        let params = PeriodicParams {
            function_index: 2,
            frequency: 3.5,
            phase: 0.4,
            amplitude_min: 0.2,
            amplitude_max: 0.8,
        };
        editor.set_periodic_params(0, params).unwrap();
        let before = editor.to_bytes();
        assert!(set_function_ranged(&mut editor, true));
        assert_eq!(editor.periodic_params(0), Some(params));
        assert_eq!(editor.periodic_params(1).unwrap().frequency, 1.0);
        assert!(set_function_ranged(&mut editor, false));
        assert_eq!(editor.to_bytes(), before);
    }

    #[test]
    fn cyclic_links_only_endpoint_y_and_roundtrips() {
        let mut editor = curve(CurveSegmentType::Spline);
        let handles = [
            editor.curve_control_point(0, 1).unwrap(),
            editor.curve_control_point(0, 2).unwrap(),
        ];
        set_function_cyclic(&mut editor, true);
        near(editor.curve_control_point(0, 3).unwrap(), (1.0, 0.0));
        near(editor.curve_control_point(0, 1).unwrap(), handles[0]);
        near(editor.curve_control_point(0, 2).unwrap(), handles[1]);
        move_function_point_with_modifiers(&mut editor, 0, 0, (0.0, 0.4), egui::Modifiers::NONE);
        near(editor.curve_control_point(0, 3).unwrap(), (1.0, 0.4));
        near(editor.curve_control_point(0, 2).unwrap(), handles[1]);
        let left_handle = editor.curve_control_point(0, 1).unwrap();
        move_function_point_with_modifiers(&mut editor, 0, 3, (1.0, 0.2), egui::Modifiers::NONE);
        near(editor.curve_control_point(0, 0).unwrap(), (0.0, 0.2));
        near(editor.curve_control_point(0, 1).unwrap(), left_handle);
        let mut reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
        assert!(reparsed.is_cyclic());
        set_function_cyclic(&mut reparsed, false);
        move_function_point_with_modifiers(&mut reparsed, 0, 0, (0.0, 0.7), egui::Modifiers::NONE);
        near(reparsed.curve_control_point(0, 3).unwrap(), (1.0, 0.2));
    }

    #[test]
    fn adding_range_to_a_cyclic_curve_links_only_the_new_range_endpoints() {
        let mut editor = curve(CurveSegmentType::Spline);
        set_function_cyclic(&mut editor, true);
        let primary = (0..4)
            .map(|point| editor.curve_control_point(0, point).unwrap())
            .collect::<Vec<_>>();
        assert!(set_function_ranged(&mut editor, true));
        assert_eq!(
            editor.curve_control_point(1, 0).unwrap().1,
            editor.curve_control_point(1, 1).unwrap().1
        );
        for (point, value) in primary.into_iter().enumerate() {
            near(editor.curve_control_point(0, point).unwrap(), value);
        }
    }

    #[test]
    fn exclusion_bounds_roundtrip_without_changing_curve_data() {
        let mut editor = curve(CurveSegmentType::Spline);
        set_editor_flag(&mut editor, FunctionFlags::EXCLUSION, true);
        let before = editor.to_bytes();
        assert!(set_function_exclusion_bounds(&mut editor, 0.2, 0.7));
        let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
        let blob = reparsed.function().as_blob().unwrap();
        assert_eq!((blob.exclusion_min(), blob.exclusion_max()), (0.2, 0.7));
        assert_eq!(&before[32..], &editor.to_bytes()[32..]);
        assert!(set_function_exclusion_bounds(&mut editor, -0.4, 1.5));
        let blob = editor.function().as_blob().unwrap();
        assert_eq!((blob.exclusion_min(), blob.exclusion_max()), (0.0, 1.0));
    }

    #[test]
    fn zoom_bounds_are_centered_limited_and_independent_of_curve_extents() {
        assert_eq!(function_view_bounds(100.0), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(function_view_bounds(50.0), [-0.5, 1.5, -0.5, 1.5]);
        assert_eq!(function_view_bounds(25.0), function_view_bounds(50.0));
        assert_eq!(function_view_bounds(150.0), function_view_bounds(100.0));
        assert_eq!(function_view_bounds(f32::NAN), function_view_bounds(100.0));
        let bounds = function_view_bounds(75.0);
        assert!((bounds[0] + bounds[1] - 1.0).abs() < 0.000001);
        assert!((bounds[2] + bounds[3] - 1.0).abs() < 0.000001);
    }

    #[test]
    fn snapping_uses_the_selected_grid_and_off_disables_it() {
        for (grid, expected) in [
            (FunctionGrid::Off, (0.26, 0.44)),
            (FunctionGrid::Ten, (0.3, 0.4)),
            (FunctionGrid::Twenty, (0.25, 0.45)),
        ] {
            let mut editor = curve(CurveSegmentType::Spline);
            assert!(move_function_point_with_grid(
                &mut editor,
                0,
                1,
                (0.26, 0.44),
                egui::Modifiers::CTRL,
                grid
            ));
            near(editor.curve_control_point(0, 1).unwrap(), expected);
        }
    }

    #[test]
    fn mismatched_handles_stay_detached_until_shift_or_smooth_rejoins_them() {
        for kind in [CurveSegmentType::Spline, CurveSegmentType::Spline2] {
            let mut editor = curve(kind);
            let anchor = insert_function_point(&mut editor, 0, 0.5).unwrap();
            let origin = editor.curve_control_point(0, anchor).unwrap();
            let incoming = (origin.0 - 0.2, origin.1 + 0.1);
            editor
                .set_curve_control_point(0, anchor - 1, incoming)
                .unwrap();
            editor
                .set_curve_control_point(0, anchor + 1, (origin.0 + 0.2, origin.1 + 0.1))
                .unwrap();
            editor
                .set_curve_join_mode(0, 1, CurvePointMode::Smooth)
                .unwrap();
            assert_eq!(function_join_mode(&editor, 0, 1), CurvePointMode::Corner);
            move_function_point_with_modifiers(
                &mut editor,
                0,
                anchor + 1,
                (origin.0 + 0.25, origin.1 - 0.2),
                egui::Modifiers::NONE,
            );
            near(editor.curve_control_point(0, anchor - 1).unwrap(), incoming);
            let mut editor = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
            assert_eq!(editor.curve_join_mode(0, 1), Some(CurvePointMode::Corner));
            let mut shifted = editor.clone();
            move_function_point_with_modifiers(
                &mut shifted,
                0,
                anchor + 1,
                (origin.0 + 0.2, origin.1 - 0.1),
                egui::Modifiers::SHIFT,
            );
            assert_eq!(function_join_mode(&shifted, 0, 1), CurvePointMode::Smooth);
            assert!(set_function_join_mode(
                &mut editor,
                0,
                1,
                CurvePointMode::Smooth,
                FunctionGraphView::default()
            ));
            near(editor.curve_control_point(0, anchor - 1).unwrap(), incoming);
            assert_eq!(function_join_mode(&editor, 0, 1), CurvePointMode::Smooth);
        }
    }

    #[test]
    fn linked_handles_keep_opposite_length_and_alt_detachment_persists_until_shift_reconnects() {
        let mut editor = curve(CurveSegmentType::Spline);
        let anchor = insert_function_point(&mut editor, 0, 0.5).unwrap();
        let incoming = anchor - 1;
        let outgoing = anchor + 1;
        let origin = editor.curve_control_point(0, anchor).unwrap();
        let original_other = editor.curve_control_point(0, incoming).unwrap();
        let old_length = (original_other.0 - origin.0).hypot(original_other.1 - origin.1);
        assert!(move_function_point_with_modifiers(
            &mut editor,
            0,
            outgoing,
            (0.75, 0.6),
            egui::Modifiers::NONE
        ));
        let other = editor.curve_control_point(0, incoming).unwrap();
        let vector = (0.25_f32, 0.1_f32);
        near(
            other,
            (
                origin.0 - vector.0 * old_length / vector.0.hypot(vector.1),
                origin.1 - vector.1 * old_length / vector.0.hypot(vector.1),
            ),
        );
        assert!(move_function_point_with_modifiers(
            &mut editor,
            0,
            outgoing,
            (0.7, 0.8),
            egui::Modifiers::ALT
        ));
        near(editor.curve_control_point(0, incoming).unwrap(), other);
        let mut reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
        assert_eq!(reparsed.curve_join_mode(0, 1), Some(CurvePointMode::Corner));
        move_function_point_with_modifiers(
            &mut reparsed,
            0,
            outgoing,
            (0.65, 0.7),
            egui::Modifiers::NONE,
        );
        near(reparsed.curve_control_point(0, incoming).unwrap(), other);
        move_function_point_with_modifiers(
            &mut reparsed,
            0,
            outgoing,
            (0.68, 0.62),
            egui::Modifiers::SHIFT,
        );
        near(
            reparsed.curve_control_point(0, incoming).unwrap(),
            (0.32, 0.38),
        );
        let mut reconnected = TagFunctionEditor::parse(&reparsed.to_bytes()).unwrap();
        assert_eq!(
            reconnected.curve_join_mode(0, 1),
            Some(CurvePointMode::Smooth)
        );
        // Ordinary movement now links directions again, from either handle.
        move_function_point_with_modifiers(
            &mut reconnected,
            0,
            incoming,
            (0.3, 0.4),
            egui::Modifiers::NONE,
        );
        let outgoing_value = reconnected.curve_control_point(0, outgoing).unwrap();
        let length = 0.18_f32.hypot(0.12);
        near(
            outgoing_value,
            (
                0.5 + 0.2 * length / 0.2_f32.hypot(0.1),
                0.5 + 0.1 * length / 0.2_f32.hypot(0.1),
            ),
        );
    }

    #[test]
    fn new_paired_segments_default_to_smooth_without_resetting_detached_pairs() {
        let mut editor = curve(CurveSegmentType::Linear);
        let join = insert_function_point(&mut editor, 0, 0.5).unwrap();
        assert_eq!(join, 1);
        set_function_segment_type(&mut editor, 0, 0, CurveSegmentType::Spline);
        set_function_segment_type(&mut editor, 0, 1, CurveSegmentType::Spline2);
        assert_eq!(editor.curve_join_mode(0, 1), Some(CurvePointMode::Smooth));
        move_function_point_with_modifiers(&mut editor, 0, 2, (0.3, 0.2), egui::Modifiers::ALT);
        set_function_segment_type(&mut editor, 0, 0, CurveSegmentType::Spline2);
        assert_eq!(editor.curve_join_mode(0, 1), Some(CurvePointMode::Corner));
    }

    #[test]
    fn zero_length_handle_and_unpaired_halo2_handles_remain_valid() {
        let mut editor = curve(CurveSegmentType::Spline);
        let anchor = insert_function_point(&mut editor, 0, 0.5).unwrap();
        let other = editor.curve_control_point(0, anchor - 1).unwrap();
        move_function_point_with_modifiers(
            &mut editor,
            0,
            anchor + 1,
            (0.5, 0.5),
            egui::Modifiers::NONE,
        );
        near(editor.curve_control_point(0, anchor - 1).unwrap(), other);
        move_function_point_with_modifiers(
            &mut editor,
            0,
            anchor + 1,
            (0.5, 0.5),
            egui::Modifiers::SHIFT,
        );
        near(
            editor.curve_control_point(0, anchor - 1).unwrap(),
            (0.5, 0.5),
        );
        assert!(TagFunctionEditor::parse(&editor.to_bytes()).is_ok());
        let mut h2 = TagFunctionEditor::from_function(TagFunction::H2(H2Function::new(
            FunctionType::Spline,
        )));
        let other = h2.curve_control_point(0, 2).unwrap();
        move_function_point_with_modifiers(&mut h2, 0, 1, (0.25, 0.8), egui::Modifiers::SHIFT);
        near(h2.curve_control_point(0, 2).unwrap(), other);
    }

    #[test]
    fn deleting_spline_joins_preserves_curved_types_shape_and_other_segments() {
        for kind in [CurveSegmentType::Spline, CurveSegmentType::Spline2] {
            let mut editor = curve(CurveSegmentType::Linear);
            editor.set_ranged(true).unwrap();
            editor.insert_curve_point(0, 0.8).unwrap();
            editor.set_curve_segment_type(0, 0, kind).unwrap();
            editor.set_curve_control_point(0, 1, (0.2, 0.0)).unwrap();
            editor.set_curve_control_point(0, 2, (0.6, 0.8)).unwrap();
            let join = insert_function_point(&mut editor, 0, 0.4).unwrap();
            let before = editor.function().clone();
            assert!(delete_function_point(&mut editor, 0, join));
            let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
            assert_eq!(reparsed.curve_segment_count(0), Some(2));
            assert_eq!(reparsed.curve_segment_type(0, 0), Some(kind));
            assert_eq!(
                reparsed.curve_segment_type(0, 1),
                Some(CurveSegmentType::Linear)
            );
            near(reparsed.curve_control_point(0, 3).unwrap(), (0.8, 0.8));
            near(reparsed.curve_control_point(0, 4).unwrap(), (1.0, 1.0));
            for step in 0..=100 {
                let x = step as f32 / 100.0;
                assert!(
                    (before.evaluate_shape(x, 0.0) - reparsed.function().evaluate_shape(x, 0.0))
                        .abs()
                        < 0.025
                );
                assert!(
                    (before.evaluate_shape(x, 1.0) - reparsed.function().evaluate_shape(x, 1.0))
                        .abs()
                        < 0.000001
                );
            }
        }
    }

    #[test]
    fn deleting_mixed_spline_kinds_stays_curved_and_endpoints_are_protected() {
        let mut editor = curve(CurveSegmentType::Spline);
        let join = insert_function_point(&mut editor, 0, 0.5).unwrap();
        editor
            .set_curve_segment_type(0, 1, CurveSegmentType::Spline2)
            .unwrap();
        let before = editor.to_bytes();
        assert!(!delete_function_point(&mut editor, 0, 0));
        assert!(!delete_function_point(&mut editor, 0, 6));
        assert!(!delete_function_point(&mut editor, 0, 1));
        assert_eq!(editor.to_bytes(), before);
        assert!(delete_function_point(&mut editor, 0, join));
        assert_eq!(
            editor.curve_segment_type(0, 0),
            Some(CurveSegmentType::Spline)
        );
    }

    #[test]
    fn endpoint_moves_translate_both_attached_handles_and_roundtrip() {
        let mut editor = curve(CurveSegmentType::Spline);
        let join = insert_function_point(&mut editor, 0, 0.5).unwrap();
        let before =
            [join - 1, join + 1].map(|point| editor.curve_control_point(0, point).unwrap());
        assert!(move_function_point(&mut editor, 0, join, (0.6, 0.7), false));
        let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
        for (point, old) in [join - 1, join + 1].into_iter().zip(before) {
            near(
                reparsed.curve_control_point(0, point).unwrap(),
                (old.0 + 0.1, old.1 + 0.2),
            );
        }
    }

    #[test]
    fn halo2_handles_follow_actual_endpoint_motion_with_fixed_x() {
        for kind in [FunctionType::Spline, FunctionType::Spline2] {
            let mut editor =
                TagFunctionEditor::from_function(TagFunction::H2(H2Function::new(kind)));
            let old = editor.curve_control_point(0, 1).unwrap();
            let endpoint = editor.curve_control_point(0, 0).unwrap();
            assert!(move_function_point(
                &mut editor,
                0,
                0,
                (0.5, endpoint.1 - 0.5),
                false
            ));
            near(
                editor.curve_control_point(0, 1).unwrap(),
                (old.0, old.1 - 0.5),
            );
        }
    }

    #[test]
    fn graph_points_are_bounded_but_unclamped_handles_and_view_are_not() {
        let mut editor = curve(CurveSegmentType::Spline);
        set_editor_flag(&mut editor, FunctionFlags::CLAMPED, false);
        move_function_point(&mut editor, 0, 0, (-2.0, 2.0), false);
        near(editor.curve_control_point(0, 0).unwrap(), (0.0, 1.0));
        assert!(move_function_point(&mut editor, 0, 1, (-0.3, 1.7), false));
        near(editor.curve_control_point(0, 1).unwrap(), (-0.3, 1.7));
        assert_eq!(function_view_bounds(100.0), [0.0, 1.0, 0.0, 1.0]);
        set_editor_flag(&mut editor, FunctionFlags::CLAMPED, true);
        assert_eq!(function_view_bounds(100.0), [0.0, 1.0, 0.0, 1.0]);
        move_function_point(&mut editor, 0, 1, (-0.4, 1.8), false);
        near(editor.curve_control_point(0, 1).unwrap(), (0.0, 1.0));
    }

    #[test]
    fn snapping_uses_unit_grid_intersections_for_points_and_handles() {
        let mut editor = curve(CurveSegmentType::Spline);
        move_function_point(&mut editor, 0, 1, (0.26, 0.44), true);
        near(editor.curve_control_point(0, 1).unwrap(), (0.3, 0.4));
        let join = insert_function_point(&mut editor, 0, 0.5).unwrap();
        move_function_point(&mut editor, 0, join, (0.57, 0.64), true);
        near(editor.curve_control_point(0, join).unwrap(), (0.6, 0.6));
    }

    #[test]
    fn inserting_into_an_existing_out_of_bounds_curve_is_rejected_without_changes() {
        for kind in [
            CurveSegmentType::Linear,
            CurveSegmentType::Spline,
            CurveSegmentType::Spline2,
        ] {
            let mut editor = curve(kind);
            let count = editor.curve_control_point_count(0).unwrap();
            for point in 0..count {
                let (x, _) = editor.curve_control_point(0, point).unwrap();
                editor.set_curve_control_point(0, point, (x, 2.0)).unwrap();
            }
            let before = editor.to_bytes();
            assert_eq!(insert_function_point(&mut editor, 0, 0.5), None);
            assert_eq!(editor.to_bytes(), before);
        }
    }

    #[test]
    fn splitting_splines_preserves_shape_types_other_segments_and_ranged_graph() {
        for kind in [CurveSegmentType::Spline, CurveSegmentType::Spline2] {
            let mut editor = curve(CurveSegmentType::Linear);
            editor.set_ranged(true).unwrap();
            editor.insert_curve_point(0, 0.8).unwrap();
            editor.set_curve_segment_type(0, 0, kind).unwrap();
            editor.set_curve_control_point(0, 0, (0.0, 0.1)).unwrap();
            editor.set_curve_control_point(0, 1, (0.25, 0.1)).unwrap();
            editor
                .set_curve_control_point(
                    0,
                    2,
                    (
                        if kind == CurveSegmentType::Spline2 {
                            0.65
                        } else {
                            0.55
                        },
                        0.8,
                    ),
                )
                .unwrap();
            let before = editor.function().clone();
            assert_eq!(insert_function_point(&mut editor, 0, 0.37), Some(3));
            let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
            assert_eq!(reparsed.curve_segment_type(0, 0), Some(kind));
            assert_eq!(reparsed.curve_segment_type(0, 1), Some(kind));
            assert_eq!(
                reparsed.curve_segment_type(0, 2),
                Some(CurveSegmentType::Linear)
            );
            for sample in 0..=100 {
                let x = sample as f32 / 100.0;
                let error = (before.evaluate_shape(x, 0.0)
                    - reparsed.function().evaluate_shape(x, 0.0))
                .abs();
                assert!(error < 0.025, "{kind:?}: error {error} at {x}");
                assert!(
                    (before.evaluate_shape(x, 1.0) - reparsed.function().evaluate_shape(x, 1.0))
                        .abs()
                        < 0.000001
                );
            }
        }
    }
}
