use std::path::PathBuf;

use super::*;

#[test]
fn a_mod_always_gets_the_priority_suffix() {
    assert_eq!(
        ensure_priority_suffix(PathBuf::from("/mods/h2a_magnum.utoc")),
        PathBuf::from("/mods/h2a_magnum_P.utoc")
    );
    // Already correct, including the platform suffix the game itself uses.
    assert_eq!(
        ensure_priority_suffix(PathBuf::from("/mods/mymod-WinGDK_P.utoc")),
        PathBuf::from("/mods/mymod-WinGDK_P.utoc")
    );
    // The loader folds case before comparing, so a lowercase suffix
    // already has priority and must not collect a second one.
    assert_eq!(
        ensure_priority_suffix(PathBuf::from("/mods/thing_p.utoc")),
        PathBuf::from("/mods/thing_p.utoc")
    );
    // A version before the suffix raises priority further; it is still a
    // suffixed name and must be left alone.
    assert_eq!(
        ensure_priority_suffix(PathBuf::from("/mods/thing_2_P.utoc")),
        PathBuf::from("/mods/thing_2_P.utoc")
    );
}
