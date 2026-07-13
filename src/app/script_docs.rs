//! Lazy, read-only access to the generated Halo Script documentation database.

use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

const SCRIPT_DOCS_FILE: &str = "script_docs.sqlite3";
const SCRIPT_DOCS_SCHEMA_VERSION: i64 = 1;

pub(super) const SCRIPT_DOC_GAMES: [(&str, &str); 7] = [
    ("haloce_mcc", "Halo: Combat Evolved"),
    ("halo2_mcc", "Halo 2"),
    ("halo2amp_mcc", "Halo 2: Anniversary Multiplayer"),
    ("halo3_mcc", "Halo 3"),
    ("halo3odst_mcc", "Halo 3: ODST"),
    ("haloreach_mcc", "Halo: Reach"),
    ("halo4_mcc", "Halo 4"),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScriptDocCategory {
    Functions,
    Globals,
    Types,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScriptDocNetworkFilter {
    All,
    Yes,
    Unknown,
    No,
}

impl ScriptDocNetworkFilter {
    fn query_value(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Yes => "yes",
            Self::Unknown => "unknown",
            Self::No => "no",
        }
    }
}

#[derive(Clone)]
pub(super) struct ScriptDocRow {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) kind: String,
    pub(super) summary: String,
}

pub(super) enum ScriptDocDetail {
    Function {
        name: String,
        overloads: Vec<FunctionOverload>,
        examples: Vec<ScriptExample>,
    },
    Global {
        name: String,
        value_type: String,
        signature: String,
        description: String,
    },
    Type {
        name: String,
        usages: Vec<TypeUsage>,
    },
}

pub(super) struct FunctionOverload {
    pub(super) return_type: String,
    pub(super) signature: String,
    pub(super) description: String,
    pub(super) network_safe: Option<String>,
}

pub(super) struct ScriptExample {
    pub(super) source_file: String,
    pub(super) source_line: i64,
    pub(super) code: String,
}

pub(super) struct TypeUsage {
    pub(super) role: String,
    pub(super) symbol_name: String,
    pub(super) signature: String,
}

enum ScriptDocsDatabase {
    Unloaded,
    Loaded(Connection),
    Failed(String),
}

pub(super) struct ScriptDocsUiState {
    database: ScriptDocsDatabase,
    pub(super) game: String,
    pub(super) category: ScriptDocCategory,
    pub(super) network_filter: ScriptDocNetworkFilter,
    pub(super) search: String,
    pub(super) selected: Option<String>,
    pub(super) rows: Vec<ScriptDocRow>,
    pub(super) detail: Option<ScriptDocDetail>,
    last_query: Option<(String, ScriptDocCategory, ScriptDocNetworkFilter, String)>,
}

impl Default for ScriptDocsUiState {
    fn default() -> Self {
        Self {
            database: ScriptDocsDatabase::Unloaded,
            game: SCRIPT_DOC_GAMES[0].0.to_owned(),
            category: ScriptDocCategory::Functions,
            network_filter: ScriptDocNetworkFilter::All,
            search: String::new(),
            selected: None,
            rows: Vec::new(),
            detail: None,
            last_query: None,
        }
    }
}

impl ScriptDocsUiState {
    pub(super) fn error(&self) -> Option<&str> {
        match &self.database {
            ScriptDocsDatabase::Failed(error) => Some(error),
            _ => None,
        }
    }

    pub(super) fn ensure_loaded(&mut self, docs_root: &Path) {
        if !matches!(self.database, ScriptDocsDatabase::Unloaded) {
            return;
        }
        let path = docs_root.join(SCRIPT_DOCS_FILE);
        self.database = match open_database(&path) {
            Ok(connection) => ScriptDocsDatabase::Loaded(connection),
            Err(error) => ScriptDocsDatabase::Failed(error),
        };
    }

    pub(super) fn invalidate(&mut self) {
        self.selected = None;
        self.detail = None;
        self.last_query = None;
    }

    pub(super) fn refresh(&mut self) {
        let query = (
            self.game.clone(),
            self.category,
            self.network_filter,
            self.search.clone(),
        );
        if self.last_query.as_ref() == Some(&query) {
            return;
        }
        let ScriptDocsDatabase::Loaded(connection) = &self.database else {
            return;
        };
        match query_rows(
            connection,
            &self.game,
            self.category,
            self.network_filter,
            &self.search,
        ) {
            Ok(rows) => {
                self.rows = rows;
                self.last_query = Some(query);
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|selected| self.rows.iter().all(|row| &row.key != selected))
                {
                    self.selected = None;
                    self.detail = None;
                }
            }
            Err(error) => self.database = ScriptDocsDatabase::Failed(error),
        }
    }

    pub(super) fn select(&mut self, key: String) {
        if self.selected.as_ref() == Some(&key) {
            return;
        }
        let ScriptDocsDatabase::Loaded(connection) = &self.database else {
            return;
        };
        match query_detail(connection, &self.game, self.category, &key) {
            Ok(detail) => {
                self.selected = Some(key);
                self.detail = detail;
            }
            Err(error) => self.database = ScriptDocsDatabase::Failed(error),
        }
    }
}

fn open_database(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not read script documentation schema: {error}"))?;
    if version
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok())
        != Some(SCRIPT_DOCS_SCHEMA_VERSION)
    {
        return Err(format!(
            "Unsupported script documentation schema in {} (expected version {}).",
            path.display(),
            SCRIPT_DOCS_SCHEMA_VERSION
        ));
    }
    Ok(connection)
}

fn query_rows(
    connection: &Connection,
    game: &str,
    category: ScriptDocCategory,
    network_filter: ScriptDocNetworkFilter,
    search: &str,
) -> Result<Vec<ScriptDocRow>, String> {
    let needle = search.trim().to_ascii_lowercase();
    let contains = format!("%{needle}%");
    let prefix = format!("{needle}%");
    let sql = match category {
        ScriptDocCategory::Functions => {
            "SELECT name, MIN(return_type),
                    COALESCE(NULLIF(MAX(description),''),'No description supplied.'),
                    CASE WHEN lower(name)=?2 THEN 0 WHEN lower(name) LIKE ?3 THEN 1 ELSE 2 END rank
             FROM functions f
             WHERE game_id=?1 AND (?2='' OR lower(name) LIKE ?4 OR lower(signature) LIKE ?4
                 OR lower(description) LIKE ?4 OR EXISTS(SELECT 1 FROM examples e WHERE e.game_id=f.game_id AND e.function_name=f.name AND lower(e.code) LIKE ?4))
               AND (?5='all'
                    OR (?5='yes' AND lower(trim(COALESCE(network_safe,''))) LIKE 'yes%')
                    OR (?5='no' AND lower(trim(COALESCE(network_safe,''))) LIKE 'no%')
                    OR (?5='unknown' AND lower(trim(COALESCE(network_safe,''))) NOT LIKE 'yes%'
                                      AND lower(trim(COALESCE(network_safe,''))) NOT LIKE 'no%'))
             GROUP BY name ORDER BY rank,name COLLATE NOCASE"
        }
        ScriptDocCategory::Globals => {
            "SELECT name,value_type,COALESCE(NULLIF(description,''),signature),
                    CASE WHEN lower(name)=?2 THEN 0 WHEN lower(name) LIKE ?3 THEN 1 ELSE 2 END rank
             FROM globals WHERE game_id=?1 AND ?5=?5 AND (?2='' OR lower(name) LIKE ?4 OR lower(signature) LIKE ?4 OR lower(description) LIKE ?4)
             ORDER BY rank,name COLLATE NOCASE"
        }
        ScriptDocCategory::Types => {
            "SELECT t.name,COUNT(u.symbol_name),printf('%d documented usages',COUNT(u.symbol_name)),
                    CASE WHEN lower(t.name)=?2 THEN 0 WHEN lower(t.name) LIKE ?3 THEN 1 ELSE 2 END rank
             FROM types t LEFT JOIN type_usages u ON u.game_id=t.game_id AND u.type_name=t.name
             WHERE t.game_id=?1 AND ?5=?5 AND (?2='' OR lower(t.name) LIKE ?4 OR lower(u.symbol_name) LIKE ?4 OR lower(u.signature) LIKE ?4)
             GROUP BY t.name ORDER BY rank,t.name COLLATE NOCASE"
        }
    };
    let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![game, needle, prefix, contains, network_filter.query_value()],
            |row| {
                let name: String = row.get(0)?;
                Ok(ScriptDocRow {
                    key: name.clone(),
                    name,
                    kind: match category {
                        ScriptDocCategory::Types => {
                            let count: i64 = row.get(1)?;
                            format!("{count} uses")
                        }
                        _ => row.get(1)?,
                    },
                    summary: row.get(2)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn query_detail(
    connection: &Connection,
    game: &str,
    category: ScriptDocCategory,
    key: &str,
) -> Result<Option<ScriptDocDetail>, String> {
    match category {
        ScriptDocCategory::Functions => {
            let mut statement = connection.prepare("SELECT return_type,signature,description,network_safe FROM functions WHERE game_id=?1 AND name=?2 ORDER BY source_order").map_err(|error| error.to_string())?;
            let overloads = statement.query_map(params![game,key], |row| Ok(FunctionOverload { return_type: row.get(0)?, signature: row.get(1)?, description: row.get(2)?, network_safe: row.get(3)? })).map_err(|error| error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
            if overloads.is_empty() { return Ok(None); }
            let mut statement = connection.prepare("SELECT source_file,source_line,code FROM examples WHERE game_id=?1 AND function_name=?2 ORDER BY source_file,source_line LIMIT 3").map_err(|error| error.to_string())?;
            let examples = statement.query_map(params![game,key], |row| Ok(ScriptExample { source_file: row.get(0)?, source_line: row.get(1)?, code: row.get(2)? })).map_err(|error| error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
            Ok(Some(ScriptDocDetail::Function { name: key.to_owned(), overloads, examples }))
        }
        ScriptDocCategory::Globals => connection.query_row("SELECT name,value_type,signature,description FROM globals WHERE game_id=?1 AND name=?2", params![game,key], |row| Ok(ScriptDocDetail::Global { name: row.get(0)?, value_type: row.get(1)?, signature: row.get(2)?, description: row.get(3)? })).optional().map_err(|error| error.to_string()),
        ScriptDocCategory::Types => {
            let mut statement = connection.prepare("SELECT role,symbol_name,signature FROM type_usages WHERE game_id=?1 AND type_name=?2 ORDER BY role,symbol_name COLLATE NOCASE LIMIT 500").map_err(|error| error.to_string())?;
            let usages = statement.query_map(params![game,key], |row| Ok(TypeUsage { role: row.get(0)?, symbol_name: row.get(1)?, signature: row.get(2)? })).map_err(|error| error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
            Ok(Some(ScriptDocDetail::Type { name: key.to_owned(), usages }))
        }
    }
}

#[cfg(test)]
#[path = "tests/script_docs_runtime.rs"]
mod tests;
