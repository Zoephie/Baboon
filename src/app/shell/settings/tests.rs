use super::*;

#[test]
fn ui_scale_percentage_conversion_clamps_to_supported_range() {
    assert_eq!(ui_scale_percent(1.25), 125.0);
    assert_eq!(ui_scale_from_percent(125.0), 1.25);
    assert_eq!(ui_scale_from_percent(20.0), MIN_UI_SCALE);
    assert_eq!(ui_scale_from_percent(400.0), MAX_UI_SCALE);
}
