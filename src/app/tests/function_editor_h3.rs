use super::*;

fn constant_editor() -> TagFunctionEditor {
    let mut bytes = vec![0u8; 32];
    bytes[0] = FunctionType::Constant as u8;
    bytes[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    TagFunctionEditor::parse(&bytes).expect("constant function")
}

#[test]
fn argb_remap_preserves_endpoints_and_interpolates_interior() {
    let stops = [0x0010_2030, 0x8040_6080];
    assert_eq!(sample_argb_stops(&stops, 0.0), stops[0]);
    assert_eq!(sample_argb_stops(&stops, 1.0), stops[1]);
    assert_eq!(sample_argb_stops(&stops, 0.5), 0x4028_4058);
}

#[test]
fn new_editor_curve_and_ranged_compacts_roundtrip() {
    let mut editor = constant_editor();
    editor.set_master_type(EngineMasterType::Curve).unwrap();
    editor.set_ranged(true).unwrap();
    editor.insert_curve_point(0, 0.5).unwrap();
    editor
        .set_curve_segment_type(0, 0, CurveSegmentType::Spline)
        .unwrap();

    let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
    assert_eq!(reparsed.master_type(), EngineMasterType::Curve);
    assert_eq!(reparsed.graph_count(), 2);
    assert_eq!(reparsed.curve_segment_count(0), Some(2));
    assert_eq!(
        reparsed.curve_segment_type(0, 0),
        Some(CurveSegmentType::Spline)
    );
}

#[test]
fn new_editor_periodic_slots_roundtrip_independently() {
    let mut editor = constant_editor();
    editor.set_master_type(EngineMasterType::Periodic).unwrap();
    editor.set_ranged(true).unwrap();
    let first = PeriodicParams {
        function_index: 2,
        frequency: 3.0,
        phase: 0.25,
        amplitude_min: -1.0,
        amplitude_max: 2.0,
    };
    let second = PeriodicParams {
        function_index: 11,
        frequency: 0.5,
        phase: 0.75,
        amplitude_min: 4.0,
        amplitude_max: 8.0,
    };
    editor.set_periodic_params(0, first).unwrap();
    editor.set_periodic_params(1, second).unwrap();

    let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
    assert_eq!(reparsed.periodic_params(0), Some(first));
    assert_eq!(reparsed.periodic_params(1), Some(second));
}

/// One frame of the function editor's graph at the origin, so its plot sits at
/// a known place: the graph allocates 465x225 and plots inside a 30x20 inset.
fn graph_frame(ctx: &egui::Context, editor: &mut TagFunctionEditor, selected: &mut (usize, usize), events: Vec<egui::Event>) {
    let _ = ctx.run(
        egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))),
            ..Default::default()
        },
        |ctx| {
            egui::Area::new(egui::Id::new("function_graph_test"))
                .fixed_pos(egui::Pos2::ZERO)
                .show(ctx, |ui| {
                    draw_foundation_graph(ui, editor, true, &mut selected.0, &mut selected.1);
                });
        },
    );
}

fn graph_screen((x, y): (f32, f32)) -> egui::Pos2 {
    egui::pos2(30.0 + x * 405.0, 205.0 - y * 185.0)
}

/// Press at `from`, move to `to` in steps, release: a user's drag.
fn graph_drag(editor: &mut TagFunctionEditor, from: (f32, f32), to: (f32, f32)) -> (usize, usize) {
    let ctx = egui::Context::default();
    let mut selected = (0, 0);
    let (start, end) = (graph_screen(from), graph_screen(to));
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    graph_frame(&ctx, editor, &mut selected, vec![]);
    graph_frame(&ctx, editor, &mut selected, vec![egui::Event::PointerMoved(start)]);
    graph_frame(&ctx, editor, &mut selected, vec![button(start, true)]);
    for step in 1..=10 {
        let pos = start + (end - start) * (step as f32 / 10.0);
        graph_frame(&ctx, editor, &mut selected, vec![egui::Event::PointerMoved(pos)]);
    }
    graph_frame(&ctx, editor, &mut selected, vec![button(end, false)]);
    graph_frame(&ctx, editor, &mut selected, vec![]);
    selected
}

fn h2_editor(kind: FunctionType) -> TagFunctionEditor {
    TagFunctionEditor::from_function(TagFunction::H2(H2Function::new(kind)))
}

#[test]
fn dragging_an_h2_linear_key_point_holds_it_between_its_neighbours() {
    let mut editor = h2_editor(FunctionType::LinearKey);
    let before = editor.curve_control_point(0, 1).unwrap();
    let selected = graph_drag(&mut editor, before, (0.95, 0.5));

    assert_eq!(selected, (0, 1), "the drag picked the point under the pointer");
    let (x, y) = editor.curve_control_point(0, 1).unwrap();
    assert_eq!(x, editor.curve_control_point(0, 2).unwrap().0, "stopped at the next point");
    assert!((y - 0.5).abs() < 0.01, "y follows the pointer ({y})");
}

#[test]
fn dragging_an_h2_end_point_moves_it_only_vertically() {
    let mut editor = h2_editor(FunctionType::Spline);
    graph_drag(&mut editor, (0.0, 1.0), (0.5, 0.25));
    let (x, y) = editor.curve_control_point(0, 0).unwrap();
    assert_eq!(x, 0.0);
    assert!((y - 0.25).abs() < 0.01, "y follows the pointer ({y})");
}

#[test]
fn a_drag_from_empty_space_moves_no_point() {
    // Before the fix a press away from every point still dragged whatever
    // point was selected (point 0 here).
    let mut editor = h2_editor(FunctionType::Spline);
    let before = editor.to_bytes();
    graph_drag(&mut editor, (0.5, 0.5), (0.9, 0.1));
    assert_eq!(editor.to_bytes(), before);
}

#[test]
fn clicking_empty_graph_space_adds_no_h2_point() {
    let mut editor = h2_editor(FunctionType::LinearKey);
    let before = editor.to_bytes();
    graph_drag(&mut editor, (0.5, 0.1), (0.5, 0.1));
    assert_eq!(editor.curve_control_point_count(0), Some(4));
    assert_eq!(editor.to_bytes(), before, "a click away from every point changes nothing");
}
