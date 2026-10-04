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
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        if self.tag_ops.folder_refactor.is_some() {
            self.status = "A folder move/rename is already running".to_owned();
            return;
        }
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Rename requires a loaded tags folder".to_owned();
            return;
        };
        let rel_path = match validate_relative_folder_path(&rel_path) {
            Ok(rel_path) => rel_path,
            Err(_) => {
                self.status = "The tags root itself cannot be renamed".to_owned();
                return;
            }
        };
        let Some(old_name) = rel_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            self.status = format!("Cannot rename {label}");
            return;
        };
        // From the workspace's tag list when it holds every tag: walking a
        // large folder on disk here stalls the window (Halo 3's `objects`,
        // 15,555 tags, took 0.3 s on a warm cache). Otherwise from disk, the
        // way the job itself finds them.
        let names = self.names().clone();
        let folder = root.join(&rel_path);
        let loaded = self
            .source()
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
                    self.status = format!("Could not scan {label}: {error}");
                    return;
                }
            },
        };
        let outside_referrers = self.source().and_then(|source| {
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
        self.tag_ops.loose_folder_rename = Some(LooseFolderRenameState {
            kit: self.active_kit_id(),
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
        let Some(state) = self.tag_ops.loose_folder_rename.as_ref() else {
            return true;
        };
        let kit = state.kit;
        let rel_path = state.rel_path.clone();
        let old_name = state.old_name.clone();
        let raw = state.name_input.clone();
        if !self.focus_navigation_kit(kit) {
            self.status = "The workspace this rename came from is closed".to_owned();
            return true;
        }
        if self.refuse_read_only_edit(self.active) {
            return true;
        }
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Rename requires a loaded tags folder".to_owned();
            return true;
        };
        let reject = |this: &mut Self, error: String| {
            if let Some(state) = this.tag_ops.loose_folder_rename.as_mut() {
                state.error = Some(error);
            }
            false
        };
        if self.tag_ops.folder_refactor.is_some() {
            return reject(self, "A folder move/rename is already running".to_owned());
        }
        if self.kits[self.active]
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
mod tests;
