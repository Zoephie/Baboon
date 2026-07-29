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
    match flag.to_ascii_uppercase().as_str() {
        "-HCEEK" | "-H1EK" => Some(("haloce_mcc", "HCEEK")),
        "-H2EK" => Some(("halo2_mcc", "H2EK")),
        "-H3EK" => Some(("halo3_mcc", "H3EK")),
        "-H3ODSTEK" => Some(("halo3odst_mcc", "H3ODSTEK")),
        "-HREK" => Some(("haloreach_mcc", "HREK")),
        "-H4EK" => Some(("halo4_mcc", "H4EK")),
        "-H2AMPEK" | "-H2AEK" => Some(("halo2amp_mcc", "H2AMPEK")),
        _ => None,
    }
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
    for path in resolved.paths {
        match loose_file_entry(tags_root, &path, names) {
            Ok(Some(entry)) => entries.push(entry),
            Ok(None) => errors.push(format!("{} is not a supported tag", path.display())),
            Err(error) => errors.push(format!("{}: {error:#}", path.display())),
        }
    }
    Ok(ResolvedLaunchEntries { entries, errors })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "baboon-command-line-{label}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn write_classic_tag(path: &Path, group: &[u8; 4]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = [0u8; 64];
        bytes[36..40].copy_from_slice(group);
        bytes[60..64].copy_from_slice(b"blam");
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn no_arguments_preserve_normal_startup() {
        let parsed = parse_startup_arguments(Vec::<OsString>::new());
        assert_eq!(parsed, StartupArguments::Normal);
        assert!(!parsed.suppresses_startup_popups());
    }

    #[test]
    fn every_builtin_flag_and_alias_is_case_insensitive() {
        for (flag, game) in [
            ("-HCEEK", "haloce_mcc"),
            ("-h1ek", "haloce_mcc"),
            ("-H2EK", "halo2_mcc"),
            ("-h3ek", "halo3_mcc"),
            ("-H3ODSTEK", "halo3odst_mcc"),
            ("-hrek", "haloreach_mcc"),
            ("-H4EK", "halo4_mcc"),
            ("-H2AMPEK", "halo2amp_mcc"),
            ("-h2aek", "halo2amp_mcc"),
        ] {
            let StartupArguments::Launch(launch) =
                parse_startup_arguments(args(&[flag, "objects/example.weapon"]))
            else {
                panic!("{flag} should parse");
            };
            assert_eq!(launch.game, game);
        }
    }

    #[test]
    fn parser_preserves_multiple_unicode_paths() {
        let StartupArguments::Launch(launch) = parse_startup_arguments(args(&[
            "-HREK",
            "objects/éclair.weapon",
            "objects/二.model",
        ])) else {
            panic!("launch should parse");
        };
        assert_eq!(
            launch.tag_paths,
            [
                PathBuf::from("objects/éclair.weapon"),
                PathBuf::from("objects/二.model")
            ]
        );
        assert!(StartupArguments::Launch(launch).suppresses_startup_popups());
    }

    #[test]
    fn malformed_command_lines_suppress_startup_popups() {
        for parsed in [
            parse_startup_arguments(args(&["-UNKNOWN", "objects/example.weapon"])),
            parse_startup_arguments(args(&["-HREK"])),
        ] {
            assert!(matches!(parsed, StartupArguments::Invalid(_)));
            assert!(parsed.suppresses_startup_popups());
        }
    }

    #[test]
    fn paths_resolve_under_root_deduplicate_and_reject_outside_files() {
        let base = unique_test_dir("paths");
        let tags = base.join("tags");
        let outside = base.join("outside.weapon");
        let relative = PathBuf::from("objects")
            .join("path with spaces")
            .join("éclair.weapon");
        let inside = tags.join(&relative);
        fs::create_dir_all(inside.parent().unwrap()).unwrap();
        fs::write(&inside, b"tag").unwrap();
        fs::write(&outside, b"tag").unwrap();

        let resolved = resolve_launch_tag_paths(
            &tags,
            &[
                relative.clone(),
                inside.clone(),
                outside.clone(),
                PathBuf::from("../outside.weapon"),
                PathBuf::from("missing.weapon"),
            ],
        )
        .unwrap();

        assert_eq!(resolved.paths, [fs::canonicalize(&inside).unwrap()]);
        assert_eq!(resolved.errors.len(), 3);
        assert!(
            resolved
                .errors
                .iter()
                .any(|error| error.contains("outside"))
        );
        assert!(
            resolved
                .errors
                .iter()
                .any(|error| error.contains("missing.weapon"))
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn forward_slash_relative_paths_resolve() {
        let base = unique_test_dir("slashes");
        let tags = base.join("tags");
        let inside = tags.join("objects").join("weapon.weapon");
        fs::create_dir_all(inside.parent().unwrap()).unwrap();
        fs::write(&inside, b"tag").unwrap();

        let resolved =
            resolve_launch_tag_paths(&tags, &[PathBuf::from("objects/weapon.weapon")]).unwrap();

        assert_eq!(resolved.paths, [fs::canonicalize(&inside).unwrap()]);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn supported_entries_keep_argument_order_and_invalid_files_do_not_block_them() {
        let base = unique_test_dir("entries");
        let tags = base.join("tags");
        let first = tags.join("objects").join("first.weapon");
        let second = tags.join("objects").join("second.weapon");
        let unsupported = tags.join("objects").join("notes.txt");
        write_classic_tag(&first, b"weap");
        write_classic_tag(&second, b"weap");
        fs::write(&unsupported, b"not a tag").unwrap();

        let resolved = resolve_launch_tag_entries(
            &tags,
            &[
                PathBuf::from("objects/first.weapon"),
                PathBuf::from("objects/notes.txt"),
                first.clone(),
                PathBuf::from("objects/second.weapon"),
            ],
            &TagNameIndex::default(),
        )
        .unwrap();

        let entry_paths = resolved
            .entries
            .iter()
            .map(|entry| match &entry.location {
                TagEntryLocation::LooseFile(path) => path.clone(),
                _ => panic!("expected loose file"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            entry_paths,
            [
                fs::canonicalize(&first).unwrap(),
                fs::canonicalize(&second).unwrap()
            ]
        );
        assert_eq!(resolved.errors.len(), 1);
        assert!(resolved.errors[0].contains("not a supported tag"));
        let _ = fs::remove_dir_all(base);
    }
}
