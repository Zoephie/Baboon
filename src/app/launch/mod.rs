//! Command-line startup parsing and loose-tag path resolution.

use super::*;
use std::ffi::OsString;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandLineLaunch {
    pub(crate) game: &'static str,
    pub(crate) kit_label: &'static str,
    pub(crate) tag_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartupArguments {
    Normal,
    Launch(CommandLineLaunch),
    Invalid(String),
}

impl StartupArguments {
    pub(super) fn suppresses_startup_popups(&self) -> bool {
        !matches!(self, Self::Normal)
    }
}

pub(crate) fn parse_startup_arguments<I>(arguments: I) -> StartupArguments
where
    I: IntoIterator<Item = OsString>,
{
    let mut arguments = arguments.into_iter();
    let Some(flag) = arguments.next() else {
        return StartupArguments::Normal;
    };
    let Some(flag) = flag.to_str() else {
        return StartupArguments::Invalid("The editing-kit flag is not valid Unicode".to_owned());
    };
    let Some((game, kit_label)) = command_line_kit(flag) else {
        return StartupArguments::Invalid(format!(
            "Unknown editing-kit flag {flag}. Expected -HCEEK/-H1EK, -H2EK, -H3EK, \
             -H3ODSTEK, -HREK, -H4EK, or -H2AMPEK/-H2AEK"
        ));
    };
    let tag_paths = arguments.map(PathBuf::from).collect::<Vec<_>>();
    if tag_paths.is_empty() {
        return StartupArguments::Invalid(format!("{flag} requires at least one tag path"));
    }
    StartupArguments::Launch(CommandLineLaunch {
        game,
        kit_label,
        tag_paths,
    })
}

fn command_line_kit(flag: &str) -> Option<(&'static str, &'static str)> {
    let game = game_for_launch_flag(flag)?;
    Some((game.as_str(), game.kit_name()?))
}

pub(super) struct ResolvedLaunchPaths {
    pub(super) paths: Vec<PathBuf>,
    pub(super) errors: Vec<String>,
}

pub(super) struct ResolvedLaunchEntries {
    pub(super) entries: Vec<TagEntry>,
    pub(super) errors: Vec<String>,
}

pub(super) fn resolve_launch_tag_paths(
    tags_root: &Path,
    requested: &[PathBuf],
) -> Result<ResolvedLaunchPaths, String> {
    let tags_root = fs::canonicalize(tags_root).map_err(|error| {
        format!(
            "Could not resolve editing-kit tags folder {}: {error}",
            tags_root.display()
        )
    })?;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut errors = Vec::new();
    for requested_path in requested {
        let candidate = if requested_path.is_absolute() {
            requested_path.clone()
        } else {
            tags_root.join(requested_path)
        };
        let path = match fs::canonicalize(&candidate) {
            Ok(path) => path,
            Err(error) => {
                errors.push(format!("{}: {error}", requested_path.display()));
                continue;
            }
        };
        if !path.starts_with(&tags_root) {
            errors.push(format!(
                "{} is outside {}",
                requested_path.display(),
                tags_root.display()
            ));
            continue;
        }
        if !path.is_file() {
            errors.push(format!("{} is not a file", requested_path.display()));
            continue;
        }
        if paths
            .iter()
            .any(|existing| same_recent_path(existing, &path))
        {
            continue;
        }
        paths.push(path);
    }
    Ok(ResolvedLaunchPaths { paths, errors })
}

pub(super) fn resolve_launch_tag_entries(
    tags_root: &Path,
    requested: &[PathBuf],
    names: &TagNameIndex,
) -> Result<ResolvedLaunchEntries, String> {
    let resolved = resolve_launch_tag_paths(tags_root, requested)?;
    let mut errors = resolved.errors;
    let mut entries = Vec::new();
    for canonical in resolved.paths {
        // Spelled on the root the source holds, so the key is the one the
        // folder scan makes; the canonical form is for the containment check.
        let path = match crate::core::source::path_on_root(tags_root, &canonical) {
            Ok(Some(path)) => path,
            Ok(None) => {
                errors.push(format!(
                    "{} is outside {}",
                    canonical.display(),
                    tags_root.display()
                ));
                continue;
            }
            Err(error) => {
                errors.push(format!("{}: {error}", canonical.display()));
                continue;
            }
        };
        match loose_file_entry(tags_root, &path, names) {
            Ok(Some(entry)) => entries.push(entry),
            Ok(None) => errors.push(format!("{} is not a supported tag", path.display())),
            Err(error) => errors.push(format!("{}: {error:#}", path.display())),
        }
    }
    Ok(ResolvedLaunchEntries { entries, errors })
}

#[cfg(test)]
mod tests;
