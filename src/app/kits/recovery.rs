//! Profile identity and UI-independent routing for loose-file recovery.
use super::*;
use crate::core::recovery::{self, RecoveryOrigin, RecoverySource};

pub(in crate::app) fn assign_recovery_names(profiles: &mut [CustomEditingKitProfile]) {
    let mut used = HashSet::new();
    // Preserve already-assigned names first, even when an earlier profile is new.
    for profile in profiles.iter_mut() {
        if profile.recovery.name.is_empty() {
            continue;
        }
        if recovery::safe_name(&profile.recovery.name) != profile.recovery.name
            || !used.insert((profile.game.clone(), profile.recovery.name.clone()))
        {
            profile.recovery.name.clear();
        }
    }
    for profile in profiles {
        if !profile.recovery.name.is_empty() {
            continue;
        }
        let base = recovery::safe_name(&profile.name);
        let mut name = base.clone();
        let mut suffix = 1;
        while !used.insert((profile.game.clone(), name.clone())) {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }
        profile.recovery.name = name;
    }
}

pub(in crate::app) fn profile_recovery_folder(profile: &CustomEditingKitProfile) -> PathBuf {
    let name = if profile.recovery.name.is_empty() {
        recovery::safe_name(&profile.name)
    } else {
        profile.recovery.name.clone()
    };
    match &profile.recovery.folder {
        Some(folder) => profile.root.join(folder),
        None => recovery::default_folder(
            &crate::core::storage::data_path(""),
            Some(&profile.game),
            Some(&name),
        ),
    }
}

pub(in crate::app) fn profile_tags_root(profile: &CustomEditingKitProfile) -> PathBuf {
    validate_kit_layout(
        &profile.root,
        &profile.game,
        profile.tags_folder.as_deref(),
        profile.data_folder.as_deref(),
    )
    .map(|layout| layout.tags)
    .unwrap_or_else(|_| {
        profile
            .root
            .join(profile.tags_folder.as_deref().unwrap_or(Path::new("tags")))
    })
}

pub(in crate::app) fn draft_recovery_folder(draft: &CustomEditingKitDraft) -> PathBuf {
    if let Some(folder) = &draft.recovery.folder {
        let root = PathBuf::from(draft.root_input.trim());
        return canonical_or_clean(&root).join(folder);
    }
    let name = if draft.recovery.name.is_empty() {
        recovery::safe_name(&draft.name)
    } else {
        draft.recovery.name.clone()
    };
    recovery::default_folder(
        &crate::core::storage::data_path(""),
        Some(&draft.game),
        Some(&name),
    )
}

pub(in crate::app) fn draft_recovery_error(draft: &CustomEditingKitDraft) -> Option<String> {
    if draft.is_campaign_evolved() {
        return None;
    }
    if draft
        .recovery
        .folder
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Some("Enter a folder or choose Reset to Default".into());
    }
    if draft.root_input.trim().is_empty()
        && draft
            .recovery
            .folder
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
    {
        return Some("Choose the editing kit root before using a relative recovery folder".into());
    }
    let root = PathBuf::from(draft.root_input.trim());
    // Existing root/layout validation owns errors for a missing or invalid kit.
    if !root.is_dir() {
        return None;
    }
    let choosable = kit_folders_are_choosable(&draft.game);
    let tags = (choosable && !draft.tags_folder_input.trim().is_empty())
        .then(|| PathBuf::from(draft.tags_folder_input.trim()));
    let data = (choosable && !draft.data_folder_input.trim().is_empty())
        .then(|| PathBuf::from(draft.data_folder_input.trim()));
    let root = canonical_or_clean(&root);
    let tags = validate_kit_layout(&root, &draft.game, tags.as_deref(), data.as_deref())
        .map(|layout| layout.tags)
        .unwrap_or_else(|_| root.join(tags.as_deref().unwrap_or(Path::new("tags"))));
    recovery::validate_folder(&draft_recovery_folder(draft), &[tags]).err()
}

pub(in crate::app) fn refresh_draft_recovery(
    draft: &mut CustomEditingKitDraft,
    profiles: &[CustomEditingKitProfile],
) {
    let base = if draft.editing_id.is_none() || draft.recovery.name.is_empty() {
        recovery::safe_name(&draft.name)
    } else {
        draft.recovery.name.clone()
    };
    let mut name = base.clone();
    let mut suffix = 1;
    while profiles.iter().any(|p| {
        Some(p.id.as_str()) != draft.editing_id.as_deref()
            && p.game == draft.game
            && p.recovery.name == name
    }) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    draft.recovery.name = name;
    if draft.recovery.folder.is_none() {
        draft.recovery_input.clear();
    }
}

/// Abbreviations are presentation only; stored defaults still follow Baboon's
/// actual storage mode and use native filesystem paths.
pub(in crate::app) fn abbreviated_path_display(path: &Path) -> String {
    #[cfg(windows)]
    let (base, alias) = (std::env::var_os("APPDATA").map(PathBuf::from), "%APPDATA%");
    #[cfg(not(windows))]
    let (base, alias) = (std::env::var_os("HOME").map(PathBuf::from), "~");
    abbreviated_path(path, base.as_deref(), alias)
}

fn abbreviated_path(path: &Path, base: Option<&Path>, alias: &str) -> String {
    if let Some(relative) = base.and_then(|base| path.strip_prefix(base).ok()) {
        if relative.as_os_str().is_empty() {
            return alias.into();
        }
        return format!("{alias}{}{}", std::path::MAIN_SEPARATOR, relative.display());
    }
    path.display().to_string()
}

pub(in crate::app) fn validate_draft_recovery(
    draft: &CustomEditingKitDraft,
    profiles: &[CustomEditingKitProfile],
) -> Option<String> {
    if let Some(error) = draft_recovery_error(draft) {
        return Some(error);
    }
    if draft.is_campaign_evolved() {
        return None;
    }
    let roots: Vec<_> = profiles
        .iter()
        .filter(|p| !p.is_campaign_evolved())
        .map(profile_tags_root)
        .filter(|root| recovery::resolved_path(root).is_ok())
        .collect();
    recovery::validate_folder(&draft_recovery_folder(draft), &roots).err()
}

/// Validate before touching disk, then materialize an explicitly chosen location
/// so a successful Save leaves the folder ready to inspect.
pub(in crate::app) fn prepare_draft_recovery_folder(
    draft: &CustomEditingKitDraft,
    profiles: &[CustomEditingKitProfile],
    loaded_roots: &[PathBuf],
) -> Result<(), String> {
    if let Some(error) = validate_draft_recovery(draft, profiles) {
        return Err(error);
    }
    if draft.is_campaign_evolved() {
        return Ok(());
    }
    let folder = recovery::validate_folder(&draft_recovery_folder(draft), loaded_roots)?;
    if draft.recovery.folder.is_some() {
        recovery::prepare_custom_location(&crate::core::storage::data_path("deleted-tags"), &folder)?;
        fs::create_dir_all(&folder)
            .map_err(|error| format!("Could not create Deleted Tags Folder at {}: {error}", folder.display()))?;
        // Resolve again after creation, matching the checks made when opening it.
        if let Some(error) = validate_draft_recovery(draft, profiles) {
            return Err(error);
        }
        recovery::validate_folder(&draft_recovery_folder(draft), loaded_roots)?;
    }
    Ok(())
}

impl Baboon {
    pub(in crate::app) fn prepare_loose_recovery_target(
        &self,
        index: usize,
    ) -> Result<(PathBuf, RecoveryOrigin), String> {
        let target = self.loose_recovery_context(index)?;
        let custom = target
            .1
            .kit_id
            .as_ref()
            .and_then(|id| {
                self.model
                    .prefs
                    .custom_editing_kit_profiles
                    .iter()
                    .find(|p| &p.id == id)
            })
            .is_some_and(|p| p.recovery.folder.is_some());
        if custom {
            recovery::prepare_custom_location(
                &crate::core::storage::data_path("deleted-tags"),
                &target.0,
            )?;
        } else {
            recovery::migrate_legacy_game(
                &crate::core::storage::data_path("deleted-tags"),
                target.1.game.as_deref().unwrap_or("unknown-game"),
            )?;
        }
        Ok(target)
    }
    pub(in crate::app) fn loose_recovery_context(
        &self,
        index: usize,
    ) -> Result<(PathBuf, RecoveryOrigin), String> {
        let kit = &self.model.kits[index];
        let (root, kind) = match kit.source.as_ref().map(|source| &source.source) {
            Some(TagSource::LooseFolder { root, .. }) => (root.clone(), RecoverySource::TagsFolder),
            Some(TagSource::SingleFile { path }) => (
                path.parent()
                    .ok_or("Tag file has no parent folder")?
                    .to_path_buf(),
                RecoverySource::SingleFile,
            ),
            _ => {
                return Err(
                    "Deleted Tags Folder requires a loaded loose tag or tags folder".into(),
                );
            }
        };
        let root = canonical_or_clean(&root);
        let game = kit
            .source
            .as_ref()
            .and_then(|source| source.game)
            .map(|game| game.as_str().to_owned());
        let profile = kit.profile.as_ref().and_then(|identity| {
            self.model
                .prefs
                .custom_editing_kit_profiles
                .iter()
                .find(|profile| profile.id == identity.id)
        });
        let base = profile.map(profile_recovery_folder).unwrap_or_else(|| {
            recovery::default_folder(&crate::core::storage::data_path(""), game.as_deref(), None)
        });
        let mut roots = match kind {
            RecoverySource::TagsFolder => vec![root.clone()],
            RecoverySource::SingleFile => Vec::new(),
        };
        roots.extend(
            self.model
                .prefs
                .custom_editing_kit_profiles
                .iter()
                .filter(|profile| !profile.is_campaign_evolved())
                .map(profile_tags_root)
                .filter(|root| recovery::resolved_path(root).is_ok()),
        );
        for loaded in 0..self.model.kits.len() {
            if let Some(root) = self.model.loaded_tags_root_for(loaded) {
                roots.push(root);
            }
        }
        // Missing profiles need not block recovery in a valid loaded kit.
        roots.retain(|root| root.is_absolute());
        let base = recovery::validate_folder(&base, &roots)?;
        Ok((
            base,
            RecoveryOrigin {
                source: kind,
                tags_root: root,
                game,
                kit_id: profile.map(|p| p.id.clone()),
                kit_name: profile.map(|p| p.name.clone()),
            },
        ))
    }

    pub(in crate::app) fn open_deleted_tags_folder(&mut self) {
        let result = (|| {
            let (folder, _) = self.prepare_loose_recovery_target(self.model.active)?;
            fs::create_dir_all(&folder)
                .map_err(|e| format!("Could not create Deleted Tags Folder: {e}"))?;
            self.loose_recovery_context(self.model.active)
                .map(|(folder, _)| folder)
        })();
        match result {
            Ok(folder) => self.open_folder_in_explorer(clean_recent_path(folder), "Deleted Tags"),
            Err(error) => self.model.status = error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_custom_recovery_folder_creates_missing_parents_and_keeps_existing_files() {
        let root = std::env::temp_dir().join(format!("baboon-recovery-save-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("tags")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        let mut draft = CustomEditingKitDraft::new();
        draft.game = "halo2_mcc".into();
        draft.root_input = root.display().to_string();
        draft.recovery.folder = Some("deleted/archive".into());
        let folder = root.join("deleted/archive");
        assert!(!folder.exists());
        prepare_draft_recovery_folder(&draft, &[], &[]).unwrap();
        assert!(folder.is_dir());
        fs::write(folder.join("existing-tag"), b"keep me").unwrap();
        prepare_draft_recovery_folder(&draft, &[], &[]).unwrap();
        assert_eq!(fs::read(folder.join("existing-tag")).unwrap(), b"keep me");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saving_invalid_or_uncreatable_recovery_folder_returns_an_error() {
        let root = std::env::temp_dir().join(format!("baboon-recovery-save-error-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("tags")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        let mut draft = CustomEditingKitDraft::new();
        draft.game = "halo2_mcc".into();
        draft.root_input = root.display().to_string();
        draft.recovery.folder = Some("tags/deleted".into());
        assert!(prepare_draft_recovery_folder(&draft, &[], &[]).is_err());
        assert!(!root.join("tags/deleted").exists());
        draft.recovery.folder = Some("other-tags/deleted".into());
        assert!(prepare_draft_recovery_folder(&draft, &[], &[root.join("other-tags")]).is_err());
        assert!(!root.join("other-tags").exists());
        fs::write(root.join("blocked"), b"a file").unwrap();
        draft.recovery.folder = Some("blocked/deleted".into());
        assert!(prepare_draft_recovery_folder(&draft, &[], &[]).is_err());
        assert_eq!(fs::read(root.join("blocked")).unwrap(), b"a file");
        fs::remove_dir_all(root).unwrap();
    }

    fn profile(name: &str, id: &str, root: PathBuf) -> CustomEditingKitProfile {
        CustomEditingKitProfile {
            recovery: Default::default(),
            id: id.into(),
            name: name.into(),
            game: "halo2_mcc".into(),
            root,
            tags_folder: None,
            data_folder: None,
            icon: None,
            read_only: false,
            git_tracked: false,
        }
    }

    #[test]
    fn recovery_names_are_assigned_from_display_names_and_survive_renames() {
        let mut profiles = vec![
            profile("Halo 2 Rebalance", "one", "same_install".into()),
            profile("Halo 2 Rebalance", "two", "another_install".into()),
        ];
        assign_recovery_names(&mut profiles);
        assert_eq!(profiles[0].recovery.name, "halo_2_rebalance");
        assert_eq!(profiles[1].recovery.name, "halo_2_rebalance_1");
        profiles[0].name = "Renamed".into();
        assign_recovery_names(&mut profiles);
        assert_eq!(profiles[0].recovery.name, "halo_2_rebalance");
        let before = profile_recovery_folder(&profiles[0]);
        profiles[0].game = "halo3_mcc".into();
        assert_ne!(profile_recovery_folder(&profiles[0]), before);
        assert!(profile_recovery_folder(&profiles[0]).ends_with("halo3_mcc/halo_2_rebalance"));
    }

    #[test]
    fn recovery_default_is_an_empty_input_with_a_display_only_hint() {
        let mut profiles = vec![profile("Rebalance", "one", "kit".into())];
        assign_recovery_names(&mut profiles);
        let mut draft = CustomEditingKitDraft::from_profile(&profiles[0]);
        let default = draft_recovery_folder(&draft);
        refresh_draft_recovery(&mut draft, &profiles);
        assert!(draft.recovery_input.is_empty() && draft.recovery.folder.is_none());
        assert_eq!(draft_recovery_folder(&draft), default);
        draft.recovery.folder = Some("deleted-tags".into());
        draft.recovery_input = "deleted-tags".into();
        refresh_draft_recovery(&mut draft, &profiles);
        assert_eq!(draft.recovery_input, "deleted-tags");
        let base = Path::new("home");
        assert_eq!(
            abbreviated_path(&base.join("baboon/deleted-tags"), Some(base), "~"),
            format!(
                "~{}{}",
                std::path::MAIN_SEPARATOR,
                Path::new("baboon/deleted-tags").display()
            )
        );
        let portable = Path::new("portable/deleted-tags");
        assert_eq!(
            abbreviated_path(portable, Some(base), "%APPDATA%"),
            portable.display().to_string()
        );
    }

    #[test]
    fn recovery_paths_validate_other_kits_and_follow_relative_root_changes() {
        let outer =
            std::env::temp_dir().join(format!("baboon-recovery-profiles-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(outer.join("first/tags")).unwrap();
        fs::create_dir_all(outer.join("second/tags")).unwrap();
        let outer = canonical_or_clean(&outer);
        let p = profile("First", "one", outer.join("first"));
        let mut draft = CustomEditingKitDraft::from_profile(&p);
        draft.recovery.folder = Some("deleted-tags".into());
        assert_eq!(
            draft_recovery_folder(&draft),
            outer.join("first/deleted-tags")
        );
        assert!(draft_recovery_error(&draft).is_none());
        draft.root_input = outer.join("second").display().to_string();
        assert_eq!(
            draft_recovery_folder(&draft),
            outer.join("second/deleted-tags")
        );
        draft.recovery.folder = Some("tags/deleted".into());
        assert!(draft_recovery_error(&draft).is_some());
        draft.recovery.folder = Some(outer.join("first/tags/deleted"));
        assert!(validate_draft_recovery(&draft, &[p.clone()]).is_some());
        draft.recovery.folder = Some(outer.join("shared"));
        assert!(validate_draft_recovery(&draft, &[p]).is_none());
        let absolute = draft_recovery_folder(&draft);
        draft.root_input = outer.join("first").display().to_string();
        assert_eq!(draft_recovery_folder(&draft), absolute);
        fs::remove_dir_all(outer).unwrap();
    }
}
