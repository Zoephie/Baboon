//! The colours Baboon painted before the theme registry existed, captured
//! from every colour accessor and from egui's visuals under the three looks
//! (`main` at `750178f3`), and the check that the built-in themes still paint
//! exactly them.

use super::super::style as s;
use super::{BuiltIn, with_colors};
use eframe::egui::Color32;

/// Each source's colour under the dark, light and classic light looks, as
/// premultiplied RGBA.
const GOLDEN: &[(&str, [u32; 3])] = &[
    ("menu_bar", [0x323232FF, 0xE5CCB2FF, 0xDBDBD8FF]),
    ("chrome_edge", [0x525252FF, 0xD0966AFF, 0xB4B4AEFF]),
    ("chrome_bar", [0x373737FF, 0xE5CCB2FF, 0xDBDBD8FF]),
    ("active_tab", [0x484848FF, 0xE5CCB2FF, 0xA1A19DFF]),
    ("foundation_blue", [0x86B8D5FF, 0xB4440CFF, 0x0F2B40FF]),
    ("left_panel", [0x1F1F1FFF, 0xEBE5E0FF, 0xEEEEEAFF]),
    ("editor_bg", [0x282828FF, 0xF2F1F0FF, 0xE0E0DCFF]),
    ("row_type", [0x373737FF, 0xECE8E4FF, 0xDBDBD8FF]),
    ("grid_line", [0x525252FF, 0xDED6CFFF, 0xB4B4AEFF]),
    ("foundation_group_bg", [0x2B2B2BFF, 0xF5F4F3FF, 0xECECEAFF]),
    (
        "foundation_documentation_bg",
        [0x0D0D0D0D, 0x776A5C80, 0x0000000D],
    ),
    (
        "foundation_group_edge",
        [0x525252FF, 0xD8CFC7FF, 0x989894FF],
    ),
    (
        "foundation_section_bar",
        [0x444444FF, 0xF2E7DBFF, 0xD6D6D2FF],
    ),
    ("foundation_block_bar", [0x484848FF, 0xF0E2D4FF, 0xCCCCC8FF]),
    (
        "foundation_block_text",
        [0xF2F2F2FF, 0x1C1815FF, 0x191918FF],
    ),
    ("playhead", [0x86B8D5FF, 0xB4440CFF, 0x0F2B40FF]),
    ("foundation_jump_cyan", [0x4DD0E1FF, 0x4DD0E1FF, 0x4DD0E1FF]),
    ("foundation_input", [0x0000004D, 0x776A5D80, 0x4D4D4D4D]),
    (
        "foundation_input_read_only",
        [0x0000004D, 0x776A5D80, 0x4D4D4D4D],
    ),
    (
        "foundation_input_edge",
        [0x545454FF, 0xCC9966FF, 0x70706CFF],
    ),
    ("text_dark", [0xF2F2F2FF, 0x1C1815FF, 0x191918FF]),
    ("subtle_dark", [0xA4A4A4FF, 0x6E6660FF, 0x52524EFF]),
    ("good_news", [0x7ECD85FF, 0x10642AFF, 0x10642AFF]),
    ("invalid_value_fill", [0x301E20FF, 0xFFF0F0FF, 0xFFF0F0FF]),
    ("invalid_value_text", [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF]),
    ("unapplied_value_fill", [0x362B18FF, 0xFFF3C9FF, 0xFFF7DCFF]),
    ("unapplied_value_text", [0xF0BE5AFF, 0x7A5000FF, 0x7A5000FF]),
    ("markup_tag_text", [0xC592E6FF, 0x792E9CFF, 0x792E9CFF]),
    ("markup_warning_text", [0xF0B446FF, 0xB06800FF, 0xB06800FF]),
    ("function_plot_bg", [0x40403EFF, 0xE6E0DBFF, 0xCDCDCDFF]),
    ("function_grid_line", [0x84847EFF, 0x766C64FF, 0x5C5C58FF]),
    (
        "foundation_flag_hover",
        [0x3A3A38FF, 0xECE8E4FF, 0xE8E8E4FF],
    ),
    ("material_panel", [0x3A3C38FF, 0xF6F3F0FF, 0xEEEEEBFF]),
    ("material_panel_edge", [0x484A46FF, 0xC4B8AEFF, 0xA8A8A2FF]),
    (
        "disclosure_triangle_green",
        [0x22CD54FF, 0x22CD54FF, 0x22CD54FF],
    ),
    (
        "foundation_block_edge",
        [0x5E5E5EFF, 0xD6C8BCFF, 0x9A9A95FF],
    ),
    ("browser_search_bg", [0x00000066, 0x776A5D80, 0x66666666]),
    ("browser_toolbar_bg", [0x444444FF, 0xEEECEAFF, 0xE2E2DEFF]),
    (
        "browser_toolbar_active",
        [0x497088FF, 0x5D899EFF, 0x5D899EFF],
    ),
    ("context_menu_hover", [0x1A1A1A1A, 0x0703011A, 0x0000001A]),
    (
        "disclosure_triangle_blue",
        [0x186FCDFF, 0x186FCDFF, 0x186FCDFF],
    ),
    ("material_ref_row", [0x1E3A28FF, 0xA6CDA6FF, 0xA6CDA6FF]),
    ("material_numeric_row", [0x3E2D27FF, 0xE8BFABFF, 0xE8BFABFF]),
    ("material_data_row", [0x2A2B29FF, 0xE8E2DDFF, 0xD8D8D8FF]),
    ("material_grid_light", [0x363834FF, 0xDAD2CBFF, 0xC6C6C0FF]),
    ("material_input_edge", [0x52564EFF, 0xAFA298FF, 0x707070FF]),
    ("material_text", [0xE7E8E2FF, 0x1C1815FF, 0x141414FF]),
    ("material_muted_text", [0x9B9E96FF, 0x6E6660FF, 0x606060FF]),
    (
        "material_function_row",
        [0x3A2F20FF, 0xEFCD89FF, 0xEFCD89FF],
    ),
    ("text_strong", [0xFFFFFFFF, 0x000000FF, 0x000000FF]),
    ("menu_row_hover", [0x1A1A1A1A, 0x0000001A, 0x0000001A]),
    ("modal_scrim", [0x0000008C, 0x0000008C, 0x0000008C]),
    ("added_text", [0x7EBA6CFF, 0x7EBA6CFF, 0x7EBA6CFF]),
    ("removed_wash", [0x341E1EFF, 0x341E1EFF, 0x341E1EFF]),
    ("added_wash", [0x1C2E20FF, 0x1C2E20FF, 0x1C2E20FF]),
    ("removed_text", [0xD66A6AFF, 0xD66A6AFF, 0xD66A6AFF]),
    ("modified_text", [0xD6A82EFF, 0xD6A82EFF, 0xD6A82EFF]),
    ("unchanged_text", [0x828282FF, 0x828282FF, 0x828282FF]),
    ("dirty_tab_tint", [0xB8860BFF, 0xB8860BFF, 0xB8860BFF]),
    (
        "shader_modified_accent",
        [0xE09E3EFF, 0xE09E3EFF, 0xE09E3EFF],
    ),
    ("value_decreased", [0xE25C5CFF, 0x8B0000FF, 0x8B0000FF]),
    ("match_tick", [0x7ECD85FF, 0x10642AFF, 0x10642AFF]),
    ("open_folder_icon", [0x22CD54FF, 0x22CD54FF, 0x22CD54FF]),
    ("destructive_icon", [0xE25C5CFF, 0x8B0000FF, 0x8B0000FF]),
    ("unused_value_text", [0xE25C5CFF, 0x8B0000FF, 0x8B0000FF]),
    ("group_badge_fill", [0x303A42FF, 0xE8EDF0FF, 0xE8EDF0FF]),
    ("group_badge_edge", [0x4C5962FF, 0xC4CED5FF, 0xC4CED5FF]),
    ("group_badge_text", [0xE2EBF0FF, 0x3A464FFF, 0x3A464FFF]),
    ("welcome_side_panel", [0x00000033, 0x4B423A50, 0x43434350]),
    ("favourite_star", [0xF2C430FF, 0xF2C430FF, 0xF2C430FF]),
    ("clipboard_cut_mark", [0xFFD138FF, 0xFFD138FF, 0xFFD138FF]),
    ("clipboard_copy_mark", [0x47AEEBFF, 0x47AEEBFF, 0x47AEEBFF]),
    ("search_match_bg", [0xFFDC46FF, 0xFFDC46FF, 0xFFDC46FF]),
    (
        "search_active_match_bg",
        [0xFFA826FF, 0xFFA826FF, 0xFFA826FF],
    ),
    ("search_match_text", [0x000000FF, 0x000000FF, 0x000000FF]),
    ("jump_glow", [0x26200026, 0x26200026, 0x26200026]),
    ("help_cue", [0x3DA1CCFF, 0x3DA1CCFF, 0x3DA1CCFF]),
    ("read_only_badge", [0xD2913CFF, 0xD2913CFF, 0xD2913CFF]),
    ("read_only_badge_mark", [0x1E1E1EFF, 0x1E1E1EFF, 0x1E1E1EFF]),
    ("import_button_fill", [0x287A3EFF, 0x287A3EFF, 0x287A3EFF]),
    ("graph_container_fill", [0x00000040, 0x000000FF, 0x000000FF]),
    ("graph_band_fill", [0x1C1C1C1C, 0x1C1C1C1C, 0x1C1C1C1C]),
    ("graph_band_edge", [0x78787878, 0x78787878, 0x78787878]),
    ("terminal_bg", [0x181817FF, 0x181817FF, 0x181817FF]),
    ("terminal_text", [0xE8E8E4FF, 0xE8E8E4FF, 0xE8E8E4FF]),
    (
        "terminal_warning_text",
        [0xEEC45BFF, 0xEEC45BFF, 0xEEC45BFF],
    ),
    ("terminal_error_text", [0xF46969FF, 0xF46969FF, 0xF46969FF]),
    (
        "terminal_success_text",
        [0x7BB889FF, 0x7BB889FF, 0x7BB889FF],
    ),
    // What the slider painted its rail with: a dark fill in the dark look,
    // the input's fill and outline in the light ones (`slider.rs`).
    ("slider_rail_fill", [0x0000005A, 0x776A5D80, 0x4D4D4D4D]),
    ("slider_rail_edge", [0x00000000, 0xCC9966FF, 0x70706CFF]),
    // Written in `selection_art.rs` and `button_icons.rs` before.
    ("control_well", [0x000000FF, 0xFFFFFFFF, 0xFFFFFFFF]),
    (
        "button_destructive_accent",
        [0xEB4747FF, 0xEB4747FF, 0xEB4747FF],
    ),
    ("progress_track", [0x1F1F1EFF, 0xE2D0C2FF, 0xD7D7D2FF]),
    ("progress_fill", [0x456F84FF, 0x5B92ACFF, 0x5B92ACFF]),
    ("help_code_bg", [0x181B1DFF, 0xF7EEE7FF, 0xEEF1F3FF]),
    ("settings_card_fill", [0x0D0D0D0D, 0x0000000D, 0x0000000D]),
    (
        "settings_card_invalid_fill",
        [0x622824FF, 0xFFDED8FF, 0xFFDED8FF],
    ),
    ("import_warning_text", [0xF0BE5AFF, 0x7A5000FF, 0x7A5000FF]),
    ("replace_warning_text", [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF]),
    ("chimp_error_text", [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF]),
    ("save_error_text", [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF]),
    (
        "installed_overwrite_text",
        [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF],
    ),
    ("panel_error_text", [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF]),
    ("bitmap_error_text", [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF]),
    ("disk_banner_fill", [0x483816FF, 0xFCF0CCFF, 0xFCF0CCFF]),
    ("disk_banner_edge", [0x8C6820FF, 0xDEB048FF, 0xDEB048FF]),
    ("disk_banner_icon", [0xF2C430FF, 0xB07800FF, 0xB07800FF]),
    ("disk_missing_fill", [0x4E201EFF, 0xFCE2DFFF, 0xFCE2DFFF]),
    ("disk_missing_edge", [0x9E3E38FF, 0xE08076FF, 0xE08076FF]),
    ("disk_missing_icon", [0xF05C50FF, 0xC83026FF, 0xC83026FF]),
    ("header_notice_text", [0xF0BE5AFF, 0x7A5000FF, 0x7A5000FF]),
    ("filter_warning_text", [0xF0BE5AFF, 0x7A5000FF, 0x7A5000FF]),
    (
        "settings_warning_text",
        [0xF0BE5AFF, 0x7A5000FF, 0x7A5000FF],
    ),
    ("expert_mode_text", [0xF0BE5AFF, 0x7A5000FF, 0x7A5000FF]),
    ("accept_text", [0x7ECD85FF, 0x10642AFF, 0x10642AFF]),
    ("add_icon", [0x7ECD85FF, 0x10642AFF, 0x10642AFF]),
    (
        "chimp_browser_error_text",
        [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF],
    ),
    ("chimp_archive_text", [0xD29646FF, 0xD29646FF, 0xD29646FF]),
    ("chimp_raw_value_text", [0xF0BE5AFF, 0x7A5000FF, 0x7A5000FF]),
    ("review_error_text", [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF]),
    ("first_run_error_text", [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF]),
    (
        "reference_missing_text",
        [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF],
    ),
    ("error_text", [0xFF7C7CFF, 0xA6232DFF, 0xA6232DFF]),
    ("graph_curve(0)", [0x2ABE48FF, 0x2ABE48FF, 0x2ABE48FF]),
    ("graph_curve(1)", [0xDC3737FF, 0xDC3737FF, 0xDC3737FF]),
    ("graph_curve_fill(0)", [0x0413071A, 0x0413071A, 0x0413071A]),
    ("graph_curve_fill(1)", [0x1606061A, 0x1606061A, 0x1606061A]),
    ("focused_tab_border", [0xA0A0A0FF, 0xD0966AFF, 0x4E78A0FF]),
    ("invalid_value_stroke", [0xAB464CFF, 0xC04B54FF, 0xC04B54FF]),
    ("icon_backdrop", [0x00000000, 0xECDCCDFF, 0xE0E0DCFF]),
    (
        "json_syntax_colors.plain",
        [0xD2D6DEFF, 0x2D323AFF, 0x2D323AFF],
    ),
    (
        "json_syntax_colors.key",
        [0xF4BF5CFF, 0x915B00FF, 0x915B00FF],
    ),
    (
        "json_syntax_colors.string",
        [0xBEEB7DFF, 0x37700FFF, 0x37700FFF],
    ),
    (
        "json_syntax_colors.path",
        [0xE89DDEFF, 0x9A3488FF, 0x9A3488FF],
    ),
    (
        "json_syntax_colors.number",
        [0xF28B82FF, 0xB93E30FF, 0xB93E30FF],
    ),
    (
        "json_syntax_colors.literal",
        [0x69BEFFFF, 0x0068B2FF, 0x0068B2FF],
    ),
    (
        "json_syntax_colors.punctuation",
        [0x69D2E1FF, 0x007080FF, 0x007080FF],
    ),
    ("MATERIAL_DATA_ROW", [0xD8D8D8FF, 0xD8D8D8FF, 0xD8D8D8FF]),
    ("MATERIAL_GRID", [0x5C5C5CFF, 0x5C5C5CFF, 0x5C5C5CFF]),
    ("MATERIAL_INPUT_EDGE", [0x707070FF, 0x707070FF, 0x707070FF]),
    ("MATERIAL_DEFAULT_BOX", [0xE0E0E0FF, 0xE0E0E0FF, 0xE0E0E0FF]),
    ("MATERIAL_TEXT", [0x141414FF, 0x141414FF, 0x141414FF]),
    ("MATERIAL_MUTED_TEXT", [0x606060FF, 0x606060FF, 0x606060FF]),
    (
        "MATERIAL_FUNCTION_ROW",
        [0xEFCD89FF, 0xEFCD89FF, 0xEFCD89FF],
    ),
];

/// The source `name` now.
fn current(name: &str) -> Color32 {
    match name {
        "menu_bar" => s::menu_bar(),
        "chrome_edge" => s::chrome_edge(),
        "chrome_bar" => s::chrome_bar(),
        "active_tab" => s::active_tab(),
        "foundation_blue" => s::foundation_blue(),
        "left_panel" => s::left_panel(),
        "editor_bg" => s::editor_bg(),
        "row_type" => s::row_type(),
        "grid_line" => s::grid_line(),
        "foundation_group_bg" => s::foundation_group_bg(),
        "foundation_documentation_bg" => s::foundation_documentation_bg(),
        "foundation_group_edge" => s::foundation_group_edge(),
        "foundation_section_bar" => s::foundation_section_bar(),
        "foundation_block_bar" => s::foundation_block_bar(),
        "foundation_block_text" => s::foundation_block_text(),
        "playhead" => s::playhead(),
        "foundation_jump_cyan" => s::foundation_jump_cyan(),
        "foundation_input" => s::foundation_input(),
        "foundation_input_read_only" => s::foundation_input_read_only(),
        "foundation_input_edge" => s::foundation_input_edge(),
        "text_dark" => s::text_dark(),
        "subtle_dark" => s::subtle_dark(),
        "good_news" => s::good_news(),
        "invalid_value_fill" => s::invalid_value_fill(),
        "invalid_value_text" => s::invalid_value_text(),
        "unapplied_value_fill" => s::unapplied_value_fill(),
        "unapplied_value_text" => s::unapplied_value_text(),
        "markup_tag_text" => s::markup_tag_text(),
        "markup_warning_text" => s::markup_warning_text(),
        "function_plot_bg" => s::function_plot_bg(),
        "function_grid_line" => s::function_grid_line(),
        "foundation_flag_hover" => s::foundation_flag_hover(),
        "material_panel" => s::material_panel(),
        "material_panel_edge" => s::material_panel_edge(),
        "disclosure_triangle_green" => s::disclosure_triangle_green(),
        "foundation_block_edge" => s::foundation_block_edge(),
        "browser_search_bg" => s::browser_search_bg(),
        "browser_toolbar_bg" => s::browser_toolbar_bg(),
        "browser_toolbar_active" => s::browser_toolbar_active(),
        "context_menu_hover" => s::context_menu_hover(),
        "disclosure_triangle_blue" => s::disclosure_triangle_blue(),
        "material_ref_row" => s::material_ref_row(),
        "material_numeric_row" => s::material_numeric_row(),
        "material_data_row" => s::material_data_row(),
        "material_grid_light" => s::material_grid_light(),
        "material_input_edge" => s::material_input_edge(),
        "material_text" => s::material_text(),
        "material_muted_text" => s::material_muted_text(),
        "material_function_row" => s::material_function_row(),
        "text_strong" => s::text_strong(),
        "menu_row_hover" => s::menu_row_hover(),
        "modal_scrim" => s::modal_scrim(),
        "added_text" => s::added_text(),
        "removed_wash" => s::removed_wash(),
        "added_wash" => s::added_wash(),
        "removed_text" => s::removed_text(),
        "modified_text" => s::modified_text(),
        "unchanged_text" => s::unchanged_text(),
        "dirty_tab_tint" => s::dirty_tab_tint(),
        "shader_modified_accent" => s::shader_modified_accent(),
        "value_decreased" => s::value_decreased(),
        "match_tick" => s::match_tick(),
        "open_folder_icon" => s::open_folder_icon(),
        "destructive_icon" => s::destructive_icon(),
        "unused_value_text" => s::unused_value_text(),
        "group_badge_fill" => s::group_badge_fill(),
        "group_badge_edge" => s::group_badge_edge(),
        "group_badge_text" => s::group_badge_text(),
        "welcome_side_panel" => s::welcome_side_panel(),
        "favourite_star" => s::favourite_star(),
        "clipboard_cut_mark" => s::clipboard_cut_mark(),
        "clipboard_copy_mark" => s::clipboard_copy_mark(),
        "search_match_bg" => s::search_match_bg(),
        "search_active_match_bg" => s::search_active_match_bg(),
        "search_match_text" => s::search_match_text(),
        "jump_glow" => s::jump_glow(),
        "help_cue" => s::help_cue(),
        "read_only_badge" => s::read_only_badge(),
        "read_only_badge_mark" => s::read_only_badge_mark(),
        "import_button_fill" => s::import_button_fill(),
        "graph_container_fill" => s::graph_container_fill(),
        "graph_band_fill" => s::graph_band_fill(),
        "graph_band_edge" => s::graph_band_edge(),
        "terminal_bg" => s::terminal_bg(),
        "terminal_text" => s::terminal_text(),
        "terminal_warning_text" => s::terminal_warning_text(),
        "terminal_error_text" => s::terminal_error_text(),
        "terminal_success_text" => s::terminal_success_text(),
        "slider_rail_fill" => s::slider_rail_fill(),
        "slider_rail_edge" => s::slider_rail_edge(),
        "control_well" => s::control_well(),
        "button_destructive_accent" => s::button_destructive_accent(),
        "progress_track" => s::progress_track(),
        "progress_fill" => s::progress_fill(),
        "help_code_bg" => s::help_code_bg(),
        "settings_card_fill" => s::settings_card_fill(),
        "settings_card_invalid_fill" => s::settings_card_invalid_fill(),
        "import_warning_text" => s::import_warning_text(),
        "replace_warning_text" => s::replace_warning_text(),
        "chimp_error_text" => s::chimp_error_text(),
        "save_error_text" => s::save_error_text(),
        "installed_overwrite_text" => s::installed_overwrite_text(),
        "panel_error_text" => s::panel_error_text(),
        "bitmap_error_text" => s::bitmap_error_text(),
        "disk_banner_fill" => s::disk_banner_fill(),
        "disk_banner_edge" => s::disk_banner_edge(),
        "disk_banner_icon" => s::disk_banner_icon(),
        "disk_missing_fill" => s::disk_missing_fill(),
        "disk_missing_edge" => s::disk_missing_edge(),
        "disk_missing_icon" => s::disk_missing_icon(),
        "header_notice_text" => s::header_notice_text(),
        "filter_warning_text" => s::filter_warning_text(),
        "settings_warning_text" => s::settings_warning_text(),
        "expert_mode_text" => s::expert_mode_text(),
        "accept_text" => s::accept_text(),
        "add_icon" => s::add_icon(),
        "chimp_browser_error_text" => s::chimp_browser_error_text(),
        "chimp_archive_text" => s::chimp_archive_text(),
        "chimp_raw_value_text" => s::chimp_raw_value_text(),
        "review_error_text" => s::review_error_text(),
        "first_run_error_text" => s::first_run_error_text(),
        "reference_missing_text" => s::reference_missing_text(),
        "error_text" => s::error_text(),
        "graph_curve(0)" => s::graph_curve(0),
        "graph_curve(1)" => s::graph_curve(1),
        "graph_curve_fill(0)" => s::graph_curve_fill(0),
        "graph_curve_fill(1)" => s::graph_curve_fill(1),
        "focused_tab_border" => s::focused_tab_border(),
        "invalid_value_stroke" => s::invalid_value_stroke().color,
        "icon_backdrop" => s::icon_backdrop().unwrap_or(Color32::TRANSPARENT),
        "json_syntax_colors.plain" => s::json_syntax_colors().plain,
        "json_syntax_colors.key" => s::json_syntax_colors().key,
        "json_syntax_colors.string" => s::json_syntax_colors().string,
        "json_syntax_colors.path" => s::json_syntax_colors().path,
        "json_syntax_colors.number" => s::json_syntax_colors().number,
        "json_syntax_colors.literal" => s::json_syntax_colors().literal,
        "json_syntax_colors.punctuation" => s::json_syntax_colors().punctuation,
        "MATERIAL_DATA_ROW" => s::material_editor_data_row(),
        "MATERIAL_GRID" => s::material_editor_grid(),
        "MATERIAL_INPUT_EDGE" => s::material_editor_input_edge(),
        "MATERIAL_DEFAULT_BOX" => s::material_editor_box(),
        "MATERIAL_TEXT" => s::material_editor_text(),
        "MATERIAL_MUTED_TEXT" => s::material_editor_muted_text(),
        "MATERIAL_FUNCTION_ROW" => s::material_editor_function_row(),
        other => panic!("no colour source named {other}"),
    }
}

const LOOKS: [BuiltIn; 3] = [BuiltIn::Dark, BuiltIn::Light, BuiltIn::ClassicLight];

/// Every source that differs from `golden` under some built-in, by name.
fn mismatches(golden: &[(&str, [u32; 3])]) -> Vec<String> {
    let mut found = Vec::new();
    for (column, built_in) in LOOKS.into_iter().enumerate() {
        with_colors(&built_in.colors(), || {
            for (name, values) in golden {
                let now = u32::from_be_bytes(current(name).to_array());
                if now != values[column] {
                    found.push(format!(
                        "{name} under {}: {now:08X}, was {:08X}",
                        built_in.id(),
                        values[column]
                    ));
                }
            }
        });
    }
    found
}

/// Every accessor paints what it painted before the registry, under all
/// three built-ins.
#[test]
fn the_built_in_themes_paint_what_baboon_painted_before() {
    assert_eq!(mismatches(GOLDEN), Vec::<String>::new());
}

/// The comparison names a colour that moved: one entry of a copy, altered.
#[test]
fn the_golden_comparison_names_a_colour_that_moved() {
    let mut altered = GOLDEN.to_vec();
    altered[0].1[1] ^= 0x0100_0000;
    let found = mismatches(&altered);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].starts_with(&format!("{} under baboon-light", altered[0].0)),
        "{found:?}"
    );
}

/// egui's visuals under each built-in are the ones captured, field for field.
#[test]
fn the_built_in_themes_build_the_visuals_baboon_built_before() {
    let captured = [
        include_str!("golden_visuals_dark.txt"),
        include_str!("golden_visuals_light.txt"),
        include_str!("golden_visuals_classic.txt"),
    ];
    for (built_in, captured) in LOOKS.into_iter().zip(captured) {
        let now = with_colors(&built_in.colors(), || {
            format!("{:#?}", s::foundation_visuals())
        });
        let changed: Vec<(&str, &str)> = captured
            .lines()
            .zip(now.lines())
            .filter(|(was, is)| was != is)
            .collect();
        assert!(
            changed.is_empty() && captured.lines().count() == now.lines().count(),
            "{}: {changed:?}",
            built_in.id()
        );
    }
}
