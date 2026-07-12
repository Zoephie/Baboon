//! Compile-time access to the generated editing-kit command catalogs.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

pub fn get_tool_commands_json(game: &str) -> Option<&'static str> {
    match game {
        "haloce_mcc" => Some(include_str!("../tool_commands/haloce_mcc.json")),
        "halo2_mcc" => Some(include_str!("../tool_commands/halo2_mcc.json")),
        "halo3_mcc" | "halo3odst_mcc" => Some(include_str!("../tool_commands/halo3_mcc.json")),
        "haloreach_mcc" => Some(include_str!("../tool_commands/haloreach_mcc.json")),
        "halo4_mcc" => Some(include_str!("../tool_commands/halo4_mcc.json")),
        _ => None,
    }
}
