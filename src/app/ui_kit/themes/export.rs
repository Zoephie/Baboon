//! Writing themes out: a complete theme file for the theme in use, the JSON
//! schema an editor completes theme files from, and the authoring guide's
//! token reference. All three come from the token table.

use super::file::Report;
use super::library::{Resolved, ThemeLibrary};
use super::{Appearance, BuiltIn, Token};
use eframe::egui::Color32;

/// `color` as a theme file writes it: `#RRGGBB`, or `#RRGGBBAA` with
/// straight alpha, chosen so reading it back gives exactly `color`.
pub(in crate::app) fn straight_hex(color: Color32) -> String {
    let [r, g, b, a] = color.to_array();
    if a == 255 {
        return format!("#{r:02X}{g:02X}{b:02X}");
    }
    if a == 0 {
        return "#00000000".to_owned();
    }
    // ecolor premultiplies with this rounding; find the straight value it
    // maps to each stored channel, the plain conversion's if that is one.
    let premultiply = |value: u8| {
        let p = u16::from(value) * u16::from(a) + 128;
        ((p + (p >> 8)) >> 8) as u8
    };
    let straight = |channel: u8| {
        let guess = (255.0 / f32::from(a) * f32::from(channel) + 0.5).min(255.0) as u8;
        if premultiply(guess) == channel {
            guess
        } else {
            (0..=255)
                .find(|value| premultiply(*value) == channel)
                .unwrap_or(guess)
        }
    };
    format!(
        "#{:02X}{:02X}{:02X}{a:02X}",
        straight(r),
        straight(g),
        straight(b)
    )
}

/// A complete theme file for the theme `id`: its name, appearance and every
/// token with its value, grouped and described. For a user theme, what its
/// files did not set is written as comments, recording what was chosen and
/// what was inherited; reading the file back gives the same colours.
pub(in crate::app) fn export_theme(library: &ThemeLibrary, id: &str) -> Result<String, Report> {
    let resolved = library.resolve(id)?;
    Ok(theme_file_text(&resolved, BuiltIn::from_id(id).is_some()))
}

fn theme_file_text(resolved: &Resolved, built_in: bool) -> String {
    let quote = |text: &str| serde_json::to_string(text).unwrap_or_default();
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"$schema\": \"./baboon-theme.schema.json\",\n");
    let name = if built_in {
        format!("{} (exported)", resolved.name)
    } else {
        resolved.name.clone()
    };
    out.push_str(&format!("  \"name\": {},\n", quote(&name)));
    out.push_str(&format!(
        "  \"appearance\": \"{}\",\n",
        resolved.colors.appearance.key()
    ));
    if let Some(root) = resolved.root_built_in {
        out.push_str(&format!("  \"extends\": \"{}\",\n", root.id()));
    }
    out.push_str("  \"colors\": {\n");
    let mut group = "";
    let set: Vec<Token> = Token::ALL
        .iter()
        .copied()
        .filter(|token| resolved.explicit.contains(token))
        .collect();
    let last_set = set.last().copied();
    for token in Token::ALL {
        if token.group() != group {
            group = token.group();
            out.push_str(&format!("\n    // {group}\n"));
        }
        out.push_str(&format!("    // {}\n", token.description()));
        let line = format!(
            "{}: \"{}\"",
            quote(token.id()),
            straight_hex(resolved.colors.get(*token))
        );
        if resolved.explicit.contains(token) {
            let comma = if Some(*token) == last_set { "" } else { "," };
            out.push_str(&format!("    {line}{comma}\n"));
        } else {
            out.push_str(&format!("    // {line}, (inherited)\n"));
        }
    }
    out.push_str("  }\n}\n");
    out
}

/// The JSON schema of a theme file: every token id with its description, and
/// the colour pattern. A theme naming it in `$schema` gets completion in an
/// editor that reads schemas.
pub(in crate::app) fn schema() -> String {
    let color = serde_json::json!({
        "type": "string",
        "pattern": "^(#[0-9A-Fa-f]{3,4}|#[0-9A-Fa-f]{6}|#[0-9A-Fa-f]{8}|\\$[a-z0-9_-]+)$",
        "description": "#RGB, #RGBA, #RRGGBB or #RRGGBBAA (straight alpha, as in CSS), or $name from the palette."
    });
    let mut colors = serde_json::Map::new();
    for token in Token::ALL {
        let mut property = color.clone();
        property["description"] =
            serde_json::Value::String(format!("{} ({})", token.description(), token.group()));
        colors.insert(token.id().to_owned(), property);
    }
    let schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "Baboon theme",
        "type": "object",
        "properties": {
            "$schema": { "type": "string" },
            "name": { "type": "string", "description": "The name Settings shows. The theme's id is its file name." },
            "appearance": { "enum": [Appearance::Dark.key(), Appearance::Light.key()], "description": "Picks egui's base visuals and the fallback colours." },
            "extends": { "type": "string", "description": "Another theme's id: a built-in (baboon-dark, baboon-light, baboon-light-classic) or a file in this folder." },
            "palette": {
                "type": "object",
                "description": "Names for colours, used below as $name.",
                "propertyNames": { "pattern": "^[a-z0-9_-]+$" },
                "additionalProperties": color.clone()
            },
            "colors": {
                "type": "object",
                "description": "Token id to colour.",
                "properties": colors,
                "additionalProperties": false
            }
        },
        "additionalProperties": false
    });
    serde_json::to_string_pretty(&schema).unwrap_or_default() + "\n"
}

/// Writes [`schema`] into `folder`, making the folder if needed.
pub(in crate::app) fn write_schema(folder: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    std::fs::write(folder.join("baboon-theme.schema.json"), schema())
}

/// Where Export Current Theme writes the theme `id` in `folder`: its id with
/// `-exported`, numbered past any file already there.
pub(in crate::app) fn export_path(folder: &std::path::Path, id: &str) -> std::path::PathBuf {
    let base = format!("{id}-exported");
    let mut path = folder.join(format!("{base}.json"));
    let mut number = 2;
    while path.exists() {
        path = folder.join(format!("{base}-{number}.json"));
        number += 1;
    }
    path
}

/// The authoring guide's token reference: a table per group of every token's
/// id, description and value in each built-in. `docs/themes.md` holds it, and
/// a test keeps the two the same.
#[cfg(test)]
pub(in crate::app) fn token_reference_markdown() -> String {
    let mut out = String::new();
    let mut group = "";
    for token in Token::ALL {
        if token.group() != group {
            group = token.group();
            out.push_str(&format!("\n### {group}\n\n| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |\n|---|---|---|---|---|---|\n"));
        }
        let value = |built_in: BuiltIn| straight_hex(built_in.color(*token));
        let rule = |rule: Option<super::rules::Rule>| {
            rule.map_or_else(|| "nothing".to_owned(), |rule| rule.to_string())
        };
        let rules = token.rules();
        out.push_str(&format!(
            "| `{}` | {} | `{}` | `{}` | `{}` | {}; {} |\n",
            token.id(),
            token.description(),
            value(BuiltIn::Dark),
            value(BuiltIn::Light),
            value(BuiltIn::ClassicLight),
            rule(rules.dark),
            rule(rules.light),
        ));
    }
    out
}

/// A complete theme written from scratch: `colors` with every token set and
/// nothing extended, as `docs/theme-examples/nightfall.json` is.
#[cfg(test)]
pub(in crate::app) fn complete_theme_text(name: &str, colors: &super::ThemeColors) -> String {
    let resolved = Resolved {
        colors: colors.clone(),
        reports: Vec::new(),
        files: Vec::new(),
        explicit: Token::ALL.iter().copied().collect(),
        following: Default::default(),
        root_built_in: None,
        name: name.to_owned(),
    };
    theme_file_text(&resolved, false)
}
