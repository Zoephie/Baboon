//! Human-readable formatting for tag groups, fields, and schema-backed values.
//! It owns deterministic display formatting and name lookup; source I/O and UI state belong elsewhere.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use blam_tags::{StringIdData, TagFieldData, TagReferenceData, format_group_tag, parse_group_tag};
use serde_json::Value;

#[derive(Clone, Debug, Default)]
/// Bidirectional group-tag/name lookup assembled from definition metadata.
/// Merging keeps the first observed mapping so the default cross-game index is
/// deterministic even when several games define the same group tag.
pub struct TagNameIndex {
    name_for_group_tag: BTreeMap<u32, String>,
    group_tag_for_name: BTreeMap<String, u32>,
}

impl TagNameIndex {
    /// Builds a best-effort cross-game index, skipping unreadable game folders.
    pub fn load_from_definitions(definitions_root: &Path) -> Self {
        let mut index = TagNameIndex::default();
        let Ok(games) = std::fs::read_dir(definitions_root) else {
            return index;
        };

        for game in games.flatten() {
            let Ok(file_type) = game.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let meta_path = game.path().join("_meta.json");
            if let Ok(game_index) = TagNameIndex::load_meta(&meta_path) {
                index.merge_missing(game_index);
            }
        }
        index
    }

    /// Loads the exact metadata index for one definition game.
    pub fn load_game(definitions_root: &Path, game: &str) -> Result<Self> {
        TagNameIndex::load_meta(&definitions_root.join(game).join("_meta.json"))
    }

    /// Loads one `_meta.json`, retaining path context in parse errors.
    pub fn load_meta(meta_path: &Path) -> Result<Self> {
        let bytes = std::fs::read(meta_path)
            .with_context(|| format!("failed to read {}", meta_path.display()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", meta_path.display()))?;
        Self::from_meta_value(&value)
            .with_context(|| format!("{} missing tag_index", meta_path.display()))
    }

    /// Parse a `_meta.json` document (already deserialized) into an index.
    fn from_meta_value(value: &Value) -> Result<Self> {
        let map = value
            .get("tag_index")
            .and_then(|v| v.as_object())
            .context("missing tag_index")?;
        let mut index = TagNameIndex::default();
        for (group_tag_str, name_value) in map {
            let Some(name) = name_value.as_str() else {
                continue;
            };
            let Some(group_tag) = parse_group_tag(group_tag_str) else {
                continue;
            };
            index.name_for_group_tag.insert(group_tag, name.to_owned());
            index.group_tag_for_name.insert(name.to_owned(), group_tag);
        }
        Ok(index)
    }

    /// Returns the definition name without inventing a fallback label.
    pub fn name_for(&self, group_tag: u32) -> Option<&str> {
        self.name_for_group_tag.get(&group_tag).map(String::as_str)
    }

    pub fn group_tag_for(&self, name: &str) -> Option<u32> {
        self.group_tag_for_name.get(name).copied()
    }

    /// Adds only unknown mappings, preserving the receiver's precedence.
    pub fn merge_missing(&mut self, other: TagNameIndex) {
        for (group_tag, name) in other.name_for_group_tag {
            self.name_for_group_tag
                .entry(group_tag)
                .or_insert_with(|| name.clone());
            self.group_tag_for_name.entry(name).or_insert(group_tag);
        }
    }
}

/// Formats a field value for reports and read-only UI text.
///
/// `hex_mode` affects integer presentation only; references, strings, colors,
/// and compound values retain their established user-facing formats.
pub fn format_value(index: &TagNameIndex, value: &TagFieldData, hex_mode: bool) -> String {
    let mut s = String::new();
    write_value(index, &mut s, value, hex_mode);
    s
}

fn write_value(index: &TagNameIndex, out: &mut String, value: &TagFieldData, hex: bool) {
    use std::fmt::Write;
    match value {
        TagFieldData::String(s) | TagFieldData::LongString(s) => {
            write!(out, "\"{}\"", s).unwrap();
        }

        TagFieldData::StringId(s) | TagFieldData::OldStringId(s) => write_string_id(out, s),
        TagFieldData::TagReference(r) => write_tag_reference(index, out, r),
        TagFieldData::Data(d) => write!(out, "data [{} bytes]", d.len()).unwrap(),
        TagFieldData::ApiInterop(i) => match (i.descriptor(), i.address(), i.definition_address()) {
            (Some(d), Some(a), Some(da)) => write!(
                out,
                "api_interop {{ descriptor=0x{d:08X}, address=0x{a:08X}, definition_address=0x{da:08X} }}"
            )
            .unwrap(),
            _ => write!(out, "api_interop [{} bytes]", i.raw.len()).unwrap(),
        },

        TagFieldData::CharInteger(v) => write_int(out, *v as i128, *v as u8 as u128, 2, hex),
        TagFieldData::ShortInteger(v) => write_int(out, *v as i128, *v as u16 as u128, 4, hex),
        TagFieldData::LongInteger(v) => write_int(out, *v as i128, *v as u32 as u128, 8, hex),
        TagFieldData::Int64Integer(v) => write_int(out, *v as i128, *v as u64 as u128, 16, hex),
        TagFieldData::ByteInteger(v) => write_int(out, *v as i128, *v as u128, 2, hex),
        TagFieldData::WordInteger(v) => write_int(out, *v as i128, *v as u128, 4, hex),
        TagFieldData::DwordInteger(v) => write_int(out, *v as i128, *v as u128, 8, hex),
        TagFieldData::QwordInteger(v) => write_int(out, *v as i128, *v as u128, 16, hex),
        TagFieldData::Tag(v) => out.push_str(&format_group_tag(*v)),

        TagFieldData::CharEnum { value, name } => write_enum(out, *value as i64, name.as_deref()),
        TagFieldData::ShortEnum { value, name } => write_enum(out, *value as i64, name.as_deref()),
        TagFieldData::LongEnum { value, name } => write_enum(out, *value as i64, name.as_deref()),

        TagFieldData::ByteFlags { value, names } => write_flags(out, *value as u64, names, 2),
        TagFieldData::WordFlags { value, names } => write_flags(out, *value as u64, names, 4),
        TagFieldData::LongFlags { value, names } => {
            write_flags(out, *value as u32 as u64, names, 8)
        }

        TagFieldData::ByteBlockFlags(v) => write!(out, "0x{v:02X}").unwrap(),
        TagFieldData::WordBlockFlags(v) => write!(out, "0x{v:04X}").unwrap(),
        TagFieldData::LongBlockFlags(v) => write!(out, "0x{:08X}", *v as u32).unwrap(),

        TagFieldData::CharBlockIndex(v) | TagFieldData::CustomCharBlockIndex(v) => {
            write_block_index(out, *v as i64)
        }
        TagFieldData::ShortBlockIndex(v) | TagFieldData::CustomShortBlockIndex(v) => {
            write_block_index(out, *v as i64)
        }
        TagFieldData::LongBlockIndex(v) | TagFieldData::CustomLongBlockIndex(v) => {
            write_block_index(out, *v as i64)
        }

        TagFieldData::Angle(v) => write!(out, "{v:.4} rad ({:.2} deg)", v.to_degrees()).unwrap(),
        TagFieldData::Real(v) | TagFieldData::RealSlider(v) | TagFieldData::RealFraction(v) => {
            write!(out, "{v}").unwrap()
        }

        TagFieldData::Point2d(p) => write!(out, "{}, {}", p.x, p.y).unwrap(),
        TagFieldData::Rectangle2d(r) => {
            write!(out, "{}, {}, {}, {}", r.top, r.left, r.bottom, r.right).unwrap()
        }
        TagFieldData::RealPoint2d(p) => write!(out, "x={}, y={}", p.x, p.y).unwrap(),
        TagFieldData::RealPoint3d(p) => write!(out, "x={}, y={}, z={}", p.x, p.y, p.z).unwrap(),
        TagFieldData::RealVector2d(v) => write!(out, "i={}, j={}", v.i, v.j).unwrap(),
        TagFieldData::RealVector3d(v) => write!(out, "i={}, j={}, k={}", v.i, v.j, v.k).unwrap(),
        TagFieldData::RealQuaternion(q) => {
            write!(out, "i={}, j={}, k={}, w={}", q.i, q.j, q.k, q.w).unwrap()
        }
        TagFieldData::RealEulerAngles2d(e) => {
            write!(out, "yaw={}, pitch={}", e.yaw, e.pitch).unwrap()
        }
        TagFieldData::RealEulerAngles3d(e) => {
            write!(out, "yaw={}, pitch={}, roll={}", e.yaw, e.pitch, e.roll).unwrap()
        }
        TagFieldData::RealPlane2d(p) => write!(out, "i={}, j={}, d={}", p.i, p.j, p.d).unwrap(),
        TagFieldData::RealPlane3d(p) => {
            write!(out, "i={}, j={}, k={}, d={}", p.i, p.j, p.k, p.d).unwrap()
        }

        TagFieldData::RgbColor(c) => write!(out, "0x{:08X}", c.0).unwrap(),
        TagFieldData::ArgbColor(c) => write!(out, "0x{:08X}", c.0).unwrap(),
        TagFieldData::RealRgbColor(c) => {
            write!(out, "r={}, g={}, b={}", c.red, c.green, c.blue).unwrap()
        }
        TagFieldData::RealArgbColor(c) => {
            write!(out, "a={}, r={}, g={}, b={}", c.alpha, c.red, c.green, c.blue).unwrap()
        }
        TagFieldData::RealHsvColor(c) => {
            write!(out, "h={}, s={}, v={}", c.hue, c.saturation, c.value).unwrap()
        }
        TagFieldData::RealAhsvColor(c) => {
            write!(out, "a={}, h={}, s={}, v={}", c.alpha, c.hue, c.saturation, c.value)
                .unwrap()
        }

        TagFieldData::ShortIntegerBounds(b) => write!(out, "{}..{}", b.lower, b.upper).unwrap(),
        TagFieldData::AngleBounds(b)
        | TagFieldData::RealBounds(b)
        | TagFieldData::FractionBounds(b) => write!(out, "{}..{}", b.lower, b.upper).unwrap(),

        TagFieldData::Custom(d) => write!(out, "custom [{} bytes]", d.len()).unwrap(),
    }
}

/// Formats a four-character group tag with its definition name when known.
pub fn group_label(index: &TagNameIndex, group_tag: u32) -> String {
    match index.name_for(group_tag) {
        Some(name) => format!("{} ({})", format_group_tag(group_tag), name),
        None => format_group_tag(group_tag),
    }
}

fn write_string_id(out: &mut String, s: &StringIdData) {
    use std::fmt::Write;
    if s.string.is_empty() {
        out.push_str("NONE");
    } else {
        write!(out, "\"{}\"", s.string).unwrap();
    }
}

fn write_tag_reference(index: &TagNameIndex, out: &mut String, r: &TagReferenceData) {
    use std::fmt::Write;
    let Some((group_tag, raw_path)) = &r.group_tag_and_name else {
        return;
    };
    // On-disk paths are null-terminated; drop the trailing NUL for display.
    let path = raw_path.trim_end_matches('\u{0}');
    if path.trim().is_empty() {
        return;
    }
    match index.name_for(*group_tag) {
        Some(name) => write!(out, "{path}.{name}").unwrap(),
        None => write!(out, "{}:{path}", format_group_tag(*group_tag)).unwrap(),
    }
}

fn write_int(out: &mut String, signed: i128, hex_value: u128, width: usize, hex: bool) {
    use std::fmt::Write;
    if hex {
        write!(out, "0x{hex_value:0width$X}").unwrap();
    } else {
        write!(out, "{signed}").unwrap();
    }
}

fn write_enum(out: &mut String, value: i64, name: Option<&str>) {
    use std::fmt::Write;
    match name {
        Some(name) => write!(out, "{value} ({name})").unwrap(),
        None => write!(out, "{value}").unwrap(),
    }
}

fn write_flags(out: &mut String, value: u64, names: &[(u32, String)], hex_width: usize) {
    use std::fmt::Write;
    if names.is_empty() {
        write!(out, "0x{value:0hex_width$X} (none set)").unwrap();
    } else {
        let joined = names
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        write!(out, "0x{value:0hex_width$X} [{joined}]").unwrap();
    }
}

fn write_block_index(out: &mut String, value: i64) {
    use std::fmt::Write;
    if value == -1 {
        out.push_str("NONE");
    } else {
        write!(out, "{value}").unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn loads_group_names_from_meta_json() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("blam_tag_gui_meta_{stamp}"));
        let game = root.join("halo_test");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("_meta.json"),
            r#"{"tag_index":{"bipd":"biped","hlmt":"model"}}"#,
        )
        .unwrap();

        let index = TagNameIndex::load_from_definitions(&root);
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(index.name_for(u32::from_be_bytes(*b"bipd")), Some("biped"));
        assert_eq!(
            index.group_tag_for("model"),
            Some(u32::from_be_bytes(*b"hlmt"))
        );
        assert_eq!(
            group_label(&index, u32::from_be_bytes(*b"bipd")),
            "bipd (biped)"
        );
    }

    #[test]
    fn empty_tag_references_format_blank() {
        let index = TagNameIndex::default();
        assert_eq!(
            format_value(
                &index,
                &TagFieldData::TagReference(TagReferenceData {
                    group_tag_and_name: None
                }),
                false
            ),
            ""
        );
        assert_eq!(
            format_value(
                &index,
                &TagFieldData::TagReference(TagReferenceData {
                    group_tag_and_name: Some((u32::from_be_bytes(*b"bitm"), "\0".to_owned()))
                }),
                false
            ),
            ""
        );
    }
}
