//! Rename Folder for a loose tags folder: rename it in place and rewrite every
//! reference to the tags beneath it.
//! It owns the dialog's preview counts and naming rules; the work itself is the
//! folder move job, run into the folder's own parent with a new leaf.

use super::*;
use crate::app::tag_ops::duplicate::validate_leaf_characters;
use std::collections::BTreeSet;

/// Validate a new name for a loose folder against its current name and the
/// names already in its parent (files and folders alike).
///
/// Tag paths ignore case, so a sibling that differs only in case is a
/// conflict, and a change of case alone is refused: no reference would change,
/// and on a case-insensitive file system the folder would collide with itself.
pub(in crate::app) fn validate_loose_folder_rename(
    raw: &str,
    old_name: &str,
    siblings: &[String],
) -> Result<String, String> {
    let name = validate_leaf_characters(raw, "Folder names", "Enter a folder name")?;
    if name == old_name {
        return Err("That is already the folder's name".to_owned());
    }
    if name.eq_ignore_ascii_case(old_name) {
        return Err(
            "Tag paths ignore case, so changing only the case would not change any reference"
                .to_owned(),
        );
    }
    if siblings
        .iter()
        .filter(|sibling| !sibling.eq_ignore_ascii_case(old_name))
        .any(|sibling| sibling.eq_ignore_ascii_case(&name))
    {
        return Err("Something with that name already exists in this folder".to_owned());
    }
    Ok(name)
}

/// Every name directly inside `dir`, files and folders.
fn directory_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// A sibling of `path` whose name matches its leaf ignoring case but not
/// exactly — a conflict for tag paths that `Path::exists` misses on a
/// case-sensitive file system.
pub(in crate::app) fn sibling_differing_in_case(path: &Path) -> Option<PathBuf> {
    let leaf = path.file_name()?.to_string_lossy().into_owned();
    let parent = path.parent()?;
    directory_names(parent)
        .into_iter()
        .find(|name| *name != leaf && name.eq_ignore_ascii_case(&leaf))
        .map(|name| parent.join(name))
}

/// The tags outside a folder that reference one inside it, by key.
///
/// `inside` is every tag in the folder. A referrer that is itself inside is
/// left out: it moves with the folder and is rewritten either way.
pub(in crate::app) fn outside_referrer_keys(
    inside: &[TagEntry],
    index: &ReverseDependencyIndex,
    names: &TagNameIndex,
) -> BTreeSet<String> {
    let inside_keys = inside
        .iter()
        .map(|entry| entry.key.as_str())
        .collect::<HashSet<_>>();
    let mut out = BTreeSet::new();
    for entry in inside {
        let Some(rel) = dependency_entry_reference_path(entry, names) else {
            continue;
        };
        for key in index.dependents_for(entry.group_tag, &rel) {
            if !inside_keys.contains(key.as_str()) {
                out.insert(key.clone());
            }
        }
    }
    out
}

impl Baboon {
    /// Open Rename Folder for a loose folder, with what it would change
    /// counted up front.
    pub(in crate::app) fn open_loose_folder_rename(&mut self, rel_path: PathBuf, label: String) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        if self.tag_ops.folder_refactor.is_some() {
            self.model.status = "A folder move/rename is already running".to_owned();
            return;
        }
        let Some(root) = self.model.loaded_tags_root() else {
            self.model.status = "Rename requires a loaded tags folder".to_owned();
            return;
        };
        let rel_path = match validate_relative_folder_path(&rel_path) {
            Ok(rel_path) => rel_path,
            Err(_) => {
                self.model.status = "The tags root itself cannot be renamed".to_owned();
                return;
            }
        };
        let Some(old_name) = rel_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            self.model.status = format!("Cannot rename {label}");
            return;
        };
        // From the workspace's tag list when it holds every tag: walking a
        // large folder on disk here stalls the window (Halo 3's `objects`,
        // 15,555 tags, took 0.3 s on a warm cache). Otherwise from disk, the
        // way the job itself finds them.
        let names = self.model.names().clone();
        let folder = root.join(&rel_path);
        let loaded = self
            .model.source()
            .filter(|source| !source.all_entries.is_empty())
            .map(|source| {
                source
                    .all_entries
                    .iter()
                    .filter(|entry| {
                        matches!(&entry.location, TagEntryLocation::LooseFile(path)
                            if path.starts_with(&folder))
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            });
        let inside = match loaded {
            Some(entries) => entries,
            None => match scan_folder_subtree_entries(&root, &rel_path, &names) {
                Ok(entries) => entries,
                Err(error) => {
                    self.model.status = format!("Could not scan {label}: {error}");
                    return;
                }
            },
        };
        let outside_referrers = self.model.source().and_then(|source| {
            let index = source.reverse_dependencies.as_ref()?;
            let keys = outside_referrer_keys(&inside, index, &names);
            let mut paths = source
                .full_entry_set()
                .iter()
                .filter(|entry| keys.contains(&entry.key))
                .map(|entry| entry.display_path.replace('\\', "/"))
                .collect::<Vec<_>>();
            paths.sort_by_cached_key(|path| crate::core::source::natural_key(path));
            Some(paths)
        });
        let parent_display = rel_path
            .parent()
            .map(|parent| parent.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        self.dialogs.open(LooseFolderRenameState {
            kit: self.model.active_kit_id(),
            rel_path,
            parent_display,
            name_input: old_name.clone(),
            old_name,
            focus_input: true,
            error: None,
            tag_count: inside.len(),
            outside_referrers,
        });
    }

    /// Apply the open Rename Folder dialog. Returns whether it should close:
    /// a rejected name keeps it open with the reason beside the field.
    pub(in crate::app) fn apply_loose_folder_rename(&mut self) -> bool {
        let Some(state) = self.dialogs.get::<LooseFolderRenameState>() else {
            return true;
        };
        let kit = state.kit;
        let rel_path = state.rel_path.clone();
        let old_name = state.old_name.clone();
        let raw = state.name_input.clone();
        if !self.focus_navigation_kit(kit) {
            self.model.status = "The workspace this rename came from is closed".to_owned();
            return true;
        }
        if self.refuse_read_only_edit(self.model.active) {
            return true;
        }
        let Some(root) = self.model.loaded_tags_root() else {
            self.model.status = "Rename requires a loaded tags folder".to_owned();
            return true;
        };
        let reject = |this: &mut Self, error: String| {
            if let Some(state) = this.dialogs.get_mut::<LooseFolderRenameState>() {
                state.error = Some(error);
            }
            false
        };
        if self.tag_ops.folder_refactor.is_some() {
            return reject(self, "A folder move/rename is already running".to_owned());
        }
        if self.model.kits[self.model.active]
            .parsed_tags
            .values()
            .any(|doc| doc.dirty.is_set())
        {
            return reject(
                self,
                "Save or close tags with unsaved changes before renaming a folder".to_owned(),
            );
        }
        let source = root.join(&rel_path);
        let Some(parent) = source.parent().map(Path::to_path_buf) else {
            return reject(self, "The tags root itself cannot be renamed".to_owned());
        };
        let name = match validate_loose_folder_rename(&raw, &old_name, &directory_names(&parent)) {
            Ok(name) => name,
            Err(error) => return reject(self, error),
        };
        self.spawn_folder_refactor(
            root,
            rel_path,
            parent,
            Some(name.clone()),
            true,
            format!("Renaming {old_name} to {name}"),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn siblings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn empty_folder_rename_is_rejected_without_moving_the_folder() {
        let root = crate::test_kits::unique_temp_dir("empty-folder-rename");
        fs::create_dir_all(root.join("old/nested")).unwrap();
        let (tx, _rx) = mpsc::channel();
        let result = run_folder_refactor_job(
            root.clone(),
            "old".into(),
            root.clone(),
            Some("new".into()),
            true,
            "Renaming".into(),
            TagNameIndex::default(),
            None,
            Vec::new(),
            None,
            &tx,
        );
        assert!(matches!(result, Err(error) if error == "No tags found in that folder"));
        assert!(root.join("old/nested").is_dir());
        assert!(!root.join("new").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_name_is_accepted_and_trimmed() {
        assert_eq!(
            validate_loose_folder_rename("  shadow", "creep", &siblings(&["creep", "ghost"])),
            Ok("shadow".to_owned())
        );
    }

    #[test]
    fn the_same_name_and_a_case_only_change_are_refused() {
        let here = siblings(&["creep"]);
        assert!(validate_loose_folder_rename("creep", "creep", &here).is_err());
        assert!(validate_loose_folder_rename("Creep", "creep", &here).is_err());
    }

    #[test]
    fn a_sibling_folder_or_file_conflicts_ignoring_case() {
        let here = siblings(&["creep", "Ghost", "shadow.vehicle"]);
        assert!(validate_loose_folder_rename("ghost", "creep", &here).is_err());
        // A dotted name is refused before the sibling check ever sees it, but
        // a plain one that matches a file's full name is still a conflict.
        let here = siblings(&["creep", "readme"]);
        assert!(validate_loose_folder_rename("README", "creep", &here).is_err());
    }

    #[test]
    fn separators_and_illegal_characters_are_refused() {
        let here = siblings(&["creep"]);
        for bad in ["a/b", "a\\b", "", "  ", "..", "a:b", "x.y", "CON"] {
            assert!(
                validate_loose_folder_rename(bad, "creep", &here).is_err(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn outside_referrers_exclude_tags_inside_the_folder() {
        let names = TagNameIndex::default();
        let vehicle = parse_group_tag("vehi").unwrap();
        let model = parse_group_tag("hlmt").unwrap();
        let inside_vehicle = TagEntry {
            key: "file:tags/objects/vehicles/creep/shadow.vehicle".to_owned(),
            display_path: "objects/vehicles/creep/shadow.vehicle".to_owned(),
            group_tag: vehicle,
            group_name: Some("vehicle".to_owned()),
            location: TagEntryLocation::LooseFile(PathBuf::from(
                "tags/objects/vehicles/creep/shadow.vehicle",
            )),
        };
        let inside_model = TagEntry {
            key: "file:tags/objects/vehicles/creep/shadow.model".to_owned(),
            display_path: "objects/vehicles/creep/shadow.model".to_owned(),
            group_tag: model,
            group_name: Some("model".to_owned()),
            location: TagEntryLocation::LooseFile(PathBuf::from(
                "tags/objects/vehicles/creep/shadow.model",
            )),
        };
        let reference = |group_tag, rel_path: &str| DependencyRef {
            group_tag,
            rel_path: rel_path.to_owned(),
        };
        let mut index = ReverseDependencyIndex::default();
        // The vehicle references its own model: inside -> inside.
        index.set_tag_dependencies(
            inside_vehicle.key.clone(),
            vec![reference(model, "objects\\vehicles\\creep\\shadow")],
        );
        // A scenario outside references the vehicle.
        index.set_tag_dependencies(
            "file:tags/levels/a10/a10.scenario".to_owned(),
            vec![reference(vehicle, "objects\\vehicles\\creep\\shadow")],
        );
        // Something unrelated.
        index.set_tag_dependencies(
            "file:tags/levels/a30/a30.scenario".to_owned(),
            vec![reference(vehicle, "objects\\vehicles\\ghost\\ghost")],
        );

        let keys = outside_referrer_keys(&[inside_vehicle, inside_model], &index, &names);

        assert_eq!(
            keys.into_iter().collect::<Vec<_>>(),
            vec!["file:tags/levels/a10/a10.scenario".to_owned()]
        );
    }

    /// Every tag reference under `root`, as `(referencing tag, referenced path)`.
    fn all_references(root: &Path, game: &str) -> Vec<(String, String)> {
        let source = TagSource::LooseFolder {
            root: root.to_path_buf(),
            game: GameId::from_id(game),
            definitions_root: locate_definitions_root(),
        };
        let names = TagNameIndex::load_game(&locate_definitions_root(), GameId::from_id(game).unwrap()).unwrap();
        let mut out = Vec::new();
        for entry in scan_folder_subtree_entries(root, Path::new(""), &names).unwrap() {
            let tag = read_entry(&source, &entry)
                .unwrap_or_else(|error| panic!("{} reads: {error}", entry.display_path));
            let mut refs = Vec::new();
            collect_tag_references(tag.root(), "", &mut refs);
            out.extend(
                refs.into_iter()
                    .map(|reference| (entry.display_path.clone(), reference.rel_path)),
            );
        }
        out
    }

    fn copy_tree(from: &Path, to: &Path) {
        for item in walkdir::WalkDir::new(from) {
            let item = item.unwrap();
            let target = to.join(item.path().strip_prefix(from).unwrap());
            if item.file_type().is_dir() {
                fs::create_dir_all(&target).unwrap();
            } else {
                fs::copy(item.path(), &target).unwrap();
            }
        }
    }

    /// Rename `folder` (inside `vehicle`, copied out of a real kit) and check
    /// that the tags outside it that referenced it now reference the new path,
    /// and that nothing references the old one.
    fn renames_and_rewrites(var: &str, game: &str, vehicle: &str, folder: &str) {
        let Some(kit) = std::env::var_os(var).map(PathBuf::from) else {
            eprintln!("skipping: set {var} to a {game} kit's tags folder");
            return;
        };
        let root = crate::test_kits::unique_temp_dir("folder-rename");
        copy_tree(&kit.join(vehicle), &root.join(vehicle));
        let old_prefix = format!("{}\\{folder}\\", vehicle.replace('/', "\\"));
        let new_prefix = format!("{}\\{folder}_renamed\\", vehicle.replace('/', "\\"));
        let referenced_before = all_references(&root, game)
            .into_iter()
            .filter(|(_, target)| target.to_ascii_lowercase().starts_with(&old_prefix))
            .count();
        assert!(referenced_before > 0, "the copy references {old_prefix}");

        let names = TagNameIndex::load_game(&locate_definitions_root(), GameId::from_id(game).unwrap()).unwrap();
        let (tx, _rx) = mpsc::channel();
        let rel = PathBuf::from(vehicle).join(folder);
        let done = run_folder_refactor_job(
            root.clone(),
            rel.clone(),
            root.join(vehicle),
            Some(format!("{folder}_renamed")),
            true,
            "Renaming".to_owned(),
            names,
            GameId::from_id(game),
            Vec::new(),
            None,
            &tx,
        )
        .unwrap();

        assert!(!root.join(&rel).exists());
        assert!(
            root.join(vehicle)
                .join(format!("{folder}_renamed"))
                .is_dir()
        );
        assert!(done.status.starts_with("Renamed"), "{}", done.status);
        assert!(!done.status.contains("NOT"), "{}", done.status);
        assert!(!done.old_to_new_keys.is_empty());
        let after = all_references(&root, game);
        let stale = after
            .iter()
            .filter(|(_, target)| target.to_ascii_lowercase().starts_with(&old_prefix))
            .collect::<Vec<_>>();
        assert!(
            stale.is_empty(),
            "still pointing at the old folder: {stale:?}"
        );
        let renamed = after
            .iter()
            .filter(|(_, target)| target.to_ascii_lowercase().starts_with(&new_prefix))
            .count();
        assert_eq!(renamed, referenced_before);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn renaming_a_halo3_folder_rewrites_references_into_it() {
        renames_and_rewrites(
            "BLAM_TEST_H3EK",
            "halo3_mcc",
            "objects/vehicles/ghost",
            "shaders",
        );
    }

    #[test]
    fn renaming_a_halo_ce_folder_rewrites_references_into_it() {
        renames_and_rewrites("BLAM_TEST_HCEEK", "haloce_mcc", "vehicles/ghost", "shaders");
    }
}
