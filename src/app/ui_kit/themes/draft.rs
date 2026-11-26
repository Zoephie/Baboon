//! The theme editor's draft: a theme being edited, resolved after every
//! change so it can be drawn live, and written back to its file or to a new
//! one. Holds no UI; the window is `shell::theme_editor`.

use super::export::straight_hex;
use super::file::{Report, ThemeFile, parse_theme};
use super::library::{Fingerprint, Resolved, ThemeLibrary};
use super::rules::Rule;
use super::{BuiltIn, ThemeColors, Token};
use eframe::egui::Color32;
use std::path::{Path, PathBuf};

/// The id a built-in's draft resolves under. Not a possible file name, so no
/// theme file can have it.
const BUILT_IN_DRAFT_ID: &str = "<draft>";

/// Where a row's colour comes from.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) enum RowState {
    /// The theme sets it, as written: hex or `$name`.
    Set(String),
    /// Unset, and computed from a token the theme sets.
    Following(Rule),
    /// Unset: the parent's colour, or the built-in's.
    Inherited,
}

/// Why a save did not happen.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) enum SaveError {
    /// The file changed on disk since the editor read it; saving again with
    /// `overwrite` replaces it.
    ChangedOnDisk(PathBuf),
    Failed(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::ChangedOnDisk(path) => {
                write!(
                    formatter,
                    "{} changed on disk since the editor read it",
                    path.display()
                )
            }
            SaveError::Failed(message) => formatter.write_str(message),
        }
    }
}

/// A theme being edited.
pub(in crate::app) struct ThemeDraft {
    /// The theme edited: a file's id or a built-in's.
    id: String,
    built_in: Option<BuiltIn>,
    library: ThemeLibrary,
    /// The file as last read or saved; for a built-in, an empty theme that
    /// extends it.
    saved: ThemeFile,
    /// Its text and fingerprint as last read or saved.
    text: Option<String>,
    path: Option<PathBuf>,
    fingerprint: Option<Fingerprint>,
    /// What `saved` resolves to.
    saved_resolved: Resolved,
    /// The draft and what it resolves to.
    file: ThemeFile,
    resolved: Resolved,
    /// The colours installed when the editor opened, or at the last save:
    /// what Cancel puts back.
    before: ThemeColors,
}

impl ThemeDraft {
    /// A draft of the theme `id` from `library`, opened while `before` was
    /// installed.
    pub(in crate::app) fn open(
        library: ThemeLibrary,
        id: &str,
        before: ThemeColors,
    ) -> Result<Self, Report> {
        let built_in = BuiltIn::from_id(id);
        let (saved, path, text) = match built_in {
            Some(built_in) => (
                ThemeFile {
                    name: format!("{} (custom)", built_in.name()),
                    extends: Some(built_in.id().to_owned()),
                    ..ThemeFile::default()
                },
                None,
                None,
            ),
            None => {
                let listed = library.listed(id).ok_or_else(|| {
                    Report::new(&format!("{id}.json"), None, "is not in the themes folder")
                })?;
                let (file, _) = listed.parsed.clone()?;
                let text = std::fs::read_to_string(&listed.path).ok();
                (file, Some(listed.path.clone()), text)
            }
        };
        let fingerprint = path.as_deref().and_then(Fingerprint::of);
        let draft_id = if built_in.is_some() {
            BUILT_IN_DRAFT_ID
        } else {
            id
        };
        let saved_resolved = resolve(&library, draft_id, &path, &saved)?;
        Ok(Self {
            id: id.to_owned(),
            built_in,
            resolved: saved_resolved.clone(),
            file: saved.clone(),
            library,
            saved,
            text,
            path,
            fingerprint,
            saved_resolved,
            before,
        })
    }

    /// The draft of a theme just chosen: Cancel puts back its own colours,
    /// not those drawn before it was chosen.
    pub(in crate::app) fn chosen(mut self) -> Self {
        self.before = self.saved_resolved.colors.clone();
        self
    }

    /// The id of the theme edited.
    pub(in crate::app) fn id(&self) -> &str {
        &self.id
    }

    fn draft_id(&self) -> &str {
        if self.built_in.is_some() {
            BUILT_IN_DRAFT_ID
        } else {
            &self.id
        }
    }

    fn resolve_draft(&mut self) {
        // A draft holds only colours the editor wrote and the file's own, so
        // it resolves whenever the file did.
        if let Ok(resolved) = resolve(&self.library, self.draft_id(), &self.path, &self.file) {
            self.resolved = resolved;
        }
    }

    /// Its name, as Settings shows it.
    pub(in crate::app) fn name(&self) -> &str {
        match self.built_in {
            Some(built_in) => built_in.name(),
            None => &self.saved.name,
        }
    }

    /// Whether the theme is a built-in, which only Save As can keep.
    pub(in crate::app) fn is_built_in(&self) -> bool {
        self.built_in.is_some()
    }

    /// The draft's colours, to install.
    pub(in crate::app) fn colors(&self) -> &ThemeColors {
        &self.resolved.colors
    }

    /// What Cancel installs.
    pub(in crate::app) fn before(&self) -> &ThemeColors {
        &self.before
    }

    pub(in crate::app) fn color(&self, token: Token) -> Color32 {
        self.resolved.colors.get(token)
    }

    /// Where `token`'s colour comes from in the draft.
    pub(in crate::app) fn state(&self, token: Token) -> RowState {
        if let Some(value) = written(&self.file, token) {
            RowState::Set(value.to_owned())
        } else if self.resolved.following.contains(&token)
            && let Some(rule) = token.rules().of(self.resolved.colors.appearance)
        {
            RowState::Following(rule)
        } else {
            RowState::Inherited
        }
    }

    /// Sets `token` to `color`.
    pub(in crate::app) fn set(&mut self, token: Token, color: Color32) {
        self.write(token, Some(straight_hex(color)));
    }

    /// Unsets `token`, so it inherits or follows again.
    pub(in crate::app) fn reset(&mut self, token: Token) {
        self.write(token, None);
    }

    /// Writes `token`'s value as text, or removes it: how a picker's Cancel
    /// puts back what the row held, `$name` included.
    pub(in crate::app) fn write(&mut self, token: Token, value: Option<String>) {
        let at = self.file.colors.iter().position(|(id, _)| id == token.id());
        match (at, value) {
            (Some(at), Some(value)) => self.file.colors[at].1 = value,
            (None, Some(value)) => self.file.colors.push((token.id().to_owned(), value)),
            (Some(_), None) => self.file.colors.retain(|(id, _)| id != token.id()),
            (None, None) => return,
        }
        self.resolve_draft();
    }

    /// Whether the draft differs from what was last read or saved.
    pub(in crate::app) fn is_dirty(&self) -> bool {
        !same_colors(&self.file, &self.saved)
    }

    /// Whether Save can write the draft back: a theme file, not a built-in.
    pub(in crate::app) fn can_save(&self) -> bool {
        self.built_in.is_none() && self.path.is_some()
    }

    /// Writes the draft back to its file, keeping its other keys and, where
    /// it can, its comments and layout. A file that changed on disk since it
    /// was read is replaced only with `overwrite`.
    pub(in crate::app) fn save(&mut self, overwrite: bool) -> Result<PathBuf, SaveError> {
        let path = self
            .path
            .clone()
            .filter(|_| self.built_in.is_none())
            .ok_or_else(|| SaveError::Failed("a built-in theme is kept with Save As".to_owned()))?;
        if !overwrite && Fingerprint::of(&path) != self.fingerprint {
            return Err(SaveError::ChangedOnDisk(path));
        }
        let text = self
            .text
            .as_deref()
            .and_then(|text| patch_colors(&self.id, text, &self.file))
            .unwrap_or_else(|| file_text(&self.file));
        std::fs::write(&path, &text).map_err(|error| {
            SaveError::Failed(format!("could not write {}: {error}", path.display()))
        })?;
        self.fingerprint = Fingerprint::of(&path);
        self.text = Some(text);
        self.saved = self.file.clone();
        self.saved_resolved = self.resolved.clone();
        self.before = self.resolved.colors.clone();
        self.library = self
            .library
            .with_file(&self.id, path.clone(), self.file.clone());
        Ok(path)
    }

    /// The text of a new theme called `name` that extends the theme edited
    /// and sets only what the draft changed: every token whose colour moved,
    /// except those that follow a token it sets, which follow again.
    pub(in crate::app) fn save_as_text(&self, name: &str) -> String {
        file_text(&self.save_as_file(name))
    }

    fn save_as_file(&self, name: &str) -> ThemeFile {
        let changed = |token: &Token| {
            self.resolved.colors.get(*token) != self.saved_resolved.colors.get(*token)
        };
        let mut colors: Vec<(String, String)> = Token::ALL
            .iter()
            .filter(|token| changed(token) && !self.resolved.following.contains(token))
            .map(|token| {
                (
                    token.id().to_owned(),
                    straight_hex(self.resolved.colors.get(*token)),
                )
            })
            .collect();
        let child = |colors: Vec<(String, String)>| ThemeFile {
            name: name.to_owned(),
            appearance: None,
            extends: Some(self.id.clone()),
            palette: Vec::new(),
            colors,
        };
        // A token can move only because its rule now reads a colour the
        // child inherits rather than sets; then it is written too.
        let first = child(colors.clone());
        if let Ok(resolved) = resolve(&self.library, BUILT_IN_DRAFT_ID, &None, &first) {
            for token in Token::ALL {
                if resolved.colors.get(*token) != self.resolved.colors.get(*token)
                    && !colors.iter().any(|(id, _)| id == token.id())
                {
                    colors.push((
                        token.id().to_owned(),
                        straight_hex(self.resolved.colors.get(*token)),
                    ));
                }
            }
        }
        child(colors)
    }

    /// Writes a new theme called `name` into `folder` (see
    /// [`ThemeDraft::save_as_text`]), and returns its id and path. The id is
    /// the name in lowercase with dashes; a built-in's id or a file's that
    /// exists already is refused.
    pub(in crate::app) fn save_as(
        &self,
        folder: &Path,
        name: &str,
    ) -> Result<(String, PathBuf), String> {
        let name = name.trim();
        let id = theme_id_for(name).ok_or("the name needs a letter or digit")?;
        if BuiltIn::from_id(&id).is_some() {
            return Err(format!("{id} is a built-in theme's id"));
        }
        if self.library.listed(&id).is_some() {
            return Err(format!("a theme with the id {id} exists already"));
        }
        let path = folder.join(format!("{id}.json"));
        if path.exists() {
            return Err(format!("{} exists already", path.display()));
        }
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("could not make {}: {error}", folder.display()))?;
        std::fs::write(&path, self.save_as_text(name))
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        Ok((id, path))
    }
}

/// Resolves `file` as the theme `id` over `library`.
fn resolve(
    library: &ThemeLibrary,
    id: &str,
    path: &Option<PathBuf>,
    file: &ThemeFile,
) -> Result<Resolved, Report> {
    let path = path
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("{id}.json")));
    library.with_file(id, path, file.clone()).resolve(id)
}

/// The value `file` writes for `token`: the last, as JSON reads it.
fn written(file: &ThemeFile, token: Token) -> Option<&str> {
    file.colors
        .iter()
        .rev()
        .find(|(id, _)| id == token.id())
        .map(|(_, value)| value.as_str())
}

/// Whether two files set the same colours, in any order.
fn same_colors(a: &ThemeFile, b: &ThemeFile) -> bool {
    let map = |file: &ThemeFile| -> std::collections::BTreeMap<String, String> {
        file.colors.iter().cloned().collect()
    };
    map(a) == map(b)
}

/// A theme's id from its name: lowercase letters and digits, other runs as
/// one dash. `None` when nothing is left.
pub(in crate::app) fn theme_id_for(name: &str) -> Option<String> {
    let mut id = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            id.extend(c.to_lowercase());
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_end_matches('-').to_owned();
    (!id.is_empty()).then_some(id)
}

/// `file` written out whole.
pub(in crate::app) fn file_text(file: &ThemeFile) -> String {
    let quote = |text: &str| serde_json::to_string(text).unwrap_or_default();
    let mut out = String::from("{\n  \"$schema\": \"./baboon-theme.schema.json\",\n");
    out.push_str(&format!("  \"name\": {}", quote(&file.name)));
    if let Some(appearance) = file.appearance {
        out.push_str(&format!(",\n  \"appearance\": \"{}\"", appearance.key()));
    }
    if let Some(extends) = &file.extends {
        out.push_str(&format!(",\n  \"extends\": {}", quote(extends)));
    }
    let object = |out: &mut String, key: &str, entries: &[(String, String)]| {
        out.push_str(&format!(",\n  \"{key}\": {{"));
        for (at, (name, value)) in entries.iter().enumerate() {
            let comma = if at + 1 < entries.len() { "," } else { "" };
            out.push_str(&format!("\n    {}: {}{comma}", quote(name), quote(value)));
        }
        out.push_str(if entries.is_empty() { "}" } else { "\n  }" });
    };
    if !file.palette.is_empty() {
        object(&mut out, "palette", &file.palette);
    }
    object(&mut out, "colors", &file.colors);
    out.push_str("\n}\n");
    out
}

/// `text`, the theme `id`'s file, with its colours changed to `wanted`'s:
/// each changed value replaced where it stands, each removed entry's line
/// taken out, each new entry added at the top of `colors`. Everything else,
/// comments included, is kept. `None` when the file is laid out so that this
/// cannot be done safely, or when the result would not read back as `wanted`;
/// the caller then writes the file whole.
pub(in crate::app) fn patch_colors(id: &str, text: &str, wanted: &ThemeFile) -> Option<String> {
    let file_name = format!("{id}.json");
    let (current, _) = parse_theme(id, &file_name, text).ok()?;
    let mut text = text.to_owned();
    let entry = |key: &str, value: &str| -> String {
        format!(
            "{}: {}",
            serde_json::to_string(key).unwrap(),
            serde_json::to_string(value).unwrap()
        )
    };
    for (key, value) in &current.colors {
        // Each key must be written once, in the plain form, to be edited.
        let occurrences = text
            .matches(&format!("{}:", serde_json::to_string(key).unwrap()))
            .count();
        let plain = entry(key, value);
        if occurrences != 1 || text.matches(&plain).count() != 1 {
            return None;
        }
        match wanted.colors.iter().find(|(id, _)| id == key) {
            Some((_, new)) if new == value => {}
            Some((_, new)) => text = text.replacen(&plain, &entry(key, new), 1),
            None => {
                // Only an entry alone on its line is taken out.
                let at = text.find(&plain)?;
                let start = text[..at].rfind('\n').map_or(0, |line| line + 1);
                let end = text[at..]
                    .find('\n')
                    .map_or(text.len(), |line| at + line + 1);
                let rest = text[at + plain.len()..end].trim();
                if !text[start..at].trim().is_empty() || !matches!(rest, "" | ",") {
                    return None;
                }
                text.replace_range(start..end, "");
            }
        }
    }
    let added: Vec<&(String, String)> = wanted
        .colors
        .iter()
        .filter(|(key, _)| !current.colors.iter().any(|(id, _)| id == key))
        .collect();
    if !added.is_empty() {
        if text.matches("\"colors\"").count() != 1 {
            return None;
        }
        let colors = text.find("\"colors\"")?;
        let open = colors + text[colors..].find('{')?;
        if !text[colors + "\"colors\"".len()..open].trim().eq(":") {
            return None;
        }
        // Indented like the entry after it, or four spaces.
        let after = &text[open + 1..];
        let indent: String = after
            .lines()
            .skip(1)
            .find(|line| !line.trim().is_empty() && !line.trim().starts_with('}'))
            .map(|line| line.chars().take_while(|c| c.is_whitespace()).collect())
            .unwrap_or_else(|| "    ".to_owned());
        let inserted: String = added
            .iter()
            .map(|(key, value)| format!("\n{indent}{},", entry(key, value)))
            .collect();
        text.insert_str(open + 1, &inserted);
    }
    let (reread, _) = parse_theme(id, &file_name, &text).ok()?;
    let unchanged = ThemeFile {
        colors: Vec::new(),
        ..reread.clone()
    } == ThemeFile {
        colors: Vec::new(),
        ..current
    };
    (unchanged && same_colors(&reread, wanted)).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::super::{install, installed};
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("baboon-theme-draft-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    const PARENT: &str = r##"{
  // A comment the save keeps.
  "name": "Parent",
  "extends": "baboon-dark",
  "palette": { "brand": "#336699" },
  "colors": {
    // The accent, from the palette.
    "accent": "$brand",
    "text": "#EEEEEE",
    "border": "#444444"
  }
}
"##;

    fn open(folder: &Path, id: &str) -> ThemeDraft {
        let library = ThemeLibrary::scan(folder);
        ThemeDraft::open(library, id, BuiltIn::Dark.colors()).unwrap()
    }

    /// Save As then reading the new file gives exactly the colours the
    /// editor showed, for a built-in and for a theme file.
    #[test]
    fn save_as_reads_back_as_what_the_editor_showed() {
        let folder = folder("save-as");
        std::fs::write(folder.join("parent.json"), PARENT).unwrap();
        for id in ["baboon-dark", "baboon-light-classic", "parent"] {
            let mut draft = open(&folder, id);
            draft.set(Token::Background, Color32::from_rgb(0x20, 0x24, 0x30));
            draft.set(
                Token::StatusWarning,
                Color32::from_rgba_unmultiplied(0xF0, 0xC0, 0x40, 0xCC),
            );
            draft.reset(Token::Text);
            let name = format!("Child of {id}");
            let (child, path) = draft.save_as(&folder, &name).unwrap();
            assert_eq!(path, folder.join(format!("{child}.json")));
            let resolved = ThemeLibrary::scan(&folder).resolve(&child).unwrap();
            assert_eq!(resolved.colors, *draft.colors(), "{id}");
            assert!(resolved.reports.is_empty(), "{id}: {:?}", resolved.reports);
            // A changed token left out of the file changes what loads.
            let text = draft.save_as_text(&name);
            let dropped: String = text
                .lines()
                .filter(|line| !line.contains("\"background\""))
                .collect::<Vec<_>>()
                .join("\n");
            let reread = ThemeLibrary::scan(&folder).with_file(
                "dropped",
                PathBuf::from("dropped.json"),
                parse_theme("dropped", "dropped.json", &dropped).unwrap().0,
            );
            assert_ne!(
                reread.resolve("dropped").unwrap().colors,
                *draft.colors(),
                "{id}: dropping a token went unnoticed"
            );
            std::fs::remove_file(&path).unwrap();
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Save As writes the changed tokens and no others: not the ones that
    /// follow, not the ones inherited.
    #[test]
    fn save_as_writes_only_what_changed() {
        let library = ThemeLibrary::default();
        let mut draft = ThemeDraft::open(library, "baboon-dark", BuiltIn::Dark.colors()).unwrap();
        draft.set(Token::Accent, Color32::from_rgb(0xE8, 0x86, 0x2A));
        draft.set(Token::TerminalText, Color32::from_rgb(0, 0xFF, 0));
        let file = draft.save_as_file("Orange");
        let written: Vec<&str> = file.colors.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(written, ["accent", "terminal.text"]);
        assert_eq!(file.extends.as_deref(), Some("baboon-dark"));
        assert!(
            !draft.resolved.following.is_empty(),
            "the accent's dependents moved, and were not written"
        );
        // The check notices an inherited token written as set.
        let mut extra = file.clone();
        extra
            .colors
            .push(("text".to_owned(), straight_hex(draft.color(Token::Text))));
        let extra_written: Vec<&str> = extra.colors.iter().map(|(id, _)| id.as_str()).collect();
        assert_ne!(extra_written, written);
    }

    /// Cancel puts back every colour installed when the editor opened.
    #[test]
    fn cancel_restores_what_was_installed() {
        let _lock = super::super::tests::INSTALL_LOCK.lock().unwrap();
        let outside = installed();
        install(&BuiltIn::Light.colors());
        let mut draft =
            ThemeDraft::open(ThemeLibrary::default(), "baboon-light", installed()).unwrap();
        draft.set(Token::Background, Color32::from_rgb(1, 2, 3));
        draft.set(Token::Accent, Color32::from_rgb(4, 5, 6));
        install(draft.colors());
        assert_ne!(
            installed(),
            BuiltIn::Light.colors(),
            "the live edit was installed"
        );
        install(draft.before());
        assert_eq!(installed(), BuiltIn::Light.colors());
        install(&outside);
    }

    /// Reset makes a row inherit again, and saving takes its key out of the
    /// file, keeping the file's comments, palette and other entries.
    #[test]
    fn reset_inherits_again_and_saving_removes_the_key() {
        let folder = folder("reset");
        std::fs::write(folder.join("parent.json"), PARENT).unwrap();
        let mut draft = open(&folder, "parent");
        assert_eq!(
            draft.state(Token::Border),
            RowState::Set("#444444".to_owned())
        );
        draft.reset(Token::Border);
        assert_ne!(
            draft.state(Token::Border),
            RowState::Set("#444444".to_owned())
        );
        assert_eq!(
            draft.color(Token::Border),
            BuiltIn::Dark.colors().get(Token::Border)
        );
        draft.set(Token::Text, Color32::from_rgb(0xDD, 0xDD, 0xDD));
        draft.set(Token::ProgressBar, Color32::from_rgb(0x11, 0x22, 0x33));
        assert!(draft.is_dirty());
        draft.save(false).unwrap();
        assert!(!draft.is_dirty());
        let text = std::fs::read_to_string(folder.join("parent.json")).unwrap();
        assert!(!text.contains("\"border\""), "{text}");
        assert!(text.contains("// A comment the save keeps."), "{text}");
        assert!(text.contains("// The accent, from the palette."), "{text}");
        assert!(text.contains("\"accent\": \"$brand\""), "{text}");
        assert!(text.contains("\"text\": \"#DDDDDD\""), "{text}");
        assert!(text.contains("\"progress.bar\": \"#112233\""), "{text}");
        let resolved = ThemeLibrary::scan(&folder).resolve("parent").unwrap();
        assert_eq!(resolved.colors, *draft.colors());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A file changed on disk since the editor read it is not overwritten
    /// unless asked.
    #[test]
    fn saving_over_a_changed_file_asks_first() {
        let folder = folder("changed");
        let path = folder.join("parent.json");
        std::fs::write(&path, PARENT).unwrap();
        let mut draft = open(&folder, "parent");
        draft.set(Token::Text, Color32::from_rgb(0xDD, 0xDD, 0xDD));
        std::fs::write(
            &path,
            PARENT
                .replace("#444444", "#555555 ")
                .replace("#555555 ", "#555555"),
        )
        .unwrap();
        // Same length: change the modified time too, for file systems with
        // coarse times.
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        drop(file);
        assert_eq!(
            draft.save(false),
            Err(SaveError::ChangedOnDisk(path.clone()))
        );
        assert!(std::fs::read_to_string(&path).unwrap().contains("#555555"));
        draft.save(true).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("#DDDDDD"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A built-in is kept only with Save As, under a fresh id.
    #[test]
    fn built_ins_and_taken_ids_are_refused() {
        let folder = folder("refused");
        std::fs::write(folder.join("parent.json"), PARENT).unwrap();
        let mut draft = open(&folder, "baboon-dark");
        assert!(!draft.can_save());
        assert!(matches!(draft.save(true), Err(SaveError::Failed(_))));
        assert_eq!(
            draft.save_as(&folder, "Baboon Dark").unwrap_err(),
            "baboon-dark is a built-in theme's id"
        );
        assert_eq!(
            draft.save_as(&folder, "PARENT").unwrap_err(),
            "a theme with the id parent exists already"
        );
        assert_eq!(
            draft.save_as(&folder, " - ").unwrap_err(),
            "the name needs a letter or digit"
        );
        assert_eq!(
            theme_id_for("My  Theme (v2)!"),
            Some("my-theme-v2".to_owned())
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A file laid out so its colours cannot be edited in place is written
    /// whole, and still reads back as the draft.
    #[test]
    fn a_file_that_cannot_be_patched_is_written_whole() {
        let one_line = r##"{ "name": "One", "extends": "baboon-dark", "colors": { "text": "#EEEEEE", "border": "#444444" } }"##;
        let mut wanted = parse_theme("one", "one.json", one_line).unwrap().0;
        wanted.colors.retain(|(id, _)| id != "border");
        assert_eq!(
            patch_colors("one", one_line, &wanted),
            None,
            "border shares its line"
        );
        let whole = file_text(&wanted);
        assert_eq!(parse_theme("one", "one.json", &whole).unwrap().0, wanted);
    }
}
