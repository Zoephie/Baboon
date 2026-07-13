use super::*;

#[test]
fn generic_function_type_combo_offers_every_supported_mapping_function_type() {
    let expected = [
        FunctionType::Identity,
        FunctionType::Constant,
        FunctionType::Transition,
        FunctionType::Periodic,
        FunctionType::Linear,
        FunctionType::LinearKey,
        FunctionType::MultiLinearKey,
        FunctionType::Spline,
        FunctionType::MultiSpline,
        FunctionType::Exponent,
        FunctionType::Spline2,
    ];

    assert_eq!(EDITABLE_FUNCTION_TYPES, expected);
    for kind in expected {
        assert!(
            is_editable_function_type(kind),
            "{kind:?} should be editable"
        );
        assert_eq!(FunctionType::from_byte(kind as u8), Some(kind));
    }
}

#[test]
fn foundation_master_types_keep_all_curve_variants_in_curve_mode() {
    assert_eq!(
        EngineMasterType::from_function_type(FunctionType::Constant),
        EngineMasterType::Basic
    );
    assert_eq!(
        EngineMasterType::from_function_type(FunctionType::Periodic),
        EngineMasterType::Periodic
    );
    assert_eq!(
        EngineMasterType::from_function_type(FunctionType::Exponent),
        EngineMasterType::Exponent
    );
    assert_eq!(
        EngineMasterType::from_function_type(FunctionType::Transition),
        EngineMasterType::Transition
    );
    for kind in [
        FunctionType::Identity,
        FunctionType::Linear,
        FunctionType::LinearKey,
        FunctionType::MultiLinearKey,
        FunctionType::Spline,
        FunctionType::MultiSpline,
        FunctionType::Spline2,
    ] {
        assert_eq!(
            EngineMasterType::from_function_type(kind),
            EngineMasterType::Curve,
            "{kind:?} should retain the curve presentation"
        );
    }
    assert_eq!(
        EngineMasterType::Curve.function_type(),
        FunctionType::MultiSpline
    );
}

#[test]
fn foundation_color_stop_slots_match_engine_header_layout() {
    assert_eq!(color_graph_slots(ColorGraphType::TwoColor), &[0, 3]);
    assert_eq!(color_graph_slots(ColorGraphType::ThreeColor), &[0, 1, 3]);
    assert_eq!(color_graph_slots(ColorGraphType::FourColor), &[0, 1, 2, 3]);
}

#[test]
fn h2_legacy_function_type_options_match_supported_mapping_function_types() {
    assert_eq!(
        H2_FUNCTION_TYPE_OPTIONS.len(),
        EDITABLE_FUNCTION_TYPES.len()
    );
    for (value, _) in H2_FUNCTION_TYPE_OPTIONS {
        let kind = FunctionType::from_byte(value).expect("H2 option should map to function type");
        assert!(
            EDITABLE_FUNCTION_TYPES.contains(&kind),
            "{kind:?} should be available in generic function editor too"
        );
    }
}

#[test]
fn h2_output_type_options_match_guerilla_color_counts() {
    assert_eq!(
        H2_OUTPUT_TYPE_OPTIONS,
        [
            (0, "scalar (intensity)"),
            (1, "scalar (alpha)"),
            (0x20, "2-color"),
            (0x40, "3-color"),
            (0x80, "4-color"),
        ]
    );
    assert_eq!(h2_output_type_label(2), "2-color");
    assert_eq!(h2_output_type_label(3), "3-color");
    assert_eq!(h2_output_type_label(4), "4-color");
}

#[test]
fn h2_legacy_52_byte_periodic_function_reads_frequency_at_offset_20() {
    let mut raw = vec![0; 52];
    raw[0] = 3;
    raw[2] = 6;
    raw[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    raw[20..24].copy_from_slice(&0.25f32.to_le_bytes());
    raw[32..36].copy_from_slice(&1.0f32.to_le_bytes());
    raw[36..40].copy_from_slice(&1.0f32.to_le_bytes());

    let view = H2LegacyFunctionView::parse(raw).expect("legacy function should parse");

    assert_eq!(view.exponent, 6);
    assert_eq!(view.min, 0.0);
    assert_eq!(view.max, 1.0);
    assert_eq!(view.frequency, 0.25);
    assert_eq!(view.phase, 0.0);
    assert_eq!(&view.to_bytes()[20..24], &0.25f32.to_le_bytes());
}

#[test]
fn h2_legacy_color_function_preserves_bgra_endpoints() {
    let mut raw = vec![0; 28];
    raw[0] = 3;
    raw[1] = 0x20;
    raw[2] = 2;
    raw[4..8].copy_from_slice(&[0x10, 0x20, 0x30, 0x40]);
    raw[8..12].copy_from_slice(&[0x50, 0x60, 0x70, 0x80]);
    raw[12..16].copy_from_slice(&0.25f32.to_le_bytes());
    raw[16..20].copy_from_slice(&0.5f32.to_le_bytes());

    let view = H2LegacyFunctionView::parse(raw.clone()).expect("color function should parse");
    let data = view.to_bytes();

    assert!(view.is_color_output());
    assert_eq!(&data[4..12], &raw[4..12]);
    assert_eq!(&data[12..16], &0.25f32.to_le_bytes());
    assert_eq!(&data[16..20], &0.5f32.to_le_bytes());
}

#[test]
fn h2_legacy_four_color_function_preserves_all_color_slots() {
    let mut raw = vec![0; 28];
    raw[0] = 7;
    raw[1] = 0x80;
    raw[4..8].copy_from_slice(&[0x10, 0x11, 0x12, 0x13]);
    raw[8..12].copy_from_slice(&[0x20, 0x21, 0x22, 0x23]);
    raw[12..16].copy_from_slice(&[0x30, 0x31, 0x32, 0x33]);
    raw[16..20].copy_from_slice(&[0x40, 0x41, 0x42, 0x43]);
    let mut view = H2LegacyFunctionView::parse(raw.clone()).expect("color function should parse");

    assert_eq!(view.color_stop_count(), 4);
    assert_eq!(view.color_stop(3), Color32::from_rgb(0x42, 0x41, 0x40));
    view.set_color_stop(2, Color32::from_rgb(0xAA, 0xBB, 0xCC));
    let data = view.to_bytes();

    assert_eq!(&data[4..12], &raw[4..12]);
    assert_eq!(&data[12..16], &[0xCC, 0xBB, 0xAA, 0x33]);
    assert_eq!(&data[16..20], &raw[16..20]);
}

#[test]
fn h2_legacy_color_stop_edit_writes_bgr_and_preserves_alpha() {
    let mut raw = vec![0; 28];
    raw[0] = 3;
    raw[1] = 2;
    raw[4..8].copy_from_slice(&[1, 2, 3, 4]);
    raw[8..12].copy_from_slice(&[5, 6, 7, 8]);
    let mut view = H2LegacyFunctionView::parse(raw).expect("color function should parse");

    assert_eq!(view.color_stop(0), Color32::from_rgb(3, 2, 1));
    view.set_color_stop(1, Color32::from_rgb(0xAA, 0xBB, 0xCC));
    let data = view.to_bytes();

    assert_eq!(&data[8..12], &[0xCC, 0xBB, 0xAA, 8]);
}

#[test]
fn h2_legacy_unset_second_color_displays_as_first_until_edited() {
    let mut raw = vec![0; 28];
    raw[0] = 3;
    raw[1] = 2;
    raw[4..8].copy_from_slice(&[0x00, 0x00, 0xC6, 0x00]);
    let mut view = H2LegacyFunctionView::parse(raw).expect("color function should parse");

    assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
    assert_eq!(view.color_stop(1), Color32::from_rgb(0xC6, 0x00, 0x00));

    view.set_color_stop(1, Color32::from_rgb(0x80, 0x10, 0x20));
    let data = view.to_bytes();

    assert_eq!(&data[4..8], &[0x00, 0x00, 0xC6, 0x00]);
    assert_eq!(&data[8..12], &[0x20, 0x10, 0x80, 0x00]);
}

#[test]
fn h2_legacy_three_and_four_color_show_real_black_unset_slots() {
    let mut raw = vec![0; 28];
    raw[0] = 7;
    raw[1] = 0x40;
    raw[4..8].copy_from_slice(&[0x00, 0x00, 0xC6, 0x00]);
    let mut view = H2LegacyFunctionView::parse(raw.clone()).expect("color function should parse");

    assert_eq!(view.color_stop_count(), 3);
    assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
    assert_eq!(view.color_stop(1), Color32::BLACK);
    assert_eq!(view.color_stop(2), Color32::BLACK);

    view.output_type = 0x80;
    assert_eq!(view.color_stop_count(), 4);
    assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
    assert_eq!(view.color_stop(1), Color32::BLACK);
    assert_eq!(view.color_stop(2), Color32::BLACK);
    assert_eq!(view.color_stop(3), Color32::BLACK);
}

#[test]
fn h2_legacy_color_output_conversion_preserves_endpoints_and_inserts_black() {
    let mut raw = vec![0; 28];
    raw[0] = 7;
    raw[1] = 0x20;
    raw[4..8].copy_from_slice(&[0x00, 0x00, 0xC6, 0x00]);
    let mut view = H2LegacyFunctionView::parse(raw).expect("color function should parse");

    assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
    assert_eq!(view.color_stop(1), Color32::from_rgb(0xC6, 0x00, 0x00));

    view.set_output_type(0x40);
    assert_eq!(view.color_stop_count(), 3);
    assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
    assert_eq!(view.color_stop(1), Color32::BLACK);
    assert_eq!(view.color_stop(2), Color32::from_rgb(0xC6, 0x00, 0x00));
    assert_eq!(
        &view.to_bytes()[4..16],
        &[0x00, 0x00, 0xC6, 0x00, 0, 0, 0, 0, 0x00, 0x00, 0xC6, 0x00]
    );

    view.set_output_type(0x80);
    assert_eq!(view.color_stop_count(), 4);
    assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
    assert_eq!(view.color_stop(1), Color32::BLACK);
    assert_eq!(view.color_stop(2), Color32::BLACK);
    assert_eq!(view.color_stop(3), Color32::from_rgb(0xC6, 0x00, 0x00));
    assert_eq!(
        &view.to_bytes()[4..20],
        &[
            0x00, 0x00, 0xC6, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00, 0xC6, 0x00,
        ]
    );
}

#[test]
fn damage_effect_vibration_function_reads_transition_values_at_observed_offsets() {
    let mut raw = vec![0; 36];
    raw[0] = 2;
    raw[1] = 0;
    raw[2] = 1;
    raw[20..24].copy_from_slice(&0.8f32.to_le_bytes());
    raw[24..28].copy_from_slice(&0.4f32.to_le_bytes());
    raw[32..36].copy_from_slice(&1.0f32.to_le_bytes());

    let view = H2LegacyFunctionView::parse_damage_effect_vibration(raw)
        .expect("damage effect vibration function should parse");

    assert_eq!(view.function_type, 2);
    assert_eq!(view.output_type, 0);
    assert_eq!(view.exponent, 1);
    assert_eq!(view.min, 0.8);
    assert_eq!(view.max, 0.4);
    assert_eq!(&view.to_bytes()[20..24], &0.8f32.to_le_bytes());
    assert_eq!(&view.to_bytes()[24..28], &0.4f32.to_le_bytes());
}

#[test]
fn damage_effect_vibration_edit_emits_byte_block_op() {
    let mut raw = vec![0; 36];
    raw[0] = 2;
    raw[1] = 0;
    raw[2] = 1;
    raw[20..24].copy_from_slice(&0.8f32.to_le_bytes());
    raw[24..28].copy_from_slice(&0.4f32.to_le_bytes());
    raw[32..36].copy_from_slice(&1.0f32.to_le_bytes());
    let h2_legacy = H2LegacyFunctionView::parse_damage_effect_vibration(raw.clone())
        .expect("damage effect vibration function should parse");
    let function = TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap())
        .expect("placeholder function should parse");
    let mut view = FunctionView::from_function(function).with_h2_legacy(h2_legacy);
    let previous = FunctionSnapshot::from_view(&view);
    let h2 = view.h2_legacy.as_mut().unwrap();
    h2.exponent = 2;
    h2.min = 1.0;
    h2.max = 0.7;
    let paths = FunctionEditPaths {
        data: FunctionDataStorage::Halo2ByteBlock(
            "player responses[1]/vibration/low frequency vibration/dirty whore/data".to_owned(),
        ),
        parameter_type: String::new(),
        input_name: String::new(),
        range_name: String::new(),
        time_period: String::new(),
        block_path: String::new(),
        block_index: 0,
    };

    let batch = push_function_edit(&paths, &previous, &view);

    assert!(batch.edits.is_empty());
    assert_eq!(batch.data_ops.len(), 1);
    let data = &batch.data_ops[0].data;
    assert_eq!(data.len(), 36);
    assert_eq!(data[2], 2);
    assert_eq!(&data[20..24], &1.0f32.to_le_bytes());
    assert_eq!(&data[24..28], &0.7f32.to_le_bytes());
    assert_eq!(&data[32..36], &raw[32..36]);
}

#[test]
fn dedicated_picker_updates_h3_function_draft_logical_slot() {
    let mut function = TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap())
        .expect("constant function should parse");
    function.set_color_graph_type(ColorGraphType::TwoColor);
    function.set_color(0, 0x0011_2233);
    function.set_color(3, 0x0044_5566);
    let mut popup = FunctionPopup::new(
        "tag".to_owned(),
        "function".to_owned(),
        FunctionView::from_function(function),
        true,
    );

    popup.apply_draft_color(FunctionDraftColorTarget::H3Logical(1), 0x00AA_BBCC);

    assert_eq!(popup.view.function.header().colors[0], 0x0011_2233);
    assert_eq!(popup.view.function.header().colors[3], 0x00AA_BBCC);
}

#[test]
fn dedicated_picker_updates_h2_function_draft_stop() {
    let mut raw = vec![0u8; 28];
    raw[0] = 3;
    raw[1] = 2;
    raw[4..8].copy_from_slice(&[0x10, 0x20, 0x30, 0x40]);
    raw[8..12].copy_from_slice(&[0x50, 0x60, 0x70, 0x80]);
    let h2 = H2LegacyFunctionView::parse(raw).expect("H2 function should parse");
    let function = TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap())
        .expect("placeholder function should parse");
    let mut popup = FunctionPopup::new(
        "tag".to_owned(),
        "function".to_owned(),
        FunctionView::from_function(function).with_h2_legacy(h2),
        true,
    );

    popup.apply_draft_color(FunctionDraftColorTarget::H2Logical(1), 0x00AA_BBCC);

    let data = popup.view.h2_legacy.as_ref().unwrap().to_bytes();
    assert_eq!(&data[8..12], &[0xCC, 0xBB, 0xAA, 0x80]);
}
