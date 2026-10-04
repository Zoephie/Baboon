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
    let h3 = crate::core::tool_commands::get_tool_commands_json("halo3_mcc").unwrap();
    let odst = crate::core::tool_commands::get_tool_commands_json("halo3odst_mcc").unwrap();

    assert_eq!(h3, odst);
}

#[test]
fn halo4_has_empty_embedded_catalog() {
    let commands = load_tool_commands("halo4_mcc").unwrap();

    assert!(commands.is_empty());
}
