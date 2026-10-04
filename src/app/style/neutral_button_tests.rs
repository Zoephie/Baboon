use super::*;

#[test]
fn neutral_button_fills_use_literal_ten_and_twenty_percent_channels() {
    assert_eq!(neutral_button_fill(true, 26).to_array(), [26, 26, 26, 26]);
    assert_eq!(neutral_button_fill(true, 51).to_array(), [51, 51, 51, 51]);
    assert_eq!(neutral_button_fill(false, 26).to_array(), [0, 0, 0, 26]);
    assert_eq!(neutral_button_fill(false, 51).to_array(), [0, 0, 0, 51]);
}

#[test]
fn selection_colors_are_lighter_with_a_dark_border_in_light_mode() {
    assert_eq!(selection_fill_for(true), Color32::from_rgb(64, 108, 134));
    assert_eq!(selection_fill_for(false), Color32::from_rgb(75, 125, 155));
    assert_eq!(selection_stroke_for(false), Color32::from_rgb(38, 63, 78));
}
