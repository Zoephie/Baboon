//! Editing-kit command parsing, argument state, and command-line construction.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

#[derive(Clone, Debug)]
pub(in crate::app) struct ToolCommand {
    pub(in crate::app) name: String,
    pub(in crate::app) category: String,
    pub(in crate::app) description: String,
    pub(in crate::app) example: String,
    pub(in crate::app) args: Vec<ToolCommandArg>,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct ToolCommandArg {
    pub(in crate::app) name: String,
    pub(in crate::app) kind: ToolCommandArgKind,
    pub(in crate::app) description: String,
    pub(in crate::app) required: bool,
    pub(in crate::app) values: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ToolCommandArgKind {
    PathData,
    PathTag,
    PathFile,
    String,
    Enum,
    OptionalString,
}

#[derive(Default)]
pub(in crate::app) struct ToolCommandsUiState {
    pub(in crate::app) catalog_game: Option<GameId>,
    pub(in crate::app) commands: Vec<ToolCommand>,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) selected: Option<String>,
    pub(in crate::app) values: HashMap<String, String>,
    pub(in crate::app) optional_open: bool,
}

pub(in crate::app) fn load_tool_commands(game: GameId) -> Result<Vec<ToolCommand>, String> {
    let text = crate::core::tool_commands::get_tool_commands_json(game)
        .ok_or_else(|| format!("No tool command catalog is embedded for {game}"))?;
    parse_tool_commands_json(text).map_err(|error| {
        format!("Could not parse embedded tool command catalog for {game}: {error}")
    })
}

fn parse_tool_commands_json(text: &str) -> Result<Vec<ToolCommand>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let commands = value
        .get("commands")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing commands array".to_owned())?;
    let mut parsed = Vec::new();
    for command in commands {
        let name = json_string(command, "name")?;
        let category = json_string(command, "category")?;
        let description = json_string(command, "description").unwrap_or_default();
        let example = json_string(command, "example").unwrap_or_default();
        let args = command
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .map(parse_tool_command_arg)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        parsed.push(ToolCommand {
            name,
            category,
            description,
            example,
            args,
        });
    }
    Ok(parsed)
}

fn parse_tool_command_arg(value: &Value) -> Result<ToolCommandArg, String> {
    let kind = match json_string(value, "type")?.as_str() {
        "path_data" => ToolCommandArgKind::PathData,
        "path_tag" => ToolCommandArgKind::PathTag,
        "path_file" => ToolCommandArgKind::PathFile,
        "string" => ToolCommandArgKind::String,
        "enum" => ToolCommandArgKind::Enum,
        "optional_string" => ToolCommandArgKind::OptionalString,
        other => return Err(format!("unknown argument type {other:?}")),
    };
    let values = value
        .get("values")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(ToolCommandArg {
        name: json_string(value, "name")?,
        kind,
        description: json_string(value, "description").unwrap_or_default(),
        required: value
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        values,
    })
}

fn json_string(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string field {key:?}"))
}

pub(in crate::app) fn tool_command_preview(
    command: &ToolCommand,
    values: &HashMap<String, String>,
) -> String {
    let mut parts = vec!["tool".to_owned(), command.name.clone()];
    for arg in &command.args {
        let value = effective_arg_value(arg, values);
        if value.is_empty() {
            continue;
        }
        parts.push(value);
    }
    parts.join(" ")
}

pub(in crate::app) fn tool_command_missing_required(
    command: &ToolCommand,
    values: &HashMap<String, String>,
) -> Option<String> {
    command
        .args
        .iter()
        .find(|arg| arg.required && effective_arg_value(arg, values).trim().is_empty())
        .map(|arg| arg.name.clone())
}

pub(in crate::app) fn effective_arg_value(
    arg: &ToolCommandArg,
    values: &HashMap<String, String>,
) -> String {
    let key = tool_arg_key("", arg);
    let value = values.get(&key).map(String::as_str).unwrap_or("").trim();
    if value.is_empty() && arg.kind == ToolCommandArgKind::Enum {
        return arg.values.first().cloned().unwrap_or_default();
    }
    value.to_owned()
}

pub(in crate::app) fn tool_arg_key(command_name: &str, arg: &ToolCommandArg) -> String {
    if command_name.is_empty() {
        arg.name.clone()
    } else {
        format!("{command_name}:{}", arg.name)
    }
}

pub(in crate::app) fn path_arg_from_picker(
    path: &Path,
    base: Option<&Path>,
    strip_extension: bool,
) -> String {
    let rel = base
        .and_then(|base| path.strip_prefix(base).ok())
        .unwrap_or(path);
    let rel = if strip_extension {
        rel.with_extension("")
    } else {
        rel.to_path_buf()
    };
    rel.to_string_lossy().replace('/', "\\")
}

#[cfg(test)]
mod tests;
