//! Baboon colors, typography, spacing, and reusable egui style helpers.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn foundation_visuals() -> egui::Visuals {
    let mut visuals = if is_dark_mode() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(text_dark());
    visuals.panel_fill = editor_bg();
    visuals.window_fill = editor_bg();
    visuals.faint_bg_color = row_type();
    visuals.extreme_bg_color = if is_dark_mode() {
        Color32::from_rgb(23, 23, 23)
    } else {
        foundation_input()
    };
    visuals.selection.bg_fill = if is_dark_mode() {
        Color32::from_rgb(64, 108, 134)
    } else {
        Color32::from_rgb(42, 91, 122)
    };
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(120, 170, 198));
    visuals.widgets.noninteractive.bg_fill = row_type();
    visuals.widgets.inactive.bg_fill = if is_dark_mode() {
        Color32::from_rgb(68, 68, 68)
    } else {
        Color32::from_rgb(218, 218, 214)
    };
    visuals.widgets.hovered.bg_fill = if is_dark_mode() {
        Color32::from_rgb(82, 82, 82)
    } else {
        Color32::from_rgb(201, 215, 221)
    };
    visuals.widgets.active.bg_fill = if is_dark_mode() {
        Color32::from_rgb(92, 92, 92)
    } else {
        Color32::from_rgb(188, 207, 216)
    };
    visuals.menu_rounding = egui::Rounding::same(5.0);
    visuals.window_stroke = Stroke::new(1.0, foundation_group_edge());
    visuals
}

/// Named family used for bold headers (egui has no font-weight API — bold is a
/// separate font). Falls back to the regular family when no bold font is found.
pub(super) const FOUNDATION_BOLD: &str = "foundation_bold";

pub(super) fn foundation_fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for path in [
        r"C:\Windows\Fonts\micross.ttf",
        r"C:\Windows\Fonts\tahoma.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("foundation_ui".to_owned(), FontData::from_owned(bytes));
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "foundation_ui".to_owned());
            break;
        }
    }

    // Bold face for headers (Foundation uses FontWeight=Bold). Try common
    // system bold fonts across platforms; gracefully degrade to the regular
    // family if none are present so the named family is always valid.
    let bold_loaded = [
        r"C:\Windows\Fonts\segoeuib.ttf",
        r"C:\Windows\Fonts\tahomabd.ttf",
        r"C:\Windows\Fonts\arialbd.ttf",
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/System/Library/Fonts/Supplemental/Tahoma Bold.ttf",
        "/Library/Fonts/Arial Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    ]
    .iter()
    .any(|path| match std::fs::read(path) {
        Ok(bytes) => {
            fonts
                .font_data
                .insert(FOUNDATION_BOLD.to_owned(), FontData::from_owned(bytes));
            true
        }
        Err(_) => false,
    });

    let regular = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut bold_family = Vec::new();
    if bold_loaded {
        bold_family.push(FOUNDATION_BOLD.to_owned());
    }
    bold_family.extend(regular); // glyph fallback (and the whole family if no bold)
    fonts
        .families
        .insert(FontFamily::Name(FOUNDATION_BOLD.into()), bold_family);

    fonts
}

/// A bold [`FontId`] at `size`, for headers. Renders bold where a system bold
/// font was found, otherwise regular weight.
pub(super) fn bold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FOUNDATION_BOLD.into()))
}

pub(super) fn foundation_style() -> egui::Style {
    let mut style = egui::Style::default();
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(17.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(10.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::proportional(12.0));
    style.spacing.item_spacing = Vec2::new(4.0, 3.0);
    style.spacing.button_padding = Vec2::new(5.0, 2.0);
    style
}

static DARK_MODE_ENABLED: AtomicBool = AtomicBool::new(false);

pub(super) fn set_dark_mode(enabled: bool) {
    DARK_MODE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub(super) fn is_dark_mode() -> bool {
    DARK_MODE_ENABLED.load(Ordering::Relaxed)
}

pub(super) fn menu_bar() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(50, 50, 50)
    } else {
        Color32::from_rgb(161, 161, 157)
    }
}

pub(super) fn foundation_blue() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(134, 184, 213)
    } else {
        Color32::from_rgb(15, 43, 64)
    }
}

pub(super) fn left_panel() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(31, 31, 31)
    } else {
        Color32::from_rgb(238, 238, 234)
    }
}

pub(super) fn editor_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(40, 40, 40)
    } else {
        Color32::from_rgb(224, 224, 220)
    }
}

pub(super) fn row_type() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(55, 55, 55)
    } else {
        Color32::from_rgb(219, 219, 216)
    }
}

pub(super) fn grid_line() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(82, 82, 82)
    } else {
        Color32::from_rgb(180, 180, 174)
    }
}

pub(super) fn foundation_group_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(43, 43, 43)
    } else {
        Color32::from_rgb(236, 236, 234)
    }
}

pub(super) fn foundation_group_edge() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(82, 82, 82)
    } else {
        Color32::from_rgb(152, 152, 148)
    }
}

pub(super) fn foundation_section_bar() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(68, 68, 68)
    } else {
        Color32::from_rgb(214, 214, 210)
    }
}

pub(super) fn foundation_block_bar() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(72, 72, 72)
    } else {
        Color32::from_rgb(98, 98, 96)
    }
}

pub(super) fn foundation_block_text() -> Color32 {
    Color32::from_rgb(245, 245, 245)
}

pub(super) fn foundation_input() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(31, 31, 31)
    } else {
        Color32::from_rgb(248, 248, 247)
    }
}

pub(super) fn foundation_input_edge() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(84, 84, 84)
    } else {
        Color32::from_rgb(112, 112, 108)
    }
}

pub(super) fn text_dark() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(242, 242, 242)
    } else {
        Color32::from_rgb(25, 25, 24)
    }
}

pub(super) fn subtle_dark() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(164, 164, 164)
    } else {
        Color32::from_rgb(82, 82, 78)
    }
}

pub(super) fn placeholder_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(128, 128, 128)
    } else {
        Color32::from_rgb(145, 145, 138)
    }
}

pub(super) fn function_plot_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(64, 64, 62)
    } else {
        Color32::from_rgb(205, 205, 205)
    }
}

pub(super) fn function_grid_line() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(132, 132, 126)
    } else {
        Color32::from_rgb(92, 92, 88)
    }
}

pub(super) fn foundation_flag_hover() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(58, 58, 56)
    } else {
        Color32::from_rgb(232, 232, 228)
    }
}

pub(super) fn foundation_checkbox_bg(enabled: bool) -> Color32 {
    if !enabled {
        return if is_dark_mode() {
            Color32::from_rgb(44, 44, 42)
        } else {
            Color32::from_rgb(226, 226, 222)
        };
    }
    foundation_input()
}

pub(super) const MATERIAL_PANEL: Color32 = Color32::from_rgb(238, 238, 235);
pub(super) const MATERIAL_PANEL_EDGE: Color32 = Color32::from_rgb(168, 168, 162);
pub(super) const MATERIAL_REF_ROW: Color32 = Color32::from_rgb(166, 205, 166);
pub(super) const MATERIAL_NUMERIC_ROW: Color32 = Color32::from_rgb(232, 191, 171);
pub(super) const MATERIAL_DATA_ROW: Color32 = Color32::from_rgb(216, 216, 216);
pub(super) const MATERIAL_GRID: Color32 = Color32::from_rgb(92, 92, 92);
pub(super) const MATERIAL_GRID_LIGHT: Color32 = Color32::from_rgb(198, 198, 192);
pub(super) const MATERIAL_INPUT_EDGE: Color32 = Color32::from_rgb(112, 112, 112);
pub(super) const MATERIAL_DEFAULT_BOX: Color32 = Color32::from_rgb(224, 224, 224);
pub(super) const MATERIAL_TEXT: Color32 = Color32::from_rgb(20, 20, 20);
pub(super) const MATERIAL_MUTED_TEXT: Color32 = Color32::from_rgb(96, 96, 96);
pub(super) const MATERIAL_FUNCTION_ROW: Color32 = Color32::from_rgb(239, 205, 137);
pub(super) const MATERIAL_SECTION_HEADER: Color32 = Color32::from_rgb(255, 255, 224);

pub(super) fn disclosure_triangle_green() -> Color32 {
    Color32::from_rgb(34, 205, 84)
}

pub(super) fn foundation_block_bar_hover() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(82, 82, 82)
    } else {
        Color32::from_rgb(112, 112, 108)
    }
}

pub(super) fn foundation_block_edge() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(94, 94, 94)
    } else {
        Color32::from_rgb(154, 154, 149)
    }
}

pub(super) fn foundation_disclosure_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(84, 84, 84)
    } else {
        Color32::from_rgb(232, 232, 228)
    }
}

pub(super) fn browser_search_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(23, 23, 23)
    } else {
        Color32::from_rgb(246, 246, 244)
    }
}

pub(super) fn browser_search_hover() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(35, 35, 35)
    } else {
        Color32::from_rgb(255, 255, 252)
    }
}

pub(super) fn browser_toolbar_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(68, 68, 68)
    } else {
        Color32::from_rgb(226, 226, 222)
    }
}

pub(super) fn browser_toolbar_active() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(73, 112, 136)
    } else {
        Color32::from_rgb(93, 137, 158)
    }
}

pub(super) fn context_menu_hover() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(70, 70, 70)
    } else {
        Color32::from_rgb(220, 232, 238)
    }
}

pub(super) fn disclosure_triangle_blue() -> Color32 {
    Color32::from_rgb(24, 111, 205)
}

pub(super) fn material_ref_row() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(30, 58, 40)
    } else {
        MATERIAL_REF_ROW
    }
}

pub(super) fn material_numeric_row() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(62, 45, 39)
    } else {
        MATERIAL_NUMERIC_ROW
    }
}

pub(super) fn material_data_row() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(42, 43, 41)
    } else {
        MATERIAL_DATA_ROW
    }
}

pub(super) fn material_grid_light() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(54, 56, 52)
    } else {
        MATERIAL_GRID_LIGHT
    }
}

pub(super) fn material_input_edge() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(82, 86, 78)
    } else {
        MATERIAL_INPUT_EDGE
    }
}

pub(super) fn material_default_box() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(38, 39, 37)
    } else {
        MATERIAL_DEFAULT_BOX
    }
}

pub(super) fn material_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(231, 232, 226)
    } else {
        MATERIAL_TEXT
    }
}

pub(super) fn material_text_for_bg(bg: Color32) -> Color32 {
    let luminance = 0.2126 * bg.r() as f32 + 0.7152 * bg.g() as f32 + 0.0722 * bg.b() as f32;
    if luminance < 128.0 {
        Color32::from_gray(232)
    } else {
        Color32::from_gray(20)
    }
}

pub(super) fn material_muted_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(155, 158, 150)
    } else {
        MATERIAL_MUTED_TEXT
    }
}

pub(super) fn material_function_row() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(58, 47, 32)
    } else {
        MATERIAL_FUNCTION_ROW
    }
}

pub(super) fn material_section_header() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(42, 58, 48)
    } else {
        MATERIAL_SECTION_HEADER
    }
}

pub(super) fn material_input() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(27, 28, 27)
    } else {
        Color32::WHITE
    }
}

pub(super) fn material_disabled_input() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(36, 37, 35)
    } else {
        Color32::from_gray(210)
    }
}

pub(super) fn material_default_input() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(39, 40, 38)
    } else {
        Color32::from_gray(232)
    }
}

pub(super) fn material_checkbox_disabled() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(34, 35, 33)
    } else {
        Color32::from_gray(220)
    }
}

pub(super) fn material_hover() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(46, 58, 62)
    } else {
        Color32::from_rgb(238, 244, 255)
    }
}

pub(super) fn material_pending_input() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(43, 37, 31)
    } else {
        Color32::from_rgb(255, 252, 235)
    }
}

pub(super) fn material_delete_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(226, 92, 92)
    } else {
        Color32::DARK_RED
    }
}

pub(super) fn material_color_swatch_edge(color: Color32) -> Color32 {
    let luminance =
        0.2126 * color.r() as f32 + 0.7152 * color.g() as f32 + 0.0722 * color.b() as f32;
    if luminance < 80.0 {
        Color32::from_rgb(238, 238, 232)
    } else if luminance > 188.0 {
        Color32::from_rgb(24, 24, 22)
    } else {
        material_input_edge()
    }
}

pub(super) const MATERIAL_PARAMETER_SECTIONS: &[&str] = &[
    "ALBEDO",
    "BUMP_MAPPING",
    "MATERIAL_MODEL",
    "ENVIRONMENT_MAPPING",
    "SELF_ILLUMINATION",
    "ATMOSPHERE PROPERTIES",
    "MISC",
];
pub(super) const MAX_OPEN_TABS: usize = 32;
pub(super) const MAX_PARSED_TAGS: usize = 24;

#[cfg(test)]
#[path = "tests/style.rs"]
mod tests;
pub(super) const FOUNDATION_LABEL_WIDTH: f32 = 280.0;
