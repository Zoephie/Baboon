//! The themes Baboon can apply: the built-ins, and the user's theme files in
//! the themes folder, each identified by its file name. Resolving a theme
//! follows its `extends` chain and fills every token.

use super::file::{Report, ThemeFile, parse_color, parse_theme};
use super::{Appearance, BuiltIn, ThemeColors, Token};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Theme files larger than this are skipped.
pub(in crate::app) const MOST_THEME_BYTES: u64 = 1024 * 1024;
/// How many `extends` links a chain may have.
pub(in crate::app) const MOST_CHAIN_LINKS: usize = 8;

/// Token ids a theme file may still use after the token was renamed, merged
/// or removed, each with the id it is read as now. A theme written against an
/// older Baboon keeps every colour.
pub(in crate::app) const RENAMES: &[(&str, &str)] = &[];

/// One user theme file the folder holds.
#[derive(Clone, Debug)]
pub(in crate::app) struct ListedTheme {
    pub(in crate::app) path: PathBuf,
    pub(in crate::app) parsed: Result<(ThemeFile, Vec<Report>), Report>,
}

/// A file's modified time and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct Fingerprint {
    pub(in crate::app) modified: Option<std::time::SystemTime>,
    pub(in crate::app) len: u64,
}

impl Fingerprint {
    pub(in crate::app) fn of(path: &Path) -> Option<Fingerprint> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Fingerprint {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// A theme as a menu lists it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct ThemeEntry {
    pub(in crate::app) id: String,
    pub(in crate::app) name: String,
    /// `None` for a file whose appearance comes from no file or parent.
    pub(in crate::app) appearance: Option<Appearance>,
    pub(in crate::app) built_in: bool,
    /// It cannot be applied.
    pub(in crate::app) broken: bool,
}

/// What resolving a theme gives.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct Resolved {
    pub(in crate::app) colors: ThemeColors,
    pub(in crate::app) reports: Vec<Report>,
    /// The files it was read from, child first, for the reload check.
    pub(in crate::app) files: Vec<PathBuf>,
    /// The tokens its files set; the rest come from a built-in, or follow
    /// what it set.
    pub(in crate::app) explicit: std::collections::BTreeSet<Token>,
    /// The tokens it left unset that follow tokens it set (step 6).
    pub(in crate::app) following: std::collections::BTreeSet<Token>,
    /// The built-in its chain ends at, if it ends at one.
    pub(in crate::app) root_built_in: Option<BuiltIn>,
    /// Its display name.
    pub(in crate::app) name: String,
}

/// The built-ins and the themes folder's files.
#[derive(Clone, Debug, Default)]
pub(in crate::app) struct ThemeLibrary {
    themes: BTreeMap<String, ListedTheme>,
    /// Problems with the folder itself: skipped files, reserved and
    /// duplicate ids.
    pub(in crate::app) reports: Vec<Report>,
}

impl ThemeLibrary {
    /// Reads every `.json` file in `folder`. A missing folder holds nothing.
    pub(in crate::app) fn scan(folder: &Path) -> ThemeLibrary {
        let mut library = ThemeLibrary::default();
        let Ok(entries) = std::fs::read_dir(folder) else {
            return library;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            })
            .collect();
        // Folder order differs by platform: sort, so the same file always
        // wins a duplicate id.
        paths.sort();
        for path in paths {
            let file_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let Some(id) = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_lowercase())
            else {
                continue;
            };
            if id == "baboon-theme.schema" {
                continue;
            }
            let Some(fingerprint) = Fingerprint::of(&path) else {
                continue;
            };
            if BuiltIn::from_id(&id).is_some() {
                library.reports.push(Report::new(
                    &file_name,
                    None,
                    format!("{id} is a built-in theme's id; the file is skipped"),
                ));
                continue;
            }
            if library.themes.contains_key(&id) {
                library.reports.push(Report::new(
                    &file_name,
                    None,
                    format!("another file already has the id {id}; skipped"),
                ));
                continue;
            }
            if fingerprint.len > MOST_THEME_BYTES {
                library.reports.push(Report::new(
                    &file_name,
                    None,
                    "is larger than 1 MiB; skipped",
                ));
                continue;
            }
            let parsed = match std::fs::read_to_string(&path) {
                Ok(text) => parse_theme(&id, &file_name, &text),
                Err(error) => Err(Report::new(
                    &file_name,
                    None,
                    format!("could not be read: {error}"),
                )),
            };
            library.themes.insert(id, ListedTheme { path, parsed });
        }
        library
    }

    /// A library of files already read, for tests.
    #[cfg(test)]
    pub(in crate::app) fn from_texts(files: &[(&str, &str)]) -> ThemeLibrary {
        let mut library = ThemeLibrary::default();
        for (id, text) in files {
            let file_name = format!("{id}.json");
            library.themes.insert(
                (*id).to_owned(),
                ListedTheme {
                    path: PathBuf::from(&file_name),
                    parsed: parse_theme(id, &file_name, text),
                },
            );
        }
        library
    }

    /// This library with `file` as the theme `id`, read from `path`, in
    /// place of any file with that id: how the theme editor resolves its
    /// draft.
    pub(in crate::app) fn with_file(
        &self,
        id: &str,
        path: PathBuf,
        file: ThemeFile,
    ) -> ThemeLibrary {
        let mut library = self.clone();
        library.themes.insert(
            id.to_owned(),
            ListedTheme {
                path,
                parsed: Ok((file, Vec::new())),
            },
        );
        library
    }

    /// Every theme, the built-ins first, then the files by name.
    pub(in crate::app) fn entries(&self) -> Vec<ThemeEntry> {
        let mut entries: Vec<ThemeEntry> = BuiltIn::ALL
            .into_iter()
            .map(|built_in| ThemeEntry {
                id: built_in.id().to_owned(),
                name: built_in.name().to_owned(),
                appearance: Some(built_in.appearance()),
                built_in: true,
                broken: false,
            })
            .collect();
        let mut files: Vec<ThemeEntry> = self
            .themes
            .iter()
            .map(|(id, listed)| {
                let resolved = self.resolve(id);
                ThemeEntry {
                    id: id.clone(),
                    name: listed
                        .parsed
                        .as_ref()
                        .map_or_else(|_| id.clone(), |(file, _)| file.name.clone()),
                    appearance: resolved
                        .as_ref()
                        .ok()
                        .map(|resolved| resolved.colors.appearance),
                    built_in: false,
                    broken: resolved.is_err(),
                }
            })
            .collect();
        files.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        entries.extend(files);
        entries
    }

    pub(in crate::app) fn listed(&self, id: &str) -> Option<&ListedTheme> {
        self.themes.get(id)
    }

    /// The theme `id` resolved, or why it cannot be applied.
    pub(in crate::app) fn resolve(&self, id: &str) -> Result<Resolved, Report> {
        self.resolve_with_renames(id, RENAMES)
    }

    pub(in crate::app) fn resolve_with_renames(
        &self,
        id: &str,
        renames: &[(&str, &str)],
    ) -> Result<Resolved, Report> {
        if let Some(built_in) = BuiltIn::from_id(id) {
            return Ok(Resolved {
                colors: built_in.colors(),
                reports: Vec::new(),
                files: Vec::new(),
                explicit: Token::ALL.iter().copied().collect(),
                following: Default::default(),
                root_built_in: None,
                name: built_in.name().to_owned(),
            });
        }
        // 1. The chain, child first, to a built-in or a theme written from
        // scratch.
        let mut chain: Vec<(&str, &ListedTheme, &ThemeFile)> = Vec::new();
        let mut reports = Vec::new();
        let mut root_built_in = None;
        let mut current = id.to_owned();
        loop {
            if let Some(built_in) = BuiltIn::from_id(&current) {
                root_built_in = Some(built_in);
                break;
            }
            let Some((key, listed)) = self.themes.get_key_value(current.as_str()) else {
                let child = chain.last().map(|(id, _, _)| *id);
                match child {
                    None => {
                        return Err(Report::new(
                            &format!("{id}.json"),
                            None,
                            "there is no theme with this id",
                        ));
                    }
                    Some(child) => {
                        reports.push(Report::new(
                            &file_name(self, child),
                            Some("extends"),
                            format!(
                                "names no theme ({current}); the built-in of its appearance is used"
                            ),
                        ));
                        break;
                    }
                }
            };
            if chain.iter().any(|(seen, _, _)| *seen == key) {
                return Err(Report::new(
                    &file_name(self, id),
                    Some("extends"),
                    format!("goes round in a circle through {key}"),
                ));
            }
            let file = match &listed.parsed {
                Ok((file, file_reports)) => {
                    reports.extend(file_reports.iter().cloned());
                    file
                }
                Err(report) => return Err(report.clone()),
            };
            chain.push((key, listed, file));
            if chain.len() > MOST_CHAIN_LINKS + 1 {
                return Err(Report::new(
                    &file_name(self, id),
                    Some("extends"),
                    format!("is more than {MOST_CHAIN_LINKS} links deep"),
                ));
            }
            match &file.extends {
                Some(parent) => current = parent.clone(),
                None => break,
            }
        }
        // 2 and 3. Merge root first, per palette name and per token, then
        // look up each `$name` in the merged palette.
        let mut palette: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
        let mut set: BTreeMap<Token, (&str, &str, &str)> = BTreeMap::new();
        for (theme, _, file) in chain.iter().rev() {
            for (name, value) in &file.palette {
                palette.insert(name, (value, theme));
            }
            for (key, value) in &file.colors {
                let key = key.as_str();
                let id = match renames.iter().find(|(old, _)| *old == key) {
                    Some((_, new)) => {
                        reports.push(Report::new(
                            &file_name(self, theme),
                            Some(key),
                            format!("was renamed {new}; read as {new}"),
                        ));
                        *new
                    }
                    None => key,
                };
                match Token::from_id(id) {
                    Some(token) => {
                        set.insert(token, (value, theme, key));
                    }
                    None => reports.push(Report::new(
                        &file_name(self, theme),
                        Some(key),
                        "is not a token id; ignored",
                    )),
                }
            }
        }
        let mut explicit = BTreeMap::new();
        for (token, (value, theme, key)) in &set {
            let written = match value.strip_prefix('$') {
                Some(name) => match palette.get(name) {
                    Some((value, _)) => *value,
                    None => {
                        reports.push(Report::new(
                            &file_name(self, theme),
                            Some(key),
                            format!("${name} is not in the palette; ignored"),
                        ));
                        continue;
                    }
                },
                None => *value,
            };
            match parse_color(written) {
                Some(color) => {
                    explicit.insert(*token, color);
                }
                None => reports.push(Report::new(
                    &file_name(self, theme),
                    Some(key),
                    format!("{written} is not a colour; ignored"),
                )),
            }
        }
        for (name, (value, theme)) in &palette {
            if parse_color(value).is_none() {
                reports.push(Report::new(
                    &file_name(self, theme),
                    Some(&format!("palette.{name}")),
                    format!("{value} is not a colour"),
                ));
            }
        }
        // The appearance: the nearest file that says, else the built-in the
        // chain ends at, else a guess from the background.
        let appearance = chain
            .iter()
            .find_map(|(_, _, file)| file.appearance)
            .or(root_built_in.map(BuiltIn::appearance))
            .unwrap_or_else(|| {
                let guess = match explicit.get(&Token::Background) {
                    Some(background) if luminance(*background) > 0.2 => Appearance::Light,
                    _ => Appearance::Dark,
                };
                reports.push(Report::new(
                    &file_name(self, id),
                    Some("appearance"),
                    format!("is missing; {} is guessed from the background", guess.key()),
                ));
                guess
            });
        // 4 and 5. The fallback is a built-in: the chain's, or, for a theme
        // written from scratch, the one of its appearance.
        let fallback = root_built_in.unwrap_or_else(|| BuiltIn::for_appearance(appearance));
        if root_built_in.is_none() {
            let unset: Vec<&str> = Token::ALL
                .iter()
                .filter(|token| !explicit.contains_key(token))
                .map(|token| token.id())
                .collect();
            if !unset.is_empty() {
                let root = chain.last().map_or(id, |(theme, _, _)| *theme);
                reports.push(Report::new(
                    &file_name(self, root),
                    Some("colors"),
                    format!(
                        "leaves {} token(s) unset, taken from {} or following what it sets: {}",
                        unset.len(),
                        fallback.id(),
                        unset.join(", ")
                    ),
                ));
            }
        }
        let mut colors = fallback.colors();
        colors.appearance = appearance;
        for (token, color) in &explicit {
            colors.set(*token, *color);
        }
        // 6. Dependents follow: a token left unset whose rule reads a token
        // the theme set, directly or through another, takes its rule's colour
        // rather than the built-in's.
        let mut moved: std::collections::BTreeSet<Token> = explicit.keys().copied().collect();
        let mut following = std::collections::BTreeSet::new();
        for token in Token::ALL {
            if explicit.contains_key(token) {
                continue;
            }
            if let Some(rule) = token.rules().of(appearance)
                && rule.inputs().iter().any(|input| moved.contains(input))
            {
                let color = rule.evaluate(|input| colors.get(input));
                colors.set(*token, color);
                moved.insert(*token);
                following.insert(*token);
            }
        }
        Ok(Resolved {
            colors,
            reports,
            files: chain
                .iter()
                .map(|(_, listed, _)| listed.path.clone())
                .collect(),
            explicit: explicit.keys().copied().collect(),
            following,
            root_built_in,
            name: chain
                .first()
                .map_or_else(|| id.to_owned(), |(_, _, file)| file.name.clone()),
        })
    }
}

fn file_name(library: &ThemeLibrary, id: &str) -> String {
    library
        .themes
        .get(id)
        .and_then(|listed| listed.path.file_name())
        .map_or_else(
            || format!("{id}.json"),
            |name| name.to_string_lossy().into_owned(),
        )
}

/// Relative luminance, 0 for black and 1 for white.
pub(in crate::app) fn luminance(color: eframe::egui::Color32) -> f32 {
    let linear = eframe::egui::Rgba::from(color);
    0.2126 * linear.r() + 0.7152 * linear.g() + 0.0722 * linear.b()
}

/// What is wrong with a rename table: a rename to an id no token has, or one
/// that hides a live token's id.
#[cfg(test)]
pub(in crate::app) fn rename_problems(renames: &[(&str, &str)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (old, new) in renames {
        if Token::from_id(new).is_none() {
            problems.push(format!("{old} is renamed {new}, which no token has"));
        }
        if Token::from_id(old).is_some() {
            problems.push(format!("{old} is renamed, but a token still has it"));
        }
    }
    problems
}
