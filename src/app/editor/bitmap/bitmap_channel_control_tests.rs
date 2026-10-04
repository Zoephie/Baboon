use super::*;

#[test]
fn soloing_a_channel_enables_it_and_disables_the_rest() {
    let mut preview = BitmapPreviewState::default();
    apply_bitmap_channel_action(
        &mut preview,
        ButtonIcon::ChannelBlue,
        BitmapChannelAction::Solo,
    );

    assert!(!preview.show_red);
    assert!(!preview.show_green);
    assert!(preview.show_blue);
    assert!(!preview.show_alpha);
}

#[test]
fn excluding_a_channel_disables_it_and_enables_the_rest() {
    let mut preview = BitmapPreviewState::default();
    preview.show_red = false;
    preview.show_green = false;
    preview.show_blue = false;
    preview.show_alpha = false;
    apply_bitmap_channel_action(
        &mut preview,
        ButtonIcon::ChannelGreen,
        BitmapChannelAction::Exclude,
    );

    assert!(preview.show_red);
    assert!(!preview.show_green);
    assert!(preview.show_blue);
    assert!(preview.show_alpha);
}

#[test]
fn active_channel_fills_are_explicitly_forty_percent() {
    assert_eq!(
        bitmap_channel_active_colors(ButtonIcon::ChannelRed)
            .0
            .to_array(),
        [102, 0, 0, 102]
    );
    assert_eq!(
        bitmap_channel_active_colors(ButtonIcon::ChannelGreen)
            .0
            .to_array(),
        [0, 102, 0, 102]
    );
    assert_eq!(
        bitmap_channel_active_colors(ButtonIcon::ChannelBlue)
            .0
            .to_array(),
        [0, 0, 102, 102]
    );
}
