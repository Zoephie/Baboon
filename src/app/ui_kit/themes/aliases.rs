//! The accessors in `ui_kit/style.rs` that return one token's colour, named
//! for where they are used (`save_error_text` is `status.error`). The Theme
//! Editor searches them, so an author who knows a feature's name finds its
//! token. A test keeps the table equal to `style.rs`.

use super::Token;

/// Each accessor's name and the token it returns, by name.
pub(in crate::app) const ALIASES: &[(&str, Token)] = &[
    ("accept_text", Token::StatusSuccess),
    ("active_tab", Token::TabActiveBackground),
    ("add_icon", Token::StatusSuccess),
    ("added_text", Token::DiffAdded),
    ("added_wash", Token::DiffAddedBackground),
    ("bitmap_error_text", Token::StatusError),
    ("browser_search_bg", Token::BrowserSearchBackground),
    (
        "browser_toolbar_active",
        Token::BrowserToolbarActiveBackground,
    ),
    ("browser_toolbar_bg", Token::BrowserToolbarBackground),
    ("button_destructive_accent", Token::ButtonDestructive),
    ("chimp_archive_text", Token::ChimpArchiveText),
    ("chimp_browser_error_text", Token::StatusError),
    ("chimp_error_text", Token::StatusError),
    ("chimp_raw_value_text", Token::StatusWarning),
    ("chrome_bar", Token::ChromeBackground),
    ("chrome_edge", Token::ChromeBorder),
    ("clipboard_copy_mark", Token::ClipboardCopyBorder),
    ("clipboard_cut_mark", Token::ClipboardCutBorder),
    ("context_menu_hover", Token::MenuHover),
    ("control_well", Token::ControlWell),
    ("destructive_icon", Token::ActionDestructive),
    ("dirty_tab_tint", Token::TabModifiedTint),
    ("disclosure_triangle_blue", Token::TreeFolderClosedIcon),
    ("disclosure_triangle_green", Token::TreeFolderOpenIcon),
    ("disk_banner_edge", Token::BannerWarningBorder),
    ("disk_banner_fill", Token::BannerWarningBackground),
    ("disk_banner_icon", Token::BannerWarningIcon),
    ("disk_missing_edge", Token::BannerErrorBorder),
    ("disk_missing_fill", Token::BannerErrorBackground),
    ("disk_missing_icon", Token::BannerErrorIcon),
    ("editor_bg", Token::Background),
    ("error_text", Token::StatusError),
    ("expert_mode_text", Token::StatusWarning),
    ("favourite_star", Token::FavouriteIcon),
    ("filter_warning_text", Token::StatusWarning),
    ("first_run_error_text", Token::StatusError),
    ("focused_tab_border", Token::FocusBorder),
    ("foundation_block_bar", Token::BlockHeaderBackground),
    ("foundation_block_edge", Token::BlockBorder),
    ("foundation_block_text", Token::BlockHeaderText),
    ("foundation_blue", Token::AccentText),
    ("foundation_documentation_bg", Token::CardBackground),
    ("foundation_flag_hover", Token::FlagsHover),
    ("foundation_group_bg", Token::GroupBackground),
    ("foundation_group_edge", Token::GroupBorder),
    ("foundation_input", Token::InputBackground),
    ("foundation_input_edge", Token::InputBorder),
    ("foundation_input_read_only", Token::InputBackground),
    ("foundation_jump_cyan", Token::BlockJumpIcon),
    ("foundation_section_bar", Token::SectionHeaderBackground),
    ("function_grid_line", Token::GraphGrid),
    ("function_plot_bg", Token::GraphBackground),
    ("good_news", Token::StatusSuccess),
    ("graph_band_edge", Token::GraphBandBorder),
    ("graph_band_fill", Token::GraphBandBackground),
    ("graph_container_fill", Token::GraphContainerBackground),
    ("grid_line", Token::Border),
    ("group_badge_edge", Token::BadgeBorder),
    ("group_badge_fill", Token::BadgeBackground),
    ("group_badge_text", Token::BadgeText),
    ("header_notice_text", Token::StatusWarning),
    ("help_code_bg", Token::CodeBackground),
    ("help_cue", Token::FieldHelpText),
    ("import_button_fill", Token::ButtonPrimaryBackground),
    ("import_warning_text", Token::StatusWarning),
    ("installed_overwrite_text", Token::StatusError),
    ("invalid_value_fill", Token::StatusErrorBackground),
    ("invalid_value_text", Token::StatusError),
    ("jump_glow", Token::BlockJumpHighlight),
    ("left_panel", Token::PanelBackground),
    ("loading_lines", Token::LoadingLines),
    ("loading_primary", Token::LoadingPrimary),
    ("loading_secondary", Token::LoadingSecondary),
    ("markup_tag_text", Token::SyntaxMarkupTag),
    ("markup_warning_text", Token::SyntaxMarkupWarning),
    ("match_tick", Token::StatusSuccess),
    ("material_data_row", Token::ShaderRowDataBackground),
    ("material_editor_box", Token::MaterialBoxBackground),
    ("material_editor_data_row", Token::MaterialRowDataBackground),
    (
        "material_editor_function_row",
        Token::MaterialRowFunctionBackground,
    ),
    ("material_editor_grid", Token::MaterialGrid),
    ("material_editor_input_edge", Token::MaterialInputBorder),
    ("material_editor_muted_text", Token::MaterialTextMuted),
    ("material_editor_text", Token::MaterialText),
    ("material_function_row", Token::ShaderRowFunctionBackground),
    ("material_grid_light", Token::ShaderGrid),
    ("material_input_edge", Token::ShaderInputBorder),
    ("material_muted_text", Token::ShaderTextMuted),
    ("material_numeric_row", Token::ShaderRowNumericBackground),
    ("material_panel", Token::ShaderPanelBackground),
    ("material_panel_edge", Token::ShaderPanelBorder),
    ("material_ref_row", Token::ShaderRowReferenceBackground),
    ("material_text", Token::ShaderText),
    ("menu_bar", Token::MenuBarBackground),
    ("menu_row_hover", Token::MenuBarItemHover),
    ("modal_scrim", Token::ModalBackdrop),
    ("modified_text", Token::DiffModified),
    ("open_folder_icon", Token::TreeFolderOpenIcon),
    ("panel_error_text", Token::StatusError),
    ("playhead", Token::AccentText),
    ("progress_fill", Token::ProgressBar),
    ("progress_track", Token::ProgressBackground),
    ("read_only_badge", Token::BadgeReadOnlyBackground),
    ("read_only_badge_mark", Token::BadgeReadOnlyIcon),
    ("reference_missing_text", Token::StatusError),
    ("removed_text", Token::DiffRemoved),
    ("removed_wash", Token::DiffRemovedBackground),
    ("replace_warning_text", Token::StatusError),
    ("review_error_text", Token::StatusError),
    ("row_type", Token::RowBackground),
    ("save_error_text", Token::StatusError),
    ("search_active_match_bg", Token::SearchMatchActiveBackground),
    ("search_match_bg", Token::SearchMatchBackground),
    ("search_match_text", Token::SearchMatchText),
    ("settings_card_fill", Token::SettingsCardBackground),
    (
        "settings_card_invalid_fill",
        Token::SettingsCardErrorBackground,
    ),
    ("settings_warning_text", Token::StatusWarning),
    ("shader_modified_accent", Token::ShaderRowModifiedMarker),
    ("slider_rail_edge", Token::SliderRailBorder),
    ("slider_rail_fill", Token::SliderRailBackground),
    ("subtle_dark", Token::TextMuted),
    ("terminal_bg", Token::TerminalBackground),
    ("terminal_error_text", Token::TerminalError),
    ("terminal_success_text", Token::TerminalSuccess),
    ("terminal_text", Token::TerminalText),
    ("terminal_warning_text", Token::TerminalWarning),
    ("text_dark", Token::Text),
    ("text_strong", Token::TextStrong),
    ("unapplied_value_fill", Token::StatusWarningBackground),
    ("unapplied_value_text", Token::StatusWarning),
    ("unchanged_text", Token::DiffUnchanged),
    ("unused_value_text", Token::ActionDestructive),
    ("value_decreased", Token::ActionDestructive),
    ("welcome_side_panel", Token::WelcomePanelBackground),
];

/// The accessors that return `token`.
pub(in crate::app) fn of(token: Token) -> impl Iterator<Item = &'static str> {
    ALIASES
        .iter()
        .filter(move |(_, aliased)| *aliased == token)
        .map(|(name, _)| *name)
}

/// The accessors in `source` whose whole body is one token's colour.
#[cfg(test)]
fn accessors_in(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let head = "pub(in crate::app) fn ";
    for (start, _) in source.match_indices(head) {
        let rest = &source[start + head.len()..];
        let Some((name, rest)) = rest.split_once("() -> Color32 {") else {
            continue;
        };
        if name.contains(|c: char| !(c.is_alphanumeric() || c == '_')) {
            continue;
        }
        let body = rest.split('}').next().unwrap_or_default();
        let body: String = body.split_whitespace().collect();
        if let Some(token) = body
            .strip_prefix("themes::color(themes::Token::")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            found.push((name.to_owned(), token.to_owned()));
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Vec<(String, String)> {
        let mut table: Vec<(String, String)> = ALIASES
            .iter()
            .map(|(name, token)| ((*name).to_owned(), format!("{token:?}")))
            .collect();
        table.sort();
        table
    }

    #[test]
    fn the_alias_table_matches_the_style_module() {
        let source = include_str!("../style.rs");
        let found = accessors_in(source);
        assert!(found.len() > 100, "the scan finds the accessors");
        assert_eq!(table(), found, "regenerate ALIASES from ui_kit/style.rs");
        // The check notices an accessor whose token changed.
        let altered = source.replacen(
            "themes::color(themes::Token::StatusError)",
            "themes::color(themes::Token::Text)",
            1,
        );
        assert_ne!(table(), accessors_in(&altered));
    }

    #[test]
    fn a_token_lists_its_aliases() {
        assert!(of(Token::StatusError).any(|name| name == "save_error_text"));
    }
}
