//! Cross-game MCC tag conversion and its preview/save workflow.

use super::*;
use blam_tags::{
    ApiInteropData, StringIdData, TagFieldMut, TagOptions, TagResourceKind, TagStructMut,
};
use sha2::{Digest, Sha256};
use std::cell::RefCell;

mod companions;
use companions::*;

pub(super) const CONVERSION_GAMES: &[&str] = &[
    "halo3_mcc",
    "halo3odst_mcc",
    "haloreach_mcc",
    "halo4_mcc",
    "halo2amp_mcc",
];

const CONVERSION_MAPPING_CATALOG: &str = include_str!("../../mappings/conversion_mappings.json");

/// These groups contain layout features which `TagFile::new` cannot currently
/// reconstruct closely enough for the native editing kits. Start from an
/// editing-kit-authored target tag so its embedded layout tables stay native.
#[cfg(test)]
const NATIVE_LAYOUT_TEMPLATE_GROUPS: &[&str] = &["particle", "model", "biped"];

#[cfg(test)]
fn requires_native_layout_template(group_name: &str) -> bool {
    NATIVE_LAYOUT_TEMPLATE_GROUPS
        .iter()
        .any(|group| group_name.eq_ignore_ascii_case(group))
}

/// Stamp a freshly-created MCC tag with the file-header generation expected by
/// the corresponding editing kit. `TagFile::new` deliberately initializes
/// these fields to zero, which is sufficient for the library's own parser but
/// is rejected (and can crash) in the native editing-kit tools.
pub(super) fn apply_editing_kit_mcc_header(tag: &mut TagFile, game: &str) -> Result<(), String> {
    let build_number = match game {
        "halo3_mcc" | "halo3odst_mcc" => 1,
        "haloreach_mcc" | "halo4_mcc" | "halo2amp_mcc" => 2,
        _ => return Err(format!("No MCC tag-header defaults are known for {game}")),
    };
    tag.header.build_version = 1;
    tag.header.build_number = build_number;
    // Stock/tool-created tags use -1 when no per-file source revision is known.
    tag.header.version = u32::MAX;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ConversionIssueKind {
    Unsupported,
    Truncated,
    Warning,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct ConversionIssue {
    pub(in crate::app) kind: ConversionIssueKind,
    pub(in crate::app) path: String,
    pub(in crate::app) message: String,
}

#[derive(Default)]
pub(in crate::app) struct TagConversionReport {
    pub(in crate::app) copied_exact: usize,
    pub(in crate::app) converted_semantic: usize,
    pub(in crate::app) mapped_aliases: usize,
    pub(in crate::app) defaulted_target: usize,
    pub(in crate::app) unsupported_source: usize,
    pub(in crate::app) truncated: usize,
    pub(in crate::app) issues: Vec<ConversionIssue>,
}

pub(in crate::app) struct TagConversionDraft {
    pub(in crate::app) tag: TagFile,
    pub(in crate::app) companion_tags: Vec<CompanionTagDraft>,
    pub(in crate::app) report: TagConversionReport,
    pub(in crate::app) source_fingerprint: [u8; 32],
    pub(in crate::app) target_group_name: String,
    pub(in crate::app) target_extension: String,
    pub(in crate::app) native_layout_template: Option<PathBuf>,
}

pub(in crate::app) struct CompanionTagDraft {
    pub(in crate::app) key: String,
    pub(in crate::app) file_suffix: String,
    pub(in crate::app) group_name: String,
    pub(in crate::app) extension: String,
    pub(in crate::app) tag: TagFile,
    pub(in crate::app) native_layout_template: Option<PathBuf>,
}

pub(in crate::app) struct TagConversionDialog {
    pub(in crate::app) source_key: String,
    pub(in crate::app) source_label: String,
    pub(in crate::app) source_game: String,
    pub(in crate::app) target_game: String,
    pub(in crate::app) draft: Option<TagConversionDraft>,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) pending_source_destination: Option<PathBuf>,
}

pub(in crate::app) struct FolderConversionDialog {
    pub(in crate::app) source_rel_path: PathBuf,
    pub(in crate::app) source_label: String,
    pub(in crate::app) source_game: String,
    pub(in crate::app) target_game: String,
    pub(in crate::app) destination_parent: Option<PathBuf>,
    pub(in crate::app) running: bool,
    pub(in crate::app) progress: Option<FolderConversionProgress>,
    pub(in crate::app) report: Option<FolderConversionReport>,
    pub(in crate::app) error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum FolderConversionFileStatus {
    NativeLayout,
    GeneratedLayout,
    Failed,
}

pub(in crate::app) struct FolderConversionFileResult {
    pub(in crate::app) source: String,
    pub(in crate::app) output: Option<PathBuf>,
    pub(in crate::app) status: FolderConversionFileStatus,
    pub(in crate::app) overwritten: bool,
    pub(in crate::app) detail: String,
}

pub(in crate::app) struct FolderConversionReport {
    pub(in crate::app) source_label: String,
    pub(in crate::app) target_game: String,
    pub(in crate::app) destination_root: PathBuf,
    pub(in crate::app) files: Vec<FolderConversionFileResult>,
    pub(in crate::app) ignored_files: Vec<String>,
}

impl FolderConversionReport {
    pub(in crate::app) fn native_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.status == FolderConversionFileStatus::NativeLayout)
            .count()
    }

    pub(in crate::app) fn generated_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.status == FolderConversionFileStatus::GeneratedLayout)
            .count()
    }

    pub(in crate::app) fn failed_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.status == FolderConversionFileStatus::Failed)
            .count()
    }

    fn converted_count(&self) -> usize {
        self.native_count() + self.generated_count()
    }
}

#[derive(Default)]
struct GameTagIndex {
    by_tag: HashMap<u32, String>,
    by_name: HashMap<String, u32>,
}

#[derive(Default)]
struct NativeTemplateIndex {
    by_group: HashMap<u32, Vec<PathBuf>>,
    cached: RefCell<HashMap<u32, Option<(Vec<u8>, PathBuf)>>>,
}

impl NativeTemplateIndex {
    fn build(tags_root: &Path, groups: &GameTagIndex) -> Self {
        let mut by_extension = HashMap::new();
        for (group_tag, group_name) in &groups.by_tag {
            let extension = group_tag_to_extension(*group_tag).unwrap_or(group_name);
            by_extension.insert(extension.to_ascii_lowercase(), *group_tag);
        }
        let mut result = Self::default();
        for item in walkdir::WalkDir::new(tags_root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !item.file_type().is_file() {
                continue;
            }
            let Some(extension) = item.path().extension().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(group_tag) = by_extension.get(&extension.to_ascii_lowercase()).copied() else {
                continue;
            };
            result
                .by_group
                .entry(group_tag)
                .or_default()
                .push(item.into_path());
        }
        for paths in result.by_group.values_mut() {
            paths.sort();
        }
        result
    }
}

impl GameTagIndex {
    fn load(definitions_root: &Path, game: &str) -> Result<Self, String> {
        let path = definitions_root.join(game).join("_meta.json");
        let bytes = fs::read(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
        let entries = value
            .get("tag_index")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{} is missing tag_index", path.display()))?;
        let mut index = Self::default();
        for (fourcc, name) in entries {
            let (Some(tag), Some(name)) = (parse_group_tag(fourcc), name.as_str()) else {
                continue;
            };
            index.by_tag.insert(tag, name.to_owned());
            index.by_name.insert(name.to_ascii_lowercase(), tag);
        }
        Ok(index)
    }
}

struct ConversionContext<'a> {
    source_groups: &'a GameTagIndex,
    target_groups: &'a GameTagIndex,
    source_field_aliases: &'a SchemaFieldAliases,
    target_field_aliases: &'a SchemaFieldAliases,
    mapping_catalog: &'a ConversionMappingCatalog,
    definitions_root: &'a Path,
    native_templates: Option<&'a NativeTemplateIndex>,
    source_game: &'a str,
    target_game: &'a str,
    group_name: &'a str,
    report: TagConversionReport,
    companion_tags: Vec<CompanionTagDraft>,
    fatal_error: Option<String>,
    root_matches: usize,
}

#[derive(serde::Deserialize)]
struct ConversionMappingCatalog {
    version: u32,
    coverage: String,
    /// Canonical groups whose five-game mapping surface has been reviewed.
    /// Most fields in these groups deliberately remain schema-derived; this
    /// list makes that coverage explicit and machine-checkable without
    /// duplicating thousands of identical field names in the JSON catalog.
    #[serde(default)]
    covered_groups: Vec<String>,
    #[serde(default)]
    struct_mappings: Vec<StructMappingRule>,
    #[serde(default)]
    incompatible_pairs: Vec<IncompatiblePairRule>,
    #[serde(default)]
    unusable_schemas: Vec<UnusableSchemaRule>,
    #[serde(default)]
    reference_drops: Vec<ReferenceDropRule>,
    #[serde(default)]
    field_aliases: Vec<FieldAliasRule>,
    #[serde(default)]
    option_aliases: Vec<OptionAliasRule>,
}

#[derive(serde::Deserialize)]
struct FieldAliasRule {
    group: String,
    #[serde(default)]
    source_games: Vec<String>,
    #[serde(default)]
    target_games: Vec<String>,
    source_struct_guid: Option<String>,
    target_struct_guid: Option<String>,
    source: String,
    target: String,
}

#[derive(serde::Deserialize)]
struct StructMappingRule {
    group: String,
    source_games: Vec<String>,
    target_games: Vec<String>,
    source_path: String,
    target_path: String,
}

#[derive(serde::Deserialize)]
struct IncompatiblePairRule {
    group: String,
    source_games: Vec<String>,
    target_games: Vec<String>,
    reason: String,
}

#[derive(serde::Deserialize)]
struct UnusableSchemaRule {
    group: String,
    games: Vec<String>,
    reason: String,
}

#[derive(serde::Deserialize)]
struct ReferenceDropRule {
    group: String,
    source_games: Vec<String>,
    target_games: Vec<String>,
    source_path: String,
    reason: String,
}

#[derive(serde::Deserialize)]
struct OptionAliasRule {
    group: String,
    field: String,
    #[serde(default)]
    source_games: Vec<String>,
    #[serde(default)]
    target_games: Vec<String>,
    source: String,
    target: String,
}

impl ConversionMappingCatalog {
    fn load() -> Result<Self, String> {
        let catalog: Self = serde_json::from_str(CONVERSION_MAPPING_CATALOG)
            .map_err(|error| format!("Could not parse conversion_mappings.json: {error}"))?;
        if catalog.version != 1 {
            return Err(format!(
                "Unsupported conversion mapping catalog version {}",
                catalog.version
            ));
        }
        if catalog.coverage != "all_supported_groups" {
            return Err(format!(
                "Unsupported conversion mapping coverage policy {}",
                catalog.coverage
            ));
        }
        let mut covered_groups = HashSet::new();
        for (index, group) in catalog.covered_groups.iter().enumerate() {
            let normalized = group.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                return Err(format!(
                    "conversion_mappings.json covered_groups[{index}] is empty"
                ));
            }
            if !covered_groups.insert(normalized) {
                return Err(format!(
                    "conversion_mappings.json covered_groups[{index}] duplicates {group}"
                ));
            }
        }
        for (index, rule) in catalog.field_aliases.iter().enumerate() {
            validate_mapping_rule_scope(
                "field_aliases",
                index,
                &rule.group,
                &rule.source_games,
                &rule.target_games,
                &rule.source,
                &rule.target,
            )?;
            for (label, guid) in [
                ("source_struct_guid", rule.source_struct_guid.as_deref()),
                ("target_struct_guid", rule.target_struct_guid.as_deref()),
            ] {
                if guid.is_some_and(|guid| parse_schema_guid(guid).is_none()) {
                    return Err(format!(
                        "conversion_mappings.json field_aliases[{index}] has an invalid {label}"
                    ));
                }
            }
        }
        for (index, rule) in catalog.struct_mappings.iter().enumerate() {
            validate_game_scopes(
                "struct_mappings",
                index,
                &rule.group,
                &rule.source_games,
                &rule.target_games,
            )?;
            if rule.source_path.split('/').any(str::is_empty) && !rule.source_path.is_empty()
                || rule.target_path.split('/').any(str::is_empty) && !rule.target_path.is_empty()
            {
                return Err(format!(
                    "conversion_mappings.json struct_mappings[{index}] has an invalid path"
                ));
            }
        }
        for (index, rule) in catalog.incompatible_pairs.iter().enumerate() {
            validate_game_scopes(
                "incompatible_pairs",
                index,
                &rule.group,
                &rule.source_games,
                &rule.target_games,
            )?;
            if rule.reason.trim().is_empty() {
                return Err(format!(
                    "conversion_mappings.json incompatible_pairs[{index}] has no reason"
                ));
            }
        }
        for (index, rule) in catalog.unusable_schemas.iter().enumerate() {
            validate_game_scopes(
                "unusable_schemas",
                index,
                &rule.group,
                &rule.games,
                &rule.games,
            )?;
            if rule.reason.trim().is_empty() {
                return Err(format!(
                    "conversion_mappings.json unusable_schemas[{index}] has no reason"
                ));
            }
        }
        for (index, rule) in catalog.reference_drops.iter().enumerate() {
            validate_game_scopes(
                "reference_drops",
                index,
                &rule.group,
                &rule.source_games,
                &rule.target_games,
            )?;
            if clean_field_key(&rule.source_path).is_empty() || rule.reason.trim().is_empty() {
                return Err(format!(
                    "conversion_mappings.json reference_drops[{index}] has an empty path or reason"
                ));
            }
        }
        for (index, rule) in catalog.option_aliases.iter().enumerate() {
            validate_mapping_rule_scope(
                "option_aliases",
                index,
                &rule.group,
                &rule.source_games,
                &rule.target_games,
                &rule.source,
                &rule.target,
            )?;
            if normalize_option_name(&rule.field).is_empty() {
                return Err(format!(
                    "conversion_mappings.json option_aliases[{index}] has an empty field"
                ));
            }
        }
        Ok(catalog)
    }

    fn field_names_match(&self, request: FieldMappingRequest<'_>) -> bool {
        self.field_aliases.iter().any(|rule| {
            if !rule.group.eq_ignore_ascii_case(request.group) {
                return false;
            }
            mapping_rule_direction_matches(
                &rule.source_games,
                &rule.target_games,
                request.source_game,
                request.target_game,
                &rule.source,
                &rule.target,
                request.source_name,
                request.target_name,
            ) && guid_rule_matches(
                rule.source_struct_guid.as_deref(),
                rule.target_struct_guid.as_deref(),
                request.source_guid,
                request.target_guid,
                request.source_game,
                request.target_game,
                &rule.source_games,
                &rule.target_games,
            )
        })
    }

    fn option_names_match(
        &self,
        group: &str,
        field_path: &str,
        source_game: &str,
        target_game: &str,
        source_name: &str,
        target_name: &str,
    ) -> bool {
        let field = field_path
            .rsplit('/')
            .next()
            .unwrap_or(field_path)
            .split('[')
            .next()
            .unwrap_or(field_path);
        self.option_aliases.iter().any(|rule| {
            rule.group.eq_ignore_ascii_case(group)
                && normalize_option_name(&rule.field) == normalize_option_name(field)
                && mapping_rule_direction_matches(
                    &rule.source_games,
                    &rule.target_games,
                    source_game,
                    target_game,
                    &rule.source,
                    &rule.target,
                    source_name,
                    target_name,
                )
        })
    }

    fn struct_mapping<'a>(
        &'a self,
        group: &str,
        source_game: &str,
        target_game: &str,
    ) -> Option<(&'a str, &'a str)> {
        self.struct_mappings.iter().find_map(|rule| {
            if !rule.group.eq_ignore_ascii_case(group) {
                return None;
            }
            if game_scope_matches(&rule.source_games, source_game)
                && game_scope_matches(&rule.target_games, target_game)
            {
                Some((rule.source_path.as_str(), rule.target_path.as_str()))
            } else if game_scope_matches(&rule.source_games, target_game)
                && game_scope_matches(&rule.target_games, source_game)
            {
                Some((rule.target_path.as_str(), rule.source_path.as_str()))
            } else {
                None
            }
        })
    }

    fn incompatibility_reason<'a>(
        &'a self,
        group: &str,
        source_game: &str,
        target_game: &str,
    ) -> Option<&'a str> {
        self.incompatible_pairs.iter().find_map(|rule| {
            (rule.group.eq_ignore_ascii_case(group)
                && ((game_scope_matches(&rule.source_games, source_game)
                    && game_scope_matches(&rule.target_games, target_game))
                    || (game_scope_matches(&rule.source_games, target_game)
                        && game_scope_matches(&rule.target_games, source_game))))
            .then_some(rule.reason.as_str())
        })
    }

    fn unusable_schema_reason<'a>(&'a self, group: &str, game: &str) -> Option<&'a str> {
        self.unusable_schemas.iter().find_map(|rule| {
            (rule.group.eq_ignore_ascii_case(group) && game_scope_matches(&rule.games, game))
                .then_some(rule.reason.as_str())
        })
    }

    fn reference_drop_reason<'a>(
        &'a self,
        group: &str,
        source_game: &str,
        target_game: &str,
        source_path: &str,
    ) -> Option<&'a str> {
        self.reference_drops.iter().find_map(|rule| {
            (rule.group.eq_ignore_ascii_case(group)
                && game_scope_matches(&rule.source_games, source_game)
                && game_scope_matches(&rule.target_games, target_game)
                && clean_field_key(&rule.source_path) == clean_field_key(source_path))
            .then_some(rule.reason.as_str())
        })
    }
}

fn validate_game_scopes(
    section: &str,
    index: usize,
    group: &str,
    source_games: &[String],
    target_games: &[String],
) -> Result<(), String> {
    if group.trim().is_empty() || source_games.is_empty() || target_games.is_empty() {
        return Err(format!(
            "conversion_mappings.json {section}[{index}] has an empty group or game scope"
        ));
    }
    for game in source_games.iter().chain(target_games) {
        if !CONVERSION_GAMES.contains(&game.as_str()) {
            return Err(format!(
                "conversion_mappings.json {section}[{index}] uses unsupported game {game}"
            ));
        }
    }
    Ok(())
}

fn validate_mapping_rule_scope(
    section: &str,
    index: usize,
    group: &str,
    source_games: &[String],
    target_games: &[String],
    source: &str,
    target: &str,
) -> Result<(), String> {
    if group.trim().is_empty()
        || normalize_option_name(source).is_empty()
        || normalize_option_name(target).is_empty()
    {
        return Err(format!(
            "conversion_mappings.json {section}[{index}] has an empty group or name"
        ));
    }
    for game in source_games.iter().chain(target_games) {
        if !CONVERSION_GAMES.contains(&game.as_str()) {
            return Err(format!(
                "conversion_mappings.json {section}[{index}] uses unsupported game {game}"
            ));
        }
    }
    Ok(())
}

struct FieldMappingRequest<'a> {
    group: &'a str,
    source_game: &'a str,
    target_game: &'a str,
    source_guid: [u8; 16],
    target_guid: [u8; 16],
    source_name: &'a str,
    target_name: &'a str,
}

fn mapping_rule_direction_matches(
    source_games: &[String],
    target_games: &[String],
    source_game: &str,
    target_game: &str,
    rule_source: &str,
    rule_target: &str,
    source_name: &str,
    target_name: &str,
) -> bool {
    let forward = game_scope_matches(source_games, source_game)
        && game_scope_matches(target_games, target_game)
        && normalized_names_equal(rule_source, source_name)
        && normalized_names_equal(rule_target, target_name);
    let reverse = game_scope_matches(source_games, target_game)
        && game_scope_matches(target_games, source_game)
        && normalized_names_equal(rule_source, target_name)
        && normalized_names_equal(rule_target, source_name);
    forward || reverse
}

fn game_scope_matches(games: &[String], game: &str) -> bool {
    games.is_empty() || games.iter().any(|candidate| candidate == game)
}

fn normalized_names_equal(left: &str, right: &str) -> bool {
    normalize_option_name(left) == normalize_option_name(right)
}

fn guid_rule_matches(
    source_rule: Option<&str>,
    target_rule: Option<&str>,
    source_guid: [u8; 16],
    target_guid: [u8; 16],
    source_game: &str,
    target_game: &str,
    source_games: &[String],
    target_games: &[String],
) -> bool {
    let source_rule = source_rule.and_then(parse_schema_guid);
    let target_rule = target_rule.and_then(parse_schema_guid);
    let forward = game_scope_matches(source_games, source_game)
        && game_scope_matches(target_games, target_game)
        && source_rule.is_none_or(|guid| guid == source_guid)
        && target_rule.is_none_or(|guid| guid == target_guid);
    let reverse = game_scope_matches(source_games, target_game)
        && game_scope_matches(target_games, source_game)
        && source_rule.is_none_or(|guid| guid == target_guid)
        && target_rule.is_none_or(|guid| guid == source_guid);
    forward || reverse
}

#[derive(Default)]
struct SchemaFieldAliases {
    by_struct: HashMap<[u8; 16], HashMap<String, HashSet<String>>>,
}

impl SchemaFieldAliases {
    fn load(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
        let mut result = Self::default();
        let Some(structs) = value.get("structs").and_then(Value::as_object) else {
            return Ok(result);
        };
        for structure in structs.values() {
            let (Some(guid), Some(fields)) = (
                structure
                    .get("guid")
                    .and_then(Value::as_str)
                    .and_then(parse_schema_guid),
                structure.get("fields").and_then(Value::as_array),
            ) else {
                continue;
            };
            let aliases = result.by_struct.entry(guid).or_default();
            for field in fields {
                let Some(name) = field.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let names = option_name_aliases(name.split(['#', ':']).next().unwrap_or(name));
                for name in &names {
                    aliases
                        .entry(name.clone())
                        .or_default()
                        .extend(names.iter().filter(|alias| *alias != name).cloned());
                }
            }
        }
        Ok(result)
    }

    fn matches(&self, guid: [u8; 16], left: &str, right: &str) -> bool {
        self.by_struct
            .get(&guid)
            .and_then(|fields| fields.get(left))
            .is_some_and(|aliases| aliases.contains(right))
    }
}

fn parse_schema_guid(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    let mut result = [0u8; 16];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(result)
}

#[derive(Clone)]
struct TargetFieldInfo {
    ordinal: usize,
    name: String,
    key: String,
    field_type: TagFieldType,
}

pub(super) fn tag_fingerprint(tag: &TagFile) -> Result<[u8; 32], String> {
    let bytes = tag.write_to_bytes().map_err(|error| error.to_string())?;
    Ok(Sha256::digest(bytes).into())
}

pub(super) fn analyze_conversion(
    source: &TagFile,
    source_game: &str,
    target_game: &str,
    definitions_root: &Path,
    target_tags_root: Option<&Path>,
) -> Result<TagConversionDraft, String> {
    let target_groups = GameTagIndex::load(definitions_root, target_game)?;
    let native_templates =
        target_tags_root.map(|root| NativeTemplateIndex::build(root, &target_groups));
    analyze_conversion_with_templates(
        source,
        source_game,
        target_game,
        definitions_root,
        native_templates.as_ref(),
    )
}

fn analyze_conversion_with_templates(
    source: &TagFile,
    source_game: &str,
    target_game: &str,
    definitions_root: &Path,
    native_templates: Option<&NativeTemplateIndex>,
) -> Result<TagConversionDraft, String> {
    if source.classic_engine().is_some() || source.endian != Endian::Le {
        return Err("Only little-endian MCC tags can be converted".to_owned());
    }
    if !CONVERSION_GAMES.contains(&source_game) || !CONVERSION_GAMES.contains(&target_game) {
        return Err(
            "The selected source or target profile is not supported by this converter".to_owned(),
        );
    }
    if source_game == target_game {
        return Err("Choose a different target game profile".to_owned());
    }

    let source_groups = GameTagIndex::load(definitions_root, source_game)?;
    let target_groups = GameTagIndex::load(definitions_root, target_game)?;
    let source_group_name = source_groups
        .by_tag
        .get(&source.group().tag)
        .ok_or_else(|| {
            format!(
                "{} does not identify group {}",
                source_game,
                format_group_tag(source.group().tag)
            )
        })?;
    let target_group_tag = target_groups
        .by_name
        .get(&source_group_name.to_ascii_lowercase())
        .copied()
        .ok_or_else(|| format!("{target_game} has no {source_group_name} tag group"))?;
    let target_group_name = target_groups
        .by_tag
        .get(&target_group_tag)
        .cloned()
        .unwrap_or_else(|| source_group_name.clone());
    let source_schema_path = definitions_root
        .join(source_game)
        .join(format!("{source_group_name}.json"));
    let schema_path = definitions_root
        .join(target_game)
        .join(format!("{target_group_name}.json"));
    let source_field_aliases = SchemaFieldAliases::load(&source_schema_path)?;
    let target_field_aliases = SchemaFieldAliases::load(&schema_path)?;
    let mapping_catalog = ConversionMappingCatalog::load()?;
    let native_target = native_templates
        .map(|templates| find_native_target_template(templates, target_group_tag))
        .transpose()?
        .flatten();
    for game in [source_game, target_game] {
        if let Some(reason) = mapping_catalog.unusable_schema_reason(source_group_name, game) {
            let native_layout_avoids_schema_construction = game == target_game
                && native_target.is_some()
                && source_group_name.eq_ignore_ascii_case("contrail_system");
            if native_layout_avoids_schema_construction || game == source_game {
                continue;
            }
            return Err(format!(
                "{game} {source_group_name} schema cannot be converted safely: {reason}"
            ));
        }
    }
    if let Some(reason) =
        mapping_catalog.incompatibility_reason(source_group_name, source_game, target_game)
    {
        let native_contrail_layout =
            native_target.is_some() && source_group_name.eq_ignore_ascii_case("contrail_system");
        if !native_contrail_layout {
            return Err(format!(
                "{source_game} and {target_game} {source_group_name} layouts are explicitly incompatible: {reason}"
            ));
        }
    }
    let (mut target, target_template) = if let Some((template, template_path)) = native_target {
        (template, Some(template_path))
    } else {
        let mut target = TagFile::new(&schema_path).map_err(|error| {
            format!(
                "Could not create target tag from {}: {error}",
                schema_path.display()
            )
        })?;
        initialize_block_index_defaults(target.root_mut());
        (target, None)
    };
    apply_editing_kit_mcc_header(&mut target, target_game)?;

    let mut context = ConversionContext {
        source_groups: &source_groups,
        target_groups: &target_groups,
        source_field_aliases: &source_field_aliases,
        target_field_aliases: &target_field_aliases,
        mapping_catalog: &mapping_catalog,
        definitions_root,
        native_templates,
        source_game,
        target_game,
        group_name: source_group_name,
        report: TagConversionReport::default(),
        companion_tags: Vec::new(),
        fatal_error: None,
        root_matches: 0,
    };
    if let Some(template_path) = target_template.as_ref() {
        context.report.issues.push(ConversionIssue {
            kind: ConversionIssueKind::Warning,
            path: "target layout".to_owned(),
            message: format!(
                "Used native {target_group_name} layout template {} and cleared its values before conversion",
                template_path.display()
            ),
        });
    } else {
        context.report.issues.push(ConversionIssue {
            kind: ConversionIssueKind::Warning,
            path: "target layout".to_owned(),
            message: format!(
                "Used generated {target_group_name} layout; Baboon round-trip verification cannot prove native editing-kit stream compatibility"
            ),
        });
    }
    if let Some((source_path, target_path)) =
        mapping_catalog.struct_mapping(source_group_name, source_game, target_game)
    {
        let source_struct = struct_at_path(source.root(), source_path).ok_or_else(|| {
            format!(
                "Configured source struct path '{source_path}' was not found in {source_game} {source_group_name}"
            )
        })?;
        if !convert_to_struct_path(source_struct, target.root_mut(), target_path, &mut context) {
            return Err(format!(
                "Configured target struct path '{target_path}' was not found in {target_game} {target_group_name}"
            ));
        }
    } else {
        convert_struct(source.root(), target.root_mut(), "", true, &mut context);
    }
    if let Some(error) = context.fatal_error.take() {
        return Err(error);
    }
    if context.root_matches == 0 {
        return Err(format!(
            "{} and {} do not share a compatible root structure for {}",
            source_game, target_game, source_group_name
        ));
    }

    let dependency_schema = definitions_root
        .join(target_game)
        .join("tag_dependency_list.json");
    if dependency_schema.is_file() {
        target
            .rebuild_dependency_list(&dependency_schema)
            .map_err(|error| format!("Could not rebuild target dependencies: {error}"))?;
    } else {
        context.report.issues.push(ConversionIssue {
            kind: ConversionIssueKind::Warning,
            path: "dependency list".to_owned(),
            message: format!(
                "Target dependency schema is missing: {}",
                dependency_schema.display()
            ),
        });
    }

    validate_reference_fidelity(
        source,
        &target,
        &source_groups,
        &target_groups,
        source_group_name,
        source_game,
        target_game,
        &mapping_catalog,
        &mut context.report,
    )?;
    validate_critical_runtime_safety(source, &context)?;

    let target_extension = group_tag_to_extension(target_group_tag)
        .unwrap_or(&target_group_name)
        .to_owned();
    Ok(TagConversionDraft {
        tag: target,
        companion_tags: context.companion_tags,
        report: context.report,
        source_fingerprint: tag_fingerprint(source)?,
        target_group_name,
        target_extension,
        native_layout_template: target_template,
    })
}

fn validate_critical_runtime_safety(
    source: &TagFile,
    context: &ConversionContext<'_>,
) -> Result<(), String> {
    if struct_contains_non_null_resource(source.root()) {
        return Err(format!(
            "{} contains non-null pageable runtime resources that cannot yet be translated safely from {} to {}",
            context.group_name, context.source_game, context.target_game
        ));
    }
    const FAIL_CLOSED_GROUPS: &[&str] = &[
        "model_animation_graph",
        "damage_effect",
        "effect",
        "lens_flare",
        "light",
        "particle",
    ];
    let critical_issues = context
        .report
        .issues
        .iter()
        .filter(|issue| issue.kind == ConversionIssueKind::Unsupported)
        .filter(|issue| {
            context
                .mapping_catalog
                .reference_drop_reason(
                    context.group_name,
                    context.source_game,
                    context.target_game,
                    &issue.path,
                )
                .is_none()
        })
        .filter(|issue| {
            !(context
                .group_name
                .eq_ignore_ascii_case("model_animation_graph")
                && ["desired compression", "current compression"]
                    .iter()
                    .any(|field| clean_field_key(&issue.path).ends_with(field)))
        })
        .collect::<Vec<_>>();
    let animation_graph = context
        .group_name
        .eq_ignore_ascii_case("model_animation_graph");
    let audited_h3_to_reach = ["halo3_mcc", "halo3odst_mcc"].contains(&context.source_game)
        && context.target_game == "haloreach_mcc";
    if (animation_graph
        || (audited_h3_to_reach
            && FAIL_CLOSED_GROUPS
                .iter()
                .any(|group| context.group_name.eq_ignore_ascii_case(group))))
        && !critical_issues.is_empty()
    {
        let examples = critical_issues
            .iter()
            .take(4)
            .map(|issue| issue.path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "{} conversion would lose {} meaningful runtime or authored field(s) ({examples}); the tag was not written",
            context.group_name,
            critical_issues.len()
        ));
    }
    Ok(())
}

fn struct_contains_non_null_resource(structure: TagStruct<'_>) -> bool {
    structure.fields().any(|field| {
        field
            .as_resource()
            .is_some_and(|resource| !matches!(resource.kind(), TagResourceKind::Null))
            || field
                .as_struct()
                .is_some_and(struct_contains_non_null_resource)
            || field
                .as_block()
                .is_some_and(|block| block.iter().any(struct_contains_non_null_resource))
            || field
                .as_array()
                .is_some_and(|array| array.iter().any(struct_contains_non_null_resource))
    })
}

fn struct_at_path<'a>(mut structure: TagStruct<'a>, path: &str) -> Option<TagStruct<'a>> {
    for component in path.split('/').filter(|component| !component.is_empty()) {
        structure = structure
            .fields()
            .find(|field| clean_field_key(field.name()) == clean_field_key(component))?
            .as_struct()?;
    }
    Some(structure)
}

fn convert_to_struct_path(
    source: TagStruct<'_>,
    mut target: TagStructMut<'_>,
    path: &str,
    context: &mut ConversionContext<'_>,
) -> bool {
    let mut components = path.split('/').filter(|component| !component.is_empty());
    let Some(component) = components.next() else {
        convert_struct(source, target, "", true, context);
        return true;
    };
    let remainder = components.collect::<Vec<_>>().join("/");
    let ordinal = target
        .as_ref()
        .fields()
        .enumerate()
        .find(|(_, field)| clean_field_key(field.name()) == clean_field_key(component))
        .map(|(ordinal, _)| ordinal);
    let Some(ordinal) = ordinal else {
        return false;
    };
    let Some(mut field) = target.field_at_mut(ordinal) else {
        return false;
    };
    let Some(nested) = field.as_struct_mut() else {
        return false;
    };
    convert_to_struct_path(source, nested, &remainder, context)
}

fn find_native_target_template(
    templates: &NativeTemplateIndex,
    target_group_tag: u32,
) -> Result<Option<(TagFile, PathBuf)>, String> {
    {
        let cached = templates.cached.borrow();
        if let Some(value) = cached.get(&target_group_tag) {
            return match value {
                Some((bytes, path)) => TagFile::read_from_bytes(bytes)
                    .map(|tag| Some((tag, path.clone())))
                    .map_err(|error| format!("Could not restore cached native template: {error}")),
                None => Ok(None),
            };
        }
    }
    let Some(paths) = templates.by_group.get(&target_group_tag) else {
        templates.cached.borrow_mut().insert(target_group_tag, None);
        return Ok(None);
    };
    for path in paths {
        let Ok(mut tag) = TagFile::read(path) else {
            continue;
        };
        // Converted drafts use -1. Prefer an editing-kit-authored tag so its
        // embedded layout contains expansions for custom schema fields.
        if tag.group().tag == target_group_tag
            && tag.classic_engine().is_none()
            && tag.endian == Endian::Le
            && tag.header.version != u32::MAX
            && reset_tag_to_defaults(&mut tag).is_ok()
        {
            let bytes = tag.write_to_bytes().map_err(|error| {
                format!(
                    "Could not cache native template {}: {error}",
                    path.display()
                )
            })?;
            templates
                .cached
                .borrow_mut()
                .insert(target_group_tag, Some((bytes, path.clone())));
            return Ok(Some((tag, path.to_path_buf())));
        }
    }
    templates.cached.borrow_mut().insert(target_group_tag, None);
    Ok(None)
}

fn create_companion_tag(
    key: &str,
    file_suffix: &str,
    group_name: &str,
    context: &ConversionContext<'_>,
) -> Result<CompanionTagDraft, String> {
    let group_tag = context
        .target_groups
        .by_name
        .get(&group_name.to_ascii_lowercase())
        .copied()
        .ok_or_else(|| format!("{} has no {group_name} tag group", context.target_game))?;
    let native_target = context
        .native_templates
        .map(|templates| find_native_target_template(templates, group_tag))
        .transpose()?
        .flatten();
    let schema = context
        .definitions_root
        .join(context.target_game)
        .join(format!("{group_name}.json"));
    let (mut tag, native_layout_template) = if let Some((template, template_path)) = native_target {
        (template, Some(template_path))
    } else {
        let mut tag = TagFile::new(&schema).map_err(|error| {
            format!(
                "Could not create companion {group_name} tag from {}: {error}",
                schema.display()
            )
        })?;
        initialize_block_index_defaults(tag.root_mut());
        (tag, None)
    };
    apply_editing_kit_mcc_header(&mut tag, context.target_game)?;
    let extension = group_tag_to_extension(group_tag)
        .unwrap_or(group_name)
        .to_owned();
    Ok(CompanionTagDraft {
        key: key.to_owned(),
        file_suffix: file_suffix.to_owned(),
        group_name: group_name.to_owned(),
        extension,
        tag,
        native_layout_template,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct ReferenceValue {
    field_path: String,
    group_tag: u32,
    tag_path: String,
}

fn collect_reference_values(
    structure: TagStruct<'_>,
    parent_path: &str,
    values: &mut Vec<ReferenceValue>,
) {
    for field in structure.fields() {
        let key = clean_field_key(field.name());
        let field_path = join_path(
            parent_path,
            if key.is_empty() {
                field.type_name()
            } else {
                &key
            },
        );
        if let Some(TagFieldData::TagReference(reference)) = field.value()
            && let Some((group_tag, path)) = reference.group_tag_and_name
            && !path.is_empty()
            && !path.eq_ignore_ascii_case("none")
        {
            values.push(ReferenceValue {
                field_path,
                group_tag,
                tag_path: path,
            });
            continue;
        }
        if let Some(nested) = field.as_struct() {
            collect_reference_values(nested, &field_path, values);
        } else if let Some(block) = field.as_block() {
            for (index, element) in block.iter().enumerate() {
                collect_reference_values(element, &format!("{field_path}[{index}]"), values);
            }
        } else if let Some(array) = field.as_array() {
            for (index, element) in array.iter().enumerate() {
                collect_reference_values(element, &format!("{field_path}[{index}]"), values);
            }
        }
    }
}

fn validate_reference_fidelity(
    source: &TagFile,
    target: &TagFile,
    source_groups: &GameTagIndex,
    target_groups: &GameTagIndex,
    group_name: &str,
    source_game: &str,
    target_game: &str,
    mapping_catalog: &ConversionMappingCatalog,
    report: &mut TagConversionReport,
) -> Result<(), String> {
    let mut source_values = Vec::new();
    collect_reference_values(source.root(), "", &mut source_values);
    let mut expected = HashSet::<(u32, String)>::new();
    for reference in source_values {
        if let Some(reason) = mapping_catalog.reference_drop_reason(
            group_name,
            source_game,
            target_game,
            &reference.field_path,
        ) {
            report.issues.push(ConversionIssue {
                kind: ConversionIssueKind::Warning,
                path: reference.field_path,
                message: format!(
                    "Target schema has no safe slot for reference {}: {reason}",
                    reference.tag_path
                ),
            });
            continue;
        }
        let source_group_name =
            source_groups
                .by_tag
                .get(&reference.group_tag)
                .ok_or_else(|| {
                    format!(
                        "Cannot preserve reference {}: unknown source group {}",
                        reference.tag_path,
                        format_group_tag(reference.group_tag)
                    )
                })?;
        let target_group = target_groups
            .by_name
            .get(&source_group_name.to_ascii_lowercase())
            .copied()
            .ok_or_else(|| {
                format!(
                    "Cannot preserve reference {}: target has no {source_group_name} group",
                    reference.tag_path
                )
            })?;
        expected.insert((target_group, reference.tag_path));
    }

    let mut actual_values = Vec::new();
    collect_reference_values(target.root(), "", &mut actual_values);
    let actual = actual_values
        .into_iter()
        .map(|value| (value.group_tag, value.tag_path))
        .collect::<HashSet<_>>();
    let missing = expected
        .into_iter()
        .filter_map(|(group, path)| {
            (!actual.contains(&(group, path.clone())))
                .then(|| format!("{}:{path}", format_group_tag(group)))
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Conversion would lose {} non-empty tag reference(s): {}",
            missing.len(),
            missing.join(", ")
        ))
    }
}

fn reset_tag_to_defaults(tag: &mut TagFile) -> Result<(), String> {
    reset_struct_to_defaults(tag.root_mut(), "")?;
    tag.remove_dependency_list();
    tag.remove_import_info();
    tag.remove_asset_depot_storage();
    Ok(())
}

fn reset_struct_to_defaults(mut value: TagStructMut<'_>, path: &str) -> Result<(), String> {
    let field_count = value.as_ref().fields().count();
    for ordinal in 0..field_count {
        let Some(mut field) = value.field_at_mut(ordinal) else {
            continue;
        };
        let key = clean_field_key(field.as_ref().name());
        let field_path = join_path(
            path,
            if key.is_empty() {
                field.as_ref().type_name()
            } else {
                &key
            },
        );
        match field.as_ref().field_type() {
            TagFieldType::Struct => {
                if let Some(nested) = field.as_struct_mut() {
                    reset_struct_to_defaults(nested, &field_path)?;
                }
            }
            TagFieldType::Block => {
                if let Some(mut block) = field.as_block_mut() {
                    block.clear();
                }
            }
            TagFieldType::Array => {
                if let Some(mut array) = field.as_array_mut() {
                    for index in 0..array.len() {
                        if let Some(element) = array.element_mut(index) {
                            reset_struct_to_defaults(element, &format!("{field_path}[{index}]"))?;
                        }
                    }
                }
            }
            TagFieldType::PageableResource => {
                if field
                    .as_ref()
                    .as_resource()
                    .is_some_and(|resource| !matches!(resource.kind(), TagResourceKind::Null))
                {
                    return Err(format!(
                        "Native template has a non-null pageable resource at {field_path}"
                    ));
                }
            }
            TagFieldType::ApiInterop => {
                field
                    .set(TagFieldData::ApiInterop(ApiInteropData::reset()))
                    .map_err(|error| format!("Could not reset {field_path}: {error:?}"))?;
            }
            _ => {
                let Some(current) = field.as_ref().value() else {
                    continue;
                };
                if let Some(default) = default_field_value(current) {
                    field
                        .set(default)
                        .map_err(|error| format!("Could not reset {field_path}: {error:?}"))?;
                }
            }
        }
    }
    Ok(())
}

/// A newly allocated schema struct is byte-zeroed, but a block index uses -1
/// as its null value. Native post-processing treats zero as a real index, so
/// leaving a target-only index at the allocator default can make an otherwise
/// valid converted tag assert while loading.
fn initialize_block_index_defaults(mut value: TagStructMut<'_>) {
    let field_count = value.as_ref().fields().count();
    for ordinal in 0..field_count {
        let Some(mut field) = value.field_at_mut(ordinal) else {
            continue;
        };
        match field.as_ref().field_type() {
            TagFieldType::Struct => {
                if let Some(nested) = field.as_struct_mut() {
                    initialize_block_index_defaults(nested);
                }
            }
            TagFieldType::Array => {
                if let Some(mut array) = field.as_array_mut() {
                    for index in 0..array.len() {
                        if let Some(element) = array.element_mut(index) {
                            initialize_block_index_defaults(element);
                        }
                    }
                }
            }
            TagFieldType::CharBlockIndex => {
                let _ = field.set(TagFieldData::CharBlockIndex(-1));
            }
            TagFieldType::CustomCharBlockIndex => {
                let _ = field.set(TagFieldData::CustomCharBlockIndex(-1));
            }
            TagFieldType::ShortBlockIndex => {
                let _ = field.set(TagFieldData::ShortBlockIndex(-1));
            }
            TagFieldType::CustomShortBlockIndex => {
                let _ = field.set(TagFieldData::CustomShortBlockIndex(-1));
            }
            TagFieldType::LongBlockIndex => {
                let _ = field.set(TagFieldData::LongBlockIndex(-1));
            }
            TagFieldType::CustomLongBlockIndex => {
                let _ = field.set(TagFieldData::CustomLongBlockIndex(-1));
            }
            _ => {}
        }
    }
}

fn default_field_value(value: TagFieldData) -> Option<TagFieldData> {
    Some(match value {
        TagFieldData::String(_) => TagFieldData::String(String::new()),
        TagFieldData::LongString(_) => TagFieldData::LongString(String::new()),
        TagFieldData::StringId(_) => TagFieldData::StringId(StringIdData {
            string: String::new(),
        }),
        TagFieldData::OldStringId(_) => TagFieldData::OldStringId(StringIdData {
            string: String::new(),
        }),
        TagFieldData::TagReference(_) => TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: None,
        }),
        TagFieldData::Data(_) => TagFieldData::Data(Vec::new()),
        TagFieldData::ApiInterop(_) => TagFieldData::ApiInterop(ApiInteropData::reset()),
        TagFieldData::CharInteger(_) => TagFieldData::CharInteger(0),
        TagFieldData::ShortInteger(_) => TagFieldData::ShortInteger(0),
        TagFieldData::LongInteger(_) => TagFieldData::LongInteger(0),
        TagFieldData::Int64Integer(_) => TagFieldData::Int64Integer(0),
        TagFieldData::ByteInteger(_) => TagFieldData::ByteInteger(0),
        TagFieldData::WordInteger(_) => TagFieldData::WordInteger(0),
        TagFieldData::DwordInteger(_) => TagFieldData::DwordInteger(0),
        TagFieldData::QwordInteger(_) => TagFieldData::QwordInteger(0),
        TagFieldData::Tag(_) => TagFieldData::Tag(0),
        TagFieldData::CharEnum { .. } => TagFieldData::CharEnum {
            value: 0,
            name: None,
        },
        TagFieldData::ShortEnum { .. } => TagFieldData::ShortEnum {
            value: 0,
            name: None,
        },
        TagFieldData::LongEnum { .. } => TagFieldData::LongEnum {
            value: 0,
            name: None,
        },
        TagFieldData::ByteFlags { .. } => TagFieldData::ByteFlags {
            value: 0,
            names: Vec::new(),
        },
        TagFieldData::WordFlags { .. } => TagFieldData::WordFlags {
            value: 0,
            names: Vec::new(),
        },
        TagFieldData::LongFlags { .. } => TagFieldData::LongFlags {
            value: 0,
            names: Vec::new(),
        },
        TagFieldData::ByteBlockFlags(_) => TagFieldData::ByteBlockFlags(0),
        TagFieldData::WordBlockFlags(_) => TagFieldData::WordBlockFlags(0),
        TagFieldData::LongBlockFlags(_) => TagFieldData::LongBlockFlags(0),
        TagFieldData::CharBlockIndex(_) => TagFieldData::CharBlockIndex(-1),
        TagFieldData::CustomCharBlockIndex(_) => TagFieldData::CustomCharBlockIndex(-1),
        TagFieldData::ShortBlockIndex(_) => TagFieldData::ShortBlockIndex(-1),
        TagFieldData::CustomShortBlockIndex(_) => TagFieldData::CustomShortBlockIndex(-1),
        TagFieldData::LongBlockIndex(_) => TagFieldData::LongBlockIndex(-1),
        TagFieldData::CustomLongBlockIndex(_) => TagFieldData::CustomLongBlockIndex(-1),
        TagFieldData::Angle(_) => TagFieldData::Angle(0.0),
        TagFieldData::Real(_) => TagFieldData::Real(0.0),
        TagFieldData::RealSlider(_) => TagFieldData::RealSlider(0.0),
        TagFieldData::RealFraction(_) => TagFieldData::RealFraction(0.0),
        TagFieldData::Point2d(_) => TagFieldData::Point2d(Default::default()),
        TagFieldData::Rectangle2d(_) => TagFieldData::Rectangle2d(Default::default()),
        TagFieldData::RealPoint2d(_) => TagFieldData::RealPoint2d(Default::default()),
        TagFieldData::RealPoint3d(_) => TagFieldData::RealPoint3d(Default::default()),
        TagFieldData::RealVector2d(_) => TagFieldData::RealVector2d(Default::default()),
        TagFieldData::RealVector3d(_) => TagFieldData::RealVector3d(Default::default()),
        TagFieldData::RealQuaternion(_) => TagFieldData::RealQuaternion(Default::default()),
        TagFieldData::RealEulerAngles2d(_) => TagFieldData::RealEulerAngles2d(Default::default()),
        TagFieldData::RealEulerAngles3d(_) => TagFieldData::RealEulerAngles3d(Default::default()),
        TagFieldData::RealPlane2d(_) => TagFieldData::RealPlane2d(Default::default()),
        TagFieldData::RealPlane3d(_) => TagFieldData::RealPlane3d(Default::default()),
        TagFieldData::RgbColor(_) => TagFieldData::RgbColor(Default::default()),
        TagFieldData::ArgbColor(_) => TagFieldData::ArgbColor(Default::default()),
        TagFieldData::RealRgbColor(_) => TagFieldData::RealRgbColor(Default::default()),
        TagFieldData::RealArgbColor(_) => TagFieldData::RealArgbColor(Default::default()),
        TagFieldData::RealHsvColor(_) => TagFieldData::RealHsvColor(Default::default()),
        TagFieldData::RealAhsvColor(_) => TagFieldData::RealAhsvColor(Default::default()),
        TagFieldData::ShortIntegerBounds(_) => TagFieldData::ShortIntegerBounds(Default::default()),
        TagFieldData::AngleBounds(_) => TagFieldData::AngleBounds(Default::default()),
        TagFieldData::RealBounds(_) => TagFieldData::RealBounds(Default::default()),
        TagFieldData::FractionBounds(_) => TagFieldData::FractionBounds(Default::default()),
        TagFieldData::Custom(bytes) => TagFieldData::Custom(vec![0; bytes.len()]),
    })
}

fn convert_struct(
    source: TagStruct<'_>,
    mut target: TagStructMut<'_>,
    path: &str,
    root: bool,
    context: &mut ConversionContext<'_>,
) {
    let source_guid = source.definition().guid();
    let target_guid = target.as_ref().definition().guid();
    let same_guid = source_guid == target_guid;
    let mut reparented_fields = if context.group_name == "model_animation_graph"
        && path.contains("animations")
        && source
            .fields()
            .all(|field| !clean_field_key(field.name()).starts_with("shared animation data"))
    {
        convert_local_animation_payload(source, &mut target, path, context)
    } else {
        HashSet::new()
    };
    if root && context.group_name.eq_ignore_ascii_case("weapon") {
        reparented_fields.extend(convert_weapon_melee_layout(source, &mut target, context));
    }
    if root && context.group_name.eq_ignore_ascii_case("effect") {
        reparented_fields.extend(convert_effect_looping_sound_layout(
            source,
            &mut target,
            context,
        ));
    }
    if root && context.group_name.eq_ignore_ascii_case("damage_effect") {
        match convert_h3_player_responses_to_reach_companions(source, &mut target, context) {
            Ok(fields) => reparented_fields.extend(fields),
            Err(error) => context.fatal_error = Some(error),
        }
    }
    let target_fields = target
        .as_ref()
        .fields()
        .enumerate()
        .map(|(ordinal, field)| TargetFieldInfo {
            ordinal,
            name: field.name().to_owned(),
            key: clean_field_key(field.name()),
            field_type: field.field_type(),
        })
        .collect::<Vec<_>>();
    let mut used = vec![false; target_fields.len()];

    for source_field in source.fields() {
        let key = clean_field_key(source_field.name());
        let matched = target_fields.iter().enumerate().find(|(index, candidate)| {
            !used[*index]
                && (field_names_match(source_field.name(), &candidate.name)
                    || context
                        .target_field_aliases
                        .matches(target_guid, &candidate.key, &key)
                    || context
                        .source_field_aliases
                        .matches(source_guid, &key, &candidate.key)
                    || context
                        .mapping_catalog
                        .field_names_match(FieldMappingRequest {
                            group: context.group_name,
                            source_game: context.source_game,
                            target_game: context.target_game,
                            source_guid,
                            target_guid,
                            source_name: &key,
                            target_name: &candidate.key,
                        }))
                && (compatible_field_shapes(source_field.field_type(), candidate.field_type)
                    || compatible_semantic_field(
                        context.group_name,
                        &key,
                        source_field.field_type(),
                        candidate.field_type,
                    ))
                && (!key.is_empty() || same_guid)
        });
        let field_path = join_path(
            path,
            if key.is_empty() {
                source_field.type_name()
            } else {
                &key
            },
        );
        let Some((target_index, target_info)) = matched else {
            if !reparented_fields.contains(&key) {
                record_unmatched_field_values(source_field, &field_path, context);
            }
            continue;
        };
        used[target_index] = true;
        if key != target_info.key {
            context.report.mapped_aliases += 1;
        }
        if root {
            context.root_matches += 1;
        }
        let Some(target_field) = target.field_at_mut(target_info.ordinal) else {
            continue;
        };
        convert_field(source_field, target_field, &field_path, same_guid, context);
    }

    let defaulted = used
        .iter()
        .zip(&target_fields)
        .filter(|(used, field)| !**used && is_reportable_target_default(field.field_type))
        .count();
    context.report.defaulted_target += defaulted;
}

fn convert_weapon_melee_layout(
    source: TagStruct<'_>,
    target: &mut TagStructMut<'_>,
    context: &mut ConversionContext<'_>,
) -> HashSet<String> {
    const LEGACY_GAMES: &[&str] = &["halo3_mcc", "halo3odst_mcc"];
    const BLOCK_GAMES: &[&str] = &["haloreach_mcc", "halo4_mcc", "halo2amp_mcc"];
    let legacy_to_block =
        LEGACY_GAMES.contains(&context.source_game) && BLOCK_GAMES.contains(&context.target_game);
    let block_to_legacy =
        BLOCK_GAMES.contains(&context.source_game) && LEGACY_GAMES.contains(&context.target_game);
    if !legacy_to_block && !block_to_legacy {
        return HashSet::new();
    }

    if legacy_to_block {
        convert_legacy_melee_to_block(source, target, context)
    } else {
        convert_block_melee_to_legacy(source, target, context)
    }
}

type ReferencePair = (Option<(u32, String)>, Option<(u32, String)>);

fn field_ordinal_by_key(structure: TagStruct<'_>, key: &str) -> Option<usize> {
    structure
        .fields()
        .enumerate()
        .find(|(_, field)| clean_field_key(field.name()) == clean_field_key(key))
        .map(|(ordinal, _)| ordinal)
}

fn struct_field_by_key<'a>(structure: TagStruct<'a>, key: &str) -> Option<TagStruct<'a>> {
    structure
        .fields()
        .find(|field| clean_field_key(field.name()) == clean_field_key(key))?
        .as_struct()
}

fn reference_by_key(structure: TagStruct<'_>, key: &str) -> Option<(u32, String)> {
    let field = structure
        .fields()
        .find(|field| clean_field_key(field.name()) == clean_field_key(key))?;
    let TagFieldData::TagReference(reference) = field.value()? else {
        return None;
    };
    reference
        .group_tag_and_name
        .filter(|(_, path)| !path.is_empty() && !path.eq_ignore_ascii_case("none"))
}

fn push_unique_reference_pair(pairs: &mut Vec<ReferencePair>, pair: ReferencePair) {
    if (pair.0.is_some() || pair.1.is_some()) && !pairs.contains(&pair) {
        pairs.push(pair);
    }
}

fn set_mapped_reference(
    target: &mut TagStructMut<'_>,
    target_key: &str,
    value: Option<(u32, String)>,
    path: &str,
    context: &mut ConversionContext<'_>,
) {
    let Some((source_group, name)) = value else {
        return;
    };
    let Some(group_name) = context.source_groups.by_tag.get(&source_group) else {
        record_unsupported(
            context,
            path.to_owned(),
            format!(
                "Source reference group {} is unknown",
                format_group_tag(source_group)
            ),
        );
        return;
    };
    let Some(target_group) = context
        .target_groups
        .by_name
        .get(&group_name.to_ascii_lowercase())
        .copied()
    else {
        record_unsupported(
            context,
            path.to_owned(),
            format!("Target profile has no {group_name} reference group"),
        );
        return;
    };
    let Some(ordinal) = field_ordinal_by_key(target.as_ref(), target_key) else {
        record_unsupported(
            context,
            path.to_owned(),
            format!("Target melee layout has no {target_key} field"),
        );
        return;
    };
    let Some(mut field) = target.field_at_mut(ordinal) else {
        return;
    };
    set_converted(
        &mut field,
        TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((target_group, name)),
        }),
        path,
        source_group == target_group,
        context,
    );
}

fn convert_legacy_melee_to_block(
    source: TagStruct<'_>,
    target: &mut TagStructMut<'_>,
    context: &mut ConversionContext<'_>,
) -> HashSet<String> {
    let Some(source_melee) = struct_field_by_key(source, "melee damage parameters") else {
        return HashSet::new();
    };
    let Some(target_ordinal) = field_ordinal_by_key(target.as_ref(), "melee damage parameters")
    else {
        return HashSet::new();
    };
    let mut pairs = Vec::new();
    push_unique_reference_pair(
        &mut pairs,
        (
            reference_by_key(source, "player melee damage"),
            reference_by_key(source, "player melee response"),
        ),
    );
    for prefix in ["1st hit", "2nd hit", "3rd hit"] {
        push_unique_reference_pair(
            &mut pairs,
            (
                reference_by_key(source_melee, &format!("{prefix} melee damage")),
                reference_by_key(source_melee, &format!("{prefix} melee response")),
            ),
        );
    }
    let unique_pair_count = pairs.len();
    if pairs.is_empty() && struct_has_meaningful_value(source_melee) {
        pairs.push((None, None));
    }

    let Some(mut target_field) = target.field_at_mut(target_ordinal) else {
        return HashSet::new();
    };
    let Some(mut target_block) = target_field.as_block_mut() else {
        return HashSet::new();
    };
    target_block.clear();
    let maximum = target_block.definition().max_count() as usize;
    let count = pairs.len().min(maximum);
    for (index, pair) in pairs.iter().take(count).cloned().enumerate() {
        let target_index = target_block.add_element();
        if let Some(element) = target_block.element_mut(target_index) {
            initialize_block_index_defaults(element);
        }
        if let Some(element) = target_block.element_mut(target_index) {
            convert_struct(
                source_melee,
                element,
                &format!("melee damage parameters[{index}]"),
                false,
                context,
            );
        }
        let mut removed_unsupported = 0;
        context.report.issues.retain(|issue| {
            let transferred_hit = issue
                .path
                .starts_with(&format!("melee damage parameters[{index}]"))
                && ["1st hit melee", "2nd hit melee", "3rd hit melee"]
                    .iter()
                    .any(|name| issue.path.contains(name));
            if transferred_hit && issue.kind == ConversionIssueKind::Unsupported {
                removed_unsupported += 1;
            }
            !transferred_hit
        });
        context.report.unsupported_source = context
            .report
            .unsupported_source
            .saturating_sub(removed_unsupported);
        let Some(mut element) = target_block.element_mut(target_index) else {
            continue;
        };
        set_mapped_reference(
            &mut element,
            "melee damage",
            pair.0,
            &format!("melee damage parameters[{index}]/melee damage"),
            context,
        );
        set_mapped_reference(
            &mut element,
            "melee response",
            pair.1,
            &format!("melee damage parameters[{index}]/melee response"),
            context,
        );
    }
    if unique_pair_count > maximum {
        let omitted = unique_pair_count - maximum;
        context.report.truncated += omitted;
        context.report.issues.push(ConversionIssue {
            kind: ConversionIssueKind::Truncated,
            path: "melee damage parameters".to_owned(),
            message: format!("Target melee block limit omitted {omitted} unique damage pair(s)"),
        });
    }
    HashSet::from([
        "player melee damage".to_owned(),
        "player melee response".to_owned(),
        "melee damage parameters".to_owned(),
    ])
}

fn convert_block_melee_to_legacy(
    source: TagStruct<'_>,
    target: &mut TagStructMut<'_>,
    context: &mut ConversionContext<'_>,
) -> HashSet<String> {
    let Some(source_field) = source
        .fields()
        .find(|field| clean_field_key(field.name()) == "melee damage parameters")
    else {
        return HashSet::new();
    };
    let Some(source_block) = source_field.as_block() else {
        return HashSet::new();
    };
    let pairs = source_block
        .iter()
        .map(|element| {
            (
                reference_by_key(element, "melee damage"),
                reference_by_key(element, "melee response"),
            )
        })
        .collect::<Vec<_>>();
    let Some(target_ordinal) = field_ordinal_by_key(target.as_ref(), "melee damage parameters")
    else {
        return HashSet::new();
    };
    if let Some(first) = source_block.element(0) {
        let Some(mut target_field) = target.field_at_mut(target_ordinal) else {
            return HashSet::new();
        };
        let Some(target_melee) = target_field.as_struct_mut() else {
            return HashSet::new();
        };
        convert_struct(
            first,
            target_melee,
            "melee damage parameters",
            false,
            context,
        );
    }
    for (index, pair) in pairs.into_iter().take(3).enumerate() {
        let Some(mut target_field) = target.field_at_mut(target_ordinal) else {
            continue;
        };
        let Some(mut target_melee) = target_field.as_struct_mut() else {
            continue;
        };
        let prefix = ["1st hit", "2nd hit", "3rd hit"][index];
        set_mapped_reference(
            &mut target_melee,
            &format!("{prefix} melee damage"),
            pair.0,
            &format!("melee damage parameters/{prefix} melee damage"),
            context,
        );
        set_mapped_reference(
            &mut target_melee,
            &format!("{prefix} melee response"),
            pair.1,
            &format!("melee damage parameters/{prefix} melee response"),
            context,
        );
    }
    HashSet::from(["melee damage parameters".to_owned()])
}

fn field_by_key<'a>(structure: TagStruct<'a>, key: &str) -> Option<TagField<'a>> {
    structure
        .fields()
        .find(|field| clean_field_key(field.name()) == clean_field_key(key))
}

fn convert_effect_looping_sound_layout(
    source: TagStruct<'_>,
    target: &mut TagStructMut<'_>,
    context: &mut ConversionContext<'_>,
) -> HashSet<String> {
    const LEGACY_GAMES: &[&str] = &["halo3_mcc", "halo3odst_mcc"];
    const BLOCK_GAMES: &[&str] = &["haloreach_mcc", "halo4_mcc", "halo2amp_mcc"];
    if LEGACY_GAMES.contains(&context.source_game) && BLOCK_GAMES.contains(&context.target_game) {
        convert_legacy_effect_looping_sound_to_block(source, target, context)
    } else if BLOCK_GAMES.contains(&context.source_game)
        && LEGACY_GAMES.contains(&context.target_game)
    {
        convert_effect_looping_sound_block_to_legacy(source, target, context)
    } else {
        HashSet::new()
    }
}

fn convert_legacy_effect_looping_sound_to_block(
    source: TagStruct<'_>,
    target: &mut TagStructMut<'_>,
    context: &mut ConversionContext<'_>,
) -> HashSet<String> {
    let Some(source_sound) = field_by_key(source, "looping sound") else {
        return HashSet::new();
    };
    let Some(target_ordinal) = field_ordinal_by_key(target.as_ref(), "looping sounds") else {
        return HashSet::new();
    };
    let has_sound = matches!(
        source_sound.value(),
        Some(TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((_, ref path)),
        })) if !path.is_empty() && !path.eq_ignore_ascii_case("none")
    );
    let Some(mut target_field) = target.field_at_mut(target_ordinal) else {
        return HashSet::new();
    };
    let Some(mut target_block) = target_field.as_block_mut() else {
        return HashSet::new();
    };
    target_block.clear();
    if has_sound {
        let index = target_block.add_element();
        if let Some(element) = target_block.element_mut(index) {
            initialize_block_index_defaults(element);
        }
        if let Some(mut element) = target_block.element_mut(index) {
            for key in ["looping sound", "location", "bind scale to event"] {
                let (Some(source_field), Some(target_ordinal)) = (
                    field_by_key(source, key),
                    field_ordinal_by_key(element.as_ref(), key),
                ) else {
                    continue;
                };
                if let Some(target_field) = element.field_at_mut(target_ordinal) {
                    convert_field(
                        source_field,
                        target_field,
                        &format!("looping sounds[0]/{key}"),
                        false,
                        context,
                    );
                }
            }
        }
    }
    HashSet::from([
        "looping sound".to_owned(),
        "location".to_owned(),
        "bind scale to event".to_owned(),
    ])
}

fn convert_effect_looping_sound_block_to_legacy(
    source: TagStruct<'_>,
    target: &mut TagStructMut<'_>,
    context: &mut ConversionContext<'_>,
) -> HashSet<String> {
    let Some(source_block) =
        field_by_key(source, "looping sounds").and_then(|field| field.as_block())
    else {
        return HashSet::new();
    };
    if let Some(element) = source_block.element(0) {
        for key in ["looping sound", "location", "bind scale to event"] {
            let (Some(source_field), Some(target_ordinal)) = (
                field_by_key(element, key),
                field_ordinal_by_key(target.as_ref(), key),
            ) else {
                continue;
            };
            if let Some(target_field) = target.field_at_mut(target_ordinal) {
                convert_field(source_field, target_field, key, false, context);
            }
        }
    }
    if source_block.len() > 1 {
        record_unsupported(
            context,
            "looping sounds".to_owned(),
            format!(
                "Legacy target supports one looping sound but source has {}",
                source_block.len()
            ),
        );
    }
    HashSet::from(["looping sounds".to_owned()])
}

/// Reach-family animation entries moved the H3 inline payload into a
/// single-element `shared animation data` block. This is a structural move,
/// not a rename, so map the compatible source fields into that nested element
/// without reporting the already-copied entry metadata as unmatched.
fn convert_local_animation_payload(
    source: TagStruct<'_>,
    target: &mut TagStructMut<'_>,
    path: &str,
    context: &mut ConversionContext<'_>,
) -> HashSet<String> {
    let mut transferred = HashSet::new();
    let payload_ordinal = target
        .as_ref()
        .fields()
        .enumerate()
        .find(|(_, field)| {
            field.field_type() == TagFieldType::Block
                && clean_field_key(field.name()).starts_with("shared animation data")
        })
        .map(|(ordinal, _)| ordinal);
    let Some(payload_ordinal) = payload_ordinal else {
        return transferred;
    };
    let Some(mut payload_field) = target.field_at_mut(payload_ordinal) else {
        return transferred;
    };
    let Some(mut payload_block) = payload_field.as_block_mut() else {
        return transferred;
    };
    payload_block.clear();
    let payload_index = payload_block.add_element();
    let Some(payload) = payload_block.element_mut(payload_index) else {
        return transferred;
    };
    initialize_block_index_defaults(payload);
    let Some(mut payload) = payload_block.element_mut(payload_index) else {
        return transferred;
    };
    let target_guid = payload.as_ref().definition().guid();
    let target_fields = payload
        .as_ref()
        .fields()
        .enumerate()
        .map(|(ordinal, field)| TargetFieldInfo {
            ordinal,
            name: field.name().to_owned(),
            key: clean_field_key(field.name()),
            field_type: field.field_type(),
        })
        .collect::<Vec<_>>();
    let mut used = vec![false; target_fields.len()];
    let source_guid = source.definition().guid();
    for source_field in source.fields() {
        let key = clean_field_key(source_field.name());
        let matched = target_fields.iter().enumerate().find(|(index, candidate)| {
            !used[*index]
                && (field_names_match(source_field.name(), &candidate.name)
                    || context
                        .target_field_aliases
                        .matches(target_guid, &candidate.key, &key)
                    || context
                        .source_field_aliases
                        .matches(source_guid, &key, &candidate.key)
                    || context
                        .mapping_catalog
                        .field_names_match(FieldMappingRequest {
                            group: context.group_name,
                            source_game: context.source_game,
                            target_game: context.target_game,
                            source_guid,
                            target_guid,
                            source_name: &key,
                            target_name: &candidate.key,
                        }))
                && compatible_field_shapes(source_field.field_type(), candidate.field_type)
        });
        let Some((target_index, target_info)) = matched else {
            continue;
        };
        used[target_index] = true;
        transferred.insert(key.clone());
        if key != target_info.key {
            context.report.mapped_aliases += 1;
        }
        if let Some(target_field) = payload.field_at_mut(target_info.ordinal) {
            convert_field(
                source_field,
                target_field,
                &join_path(
                    path,
                    &format!("shared animation data[0]/{}", target_info.key),
                ),
                source_guid == target_guid,
                context,
            );
        }
    }
    transferred
}

fn is_reportable_target_default(field_type: TagFieldType) -> bool {
    !matches!(
        field_type,
        TagFieldType::Terminator
            | TagFieldType::Explanation
            | TagFieldType::Pad
            | TagFieldType::UselessPad
            | TagFieldType::Skip
            | TagFieldType::Custom
            | TagFieldType::ApiInterop
            | TagFieldType::PageableResource
    )
}

fn record_unmatched_field_values(
    field: TagField<'_>,
    path: &str,
    context: &mut ConversionContext<'_>,
) {
    if !field_has_meaningful_value(field) {
        return;
    }
    if field.name().contains('!') || clean_field_key(field.name()).starts_with("runtime ") {
        context.report.issues.push(ConversionIssue {
            kind: ConversionIssueKind::Warning,
            path: path.to_owned(),
            message: "Engine-managed source value was reset for the target engine".to_owned(),
        });
        return;
    }
    match field.field_type() {
        TagFieldType::Struct => {
            if let Some(structure) = field.as_struct() {
                for child in structure.fields() {
                    let key = clean_field_key(child.name());
                    let child_path = join_path(
                        path,
                        if key.is_empty() {
                            child.type_name()
                        } else {
                            &key
                        },
                    );
                    record_unmatched_field_values(child, &child_path, context);
                }
            }
        }
        TagFieldType::Block => {
            if let Some(block) = field.as_block() {
                for (index, element) in block.iter().enumerate() {
                    for child in element.fields() {
                        let key = clean_field_key(child.name());
                        let child_path = join_path(
                            &format!("{path}[{index}]"),
                            if key.is_empty() {
                                child.type_name()
                            } else {
                                &key
                            },
                        );
                        record_unmatched_field_values(child, &child_path, context);
                    }
                }
            }
        }
        TagFieldType::Array => {
            if let Some(array) = field.as_array() {
                for (index, element) in array.iter().enumerate() {
                    for child in element.fields() {
                        let key = clean_field_key(child.name());
                        let child_path = join_path(
                            &format!("{path}[{index}]"),
                            if key.is_empty() {
                                child.type_name()
                            } else {
                                &key
                            },
                        );
                        record_unmatched_field_values(child, &child_path, context);
                    }
                }
            }
        }
        _ => record_unsupported(
            context,
            path.to_owned(),
            format!("No compatible target field for {}", field.type_name()),
        ),
    }
}

fn field_names_match(left: &str, right: &str) -> bool {
    let left_key = clean_field_key(left);
    let right_key = clean_field_key(right);
    if left_key == right_key {
        return true;
    }
    if left_key.is_empty() || right_key.is_empty() {
        return false;
    }
    // `|ABCDCC` and similar suffixes are editor presentation/order metadata,
    // not alternate field names. Including them made unrelated blocks with
    // the same suffix appear compatible.
    let aliases =
        |name: &str| option_name_aliases(name.split(['#', ':', '|']).next().unwrap_or(name));
    let left = aliases(left);
    let right = aliases(right);
    left.iter()
        .any(|left| right.iter().any(|right| left == right))
}

fn compatible_field_shapes(source: TagFieldType, target: TagFieldType) -> bool {
    source == target
        || (is_integer_type(source) && is_integer_type(target))
        || (is_real_scalar(source) && is_real_scalar(target))
        || (is_enum_type(source) && is_enum_type(target))
        || (is_flags_type(source) && is_flags_type(target))
        || (is_string_type(source) && is_string_type(target))
        || (is_string_id_type(source) && is_string_id_type(target))
}

fn compatible_semantic_field(
    group: &str,
    field: &str,
    source: TagFieldType,
    target: TagFieldType,
) -> bool {
    group.eq_ignore_ascii_case("lens_flare")
        && clean_field_key(field) == "occlusion inner radius scale"
        && ((is_enum_type(source) && target == TagFieldType::Real)
            || (source == TagFieldType::Real && is_enum_type(target)))
}

fn convert_lens_flare_occlusion_scale(
    source: TagField<'_>,
    target: &mut TagFieldMut<'_>,
    path: &str,
    context: &mut ConversionContext<'_>,
) -> bool {
    if !context.group_name.eq_ignore_ascii_case("lens_flare")
        || clean_field_key(path.split('/').next_back().unwrap_or(path))
            != "occlusion inner radius scale"
    {
        return false;
    }

    const SCALES: [(&str, f32); 7] = [
        ("none", 0.0),
        ("1/2", 0.5),
        ("1/4", 0.25),
        ("1/8", 0.125),
        ("1/16", 0.0625),
        ("1/32", 0.03125),
        ("1/64", 0.015625),
    ];
    if is_enum_type(source.field_type()) && target.as_ref().field_type() == TagFieldType::Real {
        let name = match source.value() {
            Some(TagFieldData::CharEnum { name, .. })
            | Some(TagFieldData::ShortEnum { name, .. })
            | Some(TagFieldData::LongEnum { name, .. }) => name,
            _ => None,
        };
        let Some((_, scale)) = name.as_deref().and_then(|name| {
            SCALES
                .iter()
                .find(|(candidate, _)| option_names_match(candidate, name))
        }) else {
            if field_has_meaningful_value(source) {
                record_unsupported(
                    context,
                    path.to_owned(),
                    "Unresolved lens-flare occlusion scale enum".to_owned(),
                );
            }
            return true;
        };
        set_converted(target, TagFieldData::Real(*scale), path, false, context);
        return true;
    }

    if source.field_type() == TagFieldType::Real && is_enum_type(target.as_ref().field_type()) {
        let Some(TagFieldData::Real(scale)) = source.value() else {
            return true;
        };
        let Some((index, (name, _))) = SCALES
            .iter()
            .enumerate()
            .find(|(_, (_, candidate))| (scale - *candidate).abs() <= f32::EPSILON)
        else {
            if scale != 0.0 {
                record_unsupported(
                    context,
                    path.to_owned(),
                    format!("Legacy lens-flare schema cannot represent occlusion scale {scale}"),
                );
            }
            return true;
        };
        let value = match target.as_ref().field_type() {
            TagFieldType::CharEnum => TagFieldData::CharEnum {
                value: index as i8,
                name: Some((*name).to_owned()),
            },
            TagFieldType::ShortEnum => TagFieldData::ShortEnum {
                value: index as i16,
                name: Some((*name).to_owned()),
            },
            TagFieldType::LongEnum => TagFieldData::LongEnum {
                value: index as i32,
                name: Some((*name).to_owned()),
            },
            _ => return false,
        };
        set_converted(target, value, path, false, context);
        return true;
    }
    false
}

fn convert_field(
    source: TagField<'_>,
    mut target: TagFieldMut<'_>,
    path: &str,
    same_struct_guid: bool,
    context: &mut ConversionContext<'_>,
) {
    let source_type = source.field_type();
    let target_type = target.as_ref().field_type();
    if convert_lens_flare_occlusion_scale(source, &mut target, path, context) {
        return;
    }
    match source_type {
        TagFieldType::Struct => {
            let (Some(source_struct), Some(target_struct)) =
                (source.as_struct(), target.as_struct_mut())
            else {
                record_unsupported(
                    context,
                    path.to_owned(),
                    "Missing nested struct data".to_owned(),
                );
                return;
            };
            convert_struct(source_struct, target_struct, path, false, context);
        }
        TagFieldType::Block => {
            let (Some(source_block), Some(mut target_block)) =
                (source.as_block(), target.as_block_mut())
            else {
                record_unsupported(
                    context,
                    path.to_owned(),
                    "Missing tag block data".to_owned(),
                );
                return;
            };
            target_block.clear();
            let maximum = target_block.definition().max_count() as usize;
            let count = source_block.len().min(maximum);
            for index in 0..count {
                let target_index = target_block.add_element();
                if let Some(target_element) = target_block.element_mut(target_index) {
                    initialize_block_index_defaults(target_element);
                }
                if let (Some(source_element), Some(target_element)) = (
                    source_block.element(index),
                    target_block.element_mut(target_index),
                ) {
                    convert_struct(
                        source_element,
                        target_element,
                        &format!("{path}[{index}]"),
                        false,
                        context,
                    );
                }
            }
            if source_block.len() > count {
                let omitted = source_block.len() - count;
                context.report.truncated += omitted;
                context.report.issues.push(ConversionIssue {
                    kind: ConversionIssueKind::Truncated,
                    path: path.to_owned(),
                    message: format!("Target block limit omitted {omitted} element(s)"),
                });
            }
        }
        TagFieldType::Array => {
            let (Some(source_array), Some(mut target_array)) =
                (source.as_array(), target.as_array_mut())
            else {
                record_unsupported(
                    context,
                    path.to_owned(),
                    "Missing fixed-array data".to_owned(),
                );
                return;
            };
            let count = source_array.len().min(target_array.len());
            for index in 0..count {
                if let (Some(source_element), Some(target_element)) =
                    (source_array.element(index), target_array.element_mut(index))
                {
                    convert_struct(
                        source_element,
                        target_element,
                        &format!("{path}[{index}]"),
                        false,
                        context,
                    );
                }
            }
            if source_array.len() > count {
                let omitted = source_array.len() - count;
                context.report.truncated += omitted;
                context.report.issues.push(ConversionIssue {
                    kind: ConversionIssueKind::Truncated,
                    path: path.to_owned(),
                    message: format!("Target array omitted {omitted} element(s)"),
                });
            }
        }
        TagFieldType::PageableResource => {
            if source
                .as_resource()
                .is_some_and(|resource| !matches!(resource.kind(), TagResourceKind::Null))
            {
                record_unsupported(
                    context,
                    path.to_owned(),
                    "Non-null pageable resources are engine-specific".to_owned(),
                );
            }
        }
        TagFieldType::ApiInterop => {
            if field_has_meaningful_value(source) {
                record_unsupported(
                    context,
                    path.to_owned(),
                    "API interop runtime data is not transferred".to_owned(),
                );
            }
        }
        TagFieldType::TagReference => convert_reference(source, target, path, context),
        TagFieldType::CharEnum | TagFieldType::ShortEnum | TagFieldType::LongEnum => {
            convert_enum(source, target, path, context)
        }
        TagFieldType::ByteFlags | TagFieldType::WordFlags | TagFieldType::LongFlags => {
            convert_flags(source, target, path, context)
        }
        TagFieldType::StringId | TagFieldType::OldStringId => {
            let Some(value) = source.value() else { return };
            let string = match value {
                TagFieldData::StringId(value) | TagFieldData::OldStringId(value) => value.string,
                _ => return,
            };
            let value = if target_type == TagFieldType::StringId {
                TagFieldData::StringId(StringIdData { string })
            } else {
                TagFieldData::OldStringId(StringIdData { string })
            };
            set_converted(
                &mut target,
                value,
                path,
                source_type == target_type,
                context,
            );
        }
        TagFieldType::String | TagFieldType::LongString => {
            let Some(value) = source.value() else { return };
            let string = match value {
                TagFieldData::String(value) | TagFieldData::LongString(value) => value,
                _ => return,
            };
            let limit = if target_type == TagFieldType::String {
                31
            } else {
                255
            };
            if string.len() > limit {
                record_unsupported(
                    context,
                    path.to_owned(),
                    format!(
                        "String is {} bytes but target limit is {limit}",
                        string.len()
                    ),
                );
                return;
            }
            let value = if target_type == TagFieldType::String {
                TagFieldData::String(string)
            } else {
                TagFieldData::LongString(string)
            };
            set_converted(
                &mut target,
                value,
                path,
                source_type == target_type,
                context,
            );
        }
        TagFieldType::Data | TagFieldType::Custom => {
            if !same_struct_guid || source_type != target_type {
                if field_has_meaningful_value(source) {
                    record_unsupported(
                        context,
                        path.to_owned(),
                        "Opaque bytes require an identical struct GUID and field type".to_owned(),
                    );
                }
                return;
            }
            if let Some(value) = source.value() {
                set_converted(&mut target, value, path, true, context);
            }
        }
        _ if is_integer_type(source_type) && is_integer_type(target_type) => {
            convert_integer(source, target, path, context)
        }
        _ if is_real_scalar(source_type) && is_real_scalar(target_type) => {
            let Some(value) = source.value().and_then(real_value) else {
                return;
            };
            let converted = real_field_value(target_type, value);
            set_converted(
                &mut target,
                converted,
                path,
                source_type == target_type,
                context,
            );
        }
        _ if source_type == target_type => {
            if let Some(value) = source.value() {
                set_converted(&mut target, value, path, true, context);
            }
        }
        _ => {
            if field_has_meaningful_value(source) {
                record_unsupported(
                    context,
                    path.to_owned(),
                    format!("Cannot convert {source_type:?} to {target_type:?}"),
                );
            }
        }
    }
}

fn convert_reference(
    source: TagField<'_>,
    mut target: TagFieldMut<'_>,
    path: &str,
    context: &mut ConversionContext<'_>,
) {
    let Some(TagFieldData::TagReference(reference)) = source.value() else {
        return;
    };
    let Some((source_group, name)) = reference.group_tag_and_name else {
        return;
    };
    let Some(group_name) = context.source_groups.by_tag.get(&source_group) else {
        record_unsupported(
            context,
            path.to_owned(),
            format!(
                "Source reference group {} is unknown",
                format_group_tag(source_group)
            ),
        );
        return;
    };
    let Some(target_group) = context
        .target_groups
        .by_name
        .get(&group_name.to_ascii_lowercase())
        .copied()
    else {
        record_unsupported(
            context,
            path.to_owned(),
            format!("Target profile has no {group_name} reference group"),
        );
        return;
    };
    set_converted(
        &mut target,
        TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((target_group, name)),
        }),
        path,
        source_group == target_group,
        context,
    );
}

fn convert_enum(
    source: TagField<'_>,
    mut target: TagFieldMut<'_>,
    path: &str,
    context: &mut ConversionContext<'_>,
) {
    let source_name = match source.value() {
        Some(TagFieldData::CharEnum { name, .. })
        | Some(TagFieldData::ShortEnum { name, .. })
        | Some(TagFieldData::LongEnum { name, .. }) => name,
        _ => None,
    };
    let Some(source_name) = source_name else {
        if field_has_meaningful_value(source) {
            record_unsupported(
                context,
                path.to_owned(),
                "Unresolved source enum value".to_owned(),
            );
        }
        return;
    };
    let Some(TagOptions::Enum { names, .. }) = target.as_ref().options() else {
        return;
    };
    let Some((index, mapped_by_catalog)) = names.iter().enumerate().find_map(|(index, name)| {
        if option_names_match(name, &source_name) {
            Some((index, false))
        } else if context.mapping_catalog.option_names_match(
            context.group_name,
            path,
            context.source_game,
            context.target_game,
            &source_name,
            name,
        ) {
            Some((index, true))
        } else {
            None
        }
    }) else {
        record_unsupported(
            context,
            path.to_owned(),
            format!("Target enum has no {source_name:?} option"),
        );
        return;
    };
    if mapped_by_catalog {
        context.report.mapped_aliases += 1;
    }
    let value = match target.as_ref().field_type() {
        TagFieldType::CharEnum => TagFieldData::CharEnum {
            value: index as i8,
            name: Some(source_name),
        },
        TagFieldType::ShortEnum => TagFieldData::ShortEnum {
            value: index as i16,
            name: Some(source_name),
        },
        TagFieldType::LongEnum => TagFieldData::LongEnum {
            value: index as i32,
            name: Some(source_name),
        },
        _ => return,
    };
    set_converted(&mut target, value, path, false, context);
}

fn convert_flags(
    source: TagField<'_>,
    mut target: TagFieldMut<'_>,
    path: &str,
    context: &mut ConversionContext<'_>,
) {
    let names = match source.value() {
        Some(TagFieldData::ByteFlags { value, names }) => (value as u64, names),
        Some(TagFieldData::WordFlags { value, names }) => (value as u64, names),
        Some(TagFieldData::LongFlags { value, names }) => (value as u32 as u64, names),
        _ => return,
    };
    if names.0 != 0 && names.1.is_empty() {
        record_unsupported(
            context,
            path.to_owned(),
            "Set source flag bits have no names".to_owned(),
        );
        return;
    }
    let Some(TagOptions::Flags(target_options)) = target.as_ref().options() else {
        return;
    };
    let mut raw = 0u64;
    for (_, source_name) in names.1 {
        let Some((option, mapped_by_catalog)) = target_options.iter().find_map(|option| {
            if option_names_match(&option.name, &source_name) {
                Some((option, false))
            } else if context.mapping_catalog.option_names_match(
                context.group_name,
                path,
                context.source_game,
                context.target_game,
                &source_name,
                &option.name,
            ) {
                Some((option, true))
            } else {
                None
            }
        }) else {
            record_unsupported(
                context,
                path.to_owned(),
                format!("Target flags have no {source_name:?} bit"),
            );
            continue;
        };
        if mapped_by_catalog {
            context.report.mapped_aliases += 1;
        }
        raw |= 1u64 << option.bit;
    }
    let value = match target.as_ref().field_type() {
        TagFieldType::ByteFlags => TagFieldData::ByteFlags {
            value: raw as u8,
            names: Vec::new(),
        },
        TagFieldType::WordFlags => TagFieldData::WordFlags {
            value: raw as u16,
            names: Vec::new(),
        },
        TagFieldType::LongFlags => TagFieldData::LongFlags {
            value: raw as u32 as i32,
            names: Vec::new(),
        },
        _ => return,
    };
    set_converted(&mut target, value, path, false, context);
}

fn option_names_match(left: &str, right: &str) -> bool {
    if left.trim().is_empty() || right.trim().is_empty() {
        return left.trim().is_empty() && right.trim().is_empty();
    }
    let left = option_name_aliases(left);
    let right = option_name_aliases(right);
    left.iter()
        .any(|left| right.iter().any(|right| left == right))
}

fn option_name_aliases(name: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    let mut current = String::new();
    for character in name.chars() {
        match character {
            '{' | '}' | '|' => {
                let normalized = normalize_option_name(&current);
                if !normalized.is_empty() && !aliases.contains(&normalized) {
                    aliases.push(normalized);
                }
                current.clear();
            }
            _ => current.push(character),
        }
    }
    let normalized = normalize_option_name(&current);
    if !normalized.is_empty() && !aliases.contains(&normalized) {
        aliases.push(normalized);
    }
    aliases
}

fn normalize_option_name(name: &str) -> String {
    name.split('#')
        .next()
        .unwrap_or(name)
        .replace(['*', '!', '^'], "")
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn convert_integer(
    source: TagField<'_>,
    mut target: TagFieldMut<'_>,
    path: &str,
    context: &mut ConversionContext<'_>,
) {
    let Some(value) = source.value().and_then(integer_value) else {
        return;
    };
    let target_type = target.as_ref().field_type();
    let Some(converted) = integer_field_value(target_type, value) else {
        record_unsupported(
            context,
            path.to_owned(),
            format!("Value {value} does not fit target {target_type:?}"),
        );
        return;
    };
    set_converted(
        &mut target,
        converted,
        path,
        source.field_type() == target_type,
        context,
    );
}

fn set_converted(
    target: &mut TagFieldMut<'_>,
    value: TagFieldData,
    path: &str,
    exact: bool,
    context: &mut ConversionContext<'_>,
) {
    if let Err(error) = target.set(value) {
        record_unsupported(
            context,
            path.to_owned(),
            format!("Could not assign target value: {error:?}"),
        );
    } else if exact {
        context.report.copied_exact += 1;
    } else {
        context.report.converted_semantic += 1;
    }
}

fn record_unsupported(context: &mut ConversionContext<'_>, path: String, message: String) {
    context.report.unsupported_source += 1;
    context.report.issues.push(ConversionIssue {
        kind: ConversionIssueKind::Unsupported,
        path,
        message,
    });
}

fn join_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}/{child}")
    }
}

fn field_has_meaningful_value(field: TagField<'_>) -> bool {
    match field.field_type() {
        TagFieldType::Struct => field.as_struct().is_some_and(struct_has_meaningful_value),
        TagFieldType::Array => field
            .as_array()
            .is_some_and(|array| array.iter().any(struct_has_meaningful_value)),
        TagFieldType::Block => field.as_block().is_some_and(|block| !block.is_empty()),
        TagFieldType::PageableResource => field
            .as_resource()
            .is_some_and(|resource| !matches!(resource.kind(), TagResourceKind::Null)),
        _ => field.value().is_some_and(value_is_meaningful),
    }
}

fn struct_has_meaningful_value(value: TagStruct<'_>) -> bool {
    value.fields().any(field_has_meaningful_value)
}

fn value_is_meaningful(value: TagFieldData) -> bool {
    match value {
        TagFieldData::String(value) | TagFieldData::LongString(value) => !value.is_empty(),
        TagFieldData::StringId(value) | TagFieldData::OldStringId(value) => {
            !value.string.is_empty()
        }
        TagFieldData::TagReference(value) => value.group_tag_and_name.is_some(),
        TagFieldData::Data(value) | TagFieldData::Custom(value) => {
            value.iter().any(|byte| *byte != 0)
        }
        TagFieldData::CharInteger(value) => value != 0,
        TagFieldData::ShortInteger(value) => value != 0,
        TagFieldData::LongInteger(value) => value != 0,
        TagFieldData::Int64Integer(value) => value != 0,
        TagFieldData::ByteInteger(value) => value != 0,
        TagFieldData::WordInteger(value) => value != 0,
        TagFieldData::DwordInteger(value) | TagFieldData::Tag(value) => value != 0,
        TagFieldData::QwordInteger(value) => value != 0,
        TagFieldData::CharEnum { value, .. } => value != 0,
        TagFieldData::ShortEnum { value, .. } => value != 0,
        TagFieldData::LongEnum { value, .. } => value != 0,
        TagFieldData::ByteFlags { value, .. } | TagFieldData::ByteBlockFlags(value) => value != 0,
        TagFieldData::WordFlags { value, .. } | TagFieldData::WordBlockFlags(value) => value != 0,
        TagFieldData::LongFlags { value, .. } | TagFieldData::LongBlockFlags(value) => value != 0,
        TagFieldData::CharBlockIndex(value) | TagFieldData::CustomCharBlockIndex(value) => {
            value != 0
        }
        TagFieldData::ShortBlockIndex(value) | TagFieldData::CustomShortBlockIndex(value) => {
            value != 0
        }
        TagFieldData::LongBlockIndex(value) | TagFieldData::CustomLongBlockIndex(value) => {
            value != 0
        }
        TagFieldData::Angle(value)
        | TagFieldData::Real(value)
        | TagFieldData::RealSlider(value)
        | TagFieldData::RealFraction(value) => value != 0.0,
        TagFieldData::Point2d(value) => value != Default::default(),
        TagFieldData::Rectangle2d(value) => value != Default::default(),
        TagFieldData::RealPoint2d(value) => value != Default::default(),
        TagFieldData::RealPoint3d(value) => value != Default::default(),
        TagFieldData::RealVector2d(value) => value != Default::default(),
        TagFieldData::RealVector3d(value) => value != Default::default(),
        TagFieldData::RealQuaternion(value) => value != Default::default(),
        TagFieldData::RealEulerAngles2d(value) => value != Default::default(),
        TagFieldData::RealEulerAngles3d(value) => value != Default::default(),
        TagFieldData::RealPlane2d(value) => value != Default::default(),
        TagFieldData::RealPlane3d(value) => value != Default::default(),
        TagFieldData::RgbColor(value) => value != Default::default(),
        TagFieldData::ArgbColor(value) => value != Default::default(),
        TagFieldData::RealRgbColor(value) => value != Default::default(),
        TagFieldData::RealArgbColor(value) => value != Default::default(),
        TagFieldData::RealHsvColor(value) => value != Default::default(),
        TagFieldData::RealAhsvColor(value) => value != Default::default(),
        TagFieldData::ShortIntegerBounds(value) => value != Default::default(),
        TagFieldData::AngleBounds(value) => value != Default::default(),
        TagFieldData::RealBounds(value) => value != Default::default(),
        TagFieldData::FractionBounds(value) => value != Default::default(),
        TagFieldData::ApiInterop(value) => value.raw.iter().any(|byte| *byte != 0),
    }
}

fn is_integer_type(value: TagFieldType) -> bool {
    matches!(
        value,
        TagFieldType::CharInteger
            | TagFieldType::ShortInteger
            | TagFieldType::LongInteger
            | TagFieldType::Int64Integer
            | TagFieldType::ByteInteger
            | TagFieldType::WordInteger
            | TagFieldType::DwordInteger
            | TagFieldType::QwordInteger
            | TagFieldType::CharBlockIndex
            | TagFieldType::CustomCharBlockIndex
            | TagFieldType::ShortBlockIndex
            | TagFieldType::CustomShortBlockIndex
            | TagFieldType::LongBlockIndex
            | TagFieldType::CustomLongBlockIndex
    )
}

fn is_real_scalar(value: TagFieldType) -> bool {
    matches!(
        value,
        TagFieldType::Angle
            | TagFieldType::Real
            | TagFieldType::RealSlider
            | TagFieldType::RealFraction
    )
}

fn is_enum_type(value: TagFieldType) -> bool {
    matches!(
        value,
        TagFieldType::CharEnum | TagFieldType::ShortEnum | TagFieldType::LongEnum
    )
}

fn is_flags_type(value: TagFieldType) -> bool {
    matches!(
        value,
        TagFieldType::ByteFlags | TagFieldType::WordFlags | TagFieldType::LongFlags
    )
}

fn is_string_type(value: TagFieldType) -> bool {
    matches!(value, TagFieldType::String | TagFieldType::LongString)
}

fn is_string_id_type(value: TagFieldType) -> bool {
    matches!(value, TagFieldType::StringId | TagFieldType::OldStringId)
}

fn integer_value(value: TagFieldData) -> Option<i128> {
    match value {
        TagFieldData::CharInteger(value) => Some(value as i128),
        TagFieldData::ShortInteger(value) => Some(value as i128),
        TagFieldData::LongInteger(value) => Some(value as i128),
        TagFieldData::Int64Integer(value) => Some(value as i128),
        TagFieldData::ByteInteger(value) => Some(value as i128),
        TagFieldData::WordInteger(value) => Some(value as i128),
        TagFieldData::DwordInteger(value) => Some(value as i128),
        TagFieldData::QwordInteger(value) => Some(value as i128),
        TagFieldData::CharBlockIndex(value) | TagFieldData::CustomCharBlockIndex(value) => {
            Some(value as i128)
        }
        TagFieldData::ShortBlockIndex(value) | TagFieldData::CustomShortBlockIndex(value) => {
            Some(value as i128)
        }
        TagFieldData::LongBlockIndex(value) | TagFieldData::CustomLongBlockIndex(value) => {
            Some(value as i128)
        }
        _ => None,
    }
}

fn integer_field_value(field_type: TagFieldType, value: i128) -> Option<TagFieldData> {
    Some(match field_type {
        TagFieldType::CharInteger => TagFieldData::CharInteger(i8::try_from(value).ok()?),
        TagFieldType::ShortInteger => TagFieldData::ShortInteger(i16::try_from(value).ok()?),
        TagFieldType::LongInteger => TagFieldData::LongInteger(i32::try_from(value).ok()?),
        TagFieldType::Int64Integer => TagFieldData::Int64Integer(i64::try_from(value).ok()?),
        TagFieldType::ByteInteger => TagFieldData::ByteInteger(u8::try_from(value).ok()?),
        TagFieldType::WordInteger => TagFieldData::WordInteger(u16::try_from(value).ok()?),
        TagFieldType::DwordInteger => TagFieldData::DwordInteger(u32::try_from(value).ok()?),
        TagFieldType::QwordInteger => TagFieldData::QwordInteger(u64::try_from(value).ok()?),
        TagFieldType::CharBlockIndex => TagFieldData::CharBlockIndex(i8::try_from(value).ok()?),
        TagFieldType::CustomCharBlockIndex => {
            TagFieldData::CustomCharBlockIndex(i8::try_from(value).ok()?)
        }
        TagFieldType::ShortBlockIndex => TagFieldData::ShortBlockIndex(i16::try_from(value).ok()?),
        TagFieldType::CustomShortBlockIndex => {
            TagFieldData::CustomShortBlockIndex(i16::try_from(value).ok()?)
        }
        TagFieldType::LongBlockIndex => TagFieldData::LongBlockIndex(i32::try_from(value).ok()?),
        TagFieldType::CustomLongBlockIndex => {
            TagFieldData::CustomLongBlockIndex(i32::try_from(value).ok()?)
        }
        _ => return None,
    })
}

fn real_value(value: TagFieldData) -> Option<f32> {
    match value {
        TagFieldData::Angle(value)
        | TagFieldData::Real(value)
        | TagFieldData::RealSlider(value)
        | TagFieldData::RealFraction(value) => Some(value),
        _ => None,
    }
}

fn real_field_value(field_type: TagFieldType, value: f32) -> TagFieldData {
    match field_type {
        TagFieldType::Angle => TagFieldData::Angle(value),
        TagFieldType::RealSlider => TagFieldData::RealSlider(value),
        TagFieldType::RealFraction => TagFieldData::RealFraction(value),
        _ => TagFieldData::Real(value),
    }
}

impl Baboon {
    pub(super) fn can_convert_current_tag(&self) -> bool {
        if !self.expert_mode {
            return false;
        }
        let Some(key) = self.selected_key.as_deref() else {
            return false;
        };
        let Some(document) = self.parsed_tags.get(key) else {
            return false;
        };
        let Some(game) = self
            .source
            .as_ref()
            .and_then(|source| source.game.as_deref())
        else {
            return false;
        };
        CONVERSION_GAMES.contains(&game)
            && document.tag.classic_engine().is_none()
            && document.tag.endian == Endian::Le
    }

    pub(super) fn open_tag_conversion_dialog(&mut self) {
        if !self.expert_mode {
            self.status = "Cross-game conversion requires Expert mode".to_owned();
            return;
        }
        let Some(key) = self.selected_key.clone() else {
            return;
        };
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let Some(source_game) = source.game.clone() else {
            return;
        };
        let target_game = CONVERSION_GAMES
            .iter()
            .copied()
            .find(|game| *game != source_game)
            .unwrap_or("halo3odst_mcc")
            .to_owned();
        let source_label = self
            .entry_for_key(&key)
            .map(|entry| entry.display_path.clone())
            .unwrap_or_else(|| key.clone());
        self.tag_conversion_dialog = Some(TagConversionDialog {
            source_key: key,
            source_label,
            source_game,
            target_game,
            draft: None,
            error: None,
            pending_source_destination: None,
        });
    }

    pub(super) fn analyze_tag_conversion(&mut self) {
        if !self.expert_mode {
            self.tag_conversion_dialog = None;
            self.status = "Cross-game conversion requires Expert mode".to_owned();
            return;
        }
        let Some(dialog) = self.tag_conversion_dialog.as_ref() else {
            return;
        };
        let key = dialog.source_key.clone();
        let source_game = dialog.source_game.clone();
        let target_game = dialog.target_game.clone();
        let target_tags_root = self
            .editing_kit_paths
            .get(&target_game)
            .map(|root| configured_tags_root(root));
        let result = self
            .parsed_tags
            .get(&key)
            .ok_or_else(|| "The source tag is no longer loaded".to_owned())
            .and_then(|document| {
                analyze_conversion(
                    &document.tag,
                    &source_game,
                    &target_game,
                    &locate_definitions_root(),
                    target_tags_root.as_deref(),
                )
            });
        if let Some(dialog) = self.tag_conversion_dialog.as_mut() {
            dialog.pending_source_destination = None;
            match result {
                Ok(draft) => {
                    dialog.draft = Some(draft);
                    dialog.error = None;
                }
                Err(error) => {
                    dialog.draft = None;
                    dialog.error = Some(error);
                }
            }
        }
    }

    pub(super) fn choose_tag_conversion_destination(&mut self) {
        let Some(dialog) = self.tag_conversion_dialog.as_ref() else {
            return;
        };
        let Some(draft) = dialog.draft.as_ref() else {
            return;
        };
        let target_game = dialog.target_game.clone();
        let extension = draft.target_extension.clone();
        let source_name = self
            .entry_for_key(&dialog.source_key)
            .and_then(|entry| Path::new(&entry.display_path).file_stem())
            .and_then(|name| name.to_str())
            .unwrap_or("converted_tag");
        let mut picker = rfd::FileDialog::new()
            .set_title(format!(
                "Save {} tag for {}",
                draft.target_group_name, target_game
            ))
            .set_file_name(format!("{source_name}.{extension}"))
            .add_filter("Tag file", &[extension.as_str()]);
        if let Some(root) = self
            .editing_kit_paths
            .get(&target_game)
            .map(|root| configured_tags_root(root))
            .filter(|root| root.is_dir())
        {
            picker = picker.set_directory(root);
        }
        let Some(mut output) = picker.save_file() else {
            return;
        };
        if output.extension().is_none() {
            output.set_extension(&extension);
        }
        if self.path_is_inside_loaded_tags_root(&output) {
            if let Some(dialog) = self.tag_conversion_dialog.as_mut() {
                dialog.pending_source_destination = Some(output);
            }
            return;
        }
        self.save_tag_conversion_to(output);
    }

    pub(super) fn confirm_tag_conversion_inside_source(&mut self) {
        let output = self
            .tag_conversion_dialog
            .as_mut()
            .and_then(|dialog| dialog.pending_source_destination.take());
        if let Some(output) = output {
            self.save_tag_conversion_to(output);
        }
    }

    fn path_is_inside_loaded_tags_root(&self, output: &Path) -> bool {
        let Some(root) = self.loaded_tags_root() else {
            return false;
        };
        let root = normalize_conversion_path(&root);
        let output = normalize_conversion_path(output);
        output.starts_with(root)
    }

    fn save_tag_conversion_to(&mut self, output: PathBuf) {
        let Some(dialog) = self.tag_conversion_dialog.as_ref() else {
            return;
        };
        let Some(document) = self.parsed_tags.get(&dialog.source_key) else {
            if let Some(dialog) = self.tag_conversion_dialog.as_mut() {
                dialog.error = Some("The source tag is no longer loaded".to_owned());
            }
            return;
        };
        let fingerprint = match tag_fingerprint(&document.tag) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                if let Some(dialog) = self.tag_conversion_dialog.as_mut() {
                    dialog.error = Some(error);
                }
                return;
            }
        };
        if dialog
            .draft
            .as_ref()
            .is_none_or(|draft| draft.source_fingerprint != fingerprint)
        {
            if let Some(dialog) = self.tag_conversion_dialog.as_mut() {
                dialog.draft = None;
                dialog.error = Some(
                    "The source tag changed after analysis. Analyze the conversion again."
                        .to_owned(),
                );
            }
            return;
        }
        let target_game = dialog.target_game.clone();
        let Some(target_tags_root) = self
            .editing_kit_paths
            .get(&target_game)
            .map(|root| configured_tags_root(root))
        else {
            if let Some(dialog) = self.tag_conversion_dialog.as_mut() {
                dialog.error = Some(format!(
                    "Configure a valid {target_game} editing-kit root before saving a conversion"
                ));
            }
            return;
        };
        let dependency_schema = locate_definitions_root()
            .join(&target_game)
            .join("tag_dependency_list.json");
        let result = (|| {
            let dialog = self
                .tag_conversion_dialog
                .as_mut()
                .expect("conversion dialog checked above");
            let draft = dialog.draft.as_mut().expect("draft checked above");
            let companion_outputs =
                prepare_companion_outputs(draft, &output, &target_tags_root, &dependency_schema)?;
            for path in companion_outputs.iter().chain(std::iter::once(&output)) {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("Could not create {}: {error}", parent.display())
                    })?;
                }
            }
            for (companion, path) in draft.companion_tags.iter().zip(&companion_outputs) {
                companion
                    .tag
                    .write_atomic(path)
                    .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
            }
            draft
                .tag
                .write_atomic(&output)
                .map_err(|error| format!("Could not save {}: {error}", output.display()))?;
            Ok::<usize, String>(companion_outputs.len())
        })();
        match result {
            Ok(companion_count) => {
                self.status = if companion_count == 0 {
                    format!("Saved converted tag to {}", output.display())
                } else {
                    format!(
                        "Saved converted tag and {companion_count} companion tag(s) to {}",
                        output.display()
                    )
                };
                self.tag_conversion_dialog = None;
            }
            Err(error) => {
                if let Some(dialog) = self.tag_conversion_dialog.as_mut() {
                    dialog.error = Some(error);
                }
            }
        }
    }
}

fn normalize_conversion_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn configured_tags_root(path: &Path) -> PathBuf {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("tags"))
    {
        path.to_path_buf()
    } else {
        path.join("tags")
    }
}

impl Baboon {
    pub(super) fn open_folder_conversion_dialog(&mut self, rel_path: PathBuf, label: String) {
        if !self.expert_mode {
            self.status = "Cross-game folder conversion requires Expert mode".to_owned();
            return;
        }
        let Some(source_game) = self.source.as_ref().and_then(|source| source.game.clone()) else {
            self.status = "Folder conversion requires a detected editing-kit profile".to_owned();
            return;
        };
        if !CONVERSION_GAMES.contains(&source_game.as_str()) {
            self.status =
                "Folder conversion currently supports MCC Halo 3-family profiles only".to_owned();
            return;
        }
        let target_game = CONVERSION_GAMES
            .iter()
            .copied()
            .find(|game| *game != source_game)
            .unwrap_or("halo3odst_mcc")
            .to_owned();
        self.folder_conversion_dialog = Some(FolderConversionDialog {
            source_rel_path: rel_path,
            source_label: label,
            source_game,
            target_game,
            destination_parent: None,
            running: false,
            progress: None,
            report: None,
            error: None,
        });
    }

    pub(super) fn choose_folder_conversion_destination(&mut self) {
        let Some(dialog) = self.folder_conversion_dialog.as_ref() else {
            return;
        };
        let target_game = dialog.target_game.clone();
        let Some(target_root) = self
            .editing_kit_paths
            .get(&target_game)
            .map(|path| configured_tags_root(path))
            .filter(|path| path.is_dir())
        else {
            if let Some(dialog) = self.folder_conversion_dialog.as_mut() {
                dialog.error = Some(format!(
                    "Configure a valid {target_game} editing-kit root or tags folder in Settings"
                ));
            }
            return;
        };
        let Some(destination) = rfd::FileDialog::new()
            .set_title(format!("Choose {target_game} destination parent"))
            .set_directory(&target_root)
            .pick_folder()
        else {
            return;
        };
        let target_root = normalize_conversion_path(&target_root);
        let destination = normalize_conversion_path(&destination);
        if !destination.starts_with(&target_root) {
            if let Some(dialog) = self.folder_conversion_dialog.as_mut() {
                dialog.error =
                    Some("Choose a destination inside the target tags folder".to_owned());
            }
            return;
        }
        if let Some(dialog) = self.folder_conversion_dialog.as_mut() {
            dialog.destination_parent = Some(destination);
            dialog.report = None;
            dialog.error = None;
        }
    }

    pub(super) fn begin_folder_conversion(&mut self) {
        if !self.expert_mode {
            self.status = "Cross-game folder conversion requires Expert mode".to_owned();
            return;
        }
        let Some(dialog) = self.folder_conversion_dialog.as_ref() else {
            return;
        };
        if dialog.running {
            return;
        }
        if self.parsed_tags.values().any(|document| document.dirty) {
            if let Some(dialog) = self.folder_conversion_dialog.as_mut() {
                dialog.error =
                    Some("Save or close dirty tags before converting a folder".to_owned());
            }
            return;
        }
        let Some(destination_parent) = dialog.destination_parent.clone() else {
            if let Some(dialog) = self.folder_conversion_dialog.as_mut() {
                dialog.error = Some("Choose a destination folder first".to_owned());
            }
            return;
        };
        let (Some(source_data), Some(target_tags_root)) = (
            self.source.as_ref(),
            self.editing_kit_paths
                .get(&dialog.target_game)
                .map(|path| configured_tags_root(path)),
        ) else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source_data.source else {
            return;
        };
        let source_folder = normalize_conversion_path(&root.join(&dialog.source_rel_path));
        let output_root = normalize_conversion_path(&destination_parent.join(&dialog.source_label));
        if output_root.starts_with(&source_folder) || source_folder.starts_with(&output_root) {
            if let Some(dialog) = self.folder_conversion_dialog.as_mut() {
                dialog.error =
                    Some("The output folder must not overlap the source folder".to_owned());
            }
            return;
        }
        let source = source_data.source.clone();
        let names = source_data.names.clone();
        let source_rel_path = dialog.source_rel_path.clone();
        let source_label = dialog.source_label.clone();
        let source_game = dialog.source_game.clone();
        let target_game = dialog.target_game.clone();
        let tx = self.tx.clone();
        if let Some(dialog) = self.folder_conversion_dialog.as_mut() {
            dialog.running = true;
            dialog.progress = Some(FolderConversionProgress {
                phase: "Preparing".to_owned(),
                current: String::new(),
                processed: 0,
                total: 0,
                converted: 0,
                failed: 0,
            });
            dialog.report = None;
            dialog.error = None;
        }
        self.status = format!("Converting folder {source_label}");
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_folder_conversion_job(
                    FolderConversionJob {
                        source,
                        names,
                        source_rel_path,
                        source_label,
                        source_game,
                        target_game,
                        target_tags_root,
                        destination_parent,
                    },
                    &tx,
                )
            }))
            .unwrap_or_else(|_| Err("Folder conversion worker crashed".to_owned()));
            let _ = tx.send(WorkerMessage::FolderConversionFinished(result));
        });
    }

    pub(super) fn handle_folder_conversion_progress(
        &mut self,
        progress: FolderConversionProgress,
    ) -> bool {
        self.status = format!("Folder conversion: {}", progress.phase);
        if let Some(dialog) = self.folder_conversion_dialog.as_mut() {
            dialog.progress = Some(progress);
        }
        false
    }

    pub(super) fn handle_folder_conversion_finished(
        &mut self,
        result: Result<FolderConversionReport, String>,
    ) -> bool {
        if let Some(dialog) = self.folder_conversion_dialog.as_mut() {
            dialog.running = false;
            dialog.progress = None;
            match result {
                Ok(report) => {
                    self.status = format!(
                        "Converted {} tag(s); {} failed",
                        report.converted_count(),
                        report.failed_count()
                    );
                    dialog.report = Some(report);
                    dialog.error = None;
                }
                Err(error) => {
                    self.status = error.clone();
                    dialog.error = Some(error);
                }
            }
        }
        false
    }
}

struct FolderConversionJob {
    source: TagSource,
    names: TagNameIndex,
    source_rel_path: PathBuf,
    source_label: String,
    source_game: String,
    target_game: String,
    target_tags_root: PathBuf,
    destination_parent: PathBuf,
}

fn send_folder_conversion_progress(
    tx: &Sender<WorkerMessage>,
    phase: &str,
    current: &str,
    processed: usize,
    total: usize,
    converted: usize,
    failed: usize,
) {
    let _ = tx.send(WorkerMessage::FolderConversionProgress(
        FolderConversionProgress {
            phase: phase.to_owned(),
            current: current.to_owned(),
            processed,
            total,
            converted,
            failed,
        },
    ));
}

fn run_folder_conversion_job(
    job: FolderConversionJob,
    tx: &Sender<WorkerMessage>,
) -> Result<FolderConversionReport, String> {
    let TagSource::LooseFolder { root, .. } = &job.source else {
        return Err("Folder conversion requires a loose-folder source".to_owned());
    };
    let source_folder = normalize_conversion_path(&root.join(&job.source_rel_path));
    let target_tags_root = normalize_conversion_path(&job.target_tags_root);
    let destination_root =
        normalize_conversion_path(&job.destination_parent.join(&job.source_label));
    if !destination_root.starts_with(&target_tags_root) {
        return Err("Folder conversion destination escapes the target tags folder".to_owned());
    }

    let mut disk_paths = Vec::new();
    for item in walkdir::WalkDir::new(&source_folder).follow_links(false) {
        let item =
            item.map_err(|error| format!("Could not scan {}: {error}", source_folder.display()))?;
        if item.file_type().is_file() {
            disk_paths.push(item.into_path());
        }
    }
    disk_paths.sort();
    let total_files = disk_paths.len();
    let mut entries = Vec::new();
    let mut ignored_files = Vec::new();
    let mut scan_failures = Vec::new();
    for (index, path) in disk_paths.iter().enumerate() {
        let display = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        match loose_file_entry(root, path, &job.names) {
            Ok(Some(entry)) => entries.push(entry),
            Ok(None) => ignored_files.push(display.clone()),
            Err(error) => scan_failures.push(FolderConversionFileResult {
                source: display.clone(),
                output: None,
                status: FolderConversionFileStatus::Failed,
                overwritten: false,
                detail: format!("Could not identify tag: {error}"),
            }),
        }
        send_folder_conversion_progress(
            tx,
            "Scanning source folder",
            &display,
            index + 1,
            total_files,
            0,
            scan_failures.len(),
        );
    }

    let definitions_root = locate_definitions_root();
    let source_groups = GameTagIndex::load(&definitions_root, &job.source_game)?;
    let target_groups = GameTagIndex::load(&definitions_root, &job.target_game)?;
    send_folder_conversion_progress(
        tx,
        "Indexing native target layouts",
        "",
        0,
        entries.len(),
        0,
        scan_failures.len(),
    );
    let native_templates = NativeTemplateIndex::build(&target_tags_root, &target_groups);

    let mut planned = Vec::new();
    let mut destination_counts = HashMap::<String, usize>::new();
    for entry in entries {
        let destination = target_destination_for_entry(
            &entry,
            &source_folder,
            &destination_root,
            &source_groups,
            &target_groups,
        );
        if let Ok(path) = &destination {
            let key = normalize_conversion_path(path)
                .to_string_lossy()
                .to_ascii_lowercase();
            *destination_counts.entry(key).or_default() += 1;
        }
        planned.push((entry, destination));
    }

    let total = planned.len();
    let mut report = FolderConversionReport {
        source_label: job.source_label,
        target_game: job.target_game.clone(),
        destination_root,
        files: scan_failures,
        ignored_files,
    };
    let mut converted = 0;
    let mut failed = report.files.len();
    let mut claimed_outputs = HashSet::<String>::new();
    for (index, (entry, destination)) in planned.into_iter().enumerate() {
        let source_label = entry.display_path.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let output = destination?;
            let key = normalize_conversion_path(&output)
                .to_string_lossy()
                .to_ascii_lowercase();
            if destination_counts.get(&key).copied().unwrap_or(0) > 1 {
                return Err(format!(
                    "Multiple source tags map to the same destination: {}",
                    output.display()
                ));
            }
            let source_tag = read_entry(&job.source, &entry)
                .map_err(|error| format!("Could not read source tag: {error}"))?;
            let mut draft = analyze_conversion_with_templates(
                &source_tag,
                &job.source_game,
                &job.target_game,
                &definitions_root,
                Some(&native_templates),
            )?;
            let dependency_schema = definitions_root
                .join(&job.target_game)
                .join("tag_dependency_list.json");
            let companion_outputs = prepare_companion_outputs(
                &mut draft,
                &output,
                &target_tags_root,
                &dependency_schema,
            )?;
            let all_outputs = std::iter::once(&output)
                .chain(companion_outputs.iter())
                .collect::<Vec<_>>();
            let mut local_outputs = HashSet::new();
            for path in &all_outputs {
                let key = normalize_conversion_path(path)
                    .to_string_lossy()
                    .to_ascii_lowercase();
                if !local_outputs.insert(key.clone()) || claimed_outputs.contains(&key) {
                    return Err(format!(
                        "Multiple generated tags map to the same destination: {}",
                        path.display()
                    ));
                }
            }
            claimed_outputs.extend(local_outputs);
            let overwritten = all_outputs.iter().any(|path| path.exists());
            let native_layout_template = draft.native_layout_template.clone();
            let conversion_details = draft
                .report
                .issues
                .iter()
                .map(|issue| {
                    let kind = match issue.kind {
                        ConversionIssueKind::Unsupported => "unsupported",
                        ConversionIssueKind::Truncated => "truncated",
                        ConversionIssueKind::Warning => "warning",
                    };
                    format!("{kind}: {} — {}", issue.path, issue.message)
                })
                .collect::<Vec<_>>();
            for path in &all_outputs {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("Could not create {}: {error}", parent.display())
                    })?;
                }
            }
            for (companion, path) in draft.companion_tags.iter().zip(&companion_outputs) {
                companion
                    .tag
                    .write_atomic(path)
                    .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
            }
            draft
                .tag
                .write_atomic(&output)
                .map_err(|error| format!("Could not save {}: {error}", output.display()))?;
            let status = if native_layout_template.is_some() {
                FolderConversionFileStatus::NativeLayout
            } else {
                FolderConversionFileStatus::GeneratedLayout
            };
            let mut detail = if let Some(template) = native_layout_template {
                format!("Native layout template: {}", template.display())
            } else {
                "Generated layout; native editing-kit compatibility is unverified".to_owned()
            };
            if !conversion_details.is_empty() {
                detail.push_str(" | ");
                detail.push_str(&conversion_details.join("; "));
            }
            if !companion_outputs.is_empty() {
                detail.push_str(" | generated companions: ");
                detail.push_str(
                    &companion_outputs
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            Ok(FolderConversionFileResult {
                source: source_label.clone(),
                output: Some(output),
                status,
                overwritten,
                detail,
            })
        }))
        .unwrap_or_else(|payload| {
            let detail = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|text| (*text).to_owned())
                })
                .unwrap_or_else(|| "unknown panic payload".to_owned());
            Err(format!("Conversion panicked: {detail}"))
        });

        let file_result = match result {
            Ok(result) => {
                converted += 1;
                result
            }
            Err(error) => {
                failed += 1;
                FolderConversionFileResult {
                    source: source_label.clone(),
                    output: None,
                    status: FolderConversionFileStatus::Failed,
                    overwritten: false,
                    detail: error,
                }
            }
        };
        let status_label = match file_result.status {
            FolderConversionFileStatus::NativeLayout => "native",
            FolderConversionFileStatus::GeneratedLayout => "generated",
            FolderConversionFileStatus::Failed => "failed",
        };
        let _ = tx.send(WorkerMessage::TerminalLine(format!(
            "[folder conversion/{status_label}] {}: {}",
            file_result.source, file_result.detail
        )));
        report.files.push(file_result);
        send_folder_conversion_progress(
            tx,
            "Converting tags",
            &source_label,
            index + 1,
            total,
            converted,
            failed,
        );
    }
    report
        .files
        .sort_by(|left, right| left.source.cmp(&right.source));
    let _ = tx.send(WorkerMessage::TerminalLine(format!(
        "Folder conversion complete: {} converted, {} failed, {} ignored",
        report.converted_count(),
        report.failed_count(),
        report.ignored_files.len()
    )));
    Ok(report)
}

fn target_destination_for_entry(
    entry: &TagEntry,
    source_folder: &Path,
    destination_root: &Path,
    source_groups: &GameTagIndex,
    target_groups: &GameTagIndex,
) -> Result<PathBuf, String> {
    let TagEntryLocation::LooseFile(source_path) = &entry.location else {
        return Err("Folder conversion only supports loose tags".to_owned());
    };
    let relative = normalize_conversion_path(source_path)
        .strip_prefix(source_folder)
        .map(Path::to_path_buf)
        .map_err(|_| "Source tag escapes the selected folder".to_owned())?;
    let source_group_name = source_groups
        .by_tag
        .get(&entry.group_tag)
        .ok_or_else(|| format!("Unknown source group {}", format_group_tag(entry.group_tag)))?;
    let target_group = target_groups
        .by_name
        .get(&source_group_name.to_ascii_lowercase())
        .copied()
        .ok_or_else(|| format!("Target profile has no {source_group_name} group"))?;
    let target_group_name = target_groups
        .by_tag
        .get(&target_group)
        .map(String::as_str)
        .unwrap_or(source_group_name);
    let extension = group_tag_to_extension(target_group).unwrap_or(target_group_name);
    let mut output = normalize_conversion_path(&destination_root.join(relative));
    output.set_extension(extension);
    if !output.starts_with(destination_root) {
        return Err("Destination escapes the selected target folder".to_owned());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct LeafSeed {
        ordinal: usize,
        field_type: TagFieldType,
        option: Option<String>,
    }

    fn first_direct_leaf(tag: &TagFile, wanted: impl Fn(TagFieldType) -> bool) -> LeafSeed {
        tag.root()
            .fields()
            .enumerate()
            .find_map(|(ordinal, field)| {
                wanted(field.field_type()).then(|| {
                    let option = match field.options() {
                        Some(TagOptions::Enum { names, .. }) => {
                            names.get(1).or(names.first()).map(|s| (*s).to_owned())
                        }
                        Some(TagOptions::Flags(options)) => {
                            options.first().map(|option| option.name.to_owned())
                        }
                        None => None,
                    };
                    LeafSeed {
                        ordinal,
                        field_type: field.field_type(),
                        option,
                    }
                })
            })
            .expect("expected direct field type")
    }

    fn seed_weapon_fields(tag: &mut TagFile) {
        let reference =
            first_direct_leaf(tag, |field_type| field_type == TagFieldType::TagReference);
        tag.root_mut()
            .field_at_mut(reference.ordinal)
            .unwrap()
            .set(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((
                    u32::from_be_bytes(*b"bitm"),
                    "objects\\test\\icon".to_owned(),
                )),
            }))
            .unwrap();

        let real = first_direct_leaf(tag, is_real_scalar);
        tag.root_mut()
            .field_at_mut(real.ordinal)
            .unwrap()
            .set(real_field_value(real.field_type, 0.625))
            .unwrap();

        let enumeration = first_direct_leaf(tag, is_enum_type);
        let enum_name = enumeration.option.unwrap();
        let enum_value = match enumeration.field_type {
            TagFieldType::CharEnum => TagFieldData::CharEnum {
                value: 1,
                name: Some(enum_name),
            },
            TagFieldType::ShortEnum => TagFieldData::ShortEnum {
                value: 1,
                name: Some(enum_name),
            },
            TagFieldType::LongEnum => TagFieldData::LongEnum {
                value: 1,
                name: Some(enum_name),
            },
            _ => unreachable!(),
        };
        tag.root_mut()
            .field_at_mut(enumeration.ordinal)
            .unwrap()
            .set(enum_value)
            .unwrap();

        let flags = first_direct_leaf(tag, is_flags_type);
        let flag_name = flags.option.unwrap();
        let flag_value = match flags.field_type {
            TagFieldType::ByteFlags => TagFieldData::ByteFlags {
                value: 1,
                names: vec![(0, flag_name)],
            },
            TagFieldType::WordFlags => TagFieldData::WordFlags {
                value: 1,
                names: vec![(0, flag_name)],
            },
            TagFieldType::LongFlags => TagFieldData::LongFlags {
                value: 1,
                names: vec![(0, flag_name)],
            },
            _ => unreachable!(),
        };
        tag.root_mut()
            .field_at_mut(flags.ordinal)
            .unwrap()
            .set(flag_value)
            .unwrap();

        let string_id = first_direct_leaf(tag, is_string_id_type);
        let string_value = if string_id.field_type == TagFieldType::StringId {
            TagFieldData::StringId(StringIdData {
                string: "converted-label".to_owned(),
            })
        } else {
            TagFieldData::OldStringId(StringIdData {
                string: "converted-label".to_owned(),
            })
        };
        tag.root_mut()
            .field_at_mut(string_id.ordinal)
            .unwrap()
            .set(string_value)
            .unwrap();

        let magazines = tag
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| {
                field.field_type() == TagFieldType::Block
                    && clean_field_key(field.name()) == "magazines"
            })
            .map(|(ordinal, _)| ordinal)
            .expect("weapon has magazines block");
        let mut root = tag.root_mut();
        let mut field = root.field_at_mut(magazines).unwrap();
        let mut block = field.as_block_mut().unwrap();
        block.add_element();
    }

    #[test]
    fn halo3_weapon_converts_to_odst_and_reopens() {
        let root = locate_definitions_root();
        let mut source = TagFile::new(root.join("halo3_mcc/weapon.json")).unwrap();
        seed_weapon_fields(&mut source);

        let draft = analyze_conversion(&source, "halo3_mcc", "halo3odst_mcc", &root, None).unwrap();
        assert!(draft.native_layout_template.is_none());
        assert!(draft.report.issues.iter().any(|issue| {
            issue.path == "target layout" && issue.message.contains("native editing-kit")
        }));
        assert_eq!(draft.tag.group().tag, u32::from_be_bytes(*b"weap"));
        assert_eq!(draft.tag.header.build_version, 1);
        assert_eq!(draft.tag.header.build_number, 1);
        assert_eq!(draft.tag.header.version, u32::MAX);
        assert!(draft.report.copied_exact > 0);
        assert!(draft.report.converted_semantic > 0);
        assert!(draft
            .tag
            .root()
            .fields()
            .filter_map(|field| field.value())
            .any(|value| matches!(value, TagFieldData::TagReference(reference) if reference.group_tag_and_name.as_ref().is_some_and(|(group, path)| *group == u32::from_be_bytes(*b"bitm") && path == "objects\\test\\icon"))));
        assert_eq!(
            draft
                .tag
                .root()
                .field("magazines")
                .and_then(|field| field.as_block())
                .map(|block| block.len()),
            Some(1)
        );

        let mut path = std::env::temp_dir();
        path.push(format!(
            "baboon_conversion_weapon_{}_{}.weapon",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        draft.tag.write_atomic(&path).unwrap();
        let reopened = TagFile::read(&path).unwrap();
        assert_eq!(reopened.group().tag, u32::from_be_bytes(*b"weap"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn compute_shader_group_fourcc_remaps_for_reach() {
        let root = locate_definitions_root();
        let source = GameTagIndex::load(&root, "halo3_mcc").unwrap();
        let target = GameTagIndex::load(&root, "haloreach_mcc").unwrap();
        let name = source.by_tag.get(&u32::from_be_bytes(*b"cmpu")).unwrap();
        assert_eq!(
            target.by_name.get(name),
            Some(&u32::from_be_bytes(*b"cmps"))
        );
    }

    #[test]
    fn halo3_weapon_analyzes_for_every_supported_later_profile() {
        let root = locate_definitions_root();
        let source = TagFile::new(root.join("halo3_mcc/weapon.json")).unwrap();
        for target in ["haloreach_mcc", "halo4_mcc", "halo2amp_mcc"] {
            let draft = analyze_conversion(&source, "halo3_mcc", target, &root, None)
                .unwrap_or_else(|error| panic!("{target}: {error}"));
            assert_eq!(draft.tag.group().tag, u32::from_be_bytes(*b"weap"));
            assert_eq!(draft.tag.header.build_version, 1);
            assert_eq!(
                draft.tag.header.build_number,
                if target == "haloreach_mcc" || target == "halo4_mcc" || target == "halo2amp_mcc" {
                    2
                } else {
                    1
                }
            );
            assert_eq!(draft.tag.header.version, u32::MAX);
            draft.tag.write_to_bytes().unwrap();
        }
    }

    #[test]
    fn editing_kit_header_defaults_match_profile_generations() {
        let root = locate_definitions_root();
        for (game, build_number) in [
            ("halo3_mcc", 1),
            ("halo3odst_mcc", 1),
            ("haloreach_mcc", 2),
            ("halo4_mcc", 2),
            ("halo2amp_mcc", 2),
        ] {
            let mut tag = TagFile::new(root.join(game).join("globals.json")).unwrap();
            apply_editing_kit_mcc_header(&mut tag, game).unwrap();
            let bytes = tag.write_to_bytes().unwrap();
            assert_eq!(i32::from_le_bytes(bytes[36..40].try_into().unwrap()), 1);
            assert_eq!(
                i32::from_le_bytes(bytes[40..44].try_into().unwrap()),
                build_number
            );
            assert_eq!(
                u32::from_le_bytes(bytes[44..48].try_into().unwrap()),
                u32::MAX
            );
        }
    }

    #[test]
    fn missing_target_group_is_rejected() {
        let root = locate_definitions_root();
        let source = TagFile::new(root.join("halo3_mcc/gui_button_key_definition.json")).unwrap();
        let error = analyze_conversion(&source, "halo3_mcc", "haloreach_mcc", &root, None)
            .err()
            .expect("group should be absent");
        assert!(error.contains("has no gui_button_key_definition tag group"));
    }

    #[test]
    fn fingerprint_changes_with_source_edits() {
        let root = locate_definitions_root();
        let mut source = TagFile::new(root.join("halo3_mcc/weapon.json")).unwrap();
        let before = tag_fingerprint(&source).unwrap();
        let real = first_direct_leaf(&source, is_real_scalar);
        source
            .root_mut()
            .field_at_mut(real.ordinal)
            .unwrap()
            .set(real_field_value(real.field_type, 1.25))
            .unwrap();
        assert_ne!(before, tag_fingerprint(&source).unwrap());
    }

    #[test]
    fn integer_conversion_rejects_overflow() {
        assert!(integer_field_value(TagFieldType::ByteInteger, 256).is_none());
        assert!(integer_field_value(TagFieldType::CharInteger, -129).is_none());
        assert!(matches!(
            integer_field_value(TagFieldType::WordInteger, 65_535),
            Some(TagFieldData::WordInteger(65_535))
        ));
    }

    #[test]
    fn enum_and_flag_option_aliases_match_semantically() {
        assert!(option_names_match(
            "particle correlation 1{particle random 1}",
            "particle correlation 1"
        ));
        assert!(option_names_match(
            "resolved manually{resolved in postprocess|required by game}",
            "required by game"
        ));
        assert!(!option_names_match(
            "particle correlation 1",
            "particle correlation 2"
        ));
        assert!(option_names_match(" ", " "));
        assert!(option_names_match(
            "spew#fires its primary action barrel whenever the trigger is down",
            "spew"
        ));
        assert!(field_names_match(
            "coefficient*!",
            "spherical harmonic{coefficient}*!"
        ));
        assert!(!field_names_match("acceleration", "deceleration"));
    }

    #[test]
    fn mapping_catalog_is_scoped_and_reversible() {
        let catalog = ConversionMappingCatalog::load().unwrap();
        let coefficient_guid = parse_schema_guid("411d27e578471259100c498a81d58751").unwrap();
        assert!(catalog.field_names_match(FieldMappingRequest {
            group: "render_model",
            source_game: "halo3_mcc",
            target_game: "haloreach_mcc",
            source_guid: coefficient_guid,
            target_guid: coefficient_guid,
            source_name: "coefficient",
            target_name: "spherical harmonic",
        }));
        assert!(catalog.field_names_match(FieldMappingRequest {
            group: "render_model",
            source_game: "haloreach_mcc",
            target_game: "halo3_mcc",
            source_guid: coefficient_guid,
            target_guid: coefficient_guid,
            source_name: "spherical harmonic",
            target_name: "coefficient",
        }));
        assert!(catalog.option_names_match(
            "particle",
            "main flags",
            "halo3_mcc",
            "haloreach_mcc",
            "dies in media",
            "dies in water"
        ));
        assert!(catalog.option_names_match(
            "particle",
            "main flags",
            "haloreach_mcc",
            "halo3_mcc",
            "dies in water",
            "dies in media"
        ));
        assert!(catalog.option_names_match(
            "effect",
            "systems[0]/emitters[0]/movement/flags",
            "halo3_mcc",
            "halo4_mcc",
            "collide with media",
            "collide with water"
        ));
        assert!(catalog.option_names_match(
            "bitmap",
            "bitmap curve",
            "halo3_mcc",
            "haloreach_mcc",
            "sRGB",
            "sRGB (gamma 2.2)"
        ));
        assert!(catalog.option_names_match(
            "weapon",
            "secondary flags",
            "halo3_mcc",
            "halo2amp_mcc",
            "magnitizes only when zoomed",
            "magnetizes only when zoomed"
        ));
        assert!(!catalog.option_names_match(
            "weapon",
            "main flags",
            "halo3_mcc",
            "haloreach_mcc",
            "dies in media",
            "dies in water"
        ));
    }

    #[test]
    fn mapping_catalog_covers_complete_common_tag_base() {
        let root = locate_definitions_root();
        let catalog = ConversionMappingCatalog::load().unwrap();
        let indexes = CONVERSION_GAMES
            .iter()
            .map(|game| GameTagIndex::load(&root, game).unwrap())
            .collect::<Vec<_>>();
        let common_groups = indexes[0]
            .by_name
            .keys()
            .filter(|group| {
                indexes[1..]
                    .iter()
                    .all(|index| index.by_name.contains_key(*group))
            })
            .cloned()
            .collect::<HashSet<_>>();
        let covered_groups = catalog
            .covered_groups
            .iter()
            .map(|group| group.to_ascii_lowercase())
            .collect::<HashSet<_>>();

        assert_eq!(
            common_groups.len(),
            125,
            "common tag-base denominator changed"
        );
        assert!(
            covered_groups.is_subset(&common_groups),
            "covered groups must exist in every supported profile: {:?}",
            covered_groups
                .difference(&common_groups)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            covered_groups, common_groups,
            "the mapping catalog must cover the complete common tag base"
        );
    }

    #[test]
    fn every_covered_group_pair_is_compatible_or_explicitly_rejected() {
        let root = locate_definitions_root();
        let catalog = ConversionMappingCatalog::load().unwrap();
        let mut failures = Vec::new();
        for group in &catalog.covered_groups {
            // Layout-sensitive output intentionally requires a native
            // editing-kit template. Its path has dedicated tests below.
            if requires_native_layout_template(group) {
                continue;
            }
            for source_game in CONVERSION_GAMES {
                let source = match std::panic::catch_unwind(|| {
                    TagFile::new(root.join(source_game).join(format!("{group}.json")))
                }) {
                    Ok(Ok(source)) => source,
                    Ok(Err(error)) => {
                        failures.push(format!("{source_game}/{group}: {error}"));
                        continue;
                    }
                    Err(_) => {
                        failures.push(format!(
                            "{source_game}/{group}: schema construction panicked"
                        ));
                        continue;
                    }
                };
                for target_game in CONVERSION_GAMES {
                    if source_game == target_game {
                        continue;
                    }
                    let analysis = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        analyze_conversion(&source, source_game, target_game, &root, None)
                    }));
                    if analysis.is_err() {
                        failures.push(format!(
                            "{group}: {source_game} -> {target_game}: conversion panicked"
                        ));
                    } else if let Ok(Err(error)) = analysis {
                        if catalog
                            .incompatibility_reason(group, source_game, target_game)
                            .is_none()
                        {
                            failures
                                .push(format!("{group}: {source_game} -> {target_game}: {error}"));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn every_available_tag_group_pair_is_compatible_or_explicitly_rejected() {
        let root = locate_definitions_root();
        let catalog = ConversionMappingCatalog::load().unwrap();
        let indexes = CONVERSION_GAMES
            .iter()
            .map(|game| GameTagIndex::load(&root, game).unwrap())
            .collect::<Vec<_>>();
        let mut all_groups = indexes
            .iter()
            .flat_map(|index| index.by_name.keys().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        all_groups.sort();
        assert_eq!(all_groups.len(), 340, "supported tag-group union changed");

        let mut failures = Vec::new();
        for group in &all_groups {
            if requires_native_layout_template(group) {
                continue;
            }
            for (source_index, source_game) in CONVERSION_GAMES.iter().enumerate() {
                if !indexes[source_index].by_name.contains_key(group) {
                    continue;
                }
                if catalog.unusable_schema_reason(group, source_game).is_some() {
                    continue;
                }
                let source = TagFile::new(root.join(source_game).join(format!("{group}.json")))
                    .unwrap_or_else(|error| panic!("{source_game}/{group}: {error}"));
                for (target_index, target_game) in CONVERSION_GAMES.iter().enumerate() {
                    if source_game == target_game
                        || !indexes[target_index].by_name.contains_key(group)
                    {
                        continue;
                    }
                    if catalog.unusable_schema_reason(group, target_game).is_some() {
                        continue;
                    }
                    if let Err(error) =
                        analyze_conversion(&source, source_game, target_game, &root, None)
                    {
                        if catalog
                            .incompatibility_reason(group, source_game, target_game)
                            .is_none()
                        {
                            failures
                                .push(format!("{group}: {source_game} -> {target_game}: {error}"));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn schema_alias_table_maps_renamed_render_model_coefficient() {
        let definitions = locate_definitions_root();
        let aliases =
            SchemaFieldAliases::load(&definitions.join("haloreach_mcc/render_model.json")).unwrap();
        let guid = parse_schema_guid("411d27e578471259100c498a81d58751").unwrap();
        assert!(aliases.matches(guid, "spherical harmonic", "coefficient"));
    }

    #[test]
    fn light_values_reparent_into_halo4_midnight_parameters() {
        let definitions = locate_definitions_root();
        let mut source = TagFile::new(definitions.join("halo3_mcc/light.json")).unwrap();
        let ordinal = source
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "destroy light after")
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        source
            .root_mut()
            .field_at_mut(ordinal)
            .unwrap()
            .set(TagFieldData::Real(7.5))
            .unwrap();

        let draft =
            analyze_conversion(&source, "halo3_mcc", "halo4_mcc", &definitions, None).unwrap();
        let midnight = draft
            .tag
            .root()
            .fields()
            .find(|field| clean_field_key(field.name()) == "midnight_light_parameters")
            .and_then(|field| field.as_struct())
            .unwrap();
        assert!(matches!(
            midnight
                .fields()
                .find(|field| clean_field_key(field.name()) == "destroy light after")
                .and_then(|field| field.value()),
            Some(TagFieldData::Real(value)) if value == 7.5
        ));
    }

    #[test]
    fn halo3_reverb_values_reparent_into_reach_settings() {
        let definitions = locate_definitions_root();
        let mut source =
            TagFile::new(definitions.join("halo3_mcc/sound_environment.json")).unwrap();
        let ordinal = source
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "room intensity")
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        source
            .root_mut()
            .field_at_mut(ordinal)
            .unwrap()
            .set(TagFieldData::Real(-4.25))
            .unwrap();

        let draft =
            analyze_conversion(&source, "halo3_mcc", "haloreach_mcc", &definitions, None).unwrap();
        let reverb = draft
            .tag
            .root()
            .fields()
            .find(|field| clean_field_key(field.name()) == "reverb settings")
            .and_then(|field| field.as_struct())
            .unwrap();
        assert!(reverb.fields().any(|field| {
            clean_field_key(field.name()) == "room intensity"
                && matches!(field.value(), Some(TagFieldData::Real(value)) if value == -4.25)
        }));
    }

    #[test]
    fn model_and_biped_use_native_target_layout_templates() {
        let definitions = locate_definitions_root();
        for group in ["model", "biped"] {
            let tags_root = std::env::temp_dir().join(format!(
                "baboon_{group}_template_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&tags_root).unwrap();

            let mut template = TagFile::new(
                definitions
                    .join("haloreach_mcc")
                    .join(format!("{group}.json")),
            )
            .unwrap();
            apply_editing_kit_mcc_header(&mut template, "haloreach_mcc").unwrap();
            template.header.version = 42;
            template
                .write_atomic(tags_root.join(format!("template.{group}")))
                .unwrap();

            let source =
                TagFile::new(definitions.join("halo3_mcc").join(format!("{group}.json"))).unwrap();
            let draft = analyze_conversion(
                &source,
                "halo3_mcc",
                "haloreach_mcc",
                &definitions,
                Some(&tags_root),
            )
            .unwrap();
            assert!(draft.native_layout_template.is_some());
            assert!(draft.report.issues.iter().any(|issue| {
                issue.path == "target layout"
                    && issue.message.contains(&format!("native {group} layout"))
            }));
            let output = tags_root.join(format!("converted.{group}"));
            draft.tag.write_atomic(&output).unwrap();
            assert_eq!(
                TagFile::read(output).unwrap().group().tag,
                source.group().tag
            );
            let _ = fs::remove_dir_all(tags_root);
        }
    }

    #[test]
    fn editor_order_annotations_do_not_alias_unrelated_fields() {
        assert!(field_names_match(
            "animations*|ABCDCC",
            "animations*|ABCDCC"
        ));
        assert!(!field_names_match(
            "animations*|ABCDCC",
            "sound references|ABCDCC!*#Legacy field"
        ));
    }

    #[test]
    fn reach_animation_entries_default_unmapped_shared_reference_to_none() {
        let definitions = locate_definitions_root();
        let mut source =
            TagFile::new(definitions.join("halo3_mcc/model_animation_graph.json")).unwrap();
        let definitions_ordinal = source
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "definitions")
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        let mut root = source.root_mut();
        let mut definitions_field = root.field_at_mut(definitions_ordinal).unwrap();
        let mut definitions_struct = definitions_field.as_struct_mut().unwrap();
        let animations_ordinal = definitions_struct
            .as_ref()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()).starts_with("animations"))
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        let mut animations_field = definitions_struct.field_at_mut(animations_ordinal).unwrap();
        let mut animations = animations_field.as_block_mut().unwrap();
        let animation_index = animations.add_element();
        let mut animation = animations.element_mut(animation_index).unwrap();
        let node_count_ordinal = animation
            .as_ref()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()).starts_with("node count"))
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        animation
            .field_at_mut(node_count_ordinal)
            .unwrap()
            .set(TagFieldData::CharInteger(7))
            .unwrap();
        drop(animation);
        drop(animations);
        drop(animations_field);
        drop(definitions_struct);
        drop(definitions_field);
        drop(root);
        assert_eq!(
            struct_at_path(source.root(), "definitions")
                .unwrap()
                .fields()
                .find(|field| clean_field_key(field.name()).starts_with("animations"))
                .and_then(|field| field.as_block())
                .map(|block| block.len()),
            Some(1)
        );

        let draft =
            analyze_conversion(&source, "halo3_mcc", "haloreach_mcc", &definitions, None).unwrap();
        let target_definitions = struct_at_path(draft.tag.root(), "definitions").unwrap();
        let target_animations = target_definitions
            .fields()
            .find(|field| clean_field_key(field.name()).starts_with("animations"))
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(
            target_animations.len(),
            1,
            "animation block was not transferred"
        );
        let animation = target_animations.element(0).unwrap();
        let shared_reference = animation
            .fields()
            .find(|field| clean_field_key(field.name()).starts_with("shared animation reference"))
            .and_then(|field| field.as_struct())
            .unwrap();
        assert!(matches!(
            shared_reference
                .fields()
                .find(|field| clean_field_key(field.name()).starts_with("graph reference"))
                .and_then(|field| field.value()),
            Some(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: None
            }))
        ));
        assert!(matches!(
            shared_reference
                .fields()
                .find(|field| clean_field_key(field.name()).starts_with("shared animation index"))
                .and_then(|field| field.value()),
            Some(TagFieldData::ShortBlockIndex(-1))
        ));
        let shared_data = animation
            .fields()
            .find(|field| clean_field_key(field.name()).starts_with("shared animation data"))
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(shared_data.len(), 1);
        assert!(matches!(
            shared_data
                .element(0)
                .and_then(|payload| {
                    payload
                        .fields()
                        .find(|field| clean_field_key(field.name()).starts_with("node count"))
                })
                .and_then(|field| field.value()),
            Some(TagFieldData::CharInteger(7))
        ));
        assert_eq!(
            target_definitions
                .fields()
                .find(|field| clean_field_key(field.name()).starts_with("sound references"))
                .and_then(|field| field.as_block())
                .map(|block| block.len()),
            Some(0),
            "animation payload must not be routed through editor annotation aliases"
        );
    }

    #[test]
    fn particle_template_is_cleared_before_conversion() {
        let definitions = locate_definitions_root();
        let unique = format!(
            "baboon_particle_template_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let tags_root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&tags_root).unwrap();

        let mut template = TagFile::new(definitions.join("haloreach_mcc/particle.json")).unwrap();
        apply_editing_kit_mcc_header(&mut template, "haloreach_mcc").unwrap();
        template.header.version = 42;
        let low_res = template
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "low res switch distance")
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        template
            .root_mut()
            .field_at_mut(low_res)
            .unwrap()
            .set(TagFieldData::Real(123.0))
            .unwrap();
        template
            .add_import_info(definitions.join("haloreach_mcc/tag_import_information.json"))
            .unwrap();
        template
            .add_asset_depot_storage(definitions.join("haloreach_mcc/asset_depot_storage.json"))
            .unwrap();
        template
            .write_atomic(tags_root.join("template.particle"))
            .unwrap();

        let mut source = TagFile::new(definitions.join("halo3_mcc/particle.json")).unwrap();
        let main_flags = source
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "main flags")
            .and_then(|(ordinal, field)| match field.options() {
                Some(TagOptions::Flags(options)) => options
                    .iter()
                    .find(|option| option.name == "dies in media")
                    .map(|option| (ordinal, option.bit, option.name.to_owned())),
                _ => None,
            })
            .unwrap();
        source
            .root_mut()
            .field_at_mut(main_flags.0)
            .unwrap()
            .set(TagFieldData::LongFlags {
                value: (1u32 << main_flags.1) as i32,
                names: vec![(main_flags.1, main_flags.2)],
            })
            .unwrap();
        let billboard = source
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "particle billboard style")
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        source
            .root_mut()
            .field_at_mut(billboard)
            .unwrap()
            .set(TagFieldData::ShortEnum {
                value: 6,
                name: Some("local vertical".to_owned()),
            })
            .unwrap();
        let attachments = source
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "attachments")
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        let mut root = source.root_mut();
        let mut attachments_field = root.field_at_mut(attachments).unwrap();
        let mut attachments_block = attachments_field.as_block_mut().unwrap();
        let attachment_index = attachments_block.add_element();
        let mut attachment = attachments_block.element_mut(attachment_index).unwrap();
        let type_ordinal = attachment
            .as_ref()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "type")
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        attachment
            .field_at_mut(type_ordinal)
            .unwrap()
            .set(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((
                    u32::from_be_bytes(*b"effe"),
                    "effects\\particles\\spark_attachment".to_owned(),
                )),
            }))
            .unwrap();
        drop(attachment);
        drop(attachments_block);
        drop(attachments_field);
        drop(root);
        let draft = analyze_conversion(
            &source,
            "halo3_mcc",
            "haloreach_mcc",
            &definitions,
            Some(&tags_root),
        )
        .unwrap();
        assert!(draft.report.issues.iter().any(|issue| {
            issue.path == "target layout" && issue.message.contains("native particle layout")
        }));
        assert!(draft.report.mapped_aliases > 0);
        assert!(draft.tag.root().fields().any(|field| {
            clean_field_key(field.name()) == "main flags"
                && matches!(field.value(), Some(TagFieldData::LongFlags { names, .. }) if names.iter().any(|(_, name)| name == "dies in water"))
        }));
        assert!(draft.tag.root().fields().any(|field| {
            clean_field_key(field.name()) == "particle billboard style"
                && matches!(field.value(), Some(TagFieldData::ShortEnum { name: Some(name), .. }) if name == "local vertical")
        }));
        let target_attachments = draft
            .tag
            .root()
            .fields()
            .find(|field| clean_field_key(field.name()) == "attachments")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(target_attachments.len(), 1);
        assert!(target_attachments.element(0).unwrap().fields().any(|field| {
            clean_field_key(field.name()) == "type"
                && matches!(field.value(), Some(TagFieldData::TagReference(reference)) if reference.group_tag_and_name == Some((u32::from_be_bytes(*b"effe"), "effects\\particles\\spark_attachment".to_owned())))
        }));
        assert!(draft.tag.root().fields().any(|field| {
            clean_field_key(field.name()) == "low res switch distance"
                && matches!(field.value(), Some(TagFieldData::Real(value)) if value == 0.0)
        }));
        assert!(draft.tag.import_info().is_none());
        assert!(draft.tag.asset_depot_storage().is_none());

        fs::remove_dir_all(tags_root).unwrap();
    }

    #[test]
    fn particle_downport_rejects_unmatched_material_reference() {
        let definitions = locate_definitions_root();
        let unique = format!(
            "baboon_particle_downport_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let tags_root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&tags_root).unwrap();
        let mut template = TagFile::new(definitions.join("halo3_mcc/particle.json")).unwrap();
        apply_editing_kit_mcc_header(&mut template, "halo3_mcc").unwrap();
        template.header.version = 42;
        template
            .write_atomic(tags_root.join("template.particle"))
            .unwrap();

        let mut source = TagFile::new(definitions.join("halo4_mcc/particle.json")).unwrap();
        let material_ordinal = source
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "actual material?")
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        let mut root = source.root_mut();
        let mut material_field = root.field_at_mut(material_ordinal).unwrap();
        let mut material = material_field.as_struct_mut().unwrap();
        let shader_ordinal = material
            .as_ref()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_key(field.name()) == "material shader")
            .map(|(ordinal, _)| ordinal)
            .unwrap();
        material
            .field_at_mut(shader_ordinal)
            .unwrap()
            .set(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((
                    u32::from_be_bytes(*b"mats"),
                    "materials\\particles\\energy".to_owned(),
                )),
            }))
            .unwrap();
        drop(material);
        drop(material_field);
        drop(root);

        let error = analyze_conversion(
            &source,
            "halo4_mcc",
            "halo3_mcc",
            &definitions,
            Some(&tags_root),
        )
        .err()
        .expect("reference loss should reject conversion");
        assert!(error.contains("Cannot preserve reference"), "{error}");
        assert!(error.contains("materials\\particles\\energy"));

        fs::remove_dir_all(tags_root).unwrap();
    }

    #[test]
    fn target_default_count_excludes_layout_and_runtime_storage() {
        assert!(!is_reportable_target_default(TagFieldType::Custom));
        assert!(!is_reportable_target_default(TagFieldType::Pad));
        assert!(!is_reportable_target_default(
            TagFieldType::PageableResource
        ));
        assert!(is_reportable_target_default(TagFieldType::Real));
        assert!(is_reportable_target_default(TagFieldType::Block));
    }

    #[test]
    fn default_values_are_not_reported_as_meaningful() {
        assert!(!value_is_meaningful(TagFieldData::Real(0.0)));
        assert!(!value_is_meaningful(TagFieldData::TagReference(
            TagReferenceData {
                group_tag_and_name: None,
            }
        )));
        assert!(!value_is_meaningful(TagFieldData::Data(Vec::new())));
    }

    #[test]
    fn reference_fidelity_rejects_missing_non_empty_reference() {
        let definitions = locate_definitions_root();
        let source_groups = GameTagIndex::load(&definitions, "halo3_mcc").unwrap();
        let target_groups = GameTagIndex::load(&definitions, "haloreach_mcc").unwrap();
        let mut source = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
        seed_weapon_fields(&mut source);
        let target = TagFile::new(definitions.join("haloreach_mcc/weapon.json")).unwrap();
        let catalog = ConversionMappingCatalog::load().unwrap();
        let mut report = TagConversionReport::default();
        let error = validate_reference_fidelity(
            &source,
            &target,
            &source_groups,
            &target_groups,
            "weapon",
            "halo3_mcc",
            "haloreach_mcc",
            &catalog,
            &mut report,
        )
        .unwrap_err();
        assert!(error.contains("objects\\test\\icon"));
    }

    fn set_struct_reference(
        structure: &mut TagStructMut<'_>,
        key: &str,
        group_tag: u32,
        path: &str,
    ) {
        let ordinal = field_ordinal_by_key(structure.as_ref(), key).unwrap();
        structure
            .field_at_mut(ordinal)
            .unwrap()
            .set(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((group_tag, path.to_owned())),
            }))
            .unwrap();
    }

    #[test]
    fn halo3_melee_hit_references_map_to_unique_reach_block_entries() {
        let definitions = locate_definitions_root();
        let mut source = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
        let melee_ordinal = field_ordinal_by_key(source.root(), "melee damage parameters").unwrap();
        let mut root = source.root_mut();
        let mut melee_field = root.field_at_mut(melee_ordinal).unwrap();
        let mut melee = melee_field.as_struct_mut().unwrap();
        let damage_group = u32::from_be_bytes(*b"jpt!");
        for (prefix, damage) in [
            ("1st hit", "objects\\weapons\\damage_effects\\strike_melee"),
            ("2nd hit", "objects\\weapons\\damage_effects\\strike_melee"),
            ("3rd hit", "objects\\weapons\\damage_effects\\smash_melee"),
        ] {
            set_struct_reference(
                &mut melee,
                &format!("{prefix} melee damage"),
                damage_group,
                damage,
            );
            set_struct_reference(
                &mut melee,
                &format!("{prefix} melee response"),
                damage_group,
                "globals\\trigger_melee",
            );
        }
        drop(melee);
        drop(melee_field);
        drop(root);

        let draft =
            analyze_conversion(&source, "halo3_mcc", "haloreach_mcc", &definitions, None).unwrap();
        let melee_block = draft
            .tag
            .root()
            .fields()
            .find(|field| clean_field_key(field.name()) == "melee damage parameters")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(melee_block.len(), 2);
        let mut references = Vec::new();
        collect_reference_values(draft.tag.root(), "", &mut references);
        for path in [
            "objects\\weapons\\damage_effects\\strike_melee",
            "globals\\trigger_melee",
            "objects\\weapons\\damage_effects\\smash_melee",
        ] {
            assert!(
                references
                    .iter()
                    .any(|reference| reference.tag_path == path)
            );
        }
    }

    #[test]
    fn halo3_effect_looping_sound_maps_into_reach_block() {
        let definitions = locate_definitions_root();
        let mut source = TagFile::new(definitions.join("halo3_mcc/effect.json")).unwrap();
        let mut root = source.root_mut();
        set_struct_reference(
            &mut root,
            "looping sound",
            u32::from_be_bytes(*b"lsnd"),
            "sound\\visual_fx\\fire_large\\fire_large",
        );
        for (key, value) in [("location", 3), ("bind scale to event", 2)] {
            let ordinal = field_ordinal_by_key(root.as_ref(), key).unwrap();
            root.field_at_mut(ordinal)
                .unwrap()
                .set(TagFieldData::CharBlockIndex(value))
                .unwrap();
        }
        drop(root);

        let draft =
            analyze_conversion(&source, "halo3_mcc", "haloreach_mcc", &definitions, None).unwrap();
        let looping = field_by_key(draft.tag.root(), "looping sounds")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(looping.len(), 1);
        let element = looping.element(0).unwrap();
        assert!(matches!(
            field_by_key(element, "looping sound").and_then(|field| field.value()),
            Some(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((group, ref path)),
            })) if group == u32::from_be_bytes(*b"lsnd")
                && path == "sound\\visual_fx\\fire_large\\fire_large"
        ));
        assert!(matches!(
            field_by_key(element, "location").and_then(|field| field.value()),
            Some(TagFieldData::ShortBlockIndex(3))
        ));
        assert!(matches!(
            field_by_key(element, "bind scale to event").and_then(|field| field.value()),
            Some(TagFieldData::ShortBlockIndex(2))
        ));
    }

    #[test]
    fn halo3_lens_flare_occlusion_enum_maps_to_reach_scale() {
        let definitions = locate_definitions_root();
        let mut source = TagFile::new(definitions.join("halo3_mcc/lens_flare.json")).unwrap();
        let mut root = source.root_mut();
        let ordinal = field_ordinal_by_key(root.as_ref(), "occlusion inner radius scale").unwrap();
        root.field_at_mut(ordinal)
            .unwrap()
            .set(TagFieldData::ShortEnum {
                value: 3,
                name: Some("1/8".to_owned()),
            })
            .unwrap();
        drop(root);

        let draft =
            analyze_conversion(&source, "halo3_mcc", "haloreach_mcc", &definitions, None).unwrap();
        assert!(matches!(
            field_by_key(draft.tag.root(), "occlusion inner radius scale")
                .and_then(|field| field.value()),
            Some(TagFieldData::Real(value)) if value == 0.125
        ));
    }

    #[test]
    fn runtime_sensitive_groups_fail_instead_of_dropping_authored_fields() {
        let definitions = locate_definitions_root();
        let mut source = TagFile::new(definitions.join("halo3_mcc/light.json")).unwrap();
        let mut root = source.root_mut();
        let ordinal = field_ordinal_by_key(root.as_ref(), "percent spherical").unwrap();
        root.field_at_mut(ordinal)
            .unwrap()
            .set(TagFieldData::Real(0.75))
            .unwrap();
        drop(root);

        let error = analyze_conversion(&source, "halo3_mcc", "haloreach_mcc", &definitions, None)
            .err()
            .unwrap();
        assert!(error.contains("light conversion would lose 1 meaningful"));
        assert!(error.contains("percent spherical"));
    }

    #[test]
    fn halo3_player_response_generates_reach_companion_tags() {
        let definitions = locate_definitions_root();
        let mut source = TagFile::new(definitions.join("halo3_mcc/damage_effect.json")).unwrap();
        let responses_ordinal = field_ordinal_by_key(source.root(), "player responses").unwrap();
        let mut root = source.root_mut();
        let mut responses_field = root.field_at_mut(responses_ordinal).unwrap();
        let mut responses = responses_field.as_block_mut().unwrap();
        let response_index = responses.add_element();
        let response = responses.element_mut(response_index).unwrap();
        initialize_block_index_defaults(response);
        let mut response = responses.element_mut(response_index).unwrap();
        let response_type = field_ordinal_by_key(response.as_ref(), "response type").unwrap();
        response
            .field_at_mut(response_type)
            .unwrap()
            .set(TagFieldData::ShortEnum {
                value: 1,
                name: Some("unshielded".to_owned()),
            })
            .unwrap();
        let rumble_ordinal = field_ordinal_by_key(response.as_ref(), "rumble").unwrap();
        let mut rumble_field = response.field_at_mut(rumble_ordinal).unwrap();
        let mut rumble = rumble_field.as_struct_mut().unwrap();
        let low_ordinal = field_ordinal_by_key(rumble.as_ref(), "low frequency rumble").unwrap();
        let mut low_field = rumble.field_at_mut(low_ordinal).unwrap();
        let mut low = low_field.as_struct_mut().unwrap();
        let duration = field_ordinal_by_key(low.as_ref(), "duration").unwrap();
        low.field_at_mut(duration)
            .unwrap()
            .set(TagFieldData::Real(0.4))
            .unwrap();
        drop(low);
        drop(low_field);
        drop(rumble);
        drop(rumble_field);
        drop(response);
        drop(responses);
        drop(responses_field);
        drop(root);

        let mut draft =
            analyze_conversion(&source, "halo3_mcc", "haloreach_mcc", &definitions, None).unwrap();
        assert_eq!(draft.companion_tags.len(), 2);
        assert!(
            draft
                .companion_tags
                .iter()
                .any(|companion| companion.group_name == "damage_response_definition")
        );
        assert!(
            draft
                .companion_tags
                .iter()
                .any(|companion| companion.group_name == "rumble")
        );

        let tags_root = std::env::temp_dir().join(format!(
            "baboon_response_companions_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(tags_root.join("objects/test")).unwrap();
        let output = tags_root.join("objects/test/impact.damage_effect");
        let companion_outputs = prepare_companion_outputs(
            &mut draft,
            &output,
            &tags_root,
            &definitions.join("haloreach_mcc/tag_dependency_list.json"),
        )
        .unwrap();
        assert_eq!(companion_outputs.len(), 2);
        assert!(companion_outputs.iter().any(|path| {
            path.file_name()
                .is_some_and(|name| name == "impact__damage_response.damage_response_definition")
        }));
        let (_, response_path) = reference_by_key(draft.tag.root(), "damage response").unwrap();
        assert_eq!(response_path, "objects\\test\\impact__damage_response");
        fs::remove_dir_all(tags_root).unwrap();
    }

    #[test]
    fn native_reach_contrail_fixed_arrays_open_without_panicking() {
        let path = Path::new(
            "D:/SteamLibrary/steamapps/common/HREK/tags/cinematics/020lb_halsey/fx/010/mac_projectile.contrail_system",
        );
        if !path.is_file() {
            return;
        }
        let result = std::panic::catch_unwind(|| TagFile::read(path));
        assert!(result.is_ok(), "native Reach contrail read panicked");
        assert!(
            result.unwrap().is_ok(),
            "native Reach contrail is unreadable"
        );
    }

    #[test]
    fn h3_contrail_uses_native_reach_layout_when_kits_are_available() {
        let source_path = Path::new(
            "D:/SteamLibrary/steamapps/common/H3EK/tags/fx/cinematics/010la_jungle_intro/01/hatch.contrail_system",
        );
        let target_root = Path::new("D:/SteamLibrary/steamapps/common/HREK/tags");
        if !source_path.is_file() || !target_root.is_dir() {
            return;
        }
        let definitions = locate_definitions_root();
        let source = TagFile::read(source_path).unwrap();
        let draft = analyze_conversion(
            &source,
            "halo3_mcc",
            "haloreach_mcc",
            &definitions,
            Some(target_root),
        )
        .unwrap();
        assert!(draft.native_layout_template.is_some());
        let bytes = draft.tag.write_to_bytes().unwrap();
        TagFile::read_from_bytes(&bytes).unwrap();
    }

    #[test]
    fn real_h3_animation_payload_is_rejected_instead_of_written_incomplete() {
        let source_path = Path::new(
            "D:/SteamLibrary/steamapps/common/H3EK/tags/fx/null_object/null_up/null_up.model_animation_graph",
        );
        let target_root = Path::new("D:/SteamLibrary/steamapps/common/HREK/tags");
        if !source_path.is_file() || !target_root.is_dir() {
            return;
        }
        let definitions = locate_definitions_root();
        let source = TagFile::read(source_path).unwrap();
        let error = analyze_conversion(
            &source,
            "halo3_mcc",
            "haloreach_mcc",
            &definitions,
            Some(target_root),
        )
        .err()
        .expect("unsafe animation graph must not produce a draft");
        assert!(
            error.contains("pageable runtime resources")
                || error.contains("model_animation_graph conversion would lose"),
            "unexpected animation safety error: {error}"
        );
    }

    #[test]
    fn catalogued_legacy_model_and_particle_reference_drops_are_one_way() {
        let catalog = ConversionMappingCatalog::load().unwrap();
        for (group, field) in [("model", "lod_render_model"), ("particle", "shader")] {
            assert!(
                catalog
                    .reference_drop_reason(group, "halo3_mcc", "haloreach_mcc", field,)
                    .is_some()
            );
            assert!(
                catalog
                    .reference_drop_reason(group, "haloreach_mcc", "halo3_mcc", field,)
                    .is_none()
            );
        }
    }

    #[test]
    fn folder_conversion_recurses_overwrites_and_continues_after_failure() {
        let definitions = locate_definitions_root();
        let unique = format!(
            "baboon_folder_conversion_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let source_root = root.join("source_tags");
        let source_folder = source_root.join("characters/jackal");
        let target_tags = root.join("target_tags");
        let destination_parent = target_tags.join("objects/characters");
        fs::create_dir_all(source_folder.join("nested")).unwrap();
        fs::create_dir_all(&target_tags).unwrap();

        let mut source = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
        seed_weapon_fields(&mut source);
        apply_editing_kit_mcc_header(&mut source, "halo3_mcc").unwrap();
        source
            .write_atomic(source_folder.join("nested/jackal.weapon"))
            .unwrap();
        source
            .write_atomic(source_folder.join("nested/jackal_alt.weapon"))
            .unwrap();
        fs::write(source_folder.join("notes.txt"), b"not a tag").unwrap();

        let mut bad = TagFile::new(definitions.join("halo3_mcc/light.json")).unwrap();
        apply_editing_kit_mcc_header(&mut bad, "halo3_mcc").unwrap();
        let bad_bytes = bad.write_to_bytes().unwrap();
        fs::write(source_folder.join("broken.light"), &bad_bytes[..64]).unwrap();

        let output = destination_parent.join("jackal/nested/jackal.weapon");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        let mut existing = TagFile::new(definitions.join("haloreach_mcc/weapon.json")).unwrap();
        apply_editing_kit_mcc_header(&mut existing, "haloreach_mcc").unwrap();
        existing.header.version = 8;
        existing.write_atomic(&output).unwrap();

        let source = TagSource::LooseFolder {
            root: source_root,
            game: Some("halo3_mcc".to_owned()),
            definitions_root: definitions.clone(),
        };
        let names = TagNameIndex::load_from_definitions(&definitions);
        let (tx, _rx) = mpsc::channel();
        let report = run_folder_conversion_job(
            FolderConversionJob {
                source,
                names,
                source_rel_path: PathBuf::from("characters/jackal"),
                source_label: "jackal".to_owned(),
                source_game: "halo3_mcc".to_owned(),
                target_game: "haloreach_mcc".to_owned(),
                target_tags_root: target_tags,
                destination_parent: destination_parent.clone(),
            },
            &tx,
        )
        .unwrap();

        assert_eq!(report.native_count(), 2);
        assert_eq!(report.generated_count(), 0);
        assert_eq!(report.failed_count(), 1);
        assert_eq!(report.ignored_files, vec!["characters/jackal/notes.txt"]);
        assert!(report.files.iter().any(|file| {
            file.source == "characters/jackal/nested/jackal.weapon" && file.overwritten
        }));
        let reopened = TagFile::read(&output).unwrap();
        let mut references = Vec::new();
        collect_reference_values(reopened.root(), "", &mut references);
        assert!(references.iter().any(|reference| {
            reference.group_tag == u32::from_be_bytes(*b"bitm")
                && reference.tag_path == "objects\\test\\icon"
        }));
        assert!(
            destination_parent
                .join("jackal/nested/jackal_alt.weapon")
                .is_file()
        );

        fs::remove_dir_all(root).unwrap();
    }
}
