use super::*;

fn stored(prefs: &GuiPrefs) -> GuiPrefs {
    prefs_from_value(&prefs_to_value(prefs, &HashSet::new(), true))
}

#[test]
fn the_stable_channel_is_the_default() {
    let prefs = prefs_from_value(&json!({}));
    assert_eq!(prefs.update_channel, UpdateChannel::Stable);
    assert!(prefs.check_updates_on_startup);
}

#[test]
fn preferences_written_before_this_setting_existed_load_as_stable() {
    // No `update_channel` key, but plenty of other settings: an existing user's
    // file must not silently move them onto development builds.
    let prefs = prefs_from_value(&json!({
        "session_restore": "always",
        "dark_mode": true,
    }));
    assert_eq!(prefs.update_channel, UpdateChannel::Stable);
    assert!(prefs.check_updates_on_startup);
    assert_eq!(prefs.session_restore, SessionRestore::Always);
}

#[test]
fn the_chosen_channel_survives_a_save_and_load() {
    let mut prefs = GuiPrefs::default();
    prefs.update_channel = UpdateChannel::Development;
    prefs.check_updates_on_startup = false;

    let reloaded = stored(&prefs);
    assert_eq!(reloaded.update_channel, UpdateChannel::Development);
    assert!(!reloaded.check_updates_on_startup);

    prefs.update_channel = UpdateChannel::Stable;
    prefs.check_updates_on_startup = true;
    let reloaded = stored(&prefs);
    assert_eq!(reloaded.update_channel, UpdateChannel::Stable);
    assert!(reloaded.check_updates_on_startup);
}

#[test]
fn an_unrecognised_channel_falls_back_to_stable() {
    let prefs = prefs_from_value(&json!({"update_channel": "nightly"}));
    assert_eq!(prefs.update_channel, UpdateChannel::Stable);
}
