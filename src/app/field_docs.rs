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

/// Per-group documentation, keyed by struct GUID (stable across the name
/// stripping and matching shipped tags exactly).
#[derive(Default)]
pub(super) struct DefDocs {
    by_guid: HashMap<[u8; 16], Vec<DefEntry>>,
}

impl DefDocs {
    pub(super) fn entries_for(&self, guid: &[u8; 16]) -> &[DefEntry] {
        self.by_guid.get(guid).map(Vec::as_slice).unwrap_or(&[])
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
    for st in structs.values() {
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
        docs.by_guid.entry(guid).or_insert(entries);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fields_and_explanations_keyed_by_guid() {
        let json = r#"{
            "structs": {
                "s": {
                    "guid": "4015cede9c496f80bcd3fc8804062596",
                    "fields": [
                        {"type":"explanation","name":"attenuation distances","definition":"how it attenuates"},
                        {"type":"real","name":"minimum distance:world units#start attenuating at this distance"},
                        {"type":"string_id","name":"material name^"}
                    ]
                }
            }
        }"#;
        let docs = parse_def_docs(json);
        let guid = parse_guid_hex("4015cede9c496f80bcd3fc8804062596").unwrap();
        let entries = docs.entries_for(&guid);
        assert_eq!(entries.len(), 3);
        match &entries[0] {
            DefEntry::Explanation { title, body } => {
                assert_eq!(title, "attenuation distances");
                assert_eq!(body, "how it attenuates");
            }
            _ => panic!("expected explanation first"),
        }
        match &entries[1] {
            DefEntry::Field {
                clean_name,
                help,
                unit,
                ..
            } => {
                assert_eq!(clean_name, "minimum distance");
                assert_eq!(unit.as_deref(), Some("world units"));
                assert_eq!(help.as_deref(), Some("start attenuating at this distance"));
            }
            _ => panic!("expected field"),
        }
        // `material name^` cleans to `material name`.
        match &entries[2] {
            DefEntry::Field { clean_name, .. } => assert_eq!(clean_name, "material name"),
            _ => panic!("expected field"),
        }
    }

    #[test]
    fn parses_tag_reference_allowed_groups() {
        let json = r#"{
            "structs": {
                "s": {
                    "guid": "4015cede9c496f80bcd3fc8804062596",
                    "fields": [
                        {
                            "type":"tag_reference",
                            "name":"render model",
                            "definition":{"flags":0,"allowed":["mode"]}
                        },
                        {
                            "type":"tag_reference",
                            "name":"object",
                            "definition":{"flags":0,"allowed":["bipd","vehi"]}
                        }
                    ]
                }
            }
        }"#;
        let docs = parse_def_docs(json);
        let guid = parse_guid_hex("4015cede9c496f80bcd3fc8804062596").unwrap();
        let entries = docs.entries_for(&guid);
        let render_model = parse_group_tag("mode").unwrap();
        let biped = parse_group_tag("bipd").unwrap();
        let vehicle = parse_group_tag("vehi").unwrap();

        assert!(entries.iter().any(|entry| matches!(
            entry,
            DefEntry::Field {
                clean_name,
                tag_reference_allowed,
                ..
            } if clean_name == "render model"
                && tag_reference_allowed.as_slice() == [render_model]
        )));
        assert!(entries.iter().any(|entry| matches!(
            entry,
            DefEntry::Field {
                clean_name,
                tag_reference_allowed,
                ..
            } if clean_name == "object"
                && tag_reference_allowed.as_slice() == [biped, vehicle]
        )));
    }

    #[test]
    fn inheritance_chain_resolves_parent_struct_docs() {
        // biped inherits acceleration scale (unit) + collision damage (object);
        // their struct lives in object.json, reached via the parent_tag chain
        // (biped → unit → obje). build_def_docs must merge it in.
        let docs = build_def_docs(Path::new("definitions"), "halo3_mcc", "biped");
        // The object base struct GUID (where the inherited fields live).
        let guid = parse_guid_hex("6c5aa9947a45fcf55742a488f0943380").unwrap();
        let entries = docs.entries_for(&guid);
        assert!(
            !entries.is_empty(),
            "inherited object struct must resolve via the parent chain"
        );
        let has = |target: &str| {
            entries
                .iter()
                .any(|e| matches!(e, DefEntry::Field { clean_name, .. } if clean_name == target))
        };
        assert!(
            has("acceleration scale"),
            "inherited unit field should resolve"
        );
        assert!(
            has("collision damage"),
            "inherited object field should resolve"
        );
        assert!(
            entries
                .iter()
                .any(|e| matches!(e, DefEntry::Explanation { .. })),
            "object struct should carry explanations"
        );
    }

    #[test]
    fn overlay_aligns_with_stripped_tag_by_guid_and_clean_name() {
        // End-to-end: a stripped tag's struct GUID + clean field names line up
        // with the parsed docs, so help/units + explanations can be overlaid.
        let json = std::fs::read_to_string("definitions/haloreach_mcc/sound_classes.json").unwrap();
        let docs = parse_def_docs(&json);
        let mut tag = TagFile::new("definitions/haloreach_mcc/sound_classes.json").unwrap();
        add_block_element(&mut tag, "sound classes").unwrap();
        let classes = tag
            .root()
            .field("sound classes")
            .and_then(|f| f.as_block())
            .unwrap();
        let element = classes.element(0).unwrap();
        let params = element.descend("distance parameters").unwrap();

        // GUID keying matches between the stripped layout and the JSON docs.
        let entries = docs.entries_for(&params.definition().guid());
        assert!(
            !entries.is_empty(),
            "distance-parameters struct must resolve docs by GUID"
        );
        // A stripped tag field name matches a doc entry carrying help + unit.
        assert!(params.field_names().any(|n| n == "minimum distance"));
        assert!(
            entries.iter().any(|e| matches!(
                e,
                DefEntry::Field { clean_name, help, unit, .. }
                    if clean_name == "minimum distance"
                        && help.is_some()
                        && unit.as_deref() == Some("world units")
            )),
            "`minimum distance` should overlay help + unit"
        );
        // The sound-class struct supplies explanation rows to inject.
        assert!(
            docs.entries_for(&element.definition().guid())
                .iter()
                .any(|e| matches!(e, DefEntry::Explanation { .. })),
            "sound-class struct should supply explanations"
        );
    }
}
