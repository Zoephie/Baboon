//! Folder authoring for container sources: create, rename and retire folders a
//! Campaign Evolved pak does not yet hold a tag in.
//! It owns the pending-folder workflow and its naming contract; the tree seeding
//! itself belongs to `crate::source`, and drawing belongs to the browser UI.

use super::*;
use crate::app::controller::duplicate::validate_leaf_characters;

/// Normalise a folder path to the form the pending set and the tree agree on:
/// forward slashes, no empty segments, no leading or trailing separator.
///
/// Casing is preserved. The browser's `display_path` is already lowercased for
/// container sources, but a loose source is not, and a set that disagreed with
/// the tree about case would seed a second sibling node beside the real folder.
pub(in crate::app) fn normalize_folder_rel(input: &str) -> String {
    input
        .split(['/', '\\'])
        .filter(|segment| !segment.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("/")
}

fn folder_key(rel: &str) -> String {
    rel.to_ascii_lowercase()
}

/// Split a `display_path` into its segments, dropping empties.
fn segments(path: &str) -> Vec<&str> {
    path.split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// Everything already occupying a name directly inside `parent_rel`: child
/// folder labels and the leaf file names of tags sitting in it.
///
/// Both matter. A folder colliding with a subfolder is an obvious duplicate; a
/// folder colliding with a *tag* leaf is worse, because a pak's directory index
/// refuses a name that is both a file and a directory, so the collision would
/// not surface until a write.
fn folder_siblings(
    entries: &[TagEntry],
    pending: &std::collections::BTreeSet<String>,
    parent_rel: Option<&str>,
) -> Vec<String> {
    let parent = parent_rel.map(segments).unwrap_or_default();
    let mut names = Vec::new();

    let mut push = |name: &str| {
        let name = name.to_owned();
        if !names.iter().any(|existing: &String| existing.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    };

    for entry in entries {
        let parts = segments(&entry.display_path);
        if parts.len() <= parent.len() {
            continue;
        }
        let matches_parent = parent
            .iter()
            .zip(&parts)
            .all(|(want, have)| want.eq_ignore_ascii_case(have));
        if matches_parent {
            push(parts[parent.len()]);
        }
    }

    for folder in pending {
        let parts = segments(folder);
        if parts.len() <= parent.len() {
            continue;
        }
        let matches_parent = parent
            .iter()
            .zip(&parts)
            .all(|(want, have)| want.eq_ignore_ascii_case(have));
        if matches_parent {
            push(parts[parent.len()]);
        }
    }

    names
}

/// Validate one folder leaf against the names already in its parent.
///
/// Shares [`validate_leaf_characters`] with the tag duplicate path on purpose: a
/// folder a duplicate would have refused as a tag name is a folder no tag could
/// ever be created inside.
pub(in crate::app) fn validate_folder_leaf_name(
    raw: &str,
    siblings: &[String],
) -> Result<String, String> {
    let name = validate_leaf_characters(raw, "Folder names", "Enter a folder name")?;
    if siblings
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&name))
    {
        return Err("That folder already exists here".to_owned());
    }
    Ok(name)
}

/// Join a validated leaf onto its parent, in the casing the tree seeds against.
///
/// A container mount lowercases every `display_path` (see `build_container_set`)
/// while the seed is matched against those paths by exact string. A folder kept
/// as `Vehicles` would therefore sit *beside* the `vehicles` node that appears
/// the moment a tag lands in it — two nodes for one folder, with the seed
/// re-creating the empty one on every rebuild. The fold happens here, where the
/// source is known to be a container, rather than in the tree builder, which
/// also serves case-preserving loose sources.
fn container_folder_rel(parent_rel: Option<&str>, leaf: &str) -> String {
    let leaf = leaf.to_ascii_lowercase();
    match parent_rel {
        Some(parent) if !parent.is_empty() => format!("{parent}/{leaf}"),
        _ => leaf,
    }
}

impl Baboon {
    /// Names occupying `parent_rel` in the given workspace.
    fn folder_siblings_in(&self, kit_index: usize, parent_rel: Option<&str>) -> Vec<String> {
        let kit = &self.kits[kit_index];
        let entries = kit
            .source
            .as_ref()
            .map(|source| source.entries.as_slice())
            .unwrap_or_default();
        folder_siblings(entries, &kit.pending_container_folders, parent_rel)
    }

    /// Raise the New Folder dialog for `parent_rel` (`None` = container root).
    pub(super) fn open_new_container_folder(&mut self, parent_rel: Option<String>) {
        let kit = self.kits[self.active].id;
        self.container_folder_dialog = Some(ContainerFolderDialog {
            kit,
            parent_rel: parent_rel.map(|rel| normalize_folder_rel(&rel)),
            renaming: None,
            name_input: String::new(),
            focus_input: true,
            error: None,
        });
    }

    /// Raise the Rename Folder dialog for a pending folder.
    pub(super) fn open_rename_container_folder(&mut self, rel: String) {
        let rel = normalize_folder_rel(&rel);
        let (parent_rel, leaf) = match rel.rsplit_once('/') {
            Some((parent, leaf)) => (Some(parent.to_owned()), leaf.to_owned()),
            None => (None, rel.clone()),
        };
        let kit = self.kits[self.active].id;
        self.container_folder_dialog = Some(ContainerFolderDialog {
            kit,
            parent_rel,
            renaming: Some(rel),
            name_input: leaf,
            focus_input: true,
            error: None,
        });
    }

    /// Apply the pending New/Rename Folder dialog. Returns `true` when the
    /// dialog should close.
    pub(in crate::app) fn apply_container_folder_dialog(&mut self) -> bool {
        let Some(dialog) = self.container_folder_dialog.as_ref() else {
            return true;
        };
        let Some(kit_index) = self.kit_index(dialog.kit) else {
            // The workspace closed while the modeless dialog was open.
            return true;
        };
        let parent_rel = dialog.parent_rel.clone();
        let renaming = dialog.renaming.clone();
        let raw = dialog.name_input.clone();

        // A rename keeps its own leaf available, or renaming a folder to the
        // case-variant of its current name would collide with itself.
        let mut siblings = self.folder_siblings_in(kit_index, parent_rel.as_deref());
        if let Some(old) = renaming.as_deref() {
            let old_leaf = old.rsplit('/').next().unwrap_or(old);
            siblings.retain(|name| !name.eq_ignore_ascii_case(old_leaf));
        }

        let leaf = match validate_folder_leaf_name(&raw, &siblings) {
            Ok(leaf) => leaf,
            Err(error) => {
                if let Some(dialog) = self.container_folder_dialog.as_mut() {
                    dialog.error = Some(error);
                }
                return false;
            }
        };
        let rel = container_folder_rel(parent_rel.as_deref(), &leaf);

        match renaming {
            Some(old) => self.rename_pending_container_folder(kit_index, &old, &rel),
            None => {
                self.kits[kit_index]
                    .pending_container_folders
                    .insert(rel.clone());
                self.status = format!("Created folder {rel}");
            }
        }
        self.refresh_container_folder_tree(kit_index);
        true
    }

    /// Move a pending folder and everything pending beneath it.
    ///
    /// Descendants have to travel with it: a folder set holding `a/b` but not
    /// `a` after `a` was renamed would seed an orphan at the old path on the
    /// next rebuild.
    fn rename_pending_container_folder(&mut self, kit_index: usize, old: &str, new: &str) {
        let old_key = folder_key(old);
        let prefix = format!("{old_key}/");
        let folders = &mut self.kits[kit_index].pending_container_folders;
        let moved: Vec<String> = folders
            .iter()
            .filter(|folder| {
                let key = folder_key(folder);
                key == old_key || key.starts_with(&prefix)
            })
            .cloned()
            .collect();
        for folder in &moved {
            folders.remove(folder);
        }
        for folder in &moved {
            let suffix = &folder[old.len().min(folder.len())..];
            folders.insert(format!("{new}{suffix}"));
        }
        self.status = format!("Renamed folder {old} to {new}");
    }

    /// Retire a pending folder. Only ever called for one the browser drew as
    /// empty, so nothing beneath it can be stranded.
    pub(super) fn delete_container_folder(&mut self, rel: String) {
        let rel = normalize_folder_rel(&rel);
        let kit_index = self.active;
        let key = folder_key(&rel);
        let folders = &mut self.kits[kit_index].pending_container_folders;
        let removed: Vec<String> = folders
            .iter()
            .filter(|folder| folder_key(folder) == key)
            .cloned()
            .collect();
        for folder in removed {
            folders.remove(&folder);
        }
        self.status = format!("Removed folder {rel}");
        self.refresh_container_folder_tree(kit_index);
    }

    /// Take the folders a restored project brought back.
    ///
    /// Merged rather than assigned: a workspace can adopt its recovery file
    /// after the user has already made a folder this session (the recovery file
    /// is picked up when the source finishes mounting), and replacing the set
    /// would throw that away. A `BTreeSet` makes a repeat harmless.
    pub(in crate::app) fn adopt_project_container_folders(
        &mut self,
        kit_index: usize,
        folders: impl IntoIterator<Item = String>,
    ) {
        let normalized = folders
            .into_iter()
            .map(|folder| normalize_folder_rel(&folder))
            .filter(|folder| !folder.is_empty());
        let before = self.kits[kit_index].pending_container_folders.len();
        self.kits[kit_index]
            .pending_container_folders
            .extend(normalized);
        if self.kits[kit_index].pending_container_folders.len() != before {
            self.refresh_container_folder_tree(kit_index);
        }
    }

    /// Re-seed this workspace's folder tree after the pending set changed.
    fn refresh_container_folder_tree(&mut self, kit_index: usize) {
        let seeds = self.kits[kit_index].folder_seeds();
        if let Some(source) = self.kits[kit_index].source.as_mut() {
            crate::source::rebuild_folder_tree(source, &seeds);
        }
        // The browser memoises its filtered tree on the generation, so without
        // this a folder created while a filter is active would not appear.
        self.kits[kit_index].generation = self.kits[kit_index].generation.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(display_path: &str) -> TagEntry {
        TagEntry {
            key: display_path.into(),
            display_path: display_path.into(),
            group_tag: u32::from_be_bytes(*b"hlmt"),
            group_name: None,
            location: TagEntryLocation::LooseFile(std::path::PathBuf::from(display_path)),
        }
    }

    fn folders(paths: &[&str]) -> std::collections::BTreeSet<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn normalizes_separators_and_strays() {
        assert_eq!(normalize_folder_rel("/objects\\vehicles/"), "objects/vehicles");
        assert_eq!(normalize_folder_rel("objects//vehicles"), "objects/vehicles");
        assert_eq!(normalize_folder_rel("  "), "");
    }

    #[test]
    fn siblings_include_subfolders_and_tag_leaves() {
        let entries = vec![
            entry("objects/vehicles/warthog.model"),
            entry("objects/characters/masterchief.biped"),
            entry("sound/ambient.sound"),
        ];
        let pending = folders(&["objects/pending"]);

        let mut names = folder_siblings(&entries, &pending, Some("objects"));
        names.sort();
        // `warthog.model` is a leaf of `objects/vehicles`, not of `objects`.
        assert_eq!(names, vec!["characters", "pending", "vehicles"]);

        let mut root = folder_siblings(&entries, &pending, None);
        root.sort();
        assert_eq!(root, vec!["objects", "sound"]);
    }

    /// A pak's directory index cannot hold a name that is both a file and a
    /// directory, so this has to be refused here rather than at write time.
    #[test]
    fn a_folder_cannot_collide_with_a_tag_leaf_in_the_same_parent() {
        let entries = vec![entry("objects/warthog.model")];
        let siblings = folder_siblings(&entries, &folders(&[]), Some("objects"));
        assert!(validate_folder_leaf_name("warthog.model", &siblings).is_err());
        assert!(validate_folder_leaf_name("vehicles", &siblings).is_ok());
    }

    /// A container's `display_path` is lowercased at mount, so a seed kept in
    /// the user's casing would draw a second node beside the real folder as
    /// soon as a tag landed in it — and keep re-creating the empty one.
    #[test]
    fn a_container_folder_is_seeded_in_display_path_casing() {
        assert_eq!(
            container_folder_rel(Some("objects"), "Vehicles"),
            "objects/vehicles"
        );
        assert_eq!(container_folder_rel(None, "Objects"), "objects");
        assert_eq!(container_folder_rel(Some(""), "Objects"), "objects");

        // The seeded path must reach the same node an entry would build.
        let entries = vec![entry("objects/vehicles/warthog.model")];
        let seeded = container_folder_rel(Some("objects"), "Vehicles");
        let tree = crate::source::build_tree_with_folders(&entries, &[seeded]);
        let objects = tree
            .children
            .iter()
            .find(|child| child.label == "objects")
            .expect("objects node");
        assert_eq!(
            objects.children.len(),
            1,
            "seeding must not add a second `vehicles` beside the real one"
        );
    }

    #[test]
    fn folder_collisions_are_case_insensitive() {
        let siblings = vec!["Vehicles".to_owned()];
        assert!(validate_folder_leaf_name("vehicles", &siblings).is_err());
        assert!(validate_folder_leaf_name("VEHICLES", &siblings).is_err());
    }

    /// The naming contract is shared with the tag duplicate path, so a folder
    /// nothing could be created inside is refused up front.
    #[test]
    fn folder_names_inherit_the_shared_leaf_rules() {
        for invalid in ["", "  ", ".", "..", "a/b", "a.b", "trailing ", "CON", "we:ird"] {
            assert!(
                validate_folder_leaf_name(invalid, &[]).is_err(),
                "{invalid:?} should be refused"
            );
        }
        assert_eq!(validate_folder_leaf_name(" vehicles", &[]).unwrap(), "vehicles");
    }
}
