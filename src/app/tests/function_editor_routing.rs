use super::*;

fn constant_view() -> FunctionView {
    let bytes = decode_hex(&constant_function_hex(0.0)).expect("constant function bytes");
    FunctionView::from_function(TagFunction::parse(&bytes).expect("constant function"))
}

#[test]
fn h3_wrapped_mapping_functions_use_foundation_popup() {
    assert!(uses_foundation_function_popup(&constant_view()));
}

#[test]
fn h2_wrapped_mapping_functions_keep_legacy_editor() {
    let mut raw = vec![0; 52];
    raw[0] = FunctionType::Constant as u8;
    raw[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    let legacy = H2LegacyFunctionView::parse(raw).expect("legacy H2 function");
    let view = constant_view().with_h2_legacy(legacy);

    assert!(!uses_foundation_function_popup(&view));
}
