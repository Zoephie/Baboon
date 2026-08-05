//! Lazy, read-only access to the generated cross-game tag compatibility
//! database.
//! It owns querying and presenting what the schemas say; deriving it belongs to
//! `crate::tag_compat_build` and acting on it to `app::conversion`.

use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

const TAG_COMPAT_FILE: &str = "tag_compat.sqlite3";
const TAG_COMPAT_SCHEMA_VERSION: i64 = 1;

/// What happens to a field, struct or group crossing a profile pair. Mirrors
/// the generator's `CompatVerdict`; kept separate so the read side does not
/// depend on the build side, which pulls in `rusqlite` write paths and the
/// whole schema walker.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::app) enum CompatVerdict {
    HardBlocked,
    SourceOnly,
    OptionLoss,
    TypeChangedSafe,
    RenamedProvable,
    TargetOnly,
    Identical,
}

impl CompatVerdict {
    fn parse(value: &str) -> Self {
        match value {
            "hard_blocked" => Self::HardBlocked,
            "source_only" => Self::SourceOnly,
            "option_loss" => Self::OptionLoss,
            "type_changed_safe" => Self::TypeChangedSafe,
            "renamed_provable" => Self::RenamedProvable,
            "target_only" => Self::TargetOnly,
            _ => Self::Identical,
        }
    }

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::HardBlocked => "blocked",
            Self::SourceOnly => "dropped",
            Self::OptionLoss => "options lost",
            Self::TypeChangedSafe => "re-encoded",
            Self::RenamedProvable => "renamed",
            Self::TargetOnly => "default",
            Self::Identical => "identical",
        }
    }

    pub(in crate::app) fn explain(self) -> &'static str {
        match self {
            Self::HardBlocked => "cannot be converted",
            Self::SourceOnly => "dropped — the target has no such field",
            Self::OptionLoss => "some options have no counterpart",
            Self::TypeChangedSafe => "the same value, re-encoded",
            Self::RenamedProvable => "renamed, and the rename is recorded",
            Self::TargetOnly => "left at its default — the source has no such field",
            Self::Identical => "transfers unchanged",
        }
    }

    /// Whether this costs the author anything. Drives the default filter: a
    /// reader opening the window wants the losses, not the 30,000 rows that
    /// transfer fine.
    pub(in crate::app) fn is_loss(self) -> bool {
        matches!(self, Self::HardBlocked | Self::SourceOnly | Self::OptionLoss)
    }

    pub(in crate::app) fn color(self) -> Color32 {
        match self {
            Self::HardBlocked => material_delete_text(),
            Self::SourceOnly | Self::OptionLoss => Color32::from_rgb(242, 196, 48),
            Self::Identical => disclosure_triangle_green(),
            _ => subtle_dark(),
        }
    }
}

/// An ordered profile pair the database covers.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct CompatPair {
    pub(in crate::app) id: i64,
    pub(in crate::app) source_game: String,
    pub(in crate::app) target_game: String,
}

impl CompatPair {
    pub(in crate::app) fn label(&self) -> String {
        format!("{} → {}", self.source_game, self.target_game)
    }
}

#[derive(Clone)]
pub(in crate::app) struct CompatGroupRow {
    pub(in crate::app) group: String,
    pub(in crate::app) verdict: CompatVerdict,
    pub(in crate::app) size_diff_structs: i64,
    pub(in crate::app) source_only_fields: i64,
    pub(in crate::app) target_only_fields: i64,
    pub(in crate::app) blocked_reason: Option<String>,
}

#[derive(Clone)]
pub(in crate::app) struct CompatFieldRow {
    pub(in crate::app) struct_key: String,
    pub(in crate::app) first_path: String,
    pub(in crate::app) source_name: Option<String>,
    pub(in crate::app) source_type: Option<String>,
    pub(in crate::app) target_name: Option<String>,
    pub(in crate::app) target_type: Option<String>,
    pub(in crate::app) verdict: CompatVerdict,
    pub(in crate::app) rule: String,
    pub(in crate::app) detail: String,
}

enum CompatDatabase {
    Unloaded,
    Loaded(Connection),
    Failed(String),
}

pub(in crate::app) struct TagCompatUiState {
    database: CompatDatabase,
    pub(in crate::app) pairs: Vec<CompatPair>,
    pub(in crate::app) pair: usize,
    pub(in crate::app) losses_only: bool,
    pub(in crate::app) search: String,
    pub(in crate::app) selected_group: Option<String>,
    pub(in crate::app) groups: Vec<CompatGroupRow>,
    pub(in crate::app) fields: Vec<CompatFieldRow>,
    last_query: Option<(usize, bool, String)>,
    last_group: Option<(usize, String, bool)>,
}

impl Default for TagCompatUiState {
    fn default() -> Self {
        Self {
            database: CompatDatabase::Unloaded,
            pairs: Vec::new(),
            pair: 0,
            // A reader opens this to find out what they will lose. Thirty
            // thousand rows that transfer fine are not the answer.
            losses_only: true,
            search: String::new(),
            selected_group: None,
            groups: Vec::new(),
            fields: Vec::new(),
            last_query: None,
            last_group: None,
        }
    }
}

impl TagCompatUiState {
    pub(in crate::app) fn error(&self) -> Option<&str> {
        match &self.database {
            CompatDatabase::Failed(error) => Some(error),
            _ => None,
        }
    }

    pub(in crate::app) fn ensure_loaded(&mut self, docs_root: &Path) {
        if !matches!(self.database, CompatDatabase::Unloaded) {
            return;
        }
        let path = docs_root.join(TAG_COMPAT_FILE);
        self.database = match open_database(&path) {
            Ok(connection) => {
                match query_pairs(&connection) {
                    Ok(pairs) => {
                        self.pairs = pairs;
                        CompatDatabase::Loaded(connection)
                    }
                    Err(error) => CompatDatabase::Failed(error),
                }
            }
            Err(error) => CompatDatabase::Failed(error),
        };
    }

    /// Point the window at a specific pair and group — the hook the import and
    /// conversion dialogs use so "what transfers for this group?" lands on the
    /// answer rather than on a search box.
    pub(in crate::app) fn focus(&mut self, source_game: &str, target_game: &str, group: &str) {
        if let Some(index) = self
            .pairs
            .iter()
            .position(|pair| pair.source_game == source_game && pair.target_game == target_game)
        {
            self.pair = index;
        }
        // A focused group is one the caller already knows is interesting, so
        // show all of it rather than only its losses.
        self.losses_only = false;
        self.search = group.to_owned();
        self.selected_group = Some(group.to_owned());
        self.last_query = None;
        self.last_group = None;
    }

    pub(in crate::app) fn refresh(&mut self) {
        let query = (self.pair, self.losses_only, self.search.clone());
        if self.last_query.as_ref() != Some(&query) {
            let CompatDatabase::Loaded(connection) = &self.database else {
                return;
            };
            let Some(pair) = self.pairs.get(self.pair) else {
                return;
            };
            match query_groups(connection, pair.id, self.losses_only, &self.search) {
                Ok(groups) => {
                    self.groups = groups;
                    self.last_query = Some(query);
                    if self
                        .selected_group
                        .as_ref()
                        .is_some_and(|name| self.groups.iter().all(|row| &row.group != name))
                    {
                        self.selected_group = None;
                        self.fields.clear();
                    }
                }
                Err(error) => {
                    self.database = CompatDatabase::Failed(error);
                    return;
                }
            }
        }

        let Some(group) = self.selected_group.clone() else {
            self.fields.clear();
            return;
        };
        let key = (self.pair, group.clone(), self.losses_only);
        if self.last_group.as_ref() == Some(&key) {
            return;
        }
        let CompatDatabase::Loaded(connection) = &self.database else {
            return;
        };
        let Some(pair) = self.pairs.get(self.pair) else {
            return;
        };
        match query_fields(connection, pair.id, &group, self.losses_only) {
            Ok(fields) => {
                self.fields = fields;
                self.last_group = Some(key);
            }
            Err(error) => self.database = CompatDatabase::Failed(error),
        }
    }

    pub(in crate::app) fn select_group(&mut self, group: String) {
        if self.selected_group.as_ref() == Some(&group) {
            return;
        }
        self.selected_group = Some(group);
        self.last_group = None;
    }

    /// The currently visible rows as CSV — the "sheet" a reader takes away.
    /// Exports what is on screen, filters included, so what they get is what
    /// they were looking at.
    pub(in crate::app) fn visible_csv(&self) -> String {
        let pair = self.pairs.get(self.pair);
        let (source, target) = pair
            .map(|pair| (pair.source_game.as_str(), pair.target_game.as_str()))
            .unwrap_or(("", ""));
        let mut out = String::from(
            "source_game,target_game,group,struct,path,source_field,source_type,\
             target_field,target_type,verdict,rule,detail\n",
        );
        let group = self.selected_group.as_deref().unwrap_or("");
        for row in &self.fields {
            let cells = [
                source,
                target,
                group,
                &row.struct_key,
                &row.first_path,
                row.source_name.as_deref().unwrap_or(""),
                row.source_type.as_deref().unwrap_or(""),
                row.target_name.as_deref().unwrap_or(""),
                row.target_type.as_deref().unwrap_or(""),
                row.verdict.label(),
                &row.rule,
                &row.detail,
            ];
            out.push_str(&csv_row(&cells));
            out.push('\n');
        }
        out
    }
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

fn open_database(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let version: Option<String> = connection
        .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |row| row.get(0))
        .optional()
        .map_err(|error| format!("Could not read the tag compatibility schema: {error}"))?;
    if version.as_deref().and_then(|value| value.parse::<i64>().ok())
        != Some(TAG_COMPAT_SCHEMA_VERSION)
    {
        return Err(format!(
            "Unsupported tag compatibility schema in {} (expected version {}). \
             Rebuild it with `cargo run --bin build_tag_compat`.",
            path.display(),
            TAG_COMPAT_SCHEMA_VERSION,
        ));
    }
    Ok(connection)
}

fn query_pairs(connection: &Connection) -> Result<Vec<CompatPair>, String> {
    let mut statement = connection
        .prepare("SELECT pair_id,source_game,target_game FROM pairs ORDER BY pair_id")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(CompatPair {
                id: row.get(0)?,
                source_game: row.get(1)?,
                target_game: row.get(2)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn query_groups(
    connection: &Connection,
    pair: i64,
    losses_only: bool,
    search: &str,
) -> Result<Vec<CompatGroupRow>, String> {
    let mut statement = connection
        .prepare(
            "SELECT group_name,verdict,size_diff_structs,source_only_fields,target_only_fields,blocked_reason
             FROM groups
             WHERE pair_id=?1
               AND (?2=0 OR verdict IN ('hard_blocked','source_only','option_loss'))
               AND (?3='' OR group_name LIKE '%'||?3||'%')
             ORDER BY group_name",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![pair, losses_only as i64, search.trim()], |row| {
            Ok(CompatGroupRow {
                group: row.get(0)?,
                verdict: CompatVerdict::parse(&row.get::<_, String>(1)?),
                size_diff_structs: row.get(2)?,
                source_only_fields: row.get(3)?,
                target_only_fields: row.get(4)?,
                blocked_reason: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn query_fields(
    connection: &Connection,
    pair: i64,
    group: &str,
    losses_only: bool,
) -> Result<Vec<CompatFieldRow>, String> {
    let mut statement = connection
        .prepare(
            "SELECT f.struct_key,COALESCE(s.first_path,f.struct_key),
                    f.source_name,f.source_type,f.target_name,f.target_type,
                    f.verdict,f.rule,f.detail
             FROM fields f
             LEFT JOIN structs s
               ON s.pair_id=f.pair_id AND s.group_name=f.group_name AND s.struct_key=f.struct_key
             WHERE f.pair_id=?1 AND f.group_name=?2
               AND (?3=0 OR f.verdict IN ('hard_blocked','source_only','option_loss'))
             ORDER BY s.first_path,f.ordinal",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![pair, group, losses_only as i64], |row| {
            Ok(CompatFieldRow {
                struct_key: row.get(0)?,
                first_path: row.get(1)?,
                source_name: row.get(2)?,
                source_type: row.get(3)?,
                target_name: row.get(4)?,
                target_type: row.get(5)?,
                verdict: CompatVerdict::parse(&row.get::<_, String>(6)?),
                rule: row.get(7)?,
                detail: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "tests/tag_compat_runtime.rs"]
mod tests;
