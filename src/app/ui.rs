//! Top-level windows, menus, dialogs, and frame composition for [`Baboon`].
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::controller::open_terminal_log;
use super::*;

mod blam;
mod browser_panel;
mod dialogs;
mod find;
mod first_run;
pub(super) mod help;
mod kit_tiles;
mod recents;
mod search_windows;
mod settings;
mod shell;
mod tag_pane;
mod tag_tiles;
mod tool_commands;
mod welcome;

const PANE_HEADER_ICON_SIZE: f32 = 32.0;
const PANE_HEADER_SECTION_GAP: f32 = 20.0;
const PANE_HEADER_ICON_TEXT_GAP: f32 = 10.0;
const PANE_HEADER_ACTION_GAP: f32 = 4.0;
const PANE_HEADER_WIDE_BREAKPOINT: f32 = 600.0;
const PANE_HEADER_MIN_LEFT_WIDTH: f32 = 200.0;
const PANE_HEADER_COMMON_ACTIONS_WIDTH: f32 = 205.0;
const BROWSER_SEARCH_HEIGHT: f32 = 24.0;
const BROWSER_SEARCH_RADIUS: f32 = BROWSER_SEARCH_HEIGHT * 0.5;
const BROWSER_SEARCH_ICON_SIZE: f32 = 16.0;
const BROWSER_SEARCH_LEFT_PADDING: f32 = 4.0;
const BROWSER_SEARCH_ICON_TEXT_GAP: f32 = 8.0;
const BROWSER_SEARCH_RIGHT_PADDING: f32 = 8.0;

/// Width of the title column when pane actions can remain beside it. Returning
/// `None` is the shared signal for tag and folder headers to put actions below
/// the title instead, preventing either header from overlapping at narrow
/// docked or window sizes.
fn pane_header_inline_left_width(available: f32, action_width: f32) -> Option<f32> {
    (available >= PANE_HEADER_WIDE_BREAKPOINT).then(|| {
        (available - action_width - PANE_HEADER_SECTION_GAP).max(PANE_HEADER_MIN_LEFT_WIDTH)
    })
}

/// Border states shared by the custom search and keyword pills. Their fill is
/// intentionally stable; hover and keyboard focus use the same strokes as an
/// ordinary application text edit so the custom pill geometry does not create
/// a second input style.
fn pane_header_input_stroke(ui: &Ui, hovered: bool, focused: bool) -> Stroke {
    if focused {
        ui.visuals().selection.stroke
    } else if hovered {
        ui.visuals().widgets.hovered.bg_stroke
    } else {
        Stroke::new(1.0, foundation_input_edge())
    }
}

/// The shared browser search field. Its icon lives inside the 24-point pill so
/// the compact sidebar and a wide folder pane use exactly the same geometry.
fn browser_search_field(ui: &mut Ui, value: &mut String, hint: &str) -> egui::Response {
    let width = ui.available_width().max(BROWSER_SEARCH_HEIGHT);
    let (rect, background_response) =
        ui.allocate_exact_size(Vec2::new(width, BROWSER_SEARCH_HEIGHT), Sense::hover());
    ui.painter()
        .rect_filled(rect, BROWSER_SEARCH_RADIUS, browser_search_bg());

    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.left() + BROWSER_SEARCH_LEFT_PADDING,
            rect.center().y - BROWSER_SEARCH_ICON_SIZE * 0.5,
        ),
        Vec2::splat(BROWSER_SEARCH_ICON_SIZE),
    );
    paint_button_icon_at(ui, ButtonIcon::SearchBar, icon_rect, text_dark());
    let icon_response = ui.interact(
        icon_rect,
        background_response.id.with("search_icon"),
        Sense::click(),
    );

    let edit_rect = egui::Rect::from_min_max(
        egui::pos2(icon_rect.right() + BROWSER_SEARCH_ICON_TEXT_GAP, rect.top()),
        egui::pos2(rect.right() - BROWSER_SEARCH_RIGHT_PADDING, rect.bottom()),
    );
    let edit_response = ui.put(
        edit_rect,
        egui::TextEdit::singleline(value)
            .hint_text(placeholder_text(hint))
            .text_color(text_dark())
            .frame(false)
            .margin(egui::Margin::same(0.0))
            .vertical_align(egui::Align::Center)
            .min_size(edit_rect.size()),
    );
    if icon_response.clicked() {
        edit_response.request_focus();
    }
    let response = background_response
        .union(icon_response)
        .union(edit_response.clone());
    ui.painter().rect_stroke(
        rect,
        BROWSER_SEARCH_RADIUS,
        pane_header_input_stroke(ui, response.hovered(), edit_response.has_focus()),
    );
    response
}

fn browser_favorites_divider(ui: &mut Ui, favorites_visible: bool) {
    if favorites_visible {
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
    }
}

fn pane_header_path_parts(display_path: &str) -> (Vec<(String, PathBuf)>, String) {
    let normalized = display_path.replace('\\', "/");
    let mut components: Vec<&str> = normalized
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    let title = components.pop().unwrap_or_default().to_owned();
    let mut path = PathBuf::new();
    let breadcrumbs = components
        .into_iter()
        .map(|component| {
            path.push(component);
            (component.to_owned(), path.clone())
        })
        .collect();
    (breadcrumbs, title)
}

/// Draw clickable path segments above a pane title. A placeholder-free custom
/// row keeps the hover fill behind the text while preserving the compact tag
/// header typography.
fn pane_header_breadcrumbs(
    ui: &mut Ui,
    breadcrumbs: &[(String, PathBuf)],
) -> Option<(PathBuf, String)> {
    if breadcrumbs.is_empty() {
        return None;
    }

    const ITEM_GAP: f32 = 2.0;
    const SEGMENT_HORIZONTAL_PADDING: f32 = 8.0;
    let mut clicked = None;
    let breadcrumb_font = FontId::proportional(11.0);
    let chevron =
        ui.painter()
            .layout_no_wrap("›".to_owned(), FontId::proportional(12.0), subtle_dark());
    let segments: Vec<_> = breadcrumbs
        .iter()
        .map(|(label, _)| {
            ui.painter()
                .layout_no_wrap(label.clone(), breadcrumb_font.clone(), subtle_dark())
        })
        .collect();
    let row_height = segments
        .iter()
        .map(|galley| galley.size().y)
        .fold(chevron.size().y, f32::max);
    let row_width = segments
        .iter()
        .map(|galley| galley.size().x + SEGMENT_HORIZONTAL_PADDING + chevron.size().x)
        .sum::<f32>()
        + ITEM_GAP * (breadcrumbs.len() * 2 - 1) as f32
        - SEGMENT_HORIZONTAL_PADDING * 0.5;
    let (row_rect, row_response) =
        ui.allocate_exact_size(Vec2::new(row_width, row_height), Sense::hover());
    // Let the first segment's hover target extend into the icon/text gap. Its
    // glyph then begins at the row origin, exactly where the title begins,
    // while retaining four points of clickable padding on either side.
    let mut x = row_rect.left() - SEGMENT_HORIZONTAL_PADDING * 0.5;

    for (index, ((label, path), galley)) in breadcrumbs.iter().zip(segments).enumerate() {
        let segment_size = Vec2::new(galley.size().x + SEGMENT_HORIZONTAL_PADDING, row_height);
        let rect = egui::Rect::from_min_size(egui::pos2(x, row_rect.top()), segment_size);
        let response = ui
            .interact(rect, row_response.id.with(index), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() {
            ui.painter().rect_filled(
                rect,
                egui::Rounding::same(4.0),
                if is_dark_mode() {
                    Color32::from_white_alpha(26)
                } else {
                    Color32::from_black_alpha(26)
                },
            );
        }
        let text_position = egui::Align2::CENTER_CENTER
            .align_size_within_rect(galley.size(), rect)
            .min;
        ui.painter().galley_with_override_text_color(
            text_position,
            galley,
            if response.hovered() {
                text_dark()
            } else {
                subtle_dark()
            },
        );
        if response.clicked() {
            clicked = Some((path.clone(), label.clone()));
        }

        x = rect.right() + ITEM_GAP;
        let chevron_rect = egui::Rect::from_min_size(
            egui::pos2(x, row_rect.center().y - chevron.size().y * 0.5),
            chevron.size(),
        );
        ui.painter()
            .galley(chevron_rect.min, chevron.clone(), subtle_dark());
        x = chevron_rect.right() + ITEM_GAP;
    }
    clicked
}

fn navigate_folder_browser(pane: &mut FolderBrowserState, path: PathBuf, label: String) {
    if pane.rel_path == path {
        return;
    }
    pane.rel_path = path;
    pane.label = label;
    pane.filter.clear();
    pane.cached_generation = u64::MAX;
    pane.cached_source_len = usize::MAX;
    pane.tree = TagTree::default();
    pane.group_tree = TagTree::default();
    pane.filter_cache = FilterCache::default();
}

/// Mouse wheel over a tile tab bar scrolls it sideways.
///
/// `egui_tiles` keeps a per-bar scroll offset and shows arrow buttons when the
/// tabs overflow, but the bar itself ignores the wheel. Vertical wheel motion
/// maps onto the horizontal offset (up = left, down = right, matching how
/// browsers treat their tab strips), and sideways wheel/touchpad motion passes
/// through directly. Called from `top_bar_right_ui`, which runs before the bar
/// clamps the offset to the content, so no clamping is needed here.
fn wheel_scroll_tab_bar(ui: &Ui, scroll_offset: &mut f32) {
    if !ui.rect_contains_pointer(ui.max_rect()) {
        return;
    }
    let delta = ui.input(|input| input.smooth_scroll_delta);
    *scroll_offset -= delta.x + delta.y;
}

/// A toolbar launcher button: shows the decoded `.ico` icon when available,
/// otherwise falls back to a single-letter label. Returns the response so the
/// caller can attach a hover tooltip and read `.clicked()`.
fn launcher_button(
    ui: &mut Ui,
    icon: Option<&egui::TextureHandle>,
    fallback: &str,
    enabled: bool,
) -> egui::Response {
    match icon {
        Some(texture) => ui.add_enabled(
            enabled,
            egui::ImageButton::new(
                egui::Image::new(egui::load::SizedTexture::new(
                    texture.id(),
                    Vec2::splat(20.0),
                ))
                .tint(Color32::WHITE),
            ),
        ),
        None => ui.add_enabled(
            enabled,
            egui::Button::new(RichText::new(fallback).color(Color32::WHITE))
                .min_size(Vec2::splat(22.0)),
        ),
    }
}

fn editing_kit_menu_shortcuts() -> impl Iterator<Item = EditingKitShortcut> {
    EDITING_KIT_SHORTCUTS.into_iter().rev()
}

fn visible_builtin_editing_kit_shortcuts(
    validation: &EditingKitValidationCache,
) -> Vec<EditingKitShortcut> {
    editing_kit_menu_shortcuts()
        .filter(|shortcut| validation.builtin(*shortcut).layout().is_some())
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EditingKitMenuEntry {
    Custom(CustomEditingKitProfile),
    BuiltIn(EditingKitShortcut),
}

fn visible_editing_kit_menu_entries(
    profiles: &[CustomEditingKitProfile],
    validation: &EditingKitValidationCache,
) -> Vec<EditingKitMenuEntry> {
    profiles
        .iter()
        .cloned()
        .map(EditingKitMenuEntry::Custom)
        .chain(
            visible_builtin_editing_kit_shortcuts(validation)
                .into_iter()
                .map(EditingKitMenuEntry::BuiltIn),
        )
        .collect()
}

const EDITING_KIT_MENU_MIN_WIDTH: f32 = 240.0;
const EDITING_KIT_MENU_ICON_SIZE: f32 = 24.0;
const EDITING_KIT_MENU_HORIZONTAL_PADDING: f32 = 8.0;
const EDITING_KIT_MENU_ICON_GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug)]
struct EditingKitMenuRowLayout {
    label_rect: egui::Rect,
    icon_rect: egui::Rect,
}

fn editing_kit_menu_row_layout(row_rect: egui::Rect) -> EditingKitMenuRowLayout {
    let content = row_rect.shrink2(Vec2::new(EDITING_KIT_MENU_HORIZONTAL_PADDING, 2.0));
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(
            content.left() + EDITING_KIT_MENU_ICON_SIZE * 0.5,
            content.center().y,
        ),
        Vec2::splat(EDITING_KIT_MENU_ICON_SIZE),
    );
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(icon_rect.right() + EDITING_KIT_MENU_ICON_GAP, content.min.y),
        content.max,
    );
    EditingKitMenuRowLayout {
        label_rect,
        icon_rect,
    }
}

fn editing_kit_title_text(
    ui: &Ui,
    name: &str,
    read_only: bool,
    size: f32,
    bold: bool,
) -> egui::WidgetText {
    editing_kit_title_text_with_style(ui.style(), name, read_only, size, bold)
}

fn editing_kit_title_text_with_style(
    style: &egui::Style,
    name: &str,
    read_only: bool,
    size: f32,
    bold: bool,
) -> egui::WidgetText {
    let mut job = egui::text::LayoutJob::default();
    let mut title = RichText::new(name).size(size).color(text_dark());
    if bold {
        title = title.strong();
    }
    title.append_to(
        &mut job,
        style,
        egui::FontSelection::Default,
        egui::Align::Center,
    );
    if read_only {
        RichText::new(" (read-only)")
            .size(size)
            .color(text_dark().gamma_multiply(0.5))
            .append_to(
                &mut job,
                style,
                egui::FontSelection::Default,
                egui::Align::Center,
            );
    }
    job.into()
}

fn editing_kit_menu_row(
    ui: &mut Ui,
    label: &str,
    fallback: &str,
    texture: Option<&egui::TextureHandle>,
    default_project_icon: bool,
    enabled: bool,
) -> egui::Response {
    editing_kit_menu_row_with_read_only(
        ui,
        label,
        fallback,
        texture,
        default_project_icon,
        enabled,
        false,
    )
}

fn editing_kit_menu_row_with_read_only(
    ui: &mut Ui,
    label: &str,
    fallback: &str,
    texture: Option<&egui::TextureHandle>,
    default_project_icon: bool,
    enabled: bool,
    read_only: bool,
) -> egui::Response {
    let row_height = ui
        .spacing()
        .interact_size
        .y
        .max(EDITING_KIT_MENU_ICON_SIZE + 4.0);
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("").min_size(Vec2::new(EDITING_KIT_MENU_MIN_WIDTH, row_height)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let layout = editing_kit_menu_row_layout(response.rect);
    let text_color = text_dark();
    let title = editing_kit_title_text(
        ui,
        label,
        read_only,
        TextStyle::Button.resolve(ui.style()).size,
        false,
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
    ui.painter().with_clip_rect(layout.label_rect).galley(
        layout.label_rect.left_center() - Vec2::new(0.0, title.size().y * 0.5),
        title,
        text_color,
    );
    if let Some(texture) = texture {
        ui.painter().image(
            texture.id(),
            layout.icon_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else if default_project_icon {
        paint_button_icon_at(ui, ButtonIcon::FolderOpen, layout.icon_rect, text_dark());
    } else {
        ui.painter().with_clip_rect(layout.icon_rect).text(
            layout.icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            fallback,
            egui::FontId::proportional(8.0),
            text_color,
        );
    }
    response
}

fn terminal_line_color(severity: TerminalLineSeverity) -> Color32 {
    match severity {
        TerminalLineSeverity::Normal | TerminalLineSeverity::Summary => {
            Color32::from_rgb(232, 232, 228)
        }
        TerminalLineSeverity::Warning => Color32::from_rgb(238, 196, 91),
        TerminalLineSeverity::Error => Color32::from_rgb(244, 105, 105),
        TerminalLineSeverity::Success => Color32::from_rgb(123, 184, 137),
    }
}

fn terminal_line_is_strong(severity: TerminalLineSeverity) -> bool {
    matches!(
        severity,
        TerminalLineSeverity::Error | TerminalLineSeverity::Summary
    )
}

fn draw_index_progress_bar(ui: &mut Ui, width: f32, fraction: Option<f32>, text: &str) {
    let size = egui::vec2(width, 18.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let radius = 6.0;
    let bg = if is_dark_mode() {
        Color32::from_rgb(31, 31, 30)
    } else {
        Color32::from_rgb(215, 215, 210)
    };
    let fill = if is_dark_mode() {
        Color32::from_rgb(69, 111, 132)
    } else {
        Color32::from_rgb(91, 146, 172)
    };
    ui.painter().rect_filled(rect, radius, bg);
    if let Some(fraction) = fraction {
        let fill_width = rect.width() * fraction.clamp(0.0, 1.0);
        if fill_width > 0.0 {
            let fill_rect = egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.left() + fill_width, rect.bottom()),
            );
            ui.painter().rect_filled(fill_rect, radius, fill);
        }
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::TextStyle::Small.resolve(ui.style()),
        text_dark(),
    );
}

/// The kit's game card: emblem, game name, and source path.
///
/// Fills the sidebar width and allows long source paths to reflow as the pane
/// narrows. Zero-width break opportunities after path separators keep Windows
/// paths readable without changing the text the user sees.
fn draw_game_banner_header(
    ui: &mut Ui,
    app: &mut Baboon,
    game: &str,
    path_label: &str,
    profile_id: Option<&str>,
) {
    let texture = app.workspace_banner_texture(ui.ctx(), game, profile_id);
    let title = profile_id
        .and_then(|id| {
            app.custom_editing_kit_profiles
                .iter()
                .find(|profile| profile.id == id)
                .map(|profile| profile.name.clone())
        })
        .unwrap_or_else(|| {
            format!(
                "Tags - {} ({})",
                game_display_name(game),
                game_platform_label(game)
            )
        });
    let read_only = app.custom_editing_kit_profiles.iter().any(|profile| {
        profile.read_only
            && profile.game != "haloce_evolved"
            && (profile_id == Some(profile.id.as_str())
                || profile.is_read_only_for(None, Some(Path::new(path_label))))
    });
    draw_kit_banner_tile(ui, &title, path_label, texture.as_ref(), read_only);
}

/// Used by the kit browser and the live editing-kit form preview.
fn draw_kit_banner_tile(
    ui: &mut Ui,
    title_label: &str,
    path_label: &str,
    texture: Option<&egui::TextureHandle>,
    read_only: bool,
) {
    const EMBLEM: f32 = 72.0;
    const MARGIN: f32 = 8.0;
    const GAP: f32 = 8.0;
    const TITLE_TOP: f32 = 8.0;
    let card_width = ui.available_width();
    let text_width = (card_width - MARGIN * 2.0 - EMBLEM - GAP).max(1.0);
    let title = editing_kit_title_text(ui, title_label, read_only, 14.0, true).into_galley(
        ui,
        Some(egui::TextWrapMode::Wrap),
        text_width,
        TextStyle::Body,
    );
    let wrappable_path = sidebar_wrappable_path_label(path_label);
    let path = egui::WidgetText::from(RichText::new(wrappable_path).color(subtle_dark()).small())
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Wrap),
            text_width,
            TextStyle::Small,
        );

    let text_height = TITLE_TOP + title.size().y + path.size().y;
    let card_height = MARGIN * 2.0 + EMBLEM.max(text_height);
    let (full, _) = ui.allocate_exact_size(Vec2::new(card_width, card_height), Sense::hover());

    let painter = ui.painter_at(full);
    painter.rect_filled(full, 0.0, foundation_documentation_bg());
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            egui::Rect::from_min_size(full.min + Vec2::splat(MARGIN), Vec2::splat(EMBLEM)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let text_x = full.min.x + MARGIN + EMBLEM + GAP;
    let title_y = full.min.y + MARGIN + TITLE_TOP;
    painter.galley(egui::pos2(text_x, title_y), title.clone(), text_dark());
    painter.galley(
        egui::pos2(text_x, title_y + title.size().y),
        path,
        subtle_dark(),
    );
}

fn sidebar_wrappable_path_label(path: &str) -> String {
    let mut wrappable = String::with_capacity(path.len());
    for character in path.chars() {
        wrappable.push(character);
        if matches!(character, '\\' | '/') {
            wrappable.push('\u{200b}');
        }
    }
    wrappable
}

fn sidebar_source_path_label(source: &TagSource) -> String {
    match source {
        TagSource::SingleFile { path } => path.display().to_string(),
        TagSource::LooseFolder { root, .. } => root.display().to_string(),
        TagSource::MonolithicCache { root, .. } => root.display().to_string(),
        TagSource::IoStoreContainerSet { root, .. } => root.display().to_string(),
    }
}

const MONITOR_COMMANDS_BY_GAME: &[(&str, &[&str])] = &[
    (
        "halo2_mcc",
        &[
            "monitor-bitmaps",
            "monitor-bitmaps-data-and-tags",
            "monitor-models",
            "monitor-structures",
        ],
    ),
    (
        "halo3_mcc",
        &[
            "monitor-bitmaps",
            "monitor-models",
            "monitor-models-draft",
            "monitor-strings",
            "monitor-structures",
        ],
    ),
    (
        "halo3odst_mcc",
        &[
            "monitor-bitmaps",
            "monitor-models",
            "monitor-models-draft",
            "monitor-strings",
            "monitor-structures",
        ],
    ),
    (
        "haloreach_mcc",
        &[
            "monitor-bitmaps",
            "monitor-models",
            "monitor-models-draft",
            "monitor-strings",
        ],
    ),
    ("halo4_mcc", &["monitor-bitmaps", "monitor-strings"]),
    ("haloce_mcc", &[]),
];

fn monitor_commands_for_game(game: Option<&str>) -> &'static [&'static str] {
    let Some(game) = game else {
        return &[];
    };
    MONITOR_COMMANDS_BY_GAME
        .iter()
        .find(|(candidate, _)| *candidate == game)
        .map(|(_, commands)| *commands)
        .unwrap_or(&[])
}

#[cfg(test)]
#[path = "ui/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../app/tests/ui_scale_slider.rs"]
mod ui_scale_slider_tests;

#[cfg(test)]
#[path = "../app/tests/shader_option_reads.rs"]
mod shader_option_read_tests;

/// A clickable tag entry row in the Content Explorer. Returns true on click.
fn explorer_entry_row(ui: &mut Ui, entry: &TagEntry) -> bool {
    ui.add(
        egui::Label::new(RichText::new(entry.display_path.replace('\\', "/")).color(text_dark()))
            .sense(Sense::click()),
    )
    .on_hover_text("Click to navigate here")
    .clicked()
}

/// Blend `base` toward `accent` by `t` (0..1). Used for the unsaved-tab tint.
fn tint_toward(base: Color32, accent: Color32, t: f32) -> Color32 {
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(
        lerp(base.r(), accent.r()),
        lerp(base.g(), accent.g()),
        lerp(base.b(), accent.b()),
    )
}

impl Baboon {
    /// `kit_index` is the workspace whose pane is drawing this. Readiness is
    /// resolved against that workspace's editing kit rather than the focused
    /// one, and a launch makes it active first: it saves the tag and starts an
    /// external editor, neither of which should follow the wrong game.
    pub(super) fn draw_scenario_launcher_buttons(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        entry: &TagEntry,
    ) {
        if entry.group_tag != u32::from_be_bytes(*b"scnr") {
            return;
        }
        let key = entry.key.clone();
        // Halo Combat Evolved's Sapien cannot be handed a scenario, and
        // Campaign Evolved has no Sapien at all. Neither is a button worth
        // greying out — a control that can never work reads as something the
        // user has misconfigured.
        let offers_sapien = self.kit_offers_scenario_sapien(kit_index);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let tag_test_ready = self.can_launch_scenario_in_tag_test(kit_index, entry);
            if scenario_launcher_button(
                ui,
                "bytes://baboon_app_icons/tag-test.png",
                include_bytes!("../../assets/App Icons/Tag Test.png"),
                "TagTest",
                tag_test_ready,
            )
            .on_hover_text("Save if needed, then launch this scenario in tag_test")
            .clicked()
            {
                self.active = kit_index;
                self.launch_scenario_in_tag_test(&key);
            }
            if offers_sapien {
                let sapien_ready = self.can_launch_scenario_in_sapien(kit_index, entry);
                if scenario_launcher_button(
                    ui,
                    "bytes://baboon_app_icons/sapien.png",
                    include_bytes!("../../assets/App Icons/Sapien.png"),
                    "Sapien",
                    sapien_ready,
                )
                .on_hover_text("Save if needed, then launch this scenario in Sapien")
                .clicked()
                {
                    self.active = kit_index;
                    self.launch_scenario_in_sapien(&key);
                }
            }
            ui.label(RichText::new("Open scenario in:").color(subtle_dark()));
        });
    }

    fn draw_tool_launcher_buttons(&mut self, ui: &mut Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if launcher_button(ui, self.blender_icon.as_ref(), "B", true)
                .on_hover_text("Launch Blender")
                .clicked()
            {
                self.launch_blender();
            }

            let tag_test_ready = self
                .kit_tool_path(self.tag_test_executable())
                .is_some_and(|path| path.is_file());
            if launcher_button(ui, self.tag_test_icon.as_ref(), "T", tag_test_ready)
                .on_hover_text("Launch tag_test without an auto-start scenario")
                .clicked()
            {
                self.launch_tag_test();
            }

            let sapien_ready = self
                .kit_tool_path("sapien.exe")
                .is_some_and(|path| path.is_file());
            if launcher_button(ui, self.sapien_icon.as_ref(), "S", sapien_ready)
                .on_hover_text("Launch Sapien without an auto-start scenario")
                .clicked()
            {
                self.launch_sapien();
            }

            // Campaign Evolved holds unsaved edits in a project rather than in
            // the game's files, so a workspace accumulates stashed
            // modifications across sessions. This is the way back to the
            // shipped tags; it is drawn here, outside the workspace tree, so it
            // always acts on the focused kit.
            if self.current_source_is_campaign_project_capable(self.active) {
                let stashed = self.stashed_campaign_tags(self.active);
                let unsaved = self.kits[self.active]
                    .parsed_tags
                    .values()
                    .filter(|document| document.dirty.is_set())
                    .count();
                let anything = !stashed.is_empty() || unsaved > 0;
                let icon = button_icon_image(ui, ButtonIcon::Garbage, text_dark(), 16.0);
                let response = ui.add_enabled(anything, egui::Button::image(icon));
                if response
                    .on_hover_text(
                        "Clear this workspace's unsaved modifications, returning every tag to \
                         the way the game ships it",
                    )
                    .on_disabled_hover_text("This workspace has no unsaved modifications")
                    .clicked()
                {
                    self.clear_stash_confirm = Some(ClearStashConfirm {
                        kit: self.active_kit_id(),
                        stashed,
                        unsaved,
                    });
                }
            }
        });
    }

    fn draw_monitor_tools_menu(&mut self, ui: &mut Ui) {
        let game = self.source().and_then(|source| source.game.as_deref());
        let commands = monitor_commands_for_game(game);
        let enabled = !commands.is_empty();
        let ctx = ui.ctx().clone();
        let menu = ui
            .add_enabled_ui(enabled, |ui| {
                right_opening_menu_button(ui, "Monitor", 222.0, |ui| {
                    style_list_menu(ui);
                    ui.set_min_width(210.0);
                    for command in commands {
                        if ui.button(*command).clicked() {
                            return Some(*command);
                        }
                    }
                    None
                })
            })
            .inner;
        if let Some(command) = menu.inner.flatten() {
            self.submit_terminal_command(format!("tool {command}"), ctx);
            ui.close_menu();
        }
        let response = menu.response;
        if enabled {
            response.on_hover_text("Run monitor command");
        } else {
            response.on_disabled_hover_text("No monitor commands available for this game");
        }
    }

    /// Tools ▸ Assets: the asset libraries, browsed across the whole kit rather
    /// than one tag at a time.
    fn draw_assets_tools_menu(&mut self, ui: &mut Ui) {
        let enabled = self.source().is_some();
        let menu = ui
            .add_enabled_ui(enabled, |ui| {
                right_opening_menu_button(ui, "Assets", 222.0, |ui| {
                    style_list_menu(ui);
                    ui.set_min_width(210.0);
                    if ui.button("Bitmap Browser").clicked() {
                        return Some("bitmap");
                    }
                    if ui.button("Model Browser").clicked() {
                        return Some("model");
                    }
                    // Baboon's own import pipelines only cover Halo 3 so far,
                    // so the entry only appears there.
                    if self.active_kit_is_halo3() && ui.button("Blam!").clicked() {
                        return Some("blam");
                    }
                    None
                })
            })
            .inner;
        if let Some(asset) = menu.inner.flatten() {
            match asset {
                "bitmap" => self.open_bitmap_library(),
                "model" => self.open_model_library(),
                "blam" => {
                    // Re-detect on every open: the data folder may have
                    // changed since the pane was last shown.
                    self.kits[self.active].blam.scanned_path = None;
                    self.kits[self.active].open_tag_pane(BLAM_KEY);
                }
                _ => {}
            }
            ui.close_menu();
        }
        let response = menu.response;
        if !enabled {
            response.on_disabled_hover_text("Load an editing kit to browse its assets");
        }
    }

    /// Per-tag keyword chips (add via Enter/Add, remove via the chip button).
    /// Keywords live in an external sidecar, not the tag binary.
    fn draw_keyword_bar(&mut self, ui: &mut Ui, kit_index: usize, tag_key: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new("Keywords:").color(subtle_dark()));
            let existing = self.kits[kit_index].keywords.keywords(tag_key).to_vec();
            let mut remove: Option<String> = None;
            for keyword in &existing {
                if keyword_pill(ui, tag_key, keyword) {
                    remove = Some(keyword.clone());
                }
            }
            if let Some(keyword) = remove {
                self.kits[kit_index].keywords.remove(tag_key, &keyword);
            }
            let keyword_field = Frame::none()
                .fill(foundation_input())
                .rounding(egui::Rounding::same(BUTTON_HEIGHT / 2.0))
                .inner_margin(egui::Margin::same(2.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.spacing_mut().interact_size.y = 20.0;
                    ui.set_height(20.0);
                    ui.horizontal(|ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.keyword_input)
                                .hint_text(placeholder_text("add keyword"))
                                .desired_width(120.0)
                                .frame(false),
                        );
                        let add_response = ui
                            .scope(|ui| {
                                ui.spacing_mut().interact_size = Vec2::splat(20.0);
                                ui.add(
                                    egui::Button::new("")
                                        .min_size(Vec2::splat(20.0))
                                        .rounding(egui::Rounding::same(10.0)),
                                )
                            })
                            .inner;
                        let add_icon_rect = egui::Rect::from_center_size(
                            add_response.rect.center(),
                            Vec2::splat(BUTTON_ICON_SIZE),
                        );
                        paint_button_icon_at(ui, ButtonIcon::Add, add_icon_rect, text_dark());
                        let add_clicked = add_response.on_hover_text("Add keyword").clicked();
                        (resp, add_clicked)
                    })
                    .inner
                });
            let (resp, add_clicked) = keyword_field.inner;
            ui.painter().rect_stroke(
                keyword_field.response.rect,
                egui::Rounding::same(BUTTON_HEIGHT / 2.0),
                pane_header_input_stroke(
                    ui,
                    keyword_field.response.hovered() || resp.hovered(),
                    resp.has_focus(),
                ),
            );
            let submitted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (add_clicked || submitted) && !self.keyword_input.trim().is_empty() {
                self.kits[kit_index]
                    .keywords
                    .add(tag_key, &self.keyword_input);
                self.keyword_input.clear();
            }
        });
    }
}

/// Scenario-header launcher using Baboon's bundled application artwork rather
/// than the executable icon discovered for the global tools toolbar.
fn scenario_launcher_button(
    ui: &mut Ui,
    image_uri: &'static str,
    image_bytes: &'static [u8],
    label: &str,
    enabled: bool,
) -> egui::Response {
    let image = egui::Image::from_bytes(image_uri, image_bytes)
        .fit_to_exact_size(Vec2::splat(BUTTON_ICON_SIZE));
    ui.add_enabled(
        enabled,
        egui::Button::image_and_text(image, label).min_size(Vec2::new(0.0, BUTTON_HEIGHT)),
    )
}

fn keyword_pill(ui: &mut Ui, tag_key: &str, keyword: &str) -> bool {
    const TEXT_PADDING: f32 = 8.0;
    const REMOVE_WIDTH: f32 = 20.0;
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(keyword.to_owned(), font_id, text_dark());
    let width = TEXT_PADDING + galley.size().x + REMOVE_WIDTH + 4.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, BUTTON_HEIGHT), Sense::hover());
    let background = editor_bg();
    let target = if is_dark_mode() {
        Color32::WHITE
    } else {
        Color32::BLACK
    };
    let blend =
        |base: u8, overlay: u8| (base as f32 + (overlay as f32 - base as f32) * 0.05).round() as u8;
    let fill = Color32::from_rgb(
        blend(background.r(), target.r()),
        blend(background.g(), target.g()),
        blend(background.b(), target.b()),
    );
    ui.painter()
        .rect_filled(rect, egui::Rounding::same(BUTTON_HEIGHT / 2.0), fill);
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + TEXT_PADDING, rect.top()),
        egui::pos2(rect.right() - REMOVE_WIDTH, rect.bottom()),
    );
    let text_pos = egui::Align2::LEFT_CENTER
        .align_size_within_rect(galley.size(), text_rect)
        .min;
    ui.painter().galley(text_pos, galley, text_dark());

    let remove_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - REMOVE_WIDTH, rect.top()),
        rect.right_bottom(),
    );
    let remove = ui
        .interact(
            remove_rect,
            ui.make_persistent_id(("keyword_remove", tag_key, keyword)),
            Sense::click(),
        )
        .on_hover_text("Remove keyword");
    let stroke = ui.style().interact(&remove).fg_stroke;
    let cross = egui::Rect::from_center_size(remove_rect.center(), Vec2::splat(7.0));
    ui.painter()
        .line_segment([cross.left_top(), cross.right_bottom()], stroke);
    ui.painter()
        .line_segment([cross.right_top(), cross.left_bottom()], stroke);
    remove.clicked()
}

impl eframe::App for Baboon {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.window_state.observe(ctx);
        self.draw_root_ui(ctx, frame);
        self.run_deferred_file_action(ctx);
        // A container write whose workspace closed while it was in flight left
        // a mapping released and an Unreal package mount idle. Nothing else
        // would ever put those back.
        self.sweep_container_write_leases(ctx);
        self.maybe_autosave_campaign_projects(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.window_state.persist_now();
        self.persist_session_on_exit();
    }
}
