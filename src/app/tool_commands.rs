//! Editing-kit command parsing, argument state, and command-line construction.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

#[derive(Clone, Debug)]
pub(super) struct ToolCommand {
    pub(super) name: String,
    pub(super) category: String,
    pub(super) description: String,
    pub(super) example: String,
    pub(super) args: Vec<ToolCommandArg>,
}

#[derive(Clone, Debug)]
pub(super) struct ToolCommandArg {
    pub(super) name: String,
    pub(super) kind: ToolCommandArgKind,
    pub(super) description: String,
    pub(super) required: bool,
    pub(super) values: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolCommandArgKind {
    PathData,
    PathTag,
    PathFile,
    String,
    Enum,
    OptionalString,
}

#[derive(Default)]
pub(super) struct ToolCommandsUiState {
    pub(super) open: bool,
    pub(super) catalog_game: Option<String>,
    pub(super) commands: Vec<ToolCommand>,
    pub(super) error: Option<String>,
    pub(super) selected: Option<String>,
    pub(super) values: HashMap<String, String>,
    pub(super) optional_open: bool,
}

pub(super) fn load_tool_commands(game: &str) -> Result<Vec<ToolCommand>, String> {
    let text = crate::tool_commands::get_tool_commands_json(game)
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

pub(super) fn tool_command_preview(
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

pub(super) fn tool_command_missing_required(
    command: &ToolCommand,
    values: &HashMap<String, String>,
) -> Option<String> {
    command
        .args
        .iter()
        .find(|arg| arg.required && effective_arg_value(arg, values).trim().is_empty())
        .map(|arg| arg.name.clone())
}

pub(super) fn effective_arg_value(
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

pub(super) fn tool_arg_key(command_name: &str, arg: &ToolCommandArg) -> String {
    if command_name.is_empty() {
        arg.name.clone()
    } else {
        format!("{command_name}:{}", arg.name)
    }
}

pub(super) fn path_arg_from_picker(
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
mod tests {
    use super::*;

    #[test]
    fn tool_command_preview_leaves_path_arguments_unquoted() {
        let command = ToolCommand {
            name: "bitmaps".to_owned(),
            category: "Bitmaps".to_owned(),
            description: String::new(),
            example: String::new(),
            args: vec![ToolCommandArg {
                name: "source-directory".to_owned(),
                kind: ToolCommandArgKind::PathData,
                description: String::new(),
                required: true,
                values: Vec::new(),
            }],
        };
        let mut values = HashMap::new();
        values.insert(
            "source-directory".to_owned(),
            "levels\\multi\\chill\\bitmaps".to_owned(),
        );

        assert_eq!(
            tool_command_preview(&command, &values),
            "tool bitmaps levels\\multi\\chill\\bitmaps"
        );
    }

    #[test]
    fn parses_generated_tool_command_json_shape() {
        let commands = parse_tool_commands_json(
            r#"{"commands":[{"name":"build-cache-file","category":"Cache Files","description":"Builds a map.","example":"tool build-cache-file test pc","args":[{"name":"platform","type":"enum","description":"The platform.","required":false,"values":["pc"]}]}]}"#,
        )
        .unwrap();

        assert_eq!(commands[0].name, "build-cache-file");
        assert_eq!(commands[0].args[0].kind, ToolCommandArgKind::Enum);
    }

    #[test]
    fn loads_generated_h3_tool_commands() {
        let commands = load_tool_commands("halo3_mcc").unwrap();
        let bitmaps = commands
            .iter()
            .find(|command| command.name == "bitmaps")
            .unwrap();

        assert_eq!(bitmaps.category, "Bitmaps");
        assert_eq!(bitmaps.args[0].kind, ToolCommandArgKind::PathData);
    }

    #[test]
    fn h3odst_reuses_h3_tool_commands() {
        let h3 = crate::tool_commands::get_tool_commands_json("halo3_mcc").unwrap();
        let odst = crate::tool_commands::get_tool_commands_json("halo3odst_mcc").unwrap();

        assert_eq!(h3, odst);
    }

    #[test]
    fn halo4_has_empty_embedded_catalog() {
        let commands = load_tool_commands("halo4_mcc").unwrap();

        assert!(commands.is_empty());
    }
}
