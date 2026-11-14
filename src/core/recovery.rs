//! Recovery destinations, uniquely reserved batches and resumable legacy migration.
//! No deletion is performed here: callers move files only after preparation succeeds.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct RecoveryLocation {
    /// Assigned once, independently of later kit renames. A single safe component.
    pub name: String,
    /// None follows the active Baboon data directory and engine.
    pub folder: Option<PathBuf>,
}

pub(crate) fn safe_name(name: &str) -> String {
    let mut result = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '-' {
            result.push(character.to_ascii_lowercase());
        } else if !result.ends_with('_') {
            result.push('_');
        }
    }
    let result = result.trim_matches('_');
    if result.is_empty() {
        return "editing_kit".into();
    }
    let result: String = result.chars().take(80).collect();
    if result == "misc"
        || is_legacy_run(&result)
        || matches!(result.as_str(), "con" | "prn" | "aux" | "nul")
        || (result.len() == 4
            && (result.starts_with("com") || result.starts_with("lpt"))
            && result.ends_with(|c: char| c.is_ascii_digit()))
    {
        format!("kit_{result}")
    } else {
        result
    }
}

pub(crate) fn default_folder(data: &Path, game: Option<&str>, name: Option<&str>) -> PathBuf {
    data.join("deleted-tags")
        .join(game.unwrap_or("unknown-game"))
        .join(name.unwrap_or("misc"))
}

/// Resolve existing ancestors as well as lexical `..`, so a not-yet-created
/// directory beneath a junction cannot bypass the live-tags exclusion.
pub(crate) fn resolved_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Choose an absolute folder or a path relative to the kit root".into());
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !clean.pop() {
                    return Err("The folder path escapes its root".into());
                }
            }
            _ => clean.push(component.as_os_str()),
        }
    }
    let mut existing = clean.as_path();
    let mut tail = Vec::new();
    while !existing.try_exists().map_err(|e| e.to_string())? {
        tail.push(
            existing
                .file_name()
                .ok_or("The folder has no existing parent")?
                .to_owned(),
        );
        existing = existing
            .parent()
            .ok_or("The folder has no existing parent")?;
    }
    if !existing.is_dir() {
        return Err(format!("{} is a file, not a folder", existing.display()));
    }
    let mut resolved = existing.canonicalize().map_err(|e| e.to_string())?;
    for component in tail.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn comparable(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

pub(crate) fn validate_folder(folder: &Path, tags_roots: &[PathBuf]) -> Result<PathBuf, String> {
    let resolved = resolved_path(folder)?;
    for root in tags_roots {
        let root = resolved_path(root)?;
        if comparable(&resolved).starts_with(comparable(&root)) {
            return Err(format!(
                "Deleted Tags Folder must be outside live tags folders: {}",
                root.display()
            ));
        }
    }
    Ok(resolved)
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoverySource {
    TagsFolder,
    SingleFile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RecoveryOrigin {
    pub source: RecoverySource,
    pub tags_root: PathBuf,
    pub game: Option<String>,
    pub kit_id: Option<String>,
    pub kit_name: Option<String>,
}

#[derive(Serialize)]
struct BatchInfo<'a> {
    version: u32,
    operation: &'a str,
    created_unix_seconds: u64,
    original_relative_path: &'a Path,
    origin: &'a RecoveryOrigin,
    contents: &'a str,
}

/// Atomically reserve a unique batch before touching source files. Metadata is
/// outside `files/`, so it can never collide with user-owned payload names.
pub(crate) fn prepare_batch(
    base: &Path,
    origin: &RecoveryOrigin,
    relative: &Path,
    operation: &str,
    now: u64,
) -> Result<PathBuf, String> {
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("This item has no safe relative path to recover".into());
    }
    let live_roots = match origin.source {
        RecoverySource::TagsFolder => std::slice::from_ref(&origin.tags_root),
        RecoverySource::SingleFile => &[],
    };
    let base = validate_folder(base, live_roots)?;
    fs::create_dir_all(&base)
        .map_err(|e| format!("Could not create recovery folder {}: {e}", base.display()))?;
    // Recheck after creation in case the ancestor resolution changed.
    let base = validate_folder(&base, live_roots)?;
    for suffix in 0..u64::MAX {
        let name = if suffix == 0 {
            now.to_string()
        } else {
            format!("{now}-{suffix}")
        };
        let batch = base.join(name);
        match fs::create_dir(&batch) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not reserve recovery batch: {error}")),
            Ok(()) => {}
        }
        let info = BatchInfo {
            version: 1,
            operation,
            created_unix_seconds: now,
            original_relative_path: relative,
            origin,
            contents: "files (check this folder for recovered contents)",
        };
        let text = serde_json::to_vec_pretty(&info).map_err(|e| e.to_string())?;
        fs::write(batch.join("recovery-info.json"), text)
            .map_err(|e| format!("Could not record recovery information: {e}"))?;
        let destination = batch.join("files").join(relative);
        fs::create_dir_all(destination.parent().ok_or("Invalid recovery path")?)
            .map_err(|e| e.to_string())?;
        return Ok(destination);
    }
    Err("No unused recovery batch name is available".into())
}

fn is_legacy_run(name: &str) -> bool {
    let mut parts = name.split('-');
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    parts.next().is_some_and(digits) && parts.next().is_none_or(digits) && parts.next().is_none()
}

/// Reserve one files/ tree and record every selected root in its manifest.
pub(crate) fn prepare_batch_paths(base:&Path,origin:&RecoveryOrigin,paths:&[PathBuf],operation:&str,now:u64)->Result<PathBuf,String> {
    if paths.is_empty() || paths.iter().any(|path|path.as_os_str().is_empty() || !path.components().all(|part|matches!(part,Component::Normal(_)))) {
        return Err("This selection has no safe relative paths to recover".into());
    }
    let mut files=prepare_batch(base,origin,&paths[0],operation,now)?;
    for _ in paths[0].components() {files.pop();}
    let manifest=files.parent().ok_or("Invalid batch path")?.join("recovery-info.json");
    let mut info:serde_json::Value=serde_json::from_slice(&fs::read(&manifest).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    info["original_relative_paths"]=serde_json::to_value(paths).map_err(|e|e.to_string())?;
    fs::write(manifest,serde_json::to_vec_pretty(&info).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    Ok(files)
}

/// A custom location may itself be an engine folder (or a numeric folder
/// below it). Finish that engine's migration before creating new batches there,
/// so the next launch cannot mistake new recovery data for the legacy layout.
pub(crate) fn prepare_custom_location(storage: &Path, folder: &Path) -> Result<(), String> {
    let storage = resolved_path(storage)?;
    let folder = resolved_path(folder)?;
    if !comparable(&folder).starts_with(comparable(&storage)) {
        return Ok(());
    }
    let Some(engine) = folder.components().nth(storage.components().count()) else {
        return Ok(());
    };
    let Component::Normal(engine) = engine else {
        return Err("Invalid recovery engine folder".into());
    };
    let game = engine
        .to_str()
        .ok_or("Invalid recovery engine folder name")?;
    fs::create_dir_all(storage.join(engine)).map_err(|e| e.to_string())?;
    migrate_legacy_game(&storage, game)
}

/// Only old timestamp directories are moved. The completion marker is written
/// last; failures preserve the source and a retry handles the remaining runs.
pub(crate) fn migrate_legacy(root: &Path) -> Result<(), String> {
    migrate_legacy_filtered(root, None)
}

pub(crate) fn migrate_legacy_game(root: &Path, game: &str) -> Result<(), String> {
    if Path::new(game).components().count() != 1
        || !Path::new(game)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("Invalid recovery engine folder".into());
    }
    migrate_legacy_filtered(root, Some(game))
}

fn migrate_legacy_filtered(root: &Path, game: Option<&str>) -> Result<(), String> {
    static MIGRATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = MIGRATION
        .lock()
        .map_err(|_| "Recovery migration lock failed")?;
    if !root.try_exists().map_err(|e| e.to_string())? {
        return Ok(());
    }
    let canonical_root = root.canonicalize().map_err(|e| e.to_string())?;
    for engine in fs::read_dir(root).map_err(|e| e.to_string())? {
        let engine = engine.map_err(|e| e.to_string())?;
        if game.is_some_and(|game| engine.file_name() != std::ffi::OsStr::new(game)) {
            continue;
        }
        if !engine.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let directory = engine.path();
        if directory.join("recovery-info.json").is_file() {
            continue;
        }
        let resolved = directory.canonicalize().map_err(|e| e.to_string())?;
        if !resolved.starts_with(&canonical_root) {
            return Err("Recovery engine folder points outside storage".into());
        }
        let marker = directory.join(".recovery-layout-v2");
        if marker.try_exists().map_err(|e| e.to_string())? {
            continue;
        }
        let runs = fs::read_dir(&directory)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for run in runs {
            if !is_legacy_run(&run.file_name().to_string_lossy()) {
                continue;
            }
            let metadata = run.file_type().map_err(|e| e.to_string())?;
            if metadata.is_symlink() {
                return Err("A legacy recovery batch is a linked directory".into());
            }
            if !metadata.is_dir() {
                continue;
            }
            let misc = directory.join("misc");
            fs::create_dir_all(&misc).map_err(|e| e.to_string())?;
            if misc
                .symlink_metadata()
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
                || !misc
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&resolved)
            {
                return Err("The misc recovery folder points outside its engine folder".into());
            }
            let source = run.path();
            if !source
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(&resolved)
            {
                return Err("A legacy recovery batch points outside its engine folder".into());
            }
            let mut destination = misc.join(run.file_name());
            let mut suffix = 0u64;
            while destination.try_exists().map_err(|e| e.to_string())? {
                suffix += 1;
                destination = misc.join(format!("{}-{suffix}", run.file_name().to_string_lossy()));
            }
            fs::rename(&source, &destination).map_err(|e| {
                format!(
                    "Could not migrate {}: {e}. Existing recovery files were retained.",
                    source.display()
                )
            })?;
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)
            .or_else(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(fs::File::open(directory.join(".recovery-layout-v2"))?)
                } else {
                    Err(e)
                }
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("baboon-recovery-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(root.join("kit/tags")).unwrap();
            Self(root)
        }
        fn origin(&self) -> RecoveryOrigin {
            RecoveryOrigin {
                source: RecoverySource::TagsFolder,
                tags_root: self.0.join("kit/tags"),
                game: Some("halo2_mcc".into()),
                kit_id: Some("profile-1".into()),
                kit_name: Some("Rebalance".into()),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn recovery_batches_reserve_unique_names_and_keep_origin_outside_payload() {
        let f = Fixture::new();
        let base = f.0.join("recovery");
        let origin = f.origin();
        let relative = Path::new("objects/rifle.weapon");
        let first = prepare_batch(&base, &origin, relative, "delete tag", 99).unwrap();
        fs::write(&first, b"first deletion").unwrap();
        let second = prepare_batch(&base, &origin, relative, "delete tag", 99).unwrap();
        fs::write(&second, b"second deletion").unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), b"first deletion");
        let info: serde_json::Value =
            serde_json::from_slice(&fs::read(base.join("99/recovery-info.json")).unwrap()).unwrap();
        assert_eq!(info["origin"]["kit_name"], "Rebalance");
        assert_eq!(info["operation"], "delete tag");
        assert!(first.ends_with("99/files/objects/rifle.weapon"));
    }

    #[test]
    fn recovery_rejects_live_tags_escapes_and_file_parents_without_touching_source() {
        let f = Fixture::new();
        let origin = f.origin();
        let source = origin.tags_root.join("rifle.weapon");
        fs::write(&source, b"original").unwrap();
        for base in [
            origin.tags_root.clone(),
            origin.tags_root.join("deleted"),
            f.0.join("kit/tags/../tags/new/deleted"),
        ] {
            assert!(
                prepare_batch(&base, &origin, Path::new("rifle.weapon"), "delete", 99).is_err()
            );
        }
        for relative in [
            "../escape.weapon",
            "objects/../../escape.weapon",
            "",
            "/absolute.weapon",
        ] {
            assert!(
                prepare_batch(
                    &f.0.join("recovery"),
                    &origin,
                    Path::new(relative),
                    "delete",
                    99
                )
                .is_err()
            );
        }
        fs::write(f.0.join("occupied"), b"not a directory").unwrap();
        assert!(
            prepare_batch(
                &f.0.join("occupied/nested"),
                &origin,
                Path::new("rifle.weapon"),
                "delete",
                99
            )
            .is_err()
        );
        assert_eq!(fs::read(source).unwrap(), b"original");
    }

    #[test]
    fn recovery_migration_preserves_collisions_profiles_and_is_idempotent() {
        let f = Fixture::new();
        let root = f.0.join("deleted-tags");
        let engine = root.join("halo2_mcc");
        for folder in ["99", "100-1", "misc/99", "halo_2_rebalance"] {
            fs::create_dir_all(engine.join(folder)).unwrap();
        }
        fs::write(engine.join("99/rifle.weapon"), b"legacy").unwrap();
        fs::write(engine.join("misc/99/rifle.weapon"), b"existing").unwrap();
        fs::write(engine.join("halo_2_rebalance/kept.weapon"), b"profile").unwrap();
        migrate_legacy(&root).unwrap();
        migrate_legacy(&root).unwrap();
        assert!(!engine.join("99").exists());
        assert_eq!(
            fs::read(engine.join("misc/99/rifle.weapon")).unwrap(),
            b"existing"
        );
        assert_eq!(
            fs::read(engine.join("misc/99-1/rifle.weapon")).unwrap(),
            b"legacy"
        );
        assert!(engine.join("misc/100-1").is_dir());
        assert!(engine.join("halo_2_rebalance/kept.weapon").is_file());
        assert!(engine.join(".recovery-layout-v2").is_file());
    }

    #[test]
    fn recovery_migration_can_resume_after_an_unavailable_misc_folder() {
        let f = Fixture::new();
        let root = f.0.join("deleted-tags");
        let engine = root.join("halo3_mcc");
        fs::create_dir_all(engine.join("99")).unwrap();
        fs::write(engine.join("99/tag.weapon"), b"legacy").unwrap();
        fs::write(engine.join("misc"), b"blocked").unwrap();
        assert!(migrate_legacy(&root).is_err());
        assert!(engine.join("99/tag.weapon").is_file());
        assert!(!engine.join(".recovery-layout-v2").exists());
        let other = root.join("halo2_mcc");
        fs::create_dir_all(other.join("101")).unwrap();
        fs::write(other.join("101/another.weapon"), b"other engine").unwrap();
        migrate_legacy_game(&root, "halo2_mcc").unwrap();
        assert!(other.join("misc/101/another.weapon").is_file());
        fs::remove_file(engine.join("misc")).unwrap();
        migrate_legacy(&root).unwrap();
        assert_eq!(
            fs::read(engine.join("misc/99/tag.weapon")).unwrap(),
            b"legacy"
        );
    }

    #[test]
    fn new_custom_locations_inside_storage_are_not_later_migrated() {
        let f = Fixture::new();
        let storage = f.0.join("deleted-tags");
        let custom = storage.join("halo2_mcc/123/custom");
        prepare_custom_location(&storage, &custom).unwrap();
        let file =
            prepare_batch(&custom, &f.origin(), Path::new("tag.weapon"), "delete", 99).unwrap();
        fs::write(&file, b"new data").unwrap();
        migrate_legacy(&storage).unwrap();
        assert_eq!(fs::read(file).unwrap(), b"new data");
        assert!(!storage.join("halo2_mcc/misc/123").exists());
    }

    #[test]
    fn recovery_defaults_follow_storage_engine_and_reserved_folder_rules() {
        assert_eq!(safe_name("Halo 2: Rebalance"), "halo_2_rebalance");
        for name in ["misc", "123", "123-2", "CON", "LPT1", "../"] {
            let safe = safe_name(name);
            assert_ne!(safe, "misc");
            assert!(!is_legacy_run(&safe));
            assert_eq!(Path::new(&safe).components().count(), 1);
        }
        assert_eq!(
            default_folder(Path::new("portable"), Some("halo2_mcc"), Some("rebalance")),
            Path::new("portable/deleted-tags/halo2_mcc/rebalance")
        );
        assert_eq!(
            default_folder(Path::new("installed"), Some("halo3_mcc"), None),
            Path::new("installed/deleted-tags/halo3_mcc/misc")
        );
    }
}
