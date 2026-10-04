use super::*;

/// The stored spelling has to round trip, or the setting silently reverts
/// to Default on the next launch.
#[test]
fn every_nested_default_round_trips_through_its_stored_name() {
    for option in NestedDefault::ALL {
        assert_eq!(
            nested_default_from_str(Some(nested_default_str(option))),
            Some(option),
            "{} did not round trip",
            option.label()
        );
    }
    // An absent or unrecognised value falls back rather than failing the
    // whole preferences load.
    assert_eq!(nested_default_from_str(None), None);
    assert_eq!(nested_default_from_str(Some("nonsense")), None);
}
