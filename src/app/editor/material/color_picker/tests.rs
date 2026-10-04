use super::*;

#[test]
fn an_rgb_picker_pins_alpha_opaque_even_when_an_rgba_swatch_is_applied() {
    let mut popup =
        MaterialColorPopup::new("rgb", 0.1, 0.2, 0.3, 0.4).with_alpha_available(false);

    assert!(!popup.alpha_available);
    assert_eq!(popup.alpha, 1.0);
    assert_eq!(popup.original_color[3], 1.0);

    popup.set_rgba_bytes(10, 20, 30, 40);
    assert_eq!(popup.alpha, 1.0);
}

#[test]
fn achromatic_rgb_changes_preserve_undefined_hsb_components() {
    let mut popup = MaterialColorPopup::new("color", 0.0, 0.0, 1.0, 1.0);
    let blue_hue = popup.hue;

    popup.set_rgb_components(0.5, 0.5, 0.5);
    assert_eq!(popup.hue, blue_hue);
    assert_eq!(popup.saturation, 0);

    popup.hue = 91;
    popup.saturation = 173;
    popup.set_rgb_components(0.0, 0.0, 0.0);
    assert_eq!(popup.hue, 91);
    assert_eq!(popup.saturation, 173);
    assert_eq!(popup.brightness, 0);
}

#[test]
fn hsb_state_keeps_black_cursor_position_and_the_top_hue_endpoint() {
    let mut popup = MaterialColorPopup::new("color", 1.0, 0.0, 0.0, 1.0);
    popup.hue = 255;
    popup.saturation = 211;
    popup.brightness = 0;

    popup.update_rgb_from_hsb();

    assert_eq!(popup.hue, 255);
    assert_eq!(popup.saturation, 211);
    assert_eq!(popup.brightness, 0);
    assert_eq!([popup.red, popup.green, popup.blue], [0.0, 0.0, 0.0]);
}
