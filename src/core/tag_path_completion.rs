//! What a tag path being typed could go on to name: the folders and tags in
//! the folder it has reached, ranked against the name it has started.
//!
//! A loose tags folder is read one directory at a time, so suggestions need
//! no finished scan and are as current as the disk. A container source holds
//! every tag in its catalog already, and is filtered by folder.
//!
//! Nothing here completes a path by itself: a suggestion is only what the
//! user may pick.

use std::collections::BTreeMap;
use std::path::Path;

use crate::core::format::TagNameIndex;
use crate::core::source::{TagEntry, natural_key};
use crate::core::tag_path;

/// Where the folders and tags come from.
#[derive(Clone, Copy)]
pub(crate) enum Catalog<'a> {
    /// A loose tags folder. A file is a tag when its extension names one of
    /// the game's groups; without the game's names, one of any game's.
    Loose {
        root: &'a Path,
        names: Option<&'a TagNameIndex>,
    },
    /// Every tag a container source holds.
    Entries(&'a [TagEntry]),
}

/// The folders and tags directly inside one folder, each in the order tag
/// paths sort in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FolderListing {
    pub(crate) folders: Vec<String>,
    pub(crate) tags: Vec<ListedTag>,
}

/// A tag in a [`FolderListing`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListedTag {
    /// Its name without the extension.
    pub(crate) name: String,
    pub(crate) extension: String,
    pub(crate) group_tag: u32,
}

/// The folders and tags directly inside `folder` (a tag path; empty for the
/// top of the tags folder). A folder that can't be read lists nothing.
pub(crate) fn list_folder(catalog: Catalog<'_>, folder: &str) -> FolderListing {
    let mut listing = match catalog {
        Catalog::Loose { root, names } => list_loose_folder(root, names, folder),
        Catalog::Entries(entries) => list_entries_folder(entries, folder),
    };
    listing.folders.sort_by_cached_key(|name| natural_key(name));
    listing
        .tags
        .sort_by_cached_key(|tag| natural_key(&format!("{}.{}", tag.name, tag.extension)));
    listing
}

fn list_loose_folder(root: &Path, names: Option<&TagNameIndex>, folder: &str) -> FolderListing {
    let mut listing = FolderListing::default();
    let Ok(read) = std::fs::read_dir(root.join(tag_path::to_relative(folder))) else {
        return listing;
    };
    for entry in read.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // A link is followed, as the browser follows it.
        let is_dir = kind.is_dir() || (kind.is_symlink() && entry.path().is_dir());
        if is_dir {
            listing.folders.push(file_name.to_owned());
            continue;
        }
        let Some((name, extension)) = tag_path::split_extension(file_name) else {
            continue;
        };
        let group_tag = match names {
            Some(names) => names.group_tag_for(extension),
            None => crate::core::document::value::extension_to_group_tag(extension),
        };
        if let Some(group_tag) = group_tag {
            listing.tags.push(ListedTag {
                name: name.to_owned(),
                extension: extension.to_owned(),
                group_tag,
            });
        }
    }
    listing
}

fn list_entries_folder(entries: &[TagEntry], folder: &str) -> FolderListing {
    let mut listing = FolderListing::default();
    // Keyed without case, keeping the first spelling met.
    let mut folders: BTreeMap<String, String> = BTreeMap::new();
    for entry in entries {
        let path = entry.display_path.as_str();
        let Some(rest) = strip_folder(path, folder) else {
            continue;
        };
        match rest.split_once(tag_path::SEPARATOR) {
            Some((child, _)) if !child.is_empty() => {
                folders
                    .entry(child.to_ascii_lowercase())
                    .or_insert_with(|| child.to_owned());
            }
            Some(_) => {}
            None => {
                let (name, extension) = match tag_path::split_extension(rest) {
                    Some((name, extension)) => (name, extension.to_owned()),
                    None => (rest, entry.group_name.clone().unwrap_or_default()),
                };
                listing.tags.push(ListedTag {
                    name: name.to_owned(),
                    extension,
                    group_tag: entry.group_tag,
                });
            }
        }
    }
    listing.folders = folders.into_values().collect();
    listing
}

/// What follows `folder` and its separator in `path`, ignoring case.
fn strip_folder<'p>(path: &'p str, folder: &str) -> Option<&'p str> {
    if folder.is_empty() {
        return Some(path);
    }
    let head = path.get(..folder.len())?;
    let rest = &path[folder.len()..];
    (head.eq_ignore_ascii_case(folder) && rest.starts_with(tag_path::SEPARATOR))
        .then(|| &rest[1..])
}

/// What an input takes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Wanted<'a> {
    /// Folders only.
    Folders,
    /// Folders, and tags of these groups; `None` for tags of any group.
    Tags(Option<&'a [u32]>),
}

/// What a suggestion names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SuggestionKind {
    Folder,
    Tag { group_tag: u32 },
}

/// One row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Suggestion {
    pub(crate) kind: SuggestionKind,
    /// What the row shows: a folder's name, or a tag's name and extension.
    pub(crate) label: String,
    /// The tag path it names, without an extension.
    pub(crate) path: String,
    /// A tag's extension.
    pub(crate) extension: Option<String>,
}

impl Suggestion {
    /// What picking it puts in the box: a folder's path and a separator, to go
    /// on typing in it, or a tag's path and extension.
    pub(crate) fn text(&self) -> String {
        match &self.extension {
            Some(extension) => format!("{}.{extension}", self.path),
            None => format!("{}{}", self.path, tag_path::SEPARATOR),
        }
    }
}

/// The folder a typed path has reached and the name started in it, or `None`
/// for text that isn't a path being typed: `none`, or one naming its group
/// (`weap:…`). Typed `/` is a separator, and a leading separator is dropped.
pub(crate) fn split_typed(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("none") || trimmed.contains(':') {
        return None;
    }
    let typed = tag_path::from_input(trimmed);
    let typed = typed.trim_start_matches(tag_path::SEPARATOR);
    Some(match typed.rsplit_once(tag_path::SEPARATOR) {
        Some((folder, partial)) => (folder.to_owned(), partial.to_owned()),
        None => (String::new(), typed.to_owned()),
    })
}

/// The rows for `partial` typed in `folder`, from that folder's `listing`.
///
/// A name equal to `partial` comes first, so a pasted path lists every tag
/// of that name; then names starting with it, then names holding it, all
/// ignoring case. Within each, folders come before tags.
pub(crate) fn suggestions(
    listing: &FolderListing,
    folder: &str,
    partial: &str,
    wanted: Wanted<'_>,
) -> Vec<Suggestion> {
    let partial = partial.to_ascii_lowercase();
    let rank = |name: &str, label: &str| -> Option<u8> {
        let label = label.to_ascii_lowercase();
        if name.eq_ignore_ascii_case(&partial) || label == partial {
            Some(0)
        } else if label.starts_with(&partial) {
            Some(1)
        } else if label.contains(&partial) {
            Some(2)
        } else {
            None
        }
    };
    let mut ranked: Vec<(u8, Suggestion)> = Vec::new();
    for name in &listing.folders {
        if let Some(rank) = rank(name, name) {
            ranked.push((
                rank,
                Suggestion {
                    kind: SuggestionKind::Folder,
                    label: name.clone(),
                    path: tag_path::join(folder, name),
                    extension: None,
                },
            ));
        }
    }
    let groups = match wanted {
        Wanted::Folders => return ranked_rows(ranked),
        Wanted::Tags(groups) => groups,
    };
    for tag in &listing.tags {
        if groups.is_some_and(|groups| !groups.contains(&tag.group_tag)) {
            continue;
        }
        let label = format!("{}.{}", tag.name, tag.extension);
        if let Some(rank) = rank(&tag.name, &label) {
            ranked.push((
                rank,
                Suggestion {
                    kind: SuggestionKind::Tag {
                        group_tag: tag.group_tag,
                    },
                    label,
                    path: tag_path::join(folder, &tag.name),
                    extension: Some(tag.extension.clone()),
                },
            ));
        }
    }
    ranked_rows(ranked)
}

/// The rows best first. Stable: the listing's order holds within a rank and
/// kind.
fn ranked_rows(mut ranked: Vec<(u8, Suggestion)>) -> Vec<Suggestion> {
    ranked.sort_by_key(|(rank, suggestion)| (*rank, suggestion.kind != SuggestionKind::Folder));
    ranked.into_iter().map(|(_, suggestion)| suggestion).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::source::TagEntryLocation;

    const SKY: u32 = u32::from_be_bytes(*b"sky ");
    const BITMAP: u32 = u32::from_be_bytes(*b"bitm");
    const BIPED: u32 = u32::from_be_bytes(*b"bipd");
    const SCENERY: u32 = u32::from_be_bytes(*b"scen");

    fn entry(display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: display_path.to_owned(),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: None,
            location: TagEntryLocation::LooseFile(display_path.into()),
        }
    }

    fn labels(rows: &[Suggestion]) -> Vec<&str> {
        rows.iter().map(|row| row.label.as_str()).collect()
    }

    fn snow_folder() -> Vec<TagEntry> {
        vec![
            entry(r"altis\sky\snow\snow.sky", SKY),
            entry(r"altis\sky\snow\snow.bitmap", BITMAP),
            entry(r"altis\sky\snow\snowflake.bitmap", BITMAP),
            entry(r"altis\sky\snow\cloud_snow.sky", SKY),
            entry(r"altis\sky\snow\fx\flurry.bitmap", BITMAP),
            entry(r"altis\sky\SNOW\Snowdrift\drift.sky", SKY),
            entry(r"altis\sky\snowy.sky", SKY),
        ]
    }

    #[test]
    fn a_folder_lists_its_own_tags_and_subfolders_only() {
        let entries = snow_folder();
        let listing = list_folder(Catalog::Entries(&entries), r"altis\sky\snow");
        assert_eq!(listing.folders, ["fx", "Snowdrift"]);
        let tags: Vec<_> = listing
            .tags
            .iter()
            .map(|tag| format!("{}.{}", tag.name, tag.extension))
            .collect();
        assert_eq!(tags, ["cloud_snow.sky", "snow.bitmap", "snow.sky", "snowflake.bitmap"]);
        // Not `snowy.sky`, one folder up, nor `drift.sky`, one down.
        let top = list_folder(Catalog::Entries(&entries), "");
        assert_eq!(top.folders, ["altis"]);
        assert!(top.tags.is_empty());
    }

    #[test]
    fn a_pasted_path_lists_every_tag_of_that_name_first() {
        let entries = snow_folder();
        let listing = list_folder(Catalog::Entries(&entries), r"altis\sky\snow");
        let rows = suggestions(&listing, r"altis\sky\snow", "snow", Wanted::Tags(None));
        assert_eq!(
            labels(&rows),
            ["snow.bitmap", "snow.sky", "Snowdrift", "snowflake.bitmap", "cloud_snow.sky"]
        );
        assert_eq!(rows[1].text(), r"altis\sky\snow\snow.sky");
        assert_eq!(rows[1].kind, SuggestionKind::Tag { group_tag: SKY });
        assert_eq!(rows[2].text(), r"altis\sky\snow\Snowdrift\");
    }

    #[test]
    fn only_the_groups_an_input_takes_are_offered() {
        let entries = snow_folder();
        let listing = list_folder(Catalog::Entries(&entries), r"altis\sky\snow");
        let sky = suggestions(&listing, r"altis\sky\snow", "snow", Wanted::Tags(Some(&[SKY])));
        assert_eq!(labels(&sky), ["snow.sky", "Snowdrift", "cloud_snow.sky"]);
        let folders = suggestions(&listing, r"altis\sky\snow", "", Wanted::Folders);
        assert_eq!(labels(&folders), ["fx", "Snowdrift"]);

        // A field taking a parent group is handed its descendants: both tags
        // sharing a name are listed.
        let shared = vec![
            entry(r"objects\crate.biped", BIPED),
            entry(r"objects\crate.scenery", SCENERY),
            entry(r"objects\crate.bitmap", BITMAP),
        ];
        let listing = list_folder(Catalog::Entries(&shared), "objects");
        let object = suggestions(&listing, "objects", "crate", Wanted::Tags(Some(&[BIPED, SCENERY])));
        assert_eq!(labels(&object), ["crate.biped", "crate.scenery"]);
    }

    #[test]
    fn a_typed_extension_narrows_to_it() {
        let entries = snow_folder();
        let listing = list_folder(Catalog::Entries(&entries), r"altis\sky\snow");
        let rows = suggestions(&listing, r"altis\sky\snow", "snow.s", Wanted::Tags(None));
        assert_eq!(labels(&rows), ["snow.sky", "cloud_snow.sky"]);
        assert!(suggestions(&listing, r"altis\sky\snow", "rain", Wanted::Tags(None)).is_empty());
    }

    #[test]
    fn typed_text_splits_into_folder_and_name() {
        assert_eq!(
            split_typed(r"altis\sky\snow\sn"),
            Some((r"altis\sky\snow".to_owned(), "sn".to_owned()))
        );
        assert_eq!(
            split_typed("altis/sky/"),
            Some((r"altis\sky".to_owned(), String::new()))
        );
        assert_eq!(split_typed(r"\altis"), Some((String::new(), "altis".to_owned())));
        assert_eq!(split_typed(""), Some((String::new(), String::new())));
        assert_eq!(split_typed("NONE"), None);
        assert_eq!(split_typed(r"skyy:altis\sky"), None);
    }

    #[test]
    fn a_loose_folder_is_read_from_disk() {
        let root = std::env::temp_dir().join(format!("baboon-completion-{}", std::process::id()));
        let folder = root.join("altis").join("sky");
        std::fs::create_dir_all(folder.join("snow")).unwrap();
        std::fs::write(folder.join("clear.sky"), b"").unwrap();
        std::fs::write(folder.join("clear.bitmap"), b"").unwrap();
        std::fs::write(folder.join("notes.txt"), b"").unwrap();
        std::fs::write(folder.join(".DS_Store"), b"").unwrap();
        let names = TagNameIndex::load_game(
            &crate::core::bundled::locate_definitions_root(),
            crate::core::game::GameId::Halo2,
        )
        .expect("Halo 2's bundled names load");
        let catalog = Catalog::Loose { root: &root, names: Some(&names) };
        let listing = list_folder(catalog, r"altis\sky");
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(listing.folders, ["snow"]);
        assert_eq!(
            listing.tags,
            [
                ListedTag { name: "clear".into(), extension: "bitmap".into(), group_tag: BITMAP },
                ListedTag { name: "clear".into(), extension: "sky".into(), group_tag: SKY },
            ]
        );
        assert_eq!(
            list_folder(catalog, r"altis\sky"),
            FolderListing::default(),
            "a folder that is gone lists nothing"
        );
    }
}
