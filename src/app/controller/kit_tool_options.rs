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
pub(super) fn kit_tool_folder_options(
    layout: &KitLayout,
    game: Option<&str>,
) -> Vec<(&'static str, PathBuf)> {
    if !game.is_some_and(kit_folders_are_choosable) {
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
pub(super) fn with_tool_folder_options(
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
        .map(|(option, path)| format!("{option} \"{}\"", path.display()))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{program} {options}{rest}")
}

impl Baboon {
    /// The active kit's tool folder options; see [`kit_tool_folder_options`].
    pub(super) fn active_kit_tool_folder_options(&self) -> Vec<(&'static str, PathBuf)> {
        let Some(layout) = self.kit_layout_for(self.active) else {
            return Vec::new();
        };
        kit_tool_folder_options(
            &layout,
            self.source().and_then(|source| source.game.as_deref()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(tags: &str, data: &str) -> KitLayout {
        KitLayout {
            root: PathBuf::from("/ek"),
            tags: PathBuf::from(tags),
            data: PathBuf::from(data),
        }
    }

    #[test]
    fn only_folders_other_than_the_roots_own_become_options() {
        let stock = layout("/ek/tags", "/ek/data");
        let moda = layout("/ek/tags_moda", "/ek/data_moda");
        let tags_only = layout("/ek/tags_moda", "/ek/data");
        assert!(kit_tool_folder_options(&stock, Some("halo2_mcc")).is_empty());
        assert_eq!(
            kit_tool_folder_options(&moda, Some("haloce_mcc")),
            vec![
                ("-tags_dir", PathBuf::from("/ek/tags_moda")),
                ("-data_dir", PathBuf::from("/ek/data_moda")),
            ]
        );
        assert_eq!(
            kit_tool_folder_options(&tags_only, Some("halo2_mcc")),
            vec![("-tags_dir", PathBuf::from("/ek/tags_moda"))]
        );
        // The Halo 3-era tools can't take them, so they're never added there.
        assert!(kit_tool_folder_options(&moda, Some("halo3_mcc")).is_empty());
        assert!(kit_tool_folder_options(&moda, None).is_empty());
    }

    #[test]
    fn options_follow_the_tool_and_nothing_else() {
        let options = vec![
            ("-tags_dir", PathBuf::from("/ek/tags moda")),
            ("-data_dir", PathBuf::from("/ek/data_moda")),
        ];
        assert_eq!(
            with_tool_folder_options("tool bitmaps \"levels\\a\"", &options),
            "tool -tags_dir \"/ek/tags moda\" -data_dir \"/ek/data_moda\" bitmaps \"levels\\a\""
        );
        assert_eq!(
            with_tool_folder_options("\"tool.exe\" model x", &options),
            "\"tool.exe\" -tags_dir \"/ek/tags moda\" -data_dir \"/ek/data_moda\" model x"
        );
        assert_eq!(
            with_tool_folder_options(".\\tool.exe", &options),
            ".\\tool.exe -tags_dir \"/ek/tags moda\" -data_dir \"/ek/data_moda\""
        );
        assert_eq!(with_tool_folder_options("dir data", &options), "dir data");
        assert_eq!(with_tool_folder_options("toolkit x", &options), "toolkit x");
        assert_eq!(with_tool_folder_options("tool x", &[]), "tool x");
    }
}
