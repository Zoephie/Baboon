//! Editing-kit discovery and Steam library path parsing.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

pub(super) fn detect_editing_kit_paths() -> HashMap<String, PathBuf> {
    detect_editing_kit_paths_in_common_roots(steam_common_roots())
}

pub(super) fn detect_editing_kit_paths_in_common_roots<I>(
    common_roots: I,
) -> HashMap<String, PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut detected = HashMap::new();
    for common_root in common_roots {
        for shortcut in EDITING_KIT_SHORTCUTS {
            if detected.contains_key(shortcut.game) {
                continue;
            }
            let candidate = common_root.join(shortcut.label);
            if candidate.is_dir() && candidate.join("tags").is_dir() {
                detected.insert(shortcut.game.to_owned(), candidate);
            }
        }
    }
    detected
}

pub(super) fn apply_detected_editing_kit_paths(
    editing_kit_paths: &mut HashMap<String, PathBuf>,
    editing_kit_path_inputs: &mut HashMap<String, String>,
    editing_kit_path_attention: &mut Option<String>,
    detected: &HashMap<String, PathBuf>,
) -> usize {
    let mut added = 0;
    for shortcut in EDITING_KIT_SHORTCUTS {
        let has_existing = editing_kit_paths
            .get(shortcut.game)
            .is_some_and(|path| !path.as_os_str().is_empty());
        if has_existing {
            continue;
        }
        let Some(path) = detected.get(shortcut.game) else {
            continue;
        };
        editing_kit_paths.insert(shortcut.game.to_owned(), path.clone());
        editing_kit_path_inputs.insert(shortcut.game.to_owned(), path.display().to_string());
        if editing_kit_path_attention.as_deref() == Some(shortcut.game) {
            *editing_kit_path_attention = None;
        }
        added += 1;
    }
    added
}

fn steam_common_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for steam_root in default_steam_roots() {
        push_unique_path(&mut roots, steam_root.join("steamapps").join("common"));
        let library_file = steam_root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(library_file) {
            for library_root in parse_steam_library_paths(&text) {
                push_unique_path(&mut roots, library_root.join("steamapps").join("common"));
            }
        }
    }
    roots
}

fn default_steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(root) = std::env::var_os(var) {
            push_unique_path(&mut roots, PathBuf::from(root).join("Steam"));
        }
    }
    push_unique_path(&mut roots, PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots
}

pub(super) fn parse_steam_library_paths(text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let tokens = quoted_vdf_tokens(line);
        if tokens.len() >= 2 && tokens[0].eq_ignore_ascii_case("path") {
            push_unique_path(&mut paths, PathBuf::from(&tokens[1]));
        }
    }
    paths
}

fn quoted_vdf_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut in_quote = false;
    let mut escape = false;
    for ch in line.chars() {
        if !in_quote {
            if ch == '"' {
                in_quote = true;
                token.clear();
            }
            continue;
        }
        if escape {
            token.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '"' => {
                tokens.push(token.clone());
                token.clear();
                in_quote = false;
            }
            _ => token.push(ch),
        }
    }
    tokens
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| same_path_text(existing, &path)) {
        paths.push(path);
    }
}

fn same_path_text(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
