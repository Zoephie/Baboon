//! Which scenario palette a tag group lands in, per game, read from that game's scenario definition.
//! It owns the palette table only; how a tag reaches a palette (a Sapien drop, an edit) belongs to the controller.

use super::*;

/// One top-level palette block of a game's scenario: its name and the groups
/// its entries may reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct ScenarioPalette {
    /// The block's name with its annotations stripped, e.g. `vehicle palette`.
    pub(in crate::app) name: String,
    /// Group tags the palette's entry reference allows, e.g. `vehi`. Empty
    /// for a palette whose entries hold no tag reference (weather, acoustics),
    /// and for Halo CE and Halo 2, whose definitions do not record what a
    /// palette entry may reference; neither of those Sapiens takes a dropped
    /// file, so nothing asks.
    pub(in crate::app) groups: Vec<u32>,
}

/// Every top-level palette block of `game`'s scenario, in definition order.
pub(in crate::app) fn scenario_palettes(
    definitions_root: &Path,
    game: GameId,
) -> Result<Vec<ScenarioPalette>, String> {
    let path = definitions_root.join(game.as_str()).join("scenario.json");
    let bytes =
        fs::read(&path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let definition: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
    palettes_in_definition(&definition).map_err(|what| format!("{}: {what}", path.display()))
}

/// The palettes that take `group_tag`, in definition order. Empty when the
/// game's scenario has no palette for the group.
pub(in crate::app) fn palettes_for_group(
    palettes: &[ScenarioPalette],
    group_tag: u32,
) -> Vec<&ScenarioPalette> {
    palettes
        .iter()
        .filter(|palette| palette.groups.contains(&group_tag))
        .collect()
}

/// The palette blocks among the root struct's own fields. Nested blocks are
/// not walked: a palette inside a map-variant or zone block is not what a
/// dropped tag joins.
fn palettes_in_definition(definition: &Value) -> Result<Vec<ScenarioPalette>, String> {
    let root = definition
        .get("block")
        .and_then(Value::as_str)
        .ok_or_else(|| "no root block".to_owned())?;
    let mut palettes = Vec::new();
    for field in block_fields(definition, root)? {
        if field.get("type").and_then(Value::as_str) != Some("block") {
            continue;
        }
        let Some(name) = field
            .get("name")
            .and_then(Value::as_str)
            .map(strip_annotations)
            .filter(|name| is_palette_name(name))
        else {
            continue;
        };
        let Some(block) = field.get("definition").and_then(Value::as_str) else {
            continue;
        };
        let groups = block_fields(definition, block)?
            .iter()
            .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("tag_reference"))
            .flat_map(|entry| {
                entry
                    .get("definition")
                    .and_then(|reference| reference.get("allowed"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(Value::as_str)
            .filter_map(group_tag_from_str)
            .collect();
        palettes.push(ScenarioPalette { name, groups });
    }
    Ok(palettes)
}

/// The fields of the struct a block is made of.
fn block_fields<'a>(definition: &'a Value, block: &str) -> Result<&'a Vec<Value>, String> {
    let struct_name = definition
        .get("blocks")
        .and_then(|blocks| blocks.get(block))
        .and_then(|block| block.get("struct"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("block {block} names no struct"))?;
    definition
        .get("structs")
        .and_then(|structs| structs.get(struct_name))
        .and_then(|layout| layout.get("fields"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("struct {struct_name} has no fields"))
}

/// A field name without Guerilla's inline annotations: `{alias}`, `#help`,
/// `!` and `*` markers, `^` block-name markers and `:units`.
fn strip_annotations(name: &str) -> String {
    let end = name
        .find(['{', '#', '!', '*', '^', ':'])
        .unwrap_or(name.len());
    name[..end].trim().to_owned()
}

/// Ends with the word "palette": `vehicle palette` yes, `map variant
/// palettes` no.
fn is_palette_name(name: &str) -> bool {
    name.rsplit(' ').next() == Some("palette")
}

fn group_tag_from_str(group: &str) -> Option<u32> {
    let bytes: [u8; 4] = group.as_bytes().try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests;
