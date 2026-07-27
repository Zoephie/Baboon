//! Editing-kit profile validation and executable-relative custom icon storage.

use super::*;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

pub(super) const CUSTOM_ICON_FOLDER: &str = "editing kit icons";
pub(super) const RECOMMENDED_CUSTOM_ICON_SIZE: u32 = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EditingKitLayout {
    pub(super) root: PathBuf,
    pub(super) tags: PathBuf,
    pub(super) data: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EditingKitPathStatus {
    Unconfigured,
    Ready(EditingKitLayout),
    Invalid(String),
}

#[derive(Clone, Debug, Default)]
pub(super) struct EditingKitValidationCache {
    built_ins: HashMap<String, EditingKitPathStatus>,
    custom_layouts: HashMap<String, Result<EditingKitLayout, String>>,
    custom_icon_errors: HashMap<String, Option<String>>,
}

impl EditingKitValidationCache {
    pub(super) fn new(
        paths: &HashMap<String, PathBuf>,
        profiles: &[CustomEditingKitProfile],
    ) -> Self {
        let mut cache = Self::default();
        cache.refresh(paths, profiles);
        cache
    }

    pub(super) fn refresh(
        &mut self,
        paths: &HashMap<String, PathBuf>,
        profiles: &[CustomEditingKitProfile],
    ) {
        self.built_ins = EDITING_KIT_SHORTCUTS
            .into_iter()
            .map(|shortcut| {
                (
                    shortcut.game.to_owned(),
                    validate_builtin_editing_kit(
                        shortcut,
                        paths.get(shortcut.game).map(PathBuf::as_path),
                    ),
                )
            })
            .collect();
        self.custom_layouts = profiles
            .iter()
            .map(|profile| {
                (
                    profile.id.clone(),
                    validate_custom_editing_kit_layout(&profile.root),
                )
            })
            .collect();
        self.custom_icon_errors = profiles
            .iter()
            .map(|profile| {
                (
                    profile.id.clone(),
                    custom_profile_icon_error(profile),
                )
            })
            .collect();
    }

    pub(super) fn refresh_builtin(
        &mut self,
        shortcut: EditingKitShortcut,
        configured: Option<&Path>,
    ) -> EditingKitPathStatus {
        let status = validate_builtin_editing_kit(shortcut, configured);
        self.built_ins
            .insert(shortcut.game.to_owned(), status.clone());
        status
    }

    pub(super) fn refresh_custom(
        &mut self,
        profile: &CustomEditingKitProfile,
    ) -> Result<EditingKitLayout, String> {
        let status = validate_custom_editing_kit_layout(&profile.root);
        self.custom_layouts
            .insert(profile.id.clone(), status.clone());
        self.custom_icon_errors
            .insert(profile.id.clone(), custom_profile_icon_error(profile));
        status
    }

    pub(super) fn builtin(&self, shortcut: EditingKitShortcut) -> EditingKitPathStatus {
        self.built_ins
            .get(shortcut.game)
            .cloned()
            .unwrap_or(EditingKitPathStatus::Unconfigured)
    }

    pub(super) fn custom(&self, profile_id: &str) -> Result<EditingKitLayout, String> {
        self.custom_layouts
            .get(profile_id)
            .cloned()
            .unwrap_or_else(|| Err("Editing-kit status has not been refreshed".to_owned()))
    }

    pub(super) fn custom_icon_error(&self, profile_id: &str) -> Option<&str> {
        self.custom_icon_errors
            .get(profile_id)
            .and_then(Option::as_deref)
    }
}

impl Baboon {
    pub(super) fn refresh_editing_kit_validation(&mut self) {
        self.editing_kit_validation
            .refresh(&self.editing_kit_paths, &self.custom_editing_kit_profiles);
        self.custom_editing_kit_texture_failures.clear();
    }

    pub(super) fn refresh_builtin_editing_kit_validation(
        &mut self,
        shortcut: EditingKitShortcut,
    ) -> EditingKitPathStatus {
        self.editing_kit_validation.refresh_builtin(
            shortcut,
            self.editing_kit_paths
                .get(shortcut.game)
                .map(PathBuf::as_path),
        )
    }
}

impl EditingKitPathStatus {
    pub(super) fn layout(&self) -> Option<&EditingKitLayout> {
        match self {
            Self::Ready(layout) => Some(layout),
            Self::Unconfigured | Self::Invalid(_) => None,
        }
    }

    pub(super) fn message(&self) -> String {
        match self {
            Self::Unconfigured => "Not configured".to_owned(),
            Self::Ready(layout) => format!("Ready: {}", layout.root.display()),
            Self::Invalid(error) => error.clone(),
        }
    }
}

pub(super) fn validate_builtin_editing_kit(
    shortcut: EditingKitShortcut,
    configured: Option<&Path>,
) -> EditingKitPathStatus {
    let Some(path) = configured.filter(|path| !path.as_os_str().is_empty()) else {
        return EditingKitPathStatus::Unconfigured;
    };
    if shortcut.game == "haloce_evolved" {
        return match crate::source::find_paks_dir(path) {
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

pub(super) fn validate_custom_editing_kit_layout(path: &Path) -> Result<EditingKitLayout, String> {
    validate_loose_editing_kit_layout(path, true)
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

fn push_layout_candidate(
    candidates: &mut Vec<EditingKitLayout>,
    root: &Path,
    require_data: bool,
) {
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

pub(super) fn canonical_or_clean(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| clean_recent_path(path.to_path_buf()))
}

pub(super) fn custom_profile_root_conflicts(
    profiles: &[CustomEditingKitProfile],
    editing_profile_id: Option<&str>,
    resolved_root: &Path,
) -> bool {
    profiles.iter().any(|profile| {
        Some(profile.id.as_str()) != editing_profile_id
            && validate_custom_editing_kit_layout(&profile.root)
                .is_ok_and(|layout| same_recent_path(&layout.root, resolved_root))
    })
}

pub(super) fn executable_directory() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the Baboon executable: {error}"))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "The Baboon executable has no parent directory".to_owned())
}

pub(super) fn resolve_custom_icon_path(relative: &Path) -> Result<PathBuf, String> {
    resolve_custom_icon_path_at(&executable_directory()?, relative)
}

fn resolve_custom_icon_path_at(base: &Path, relative: &Path) -> Result<PathBuf, String> {
    if !safe_custom_icon_relative_path(relative) {
        return Err("Saved custom icon path is unsafe".to_owned());
    }
    Ok(base.join(relative))
}

pub(super) fn custom_profile_icon_error(
    profile: &CustomEditingKitProfile,
) -> Option<String> {
    let relative = profile.icon.as_deref()?;
    let absolute = match resolve_custom_icon_path(relative) {
        Ok(path) => path,
        Err(error) => return Some(error),
    };
    validate_custom_icon_source(&absolute)
        .err()
        .map(|error| format!("Using the default icon: {error}"))
}

pub(super) fn validate_custom_icon_source(path: &Path) -> Result<(u32, u32), String> {
    let png_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
    if !png_extension {
        return Err("Custom project icons must be PNG files".to_owned());
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not read custom project icon: {error}"))?;
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map_err(|error| format!("Selected file is not a readable PNG: {error}"))?;
    Ok((image.width(), image.height()))
}

pub(super) fn copy_custom_icon(
    source: &Path,
    project_name: &str,
    profile_id: &str,
    existing_icon: Option<&Path>,
) -> Result<PathBuf, String> {
    copy_custom_icon_at(
        &executable_directory()?,
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
    let bytes = fs::read(source)
        .map_err(|error| format!("Could not read custom project icon: {error}"))?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let short_id = profile_id.chars().filter(|ch| *ch != '-').take(8).collect::<String>();
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
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Could not create custom icon folder beside the executable: {error}"
        )
    })?;
    if !destination.is_file() {
        let temporary = parent.join(format!(".icon-{}.tmp", &hash[..12]));
        fs::write(&temporary, &bytes)
            .map_err(|error| format!("Could not write custom icon beside the executable: {error}"))?;
        fs::rename(&temporary, &destination).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            format!("Could not finish writing the custom icon: {error}")
        })?;
    }
    Ok(relative)
}

pub(super) fn remove_unreferenced_custom_icon(
    relative: &Path,
    profiles: &[CustomEditingKitProfile],
) -> Result<(), String> {
    remove_unreferenced_custom_icon_at(
        &executable_directory()?,
        relative,
        profiles,
    )
}

fn remove_unreferenced_custom_icon_at(
    base: &Path,
    relative: &Path,
    profiles: &[CustomEditingKitProfile],
) -> Result<(), String> {
    if profiles
        .iter()
        .any(|profile| profile.icon.as_deref() == Some(relative))
    {
        return Ok(());
    }
    let absolute = resolve_custom_icon_path_at(base, relative)?;
    if absolute.is_file() {
        fs::remove_file(&absolute)
            .map_err(|error| format!("Profile was saved, but its old icon could not be deleted: {error}"))?;
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

pub(super) fn safe_custom_icon_relative_path(path: &Path) -> bool {
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

pub(super) fn sanitise_project_name(name: &str) -> String {
    let mut output = String::new();
    let mut previous_separator = false;
    for ch in name.trim().chars() {
        let invalid = ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*');
        let mapped = if invalid || ch.is_whitespace() { '-' } else { ch };
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
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
        "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
        "LPT7", "LPT8", "LPT9",
    ];
    if output.is_empty() || reserved.iter().any(|item| item.eq_ignore_ascii_case(&output)) {
        "project".to_owned()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("baboon-editing-kits-{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
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
            .find(|shortcut| shortcut.game == "halo3_mcc")
            .unwrap();
        let paths = HashMap::from([(shortcut.game.to_owned(), root.clone())]);
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
            .find(|shortcut| shortcut.game == "halo3_mcc")
            .unwrap();
        assert!(matches!(
            validate_builtin_editing_kit(shortcut, Some(&root)),
            EditingKitPathStatus::Ready(_)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn campaign_evolved_validation_requires_discoverable_paks() {
        let root = temp_dir("campaign-evolved");
        let shortcut = EDITING_KIT_SHORTCUTS
            .into_iter()
            .find(|shortcut| shortcut.game == "haloce_evolved")
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
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_custom_roots_use_resolved_layouts() {
        let outer = temp_dir("duplicates");
        let root = outer.join("kit");
        fs::create_dir_all(root.join("tags")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        let profiles = vec![CustomEditingKitProfile {
            id: "existing".to_owned(),
            name: "Existing".to_owned(),
            game: "halo3_mcc".to_owned(),
            root: root.clone(),
            icon: None,
        }];
        assert!(custom_profile_root_conflicts(
            &profiles,
            None,
            &canonical_or_clean(&root)
        ));
        assert!(!custom_profile_root_conflicts(
            &profiles,
            Some("existing"),
            &canonical_or_clean(&root)
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
        assert!(resolve_custom_icon_path_at(&base, &relative).unwrap().is_file());

        let referencing_profile = CustomEditingKitProfile {
            id: "profile".to_owned(),
            name: "Profile".to_owned(),
            game: "halo3_mcc".to_owned(),
            root: base.clone(),
            icon: Some(relative.clone()),
        };
        remove_unreferenced_custom_icon_at(
            &base,
            &relative,
            std::slice::from_ref(&referencing_profile),
        )
        .unwrap();
        assert!(resolve_custom_icon_path_at(&base, &relative).unwrap().is_file());
        remove_unreferenced_custom_icon_at(&base, &relative, &[]).unwrap();
        assert!(!resolve_custom_icon_path_at(&base, &relative).unwrap().exists());

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
}
