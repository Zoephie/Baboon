//! Where tests find real editing kits and game installs.
//!
//! Always an environment variable, never a path on one developer's machine.
//! When a variable is unset the path is a placeholder that does not exist, so
//! a test's own "skip unless present" check skips it, and its skip message
//! names the variable to set.

use std::path::PathBuf;

fn root(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("<set {var}>")))
}

/// A Halo 2 (MCC) editing kit's `tags` folder.
pub(crate) fn h2ek_tags() -> PathBuf {
    root("BLAM_TEST_H2EK")
}

/// A Halo 3 (MCC) editing kit's `tags` folder.
pub(crate) fn h3ek_tags() -> PathBuf {
    root("BLAM_TEST_H3EK")
}

/// A Halo Reach (MCC) editing kit's `tags` folder.
pub(crate) fn hrek_tags() -> PathBuf {
    root("BLAM_TEST_HREK")
}

/// A Halo: Campaign Evolved install (the folder holding `Meteorite`).
pub(crate) fn ce_install() -> PathBuf {
    root("BLAM_TEST_CE")
}

/// A Campaign Evolved install's `Paks` folder.
pub(crate) fn ce_paks() -> PathBuf {
    ce_install().join("Meteorite/Content/Paks")
}

/// The environment variable naming a game's editing-kit `tags` folder.
fn kit_var(game: &str) -> String {
    match game {
        "halo2_mcc" => "BLAM_TEST_H2EK".to_owned(),
        "halo3_mcc" => "BLAM_TEST_H3EK".to_owned(),
        "halo3odst_mcc" => "BLAM_TEST_ODSTEK".to_owned(),
        "haloreach_mcc" => "BLAM_TEST_HREK".to_owned(),
        "halo4_mcc" => "BLAM_TEST_H4EK".to_owned(),
        "halo2amp_mcc" => "BLAM_TEST_H2AEK".to_owned(),
        "haloce_mcc" => "BLAM_TEST_HCEEK".to_owned(),
        other => format!("BLAM_TEST_{}", other.to_ascii_uppercase()),
    }
}

/// A path into a game's editing-kit `tags` folder, as a `&str` so a test that
/// used to hold a literal keeps its shape. Leaked; this is test code.
pub(crate) fn tag_path(game: &str, rel: &str) -> &'static str {
    let root = root(&kit_var(game));
    let path = if rel.is_empty() { root } else { root.join(rel) };
    leak(path)
}

/// This repository's own definitions, which tests read tags against.
pub(crate) fn definitions() -> &'static std::path::Path {
    std::path::Path::new(leak(crate::app::locate_definitions_root()))
}

pub(crate) fn leak(path: PathBuf) -> &'static str {
    Box::leak(path.display().to_string().into_boxed_str())
}

/// A path under the system temp directory that no other test, in this run or
/// a parallel one, will be handed. Not created.
pub(crate) fn unique_temp_path(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "baboon-{name}-{}-{nanos}-{serial}",
        std::process::id()
    ))
}

/// [`unique_temp_path`], created as an empty directory.
pub(crate) fn unique_temp_dir(name: &str) -> PathBuf {
    let dir = unique_temp_path(name);
    std::fs::create_dir_all(&dir).expect("create a temporary test directory");
    dir
}
