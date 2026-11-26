//! Reading a theme file: JSON with `//` and `/* */` comments and trailing
//! commas tolerated, holding a `name`, an `appearance`, an optional
//! `extends`, an optional `palette` and the `colors` it sets.

use super::Appearance;
use serde_json::Value;

/// One problem with a theme, named by its file and, where there is one, its
/// key. None of them stops Baboon from drawing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct Report {
    pub(in crate::app) file: String,
    pub(in crate::app) key: Option<String>,
    pub(in crate::app) message: String,
}

impl Report {
    pub(in crate::app) fn new(file: &str, key: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            file: file.to_owned(),
            key: key.map(str::to_owned),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.key {
            Some(key) => write!(formatter, "{}: {key}: {}", self.file, self.message),
            None => write!(formatter, "{}: {}", self.file, self.message),
        }
    }
}

/// What a theme file says, its values as written: hex or `$name` from the
/// palette. Read in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::app) struct ThemeFile {
    pub(in crate::app) name: String,
    pub(in crate::app) appearance: Option<Appearance>,
    /// Another theme's id, lowercased.
    pub(in crate::app) extends: Option<String>,
    pub(in crate::app) palette: Vec<(String, String)>,
    pub(in crate::app) colors: Vec<(String, String)>,
}

/// Reads `text`, the file `file_name` with the id `id`. A file that is not
/// a JSON object cannot be applied, and gives one report; otherwise every key
/// it cannot use is reported and the rest is kept.
pub(in crate::app) fn parse_theme(
    id: &str,
    file_name: &str,
    text: &str,
) -> Result<(ThemeFile, Vec<Report>), Report> {
    let value: Value = serde_json::from_str(&strip_json_extras(text))
        .map_err(|error| Report::new(file_name, None, format!("is not valid JSON: {error}")))?;
    let Value::Object(object) = value else {
        return Err(Report::new(file_name, None, "is not a JSON object"));
    };
    let mut reports = Vec::new();
    let mut file = ThemeFile::default();
    let mut named = false;
    for (key, value) in &object {
        match key.as_str() {
            "name" => match value.as_str() {
                Some(name) if !name.trim().is_empty() => {
                    file.name = name.trim().to_owned();
                    named = true;
                }
                _ => reports.push(Report::new(
                    file_name,
                    Some(key),
                    "is not a name; the file name is used",
                )),
            },
            "appearance" => match value.as_str().and_then(Appearance::from_key) {
                Some(appearance) => file.appearance = Some(appearance),
                None => reports.push(Report::new(
                    file_name,
                    Some(key),
                    "is not \"dark\" or \"light\"; ignored",
                )),
            },
            "extends" => match value.as_str() {
                Some(parent) if !parent.trim().is_empty() => {
                    file.extends = Some(parent.trim().to_lowercase());
                }
                _ => reports.push(Report::new(
                    file_name,
                    Some(key),
                    "is not a theme id; ignored",
                )),
            },
            "palette" => match value {
                Value::Object(palette) => {
                    for (name, value) in palette {
                        let at = format!("palette.{name}");
                        if !palette_name(name) {
                            reports.push(Report::new(
                                file_name,
                                Some(&at),
                                "is not a palette name (a-z, 0-9, _ and -); ignored",
                            ));
                        } else if let Some(value) = value.as_str() {
                            file.palette.push((name.clone(), value.trim().to_owned()));
                        } else {
                            reports.push(Report::new(
                                file_name,
                                Some(&at),
                                "is not a colour; ignored",
                            ));
                        }
                    }
                }
                _ => reports.push(Report::new(
                    file_name,
                    Some(key),
                    "is not an object; ignored",
                )),
            },
            "colors" => match value {
                Value::Object(colors) => {
                    for (token, value) in colors {
                        match value.as_str() {
                            Some(value) => {
                                file.colors.push((token.clone(), value.trim().to_owned()))
                            }
                            None => reports.push(Report::new(
                                file_name,
                                Some(token),
                                "is not a colour; ignored",
                            )),
                        }
                    }
                }
                _ => reports.push(Report::new(
                    file_name,
                    Some(key),
                    "is not an object; ignored",
                )),
            },
            "$schema" => {}
            _ => reports.push(Report::new(
                file_name,
                Some(key),
                "is not a theme key; ignored",
            )),
        }
    }
    if !named {
        if !object.contains_key("name") {
            reports.push(Report::new(
                file_name,
                None,
                "has no name; the file name is used",
            ));
        }
        file.name = id.to_owned();
    }
    Ok((file, reports))
}

/// Palette names: lowercase letters, digits, `_` and `-`.
pub(in crate::app) fn palette_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A colour as a theme file writes it: `#RGB`, `#RGBA`, `#RRGGBB` or
/// `#RRGGBBAA`, alpha straight as in CSS.
pub(in crate::app) fn parse_color(text: &str) -> Option<eframe::egui::Color32> {
    let digits = text.strip_prefix('#')?;
    if !matches!(digits.len(), 3 | 4 | 6 | 8) || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    eframe::egui::Color32::from_hex(text).ok()
}

/// `text` with comments and trailing commas removed, outside strings, so
/// `serde_json` reads it. Line and column numbers in its errors stay right:
/// a comment becomes spaces, keeping its newlines.
pub(in crate::app) fn strip_json_extras(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    let mut in_string = false;
    while at < chars.len() {
        let c = chars[at];
        if in_string {
            out.push(c);
            if c == '\\' && at + 1 < chars.len() {
                out.push(chars[at + 1]);
                at += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            at += 1;
            continue;
        }
        match (c, chars.get(at + 1)) {
            ('"', _) => {
                in_string = true;
                out.push(c);
                at += 1;
            }
            ('/', Some('/')) => {
                while at < chars.len() && chars[at] != '\n' {
                    out.push(' ');
                    at += 1;
                }
            }
            ('/', Some('*')) => {
                out.push_str("  ");
                at += 2;
                while at < chars.len() && !(chars[at] == '*' && chars.get(at + 1) == Some(&'/')) {
                    out.push(if chars[at] == '\n' { '\n' } else { ' ' });
                    at += 1;
                }
                out.push_str("  ");
                at = (at + 2).min(chars.len());
            }
            (',', _) => {
                // A comma whose next significant character closes the object
                // or array is a trailing comma.
                let mut next = at + 1;
                loop {
                    while next < chars.len() && chars[next].is_whitespace() {
                        next += 1;
                    }
                    if chars.get(next) == Some(&'/') && chars.get(next + 1) == Some(&'/') {
                        while next < chars.len() && chars[next] != '\n' {
                            next += 1;
                        }
                    } else if chars.get(next) == Some(&'/') && chars.get(next + 1) == Some(&'*') {
                        next += 2;
                        while next < chars.len()
                            && !(chars[next] == '*' && chars.get(next + 1) == Some(&'/'))
                        {
                            next += 1;
                        }
                        next += 2;
                    } else {
                        break;
                    }
                }
                out.push(if matches!(chars.get(next), Some('}') | Some(']')) {
                    ' '
                } else {
                    ','
                });
                at += 1;
            }
            _ => {
                out.push(c);
                at += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_trailing_commas_are_tolerated_outside_strings() {
        let text = r##"{
            // a comment
            "name": "A // not a comment", /* a block
               comment */
            "colors": { "text": "#FFFFFF", },
        }"##;
        let (file, reports) = parse_theme("a", "a.json", text).unwrap();
        assert_eq!(file.name, "A // not a comment");
        assert_eq!(file.colors, vec![("text".to_owned(), "#FFFFFF".to_owned())]);
        assert!(reports.is_empty(), "{reports:?}");
    }

    #[test]
    fn colours_take_the_four_hex_forms_with_straight_alpha() {
        use eframe::egui::Color32;
        assert_eq!(parse_color("#FFF"), Some(Color32::WHITE));
        assert_eq!(
            parse_color("#FFFFFF1A"),
            Some(Color32::from_rgba_premultiplied(26, 26, 26, 26))
        );
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("123456"), None);
        assert_eq!(parse_color("#GGGGGG"), None);
    }
}
