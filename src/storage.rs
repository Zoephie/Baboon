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
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join(windows_folder);
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(home).join(".config").join(unix_folder);
    }
    PathBuf::from(format!(".{unix_folder}"))
}

#[cfg(test)]
#[path = "app/tests/storage.rs"]
mod tests;
