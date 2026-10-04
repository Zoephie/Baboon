//! Documentation overlay parsed from the JSON tag definitions. Shipped tags
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.
//! embed a *stripped* layout (clean field names, no explanation fields — see the
//! `blam-tags` schema builder), so the help/units text and explanation blocks
//! live only in the definitions. We parse them once per group, keyed by struct
//! GUID, and overlay them onto the editor at render time without touching tags.

use super::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// One entry in a struct's documentation sequence, in schema order.
pub(super) enum DefEntry {
    /// A real field. `clean_name` matches the engine's stripped field name (so
    /// it lines up with the tag's fields); `help`/`unit` come from the full
    /// schema name's `#…` / `:…` suffixes.
    Field {
        clean_name: String,
        help: Option<String>,
        unit: Option<String>,
        range: Option<String>,
        tag_reference_allowed: Vec<u32>,
    },
    /// An explanation block (stripped from shipped tags). `title` is the schema
    /// name (often a section header), `body` the `definition` text.
    Explanation { title: String, body: String },
}

/// Which struct a documentation sequence belongs to: its GUID (stable across
/// the name stripping and matching shipped tags exactly), or, for definitions
/// whose structs carry no GUID (every halo2_mcc struct is all zeros), its name.
/// Keying those by the shared zero GUID handed one struct's explanations to
/// every struct in the tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum StructKey {
    Guid([u8; 16]),
    Name(String),
}

impl StructKey {
    fn new(guid: [u8; 16], name: &str) -> Self {
        if guid == [0; 16] {
            Self::Name(name.to_owned())
        } else {
            Self::Guid(guid)
        }
    }
}

/// Per-group documentation, keyed by struct ([`StructKey`]).
#[derive(Default)]
pub(super) struct DefDocs {
    by_struct: HashMap<StructKey, Vec<DefEntry>>,
}

impl DefDocs {
    /// The documentation sequence for the struct with `guid` and `name`.
    pub(super) fn entries_for(&self, guid: [u8; 16], name: &str) -> &[DefEntry] {
        self.by_struct
            .get(&StructKey::new(guid, name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Every entry of every struct, in no particular order.
    #[cfg(test)]
    pub(in crate::app) fn all_entries(&self) -> impl Iterator<Item = &DefEntry> {
        self.by_struct.values().flatten()
    }

    pub(super) fn entries_for_struct(&self, tag_struct: &TagStruct<'_>) -> &[DefEntry] {
        let definition = tag_struct.definition();
        self.entries_for(definition.guid(), definition.name())
    }
}

/// Stable renderer/Find identity for an injected explanation row. Keep the
/// numeric suffix free of `#`/`[]`: those are stripped from canonical field
/// paths because they normally identify schema ordinals and block elements.
pub(super) fn documentation_path(path_prefix: &str, entry_index: usize) -> String {
    let segment = format!("@documentation {entry_index}");
    if path_prefix.is_empty() {
        segment
    } else {
        format!("{path_prefix}/{segment}")
    }
}

/// Build a group's documentation, following the `parent_tag` inheritance chain
/// and merging every file's structs by GUID. Object-family tags (biped → unit →
/// object) inherit fields whose struct definitions live in the parent files, so
/// the chain must be walked for those fields' docs to resolve.
pub(super) fn build_def_docs(definitions_root: &Path, game: &str, group: &str) -> DefDocs {
    let mut docs = DefDocs::default();
    let mut visited = HashSet::new();
    let mut current = Some(group.to_owned());
    while let Some(g) = current.take() {
        if !visited.insert(g.clone()) {
            break; // cycle guard
        }
        let path = definitions_root.join(game).join(format!("{g}.json"));
        let Ok(json) = std::fs::read_to_string(&path) else {
            break;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
            break;
        };
        merge_structs_into(&mut docs, &value);
        current = value
            .get("parent_tag")
            .and_then(|p| p.as_str())
            .and_then(resolve_parent_group);
    }
    docs
}

/// Map a `parent_tag` (a four-CC like `obje`, or a group name like `unit`) to
/// the definition file's group name.
fn resolve_parent_group(parent_tag: &str) -> Option<String> {
    let bytes = parent_tag.as_bytes();
    if bytes.len() == 4 {
        let fourcc = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if let Some(name) = group_tag_to_extension(fourcc) {
            return Some(name.to_owned());
        }
    }
    (!parent_tag.is_empty()).then(|| parent_tag.to_owned())
}

/// Parse a single group definition JSON into a `DefDocs` (no chain). Test-only;
/// production resolution uses [`build_def_docs`] to follow the inheritance chain.
#[cfg(test)]
pub(super) fn parse_def_docs(json: &str) -> DefDocs {
    let mut docs = DefDocs::default();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
        merge_structs_into(&mut docs, &value);
    }
    docs
}

/// Merge one definition file's structs into `docs`, keyed by GUID. Existing
/// GUIDs win (a child group's own structs take precedence over parents').
fn merge_structs_into(docs: &mut DefDocs, value: &serde_json::Value) {
    let Some(structs) = value.get("structs").and_then(|v| v.as_object()) else {
        return;
    };
    for (struct_name, st) in structs {
        let Some(guid) = st
            .get("guid")
            .and_then(|g| g.as_str())
            .and_then(parse_guid_hex)
        else {
            continue;
        };
        let Some(fields) = st.get("fields").and_then(|f| f.as_array()) else {
            continue;
        };
        let mut entries = Vec::new();
        for field in fields {
            let ty = field.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let name = field.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if ty == "explanation" {
                let body = field
                    .get("definition")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_owned();
                if !name.is_empty() || !body.trim().is_empty() {
                    entries.push(DefEntry::Explanation {
                        title: name.to_owned(),
                        body,
                    });
                }
            } else if !name.is_empty() {
                let meta = field_display_meta(name); // help/unit/range from full name
                let tag_reference_allowed = if ty == "tag_reference" {
                    parse_tag_reference_allowed_groups(field)
                } else {
                    Vec::new()
                };
                entries.push(DefEntry::Field {
                    clean_name: clean_for_match(name),
                    help: meta.help,
                    unit: meta.unit,
                    range: meta.range,
                    tag_reference_allowed,
                });
            }
        }
        docs.by_struct
            .entry(StructKey::new(guid, struct_name))
            .or_insert(entries);
    }
}

fn parse_tag_reference_allowed_groups(field: &serde_json::Value) -> Vec<u32> {
    field
        .get("definition")
        .and_then(|definition| definition.get("allowed"))
        .and_then(|allowed| allowed.as_array())
        .map(|allowed| {
            allowed
                .iter()
                .filter_map(|group| group.as_str())
                .filter_map(parse_group_tag)
                .collect()
        })
        .unwrap_or_default()
}

/// Reduce a schema field name to the engine's stripped form so it matches the
/// tag's field names. MUST stay in sync with `blam-tags` `clean_blay_field_name`:
/// cut at the first `:`/`#`, drop `{alias}` groups, strip trailing `*`/`!`.
fn clean_for_match(name: &str) -> String {
    let cut = name.find([':', '#']).unwrap_or(name.len());
    let mut s = name[..cut].to_string();
    while let (Some(open), Some(close)) = (s.find('{'), s.find('}')) {
        if open < close {
            s.replace_range(open..=close, "");
        } else {
            break;
        }
    }
    s.trim_end_matches(['*', '!', '^', ' ']).trim().to_string()
}

fn parse_guid_hex(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Which tag groups inherit from which, for one game: `biped` → `unit` →
/// `object`, every shader type → `render_method`. A tag reference whose
/// schema allows a parent group takes any group descended from it, the way
/// Foundation offers every object type for an `object` field.
#[derive(Debug, Default)]
pub(in crate::app) struct GroupHierarchy {
    parents: HashMap<u32, u32>,
}

impl GroupHierarchy {
    /// Read each group's `tag` and `parent_tag` from the game's definitions.
    /// Both sit at the top of a definition file, so only its head is read.
    fn load(definitions_root: &Path, game: &str) -> Self {
        use std::io::Read;
        let mut parents = HashMap::new();
        let mut by_name: HashMap<String, u32> = HashMap::new();
        let mut named_parents: Vec<(u32, String)> = Vec::new();
        let Ok(dir) = std::fs::read_dir(definitions_root.join(game)) else {
            return Self::default();
        };
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json")
                || path.file_name().is_some_and(|name| name == "_meta.json")
            {
                continue;
            }
            let mut head = Vec::with_capacity(1024);
            if std::fs::File::open(&path)
                .and_then(|file| file.take(1024).read_to_end(&mut head))
                .is_err()
            {
                continue;
            }
            let head = String::from_utf8_lossy(&head);
            let value = |key: &str| {
                let at = head.find(&format!("\"{key}\""))?;
                let rest = &head[at + key.len() + 2..];
                let open = rest.find('"')? + 1;
                let close = rest[open..].find('"')? + open;
                Some(rest[open..close].to_owned())
            };
            let Some(tag) = value("tag").as_deref().and_then(blam_tags::parse_group_tag) else {
                continue;
            };
            if let Some(name) = value("name") {
                by_name.insert(name, tag);
            }
            if let Some(parent) = value("parent_tag").filter(|parent| !parent.is_empty()) {
                named_parents.push((tag, parent));
            }
        }
        // A parent is a four-character group tag (`obje`, `rm  `), or now and
        // then a group name.
        for (tag, parent) in named_parents {
            let parent = by_name.get(&parent).copied().or_else(|| {
                (parent.len() == 4)
                    .then(|| blam_tags::parse_group_tag(&parent))
                    .flatten()
            });
            if let Some(parent) = parent {
                parents.insert(tag, parent);
            }
        }
        Self { parents }
    }

    /// Whether `group` is `ancestor` or descends from it.
    pub(in crate::app) fn is_a(&self, group: u32, ancestor: u32) -> bool {
        let mut current = group;
        for _ in 0..=self.parents.len() {
            if current == ancestor {
                return true;
            }
            match self.parents.get(&current) {
                Some(&parent) => current = parent,
                None => return false,
            }
        }
        false
    }

    /// `groups` and every group descended from any of them, each once: the
    /// groups a reference allowing `groups` takes.
    pub(in crate::app) fn expand(&self, groups: &[u32]) -> Vec<u32> {
        let mut out: Vec<u32> = groups.to_vec();
        let mut children: Vec<u32> = self
            .parents
            .keys()
            .copied()
            .filter(|child| groups.iter().any(|&group| self.is_a(*child, group)))
            .filter(|child| !groups.contains(child))
            .collect();
        children.sort_unstable();
        out.extend(children);
        out
    }
}

/// The game's group hierarchy, read once per definitions folder and game.
pub(in crate::app) fn group_hierarchy(
    definitions_root: Option<&Path>,
    game: Option<&str>,
) -> std::sync::Arc<GroupHierarchy> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Cache = Mutex<HashMap<(std::path::PathBuf, String), Arc<GroupHierarchy>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let (Some(root), Some(game)) = (definitions_root, game) else {
        return Arc::default();
    };
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry((root.to_path_buf(), game.to_owned()))
        .or_insert_with(|| Arc::new(GroupHierarchy::load(root, game)))
        .clone()
}

#[cfg(test)]
mod tests;
