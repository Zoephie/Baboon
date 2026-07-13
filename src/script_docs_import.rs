//! Deterministic importer for Halo Script reference documents and examples.

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: i64 = 1;

const GAMES: [(&str, &str, &str); 7] = [
    ("h1", "haloce_mcc", "Halo: Combat Evolved"),
    ("h2", "halo2_mcc", "Halo 2"),
    ("h2amp", "halo2amp_mcc", "Halo 2: Anniversary Multiplayer"),
    ("h3", "halo3_mcc", "Halo 3"),
    ("h3odst", "halo3odst_mcc", "Halo 3: ODST"),
    ("hreach", "haloreach_mcc", "Halo: Reach"),
    ("h4", "halo4_mcc", "Halo 4"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionDoc {
    pub name: String,
    pub return_type: String,
    pub signature: String,
    pub description: String,
    pub network_safe: Option<String>,
    pub types: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalDoc {
    pub name: String,
    pub value_type: String,
    pub signature: String,
    pub description: String,
}

#[derive(Default)]
pub struct ParsedDocs {
    pub functions: Vec<FunctionDoc>,
    pub globals: Vec<GlobalDoc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ScriptExample {
    pub function_name: String,
    pub source_file: String,
    pub source_line: usize,
    pub code: String,
}

pub fn parse_document(text: &str) -> ParsedDocs {
    let mut parsed = ParsedDocs::default();
    let mut globals = false;
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.eq_ignore_ascii_case("; AVAILABLE EXTERNAL GLOBALS:") {
            globals = true;
            index += 1;
            continue;
        }
        if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
            index += 1;
            continue;
        }
        let signature = trimmed.to_owned();
        let mut comments = Vec::new();
        index += 1;
        while index < lines.len() {
            let next = lines[index].trim();
            if next.starts_with('(') || next.eq_ignore_ascii_case("; AVAILABLE EXTERNAL GLOBALS:") {
                break;
            }
            if let Some(comment) = next.strip_prefix(';') {
                let comment = comment.trim();
                if !comment.is_empty() {
                    comments.push(comment.to_owned());
                }
            }
            index += 1;
        }
        let Some((value_type, name, types)) = parse_signature(&signature) else {
            continue;
        };
        let network_safe = comments.iter().find_map(|line| {
            line.strip_prefix("NETWORK SAFE:")
                .map(str::trim)
                .map(str::to_owned)
        });
        let description = comments
            .into_iter()
            .filter(|line| !line.starts_with("NETWORK SAFE:"))
            .collect::<Vec<_>>()
            .join(" ");
        if globals {
            parsed.globals.push(GlobalDoc {
                name,
                value_type,
                signature,
                description,
            });
        } else {
            parsed.functions.push(FunctionDoc {
                name,
                return_type: value_type,
                signature,
                description,
                network_safe,
                types,
            });
        }
    }
    parsed
}

fn parse_signature(signature: &str) -> Option<(String, String, Vec<String>)> {
    let inner = signature.strip_prefix('(')?.strip_suffix(')')?.trim();
    let return_end = inner.find('>')?;
    let return_type = inner
        .strip_prefix('<')?
        .get(..return_end - 1)?
        .trim()
        .to_owned();
    let rest = inner.get(return_end + 1..)?.trim_start();
    let name_end = rest
        .find(|character: char| character.is_whitespace() || character == ')')
        .unwrap_or(rest.len());
    let name = rest.get(..name_end)?.trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let mut types = Vec::new();
    let mut remaining = inner;
    while let Some(start) = remaining.find('<') {
        remaining = &remaining[start + 1..];
        let Some(end) = remaining.find('>') else {
            break;
        };
        let value = remaining[..end].trim();
        if !value.is_empty() {
            types.push(value.to_owned());
        }
        remaining = &remaining[end + 1..];
    }
    Some((return_type, name, types))
}

pub fn extract_examples(
    text: &str,
    source_file: &str,
    documented: &BTreeSet<String>,
) -> Vec<ScriptExample> {
    let clean = strip_comments(text);
    let offsets = line_offsets(&clean);
    let bytes = clean.as_bytes();
    let mut candidates = BTreeSet::new();
    for start in 0..bytes.len() {
        if bytes[start] != b'(' {
            continue;
        }
        if let Some(end) = matching_paren(&clean, start) {
            let code = clean[start..=end].trim();
            if code.len() <= 600 && code.lines().count() <= 12 {
                if let Some(name) = lisp_call_name(code)
                    && documented.contains(name)
                {
                    candidates.insert(example(name, source_file, start, code, &offsets));
                }
            }
        }
    }
    for (name_start, paren_start, name) in c_style_calls(&clean) {
        if !documented.contains(name) || declaration_context(&clean, name_start) {
            continue;
        }
        if let Some(end) = matching_paren(&clean, paren_start) {
            let code = clean[name_start..=end].trim();
            if code.len() <= 600 && code.lines().count() <= 12 {
                candidates.insert(example(name, source_file, name_start, code, &offsets));
            }
        }
    }
    candidates.into_iter().collect()
}

fn example(name: &str, source: &str, offset: usize, code: &str, lines: &[usize]) -> ScriptExample {
    ScriptExample {
        function_name: name.to_owned(),
        source_file: source.to_owned(),
        source_line: lines.partition_point(|line| *line <= offset),
        code: code.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    offsets.extend(text.match_indices('\n').map(|(index, _)| index + 1));
    offsets
}

fn matching_paren(text: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (relative, character) in text[start..].char_indices() {
        if quoted {
            if escaped {
                escaped = false;
                continue;
            }
            if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(start + relative);
                }
            }
            _ => {}
        }
    }
    None
}

fn lisp_call_name(code: &str) -> Option<&str> {
    let rest = code.strip_prefix('(')?.trim_start();
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .unwrap_or(rest.len());
    let name = &rest[..end];
    (!name.is_empty()).then_some(name)
}

fn c_style_calls(text: &str) -> Vec<(usize, usize, &str)> {
    let bytes = text.as_bytes();
    let mut calls = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let name = &text[start..index];
            let mut cursor = index;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor < bytes.len() && bytes[cursor] == b'(' {
                calls.push((start, cursor, name));
            }
        } else {
            index += 1;
        }
    }
    calls
}

fn declaration_context(text: &str, name_start: usize) -> bool {
    let line_start = text[..name_start].rfind('\n').map_or(0, |value| value + 1);
    let prefix = text[line_start..name_start].trim_start();
    [
        "script ",
        "global ",
        "static ",
        "stub ",
        "dormant ",
        "command_script ",
    ]
    .iter()
    .any(|keyword| prefix.starts_with(keyword))
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    let mut quoted = false;
    let mut block = false;
    while index < chars.len() {
        if block {
            if index + 1 < chars.len() && chars[index] == '*' && chars[index + 1] == ';' {
                block = false;
                out.push(' ');
                out.push(' ');
                index += 2;
            } else {
                out.push(if chars[index] == '\n' { '\n' } else { ' ' });
                index += 1;
            }
            continue;
        }
        if !quoted && index + 1 < chars.len() && chars[index] == ';' && chars[index + 1] == '*' {
            block = true;
            out.push(' ');
            out.push(' ');
            index += 2;
            continue;
        }
        if !quoted && chars[index] == ';' {
            while index < chars.len() && chars[index] != '\n' {
                out.push(' ');
                index += 1;
            }
            continue;
        }
        if !quoted && index + 1 < chars.len() && chars[index] == '/' && chars[index + 1] == '/' {
            while index < chars.len() && chars[index] != '\n' {
                out.push(' ');
                index += 1;
            }
            continue;
        }
        if chars[index] == '"' {
            quoted = !quoted;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn build_database(source_root: &Path, output: &Path) -> Result<(), String> {
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
    transaction
        .execute(
            "INSERT INTO metadata(key,value) VALUES('schema_version',?1)",
            [SCHEMA_VERSION.to_string()],
        )
        .map_err(|e| e.to_string())?;

    for (sort_order, (folder, game_id, title)) in GAMES.iter().enumerate() {
        let directory = source_root.join(folder);
        let mut files = sorted_files(&directory)?;
        let doc_path = files
            .iter()
            .find(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
            })
            .cloned()
            .ok_or_else(|| format!("missing global document in {}", directory.display()))?;
        transaction
            .execute(
                "INSERT INTO games(id,title,sort_order) VALUES(?1,?2,?3)",
                params![game_id, title, sort_order as i64],
            )
            .map_err(|e| e.to_string())?;
        for path in &files {
            let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let relative = path.file_name().unwrap_or_default().to_string_lossy();
            let kind = if path == &doc_path {
                "document"
            } else {
                "example"
            };
            transaction
                .execute(
                    "INSERT INTO source_files(game_id,path,kind,sha256) VALUES(?1,?2,?3,?4)",
                    params![game_id, relative.as_ref(), kind, sha256_hex(&bytes)],
                )
                .map_err(|e| e.to_string())?;
        }
        let text = read_text_lossy(&doc_path)?;
        let docs = parse_document(&text);
        let names: BTreeSet<String> = docs.functions.iter().map(|doc| doc.name.clone()).collect();
        for (order, function) in docs.functions.iter().enumerate() {
            transaction.execute("INSERT INTO functions(game_id,name,return_type,signature,description,network_safe,source_order) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![game_id, function.name, function.return_type, function.signature, function.description, function.network_safe, order as i64]).map_err(|e| e.to_string())?;
            for (role_order, value_type) in function.types.iter().enumerate() {
                insert_type_usage(
                    &transaction,
                    game_id,
                    value_type,
                    if role_order == 0 {
                        "return"
                    } else {
                        "parameter"
                    },
                    &function.name,
                    &function.signature,
                )?;
            }
        }
        for (order, global) in docs.globals.iter().enumerate() {
            transaction.execute("INSERT INTO globals(game_id,name,value_type,signature,description,source_order) VALUES(?1,?2,?3,?4,?5,?6)", params![game_id, global.name, global.value_type, global.signature, global.description, order as i64]).map_err(|e| e.to_string())?;
            insert_type_usage(
                &transaction,
                game_id,
                &global.value_type,
                "global",
                &global.name,
                &global.signature,
            )?;
        }
        let mut examples: BTreeMap<String, Vec<ScriptExample>> = BTreeMap::new();
        files.retain(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("hsc"))
        });
        for path in files {
            let text = read_text_lossy(&path)?;
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            for example in extract_examples(&text, &filename, &names) {
                let values = examples.entry(example.function_name.clone()).or_default();
                if values.iter().all(|value| value.code != example.code) && values.len() < 3 {
                    values.push(example);
                }
            }
        }
        for values in examples.values() {
            for example in values {
                transaction.execute("INSERT INTO examples(game_id,function_name,source_file,source_line,code) VALUES(?1,?2,?3,?4,?5)", params![game_id, example.function_name, example.source_file, example.source_line as i64, example.code]).map_err(|e| e.to_string())?;
            }
        }
    }
    transaction.commit().map_err(|e| e.to_string())?;
    connection
        .execute_batch("ANALYZE; VACUUM;")
        .map_err(|e| e.to_string())?;
    drop(connection);
    if output.exists() {
        fs::remove_file(output).map_err(|e| e.to_string())?;
    }
    fs::rename(&temp, output).map_err(|e| e.to_string())
}

fn read_text_lossy(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn sorted_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = fs::read_dir(directory)
        .map_err(|e| format!("{}: {e}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
    });
    Ok(files)
}

fn insert_type_usage(
    connection: &Connection,
    game: &str,
    value_type: &str,
    role: &str,
    symbol: &str,
    signature: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR IGNORE INTO types(game_id,name) VALUES(?1,?2)",
            params![game, value_type],
        )
        .map_err(|e| e.to_string())?;
    connection.execute("INSERT OR IGNORE INTO type_usages(game_id,type_name,role,symbol_name,signature) VALUES(?1,?2,?3,?4,?5)", params![game, value_type, role, symbol, signature]).map_err(|e| e.to_string())?;
    Ok(())
}

fn create_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode=DELETE;
         CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
         CREATE TABLE games(id TEXT PRIMARY KEY,title TEXT NOT NULL,sort_order INTEGER NOT NULL);
         CREATE TABLE functions(id INTEGER PRIMARY KEY,game_id TEXT NOT NULL,name TEXT NOT NULL,return_type TEXT NOT NULL,signature TEXT NOT NULL,description TEXT NOT NULL,network_safe TEXT,source_order INTEGER NOT NULL);
         CREATE INDEX functions_game_name ON functions(game_id,name COLLATE NOCASE);
         CREATE TABLE globals(id INTEGER PRIMARY KEY,game_id TEXT NOT NULL,name TEXT NOT NULL,value_type TEXT NOT NULL,signature TEXT NOT NULL,description TEXT NOT NULL,source_order INTEGER NOT NULL);
         CREATE INDEX globals_game_name ON globals(game_id,name COLLATE NOCASE);
         CREATE TABLE types(game_id TEXT NOT NULL,name TEXT NOT NULL,PRIMARY KEY(game_id,name));
         CREATE TABLE type_usages(game_id TEXT NOT NULL,type_name TEXT NOT NULL,role TEXT NOT NULL,symbol_name TEXT NOT NULL,signature TEXT NOT NULL,UNIQUE(game_id,type_name,role,symbol_name,signature));
         CREATE INDEX type_usage_lookup ON type_usages(game_id,type_name,symbol_name);
         CREATE TABLE examples(game_id TEXT NOT NULL,function_name TEXT NOT NULL,source_file TEXT NOT NULL,source_line INTEGER NOT NULL,code TEXT NOT NULL,UNIQUE(game_id,function_name,source_file,source_line,code));
         CREATE INDEX examples_lookup ON examples(game_id,function_name COLLATE NOCASE);
         CREATE TABLE source_files(game_id TEXT NOT NULL,path TEXT NOT NULL,kind TEXT NOT NULL,sha256 TEXT NOT NULL,PRIMARY KEY(game_id,path));"
    )
}

#[cfg(test)]
#[path = "app/tests/script_docs_import.rs"]
mod tests;
