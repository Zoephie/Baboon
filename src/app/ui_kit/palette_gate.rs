//! Keeps colours in the style module. A colour written as numbers anywhere
//! else in the app, or a branch on the theme, fails this test, so every
//! colour has a name and a theme can change it in one place.
//!
//! A colour that must not follow the theme (a mark drawn over an image, a
//! backdrop the user picks) says so with a `// fixed:` comment and its reason,
//! on its line or the line above.

use std::path::Path;

/// Folders whose colours belong to a 3D scene, not to the interface.
const ALLOWED_FOLDERS: &[(&str, &str)] = &[
    ("model_preview/renderer/", "the 3D scene's own colours, drawn over the model"),
    ("model_preview/viewport/", "markers and outlines drawn over the 3D scene"),
];

const CONSTRUCTORS: &[&str] = &[
    "from_rgb",
    "from_rgba_unmultiplied",
    "from_rgba_premultiplied",
    "from_gray",
    "from_white_alpha",
    "from_black_alpha",
    "from_hex",
];

/// egui's named colours, other than black, white and transparent, which are
/// plain values rather than a palette choice.
const NAMED: &[&str] = &[
    "RED", "GREEN", "BLUE", "YELLOW", "GRAY", "DARK_GRAY", "LIGHT_GRAY", "LIGHT_BLUE",
    "DARK_RED", "LIGHT_RED", "LIGHT_GREEN", "DARK_GREEN", "DARK_BLUE", "GOLD", "KHAKI",
    "BROWN", "ORANGE", "LIGHT_YELLOW",
];

const THEME_READS: &[&str] = &["is_dark_mode()", "visuals().dark_mode", "visuals.dark_mode"];

fn is_literal(arg: &str) -> bool {
    let arg = arg.trim();
    let digits = arg.trim_end_matches("u8").trim_end_matches('_');
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        || (arg.starts_with('"') && arg.ends_with('"') && arg.len() >= 2)
}

/// The lines (1-based) of `source` holding a colour or theme branch that
/// belongs in the style module.
fn offending_lines(source: &str) -> Vec<usize> {
    let source = match source.find("#[cfg(test)]\nmod ") {
        Some(tests) => &source[..tests],
        None => source,
    };
    let lines: Vec<&str> = source.split('\n').collect();
    let line_of = |offset: usize| source[..offset].matches('\n').count() + 1;
    let marked = |line: usize| {
        lines[line - 1].contains("// fixed:")
            || (line >= 2 && lines[line - 2].trim_start().starts_with("// fixed:"))
    };
    let mut found = Vec::new();
    for (start, _) in source.match_indices("Color32::") {
        let rest = &source[start + "Color32::".len()..];
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..name_len];
        let after = rest[name_len..].trim_start();
        let literal_call = CONSTRUCTORS.contains(&name)
            && after.starts_with('(')
            && after[1..].find(')').is_some_and(|close| {
                let args = &after[1..1 + close];
                !args.contains('(') && args.split(',').filter(|a| !a.trim().is_empty()).all(is_literal)
            });
        if literal_call || NAMED.contains(&name) {
            found.push(line_of(start));
        }
    }
    for read in THEME_READS {
        for (start, _) in source.match_indices(read) {
            found.push(line_of(start));
        }
    }
    found.retain(|&line| !marked(line));
    found.sort_unstable();
    found.dedup();
    found
}

#[test]
fn colours_and_theme_branches_live_in_the_style_module() {
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("app");
    let mut offences = Vec::new();
    for entry in walkdir::WalkDir::new(&app).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let relative = path.strip_prefix(&app).unwrap().to_string_lossy().replace('\\', "/");
        if relative.starts_with("ui_kit/")
            || relative.ends_with("tests.rs")
            || relative.contains("tests/")
            || ALLOWED_FOLDERS.iter().any(|(folder, _)| relative.starts_with(folder))
        {
            continue;
        }
        let source = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        let lines: Vec<&str> = source.split('\n').collect();
        for line in offending_lines(&source) {
            offences.push(format!("{relative}:{line}: {}", lines[line - 1].trim()));
        }
    }
    assert!(
        offences.is_empty(),
        "Colours belong in ui_kit/style.rs as named accessors. Name these there, \
         or mark a colour that must not follow the theme with `// fixed: <reason>`:\n{}",
        offences.join("\n")
    );
}

#[test]
fn the_scan_finds_inline_colours_and_respects_fixed_marks() {
    let source = "\
let a = Color32::from_rgb(1, 2, 3);
let b = Color32::from_rgba_unmultiplied(
    255, 214, 0, 38,
);
let c = Color32::DARK_RED;
let d = if is_dark_mode() { a } else { b };
let e = Color32::from_rgb(r, g, b);
let f = Color32::from_gray(value.max(3));
let g = Color32::WHITE;
let h = Color32::from_white_alpha(9); // fixed: over an image
// fixed: over an image
let i = Color32::from_black_alpha(9);
let j = ui.visuals().dark_mode;
#[cfg(test)]
mod tests {
    let k = Color32::from_rgb(1, 2, 3);
}
";
    assert_eq!(offending_lines(source), vec![1, 2, 5, 6, 13]);
}
