use super::*;

#[test]
fn editing_kit_shortcuts_include_expected_profiles() {
    let pairs: Vec<(&str, &str)> = EDITING_KIT_SHORTCUTS
        .iter()
        .map(|shortcut| (shortcut.label, shortcut.game))
        .collect();

    assert_eq!(
        pairs,
        vec![
            ("HCEEK", "haloce_mcc"),
            ("H2EK", "halo2_mcc"),
            ("H3EK", "halo3_mcc"),
            ("H3ODSTEK", "halo3odst_mcc"),
            ("HREK", "haloreach_mcc"),
            ("H4EK", "halo4_mcc"),
            ("H2AMPEK", "halo2amp_mcc"),
            ("Campaign Evolved", "haloce_evolved"),
        ]
    );
}

#[test]
fn update_channel_round_trips_through_its_stored_string() {
    for channel in UpdateChannel::ALL {
        assert_eq!(UpdateChannel::from_str(channel.as_str()), Some(channel));
    }
    assert_eq!(UpdateChannel::from_str("nightly"), None);
    assert_eq!(UpdateChannel::default(), UpdateChannel::Stable);
}
