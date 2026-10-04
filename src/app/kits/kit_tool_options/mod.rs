//! The `-tags_dir`/`-data_dir` options a Halo CE or Halo 2 kit's tools are
//! started with when the kit uses folders other than its root's own.
//! It owns tool-option assembly; launching tools belongs to the controller.

use super::*;

/// The options a kit's tools need to work on its folders: `-tags_dir` and
/// `-data_dir` for each that isn't the root's own, for an engine whose tools
/// take them, and nothing otherwise.
///
/// The Halo 3-era tools take no such options (they open `tags\` and `data\`
/// relative to the folder they run in), and their kits can't choose folders.
pub(in crate::app) fn kit_tool_folder_options(
    layout: &KitLayout,
    game: Option<GameId>,
) -> Vec<(&'static str, PathBuf)> {
    if !game.is_some_and(GameFacts::tools_take_folder_arguments) {
        return Vec::new();
    }
    let mut options = Vec::new();
    if !layout.tags_is_root_tags_folder() {
        options.push(("-tags_dir", layout.tags.clone()));
    }
    if !layout.data_is_root_data_folder() {
        options.push(("-data_dir", layout.data.clone()));
    }
    options
}

/// `command` with the kit's folder options after its program, when the
/// program is the kit's `tool` (`tool`, `tool.exe`, `.\tool.exe`, quoted or
/// not). Any other command is left alone.
pub(in crate::app) fn with_tool_folder_options(
    command: &str,
    options: &[(&'static str, PathBuf)],
) -> String {
    if options.is_empty() {
        return command.to_owned();
    }
    let trimmed = command.trim_start();
    let program_len = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.find('"').map(|end| end + 2).unwrap_or(trimmed.len())
    } else {
        trimmed.find(char::is_whitespace).unwrap_or(trimmed.len())
    };
    let (program, rest) = trimmed.split_at(program_len);
    // Split on both separators: the command is a Windows command line, read
    // the same way whatever platform this test or build runs on.
    let name = program
        .trim_matches('"')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_tool = name == "tool" || name == "tool.exe";
    if !is_tool {
        return command.to_owned();
    }
    let options = options
        .iter()
        .map(|(option, path)| format!("{option} {}", crt_quoted(&path.display().to_string())))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{program} {options}{rest}")
}

/// `argument` in double quotes, as the Microsoft C runtime the tools parse
/// their command line with reads it back.
///
/// The runtime takes backslashes literally except before a quote, where `\"`
/// is a literal quote and a doubled backslash is one backslash. So a folder
/// ending in a backslash, such as a drive root `D:\`, came out as `"D:\"`: its
/// closing quote read as part of the argument, which swallowed the rest of
/// the command line. Trailing backslashes are doubled to keep them; a
/// Windows path holds no quotes, so nothing else needs escaping.
fn crt_quoted(argument: &str) -> String {
    let trailing = argument.len() - argument.trim_end_matches('\\').len();
    format!("\"{argument}{}\"", "\\".repeat(trailing))
}

impl Baboon {
    /// The active kit's tool folder options; see [`kit_tool_folder_options`].
    pub(in crate::app) fn active_kit_tool_folder_options(&self) -> Vec<(&'static str, PathBuf)> {
        let Some(layout) = self.kit_layout_for(self.model.active) else {
            return Vec::new();
        };
        kit_tool_folder_options(
            &layout,
            self.source().and_then(|source| source.game),
        )
    }
}

#[cfg(test)]
mod tests;
