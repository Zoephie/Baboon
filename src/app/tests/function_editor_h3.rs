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
