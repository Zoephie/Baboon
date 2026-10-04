//! Process-wide storage selection for installed and portable Baboon state.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StorageMode {
    Installed,
    Portable,
}

impl StorageMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Portable => "portable",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StorageDiscovery {
    pub(crate) mode: Option<StorageMode>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) prefs_path: Option<PathBuf>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) used_legacy_prefs: bool,
}

#[derive(Clone, Debug)]
struct StorageContext {
    portable_root: PathBuf,
    installed_root: PathBuf,
    legacy_installed_root: PathBuf,
    mode: Option<StorageMode>,
}

static STORAGE: OnceLock<RwLock<StorageContext>> = OnceLock::new();

pub(crate) fn initialize() -> StorageDiscovery {
    let portable_root = executable_dir();
    let installed_root = installed_data_root("Baboon", "baboon");
    let legacy_installed_root = installed_data_root("Genesis", "genesis");
    let discovery = detect_at(&portable_root, &installed_root, &legacy_installed_root);
    let context = StorageContext {
        portable_root,
        installed_root,
        legacy_installed_root,
        mode: discovery.mode,
    };
    let _ = STORAGE.set(RwLock::new(context));
    discovery
}

pub(crate) fn detect_at(
    portable_root: &Path,
    installed_root: &Path,
    legacy_installed_root: &Path,
) -> StorageDiscovery {
    let portable = portable_root.join("prefs.json");
    if portable.is_file() {
        return StorageDiscovery {
            mode: Some(StorageMode::Portable),
            prefs_path: Some(portable),
            used_legacy_prefs: false,
        };
    }
    let installed = installed_root.join("prefs.json");
    if installed.is_file() {
        return StorageDiscovery {
            mode: Some(StorageMode::Installed),
            prefs_path: Some(installed),
            used_legacy_prefs: false,
        };
    }
    let legacy = legacy_installed_root.join("prefs.json");
    if legacy.is_file() {
        return StorageDiscovery {
            mode: Some(StorageMode::Installed),
            prefs_path: Some(legacy),
            used_legacy_prefs: true,
        };
    }
    StorageDiscovery {
        mode: None,
        prefs_path: None,
        used_legacy_prefs: false,
    }
}

pub(crate) fn activate(mode: StorageMode) {
    let lock = context();
    lock.write().expect("storage lock poisoned").mode = Some(mode);
}

pub(crate) fn active_mode() -> Option<StorageMode> {
    context().read().expect("storage lock poisoned").mode
}

pub(crate) fn data_path(filename: &str) -> PathBuf {
    let state = context().read().expect("storage lock poisoned");
    path_at(
        state.mode.unwrap_or(StorageMode::Installed),
        &state.portable_root,
        &state.installed_root,
        filename,
    )
}

fn path_at(
    mode: StorageMode,
    portable_root: &Path,
    installed_root: &Path,
    filename: &str,
) -> PathBuf {
    match mode {
        StorageMode::Installed => installed_root.join(filename),
        StorageMode::Portable => portable_root.join(filename),
    }
}

pub(crate) fn legacy_installed_path(filename: &str) -> PathBuf {
    context()
        .read()
        .expect("storage lock poisoned")
        .legacy_installed_root
        .join(filename)
}

fn context() -> &'static RwLock<StorageContext> {
    STORAGE.get_or_init(|| {
        let portable_root = executable_dir();
        let installed_root = installed_data_root("Baboon", "baboon");
        let legacy_installed_root = installed_data_root("Genesis", "genesis");
        let discovery = detect_at(&portable_root, &installed_root, &legacy_installed_root);
        RwLock::new(StorageContext {
            portable_root,
            installed_root,
            legacy_installed_root,
            mode: discovery.mode,
        })
    })
}

fn executable_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn installed_data_root(windows_folder: &str, unix_folder: &str) -> PathBuf {
    // Tests never touch the user's data, or the working directory: one folder
    // per test process under the temp dir.
    #[cfg(test)]
    {
        let _ = windows_folder;
        return test_data_root().join(unix_folder);
    }
    #[cfg(not(test))]
    {
        user_data_root(windows_folder, unix_folder)
    }
}

/// Where installed-mode state lives for a real run.
///
/// This used to fall back to the relative path `.baboon` whenever neither
/// `APPDATA` nor `USERPROFILE` was set, which is every macOS and Linux run:
/// installed state then went wherever Baboon was started from (`/` from the
/// Finder, where writes fail), and `cargo test` wrote prefs and an index into
/// the repository. `$HOME/.config/<folder>` is the same shape the
/// `USERPROFILE` branch already used.
pub(crate) fn user_data_root(windows_folder: &str, unix_folder: &str) -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join(windows_folder);
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".config").join(unix_folder);
    }
    PathBuf::from(format!(".{unix_folder}"))
}

#[cfg(test)]
fn test_data_root() -> PathBuf {
    std::env::temp_dir().join(format!("baboon-test-data-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_root(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("baboon_storage_{label}_{stamp}"))
    }

    #[test]
    fn portable_preferences_take_precedence() {
        let root = unique_root("precedence");
        let portable = root.join("portable");
        let installed = root.join("installed");
        let legacy = root.join("legacy");
        std::fs::create_dir_all(&portable).unwrap();
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(portable.join("prefs.json"), "{}").unwrap();
        std::fs::write(installed.join("prefs.json"), "{}").unwrap();

        let found = detect_at(&portable, &installed, &legacy);

        assert_eq!(found.mode, Some(StorageMode::Portable));
        assert_eq!(found.prefs_path, Some(portable.join("prefs.json")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_and_legacy_installed_preferences_are_detected() {
        let root = unique_root("installed");
        let portable = root.join("portable");
        let installed = root.join("installed");
        let legacy = root.join("legacy");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("prefs.json"), "{}").unwrap();

        let found = detect_at(&portable, &installed, &legacy);

        assert_eq!(found.mode, Some(StorageMode::Installed));
        assert!(found.used_legacy_prefs);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fresh_install_leaves_storage_undecided() {
        let root = unique_root("fresh");
        let found = detect_at(
            &root.join("portable"),
            &root.join("installed"),
            &root.join("legacy"),
        );

        assert_eq!(found.mode, None);
        assert!(found.prefs_path.is_none());
    }

    #[test]
    fn portable_redirects_every_automatic_state_path() {
        let portable = PathBuf::from("portable");
        let installed = PathBuf::from("installed");
        for name in [
            "prefs.json",
            "last_session.json",
            "campaign_evolved_recovery.baboon",
            "indexes.sqlite3",
            "halo3_mcc_index.json",
            "halo3_mcc_keywords.json",
            "terminal-logs",
        ] {
            assert_eq!(
                path_at(StorageMode::Portable, &portable, &installed, name),
                portable.join(name)
            );
        }
    }

    #[test]
    fn installed_state_paths_remain_under_app_data() {
        let portable = PathBuf::from("portable");
        let installed = PathBuf::from("installed");
        assert_eq!(
            path_at(StorageMode::Installed, &portable, &installed, "prefs.json"),
            installed.join("prefs.json")
        );
    }

    /// Tests keep installed-mode state in a temp folder of their own. It used to
    /// resolve to `.baboon` in the working directory, so `cargo test` wrote prefs
    /// and an index database into the repository.
    #[test]
    fn tests_keep_installed_state_out_of_the_working_directory() {
        let root = installed_data_root("Baboon", "baboon");
        assert!(root.starts_with(std::env::temp_dir()), "{}", root.display());
        assert!(root.is_absolute());
    }
}
