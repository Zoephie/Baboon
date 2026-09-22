use super::*;

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

/// An H2 byte-block of `len` bytes with the given header.
fn h2_block(len: usize, function_type: u8, flags: u8, fn1: u8) -> Vec<u8> {
    let mut raw = vec![0; len];
    raw[0] = function_type;
    raw[1] = flags;
    raw[2] = fn1;
    raw
}

fn h2(raw: &[u8]) -> TagFunction {
    h2_tag_function(raw).expect("an H2 block parses")
}

#[test]
fn h2_option_tables_are_the_engines() {
    use blam_tags::tag_function::h2::{
        FUNCTION_TYPES, TRANSITION_FUNCTION_NAMES, color_graph_type_name, function_type_name,
    };
    // Guerilla's picker lists every type, the multi types included.
    assert_eq!(FUNCTION_TYPES.len(), 11);
    assert_eq!(function_type_name(FunctionType::MultiLinearKey), "multi linear key");
    assert_eq!(function_type_name(FunctionType::MultiSpline), "multi spline");
    // The color graph type is the flags' high nibble; there is no "scalar
    // (alpha)" (that was the RANGE bit). One color is "constant".
    assert_eq!(color_graph_type_name(ColorGraphType::Scalar), "scalar (intensity)");
    assert_eq!(color_graph_type_name(ColorGraphType::OneColor), "constant");
    assert_eq!(TRANSITION_FUNCTION_NAMES.len(), 8);
}

#[test]
fn h2_52_byte_periodic_reads_frequency_at_offset_20() {
    let mut raw = h2_block(52, 3, 0, 6);
    raw[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    raw[20..24].copy_from_slice(&0.25f32.to_le_bytes());
    raw[32..36].copy_from_slice(&1.0f32.to_le_bytes());
    let function = h2(&raw);
    let f = function.as_h2().unwrap();
    assert_eq!(f.function_index(0), 6);
    assert_eq!((f.clamp_range_min(), f.clamp_range_max()), (0.0, 1.0));
    assert_eq!(f.periodic_frequency_phase(0), Some((0.25, 0.0)));
    assert_eq!(f.amplitude_range(0), Some((0.0, 1.0)));
    assert_eq!(function.to_bytes(), raw, "reading never rewrites");
}

#[test]
fn h2_two_color_second_color_is_slot_3() {
    let mut raw = h2_block(52, 3, 0x20, 2);
    raw[4..8].copy_from_slice(&[0x10, 0x20, 0x30, 0x40]);
    raw[8..12].copy_from_slice(&[0x50, 0x60, 0x70, 0x80]);
    raw[16..20].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);
    let mut function = h2(&raw);
    assert_eq!(function.color_count(), 2);
    assert_eq!(function.as_h2().unwrap().color(1), Some(0x0403_0201));
    function.as_h2_mut().unwrap().set_color(1, 0x04CC_BBAA).unwrap();
    let data = function.to_bytes();
    assert_eq!(&data[16..20], &[0xAA, 0xBB, 0xCC, 0x04]);
    assert_eq!(&data[4..16], &raw[4..16], "slots 0-2 untouched");
}

#[test]
fn h2_flags_0x40_is_four_color() {
    let mut raw = h2_block(116, 7, 0x40, 4);
    for (slot, bytes) in [[0x10u8, 0x11, 0x12, 0x13], [0x20, 0x21, 0x22, 0x23], [0x30, 0x31, 0x32, 0x33], [0x40, 0x41, 0x42, 0x43]]
        .iter()
        .enumerate()
    {
        raw[4 + 4 * slot..8 + 4 * slot].copy_from_slice(bytes);
    }
    let function = h2(&raw);
    assert_eq!(function.color_graph_type(), ColorGraphType::FourColor);
    assert_eq!(function.as_h2().unwrap().color(3), Some(0x4342_4140));
}

#[test]
fn h2_color_graph_type_and_range_never_touch_each_other() {
    // Luna's report: picking the old "scalar (alpha)" output type set the
    // RANGE bit. The two now live in their own bits.
    let mut function = h2(&h2_block(28, 1, 0, 0));
    let f = function.as_h2_mut().unwrap();
    f.set_color_graph_type(ColorGraphType::TwoColor);
    assert!(!f.is_ranged());
    f.set_ranged(true);
    assert_eq!(f.color_graph_type(), ColorGraphType::TwoColor);
    assert_eq!(function.to_bytes()[1], 0x21);
}

#[test]
fn damage_effect_vibration_is_a_transition_function() {
    let mut raw = h2_block(36, 2, 0, 1);
    raw[20..24].copy_from_slice(&0.8f32.to_le_bytes());
    raw[24..28].copy_from_slice(&0.4f32.to_le_bytes());
    raw[32..36].copy_from_slice(&1.0f32.to_le_bytes());
    let mut view = FunctionView::from_function(h2(&raw));
    let previous = FunctionSnapshot::from_view(&view);
    assert_eq!(view.function.as_h2().unwrap().amplitude_range(0), Some((0.8, 0.4)));

    let f = view.function.as_h2_mut().unwrap();
    f.set_function_index(0, 2).unwrap();
    f.set_amplitude_range(0, 1.0, 0.7).unwrap();
    let paths = foundation_function_edit_paths(
        "player responses[1]/vibration/low frequency vibration/dirty whore/data",
        FunctionEncoding::H2,
    );
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
fn h2_storage_follows_the_encoding_not_the_path() {
    // Any H2 function is a byte-block, whatever its path is called; before,
    // only "vibration" paths were, and the rest were sent as hex to a data
    // field that is not there.
    let paths = foundation_function_edit_paths("particles[0]/emission rate/function/data", FunctionEncoding::H2);
    assert!(matches!(paths.data, FunctionDataStorage::Halo2ByteBlock(_)));
    let paths = foundation_function_edit_paths("particles[0]/emission rate/function/data", FunctionEncoding::Blob);
    assert!(matches!(paths.data, FunctionDataStorage::DataField(_)));
}

#[test]
fn h2_constant_writers_emit_the_h2_encoding() {
    // Fresh constants are H2 blocks the engine reads back as the value, not
    // 32-byte H3 blobs (whose GPU flag the H2 engine reads as two-color).
    let scalar = h2_constant_scalar_function_data(0.75, None);
    assert_eq!(scalar.len(), 28);
    assert_eq!(h2(&scalar).evaluate(0.3, 0.9), 0.75);

    // An existing scalar constant is patched in place: max follows a tied min.
    let mut tied = h2_block(28, 1, 0, 0);
    tied[4..8].copy_from_slice(&0.5f32.to_le_bytes());
    tied[8..12].copy_from_slice(&0.5f32.to_le_bytes());
    tied[20..28].copy_from_slice(&[0xAB; 8]);
    let patched = h2_constant_scalar_function_data(2.0, Some(&tied));
    assert_eq!(&patched[4..12], &[2.0f32.to_le_bytes(), 2.0f32.to_le_bytes()].concat());
    assert_eq!(&patched[20..28], &tied[20..28]);

    let color = h2_constant_color_function_data(1.0, 0.0, 0.0, 1.0, None);
    let function = h2(&color);
    assert_eq!(extract_constant_color(&function), Some([1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn dedicated_picker_updates_h3_function_draft_logical_slot() {
    let mut function = TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap())
        .expect("constant function should parse");
    let blob = function.as_blob_mut().unwrap();
    blob.set_color_graph_type(ColorGraphType::TwoColor);
    blob.set_color(0, 0x0011_2233);
    blob.set_color(3, 0x0044_5566);
    let mut popup = FunctionPopup::new(
        "tag".to_owned(),
        "function".to_owned(),
        FunctionView::from_function(function),
        true,
    );

    popup.apply_draft_color(FunctionDraftColorTarget::Logical(1), 0x00AA_BBCC);

    let header = popup.view.function.as_blob().unwrap().header();
    assert_eq!(header.colors[0], 0x0011_2233);
    assert_eq!(header.colors[3], 0x00AA_BBCC);
}

#[test]
fn dedicated_picker_updates_h2_logical_color() {
    let mut raw = h2_block(52, 3, 0x20, 0);
    raw[4..8].copy_from_slice(&[0x10, 0x20, 0x30, 0x40]);
    raw[16..20].copy_from_slice(&[0x50, 0x60, 0x70, 0x80]);
    let mut popup = FunctionPopup::new("tag".to_owned(), "function".to_owned(), FunctionView::from_function(h2(&raw)), true);

    // What the picker sends on OK: the swatch's original alpha over the new RGB.
    popup.apply_draft_color(FunctionDraftColorTarget::Logical(1), 0x80AA_BBCC);

    let data = popup.view.data_bytes();
    assert_eq!(&data[16..20], &[0xCC, 0xBB, 0xAA, 0x80], "logical 1 is slot 3; alpha kept");
    assert_eq!(&data[4..8], &raw[4..8]);
}
