//! Editing-kit profile validation and storage-mode-aware custom icon storage.

use super::*;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

pub(in crate::app) const CUSTOM_ICON_FOLDER: &str = "editing kit icons";
pub(in crate::app) const RECOMMENDED_CUSTOM_ICON_SIZE: u32 = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct EditingKitLayout {
    pub(in crate::app) root: PathBuf,
    pub(in crate::app) tags: PathBuf,
    pub(in crate::app) data: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum EditingKitPathStatus {
    Unconfigured,
    Ready(EditingKitLayout),
    Invalid(String),
}

#[derive(Clone, Debug, Default)]
pub(in crate::app) struct EditingKitValidationCache {
    built_ins: HashMap<String, EditingKitPathStatus>,
    custom_layouts: HashMap<String, Result<EditingKitLayout, String>>,
    custom_icon_errors: HashMap<String, Option<String>>,
}

impl EditingKitValidationCache {
    pub(in crate::app) fn new(
        paths: &HashMap<String, PathBuf>,
        profiles: &[CustomEditingKitProfile],
    ) -> Self {
        let mut cache = Self::default();
        cache.refresh(paths, profiles);
        cache
    }

    pub(in crate::app) fn refresh(
        &mut self,
        paths: &HashMap<String, PathBuf>,
        profiles: &[CustomEditingKitProfile],
    ) {
        self.built_ins = EDITING_KIT_SHORTCUTS
            .into_iter()
            .map(|shortcut| {
                (
                    shortcut.game.as_str().to_owned(),
                    validate_builtin_editing_kit(
                        shortcut,
                        paths.get(shortcut.game.as_str()).map(PathBuf::as_path),
                    ),
                )
            })
            .collect();
        self.custom_layouts = profiles
            .iter()
            .map(|profile| {
                (
                    profile.id.clone(),
                    validate_profile_layout(profile),
                )
            })
            .collect();
        self.custom_icon_errors = profiles
            .iter()
            .map(|profile| (profile.id.clone(), custom_profile_icon_error(profile)))
            .collect();
    }

    pub(in crate::app) fn refresh_builtin(
        &mut self,
        shortcut: EditingKitShortcut,
        configured: Option<&Path>,
    ) -> EditingKitPathStatus {
        let status = validate_builtin_editing_kit(shortcut, configured);
        self.built_ins
            .insert(shortcut.game.as_str().to_owned(), status.clone());
        status
    }

    pub(in crate::app) fn refresh_custom(
        &mut self,
        profile: &CustomEditingKitProfile,
    ) -> Result<EditingKitLayout, String> {
        let status = validate_profile_layout(profile);
        self.custom_layouts
            .insert(profile.id.clone(), status.clone());
        self.custom_icon_errors
            .insert(profile.id.clone(), custom_profile_icon_error(profile));
        status
    }

    pub(in crate::app) fn builtin(&self, shortcut: EditingKitShortcut) -> EditingKitPathStatus {
        self.built_ins
            .get(shortcut.game.as_str())
            .cloned()
            .unwrap_or(EditingKitPathStatus::Unconfigured)
    }

    pub(in crate::app) fn custom(&self, profile_id: &str) -> Result<EditingKitLayout, String> {
        self.custom_layouts
            .get(profile_id)
            .cloned()
            .unwrap_or_else(|| Err("Editing-kit status has not been refreshed".to_owned()))
    }

    pub(in crate::app) fn custom_icon_error(&self, profile_id: &str) -> Option<&str> {
        self.custom_icon_errors
            .get(profile_id)
            .and_then(Option::as_deref)
    }
}

impl Baboon {
    pub(in crate::app) fn refresh_editing_kit_validation(&mut self) {
        self.kit_tools.editing_kit_validation.refresh(
            &self.model.prefs.editing_kit_paths,
            &self.model.prefs.custom_editing_kit_profiles,
        );
        self.shell.artwork.retry_custom_editing_kits();
    }

    pub(in crate::app) fn refresh_builtin_editing_kit_validation(
        &mut self,
        shortcut: EditingKitShortcut,
    ) -> EditingKitPathStatus {
        self.kit_tools.editing_kit_validation.refresh_builtin(
            shortcut,
            self.model.prefs
                .editing_kit_paths
                .get(shortcut.game.as_str())
                .map(PathBuf::as_path),
        )
    }
}

impl EditingKitPathStatus {
    pub(in crate::app) fn layout(&self) -> Option<&EditingKitLayout> {
        match self {
            Self::Ready(layout) => Some(layout),
            Self::Unconfigured | Self::Invalid(_) => None,
        }
    }

    pub(in crate::app) fn message(&self) -> String {
        match self {
            Self::Unconfigured => "Not configured".to_owned(),
            Self::Ready(layout) => format!("Ready: {}", layout.root.display()),
            Self::Invalid(error) => error.clone(),
        }
    }
}

pub(in crate::app) fn validate_builtin_editing_kit(
    shortcut: EditingKitShortcut,
    configured: Option<&Path>,
) -> EditingKitPathStatus {
    let Some(path) = configured.filter(|path| !path.as_os_str().is_empty()) else {
        return EditingKitPathStatus::Unconfigured;
    };
    if shortcut.game.is_campaign_evolved() {
        return match crate::core::source::find_paks_dir(path) {
            Some(paks) => EditingKitPathStatus::Ready(EditingKitLayout {
                root: path.to_path_buf(),
                tags: paks,
                data: None,
            }),
            None => EditingKitPathStatus::Invalid(format!(
                "Campaign Evolved Paks were not found under {}",
                path.display()
            )),
        };
    }
    validate_loose_editing_kit_layout(path, false)
        .map(EditingKitPathStatus::Ready)
        .unwrap_or_else(EditingKitPathStatus::Invalid)
}

#[cfg(test)]
pub(in crate::app) fn validate_custom_editing_kit_layout(path: &Path) -> Result<EditingKitLayout, String> {
    validate_loose_editing_kit_layout(path, true)
}

pub(in crate::app) fn validate_editing_kit_profile_layout(
    path: &Path,
    game: &str,
) -> Result<EditingKitLayout, String> {
    let shortcut = EDITING_KIT_SHORTCUTS
        .into_iter()
        .find(|shortcut| shortcut.game.as_str() == game)
        .ok_or_else(|| "Choose a supported editing-kit engine".to_owned())?;
    match validate_builtin_editing_kit(shortcut, Some(path)) {
        EditingKitPathStatus::Ready(layout) => Ok(layout),
        status => Err(status.message()),
    }
}

fn validate_loose_editing_kit_layout(
    selected: &Path,
    require_data: bool,
) -> Result<EditingKitLayout, String> {
    if !selected.is_dir() {
        return Err(format!("Folder not found: {}", selected.display()));
    }

    let selected = canonical_or_clean(selected);
    let mut candidates = Vec::new();
    if is_named_dir(&selected, "tags") || is_named_dir(&selected, "data") {
        if let Some(parent) = selected.parent() {
            push_layout_candidate(&mut candidates, parent, require_data);
        }
        return finish_layout_candidates(candidates, &selected, require_data);
    } else {
        push_layout_candidate(&mut candidates, &selected, require_data);
        if candidates.len() == 1 {
            return Ok(candidates.remove(0));
        }
        for entry in WalkDir::new(&selected)
            .min_depth(1)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_dir())
        {
            push_layout_candidate(&mut candidates, entry.path(), require_data);
        }
    }

    finish_layout_candidates(candidates, &selected, require_data)
}

fn finish_layout_candidates(
    mut candidates: Vec<EditingKitLayout>,
    selected: &Path,
    require_data: bool,
) -> Result<EditingKitLayout, String> {
    candidates.sort_by(|a, b| a.root.cmp(&b.root));
    candidates.dedup_by(|a, b| same_recent_path(&a.root, &b.root));
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        n if n > 1 => Err(format!(
            "Multiple editing-kit layouts were found under {}; select the specific kit root",
            selected.display()
        )),
        _ => {
            let tags = find_first_named_directory(&selected, "tags");
            if tags.is_none() {
                Err(format!(
                    "Required tags directory was not found under {}",
                    selected.display()
                ))
            } else if require_data {
                Err(format!(
                    "Required data directory was not found beside {}",
                    tags.unwrap().display()
                ))
            } else {
                Err(format!(
                    "Required tags directory was not found under {}",
                    selected.display()
                ))
            }
        }
    }
}

fn find_first_named_directory(root: &Path, expected: &str) -> Option<PathBuf> {
    find_named_child(root, expected).or_else(|| {
        WalkDir::new(root)
            .min_depth(1)
            .max_depth(4)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .find(|entry| {
                entry.file_type().is_dir()
                    && entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
            })
            .map(|entry| entry.into_path())
    })
}

fn push_layout_candidate(candidates: &mut Vec<EditingKitLayout>, root: &Path, require_data: bool) {
    let Some(tags) = find_named_child(root, "tags") else {
        return;
    };
    let data = find_named_child(root, "data");
    if require_data && data.is_none() {
        return;
    }
    candidates.push(EditingKitLayout {
        root: canonical_or_clean(root),
        tags: canonical_or_clean(&tags),
        data: data.map(|path| canonical_or_clean(&path)),
    });
}

fn find_named_child(root: &Path, expected: &str) -> Option<PathBuf> {
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(expected))
        })
        .map(|entry| entry.path())
}

fn is_named_dir(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

pub(in crate::app) fn canonical_or_clean(path: &Path) -> PathBuf {
    clean_recent_path(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// A profile's layout: its chosen tags and data folders when it has them (see
/// [`validate_kit_layout`]), otherwise the folders found under its root.
pub(in crate::app) fn validate_profile_layout(
    profile: &CustomEditingKitProfile,
) -> Result<EditingKitLayout, String> {
    validate_kit_layout(
        &profile.root,
        &profile.game,
        profile.tags_folder.as_deref(),
        profile.data_folder.as_deref(),
    )
}

/// The layout of a kit at `root` that may name its own tags and data folders,
/// each relative to `root` or absolute.
///
/// With neither named, or for an engine that can't use them, this is the
/// ordinary discovery. With either named, `root` is taken as the kit root as
/// given (no search beneath it), the named folders must exist, and an unnamed
/// one falls back to the root's own `tags`/`data`.
pub(in crate::app) fn validate_kit_layout(
    root: &Path,
    game: &str,
    tags_folder: Option<&Path>,
    data_folder: Option<&Path>,
) -> Result<EditingKitLayout, String> {
    if !kit_folders_are_choosable(game) || (tags_folder.is_none() && data_folder.is_none()) {
        return validate_editing_kit_profile_layout(root, game);
    }
    if !root.is_dir() {
        return Err(format!("Folder not found: {}", root.display()));
    }
    let root = canonical_or_clean(root);
    let tags = match tags_folder {
        Some(folder) => {
            let tags = root.join(folder);
            if !tags.is_dir() {
                return Err(format!("Tags folder not found: {}", tags.display()));
            }
            tags
        }
        None => find_named_child(&root, "tags").ok_or_else(|| {
            format!(
                "Required tags directory was not found under {}",
                root.display()
            )
        })?,
    };
    let data = match data_folder {
        Some(folder) => {
            let data = root.join(folder);
            if !data.is_dir() {
                return Err(format!("Data folder not found: {}", data.display()));
            }
            Some(data)
        }
        None => find_named_child(&root, "data"),
    };
    Ok(EditingKitLayout {
        tags: canonical_or_clean(&tags),
        data: data.map(|data| canonical_or_clean(&data)),
        root,
    })
}

/// The tags folder that identifies a profile's kit: two profiles may share a
/// root, but not a tags folder. A profile that doesn't validate is identified
/// by where its tags folder would be.
pub(in crate::app) fn profile_tags_folder(profile: &CustomEditingKitProfile) -> PathBuf {
    validate_profile_layout(profile)
        .map(|layout| layout.tags)
        .unwrap_or_else(|_| {
            canonical_or_clean(&profile.root).join(
                profile
                    .tags_folder
                    .as_deref()
                    .filter(|_| kit_folders_are_choosable(&profile.game))
                    .unwrap_or(Path::new("tags")),
            )
        })
}

/// What a profile stores for a chosen tags or data folder (`default_name` is
/// `tags` or `data`): nothing when it is the root's own folder of that name, so
/// a kit left on its defaults saves as it always did and passes its tools no
/// folder options; otherwise the folder relative to the root when it is
/// inside it, or absolute.
pub(in crate::app) fn folder_to_store(
    root: &Path,
    folder: Option<&Path>,
    default_name: &str,
) -> Option<PathBuf> {
    let folder = folder?;
    let default = find_named_child(root, default_name).map(|path| canonical_or_clean(&path));
    if default
        .as_deref()
        .is_some_and(|default| same_recent_path(default, folder))
    {
        return None;
    }
    Some(
        folder
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| folder.to_path_buf()),
    )
}

/// The child folders of `root` whose names contain `needle` (`tags` or
/// `data`), sorted: the choices offered beside a kit's tags and data folder
/// inputs.
pub(in crate::app) fn kit_folder_candidates(root: &Path, needle: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| name.to_ascii_lowercase().contains(needle))
        .collect();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names
}

/// The name of `root`'s own `tags` or `data` folder, as it is spelled on disk,
/// for filling a folder input when the root is chosen.
pub(in crate::app) fn default_kit_folder_name(root: &Path, default_name: &str) -> Option<String> {
    find_named_child(root, default_name)
        .and_then(|path| path.file_name()?.to_str().map(str::to_owned))
}

/// The folder that tells a profile's kit apart when it is listed: its root, or
/// for a kit that chose its own folders (whose root other kits may share), its
/// tags folder.
pub(in crate::app) fn profile_location<'a>(
    profile: &'a CustomEditingKitProfile,
    layout: Option<&'a EditingKitLayout>,
) -> &'a Path {
    match layout {
        Some(layout) if profile.has_chosen_folders() => &layout.tags,
        Some(layout) => &layout.root,
        None => &profile.root,
    }
}

/// Whether another profile already uses `resolved_tags` as its tags folder.
/// Kits may share a root, but two kits on one tags folder would share its
/// index, favorites and keywords while each believing it owned them.
pub(in crate::app) fn custom_profile_tags_conflicts(
    profiles: &[CustomEditingKitProfile],
    editing_profile_id: Option<&str>,
    resolved_tags: &Path,
) -> bool {
    profiles.iter().any(|profile| {
        Some(profile.id.as_str()) != editing_profile_id
            && validate_profile_layout(profile)
                .is_ok_and(|layout| same_recent_path(&layout.tags, resolved_tags))
    })
}

pub(in crate::app) fn executable_directory() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the Baboon executable: {error}"))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "The Baboon executable has no parent directory".to_owned())
}

pub(in crate::app) fn resolve_custom_icon_path(relative: &Path) -> Result<PathBuf, String> {
    let legacy = legacy_custom_icon_base();
    resolve_custom_icon_path_in_roots(&crate::core::storage::data_path(""), legacy.as_deref(), relative)
}

#[cfg(test)]
fn resolve_custom_icon_path_at(base: &Path, relative: &Path) -> Result<PathBuf, String> {
    resolve_custom_icon_path_in_roots(base, None, relative)
}

fn legacy_custom_icon_base() -> Option<PathBuf> {
    if crate::core::storage::active_mode() == Some(crate::core::storage::StorageMode::Portable) {
        None
    } else {
        executable_directory().ok()
    }
}

fn resolve_custom_icon_path_in_roots(
    base: &Path,
    legacy: Option<&Path>,
    relative: &Path,
) -> Result<PathBuf, String> {
    if !safe_custom_icon_relative_path(relative) {
        return Err("Saved custom icon path is unsafe".to_owned());
    }
    let current = base.join(relative);
    if !current.is_file()
        && let Some(old) = legacy
            .map(|root| root.join(relative))
            .filter(|path| path.is_file())
    {
        return Ok(old);
    }
    Ok(current)
}

pub(in crate::app) fn custom_profile_icon_error(profile: &CustomEditingKitProfile) -> Option<String> {
    let relative = profile.icon.as_deref()?;
    let absolute = match resolve_custom_icon_path(relative) {
        Ok(path) => path,
        Err(error) => return Some(error),
    };
    validate_custom_icon_source(&absolute)
        .err()
        .map(|error| format!("Using the default icon: {error}"))
}

pub(in crate::app) fn validate_custom_icon_source(path: &Path) -> Result<(u32, u32), String> {
    let png_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
    if !png_extension {
        return Err("Editing kit icons must be PNG files".to_owned());
    }
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read editing kit icon: {error}"))?;
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map_err(|error| format!("Selected file is not a readable PNG: {error}"))?;
    Ok((image.width(), image.height()))
}

pub(in crate::app) fn copy_custom_icon(
    source: &Path,
    project_name: &str,
    profile_id: &str,
    existing_icon: Option<&Path>,
) -> Result<PathBuf, String> {
    copy_custom_icon_at(
        &crate::core::storage::data_path(""),
        source,
        project_name,
        profile_id,
        existing_icon,
    )
}

fn copy_custom_icon_at(
    base: &Path,
    source: &Path,
    project_name: &str,
    profile_id: &str,
    existing_icon: Option<&Path>,
) -> Result<PathBuf, String> {
    validate_custom_icon_source(source)?;
    let bytes =
        fs::read(source).map_err(|error| format!("Could not read editing kit icon: {error}"))?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let short_id = profile_id
        .chars()
        .filter(|ch| *ch != '-')
        .take(8)
        .collect::<String>();
    let existing_folder = existing_icon
        .filter(|path| safe_custom_icon_relative_path(path))
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(PathBuf::from);
    let folder = existing_folder.unwrap_or_else(|| {
        PathBuf::from(format!(
            "{}-{short_id}",
            sanitise_project_name(project_name)
        ))
    });
    let relative = PathBuf::from(CUSTOM_ICON_FOLDER)
        .join(folder)
        .join(format!("icon-{}.png", &hash[..12]));
    let destination = base.join(&relative);
    let parent = destination
        .parent()
        .ok_or_else(|| "Custom icon destination has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create editing kit icon folder: {error}"))?;
    if !destination.is_file() {
        let temporary = parent.join(format!(".icon-{}.tmp", &hash[..12]));
        fs::write(&temporary, &bytes)
            .map_err(|error| format!("Could not write editing kit icon: {error}"))?;
        fs::rename(&temporary, &destination).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            format!("Could not finish writing the custom icon: {error}")
        })?;
    }
    Ok(relative)
}

pub(in crate::app) fn remove_unreferenced_custom_icon(
    relative: &Path,
    profiles: &[CustomEditingKitProfile],
) -> Result<(), String> {
    let legacy = legacy_custom_icon_base();
    remove_unreferenced_custom_icon_in_roots(
        &crate::core::storage::data_path(""),
        legacy.as_deref(),
        relative,
        profiles,
    )
}

#[cfg(test)]
fn remove_unreferenced_custom_icon_at(
    base: &Path,
    relative: &Path,
    profiles: &[CustomEditingKitProfile],
) -> Result<(), String> {
    remove_unreferenced_custom_icon_in_roots(base, None, relative, profiles)
}

fn remove_unreferenced_custom_icon_in_roots(
    base: &Path,
    legacy: Option<&Path>,
    relative: &Path,
    profiles: &[CustomEditingKitProfile],
) -> Result<(), String> {
    if profiles
        .iter()
        .any(|profile| profile.icon.as_deref() == Some(relative))
    {
        return Ok(());
    }
    let absolute = resolve_custom_icon_path_in_roots(base, legacy, relative)?;
    if absolute.is_file() {
        fs::remove_file(&absolute).map_err(|error| {
            format!("Profile was saved, but its old icon could not be deleted: {error}")
        })?;
    }
    if let Some(parent) = absolute.parent()
        && parent
            .parent()
            .is_some_and(|root| root.ends_with(CUSTOM_ICON_FOLDER))
    {
        let _ = fs::remove_dir(parent);
    }
    Ok(())
}

pub(in crate::app) fn safe_custom_icon_relative_path(path: &Path) -> bool {
    !path.is_absolute()
        && path.starts_with(CUSTOM_ICON_FOLDER)
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

/// Whether Windows reserves `name` for a device, so no file or folder can
/// have it: `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9` and `LPT1`-`LPT9`, in
/// any case. The reservation covers the part before the first dot, so
/// `nul.wav` is the device too, and ignores trailing spaces there.
pub(in crate::app) fn is_windows_reserved_name(name: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let base = name.split('.').next().unwrap_or_default().trim_end_matches(' ');
    RESERVED.iter().any(|reserved| reserved.eq_ignore_ascii_case(base))
}

pub(in crate::app) fn sanitise_project_name(name: &str) -> String {
    let mut output = String::new();
    let mut previous_separator = false;
    for ch in name.trim().chars() {
        let invalid =
            ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*');
        let mapped = if invalid || ch.is_whitespace() {
            '-'
        } else {
            ch
        };
        if mapped == '-' {
            if !previous_separator && !output.is_empty() {
                output.push('-');
            }
            previous_separator = true;
        } else {
            output.push(mapped);
            previous_separator = false;
        }
        if output.chars().count() >= 48 {
            break;
        }
    }
    let output = output.trim_matches([' ', '.', '-']).to_owned();
    if output.is_empty() || is_windows_reserved_name(&output) {
        "project".to_owned()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        crate::core::test_kits::unique_temp_dir(&format!("editing-kits-{label}"))
    }

    #[test]
    fn icon_storage_uses_active_data_root_and_preserves_legacy_icons() {
        let root = temp_dir("icon-storage-modes");
        let installed = root.join("AppData").join("Baboon");
        let portable = root.join("PortableBaboon");
        let source = root.join("source.png");
        image::RgbaImage::new(32, 32).save(&source).unwrap();
        // The old behaviour, and portable mode, use the executable directory.
        let relative = copy_custom_icon_at(&portable, &source, "My Kit", "12345678", None).unwrap();
        let legacy = portable.join(&relative);
        assert!(legacy.is_file());
        assert_eq!(
            resolve_custom_icon_path_in_roots(&installed, Some(&portable), &relative).unwrap(),
            legacy,
        );
        assert_eq!(
            resolve_custom_icon_path_at(&portable, &relative).unwrap(),
            legacy
        );

        // Newly saved installed-mode copies go beneath the data directory.
        let saved = copy_custom_icon_at(&installed, &source, "My Kit", "12345678", Some(&relative))
            .unwrap();
        assert_eq!(saved, relative);
        assert!(installed.join(&saved).is_file());
        assert!(
            legacy.is_file(),
            "saving a new copy must not move the existing icon"
        );
        assert_eq!(
            resolve_custom_icon_path_in_roots(&installed, Some(&portable), &saved).unwrap(),
            installed.join(&saved),
        );
        // Cleanup targets the same location as lookup, not the old executable copy.
        remove_unreferenced_custom_icon_in_roots(&installed, Some(&portable), &saved, &[]).unwrap();
        assert!(!installed.join(&saved).exists());
        assert!(legacy.is_file());
        assert_eq!(
            resolve_custom_icon_path_in_roots(&installed, Some(&portable), &relative).unwrap(),
            legacy,
        );
        assert!(
            resolve_custom_icon_path_in_roots(
                &installed,
                Some(&portable),
                Path::new("../outside.png"),
            )
            .is_err()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn custom_layout_accepts_direct_selected_and_nested_roots() {
        let direct = temp_dir("direct");
        fs::create_dir_all(direct.join("tags")).unwrap();
        fs::create_dir_all(direct.join("data")).unwrap();
        fs::create_dir_all(direct.join("archive").join("tags")).unwrap();
        fs::create_dir_all(direct.join("archive").join("data")).unwrap();
        let layout = validate_custom_editing_kit_layout(&direct).unwrap();
        assert_eq!(layout.root, canonical_or_clean(&direct));
        assert_eq!(
            validate_custom_editing_kit_layout(&direct.join("tags"))
                .unwrap()
                .root,
            layout.root
        );

        let outer = temp_dir("nested");
        let nested = outer.join("projects").join("my-kit");
        fs::create_dir_all(nested.join("tags")).unwrap();
        fs::create_dir_all(nested.join("data")).unwrap();
        assert_eq!(
            validate_custom_editing_kit_layout(&outer).unwrap().root,
            canonical_or_clean(&nested)
        );
        let _ = fs::remove_dir_all(direct);
        let _ = fs::remove_dir_all(outer);
    }

    #[test]
    fn validation_cache_changes_only_when_refreshed() {
        let root = temp_dir("cached");
        fs::create_dir_all(root.join("tags")).unwrap();
        let shortcut = EDITING_KIT_SHORTCUTS
            .into_iter()
            .find(|shortcut| shortcut.game == GameId::Halo3)
            .unwrap();
        let paths = HashMap::from([(shortcut.game.as_str().to_owned(), root.clone())]);
        let mut cache = EditingKitValidationCache::new(&paths, &[]);
        assert!(cache.builtin(shortcut).layout().is_some());

        fs::remove_dir_all(root.join("tags")).unwrap();
        assert!(cache.builtin(shortcut).layout().is_some());
        cache.refresh(&paths, &[]);
        assert!(matches!(
            cache.builtin(shortcut),
            EditingKitPathStatus::Invalid(_)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn custom_layout_reports_missing_data_and_ambiguous_projects() {
        let missing = temp_dir("missing-data");
        fs::create_dir_all(missing.join("tags")).unwrap();
        let error = validate_custom_editing_kit_layout(&missing).unwrap_err();
        assert!(error.contains("data directory"), "{error}");

        let ambiguous = temp_dir("ambiguous");
        for name in ["one", "two"] {
            fs::create_dir_all(ambiguous.join(name).join("tags")).unwrap();
            fs::create_dir_all(ambiguous.join(name).join("data")).unwrap();
        }
        let error = validate_custom_editing_kit_layout(&ambiguous).unwrap_err();
        assert!(error.contains("Multiple editing-kit layouts"), "{error}");
        let _ = fs::remove_dir_all(missing);
        let _ = fs::remove_dir_all(ambiguous);
    }

    #[test]
    fn built_in_validation_keeps_existing_tags_only_contract() {
        let root = temp_dir("builtin");
        fs::create_dir_all(root.join("tags")).unwrap();
        let shortcut = EDITING_KIT_SHORTCUTS
            .into_iter()
            .find(|shortcut| shortcut.game == GameId::Halo3)
            .unwrap();
        let status = validate_builtin_editing_kit(shortcut, Some(&root));
        assert!(validate_editing_kit_profile_layout(&root, shortcut.game.as_str()).is_ok());
        // Read on Windows only, but the `expect` is the check everywhere.
        #[cfg_attr(not(windows), allow(unused_variables))]
        let layout = status.layout().expect("built-in layout should be ready");
        #[cfg(windows)]
        assert!(
            !layout.root.to_string_lossy().starts_with(r"\\?\"),
            "verbatim Windows prefix leaked into validated path"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn campaign_evolved_validation_requires_discoverable_paks() {
        let root = temp_dir("campaign-evolved");
        let shortcut = EDITING_KIT_SHORTCUTS
            .into_iter()
            .find(|shortcut| shortcut.game.is_campaign_evolved())
            .unwrap();
        assert!(matches!(
            validate_builtin_editing_kit(shortcut, Some(&root)),
            EditingKitPathStatus::Invalid(_)
        ));

        let paks = root.join("Meteorite").join("Content").join("Paks");
        fs::create_dir_all(&paks).unwrap();
        fs::write(paks.join("campaign.utoc"), []).unwrap();
        assert!(matches!(
            validate_builtin_editing_kit(shortcut, Some(&root)),
            EditingKitPathStatus::Ready(_)
        ));
        assert!(validate_editing_kit_profile_layout(&root, shortcut.game.as_str()).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_custom_roots_use_resolved_layouts() {
        let outer = temp_dir("duplicates");
        let root = outer.join("kit");
        fs::create_dir_all(root.join("tags")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        let profiles = vec![CustomEditingKitProfile {
            read_only: false,
            git_tracked: false,
            id: "existing".to_owned(),
            name: "Existing".to_owned(),
            game: "halo3_mcc".to_owned(),
            root: root.clone(),
            icon: None,
            tags_folder: None,
            data_folder: None,
        }];
        assert!(custom_profile_tags_conflicts(
            &profiles,
            None,
            &canonical_or_clean(&root.join("tags"))
        ));
        assert!(!custom_profile_tags_conflicts(
            &profiles,
            Some("existing"),
            &canonical_or_clean(&root.join("tags"))
        ));
        let _ = fs::remove_dir_all(outer);
    }

    #[test]
    fn icon_paths_are_sanitised_relative_unique_and_validated() {
        assert_eq!(sanitise_project_name("  CON  "), "project");
        assert_eq!(sanitise_project_name("My: Kit / Test"), "My-Kit-Test");
        assert!(safe_custom_icon_relative_path(Path::new(
            "editing kit icons/my-kit-12345678/icon-a.png"
        )));
        assert!(!safe_custom_icon_relative_path(Path::new("../icon.png")));
        assert!(!safe_custom_icon_relative_path(Path::new(
            "editing kit icons/../../icon.png"
        )));

        let base = temp_dir("icons");
        let source = base.join("source.png");
        image::RgbaImage::new(32, 40).save(&source).unwrap();
        assert_eq!(validate_custom_icon_source(&source).unwrap(), (32, 40));
        let relative = copy_custom_icon_at(
            &base,
            &source,
            "My: Kit",
            "12345678-1234-1234-1234-123456789abc",
            None,
        )
        .unwrap();
        assert!(relative.starts_with(CUSTOM_ICON_FOLDER));
        assert!(
            resolve_custom_icon_path_at(&base, &relative)
                .unwrap()
                .is_file()
        );

        let referencing_profile = CustomEditingKitProfile {
            read_only: false,
            git_tracked: false,
            id: "profile".to_owned(),
            name: "Profile".to_owned(),
            game: "halo3_mcc".to_owned(),
            root: base.clone(),
            icon: Some(relative.clone()),
            tags_folder: None,
            data_folder: None,
        };
        remove_unreferenced_custom_icon_at(
            &base,
            &relative,
            std::slice::from_ref(&referencing_profile),
        )
        .unwrap();
        assert!(
            resolve_custom_icon_path_at(&base, &relative)
                .unwrap()
                .is_file()
        );
        remove_unreferenced_custom_icon_at(&base, &relative, &[]).unwrap();
        assert!(
            !resolve_custom_icon_path_at(&base, &relative)
                .unwrap()
                .exists()
        );

        let renamed_relative = copy_custom_icon_at(
            &base,
            &source,
            "Renamed Kit",
            "12345678-1234-1234-1234-123456789abc",
            Some(&relative),
        )
        .unwrap();
        assert_eq!(renamed_relative.parent(), relative.parent());
        let _ = fs::remove_dir_all(base);
    }

    fn kit_with_two_folder_sets(label: &str) -> (PathBuf, PathBuf) {
        let outer = temp_dir(label);
        let root = outer.join("H2EK");
        for folder in ["tags", "data", "tags_moda", "data_moda"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        (outer, canonical_or_clean(&root))
    }

    fn profile(
        id: &str,
        game: &str,
        root: &Path,
        tags: Option<&str>,
        data: Option<&str>,
    ) -> CustomEditingKitProfile {
        CustomEditingKitProfile {
            read_only: false,
            git_tracked: false,
            id: id.to_owned(),
            name: id.to_owned(),
            game: game.to_owned(),
            root: root.to_path_buf(),
            icon: None,
            tags_folder: tags.map(PathBuf::from),
            data_folder: data.map(PathBuf::from),
        }
    }

    #[test]
    fn chosen_folders_resolve_against_the_root_and_must_exist() {
        let (outer, root) = kit_with_two_folder_sets("chosen-folders");
        let chosen = validate_kit_layout(
            &root,
            "halo2_mcc",
            Some(Path::new("tags_moda")),
            Some(Path::new("data_moda")),
        )
        .unwrap();
        // Only one named: the other is the root's own.
        let tags_only =
            validate_kit_layout(&root, "haloce_mcc", Some(Path::new("tags_moda")), None).unwrap();
        // Absolute folders stand on their own.
        let absolute =
            validate_kit_layout(&root, "halo2_mcc", Some(&root.join("tags_moda")), None).unwrap();
        let missing = validate_kit_layout(&root, "halo2_mcc", Some(Path::new("tags_modb")), None)
            .unwrap_err();
        // An engine whose tools can't follow them ignores them.
        let halo3 = validate_kit_layout(
            &root,
            "halo3_mcc",
            Some(Path::new("tags_moda")),
            Some(Path::new("data_moda")),
        )
        .unwrap();
        let _ = fs::remove_dir_all(outer);

        assert_eq!(chosen.root, root);
        assert_eq!(chosen.tags, root.join("tags_moda"));
        assert_eq!(chosen.data, Some(root.join("data_moda")));
        assert_eq!(tags_only.tags, root.join("tags_moda"));
        assert_eq!(tags_only.data, Some(root.join("data")));
        assert_eq!(absolute.tags, root.join("tags_moda"));
        assert!(missing.contains("Tags folder not found"), "{missing}");
        assert_eq!(halo3.tags, root.join("tags"));
    }

    #[test]
    fn only_folders_other_than_the_roots_own_are_stored() {
        let (outer, root) = kit_with_two_folder_sets("stored-folders");
        let elsewhere = outer.join("elsewhere_tags");
        fs::create_dir_all(&elsewhere).unwrap();
        let elsewhere = canonical_or_clean(&elsewhere);
        let own = folder_to_store(&root, Some(&root.join("tags")), "tags");
        let inside = folder_to_store(&root, Some(&root.join("tags_moda")), "tags");
        let outside = folder_to_store(&root, Some(&elsewhere), "tags");
        let _ = fs::remove_dir_all(outer);
        assert_eq!(own, None);
        assert_eq!(inside, Some(PathBuf::from("tags_moda")));
        assert_eq!(outside, Some(elsewhere));
    }

    /// Kits may share a root; they may not share a tags folder.
    #[test]
    fn kits_sharing_a_root_conflict_only_on_a_shared_tags_folder() {
        let (outer, root) = kit_with_two_folder_sets("shared-root");
        let profiles = vec![profile("stock", "halo2_mcc", &root, None, None)];
        let stock_tags = canonical_or_clean(&root.join("tags"));
        let moda_tags = canonical_or_clean(&root.join("tags_moda"));
        let conflicts_with_stock = custom_profile_tags_conflicts(&profiles, None, &stock_tags);
        let conflicts_with_moda = custom_profile_tags_conflicts(&profiles, None, &moda_tags);
        let moda = profile(
            "moda",
            "halo2_mcc",
            &root,
            Some("tags_moda"),
            Some("data_moda"),
        );
        let identity = profile_tags_folder(&moda);
        let _ = fs::remove_dir_all(outer);
        assert!(conflicts_with_stock);
        assert!(!conflicts_with_moda);
        assert_eq!(identity, moda_tags);
    }

    #[test]
    fn the_quick_picks_are_the_roots_matching_folders() {
        let (outer, root) = kit_with_two_folder_sets("candidates");
        let tags = kit_folder_candidates(&root, "tags");
        let data = kit_folder_candidates(&root, "data");
        let default_tags = default_kit_folder_name(&root, "tags");
        let _ = fs::remove_dir_all(outer);
        assert_eq!(tags, vec!["tags".to_owned(), "tags_moda".to_owned()]);
        assert_eq!(data, vec!["data".to_owned(), "data_moda".to_owned()]);
        assert_eq!(default_tags.as_deref(), Some("tags"));
    }
}
