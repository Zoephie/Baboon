//! Deterministic generator for the cross-game tag compatibility database.
//! It owns deriving the per-field verdicts and writing them; browsing them
//! belongs to `app::tag_compat` and conversion policy to `app::conversion`.
//!
//! ## Why this is a build step and not a runtime computation
//!
//! The answer is a pure function of the pinned `definitions/` submodule, so
//! computing it per launch would make every user pay for a result that cannot
//! change between bumps — and would leave nothing to review, pin, or diff. It
//! is not a `build.rs` step either: `build.rs` runs before the crate compiles
//! and so cannot call `blam-tags`. That leaves a helper binary, which is how
//! `docs/script_docs.sqlite3` is already produced.
//!
//! ## The two layers
//!
//! Everything here is *derived*: it is what the two schemas say, with no
//! judgement applied. Judgement lives in `mappings/conversion_mappings.json`,
//! which this reads and never writes. That separation is the one
//! `docs/tag-conversion-mappings.md` describes, and it is what lets a reviewed
//! rename show up as [`CompatVerdict::RenamedProvable`] instead of a field
//! dropped and an unrelated one gained.

use rusqlite::{Connection, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use blam_tags::{
    AliasIndex, BlockReason, CompatSeverity, FieldVerdict, GroupComparison, StructComparison,
    TagFile,
};

pub const SCHEMA_VERSION: i64 = 1;

/// The pair this database exists for. Others can be requested on the command
/// line; this is what a bare invocation produces.
pub const DEFAULT_PAIRS: &[(&str, &str)] = &[
    ("haloreach_mcc", "haloce_evolved"),
    ("haloce_evolved", "haloreach_mcc"),
];

/// What happens to one field when a tag crosses a profile pair.
///
/// Ordered worst-first so a struct's verdict is the `min` of its fields' and a
/// group's the `min` of its structs'. One ordering at all three levels means a
/// reader learns one legend rather than three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompatVerdict {
    /// The converter refuses the pair outright.
    HardBlocked,
    /// Present in the source, nothing in the target claims it. Dropped.
    SourceOnly,
    /// Matched, but at least one enum or flag option has no target option.
    OptionLoss,
    /// Names match; types differ but the bytes are interchangeable.
    TypeChangedSafe,
    /// Names differ, matched by a rule that can be pointed at — a schema
    /// `{alias}` or a reviewed catalog entry. `rule` says which.
    RenamedProvable,
    /// Present in the target, nothing in the source fills it. Left at default.
    TargetOnly,
    /// Same cleaned name, same type, same size.
    Identical,
}

impl CompatVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HardBlocked => "hard_blocked",
            Self::SourceOnly => "source_only",
            Self::OptionLoss => "option_loss",
            Self::TypeChangedSafe => "type_changed_safe",
            Self::RenamedProvable => "renamed_provable",
            Self::TargetOnly => "target_only",
            Self::Identical => "identical",
        }
    }

    /// A one-line explanation for a reader who has not memorized the legend.
    pub fn describe(self) -> &'static str {
        match self {
            Self::HardBlocked => "cannot be converted",
            Self::SourceOnly => "dropped — the target has no such field",
            Self::OptionLoss => "some options have no counterpart",
            Self::TypeChangedSafe => "same bytes, different type name",
            Self::RenamedProvable => "renamed, and the rename is recorded",
            Self::TargetOnly => "left at its default — the source has no such field",
            Self::Identical => "transfers unchanged",
        }
    }
}

impl fmt::Display for CompatVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Translate one engine field verdict into a catalogue verdict.
///
/// `ContainerDrift` deliberately becomes `Identical` here: a container's own
/// row carries no data, and the drift it names is reported by the child struct's
/// rows. Folding it in twice would double-count every nested difference.
fn verdict_of(field: &FieldVerdict) -> CompatVerdict {
    match field {
        FieldVerdict::Identical | FieldVerdict::ContainerDrift | FieldVerdict::StructuralOnly => {
            CompatVerdict::Identical
        }
        FieldVerdict::Renamed { .. } => CompatVerdict::RenamedProvable,
        FieldVerdict::TypeEquivalent { .. } => CompatVerdict::TypeChangedSafe,
        FieldVerdict::OptionsRemapped { .. } => CompatVerdict::TypeChangedSafe,
        // Widening carries the value intact; narrowing may not, and the
        // converter reports what it could not hold.
        FieldVerdict::Requantized { source_width, target_width } => {
            if target_width >= source_width {
                CompatVerdict::TypeChangedSafe
            } else {
                CompatVerdict::OptionLoss
            }
        }
        FieldVerdict::OptionsLost { .. } => CompatVerdict::OptionLoss,
        FieldVerdict::SourceOnly => CompatVerdict::SourceOnly,
        FieldVerdict::TargetOnly => CompatVerdict::TargetOnly,
        FieldVerdict::Blocked(_) => CompatVerdict::HardBlocked,
    }
}

fn severity_verdict(severity: CompatSeverity) -> CompatVerdict {
    match severity {
        CompatSeverity::Identical => CompatVerdict::Identical,
        CompatSeverity::Lossless => CompatVerdict::TypeChangedSafe,
        CompatSeverity::Lossy => CompatVerdict::SourceOnly,
        CompatSeverity::Blocked => CompatVerdict::HardBlocked,
    }
}

fn block_reason(reason: &BlockReason) -> String {
    match reason {
        BlockReason::TypeIncompatible { source, target } => {
            format!("{source} faces {target}")
        }
        BlockReason::ContainerShape { source, target } => {
            format!("{source} faces {target}")
        }
        BlockReason::DataDefinition { source, target } => {
            format!("data definition `{source}` faces `{target}`")
        }
        BlockReason::ApiInteropGuid => "different api-interop runtime types".to_owned(),
        BlockReason::OpaqueBytes => "opaque engine-bound bytes".to_owned(),
    }
}

/// The reviewed-override layer, read only for the parts that change a derived
/// verdict. Everything else in the catalog is conversion policy, not schema
/// fact, and is none of this generator's business.
#[derive(Default, Deserialize)]
struct ReviewedCatalog {
    #[serde(default)]
    incompatible_pairs: Vec<ScopedReason>,
    #[serde(default)]
    unusable_schemas: Vec<UnusableSchema>,
    #[serde(default)]
    field_aliases: Vec<FieldAlias>,
}

#[derive(Deserialize)]
struct ScopedReason {
    group: String,
    #[serde(default)]
    source_games: Vec<String>,
    #[serde(default)]
    target_games: Vec<String>,
    reason: String,
}

#[derive(Deserialize)]
struct UnusableSchema {
    group: String,
    #[serde(default)]
    games: Vec<String>,
    reason: String,
}

#[derive(Deserialize)]
struct FieldAlias {
    group: String,
    #[serde(default)]
    source_games: Vec<String>,
    #[serde(default)]
    target_games: Vec<String>,
    source: String,
    target: String,
}

fn scope_matches(games: &[String], game: &str) -> bool {
    games.is_empty() || games.iter().any(|candidate| candidate == game)
}

impl ReviewedCatalog {
    fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn refusal(&self, group: &str, source: &str, target: &str) -> Option<String> {
        if let Some(rule) = self.unusable_schemas.iter().find(|rule| {
            rule.group.eq_ignore_ascii_case(group)
                && (scope_matches(&rule.games, source) || scope_matches(&rule.games, target))
        }) {
            return Some(rule.reason.clone());
        }
        self.incompatible_pairs
            .iter()
            .find(|rule| {
                rule.group.eq_ignore_ascii_case(group)
                    && ((scope_matches(&rule.source_games, source)
                        && scope_matches(&rule.target_games, target))
                        || (scope_matches(&rule.source_games, target)
                            && scope_matches(&rule.target_games, source)))
            })
            .map(|rule| rule.reason.clone())
    }

    /// Reviewed renames for this group and direction, as `source -> target`.
    /// Bidirectional, matching how the converter reads them.
    fn renames(&self, group: &str, source: &str, target: &str) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for rule in &self.field_aliases {
            if !rule.group.eq_ignore_ascii_case(group) {
                continue;
            }
            if scope_matches(&rule.source_games, source) && scope_matches(&rule.target_games, target)
            {
                out.insert(rule.source.clone(), rule.target.clone());
            }
            if scope_matches(&rule.source_games, target) && scope_matches(&rule.target_games, source)
            {
                out.insert(rule.target.clone(), rule.source.clone());
            }
        }
        out
    }
}

/// The canonical group names a profile defines, from its `_meta.json`.
fn groups_of(definitions: &Path, game: &str) -> Result<BTreeSet<String>, String> {
    let meta = definitions.join(game).join("_meta.json");
    let bytes = fs::read(&meta).map_err(|e| format!("read {}: {e}", meta.display()))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", meta.display()))?;
    let index = value
        .get("tag_index")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| format!("{} has no tag_index", meta.display()))?;
    Ok(index
        .values()
        .filter_map(serde_json::Value::as_str)
        .filter(|name| definitions.join(game).join(format!("{name}.json")).is_file())
        .map(str::to_owned)
        .collect())
}

fn fourcc_of(definitions: &Path, game: &str, group: &str) -> Option<String> {
    let meta = definitions.join(game).join("_meta.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(meta).ok()?).ok()?;
    value
        .get("tag_index")?
        .as_object()?
        .iter()
        .find(|(_, name)| name.as_str() == Some(group))
        .map(|(fourcc, _)| fourcc.clone())
}

/// One row destined for the `fields` table.
struct FieldRow {
    struct_key: String,
    ordinal: usize,
    source_name: Option<String>,
    source_type: Option<String>,
    target_name: Option<String>,
    target_type: Option<String>,
    verdict: CompatVerdict,
    rule: &'static str,
    detail: String,
}

/// One row destined for the `structs` table.
struct StructRow {
    struct_key: String,
    first_path: String,
    source_struct: String,
    target_struct: String,
    source_guid: String,
    target_guid: String,
    source_size: usize,
    target_size: usize,
    pairing: &'static str,
    verdict: CompatVerdict,
}

fn hex(guid: [u8; 16]) -> String {
    guid.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Shortest-then-lexicographic path to each struct pair, so a struct reachable
/// from several parents gets one stable, navigable address rather than an
/// arbitrary one.
fn first_paths(comparison: &GroupComparison) -> Vec<String> {
    let mut paths = vec![String::new(); comparison.structs.len()];
    paths[0] = comparison.root().source_name.clone();
    let mut queue = std::collections::VecDeque::from([0usize]);
    let mut seen = vec![false; comparison.structs.len()];
    seen[0] = true;
    while let Some(index) = queue.pop_front() {
        let here = paths[index].clone();
        for row in &comparison.structs[index].fields {
            let Some(child) = row.child else { continue };
            let name = row
                .source
                .as_ref()
                .or(row.target.as_ref())
                .map(|facts| facts.clean_name.as_str())
                .unwrap_or("?");
            let candidate = format!("{here}/{name}");
            if !seen[child.0] {
                seen[child.0] = true;
                paths[child.0] = candidate;
                queue.push_back(child.0);
            } else if candidate.len() < paths[child.0].len()
                || (candidate.len() == paths[child.0].len() && candidate < paths[child.0])
            {
                paths[child.0] = candidate;
            }
        }
    }
    paths
}

/// Turn one group comparison into database rows.
fn rows_for(
    comparison: &GroupComparison,
    renames: &BTreeMap<String, String>,
) -> (Vec<StructRow>, Vec<FieldRow>) {
    let paths = first_paths(comparison);
    let mut structs = Vec::new();
    let mut fields = Vec::new();

    for (index, structure) in comparison.structs.iter().enumerate() {
        let key = struct_key(structure, index);
        let mut worst = CompatVerdict::Identical;
        for (ordinal, row) in structure.fields.iter().enumerate() {
            let mut verdict = verdict_of(&row.verdict);
            let mut rule = match &row.verdict {
                FieldVerdict::Renamed { .. } => "schema_alias",
                _ => "schema",
            };

            // A reviewed rename rescues a pair the schemas alone report as one
            // field dropped and another gained. This is the only place the
            // override layer changes a derived verdict, and it can only ever
            // improve one.
            if verdict == CompatVerdict::SourceOnly {
                if let Some(name) = row.source.as_ref().map(|f| f.clean_name.as_str()) {
                    if renames.contains_key(name) {
                        verdict = CompatVerdict::RenamedProvable;
                        rule = "catalog:field_aliases";
                    }
                }
            }

            let detail = match &row.verdict {
                FieldVerdict::Renamed { alias } => format!("was `{alias}`"),
                FieldVerdict::TypeEquivalent { reason } => format!("{reason:?}"),
                FieldVerdict::Requantized { source_width, target_width } => {
                    format!("re-encoded from {source_width} to {target_width} bytes")
                }
                FieldVerdict::OptionsLost { lost, .. } => {
                    format!("no counterpart for: {}", lost.join(", "))
                }
                FieldVerdict::OptionsRemapped { remap } => {
                    format!("{} option(s) move ordinal", remap.len())
                }
                FieldVerdict::Blocked(reason) => block_reason(reason),
                _ => String::new(),
            };

            worst = worst.min(verdict);
            fields.push(FieldRow {
                struct_key: key.clone(),
                ordinal,
                source_name: row.source.as_ref().map(|f| f.clean_name.clone()),
                source_type: row.source.as_ref().map(|f| f.type_name.clone()),
                target_name: row.target.as_ref().map(|f| f.clean_name.clone()),
                target_type: row.target.as_ref().map(|f| f.type_name.clone()),
                verdict,
                rule,
                detail,
            });
        }

        structs.push(StructRow {
            struct_key: key,
            first_path: paths[index].clone(),
            source_struct: structure.source_name.clone(),
            target_struct: structure.target_name.clone(),
            source_guid: hex(structure.source_guid),
            target_guid: hex(structure.target_guid),
            source_size: structure.source_size,
            target_size: structure.target_size,
            pairing: pairing_of(structure),
            verdict: worst,
        });
    }
    (structs, fields)
}

/// Structs are keyed by declared source name. A group declares a couple of
/// hundred structs but its *tree* is unbounded, so keying by materialized path
/// would make the artifact grow without limit for no extra information.
///
/// The index suffix is a tie-breaker for the rare case where one source struct
/// pairs with two different target structs — reachable when a group reuses a
/// struct under two parents that themselves diverged.
fn struct_key(structure: &StructComparison, index: usize) -> String {
    if structure.source_name == structure.target_name {
        structure.source_name.clone()
    } else {
        format!("{}#{index}", structure.source_name)
    }
}

/// How this struct pair was matched. Recorded because GUID equality looks
/// authoritative and is not: `shared_model_animation_block` carries one GUID at
/// 212 bytes in Reach and 200 in Campaign Evolved, so any surprising row has to
/// be explicable without re-deriving the walk.
fn pairing_of(structure: &StructComparison) -> &'static str {
    if structure.source_name == structure.target_name {
        if structure.source_guid == structure.target_guid { "name+guid" } else { "name" }
    } else if structure.source_guid == structure.target_guid {
        "guid"
    } else {
        "parent_field"
    }
}

/// Everything one profile pair contributes.
struct PairReport {
    source_game: String,
    target_game: String,
    groups: Vec<GroupRow>,
}

struct GroupRow {
    group: String,
    source_fourcc: Option<String>,
    target_fourcc: Option<String>,
    verdict: CompatVerdict,
    shared_structs: usize,
    field_diff_structs: usize,
    size_diff_structs: usize,
    source_only_fields: usize,
    target_only_fields: usize,
    option_loss_fields: usize,
    blocked_reason: Option<String>,
    structs: Vec<StructRow>,
    fields: Vec<FieldRow>,
}

/// Derive the whole report for one ordered profile pair.
pub fn analyze_pair(
    definitions: &Path,
    catalog: &ReviewedCatalogHandle,
    source_game: &str,
    target_game: &str,
) -> Result<PairReportHandle, String> {
    let source_groups = groups_of(definitions, source_game)?;
    let target_groups = groups_of(definitions, target_game)?;

    let mut groups = Vec::new();
    // Every group either side defines, so the sheet is complete rather than
    // silently omitting what only one game has.
    for group in source_groups.union(&target_groups).cloned().collect::<BTreeSet<_>>() {
        let in_source = source_groups.contains(&group);
        let in_target = target_groups.contains(&group);
        let source_fourcc = fourcc_of(definitions, source_game, &group);
        let target_fourcc = fourcc_of(definitions, target_game, &group);

        if !in_source || !in_target {
            let missing = if in_source { target_game } else { source_game };
            groups.push(GroupRow {
                group,
                source_fourcc,
                target_fourcc,
                verdict: CompatVerdict::HardBlocked,
                shared_structs: 0,
                field_diff_structs: 0,
                size_diff_structs: 0,
                source_only_fields: 0,
                target_only_fields: 0,
                option_loss_fields: 0,
                blocked_reason: Some(format!("{missing} does not define this group")),
                structs: Vec::new(),
                fields: Vec::new(),
            });
            continue;
        }

        if let Some(reason) = catalog.0.refusal(&group, source_game, target_game) {
            groups.push(GroupRow {
                group,
                source_fourcc,
                target_fourcc,
                verdict: CompatVerdict::HardBlocked,
                shared_structs: 0,
                field_diff_structs: 0,
                size_diff_structs: 0,
                source_only_fields: 0,
                target_only_fields: 0,
                option_loss_fields: 0,
                blocked_reason: Some(reason),
                structs: Vec::new(),
                fields: Vec::new(),
            });
            continue;
        }

        // A malformed dumped definition can panic while the layout is built.
        // Catch it and record the group as blocked rather than losing the whole
        // run to one bad schema.
        let path = |game: &str| definitions.join(game).join(format!("{group}.json"));
        let built = std::panic::catch_unwind(|| {
            Ok::<_, String>((
                TagFile::new(path(source_game)).map_err(|e| e.to_string())?,
                TagFile::new(path(target_game)).map_err(|e| e.to_string())?,
            ))
        });
        let (source, target) = match built {
            Ok(Ok(pair)) => pair,
            Ok(Err(error)) => {
                groups.push(blocked_group(group, source_fourcc, target_fourcc, error));
                continue;
            }
            Err(_) => {
                groups.push(blocked_group(
                    group,
                    source_fourcc,
                    target_fourcc,
                    "the dumped definition cannot be built into a layout".to_owned(),
                ));
                continue;
            }
        };

        let mut aliases = AliasIndex::from_schema_json(&path(source_game));
        aliases.extend(AliasIndex::from_schema_json(&path(target_game)));
        let comparison = blam_tags::compare_group_layouts(
            source.definitions().root_struct(),
            target.definitions().root_struct(),
            &aliases,
        );
        let renames = catalog.0.renames(&group, source_game, target_game);
        let (structs, fields) = rows_for(&comparison, &renames);

        let size_diff_structs = structs.iter().filter(|s| s.source_size != s.target_size).count();
        let field_diff_structs = structs
            .iter()
            .filter(|s| s.verdict != CompatVerdict::Identical)
            .count();
        let count = |want: CompatVerdict| fields.iter().filter(|f| f.verdict == want).count();

        groups.push(GroupRow {
            group,
            source_fourcc,
            target_fourcc,
            verdict: severity_verdict(comparison.severity)
                .min(fields.iter().map(|f| f.verdict).min().unwrap_or(CompatVerdict::Identical)),
            shared_structs: structs.len(),
            field_diff_structs,
            size_diff_structs,
            source_only_fields: count(CompatVerdict::SourceOnly),
            target_only_fields: count(CompatVerdict::TargetOnly),
            option_loss_fields: count(CompatVerdict::OptionLoss),
            blocked_reason: None,
            structs,
            fields,
        });
    }

    Ok(PairReportHandle(PairReport {
        source_game: source_game.to_owned(),
        target_game: target_game.to_owned(),
        groups,
    }))
}

fn blocked_group(
    group: String,
    source_fourcc: Option<String>,
    target_fourcc: Option<String>,
    reason: String,
) -> GroupRow {
    GroupRow {
        group,
        source_fourcc,
        target_fourcc,
        verdict: CompatVerdict::HardBlocked,
        shared_structs: 0,
        field_diff_structs: 0,
        size_diff_structs: 0,
        source_only_fields: 0,
        target_only_fields: 0,
        option_loss_fields: 0,
        blocked_reason: Some(reason),
        structs: Vec::new(),
        fields: Vec::new(),
    }
}

/// Opaque wrappers so the row types can stay private to this module.
pub struct ReviewedCatalogHandle(ReviewedCatalog);
pub struct PairReportHandle(PairReport);

impl ReviewedCatalogHandle {
    pub fn load(path: &Path) -> Self {
        Self(ReviewedCatalog::load(path))
    }
}

/// A digest over the canonical row stream.
///
/// The staleness test compares on this rather than on the file, because SQLite
/// page allocation is not byte-reproducible — a file comparison would fail for
/// reasons that have nothing to do with the data.
pub fn content_digest(reports: &[PairReportHandle]) -> String {
    let mut hasher = Sha256::new();
    for PairReportHandle(report) in reports {
        hasher.update(report.source_game.as_bytes());
        hasher.update(b"\0");
        hasher.update(report.target_game.as_bytes());
        hasher.update(b"\0");
        for group in &report.groups {
            hasher.update(group.group.as_bytes());
            hasher.update(group.verdict.as_str().as_bytes());
            hasher.update(group.blocked_reason.as_deref().unwrap_or("").as_bytes());
            for row in &group.structs {
                hasher.update(row.struct_key.as_bytes());
                hasher.update(row.source_size.to_le_bytes());
                hasher.update(row.target_size.to_le_bytes());
                hasher.update(row.verdict.as_str().as_bytes());
            }
            for row in &group.fields {
                hasher.update(row.struct_key.as_bytes());
                hasher.update(row.ordinal.to_le_bytes());
                hasher.update(row.source_name.as_deref().unwrap_or("").as_bytes());
                hasher.update(row.target_name.as_deref().unwrap_or("").as_bytes());
                hasher.update(row.verdict.as_str().as_bytes());
                hasher.update(row.detail.as_bytes());
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

pub fn create_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode=DELETE;
         CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE pairs(pair_id INTEGER PRIMARY KEY, source_game TEXT NOT NULL, target_game TEXT NOT NULL, UNIQUE(source_game,target_game));
         CREATE TABLE groups(
            pair_id INTEGER NOT NULL, group_name TEXT NOT NULL,
            source_fourcc TEXT, target_fourcc TEXT,
            verdict TEXT NOT NULL,
            shared_structs INTEGER NOT NULL, field_diff_structs INTEGER NOT NULL,
            size_diff_structs INTEGER NOT NULL,
            source_only_fields INTEGER NOT NULL, target_only_fields INTEGER NOT NULL,
            option_loss_fields INTEGER NOT NULL,
            blocked_reason TEXT,
            PRIMARY KEY(pair_id, group_name));
         CREATE TABLE structs(
            pair_id INTEGER NOT NULL, group_name TEXT NOT NULL, struct_key TEXT NOT NULL,
            first_path TEXT NOT NULL,
            source_struct TEXT NOT NULL, target_struct TEXT NOT NULL,
            source_guid TEXT NOT NULL, target_guid TEXT NOT NULL,
            source_size INTEGER NOT NULL, target_size INTEGER NOT NULL,
            pairing TEXT NOT NULL, verdict TEXT NOT NULL,
            PRIMARY KEY(pair_id, group_name, struct_key));
         CREATE TABLE fields(
            pair_id INTEGER NOT NULL, group_name TEXT NOT NULL, struct_key TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            source_name TEXT, source_type TEXT, target_name TEXT, target_type TEXT,
            verdict TEXT NOT NULL, rule TEXT NOT NULL, detail TEXT NOT NULL,
            PRIMARY KEY(pair_id, group_name, struct_key, ordinal));
         CREATE INDEX groups_by_verdict ON groups(pair_id, verdict);
         CREATE INDEX fields_by_verdict ON fields(pair_id, verdict);
         CREATE INDEX fields_by_group ON fields(pair_id, group_name);",
    )
}

/// Build the database for `pairs` and write it to `output`.
pub fn build_database(
    definitions: &Path,
    mappings: &Path,
    pairs: &[(String, String)],
    output: &Path,
) -> Result<Vec<PairReportHandle>, String> {
    let catalog = ReviewedCatalogHandle::load(mappings);
    let mut reports = Vec::new();
    for (source, target) in pairs {
        reports.push(analyze_pair(definitions, &catalog, source, target)?);
    }

    let temp = output.with_extension("sqlite3.tmp");
    if temp.exists() {
        fs::remove_file(&temp).map_err(|e| e.to_string())?;
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut connection = Connection::open(&temp).map_err(|e| e.to_string())?;
    create_schema(&connection).map_err(|e| e.to_string())?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    for (key, value) in [
        ("schema_version", SCHEMA_VERSION.to_string()),
        ("content_digest", content_digest(&reports)),
        ("definitions_commit", definitions_commit(definitions)),
    ] {
        transaction
            .execute("INSERT INTO meta(key,value) VALUES(?1,?2)", params![key, value])
            .map_err(|e| e.to_string())?;
    }

    for (pair_id, PairReportHandle(report)) in reports.iter().enumerate() {
        let pair_id = pair_id as i64;
        transaction
            .execute(
                "INSERT INTO pairs(pair_id,source_game,target_game) VALUES(?1,?2,?3)",
                params![pair_id, report.source_game, report.target_game],
            )
            .map_err(|e| e.to_string())?;
        for group in &report.groups {
            transaction.execute(
                "INSERT INTO groups(pair_id,group_name,source_fourcc,target_fourcc,verdict,shared_structs,field_diff_structs,size_diff_structs,source_only_fields,target_only_fields,option_loss_fields,blocked_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    pair_id, group.group, group.source_fourcc, group.target_fourcc,
                    group.verdict.as_str(), group.shared_structs as i64,
                    group.field_diff_structs as i64, group.size_diff_structs as i64,
                    group.source_only_fields as i64, group.target_only_fields as i64,
                    group.option_loss_fields as i64, group.blocked_reason,
                ],
            ).map_err(|e| e.to_string())?;
            for row in &group.structs {
                transaction.execute(
                    "INSERT INTO structs(pair_id,group_name,struct_key,first_path,source_struct,target_struct,source_guid,target_guid,source_size,target_size,pairing,verdict) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![
                        pair_id, group.group, row.struct_key, row.first_path,
                        row.source_struct, row.target_struct, row.source_guid, row.target_guid,
                        row.source_size as i64, row.target_size as i64,
                        row.pairing, row.verdict.as_str(),
                    ],
                ).map_err(|e| e.to_string())?;
            }
            for row in &group.fields {
                transaction.execute(
                    "INSERT INTO fields(pair_id,group_name,struct_key,ordinal,source_name,source_type,target_name,target_type,verdict,rule,detail) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![
                        pair_id, group.group, row.struct_key, row.ordinal as i64,
                        row.source_name, row.source_type, row.target_name, row.target_type,
                        row.verdict.as_str(), row.rule, row.detail,
                    ],
                ).map_err(|e| e.to_string())?;
            }
        }
    }
    transaction.commit().map_err(|e| e.to_string())?;
    drop(connection);
    fs::rename(&temp, output).map_err(|e| e.to_string())?;
    Ok(reports)
}

/// Informational only — the digest is the real staleness check.
fn definitions_commit(definitions: &Path) -> String {
    std::process::Command::new("git")
        .args(["-C"])
        .arg(definitions)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

/// Write the field-level sheet: one row per field, both directions, losses
/// first so the interesting part is at the top of the file.
pub fn write_csv(reports: &[PairReportHandle], output: &Path) -> Result<(), String> {
    let mut out = String::from(
        "source_game,target_game,group,struct,source_field,source_type,target_field,target_type,verdict,rule,detail\n",
    );
    let mut rows: Vec<(CompatVerdict, String)> = Vec::new();
    for PairReportHandle(report) in reports {
        for group in &report.groups {
            if let Some(reason) = &group.blocked_reason {
                rows.push((
                    CompatVerdict::HardBlocked,
                    csv_row(&[
                        &report.source_game,
                        &report.target_game,
                        &group.group,
                        "",
                        "",
                        "",
                        "",
                        "",
                        CompatVerdict::HardBlocked.as_str(),
                        "group",
                        reason,
                    ]),
                ));
                continue;
            }
            for row in &group.fields {
                rows.push((
                    row.verdict,
                    csv_row(&[
                        &report.source_game,
                        &report.target_game,
                        &group.group,
                        &row.struct_key,
                        row.source_name.as_deref().unwrap_or(""),
                        row.source_type.as_deref().unwrap_or(""),
                        row.target_name.as_deref().unwrap_or(""),
                        row.target_type.as_deref().unwrap_or(""),
                        row.verdict.as_str(),
                        row.rule,
                        &row.detail,
                    ]),
                ));
            }
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    for (_, row) in rows {
        out.push_str(&row);
        out.push('\n');
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(output, out).map_err(|e| e.to_string())
}

fn csv_row(cells: &[&str]) -> String {
    cells
        .iter()
        .map(|cell| {
            if cell.contains([',', '"', '\n']) {
                format!("\"{}\"", cell.replace('"', "\"\""))
            } else {
                (*cell).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Candidate `accepted_field_drops` entries for the reviewer.
///
/// Every field a conversion would silently drop, in the shape
/// `conversion_mappings.json` expects, so the review that unblocks a
/// fail-closed group is an edit rather than a transcription exercise.
pub fn suggest_drops(reports: &[PairReportHandle], group_filter: Option<&str>) -> String {
    let mut entries = Vec::new();
    for PairReportHandle(report) in reports {
        for group in &report.groups {
            if group_filter.is_some_and(|want| !group.group.eq_ignore_ascii_case(want)) {
                continue;
            }
            let paths: BTreeMap<&str, &str> = group
                .fields
                .iter()
                .filter(|row| row.verdict == CompatVerdict::SourceOnly)
                .filter_map(|row| {
                    Some((row.source_name.as_deref()?, row.struct_key.as_str()))
                })
                .collect();
            for (field, structure) in paths {
                entries.push(serde_json::json!({
                    "group": group.group,
                    "source_games": [report.source_game],
                    "target_games": [report.target_game],
                    "source_path": field,
                    "reason": format!(
                        "{} does not define `{field}` on {structure}; REVIEW: confirm the value is not needed.",
                        report.target_game,
                    ),
                }));
            }
        }
    }
    serde_json::to_string_pretty(&serde_json::json!({ "accepted_field_drops": entries }))
        .unwrap_or_default()
}

/// The repository paths a bare invocation uses.
pub fn default_paths() -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    (
        root.join("definitions"),
        root.join("mappings/conversion_mappings.json"),
        root.join("docs/tag_compat.sqlite3"),
        root.join("docs/tag-compat-reach-ce.csv"),
    )
}

#[cfg(test)]
#[path = "app/tests/tag_compat_build.rs"]
mod tests;
