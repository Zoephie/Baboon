//! Top-level windows, menus, dialogs, and frame composition for [`Baboon`].
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

pub(in crate::app) const PANE_HEADER_ICON_SIZE: f32 = 32.0;
pub(in crate::app) const PANE_HEADER_SECTION_GAP: f32 = 20.0;
pub(in crate::app) const PANE_HEADER_ICON_TEXT_GAP: f32 = 10.0;

/// The shared loaded-workspace state for a canvas that has no open document.
pub(in crate::app) fn centered_empty_state(ui: &mut Ui, detail: &str) {
    const IMAGE_SIZE: f32 = 256.0;
    const CONTENT_HEIGHT: f32 = IMAGE_SIZE + 64.0;

    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.add_space(((ui.available_height() - CONTENT_HEIGHT) * 0.5).max(0.0));
        ui.add(
            egui::Image::from_bytes(
                "bytes://baboon_branding/empty-state.svg",
                include_root_bytes!("assets/branding/empty-state.svg").as_slice(),
            )
            .fit_to_exact_size(Vec2::splat(IMAGE_SIZE)),
        );
        ui.heading(
            RichText::new("Nothing’s Open!")
                .color(text_dark())
                .strong()
                .italics(),
        );
        ui.label(RichText::new(detail).color(subtle_dark()));
    });
}
pub(in crate::app) const PANE_HEADER_ACTION_GAP: f32 = 4.0;
pub(in crate::app) const PANE_HEADER_BOTTOM_SPACE: f32 = 20.0;
const PANE_HEADER_WIDE_BREAKPOINT: f32 = 600.0;
const PANE_HEADER_MIN_LEFT_WIDTH: f32 = 200.0;
pub(in crate::app) const PANE_HEADER_COMMON_ACTIONS_WIDTH: f32 = 205.0;
const BROWSER_SEARCH_HEIGHT: f32 = 24.0;
const BROWSER_SEARCH_RADIUS: f32 = BROWSER_SEARCH_HEIGHT * 0.5;
const BROWSER_SEARCH_ICON_SIZE: f32 = 16.0;
const BROWSER_SEARCH_LEFT_PADDING: f32 = 4.0;
const BROWSER_SEARCH_ICON_TEXT_GAP: f32 = 8.0;

/// Width of the title column when pane actions can remain beside it. Returning
/// `None` is the shared signal for tag and folder headers to put actions below
/// the title instead, preventing either header from overlapping at narrow
/// docked or window sizes.
pub(in crate::app) fn pane_header_inline_left_width(available: f32, action_width: f32) -> Option<f32> {
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
        Stroke::new(1.0_f32, foundation_input_edge())
    }
}

/// The shared browser search field. Its icon lives inside the 24-point pill so
/// the compact sidebar and a wide folder pane use exactly the same geometry.
pub(in crate::app) fn browser_search_field(ui: &mut Ui, value: &mut String, hint: &str) -> egui::Response {
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

    let clear_rect = egui::Rect::from_center_size(
        egui::pos2(
            rect.right() - BROWSER_SEARCH_LEFT_PADDING - BROWSER_SEARCH_ICON_SIZE * 0.5,
            rect.center().y,
        ),
        Vec2::splat(BROWSER_SEARCH_ICON_SIZE),
    );

    let edit_rect = egui::Rect::from_min_max(
        egui::pos2(icon_rect.right() + BROWSER_SEARCH_ICON_TEXT_GAP, rect.top()),
        egui::pos2(clear_rect.left() - BROWSER_SEARCH_ICON_TEXT_GAP, rect.bottom()),
    );
    let edit_response = ui.put(
        edit_rect,
        egui::TextEdit::singleline(value)
            .hint_text(placeholder_text(hint))
            .text_color(text_dark())
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::same(0))
            .vertical_align(egui::Align::Center)
            .min_size(edit_rect.size()),
    );
    if icon_response.clicked() {
        edit_response.request_focus();
    }
    let mut response = background_response
        .union(icon_response)
        .union(edit_response.clone());
    if !value.is_empty() {
        let clear = search_clear_control_at(
            ui,
            clear_rect,
            edit_response.id.with("clear_search"),
            BROWSER_SEARCH_ICON_SIZE * 0.5,
        );
        if clear.clicked() {
            value.clear();
            edit_response.request_focus();
            response.mark_changed();
        }
        response = response.union(clear);
    }
    ui.painter().rect_stroke(
        rect,
        BROWSER_SEARCH_RADIUS,
        pane_header_input_stroke(ui, response.hovered(), edit_response.has_focus()),
        egui::StrokeKind::Middle,
    );
    response
}

pub(in crate::app) fn browser_favorites_divider(ui: &mut Ui, favorites_visible: bool) {
    if favorites_visible {
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
    }
}

pub(in crate::app) fn pane_header_path_parts(display_path: &str) -> (Vec<(String, PathBuf)>, String) {
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
pub(in crate::app) fn pane_header_breadcrumbs(
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
                egui::CornerRadius::same(4),
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

pub(in crate::app) fn navigate_folder_browser(pane: &mut FolderBrowserState, path: PathBuf, label: String) {
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
pub(in crate::app) fn wheel_scroll_tab_bar(ui: &Ui, scroll_offset: &mut f32) {
    if !ui.rect_contains_pointer(ui.max_rect()) {
        return;
    }
    let delta = ui.input(|input| input.smooth_scroll_delta);
    *scroll_offset -= delta.x + delta.y;
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
pub(in crate::app) enum EditingKitMenuEntry {
    Custom(CustomEditingKitProfile),
    BuiltIn(EditingKitShortcut),
}

pub(in crate::app) fn visible_editing_kit_menu_entries(
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

pub(in crate::app) const EDITING_KIT_MENU_MIN_WIDTH: f32 = 240.0;
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

pub(in crate::app) fn editing_kit_title_text(
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

pub(in crate::app) fn editing_kit_menu_row_with_read_only(
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

pub(in crate::app) fn terminal_line_color(severity: TerminalLineSeverity) -> Color32 {
    match severity {
        TerminalLineSeverity::Normal | TerminalLineSeverity::Summary => {
            Color32::from_rgb(232, 232, 228)
        }
        TerminalLineSeverity::Warning => Color32::from_rgb(238, 196, 91),
        TerminalLineSeverity::Error => Color32::from_rgb(244, 105, 105),
        TerminalLineSeverity::Success => Color32::from_rgb(123, 184, 137),
    }
}

pub(in crate::app) fn terminal_line_is_strong(severity: TerminalLineSeverity) -> bool {
    matches!(
        severity,
        TerminalLineSeverity::Error | TerminalLineSeverity::Summary
    )
}

pub(in crate::app) fn draw_index_progress_bar(ui: &mut Ui, width: f32, fraction: Option<f32>, text: &str) {
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
/// `texture` is the banner [`Baboon::workspace_banner_texture`] resolved for
/// this game and profile; resolving it can load it, so it is done before the
/// draw.
pub(in crate::app) fn draw_game_banner_header(
    ui: &mut Ui,
    model: &Model,
    texture: Option<&egui::TextureHandle>,
    game: GameId,
    path_label: &str,
    profile_id: Option<&str>,
) {
    let title = profile_id
        .and_then(|id| {
            model.prefs
                .custom_editing_kit_profiles
                .iter()
                .find(|profile| profile.id == id)
                .map(|profile| profile.name.clone())
        })
        .unwrap_or_else(|| {
            format!(
                "Tags - {} ({})",
                game.display_name(),
                game_platform_label(game)
            )
        });
    let read_only = model.prefs.custom_editing_kit_profiles.iter().any(|profile| {
        profile.read_only
            && !profile.is_campaign_evolved()
            && (profile_id == Some(profile.id.as_str())
                || profile.is_read_only_for(None, Some(Path::new(path_label))))
    });
    draw_kit_banner_tile(ui, &title, path_label, texture, read_only);
}

/// Used by the kit browser and the live editing-kit form preview.
pub(in crate::app) fn draw_kit_banner_tile(
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

pub(in crate::app) fn sidebar_wrappable_path_label(path: &str) -> String {
    let mut wrappable = String::with_capacity(path.len());
    for character in path.chars() {
        wrappable.push(character);
        if matches!(character, '\\' | '/') {
            wrappable.push('\u{200b}');
        }
    }
    wrappable
}

pub(in crate::app) fn sidebar_source_path_label(source: &TagSource) -> String {
    match source {
        TagSource::SingleFile { path } => path.display().to_string(),
        TagSource::LooseFolder { root, .. } => root.display().to_string(),
        TagSource::MonolithicCache { root, .. } => root.display().to_string(),
        TagSource::IoStoreContainerSet { root, .. } => root.display().to_string(),
    }
}

pub(in crate::app) fn monitor_commands_for_game(game: Option<GameId>) -> &'static [&'static str] {
    game
        .map_or(&[], GameFacts::monitor_commands)
}

/// A clickable tag entry row in the Content Explorer. Returns true on click.
pub(in crate::app) fn explorer_entry_row(ui: &mut Ui, entry: &TagEntry) -> bool {
    ui.add(
        egui::Label::new(RichText::new(entry.display_path.replace('\\', "/")).color(text_dark()))
            .sense(Sense::click()),
    )
    .on_hover_text("Click to navigate here")
    .clicked()
}

/// `probe`'s answer, re-asked at most once a second.
///
/// For file-system questions the UI asks every frame — is a tool there, does
/// an output exist. Each is a stat, and on a slow or network drive a stat per
/// frame is a stall per frame. A file created or deleted outside Baboon shows
/// up within the second. Keyed by `key` in egui's memory.
pub(in crate::app) fn recheck_cached<T: Clone + Send + Sync + 'static>(
    ctx: &egui::Context,
    key: impl std::hash::Hash + std::fmt::Debug,
    probe: impl FnOnce() -> T,
) -> T {
    const RECHECK_SECONDS: f64 = 1.0;
    let key = egui::Id::new(("recheck_cached", key));
    let now = ctx.input(|input| input.time);
    if let Some((value, checked_at)) = ctx.data(|data| data.get_temp::<(T, f64)>(key))
        && (0.0..RECHECK_SECONDS).contains(&(now - checked_at))
    {
        return value;
    }
    let value = probe();
    ctx.data_mut(|data| data.insert_temp(key, (value.clone(), now)));
    value
}

/// Whether `path` is a file, re-checked at most once a second.
pub(in crate::app) fn is_file_cached(ctx: &egui::Context, path: &std::path::Path) -> bool {
    recheck_cached(ctx, ("is_file", path), || path.is_file())
}

/// Blend `base` toward `accent` by `t` (0..1). Used for the unsaved-tab tint.
pub(in crate::app) fn tint_toward(base: Color32, accent: Color32, t: f32) -> Color32 {
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(
        lerp(base.r(), accent.r()),
        lerp(base.g(), accent.g()),
        lerp(base.b(), accent.b()),
    )
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
        .rect_filled(rect, egui::CornerRadius::same((BUTTON_HEIGHT / 2.0) as u8), fill);
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
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.native_clock = raw_input.time;
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.run_logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw_root_ui(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // A quit that never asks the window to close (macOS Cmd+Q) skips the
        // close request, which is where waiting checkpoints are otherwise flushed.
        self.flush_all_chimp_checkpoints();
        // The per-frame prefs write is throttled; whatever changed in the last
        // second would otherwise be lost.
        self.persist_prefs_if_changed();
        self.window_state.persist_now();
        self.save_keyword_sidecars();
        self.persist_session_on_exit();
    }
}

impl Baboon {
    /// One whole application frame while the window shows: what eframe runs,
    /// `App::logic` and then `App::ui`. For headless tests, which cannot
    /// build the `eframe::Frame` those take and need not, since nothing here
    /// uses one — so they drive exactly the frame the window does.
    #[cfg(test)]
    pub(crate) fn run_frame(&mut self, ui: &mut egui::Ui) {
        self.run_logic(&ui.ctx().clone());
        self.draw_root_ui(ui);
    }

    /// The part of a frame that needs no UI. eframe runs it before the UI,
    /// and on its own while the window is minimized or covered, when there is
    /// no UI pass at all; so background work keeps landing, saves keep being
    /// written and closing the window still asks first.
    pub(crate) fn run_logic(&mut self, ctx: &egui::Context) {
        // While the window is hidden egui's clock stays at the last frame
        // shown, and every timer below reads it.
        if let Some(now) = self.native_clock.take() {
            ctx.input_mut(|input| input.time = input.time.max(now));
        }
        self.window_state.observe(ctx);
        if self.dialogs.get::<FirstRunWizardState>().is_none() {
            self.process_worker_messages(ctx);
            self.expire_status(ctx);
        }
        // Raised by the previous frame, whose UI has since committed any edit
        // that was still focused, so the action sees it.
        self.run_deferred_file_action(ctx);
        self.save_keyword_sidecars();
        if self.dialogs.get::<FirstRunWizardState>().is_none() {
            // Defers the close, so the UI commits a focused edit before the
            // next frame decides whether there is anything to save.
            self.handle_app_close_request(ctx);
            self.persist_prefs_throttled(ctx.input(|input| input.time));
        }
        // A container write whose workspace closed while it was in flight left
        // a mapping released and an Unreal package mount idle. Nothing else
        // would ever put those back.
        self.sweep_container_write_leases(ctx);
        self.maybe_autosave_campaign_projects(ctx);
        // Every kit, not only one whose Chimp workspace is on screen: a
        // checkpoint waiting on a workspace the user switched away from would
        // otherwise wait until they came back.
        for kit_index in 0..self.model.kits.len() {
            self.run_due_chimp_checkpoints(kit_index, ctx);
        }
    }
}

/// Per-tag keyword chips (add via Enter/Add, remove via the chip button).
/// Keywords live in an external sidecar, not the tag binary. Adding and
/// removing are commands; the draft being typed is this pane's own.
pub(in crate::app) fn draw_keyword_bar(cx: &Ctx, ui: &mut Ui, kit_index: usize, tag_key: &str) {
    let kit = cx.model.kits[kit_index].id;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new("Keywords:").color(subtle_dark()));
        let existing = cx.model.kits[kit_index].keywords.keywords(tag_key).to_vec();
        let mut remove: Option<String> = None;
        for keyword in &existing {
            if keyword_pill(ui, tag_key, keyword) {
                remove = Some(keyword.clone());
            }
        }
        if let Some(keyword) = remove {
            cx.send(BrowserCommand::RemoveKeyword {
                kit,
                key: tag_key.to_owned(),
                keyword,
            });
        }
        // The draft is this pane's own. It used to be one field on the app,
        // so text typed into one pane's box showed in every other pane.
        let draft_id = ui.make_persistent_id(("keyword_input", tag_key));
        let mut draft = ui
            .data_mut(|data| data.get_temp::<String>(draft_id))
            .unwrap_or_default();
        let keyword_field = Frame::NONE
            .fill(foundation_input())
            .corner_radius(egui::CornerRadius::same((BUTTON_HEIGHT / 2.0) as u8))
            .inner_margin(egui::Margin::same(2))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().interact_size.y = 20.0;
                ui.set_height(20.0);
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut draft)
                            .hint_text(placeholder_text("add keyword"))
                            .desired_width(120.0)
                            .frame(egui::Frame::NONE),
                    );
                    let add_response = ui
                        .scope(|ui| {
                            ui.spacing_mut().interact_size = Vec2::splat(20.0);
                            ui.add(
                                egui::Button::new("")
                                    .min_size(Vec2::splat(20.0))
                                    .corner_radius(egui::CornerRadius::same(10)),
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
            egui::CornerRadius::same((BUTTON_HEIGHT / 2.0) as u8),
            pane_header_input_stroke(
                ui,
                keyword_field.response.hovered() || resp.hovered(),
                resp.has_focus(),
            ),
            egui::StrokeKind::Middle,
        );
        let submitted = lost_focus_once(&resp) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (add_clicked || submitted) && !draft.trim().is_empty() {
            cx.send(BrowserCommand::AddKeyword {
                kit,
                key: tag_key.to_owned(),
                keyword: draft.clone(),
            });
            draft.clear();
        }
        ui.data_mut(|data| data.insert_temp(draft_id, draft));
    });
}

/// The scenario header's launch buttons. `kit_index` is the workspace whose
/// pane is drawing this. Readiness is resolved against that workspace's
/// editing kit rather than the focused one, and a launch makes it active
/// first: it saves the tag and starts an external editor, neither of which
/// should follow the wrong game.
pub(in crate::app) fn draw_scenario_launcher_buttons(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    entry: &TagEntry,
) {
    let kit = cx.model.kits[kit_index].id;
    if entry.group_tag != u32::from_be_bytes(*b"scnr") {
        return;
    }
    let key = entry.key.clone();
    // Halo Combat Evolved's Sapien cannot be handed a scenario, and
    // Campaign Evolved has no Sapien at all. Neither is a button worth
    // greying out — a control that can never work reads as something the
    // user has misconfigured.
    let offers_sapien = cx.model.kit_offers_scenario_sapien(kit_index);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let tag_test_ready = cx.model.can_launch_scenario_in_tag_test(kit_index, entry);
        if scenario_launcher_button(
            ui,
            "bytes://baboon_app_icons/tag-test.png",
            include_root_bytes!("assets/App Icons/Tag Test.png"),
            "TagTest",
            tag_test_ready,
        )
        .on_hover_text("Save if needed, then launch this scenario in tag_test")
        .clicked()
        {
            cx.send(KitsCommand::LaunchScenario {
                kit,
                key: key.clone(),
                tool: ScenarioTool::TagTest,
            });
        }
        if offers_sapien {
            let sapien_ready = cx.model.can_launch_scenario_in_sapien(kit_index, entry);
            if scenario_launcher_button(
                ui,
                "bytes://baboon_app_icons/sapien.png",
                include_root_bytes!("assets/App Icons/Sapien.png"),
                "Sapien",
                sapien_ready,
            )
            .on_hover_text("Save if needed, then launch this scenario in Sapien")
            .clicked()
            {
                cx.send(KitsCommand::LaunchScenario {
                    kit,
                    key: key.clone(),
                    tool: ScenarioTool::Sapien,
                });
            }
        }
        ui.label(RichText::new("Open scenario in:").color(subtle_dark()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::browser::{
        ContainerFolderDialog, ContentExplorer, ExtractKind, ExtractTargetPrompt,
        LooseFolderRenameState, RenameTagState, TagNameOperation,
    };
    use crate::app::chimp::{ChimpLevelExportPrompt, ChimpMeshTexturePrompt, ChimpTextureExportPrompt};
    use crate::app::documents::{ChimpDiscardPrompt, DirtyTagEntry, SaveChangesPrompt};
    use crate::app::editor::combo_box_with_scroll;
    use crate::app::editor::{
        BlockConfirm, ColorPopupWindow, FunctionPopup, FunctionPopupWindow, FunctionView,
        MaterialColorPopup, TagReferencePickerState, TagReferencePickerWindow, TsvPasteState,
        constant_function_hex,
    };
    use crate::app::export::{ContainerDumpConfirm, ContainerDumpScope};
    use crate::app::help::HelpWindow;
    use crate::app::import::{
        CacheImportDialog, CacheImportTarget, ImportMode, ImportTagDialog, PendingImport, ReplaceChoice,
    };
    use crate::app::loose_fixture::*;
    use crate::app::mods::{
        CampaignProjectSnapshot, ExportedMod, ModExportChange, ModExportDialog, ModExportRow,
        OverwriteConfirm,
    };
    use crate::app::runtime_poke::{PokeDialog, PokeDialogState};
    use crate::app::search::QueryResultsWindow;
    use crate::app::tag_ops::{ContainerDuplicateConfirm, DeleteConfirm, DeleteKind};
    use crate::core::document::value::decode_hex;
    use crate::core::source::{LoadedSourceData, TagEntry, TagEntryLocation, TagSource};
    use std::collections::{HashMap, HashSet};
    use std::time::{Duration, Instant};

    // Unit tests for top-level UI helpers.
    // It owns test-only characterization and does not participate in runtime application behavior.

    #[test]
    fn editing_kit_read_only_titles_use_a_muted_suffix_without_changing_the_name() {
        let style = foundation_style();
        for read_only in [false, true] {
            let egui::WidgetText::LayoutJob(job) =
                editing_kit_title_text_with_style(&style, "Protected kit", read_only, 14.0, true)
            else {
                panic!("expected styled title");
            };
            assert_eq!(
                job.text,
                if read_only {
                    "Protected kit (read-only)"
                } else {
                    "Protected kit"
                }
            );
            assert_eq!(job.sections[0].format.color, text_dark());
            if read_only {
                assert_eq!(
                    job.sections[1].format.color,
                    text_dark().gamma_multiply(0.5)
                );
            }
        }
        let ctx = egui::Context::default();
        ctx.set_global_style(style);
        let output = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                editing_kit_menu_row_with_read_only(ui, "Protected kit", "EK", None, true, true, true);
                draw_kit_banner_tile(ui, "Protected kit", "C:/Kits/Protected", None, true);
            });
        });
        assert_eq!(
            output
                .shapes
                .iter()
                .filter(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Protected kit (read-only)"))
                .count(),
            2
        );
    }

    #[test]
    fn shared_browser_buttons_use_standard_point_sizes() {
        for scale in [MIN_UI_SCALE, MAX_UI_SCALE] {
            let ctx = egui::Context::default();
            ctx.set_zoom_factor(scale);
            let mut mode_rect = egui::Rect::NOTHING;
            let mut menu_rect = egui::Rect::NOTHING;

            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(320.0, 100.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            mode_rect = selectable_icon_text_button(
                                ui,
                                ButtonIcon::FolderOpen,
                                "Folders",
                                true,
                            )
                            .rect;
                            menu_rect = icon_menu_button(ui, ButtonIcon::Sort, "Sort", |_| {}).rect;
                        });
                    });
                },
            );

            assert_eq!(mode_rect.height(), BUTTON_HEIGHT);
            assert_eq!(menu_rect.size(), ICON_BUTTON_SIZE);
        }
    }

    #[test]
    fn scrolling_dropdown_matches_button_height() {
        let ctx = egui::Context::default();
        ctx.set_global_style(foundation_style());
        let mut dropdown_height = 0.0;
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(320.0, 100.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    dropdown_height = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt("button_height_test").selected_text("0. default"),
                        |_| {},
                    )
                    .0
                    .response
                    .rect
                    .height();
                });
            },
        );
        assert_eq!(dropdown_height, BUTTON_HEIGHT);
    }

    #[test]
    fn pane_header_breadcrumbs_accumulate_clickable_folder_paths() {
        let (breadcrumbs, title) = pane_header_path_parts("objects\\characters/brute/brute.biped");

        assert_eq!(title, "brute.biped");
        assert_eq!(
            breadcrumbs,
            vec![
                ("objects".to_owned(), PathBuf::from("objects")),
                (
                    "characters".to_owned(),
                    PathBuf::from("objects").join("characters"),
                ),
                (
                    "brute".to_owned(),
                    PathBuf::from("objects").join("characters").join("brute"),
                ),
            ]
        );
    }

    #[test]
    fn pane_header_two_line_title_is_centered_inside_the_icon_height() {
        let ctx = egui::Context::default();
        let mut icon_rect = egui::Rect::NOTHING;
        let mut title_rect = egui::Rect::NOTHING;
        let breadcrumbs = vec![
            ("objects".to_owned(), PathBuf::from("objects")),
            (
                "characters".to_owned(),
                PathBuf::from("objects").join("characters"),
            ),
            (
                "brute".to_owned(),
                PathBuf::from("objects").join("characters").join("brute"),
            ),
        ];

        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(500.0, 100.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        (icon_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(PANE_HEADER_ICON_SIZE), Sense::hover());
                        title_rect = ui
                            .vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                pane_header_breadcrumbs(ui, &breadcrumbs);
                                ui.label(
                                    RichText::new("brute.model")
                                        .size(15.0)
                                        .strong()
                                        .color(text_dark()),
                                );
                            })
                            .response
                            .rect;
                    });
                });
            },
        );

        assert!(title_rect.height() <= PANE_HEADER_ICON_SIZE);
        assert!((title_rect.center().y - icon_rect.center().y).abs() <= 0.5);
    }

    #[test]
    fn pane_headers_share_the_same_narrow_action_breakpoint() {
        assert_eq!(
            pane_header_inline_left_width(PANE_HEADER_WIDE_BREAKPOINT - 1.0, 205.0),
            None
        );
        assert_eq!(
            pane_header_inline_left_width(PANE_HEADER_WIDE_BREAKPOINT, 205.0),
            Some(375.0)
        );
    }

    #[test]
    fn custom_header_inputs_use_standard_hover_and_focus_strokes() {
        let ctx = egui::Context::default();
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                assert_eq!(
                    pane_header_input_stroke(ui, false, false),
                    Stroke::new(1.0_f32, foundation_input_edge())
                );
                assert_eq!(
                    pane_header_input_stroke(ui, true, false),
                    ui.visuals().widgets.hovered.bg_stroke
                );
                assert_eq!(
                    pane_header_input_stroke(ui, true, true),
                    ui.visuals().selection.stroke
                );
            });
        });
    }

    #[test]
    fn editing_kit_menu_uses_each_shortcut_once_in_reverse_engine_order() {
        let games: Vec<&str> = editing_kit_menu_shortcuts()
            .map(|shortcut| shortcut.game.as_str())
            .collect();

        assert_eq!(
            games,
            vec![
                "haloce_evolved",
                "halo2amp_mcc",
                "halo4_mcc",
                "haloreach_mcc",
                "halo3odst_mcc",
                "halo3_mcc",
                "halo2_mcc",
                "haloce_mcc",
            ]
        );
        assert_eq!(
            games
                .iter()
                .map(|game| game_display_name(game))
                .collect::<Vec<_>>(),
            vec![
                "Halo: Campaign Evolved",
                "Halo 2 Anniversary Multiplayer",
                "Halo 4",
                "Halo: Reach",
                "Halo 3: ODST",
                "Halo 3",
                "Halo 2",
                "Halo: Combat Evolved",
            ]
        );
        assert_eq!(
            games.iter().copied().collect::<HashSet<_>>().len(),
            games.len()
        );
        assert_eq!(games.len(), EDITING_KIT_SHORTCUTS.len());
    }

    #[test]
    fn editing_kit_menu_filters_invalid_built_ins_without_reordering_valid_ones() {
        let root = std::env::temp_dir().join(format!(
            "baboon-visible-kits-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let h2 = root.join("h2");
        let h4 = root.join("h4");
        let invalid = root.join("h3");
        std::fs::create_dir_all(h2.join("tags")).unwrap();
        std::fs::create_dir_all(h4.join("tags")).unwrap();
        std::fs::create_dir_all(&invalid).unwrap();
        let paths = HashMap::from([
            ("halo2_mcc".to_owned(), h2),
            ("halo4_mcc".to_owned(), h4),
            ("halo3_mcc".to_owned(), invalid),
        ]);

        let validation = EditingKitValidationCache::new(&paths, &[]);
        let games = visible_builtin_editing_kit_shortcuts(&validation)
            .into_iter()
            .map(|shortcut| shortcut.game.as_str())
            .collect::<Vec<_>>();
        assert_eq!(games, vec!["halo4_mcc", "halo2_mcc"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn shared_menu_entries_put_custom_profiles_first_in_creation_order() {
        let root = std::env::temp_dir().join(format!(
            "baboon-menu-order-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let h2 = root.join("h2");
        std::fs::create_dir_all(h2.join("tags")).unwrap();
        let profiles = vec![
            CustomEditingKitProfile {
                read_only: false,
                git_tracked: false,
                id: "one".to_owned(),
                name: "First".to_owned(),
                game: "halo3_mcc".to_owned(),
                root: root.join("temporarily-missing-one"),
                icon: None,
                tags_folder: None,
                data_folder: None,
            },
            CustomEditingKitProfile {
                read_only: false,
                git_tracked: false,
                id: "two".to_owned(),
                name: "Second".to_owned(),
                game: "haloreach_mcc".to_owned(),
                root: root.join("temporarily-missing-two"),
                icon: None,
                tags_folder: None,
                data_folder: None,
            },
        ];
        let paths = HashMap::from([("halo2_mcc".to_owned(), h2)]);
        let validation = EditingKitValidationCache::new(&paths, &profiles);
        let entries = visible_editing_kit_menu_entries(&profiles, &validation);

        assert!(matches!(
            &entries[0],
            EditingKitMenuEntry::Custom(profile) if profile.id == "one"
        ));
        assert!(matches!(
            &entries[1],
            EditingKitMenuEntry::Custom(profile) if profile.id == "two"
        ));
        assert!(matches!(
            entries[2],
            EditingKitMenuEntry::BuiltIn(shortcut) if shortcut.game == GameId::Halo2
        ));
        assert_eq!(entries.len(), 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn editing_kit_menu_games_have_distinct_embedded_primary_icons() {
        let mut icons: HashSet<&'static [u8]> = HashSet::new();

        for shortcut in editing_kit_menu_shortcuts() {
            let bytes = get_game_banner_bytes(Some(shortcut.game));
            let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                .unwrap_or_else(|error| panic!("{} icon is not a valid PNG: {error}", shortcut.game));
            assert_eq!(
                image.width(),
                image.height(),
                "{} icon is not square",
                shortcut.game
            );
            assert!(
                image.width() >= 200,
                "{} icon is too small for DPI-aware downsampling",
                shortcut.game
            );
            assert!(
                icons.insert(bytes),
                "{} reuses another editing kit's primary icon",
                shortcut.game
            );
        }

        assert_eq!(icons.len(), EDITING_KIT_SHORTCUTS.len());
    }

    #[test]
    fn editing_kit_menu_rows_keep_icons_aligned_and_separators_outside_click_targets() {
        for scale in [MIN_UI_SCALE, MAX_UI_SCALE] {
            let ctx = egui::Context::default();
            ctx.set_zoom_factor(scale);
            let mut first_row = egui::Rect::NOTHING;
            let mut separator = egui::Rect::NOTHING;
            let mut second_row = egui::Rect::NOTHING;

            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(360.0, 160.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.set_min_width(EDITING_KIT_MENU_MIN_WIDTH);
                        first_row =
                            editing_kit_menu_row_with_read_only(ui, "Halo 4", "H4", None, false, true, false)
                                .rect;
                        separator = ui.separator().rect;
                        second_row = editing_kit_menu_row_with_read_only(
                            ui,
                            "Halo 2 Anniversary Multiplayer",
                            "H2A",
                            None,
                            false,
                            true,
                            false,
                        )
                        .rect;
                    });
                },
            );

            let first_layout = editing_kit_menu_row_layout(first_row);
            let second_layout = editing_kit_menu_row_layout(second_row);
            assert_eq!(first_layout.icon_rect.min.x, second_layout.icon_rect.min.x);
            assert_eq!(first_layout.icon_rect.max.x, second_layout.icon_rect.max.x);
            assert_eq!(
                first_layout.icon_rect.size(),
                Vec2::splat(EDITING_KIT_MENU_ICON_SIZE)
            );
            assert!(first_row.contains(first_layout.icon_rect.min));
            assert!(first_row.contains(first_layout.icon_rect.max));
            assert!(
                first_layout.icon_rect.right() + EDITING_KIT_MENU_ICON_GAP
                    <= first_layout.label_rect.left()
            );
            assert!(first_row.max.y <= separator.min.y);
            assert!(separator.max.y <= second_row.min.y);

            let pixels_per_point = ctx.pixels_per_point();
            assert!(
                (first_layout.icon_rect.width() * pixels_per_point
                    - EDITING_KIT_MENU_ICON_SIZE * pixels_per_point)
                    .abs()
                    < f32::EPSILON
            );
        }
    }

    #[test]
    fn terminal_line_visuals_are_distinct_by_severity() {
        let normal = terminal_line_color(TerminalLineSeverity::Normal);
        assert_eq!(terminal_line_color(TerminalLineSeverity::Summary), normal);
        assert_ne!(terminal_line_color(TerminalLineSeverity::Warning), normal);
        assert_ne!(terminal_line_color(TerminalLineSeverity::Error), normal);
        assert_ne!(terminal_line_color(TerminalLineSeverity::Success), normal);
        assert!(terminal_line_is_strong(TerminalLineSeverity::Summary));
        assert!(terminal_line_is_strong(TerminalLineSeverity::Error));
        assert!(!terminal_line_is_strong(TerminalLineSeverity::Warning));
        assert!(!terminal_line_is_strong(TerminalLineSeverity::Success));
        assert!(!terminal_line_is_strong(TerminalLineSeverity::Normal));
    }

    #[test]
    fn monitor_commands_are_game_specific() {
        assert_eq!(
            monitor_commands_for_game(Some(GameId::Halo2)),
            &[
                "monitor-bitmaps",
                "monitor-bitmaps-data-and-tags",
                "monitor-models",
                "monitor-structures",
            ]
        );
        assert_eq!(
            monitor_commands_for_game(Some(GameId::Halo4)),
            &["monitor-bitmaps", "monitor-strings"]
        );
        assert!(monitor_commands_for_game(Some(GameId::HaloCe)).is_empty());
        assert!(monitor_commands_for_game(None).is_empty());
    }

    /// A probe the UI asks every frame is answered from memory for a second,
    /// then asked again, so a file appearing outside Baboon still shows up.
    #[test]
    fn a_rechecked_probe_runs_at_most_once_a_second() {
        let ctx = egui::Context::default();
        let probes = std::cell::Cell::new(0);
        let ask = |time: f64| {
            let mut answer = false;
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |_| {
                    answer = super::recheck_cached(&ctx, "probe", || {
                        probes.set(probes.get() + 1);
                        probes.get() > 1
                    });
                },
            );
            answer
        };
        assert!(!ask(10.0));
        assert!(
            !ask(10.5),
            "within the second: the first answer, not asked again"
        );
        assert_eq!(probes.get(), 1);
        assert!(ask(11.5), "after it: asked again, and the new answer used");
        assert_eq!(probes.get(), 2);
    }

    #[test]
    fn browser_search_clear_works_in_sidebar_and_folder_widths() {
    for width in [220.0, 720.0] {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        let mut filter = "brute".to_owned();
        let frame = |filter: &mut String, events| {
            let mut response = None;
            let _ = crate::app::run_ui_test(&ctx, 
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1000.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.scope_builder(
                            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                                ui.cursor().min,
                                Vec2::new(width, 100.0),
                            )),
                            |ui| {
                                response = Some(browser_search_field(ui, filter, "Search tags"));
                            },
                        );
                    });
                },
            );
            response.unwrap()
        };
        let response = frame(&mut filter, Vec::new());
        let pos = egui::pos2(response.rect.right() - 10.0, response.rect.center().y);
        let pointer = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &mut filter,
            vec![egui::Event::PointerMoved(pos), pointer(true)],
        );
        let cleared = frame(&mut filter, vec![pointer(false)]);
        assert!(filter.is_empty());
        assert!(cleared.changed(), "clearing must notify the filter cache");
        assert!(ctx.memory(|memory| memory.focused()).is_some());
        assert!((cleared.rect.width() - response.rect.width()).abs() < 0.1);
    }
    }

    // The welcome screen's UI scale slider, driven with real pointer input.
    //
    // The slider sets the zoom factor of the window it is drawn in. Applying the
    // value while the drag was live rescaled the slider under the pointer, so the
    // handle slid away from the cursor and the scale could not be aimed at all.

    /// One frame with `events` delivered to it, standing in for the wizard: the
    /// slider edits `pending`, and the rule decides when that reaches `live`.
    fn frame(
        ctx: &egui::Context,
        pending: &mut f32,
        live: &mut f32,
        events: Vec<egui::Event>,
        pointer: Option<egui::Pos2>,
    ) -> egui::Rect {
        let mut rect = egui::Rect::NOTHING;
        let mut events = events;
        if let Some(pos) = pointer {
            events.insert(0, egui::Event::PointerMoved(pos));
        }
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(600.0, 200.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let response = ui
                        .add(egui::Slider::new(pending, MIN_UI_SCALE..=MAX_UI_SCALE).show_value(false));
                    rect = response.rect;
                    if crate::app::shell::first_run::commit_ui_scale_now(&response, *pending, *live) {
                        *live = *pending;
                    }
                });
            },
        );
        rect
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// The reported interaction: grab the handle, drag it, let go. The window must
    /// not be rescaled until the release, however many frames the drag spans.
    #[test]
    fn the_scale_is_applied_when_the_drag_ends_not_while_it_lasts() {
        let ctx = egui::Context::default();
        let (mut pending, mut live) = (DEFAULT_UI_SCALE, DEFAULT_UI_SCALE);

        // Lay out once to learn where the slider is.
        let rect = frame(&ctx, &mut pending, &mut live, Vec::new(), None);
        assert!(rect.width() > 20.0, "the slider was laid out");

        let start = egui::pos2(rect.center().x, rect.center().y);
        frame(
            &ctx,
            &mut pending,
            &mut live,
            vec![button(start, true)],
            Some(start),
        );

        // Drag in steps, as a pointer does. Every frame that moves the value is a
        // frame that must not rescale the window.
        let mut moved = false;
        for step in 1..=4 {
            let at = egui::pos2(start.x - step as f32 * 12.0, start.y);
            let before = pending;
            frame(&ctx, &mut pending, &mut live, Vec::new(), Some(at));
            moved |= pending != before;
            assert_eq!(
                live, DEFAULT_UI_SCALE,
                "frame {step} of the drag rescaled the window mid-drag (slider {before} -> {pending})"
            );
        }
        assert!(
            moved,
            "the drag actually moved the slider — otherwise this proves nothing"
        );

        let end = egui::pos2(start.x - 48.0, start.y);
        frame(
            &ctx,
            &mut pending,
            &mut live,
            vec![button(end, false)],
            Some(end),
        );
        assert_eq!(
            live, pending,
            "releasing applies exactly the scale the user aimed at"
        );
        assert_ne!(live, DEFAULT_UI_SCALE, "and that is not where it started");
    }

    /// A click that never crosses the drag threshold still has to arrive. Waiting for
    /// a drag-stopped event that never comes would leave the slider showing a scale
    /// the window never took.
    /// Press and release are separate frames: a click lasts tens of milliseconds and
    /// the app redraws throughout. (egui moves a slider on the frames *after* the
    /// press, so a press and release collapsed into one frame moves nothing at all —
    /// that is the synthetic input being unrealistic, not the widget.)
    #[test]
    fn a_click_on_the_track_is_not_swallowed() {
        let ctx = egui::Context::default();
        let (mut pending, mut live) = (DEFAULT_UI_SCALE, DEFAULT_UI_SCALE);
        let rect = frame(&ctx, &mut pending, &mut live, Vec::new(), None);
        let at = egui::pos2(rect.left() + rect.width() * 0.85, rect.center().y);

        frame(
            &ctx,
            &mut pending,
            &mut live,
            vec![button(at, true)],
            Some(at),
        );
        frame(
            &ctx,
            &mut pending,
            &mut live,
            vec![button(at, false)],
            Some(at),
        );

        assert_ne!(pending, DEFAULT_UI_SCALE, "the click moved the slider");
        assert_eq!(live, pending, "and the window followed it");
    }

    // Reading the shipped H3 shader option tags.
    //
    // `source extern` resolves by the option name embedded in the tag, and the
    // engine panics on a name it has no variant for — deliberately, so a decode gap
    // surfaces. That made three of the H3 kit's own option tags unreadable, and in a
    // GUI an unreadable option tag is a dead process rather than a message.

    static H3_SHADERS: std::sync::LazyLock<&'static str> =
        std::sync::LazyLock::new(|| crate::core::test_kits::tag_path("halo3_mcc", "shaders"));

    /// Every `render_method_option` in the kit must decode through the typed reader
    /// the shader grid uses. Named tags are called out because they are the ones that
    /// used to panic, and because a regression here is silent until someone opens a
    /// shader that happens to use them.
    #[test]
    fn every_shipped_h3_shader_option_decodes() {
        let root = std::path::Path::new(*H3_SHADERS);
        if !root.exists() {
            eprintln!("skipping: no H3 editing kit");
            return;
        }
        const PREVIOUSLY_PANICKING: [&str; 3] = [
            "albedo_two_change_color_anim",
            "albedo_two_change_color_chameleon",
            "illum_detail_world_space_four_cc",
        ];

        let mut decoded = 0usize;
        let mut failed: Vec<String> = Vec::new();
        let mut covered: Vec<String> = Vec::new();
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("render_method_option") {
                continue;
            }
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let Ok(tag) = blam_tags::TagFile::read_from_bytes(&bytes) else {
                continue;
            };
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            // Caught rather than propagated: a panic here is exactly the defect, and
            // one failing tag should report itself instead of ending the test.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                blam_tags::render_method::RenderMethodOption::from_tag(&tag).is_ok()
            }));
            match outcome {
                Ok(true) => decoded += 1,
                Ok(false) => failed.push(format!("{stem} (error)")),
                Err(_) => failed.push(format!("{stem} (panic)")),
            }
            if PREVIOUSLY_PANICKING.contains(&stem.as_str()) {
                covered.push(stem);
            }
        }

        assert!(
            decoded > 100,
            "only {decoded} option tag(s) decoded — kit incomplete?"
        );
        assert!(
            failed.is_empty(),
            "{} option tag(s) failed: {failed:?}",
            failed.len()
        );
        assert_eq!(
            covered.len(),
            PREVIOUSLY_PANICKING.len(),
            "the tags this test exists for are missing from the kit: found {covered:?}"
        );
    }

    // Frame-time baselines: whole application frames, run headless.
    //
    // Every scenario drives [`Baboon::run_frame`] — the body of
    // `eframe::App::logic` then `App::ui` — on an egui context configured by
    // [`Baboon::configure_context`], exactly as the window does, then
    // tessellates the output as eframe would before handing it to the GPU. What
    // is timed is therefore the CPU side of a real frame: input, every panel,
    // the worker drain, the prefs throttle, autosave checks, and tessellation.
    // The GPU upload and draw are not (there is no GPU here).
    //
    // All data is synthetic: tags are built from this repository's own
    // `definitions/` schemas, never read from a kit, so this runs anywhere.
    //
    // Run (release is the number that matters; debug only proves it works):
    //
    // ```text
    // cargo test --release perf_baseline -- --ignored --nocapture --test-threads=1
    // ```
    //
    // Comparing against a baseline: run once on the commit to compare against
    // and once on the change, on the same machine, quiet, with the same
    // `BABOON_PERF_PPP`, appending to one CSV under two labels:
    //
    // ```text
    // BABOON_PERF_LABEL=before BABOON_PERF_CSV=perf.csv cargo test --release perf_baseline -- --ignored --nocapture --test-threads=1
    // BABOON_PERF_LABEL=after  BABOON_PERF_CSV=perf.csv cargo test --release perf_baseline -- --ignored --nocapture --test-threads=1
    // ```
    //
    // then compare rows by `scenario`: `median_ms` and `p95_ms` for time, and
    // the counter columns (rows laid out, labels built, ...) for work, which
    // do not depend on the machine and should match exactly unless the change
    // meant to alter them. A run on a loaded machine is noise; differences of
    // a few percent in time are noise anyway. A failed scenario check (the
    // state did not show what the scenario claims) fails the test; the
    // numbers of the scenarios that passed are still printed.
    //
    // Environment:
    // - `BABOON_PERF_WARMUP`  warm-up frames per scenario (default 20)
    // - `BABOON_PERF_FRAMES`  measured frames per scenario (default 120)
    // - `BABOON_PERF_ONLY`    comma-separated substrings; run only scenarios
    //   whose name contains one of them
    // - `BABOON_PERF_PPP`     pixels per point (default 1.0; 2.0 for Retina)
    // - `BABOON_PERF_CSV`     append one CSV row per scenario to this file
    // - `BABOON_PERF_LABEL`   the CSV's label column, e.g. `before` / `after`
    //
    // Surviving a restructure: scenarios only describe *what is on screen*
    // through the [`fixture`] module below, which is the one place that knows
    // how app state is laid out (kits, documents, the terminal, caches). When
    // that layout changes, only `fixture` needs porting; scenario definitions,
    // measurement and reporting stay as they are, so before/after numbers stay
    // comparable. The layout counters are `#[cfg(test)]` thread-locals the app
    // already keeps; [`Counters`] is the one place that reads them.

    const SCREEN: egui::Vec2 = egui::vec2(1600.0, 1000.0);
    /// Inside the kit's browser side panel (330 points wide by default).
    const BROWSER_POINT: egui::Pos2 = egui::pos2(160.0, 600.0);
    /// Inside the tag pane, right of the browser.
    const PANE_POINT: egui::Pos2 = egui::pos2(1000.0, 600.0);
    /// One wheel notch, in points.
    const WHEEL_STEP: f32 = 120.0;

    // ---------------------------------------------------------------------------
    // Counters
    // ---------------------------------------------------------------------------

    /// Layout work the app reports through its `#[cfg(test)]` counters. Each is
    /// reset before a frame and read after it.
    #[derive(Clone, Copy, Default)]
    struct Counters {
        /// Browser rows (tags and folder headers) laid out.
        tree_rows: usize,
        /// Read-only function previews built.
        function_previews: usize,
        /// Block-element dropdown labels built.
        dropdown_labels: usize,
        /// Terminal output lines laid out.
        terminal_lines: usize,
        /// Shader editor models built (should be 0 once warm: it is memoized).
        shader_models: usize,
    }

    impl Counters {
        fn reset() {
            crate::app::browser::TREE_ROWS_LAID_OUT.with(|c| c.set(0));
            crate::app::editor::fields::FUNCTION_PREVIEWS_BUILT.with(|c| c.set(0));
            crate::app::editor::fields::DROPDOWN_LABELS_BUILT.with(|c| c.set(0));
            crate::app::shell::workspace::tests::LINES_BUILT.with(|c| c.set(0));
            crate::app::editor::material::SHADER_MODELS_BUILT.with(|c| c.set(0));
        }

        fn read() -> Self {
            Self {
                tree_rows: crate::app::browser::TREE_ROWS_LAID_OUT.with(std::cell::Cell::get),
                function_previews: crate::app::editor::fields::FUNCTION_PREVIEWS_BUILT
                    .with(std::cell::Cell::get),
                dropdown_labels: crate::app::editor::fields::DROPDOWN_LABELS_BUILT
                    .with(std::cell::Cell::get),
                terminal_lines: crate::app::shell::workspace::tests::LINES_BUILT
                    .with(std::cell::Cell::get),
                shader_models: crate::app::editor::material::SHADER_MODELS_BUILT.with(std::cell::Cell::get),
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Harness: one app on one headless context, driven frame by frame
    // ---------------------------------------------------------------------------

    pub(super) struct FrameSample {
        /// `ctx.run` around the app's frame.
        run: Duration,
        /// `ctx.tessellate` of that frame's shapes.
        tessellate: Duration,
        counters: Counters,
    }

    impl FrameSample {
        fn total(&self) -> Duration {
            self.run + self.tessellate
        }
    }

    pub(super) struct Harness {
        pub(super) ctx: egui::Context,
        pub(super) app: Baboon,
        /// Seconds; advanced 1/60 s a frame so animations, tooltips and the
        /// app's own throttles see time pass as they would at 60 Hz.
        time: f64,
        pixels_per_point: f32,
        /// Every text painted by the last frame, for the scenario checks.
        pub(super) painted: Vec<String>,
        /// The same texts with where they were painted, for clicking them.
        pub(super) painted_rects: Vec<(String, egui::Rect)>,
        /// What the last frame asked the platform to do.
        pub(super) commands: Vec<egui::OutputCommand>,
        /// How long the last frame asked egui to wait before the next one;
        /// `Duration::MAX` when it asked for none.
        pub(super) repaint_delay: Duration,
    }

    impl Harness {
        pub(super) fn new() -> Self {
            let ctx = egui::Context::default();
            Baboon::configure_context(&ctx);
            let names = TagNameIndex::load_from_definitions(&locate_definitions_root());
            let mut prefs = GuiPrefs::default();
            // A wheel over a dropdown would otherwise cycle it, editing the tag
            // under a scroll scenario.
            prefs.scroll_to_cycle_dropdowns = false;
            let app = Baboon::assemble(
                &ctx,
                crate::app::shell::window_state::WindowStateTracker::for_test(),
                prefs,
                HashSet::new(),
                None,
                names,
                None,
            );
            let pixels_per_point = std::env::var("BABOON_PERF_PPP")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1.0);
            Self {
                ctx,
                app,
                time: 0.0,
                pixels_per_point,
                painted: Vec::new(),
                painted_rects: Vec::new(),
                commands: Vec::new(),
                repaint_delay: Duration::MAX,
            }
        }

        /// Run one whole application frame with `events`, timed.
        pub(super) fn frame(&mut self, events: Vec<egui::Event>) -> FrameSample {
            self.time += 1.0 / 60.0;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            };
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(self.pixels_per_point);
            Counters::reset();
            let app = &mut self.app;
            let started = Instant::now();
            let output = crate::app::run_ui_test(&self.ctx, input, |ui| app.run_frame(ui));
            let run = started.elapsed();
            let counters = Counters::read();
            let started = Instant::now();
            let shapes = output.shapes.clone();
            let primitives = self.ctx.tessellate(shapes, output.pixels_per_point);
            let tessellate = started.elapsed();
            std::hint::black_box(primitives);
            // Outside the timed spans.
            self.painted_rects = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    )),
                    _ => None,
                })
                .collect();
            self.painted = self.painted_rects.iter().map(|(text, _)| text.clone()).collect();
            self.commands = output.platform_output.commands;
            self.repaint_delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            FrameSample {
                run,
                tessellate,
                counters,
            }
        }

        fn idle(&mut self, frames: usize) {
            for _ in 0..frames {
                self.frame(Vec::new());
            }
        }

        fn painted_contains(&self, needle: &str) -> bool {
            self.painted.iter().any(|text| text.contains(needle))
        }

        /// Slide onto the `nth` painting of exactly `text` over a few frames,
        /// press and release, and return everything those frames asked the
        /// platform to do.
        pub(super) fn click(&mut self, text: &str, nth: usize) -> Vec<egui::OutputCommand> {
            let rect = self
                .painted_rects
                .iter()
                .filter(|(painted, _)| painted == text)
                .nth(nth)
                .map(|(_, rect)| *rect)
                .unwrap_or_else(|| panic!("{text:?} is not painted: {:?}", self.painted));
            let target = rect.center();
            let from = target - egui::vec2(30.0, 30.0);
            let button = |pressed| egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let mut steps: Vec<Vec<egui::Event>> = (1..=3)
                .map(|step| vec![pointer_at(from + (target - from) * step as f32 / 3.0)])
                .collect();
            steps.extend([vec![button(true)], vec![button(false)], Vec::new()]);
            let mut commands = Vec::new();
            for events in steps {
                self.frame(events);
                commands.extend(self.commands.iter().cloned());
            }
            commands
        }
    }

    fn pointer_at(point: egui::Pos2) -> egui::Event {
        egui::Event::PointerMoved(point)
    }

    fn wheel(dy: f32) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, dy),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        }
    }

    /// A wheel that scrolls down for 40 frames, then up for 40, so a long
    /// measurement neither runs out of content nor sits at one end.
    fn ping_pong_wheel(frame: usize) -> f32 {
        if (frame / 40) % 2 == 0 {
            -WHEEL_STEP
        } else {
            WHEEL_STEP
        }
    }

    // ---------------------------------------------------------------------------
    // Fixture: the only code that knows how app state is laid out
    // ---------------------------------------------------------------------------

    pub(super) mod fixture {
        use super::*;
        use crate::core::source::{LoadedSourceData, TagEntry, TagEntryLocation, TagSource};
        use blam_tags::render_method::{
            BitmapAddressMode, BitmapComparisonFunction, BitmapFilterMode, RenderMethod,
            RenderMethodDefinition, RenderMethodDefinitionCategory,
            RenderMethodDefinitionCategoryOption, RenderMethodOption, RenderMethodOptionParameter,
            RenderMethodParameterType,
        };
        use blam_tags::{Enum, TagFieldData, TagReferenceData, TagStructMut};

        pub(in crate::app) const GAME: &str = "halo3_mcc";

        /// `folders` × `subfolders` × `tags` loose-file entries, as
        /// `folder_NN/sub_NN/tag_NNN.biped`. The defaults (40 × 10 × 150) are
        /// the 60,000 tags the browser virtualization tests use.
        pub(in crate::app) fn synthetic_entries(
            folders: usize,
            subfolders: usize,
            tags: usize,
        ) -> Vec<TagEntry> {
            let mut entries = Vec::with_capacity(folders * subfolders * tags);
            for top in 0..folders {
                for sub in 0..subfolders {
                    for tag in 0..tags {
                        let path = format!("folder_{top:02}/sub_{sub:02}/tag_{tag:03}.biped");
                        entries.push(TagEntry {
                            key: format!("file:{path}"),
                            display_path: path.clone(),
                            group_tag: u32::from_be_bytes(*b"bipd"),
                            group_name: Some("biped".to_owned()),
                            location: TagEntryLocation::LooseFile(path.into()),
                        });
                    }
                }
            }
            entries
        }

        pub(in crate::app) fn entry_key(display_path: &str) -> String {
            format!("file:{display_path}")
        }

        /// The browser entry for a document built in memory.
        pub(in crate::app) fn document_entry(display_path: &str, tag: &TagFile) -> TagEntry {
            TagEntry {
                key: entry_key(display_path),
                display_path: display_path.to_owned(),
                group_tag: tag.header.group_tag,
                group_name: display_path
                    .rsplit_once('.')
                    .map(|(_, extension)| extension.to_owned()),
                location: TagEntryLocation::LooseFile(display_path.into()),
            }
        }

        /// Install `entries` as the active kit's source: an in-memory source,
        /// so the browser draws the full (non-lazy) tree and nothing is read
        /// off disk. This is the tree container and monolithic sources draw,
        /// which are the ones that reach tens of thousands of tags.
        pub(in crate::app) fn install_kit(app: &mut Baboon, entries: Vec<TagEntry>) {
            install_kit_for_game(app, entries, GAME);
        }

        pub(in crate::app) fn install_kit_for_game(
            app: &mut Baboon,
            entries: Vec<TagEntry>,
            game: &str,
        ) {
            app.install_loaded_source(LoadedSourceData {
                label: "perf".to_owned(),
                source: TagSource::SingleFile {
                    path: PathBuf::from("perf-synthetic"),
                },
                names: app.model.default_names.clone(),
                game: GameId::from_id(game),
                tree: crate::core::source::build_tree(&entries),
                group_tree: crate::core::source::build_group_tree(&entries),
                all_entries: entries.clone(),
                entries,
                reverse_dependencies: None,
                initial_tag: None,
                key_hints: Default::default(),
                complete_scan: true,
                chosen_kit_layout: None,
            });
            let kit = &mut app.model.kits[app.model.active];
            let view = &mut app.views[kit.id];
            view.browser.mode = BrowserMode::Folders;
        }

        /// Open `tag` in a tab, as if it had just finished loading. Its entry
        /// must already be in the kit (see [`document_entry`]).
        pub(in crate::app) fn open_document(
            app: &mut Baboon,
            display_path: &str,
            tag: TagFile,
        ) -> String {
            let key = entry_key(display_path);
            let mut kit = app.kit_and_view(app.model.active);
            kit.kit.parsed_tags.insert(key.clone(), TagDocument::clean(tag));
            kit.open_tag_pane(&key);
            key
        }

        /// The tag pane's "Expand all" for `key`, applied on its next draw.
        pub(in crate::app) fn expand_all(app: &mut Baboon, key: &str) {
            app.views[app.model.kits[app.model.active].id]
                .pending_expand
                .insert(key.to_owned(), true);
        }

        /// The browser search box's contents, as if typed.
        pub(in crate::app) fn set_filter(app: &mut Baboon, text: &str) {
            app.views[app.model.kits[app.model.active].id].browser.filter = text.to_owned();
        }

        /// "Reveal in browser": opens the tag's folders and scrolls to it.
        pub(in crate::app) fn reveal(app: &mut Baboon, key: &str) {
            app.reveal_in_browser(key);
        }

        pub(in crate::app) fn open_terminal(
            app: &mut Baboon,
            lines: impl IntoIterator<Item = String>,
        ) {
            app.views[app.model.kits[app.model.active].id].terminal.open = true;
            app.kit_tools.terminal.lines = lines.into_iter().map(TerminalLineEntry::new).collect();
            app.kit_tools.terminal.scroll_to_bottom = true;
        }

        /// One line of tool output arriving, with the app's own cap and
        /// autoscroll (see `push_terminal_line`).
        pub(in crate::app) fn push_terminal_line(app: &mut Baboon, line: String) {
            app.kit_tools.terminal.lines.push(TerminalLineEntry::new(line));
            if app.kit_tools.terminal.lines.len() > 20_000 {
                let remove = app.kit_tools.terminal.lines.len() - 18_000;
                app.kit_tools.terminal.lines.drain(..remove);
            }
            app.kit_tools.terminal.scroll_to_bottom = true;
        }

        pub(in crate::app) fn last_terminal_line(app: &Baboon) -> Option<String> {
            app.kit_tools.terminal.lines.last().map(|line| line.text.clone())
        }

        pub(in crate::app) fn terminal_line(index: usize) -> String {
            format!(
                "{index}: tool.exe: importing C:\\Halo\\tags\\objects\\weapons\\rifle_{index}\\\
             render\\rifle_{index}.render_model from data\\objects\\weapons ... done"
            )
        }

        /// A new tag of `group` from this repository's definitions.
        pub(in crate::app) fn new_tag(group: &str) -> TagFile {
            new_tag_for(GAME, group)
        }

        pub(in crate::app) fn new_tag_for(game: &str, group: &str) -> TagFile {
            TagFile::new(
                locate_definitions_root()
                    .join(game)
                    .join(format!("{group}.json")),
            )
            .unwrap_or_else(|error| panic!("{game}/{group}.json: {error:?}"))
        }

        /// A Halo CE sound whose one pitch range holds `permutations`
        /// permutations of `seconds` of inline 16-bit PCM (the schema's default
        /// compression, big-endian; mono; 22 kHz): a sine sweep, so the
        /// waveform has shape. Inline samples are what CE plays from, so the
        /// player and its waveform work with no sound bank or audio files.
        pub(in crate::app) fn synthetic_ce_sound(permutations: usize, seconds: f32) -> TagFile {
            let mut tag = new_tag_for("haloce_mcc", "sound");
            let frames = (22_050.0 * seconds) as usize;
            let mut root = tag.root_mut();
            let mut field = root.field_path_mut("pitch ranges").expect("pitch ranges");
            let mut ranges = field.as_block_mut().expect("pitch ranges is a block");
            ranges.add_element();
            let mut range = ranges.element_mut(0).expect("the pitch range");
            let mut field = range.field_path_mut("permutations").expect("permutations");
            let mut block = field.as_block_mut().expect("permutations is a block");
            for index in 0..permutations {
                block.add_element();
                let mut permutation = block.element_mut(index).expect("the permutation");
                permutation
                    .field_path_mut("name")
                    .expect("permutation name")
                    .set(TagFieldData::String(format!("perm_{index}")))
                    .expect("set the permutation name");
                let mut samples = Vec::with_capacity(frames * 2);
                for frame in 0..frames {
                    let t = frame as f32 / 22_050.0;
                    let envelope = 0.5 + 0.5 * (t * 0.7 + index as f32).sin();
                    let value = (t * (220.0 + 40.0 * index as f32) * std::f32::consts::TAU).sin()
                        * envelope
                        * 20_000.0;
                    samples.extend_from_slice(&(value as i16).to_be_bytes());
                }
                permutation
                    .field_path_mut("samples")
                    .expect("permutation samples")
                    .set(TagFieldData::Data(samples))
                    .expect("set the samples");
            }
            tag
        }

        /// Give every block in `tag_struct` `counts[0]` elements, and every
        /// block in each block's first element `counts[1]`, and so on down.
        pub(in crate::app) fn populate_blocks(
            tag_struct: &mut TagStructMut<'_>,
            counts: &[usize],
        ) -> usize {
            let Some((&count, deeper)) = counts.split_first() else {
                return 0;
            };
            let ordinals: Vec<usize> = tag_struct
                .as_ref()
                .fields()
                .filter(|field| field.as_block().is_some())
                .map(|field| field.ordinal())
                .collect();
            let mut added = 0;
            for ordinal in ordinals {
                let Some(mut field) = tag_struct.field_at_mut(ordinal) else {
                    continue;
                };
                let Some(mut block) = field.as_block_mut() else {
                    continue;
                };
                let max = block.definition().max_count().max(1) as usize;
                for _ in 0..count.min(max) {
                    block.add_element();
                    added += 1;
                }
                if let Some(mut first) = block.element_mut(0) {
                    added += populate_blocks(&mut first, deeper);
                }
            }
            added
        }

        /// A scenario whose every top-level block holds `counts[0]` elements and
        /// so on down (see [`populate_blocks`]). Returns it with its element
        /// count.
        pub(in crate::app) fn large_scenario(counts: &[usize]) -> (TagFile, usize) {
            let mut tag = new_tag("scenario");
            let added = populate_blocks(&mut tag.root_mut(), counts);
            (tag, added)
        }

        /// A shader naming a render method definition of `categories`
        /// categories, each option declaring `parameters` parameters. The
        /// definition and options are put straight into the kit's caches — where
        /// the editor finds them once loaded — so nothing is read off disk.
        /// Returns the shader; [`install_render_method`] must run after the kit
        /// is installed.
        pub(in crate::app) fn synthetic_shader(categories: usize) -> TagFile {
            let mut tag = new_tag("shader");
            {
                let mut root = tag.root_mut();
                root.field_path_mut("render_method/definition")
                    .expect("shader has render_method/definition")
                    .set(TagFieldData::TagReference(TagReferenceData {
                        group_tag_and_name: Some((
                            u32::from_be_bytes(*b"rmdf"),
                            "shaders\\perf_shader".to_owned(),
                        )),
                    }))
                    .expect("set the shader's definition");
                let mut field = root
                    .field_path_mut("render_method/options")
                    .expect("shader has render_method/options");
                let mut options = field.as_block_mut().expect("options is a block");
                for _ in 0..categories {
                    options.add_element();
                }
            }
            tag
        }

        pub(in crate::app) fn install_render_method(
            app: &mut Baboon,
            shader: &TagFile,
            categories: usize,
            options_per_category: usize,
            parameters: usize,
        ) {
            let render_method = RenderMethod::from_tag(shader).expect("synthetic shader parses");
            let kit = &mut app.model.kits[app.model.active];
            let view = &mut app.views[kit.id];
            let mut definition_categories = Vec::new();
            for category in 0..categories {
                let mut options = Vec::new();
                for option in 0..options_per_category {
                    let option_path = format!("shaders\\perf_options\\cat{category}_opt{option}");
                    options.push(RenderMethodDefinitionCategoryOption {
                        option_name: format!("option_{option}"),
                        option_path: option_path.clone(),
                        vertex_function: String::new(),
                        pixel_function: String::new(),
                    });
                    let kinds = [
                        RenderMethodParameterType::Bitmap,
                        RenderMethodParameterType::Color,
                        RenderMethodParameterType::Real,
                        RenderMethodParameterType::Int,
                        RenderMethodParameterType::Bool,
                        RenderMethodParameterType::ArgbColor,
                    ];
                    let option_parameters = (0..parameters)
                        .map(|index| RenderMethodOptionParameter {
                            parameter_name: format!("perf_param_{category}_{index}"),
                            parameter_type: Some(Enum::from_variant(kinds[index % kinds.len()])),
                            source_extern: None,
                            default_bitmap_path: String::new(),
                            default_real_value: index as f32,
                            default_int_bool_value: 0,
                            flags: 0,
                            default_filter_mode: Enum::from_variant(BitmapFilterMode::Trilinear),
                            default_comparison_function: Enum::from_variant(
                                BitmapComparisonFunction::Never,
                            ),
                            default_address_mode: Enum::from_variant(BitmapAddressMode::Wrap),
                            default_filter_mode_index: 0,
                            default_address_mode_index: 0,
                            anisotropy_amount: 0,
                            default_color: blam_tags::math::ArgbColor(0xff80_4020),
                            default_bitmap_scale: 1.0,
                            help_text: String::new(),
                        })
                        .collect();
                    view.caches.rmop_cache.insert(
                        format!("rmop:{option_path}"),
                        Some(Arc::new(RenderMethodOption {
                            parameters: option_parameters,
                            filter_mode_names: Vec::new(),
                            address_mode_names: Vec::new(),
                        })),
                    );
                }
                definition_categories.push(RenderMethodDefinitionCategory {
                    category_name: format!("perf_category_{category}"),
                    vertex_function: String::new(),
                    pixel_function: String::new(),
                    options,
                });
            }
            view.caches.rmdf_cache.insert(
                format!("rmdf:{}", render_method.definition_path),
                Some(Arc::new(RenderMethodDefinition {
                    global_options_path: String::new(),
                    categories: definition_categories,
                    shared_pixel_shaders_path: String::new(),
                    shared_vertex_shaders_path: String::new(),
                    flags: 0,
                    version: 0,
                })),
            );
        }
    }

    // ---------------------------------------------------------------------------
    // Scenarios
    // ---------------------------------------------------------------------------

    /// What one scenario puts on screen, and what each measured frame does.
    struct Scenario {
        name: &'static str,
        what: &'static str,
        /// Build the state, and settle it over as many frames as it needs. Not
        /// timed.
        setup: fn(&mut Harness),
        /// The events for measured (and warm-up) frame `index`; may also change
        /// app state, as typing or streaming output would.
        step: fn(&mut Harness, usize) -> Vec<egui::Event>,
        /// Evidence the scenario showed what it claims, from the last frame.
        /// A scenario whose setup silently drew nothing would otherwise time an
        /// empty window and report it as a baseline.
        check: fn(&Harness, &Measured) -> Result<(), String>,
    }

    const MIDDLE_TAG: &str = "folder_20/sub_05/tag_075.biped";

    fn no_events(_: &mut Harness, _: usize) -> Vec<egui::Event> {
        Vec::new()
    }

    fn setup_kit_60k(h: &mut Harness) {
        fixture::install_kit(&mut h.app, fixture::synthetic_entries(40, 10, 150));
        h.idle(3);
    }

    /// Every one of the 400 subfolders opened (by revealing a tag in each, the
    /// way "Reveal in browser" does), then the middle tag revealed: 60,440 rows
    /// expanded, the viewport in the middle of them.
    fn setup_browser_expanded(h: &mut Harness) {
        setup_kit_60k(h);
        for top in 0..40 {
            for sub in 0..10 {
                let key = fixture::entry_key(&format!("folder_{top:02}/sub_{sub:02}/tag_000.biped"));
                fixture::reveal(&mut h.app, &key);
                h.idle(2);
            }
        }
        fixture::reveal(&mut h.app, &fixture::entry_key(MIDDLE_TAG));
        h.idle(4);
    }

    fn setup_large_tag(h: &mut Harness) -> String {
        let (tag, _) = fixture::large_scenario(&[64, 8]);
        let path = "levels/perf/perf.scenario";
        let mut entries = fixture::synthetic_entries(4, 5, 50);
        entries.push(fixture::document_entry(path, &tag));
        fixture::install_kit(&mut h.app, entries);
        let key = fixture::open_document(&mut h.app, path, tag);
        h.idle(2);
        key
    }

    fn setup_large_tag_expanded(h: &mut Harness) {
        let key = setup_large_tag(h);
        fixture::expand_all(&mut h.app, &key);
        h.idle(3);
    }

    /// Typing `tag_075`, deleting back to `t`, then `tag_14`: every frame is a
    /// new query, so every frame re-filters all 60,000 entries.
    const FILTER_KEYSTROKES: [&str; 16] = [
        "t", "ta", "tag", "tag_", "tag_0", "tag_07", "tag_075", "tag_07", "tag_0", "tag_", "tag", "ta",
        "tag", "tag_", "tag_1", "tag_14",
    ];

    fn scenarios() -> Vec<Scenario> {
        vec![
            Scenario {
                name: "idle_welcome",
                what: "no kit loaded; the welcome screen",
                setup: |h| h.idle(3),
                step: no_events,
                check: |h, _| {
                    (!h.painted.is_empty())
                        .then_some(())
                        .ok_or_else(|| "nothing was painted".to_owned())
                },
            },
            Scenario {
                name: "idle_kit_60k",
                what: "60,000-tag kit loaded, folders collapsed, no tabs, no input",
                setup: setup_kit_60k,
                step: no_events,
                check: |h, _| {
                    h.painted_contains("folder_00")
                        .then_some(())
                        .ok_or_else(|| "browser did not show folder_00".to_owned())
                },
            },
            Scenario {
                name: "browser_60k_expanded_static",
                what: "60,440 rows expanded, viewport at the middle, no input",
                setup: setup_browser_expanded,
                step: no_events,
                check: |h, _| {
                    h.painted_contains("tag_075")
                        .then_some(())
                        .ok_or_else(|| format!("{MIDDLE_TAG} not in view after reveal"))
                },
            },
            Scenario {
                name: "browser_60k_wheel_scroll",
                what: "60,440 rows expanded, mouse wheel over the browser every frame",
                setup: setup_browser_expanded,
                step: |_, index| vec![pointer_at(BROWSER_POINT), wheel(ping_pong_wheel(index))],
                check: |_, measured| measured.scrolled(),
            },
            Scenario {
                name: "browser_filter_typing",
                what: "60,000-tag kit, the search query changes every frame",
                setup: setup_kit_60k,
                step: |h, index| {
                    fixture::set_filter(
                        &mut h.app,
                        FILTER_KEYSTROKES[index % FILTER_KEYSTROKES.len()],
                    );
                    Vec::new()
                },
                check: |h, _| {
                    (!h.painted_contains("No matching tags") && h.painted_contains("folder_"))
                        .then_some(())
                        .ok_or_else(|| "the filtered tree showed no matches".to_owned())
                },
            },
            Scenario {
                name: "tag_pane_scenario_static",
                what: "H3 scenario, every block 64 elements (first element's blocks 8), collapsed defaults, no input",
                setup: |h| {
                    setup_large_tag(h);
                },
                step: no_events,
                check: |h, _| {
                    h.painted_contains("perf.scenario")
                        .then_some(())
                        .ok_or_else(|| "the scenario tab did not draw".to_owned())
                },
            },
            Scenario {
                name: "tag_pane_scenario_expanded_static",
                what: "same scenario after Expand All, no input",
                setup: setup_large_tag_expanded,
                step: no_events,
                check: |h, _| {
                    h.painted_contains("perf.scenario")
                        .then_some(())
                        .ok_or_else(|| "the scenario tab did not draw".to_owned())
                },
            },
            Scenario {
                name: "tag_pane_scenario_expanded_scroll",
                what: "same scenario after Expand All, mouse wheel over the pane every frame",
                setup: setup_large_tag_expanded,
                step: |_, index| vec![pointer_at(PANE_POINT), wheel(ping_pong_wheel(index))],
                check: |_, measured| measured.scrolled(),
            },
            Scenario {
                name: "shader_editor",
                what: "H3 shader: 12 categories x 4 options, 12 parameters per option (memoized model)",
                setup: |h| {
                    let shader = fixture::synthetic_shader(12);
                    let path = "shaders/perf.shader";
                    let mut entries = fixture::synthetic_entries(4, 5, 50);
                    entries.push(fixture::document_entry(path, &shader));
                    fixture::install_kit(&mut h.app, entries);
                    fixture::install_render_method(&mut h.app, &shader, 12, 4, 12);
                    fixture::open_document(&mut h.app, path, shader);
                    Counters::reset();
                    h.idle(3);
                },
                step: no_events,
                check: |h, _| {
                    h.painted_contains("PERF_CATEGORY_0")
                        .then_some(())
                        .ok_or_else(|| "the shader grid did not draw (raw-field fallback?)".to_owned())
                },
            },
            Scenario {
                name: "sound_player_ce_inline",
                what: "Halo CE sound, 24 permutations x 10 s inline PCM, player + waveform, no input",
                setup: |h| {
                    let sound = fixture::synthetic_ce_sound(24, 10.0);
                    let path = "sound/perf/perf.sound";
                    let mut entries = fixture::synthetic_entries(4, 5, 50);
                    entries.push(fixture::document_entry(path, &sound));
                    fixture::install_kit_for_game(&mut h.app, entries, "haloce_mcc");
                    fixture::open_document(&mut h.app, path, sound);
                    // The idle player asks the audio worker for its waveform;
                    // give the decode time to land.
                    for _ in 0..20 {
                        h.idle(1);
                        std::thread::sleep(Duration::from_millis(25));
                    }
                },
                step: no_events,
                check: |h, _| {
                    // The clip's length is read from its inline samples: proof the
                    // player found 10 s of PCM, not an empty tag.
                    (h.painted_contains("24 permutations") && h.painted_contains("0:10.000"))
                        .then_some(())
                        .ok_or_else(|| "the sound player did not find the inline samples".to_owned())
                },
            },
            Scenario {
                name: "terminal_20k_static",
                what: "terminal open at the bottom of 20,000 lines (the app's cap), no input",
                setup: |h| {
                    fixture::install_kit(&mut h.app, fixture::synthetic_entries(4, 5, 50));
                    fixture::open_terminal(&mut h.app, (0..20_000).map(fixture::terminal_line));
                    h.idle(4);
                },
                step: no_events,
                check: |h, _| {
                    h.painted_contains("19999: ")
                        .then_some(())
                        .ok_or_else(|| "the last terminal line is not in view".to_owned())
                },
            },
            Scenario {
                name: "terminal_20k_streaming",
                what: "terminal at its 20,000-line cap, one new line and autoscroll every frame",
                setup: |h| {
                    fixture::install_kit(&mut h.app, fixture::synthetic_entries(4, 5, 50));
                    fixture::open_terminal(&mut h.app, (0..20_000).map(fixture::terminal_line));
                    h.idle(4);
                },
                step: |h, index| {
                    fixture::push_terminal_line(&mut h.app, fixture::terminal_line(1_000_000 + index));
                    Vec::new()
                },
                check: |h, _| {
                    // Autoscroll lands a frame after the line arrives, so the
                    // newest line itself may be just below the view; one of the
                    // last few streamed ones must be in it (scroll animation trails a
                    // line-per-frame stream by ~3 lines).
                    let last = fixture::last_terminal_line(&h.app).unwrap_or_default();
                    let newest: usize = last
                        .split(':')
                        .next()
                        .and_then(|number| number.parse().ok())
                        .unwrap_or_default();
                    (newest >= 1_000_000
                        && (newest - 8..=newest).any(|n| h.painted_contains(&format!("{n}: "))))
                    .then_some(())
                    .ok_or_else(|| format!("none of the newest lines (to {newest}) is in view"))
                },
            },
        ]
    }

    // ---------------------------------------------------------------------------
    // Measurement and report
    // ---------------------------------------------------------------------------

    struct Measured {
        samples: Vec<FrameSample>,
        /// Painted text of the first and last measured frames.
        first_painted: Vec<String>,
        last_painted: Vec<String>,
    }

    impl Measured {
        /// For scroll scenarios: the view changed while measuring.
        fn scrolled(&self) -> Result<(), String> {
            (self.first_painted != self.last_painted)
                .then_some(())
                .ok_or_else(|| "the wheel did not move the view".to_owned())
        }

        fn millis(&self, of: impl Fn(&FrameSample) -> Duration) -> Vec<f64> {
            let mut values: Vec<f64> = self
                .samples
                .iter()
                .map(|sample| of(sample).as_secs_f64() * 1000.0)
                .collect();
            values.sort_by(|a, b| a.partial_cmp(b).unwrap());
            values
        }

        fn mean_counter(&self, of: impl Fn(&Counters) -> usize) -> f64 {
            let total: usize = self.samples.iter().map(|sample| of(&sample.counters)).sum();
            total as f64 / self.samples.len().max(1) as f64
        }
    }

    fn mean(values: &[f64]) -> f64 {
        values.iter().sum::<f64>() / values.len().max(1) as f64
    }

    /// Nearest-rank percentile of sorted `values`.
    fn percentile(values: &[f64], p: f64) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let rank = ((p / 100.0) * values.len() as f64).ceil() as usize;
        values[rank.clamp(1, values.len()) - 1]
    }

    fn env_usize(name: &str, default: usize) -> usize {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    }

    fn run_scenario(scenario: &Scenario, warmup: usize, frames: usize) -> (Measured, Duration) {
        let setup_started = Instant::now();
        let mut harness = Harness::new();
        (scenario.setup)(&mut harness);
        // No hover unless a step puts the pointer somewhere: a pointer resting
        // on a field raises its tooltip after a delay, which would time the
        // tooltip rather than the scenario.
        harness.frame(vec![egui::Event::PointerGone]);
        let setup = setup_started.elapsed();
        for index in 0..warmup {
            let events = (scenario.step)(&mut harness, index);
            harness.frame(events);
        }
        let mut samples = Vec::with_capacity(frames);
        let mut first_painted = Vec::new();
        for index in 0..frames {
            let events = (scenario.step)(&mut harness, warmup + index);
            samples.push(harness.frame(events));
            if index == 0 {
                first_painted = harness.painted.clone();
            }
        }
        let measured = Measured {
            samples,
            first_painted,
            last_painted: harness.painted.clone(),
        };
        if std::env::var_os("BABOON_PERF_DUMP").is_some() {
            eprintln!(
                "[perf] {}: last frame painted {} texts: {:?}",
                scenario.name,
                harness.painted.len(),
                harness.painted
            );
        }
        if let Err(problem) = (scenario.check)(&harness, &measured) {
            panic!("{}: {problem}", scenario.name);
        }
        (measured, setup)
    }

    /// Frame-time table for every scenario. `#[ignore]`d: it takes minutes in a
    /// debug build and its numbers mean nothing there or on a busy machine.
    #[test]
    #[ignore]
    fn perf_baseline() {
        let warmup = env_usize("BABOON_PERF_WARMUP", 20);
        let frames = env_usize("BABOON_PERF_FRAMES", 120).max(1);
        let only: Vec<String> = std::env::var("BABOON_PERF_ONLY")
            .map(|value| value.split(',').map(str::trim).map(str::to_owned).collect())
            .unwrap_or_default();
        let label = std::env::var("BABOON_PERF_LABEL").unwrap_or_default();
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let ppp = Harness::new().pixels_per_point;

        let mut rows = Vec::new();
        let mut failures = Vec::new();
        for scenario in scenarios() {
            if !only.is_empty()
                && !only
                    .iter()
                    .any(|part| scenario.name.contains(part.as_str()))
            {
                continue;
            }
            eprintln!("[perf] {} — {}", scenario.name, scenario.what);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_scenario(&scenario, warmup, frames)
            }));
            match result {
                Ok((measured, setup)) => rows.push((scenario.name, measured, setup)),
                Err(panic) => {
                    let message = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "panicked".to_owned());
                    eprintln!("[perf] {} FAILED: {message}", scenario.name);
                    failures.push(format!("{}: {message}", scenario.name));
                }
            }
        }

        eprintln!();
        eprintln!(
            "Baboon frame-time baseline — {profile} build, {warmup} warm-up + {frames} measured frames, \
         {}x{} points @ {ppp} ppp{}",
            SCREEN.x,
            SCREEN.y,
            if label.is_empty() {
                String::new()
            } else {
                format!(", label `{label}`")
            }
        );
        eprintln!(
            "{:<34} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} | {:>8} {:>7} {:>7} {:>7} {:>6} | {:>8}",
            "scenario",
            "frames",
            "mean",
            "median",
            "p95",
            "max",
            "run",
            "tess",
            "treeRows",
            "fnPrev",
            "ddLbl",
            "termLn",
            "shader",
            "setup s"
        );
        let csv_path = std::env::var("BABOON_PERF_CSV").ok();
        let mut csv = String::new();
        for (name, measured, setup) in &rows {
            let total = measured.millis(FrameSample::total);
            let run = measured.millis(|sample| sample.run);
            let tess = measured.millis(|sample| sample.tessellate);
            let line = format!(
                "{:<34} {:>6} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} | {:>8.1} {:>7.1} {:>7.1} {:>7.1} {:>6.2} | {:>8.1}",
                name,
                measured.samples.len(),
                mean(&total),
                percentile(&total, 50.0),
                percentile(&total, 95.0),
                total.last().copied().unwrap_or_default(),
                mean(&run),
                mean(&tess),
                measured.mean_counter(|c| c.tree_rows),
                measured.mean_counter(|c| c.function_previews),
                measured.mean_counter(|c| c.dropdown_labels),
                measured.mean_counter(|c| c.terminal_lines),
                measured.mean_counter(|c| c.shader_models),
                setup.as_secs_f64(),
            );
            eprintln!("{line}");
            csv.push_str(&format!(
                "{label},{profile},{ppp},{name},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.2},{:.2},{:.2},{:.2},{:.3}\n",
                measured.samples.len(),
                mean(&total),
                percentile(&total, 50.0),
                percentile(&total, 95.0),
                total.last().copied().unwrap_or_default(),
                mean(&run),
                mean(&tess),
                measured.mean_counter(|c| c.tree_rows),
                measured.mean_counter(|c| c.function_previews),
                measured.mean_counter(|c| c.dropdown_labels),
                measured.mean_counter(|c| c.terminal_lines),
                measured.mean_counter(|c| c.shader_models),
            ));
        }
        eprintln!(
            "(ms per frame; run = ctx.run around Baboon::run_frame, tess = ctx.tessellate; counters are per-frame means)"
        );
        if let Some(path) = csv_path {
            use std::io::Write;
            let exists = std::path::Path::new(&path).exists();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .expect("open BABOON_PERF_CSV");
            if !exists {
                writeln!(
                    file,
                    "label,profile,ppp,scenario,frames,mean_ms,median_ms,p95_ms,max_ms,run_ms,tess_ms,\
                 tree_rows,function_previews,dropdown_labels,terminal_lines,shader_models"
                )
                .unwrap();
            }
            file.write_all(csv.as_bytes()).unwrap();
            eprintln!("appended to {path}");
        }
        assert!(failures.is_empty(), "scenarios failed: {failures:#?}");
    }

    // Whole-frame smoke test: every window, dialog, prompt and pane the app can
    // show, opened one at a time over a populated app and drawn through
    // [`Baboon::run_frame`] — `eframe::App::logic` then `App::ui` — for several
    // frames.
    //
    // Each case is a `base` state (a kit of some kind, or nothing) and an `open`
    // step that puts one window or pane on top of it. The case runs [`FRAMES`]
    // frames and requires every one of its `expect` strings in the last frame's
    // painted text; a panic anywhere in the frame fails it too. A window paints
    // nothing on its first frame, and one that never drew would leave its title
    // and labels unpainted, so a window that silently stayed shut fails rather
    // than passing on an empty screen.
    //
    // The expectations are themselves checked: each case also runs its `base`
    // alone, and at least one `expect` string must be missing there. An
    // expectation the base already paints (a menu label, a browser row) would
    // pass whether or not the window drew, so it is rejected.
    //
    // The cases are split across [`SHARDS`] tests, which run in parallel; each
    // runs all of its cases and reports every failure together. Run them with
    // `cargo test frame_smoke`. `BABOON_SMOKE_ONLY=name,name` runs only cases
    // whose name contains one of them; `BABOON_SMOKE_DUMP=1` (with
    // `--nocapture`) prints every case's painted text.
    //
    // All data is synthetic: tags are built from this repository's
    // `definitions/` schemas, and the loose kits are temporary folders of those
    // tags. Nothing is read from a real kit or game.
    //
    // Adding a window is one row in [`cases`]. [`every_window_has_a_smoke_case`]
    // is what notices a window without one: it reads the `Baboon` struct out of
    // `src/app/mod.rs` and every `egui::Window::new` site out of `src/app/`, and
    // requires each field shaped like window state and each file that opens a
    // window to be named by some case, or listed in [`NOT_WINDOWS`] with the
    // reason. A new `Option<…Dialog>` field, or a new file with a window in it,
    // fails that test until it has a row here.

    /// Frames each case runs after its setup. A window's first frame only
    /// measures it; the second is the first that paints; the rest let anything
    /// it asks for on its first frames (a second layout pass, a fast worker)
    /// land.
    const FRAMES: usize = 6;

    type Step = fn(&mut Harness);

    /// One smoke case: a base state, one thing opened over it, and what must be
    /// on screen.
    struct Case {
        name: &'static str,
        /// `Baboon` fields this case opens: the registry
        /// [`every_window_has_a_smoke_case`] checks the struct against.
        fields: &'static [&'static str],
        /// Files under `src/app/` whose `egui::Window::new` this case draws.
        sources: &'static [&'static str],
        base: Step,
        open: Step,
        /// Every one must be painted by the last frame, and at least one must
        /// not be painted by `base` alone.
        expect: &'static [&'static str],
    }

    const fn case(
        name: &'static str,
        fields: &'static [&'static str],
        sources: &'static [&'static str],
        base: Step,
        open: Step,
        expect: &'static [&'static str],
    ) -> Case {
        Case {
            name,
            fields,
            sources,
            base,
            open,
            expect,
        }
    }

    // ---------------------------------------------------------------------------
    // Bases
    // ---------------------------------------------------------------------------

    fn welcome(_: &mut Harness) {}

    thread_local! {
        /// Folders the current case's bases made, removed after its frames.
        static TEMP_DIRS: std::cell::RefCell<Vec<PathBuf>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = crate::core::test_kits::unique_temp_dir(name);
        TEMP_DIRS.with(|dirs| dirs.borrow_mut().push(dir.clone()));
        dir
    }

    /// The perf fixture's 1,000-tag in-memory Halo 3 kit.
    fn memory_kit(h: &mut Harness) {
        fixture::install_kit(&mut h.app, fixture::synthetic_entries(4, 5, 50));
    }

    /// A loose Halo 3 editing kit in a fresh temporary folder, holding a few
    /// synthetic tags written from the definitions.
    fn loose_kit(h: &mut Harness) {
        let root = temp_dir("frame-smoke-kit").join("tags");
        for (rel, group) in [
            ("objects/weapons/rifle/rifle.biped", "biped"),
            ("objects/weapons/rifle/rifle.scenery", "scenery"),
            ("levels/smoke/smoke.scenario", "scenario"),
        ] {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            fixture::new_tag(group).write(&path).unwrap();
        }
        let source = crate::core::source::load_editing_kit_layout(
            root,
            "Smoke Kit".to_owned(),
            GameId::from_id(fixture::GAME).unwrap(),
            &h.app.model.default_names,
            &locate_definitions_root(),
        )
        .expect("the synthetic loose kit loads");
        h.app.install_loaded_source(source);
    }

    /// The tags root of the [`loose_kit`] base.
    fn loose_root(h: &Harness) -> PathBuf {
        h.app.model.loaded_tags_root().expect("a loose kit is loaded")
    }

    const CE_TAG: &str = "objects/weapons/rifle/rifle.weapon";

    /// A Campaign Evolved container workspace with no containers on disk: its
    /// entries are in memory, which is all the dialogs over it read.
    fn container_kit(h: &mut Harness) {
        let entries: Vec<TagEntry> = [CE_TAG, "objects/weapons/rifle/rifle.model"]
            .into_iter()
            .map(|path| TagEntry {
                key: format!("ce:{path}"),
                display_path: path.to_owned(),
                group_tag: if path.ends_with(".weapon") {
                    u32::from_be_bytes(*b"weap")
                } else {
                    u32::from_be_bytes(*b"hlmt")
                },
                group_name: path.rsplit_once('.').map(|(_, ext)| ext.to_owned()),
                location: TagEntryLocation::LooseFile(path.into()),
            })
            .collect();
        let root = temp_dir("frame-smoke-paks");
        h.app.install_loaded_source(LoadedSourceData {
            label: "Campaign Evolved".to_owned(),
            source: TagSource::IoStoreContainerSet {
                root,
                containers: Vec::new(),
                index: Default::default(),
                packages: Default::default(),
                shipped: Default::default(),
            },
            names: h.app.model.default_names.clone(),
            game: Some(GameId::CampaignEvolved),
            tree: crate::core::source::build_tree(&entries),
            group_tree: crate::core::source::build_group_tree(&entries),
            all_entries: entries.clone(),
            entries,
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: true,
            chosen_kit_layout: None,
        });
    }

    fn ce_key() -> String {
        format!("ce:{CE_TAG}")
    }

    fn active_id(h: &Harness) -> KitId {
        h.app.model.active_kit_id()
    }

    fn biped_key() -> String {
        fixture::entry_key("folder_00/sub_00/tag_000.biped")
    }

    // --- tags for the pane cases: each base lists the tag, each open opens it ---

    const SCENARIO: &str = "levels/smoke/smoke.scenario";

    fn scenario_tag() -> TagFile {
        fixture::large_scenario(&[2]).0
    }

    /// An in-memory kit of `game` whose browser lists `path`.
    fn pane_kit(h: &mut Harness, game: &str, path: &str, tag: &TagFile) {
        let mut entries = fixture::synthetic_entries(2, 2, 10);
        entries.push(fixture::document_entry(path, tag));
        fixture::install_kit_for_game(&mut h.app, entries, game);
    }

    fn scenario_kit(h: &mut Harness) {
        pane_kit(h, fixture::GAME, SCENARIO, &scenario_tag());
    }

    fn open_scenario(h: &mut Harness) -> String {
        fixture::open_document(&mut h.app, SCENARIO, scenario_tag())
    }

    fn shader_kit(h: &mut Harness) {
        let shader = fixture::synthetic_shader(3);
        pane_kit(h, fixture::GAME, "shaders/smoke.shader", &shader);
        fixture::install_render_method(&mut h.app, &shader, 3, 2, 4);
    }

    fn material_tag() -> TagFile {
        fixture::new_tag_for("halo4_mcc", "material")
    }

    fn sound_tag() -> TagFile {
        fixture::synthetic_ce_sound(2, 1.0)
    }

    fn render_model_tag() -> TagFile {
        fixture::new_tag("render_model")
    }

    /// A Halo 3 light, whose function fields sit near the top of the tag.
    fn light_tag() -> TagFile {
        fixture::new_tag("light")
    }

    // ---------------------------------------------------------------------------
    // Cases
    // ---------------------------------------------------------------------------

    fn cases() -> Vec<Case> {
        let mut cases = vec![
            // --- the shell ---
            case(
                "kit_browser",
                &[],
                &[],
                welcome,
                memory_kit,
                &["folder_00", "folder_03", "search tags"],
            ),
            case(
                "terminal",
                &["kit_tools.terminal"],
                &[],
                memory_kit,
                |h| fixture::open_terminal(&mut h.app, (0..50).map(fixture::terminal_line)),
                &["Open full log", ": tool.exe: importing"],
            ),
            // --- tag panes, one per editor kind ---
            case(
                "pane_generic_fields",
                &[],
                &[],
                scenario_kit,
                |h| {
                    open_scenario(h);
                },
                &["smoke.scenario", "Skies"],
            ),
            case(
                "pane_shader",
                &[],
                &[],
                shader_kit,
                |h| {
                    fixture::open_document(
                        &mut h.app,
                        "shaders/smoke.shader",
                        fixture::synthetic_shader(3),
                    );
                },
                &["smoke.shader", "PERF_CATEGORY_0"],
            ),
            case(
                "pane_material",
                &[],
                &[],
                |h| pane_kit(h, "halo4_mcc", "shaders/smoke.material", &material_tag()),
                |h| {
                    fixture::open_document(&mut h.app, "shaders/smoke.material", material_tag());
                },
                &["smoke.material"],
            ),
            case(
                "pane_sound",
                &["audio"],
                &[],
                |h| pane_kit(h, "haloce_mcc", "sound/smoke.sound", &sound_tag()),
                |h| {
                    fixture::open_document(&mut h.app, "sound/smoke.sound", sound_tag());
                },
                &["smoke.sound", "2 permutations"],
            ),
            case(
                "pane_bitmap",
                &[],
                &[],
                |h| {
                    let bitmap = fixture::new_tag("bitmap");
                    pane_kit(h, fixture::GAME, "bitmaps/smoke.bitmap", &bitmap);
                },
                |h| {
                    let bitmap = fixture::new_tag("bitmap");
                    fixture::open_document(&mut h.app, "bitmaps/smoke.bitmap", bitmap);
                },
                &["smoke.bitmap"],
            ),
            case(
                "pane_model_preview",
                &[],
                &[],
                |h| {
                    let model = render_model_tag();
                    pane_kit(h, fixture::GAME, "objects/smoke.render_model", &model);
                },
                |h| {
                    let model = render_model_tag();
                    let key = fixture::open_document(&mut h.app, "objects/smoke.render_model", model);
                    let kit = &mut h.app.model.kits[h.app.model.active];
                    let view = &mut h.app.views[kit.id];
                    view.caches.model_previews.entry(key).or_default().active_tab =
                        ModelTagPanelTab::ModelPreview;
                },
                &["smoke.render_model", "Model Preview"],
            ),
            case(
                "pane_function_rows",
                &[],
                &[],
                |h| pane_kit(h, fixture::GAME, "objects/smoke.light", &light_tag()),
                |h| {
                    let key = fixture::open_document(&mut h.app, "objects/smoke.light", light_tag());
                    fixture::expand_all(&mut h.app, &key);
                },
                &["smoke.light", "Function type:"],
            ),
            // --- synthetic panes ---
            case(
                "pane_bitmap_library",
                &[],
                &[],
                memory_kit,
                |h| h.app.open_bitmap_library(),
                &["Bitmap Library", "No bitmap tags in this workspace."],
            ),
            case(
                "pane_model_library",
                &[],
                &[],
                memory_kit,
                |h| h.app.open_model_library(),
                &["Model Library", "No render model tags in this workspace."],
            ),
            case(
                "pane_git_review",
                &[],
                &[],
                loose_kit,
                |h| h.app.kit_and_view(h.app.model.active).open_tag_pane(GIT_REVIEW_KEY),
                &["Git Review", "No Git repository found"],
            ),
            case(
                "pane_blam",
                &[],
                &[],
                loose_kit,
                |h| h.app.kit_and_view(h.app.model.active).open_tag_pane(BLAM_KEY),
                &["Blam!", "No import has run yet."],
            ),
            case(
                "pane_folder",
                &[],
                &[],
                loose_kit,
                |h| {
                    let ctx = h.ctx.clone();
                    h.app.handle_browser_action(
                        BrowserAction::OpenFolderBrowser {
                            rel_path: PathBuf::from("objects/weapons/rifle"),
                            label: "rifle".to_owned(),
                            open_in_new_tab: false,
                        },
                        ctx,
                    );
                },
                &["rifle.biped", "rifle.scenery"],
            ),
            // --- the three tile trees, split ---
            case(
                "tiles_tag_tree_split",
                &[],
                &[],
                |h| {
                    let mut entries = fixture::synthetic_entries(2, 2, 10);
                    entries.push(fixture::document_entry(
                        "shaders/left.shader",
                        &fixture::synthetic_shader(1),
                    ));
                    entries.push(fixture::document_entry("levels/right.scenario", &scenario_tag()));
                    fixture::install_kit(&mut h.app, entries);
                },
                |h| {
                    fixture::open_document(
                        &mut h.app,
                        "shaders/left.shader",
                        fixture::synthetic_shader(1),
                    );
                    let mut kit = h.app.kit_and_view(h.app.model.active);
                    let key = fixture::entry_key("levels/right.scenario");
                    kit.kit.parsed_tags.insert(key.clone(), TagDocument::clean(scenario_tag()));
                    kit.open_tag_pane_beside(&key);
                },
                &["left.shader", "right.scenario"],
            ),
            case(
                "tiles_kit_tree_split",
                &[],
                &[],
                |h| {
                    fixture::install_kit_for_game(
                        &mut h.app,
                        fixture::synthetic_entries(2, 2, 10),
                        fixture::GAME,
                    );
                },
                |h| {
                    h.app.add_kit();
                    let entries = fixture::synthetic_entries(1, 1, 5)
                        .into_iter()
                        .map(|mut entry| {
                            entry.display_path = entry.display_path.replace("folder_", "second_");
                            entry.key = fixture::entry_key(&entry.display_path);
                            entry
                        })
                        .collect();
                    fixture::install_kit_for_game(&mut h.app, entries, "haloreach_mcc");
                    let ids: Vec<KitId> = h.app.model.kits.iter().map(|kit| kit.id).collect();
                    let mut tree = egui_tiles::Tree::empty("smoke_kit_tree");
                    let panes = ids.iter().map(|id| tree.tiles.insert_pane(*id)).collect();
                    tree.root = Some(tree.tiles.insert_horizontal_tile(panes));
                    h.app.kit_tree = tree;
                },
                &["folder_00", "second_00"],
            ),
            case(
                "tiles_chimp_surface",
                &[],
                &[],
                |h| {
                    h.app.model.prefs.enable_chimp = true;
                    container_kit(h);
                },
                |h| h.app.views[h.app.model.kits[h.app.model.active].id].surface = KitSurface::Chimp,
                &["The Unreal package index has not been started."],
            ),
            // --- windows over the shell ---
            case(
                "first_run_storage",
                &["dialog:FirstRunWizardState"],
                &["shell/first_run.rs"],
                welcome,
                |h| {
                    h.app
                        .dialogs
                        .open(FirstRunWizardState::new(None, &h.app.model.prefs))
                },
                &["Welcome to Baboon", "Installed mode (recommended)"],
            ),
            case(
                "first_run_interface",
                &["dialog:FirstRunWizardState"],
                &["shell/first_run.rs"],
                welcome,
                |h| {
                    let mut wizard = FirstRunWizardState::new(None, &h.app.model.prefs);
                    wizard.page = FirstRunPage::Interface;
                    h.app.dialogs.open(wizard);
                },
                &["Welcome to Baboon", "Updates and interface"],
            ),
            case(
                "first_run_editing_kits",
                &["dialog:FirstRunWizardState"],
                &["shell/first_run.rs"],
                welcome,
                |h| {
                    let mut wizard = FirstRunWizardState::new(None, &h.app.model.prefs);
                    wizard.page = FirstRunPage::EditingKits;
                    // Detection searches the machine for installed kits; the
                    // case is about the page, not about what is installed here.
                    wizard.editing_kit_detection_ran = true;
                    h.app.dialogs.open(wizard);
                },
                &["Welcome to Baboon", "Detected paths fill only empty entries."],
            ),
            case(
                "help_about",
                &["dialog:HelpWindow"],
                &["help/window.rs"],
                welcome,
                |h| {
                    h.app
                        .dialogs
                        .open(HelpWindow::new(&h.app.help, HelpPanelTab::About));
                },
                &["Baboon Help", "blam-tags created by"],
            ),
            case(
                "help_doc",
                &["dialog:HelpWindow"],
                &["help/window.rs"],
                welcome,
                |h| {
                    h.app
                        .dialogs
                        .open(HelpWindow::new(&h.app.help, HelpPanelTab::Doc));
                },
                &["Baboon Help", "Supported games"],
            ),
            case(
                "help_tutorials",
                &["dialog:HelpWindow"],
                &["help/window.rs"],
                welcome,
                |h| {
                    h.app
                        .dialogs
                        .open(HelpWindow::new(&h.app.help, HelpPanelTab::Tutorials));
                },
                &["Baboon Help", "Watch on YouTube"],
            ),
            case(
                "help_script_doc",
                &["dialog:HelpWindow"],
                &["help/window.rs"],
                welcome,
                |h| {
                    h.app
                        .dialogs
                        .open(HelpWindow::new(&h.app.help, HelpPanelTab::ScriptDoc));
                },
                &["Baboon Help", "Network safe"],
            ),
            case(
                "help_tag_compat",
                &["dialog:HelpWindow"],
                &["help/window.rs"],
                welcome,
                |h| {
                    h.app
                        .dialogs
                        .open(HelpWindow::new(&h.app.help, HelpPanelTab::TagCompat));
                },
                &["Baboon Help", "Only what is lost"],
            ),
            case(
                "help_map_names",
                &["dialog:HelpWindow"],
                &["help/window.rs"],
                welcome,
                |h| {
                    h.app
                        .dialogs
                        .open(HelpWindow::new(&h.app.help, HelpPanelTab::MapNames));
                },
                &["Baboon Help", "The Pillar of Autumn"],
            ),
            case(
                "settings_startup",
                &["dialog:SettingsWindow"],
                &["shell/settings.rs"],
                welcome,
                |h| {
                    h.app.open_settings(Some(SettingsTab::Startup));
                },
                &["Settings", "When reopening Baboon with a previous session:"],
            ),
            case(
                "settings_browser",
                &["dialog:SettingsWindow"],
                &["shell/settings.rs"],
                welcome,
                |h| {
                    h.app.open_settings(Some(SettingsTab::Browser));
                },
                &["Settings", "Double-click to open tags"],
            ),
            case(
                "settings_editing_kits",
                &["dialog:SettingsWindow"],
                &["shell/settings.rs"],
                welcome,
                |h| {
                    h.app.open_settings(Some(SettingsTab::EditingKits));
                },
                &["Settings", "Auto Detect"],
            ),
            case(
                "settings_appearance",
                &["dialog:SettingsWindow"],
                &["shell/settings.rs"],
                welcome,
                |h| {
                    h.app.open_settings(Some(SettingsTab::Appearance));
                },
                &["Settings", "Angles in degrees"],
            ),
            case(
                "settings_tools",
                &["dialog:SettingsWindow"],
                &["shell/settings.rs"],
                welcome,
                |h| {
                    h.app.open_settings(Some(SettingsTab::Tools));
                },
                &["Settings", "Chimp — Unreal mappings"],
            ),
            case(
                "settings_custom_kit_draft",
                &["dialog:CustomEditingKitDraft"],
                &["shell/settings.rs"],
                welcome,
                |h| {
                    h.app.open_settings(Some(SettingsTab::EditingKits));
                    h.app.dialogs.open(CustomEditingKitDraft::new());
                },
                &["Editing Kit Root Folder"],
            ),
            case(
                "settings_custom_kit_removal",
                &["dialog:CustomEditingKitRemoval"],
                &["shell/settings.rs"],
                welcome,
                |h| {
                    h.app.open_settings(Some(SettingsTab::EditingKits));
                    h.app.dialogs.open(CustomEditingKitRemoval {
                        id: "smoke".to_owned(),
                        name: "Smoke Kit".to_owned(),
                    });
                },
                &["Remove Editing Kit?"],
            ),
            case(
                "find",
                &["search.find", "dialog:FindWindow"],
                &["search/find_window.rs"],
                |h| {
                    scenario_kit(h);
                    open_scenario(h);
                },
                |h| {
                    h.app.open_find();
                    h.app.search.find.query = "sky".to_owned();
                },
                &["Find", "Filter Results"],
            ),
            case(
                "tool_commands",
                &["dialog:ToolCommandsUiState"],
                &["kits/tool_commands_window.rs"],
                loose_kit,
                |h| h.app.dialogs.open(ToolCommandsUiState::default()),
                &["Tool Commands"],
            ),
            case(
                "tag_compare",
                &["dialog:TagDiffState"],
                &["compare/tag_compare.rs"],
                |h| {
                    scenario_kit(h);
                    open_scenario(h);
                },
                |h| {
                    h.app.dialogs.open(TagDiffState {
                        kit: active_id(h),
                        a_key: fixture::entry_key(SCENARIO),
                        source: TagCompareSource::OpenTag,
                        b_kit: None,
                        b_key: None,
                        b_path: None,
                        comparison_kit_root: None,
                        git_history: Default::default(),
                        error: None,
                        filters: Default::default(),
                        swapped: false,
                        results: None,
                        git_pending: None,
                    });
                },
                &["Compare"],
            ),
            case(
                "content_explorer",
                &["dialog:ContentExplorer"],
                &["references/explorer.rs"],
                memory_kit,
                |h| {
                    let focus = h.app.model.kits[h.app.model.active]
                        .source
                        .as_ref()
                        .unwrap()
                        .entries[0]
                        .clone();
                    h.app.dialogs.open(ContentExplorer {
                        kit: active_id(h),
                        focus,
                        parents: Vec::new(),
                        children: Vec::new(),
                        filter: String::new(),
                        index_unavailable: true,
                        back: Vec::new(),
                        forward: Vec::new(),
                    });
                },
                &["Content Explorer"],
            ),
            case(
                "query_results",
                &["dialog:QueryResultsWindow"],
                &["search/result_windows.rs"],
                memory_kit,
                |h| {
                    let entries = h.app.model.kits[h.app.model.active].source.as_ref().unwrap().entries[..3]
                        .to_vec();
                    h.app.dialogs.open(QueryResultsWindow::new(TagQueryResults {
                        kit: active_id(h),
                        title: "Smoke Query Results".to_owned(),
                        entries,
                        annotations: Vec::new(),
                        note: None,
                        ref_target: None,
                    }));
                },
                &["Smoke Query Results", "tag_000"],
            ),
            case(
                "field_value_search",
                &["dialog:FieldValueSearchWindow"],
                &["search/result_windows.rs"],
                loose_kit,
                |h| h.app.dialogs.open(FieldValueSearchWindow::default()),
                &["Search Field Values"],
            ),
            case(
                "tag_reference_picker",
                &["dialog:TagReferencePickerWindow"],
                &["editor/dialogs.rs"],
                container_kit,
                |h| {
                    let kit = active_id(h);
                    h.app.dialogs.open(TagReferencePickerWindow {
                        state: TagReferencePickerState {
                            tag_key: ce_key(),
                            field_path: "model".to_owned(),
                            allowed_groups: vec![u32::from_be_bytes(*b"hlmt")],
                            search: String::new(),
                        },
                        kit,
                    });
                },
                &["Select Tag Reference"],
            ),
            case(
                "colour_popup",
                &["dialog:ColorPopupWindow"],
                &["editor/material/color_picker.rs"],
                memory_kit,
                |h| {
                    let kit = active_id(h);
                    h.app.dialogs.open(ColorPopupWindow {
                        popup: Some(MaterialColorPopup::new("Smoke Tint", 1.0, 0.5, 0.25, 1.0)),
                        kit,
                        opened_at: None,
                    });
                },
                &["Color Picker"],
            ),
            case(
                "function_popup",
                &["dialog:FunctionPopupWindow"],
                &["editor/function_editor/mod.rs"],
                memory_kit,
                |h| {
                    let bytes = decode_hex(&constant_function_hex(0.5)).unwrap();
                    let view = FunctionView::from_function(TagFunction::parse(&bytes).unwrap());
                    let kit = active_id(h);
                    h.app.dialogs.open(FunctionPopupWindow {
                        popup: Some(FunctionPopup::new(
                            biped_key(),
                            "Smoke Function".to_owned(),
                            view,
                            true,
                        )),
                        kit,
                        opened_at: None,
                    });
                },
                &["Smoke Function"],
            ),
            case(
                "save_changes_prompt",
                &["dialog:SaveChangesPrompt"],
                &["documents/close.rs"],
                memory_kit,
                |h| {
                    h.app.dialogs.open(SaveChangesPrompt {
                        kit: h.app.model.kits[0].id,
                        can_stash: false,
                        dirty_tags: vec![DirtyTagEntry {
                            path: "objects/smoke.biped".to_owned(),
                            tag_id: biped_key(),
                            checked: true,
                        }],
                        pending_action: PendingCloseAction::CloseApp,
                        error: None,
                        stash_file: None,
                        stashed: 0,
                        confirm_discard: false,
                    });
                },
                &["Baboon - Save Changes?", "objects/smoke.biped"],
            ),
            case(
                "last_opened_windows",
                &["dialog:LastOpenedWindowsPrompt"],
                &["shell/session.rs"],
                welcome,
                |h| {
                    h.app.dialogs.open(LastOpenedWindowsPrompt {
                        kits: vec![crate::app::shell::session::LastOpenedWindowsKit {
                            checked: true,
                            source_kind: crate::app::shell::session::LastSessionSourceKind::LooseFolder,
                            source_path: PathBuf::from("/no/such/smoke/tags"),
                            game: Some(fixture::GAME.to_owned()),
                            profile_id: None,
                            profile_name: None,
                            profile_root: None,
                            source_available: false,
                            project_path: None,
                            has_project: false,
                            browser_mode: None,
                            browser_sort: None,
                            entries: vec![crate::app::shell::session::LastOpenedWindowEntry {
                                tag: crate::app::shell::session::LastSessionTag {
                                    key: "file:/no/such/smoke.biped".to_owned(),
                                    label: "smoke.biped".to_owned(),
                                    group_tag: u32::from_be_bytes(*b"bipd"),
                                    path: None,
                                },
                                checked: false,
                                available: false,
                            }],
                            folder_entries: Vec::new(),
                            chimp_entries: Vec::new(),
                            bitmap_library_open: false,
                            model_library_open: false,
                            active_chimp_package: None,
                            was_active: true,
                        }],
                        dont_ask_again: false,
                    });
                },
                &["Restore Last Opened Windows", "smoke.biped"],
            ),
            case(
                "block_confirm",
                &["dialog:BlockConfirm"],
                &["editor/actions.rs"],
                |h| {
                    scenario_kit(h);
                    open_scenario(h);
                },
                |h| {
                    h.app.dialogs.open(BlockConfirm {
                        opened_at: None,
                        kit: Some(active_id(h)),
                        tag_key: fixture::entry_key(SCENARIO),
                        path: "skies".to_owned(),
                        kind: BlockOpKind::DeleteAll,
                        message: "Delete every smoke element?".to_owned(),
                        confirm_label: "Delete".to_owned(),
                    });
                },
                &["Confirm", "Delete every smoke element?"],
            ),
            case(
                "entry_index_wait_notice",
                &["dialog:IndexingNotice"],
                &["shell/workspace.rs"],
                memory_kit,
                |h| {
                    h.app.dialogs.open(IndexingNotice);
                    h.app.model.kits[h.app.model.active].scanning_entries = true;
                },
                &["Indexing"],
            ),
            case(
                "folder_refactor_lock",
                &["tag_ops.folder_refactor"],
                &[],
                memory_kit,
                |h| {
                    h.app.tag_ops.folder_refactor = Some(FolderRefactorUiState {
                        label: "Renaming smoke to fog".to_owned(),
                        phase: "Moving files".to_owned(),
                        progress: Some(0.5),
                    });
                },
                &["Renaming smoke to fog", "Baboon is locked until references are updated."],
            ),
            // --- ui/dialogs/ ---
            case(
                "new_tag",
                &["dialog:NewTagDialog"],
                &["tag_ops/new_tag_window.rs"],
                loose_kit,
                |h| h.app.open_new_tag_dialog(),
                &["New Tag"],
            ),
            case(
                "delete_confirm_loose",
                &["dialog:DeleteConfirm"],
                &["tag_ops/delete_confirm.rs"],
                loose_kit,
                |h| {
                    let path = loose_root(h).join("objects/weapons/rifle/rifle.biped");
                    h.app.dialogs.open(DeleteConfirm {
                        kit: active_id(h),
                        key: file_entry_key(&path),
                        display_path: "objects/weapons/rifle/rifle.biped".to_owned(),
                        kind: DeleteKind::Loose,
                        referrers: vec!["levels/smoke/smoke.scenario".to_owned()],
                        referrers_unavailable: false,
                        has_unsaved_edits: true,
                    });
                },
                &["Delete tag?", "levels/smoke/smoke.scenario"],
            ),
            case(
                "delete_confirm_container",
                &["dialog:DeleteConfirm"],
                &["tag_ops/delete_confirm.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(DeleteConfirm {
                        kit: active_id(h),
                        key: ce_key(),
                        display_path: CE_TAG.to_owned(),
                        kind: DeleteKind::Container {
                            target_label: "pakchunk0-smoke".to_owned(),
                        },
                        referrers: Vec::new(),
                        referrers_unavailable: true,
                        has_unsaved_edits: false,
                    });
                },
                &["Delete from Campaign Evolved container?"],
            ),
            case(
                "rename_tag",
                &["dialog:RenameTagState"],
                &["tag_ops/rename_tag_window.rs"],
                loose_kit,
                |h| {
                    let path = loose_root(h).join("objects/weapons/rifle/rifle.biped");
                    h.app.dialogs.open(RenameTagState {
                        kit: active_id(h),
                        key: file_entry_key(&path),
                        old_display: "objects/weapons/rifle/rifle.biped".to_owned(),
                        extension: "biped".to_owned(),
                        operation: TagNameOperation::Rename,
                        new_path_input: "objects/weapons/rifle/smoke_rifle".to_owned(),
                        fixed_parent: String::new(),
                        focus_input: true,
                        referrers: Vec::new(),
                        referrers_unavailable: true,
                        is_container: false,
                        is_new_container: false,
                        whole_path_editable: true,
                        in_place_pak: None,
                    });
                },
                &["Rename / Move Tag"],
            ),
            case(
                "duplicate_tag",
                &["dialog:RenameTagState"],
                &["tag_ops/rename_tag_window.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(RenameTagState {
                        kit: active_id(h),
                        key: ce_key(),
                        old_display: CE_TAG.to_owned(),
                        extension: "weapon".to_owned(),
                        operation: TagNameOperation::Duplicate,
                        new_path_input: "rifle_copy".to_owned(),
                        fixed_parent: "objects/weapons/rifle".to_owned(),
                        focus_input: true,
                        referrers: Vec::new(),
                        referrers_unavailable: true,
                        is_container: true,
                        is_new_container: false,
                        whole_path_editable: false,
                        in_place_pak: None,
                    });
                },
                &["Duplicate Tag"],
            ),
            case(
                "import_tag",
                &["dialog:ImportTagDialog"],
                &["import/import_tag_dialog.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ImportTagDialog {
                        kit: active_id(h),
                        source_path: PathBuf::from("/no/such/smoke.weapon"),
                        folder_rel: "objects/weapons".to_owned(),
                        name: "smoke".to_owned(),
                        group_tag: u32::from_be_bytes(*b"weap"),
                        group_name: "weapon".to_owned(),
                        extension: "weapon".to_owned(),
                        tag: Some(fixture::new_tag_for("haloce_evolved", "weapon")),
                        mode: ImportMode::Native {
                            comparison: None,
                            import_anyway: false,
                        },
                        profile_verdicts: Vec::new(),
                        error: None,
                    });
                },
                &["Import Tag"],
            ),
            case(
                "import_discard_confirm",
                &["dialog:PendingImport"],
                &["import/import_tag_dialog.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(PendingImport {
                        kit: active_id(h),
                        tag: fixture::new_tag_for("haloce_evolved", "weapon"),
                        target_key: ce_key(),
                    });
                },
                &["Discard unsaved changes?"],
            ),
            case(
                "tag_import",
                &["dialog:TagImportDialog"],
                &["import/tags_window.rs"],
                loose_kit,
                |h| h.app.open_tag_import_dialog(Some("objects".to_owned())),
                &["Import Tags"],
            ),
            case(
                "cache_import",
                &["dialog:CacheImportDialog"],
                &["import/cache_window.rs"],
                loose_kit,
                |h| {
                    let target = CacheImportTarget {
                        kit: active_id(h),
                        label: "Smoke Kit".to_owned(),
                        game: GameId::from_id(fixture::GAME).unwrap(),
                        tags_root: loose_root(h),
                    };
                    h.app.dialogs.open(CacheImportDialog {
                        kit: active_id(h),
                        prefix: "objects/weapons".to_owned(),
                        selected: 2,
                        targets: vec![target],
                        target_index: 0,
                        outside_tree: Default::default(),
                        outside_picked: Default::default(),
                        single: None,
                        destination: None,
                        replace: ReplaceChoice::Always,
                        conflicts: Default::default(),
                        conflict_picked: Default::default(),
                        conflicts_stale: false,
                        scanning: false,
                        running: false,
                        cancel: Default::default(),
                        progress: None,
                        report: None,
                        error: None,
                    });
                },
                &["Import Cache Folder"],
            ),
            case(
                "overwrite_confirm",
                &["dialog:OverwriteConfirm"],
                &["mods/overwrite_confirm.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(OverwriteConfirm {
                        kit: active_id(h),
                        key: ce_key(),
                    });
                },
                &["Overwrite game files?"],
            ),
            case(
                "clear_stash_confirm",
                &["dialog:ClearStashConfirm"],
                &["mods/clear_stash_confirm.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ClearStashConfirm {
                        kit: active_id(h),
                        stashed: vec![CE_TAG.to_owned()],
                        unsaved: 1,
                    });
                },
                &["Clear unsaved modifications?"],
            ),
            case(
                "container_dump_confirm",
                &["dialog:ContainerDumpConfirm"],
                &["export/container_dump_confirm.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ContainerDumpConfirm {
                        kit: active_id(h),
                        output: PathBuf::from("/no/such/smoke-out"),
                        total: 2,
                        scope: ContainerDumpScope::AllShipped,
                    });
                },
                &["Extract every shipped tag?"],
            ),
            case(
                "container_duplicate_confirm",
                &["dialog:ContainerDuplicateConfirm"],
                &["tag_ops/container_duplicate_confirm.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ContainerDuplicateConfirm {
                        kit: active_id(h),
                        key: ce_key(),
                        destination_leaf: "rifle_copy".to_owned(),
                    });
                },
                &["Duplicate in Campaign Evolved container?"],
            ),
            case(
                "container_folder",
                &["dialog:ContainerFolderDialog"],
                &["tag_ops/container_folder_window.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ContainerFolderDialog {
                        kit: active_id(h),
                        parent_rel: Some("objects".to_owned()),
                        renaming: None,
                        name_input: "smoke".to_owned(),
                        focus_input: true,
                        error: None,
                    });
                },
                &["New Folder"],
            ),
            case(
                "loose_folder_rename",
                &["dialog:LooseFolderRenameState"],
                &["tag_ops/loose_folder_rename_window.rs"],
                loose_kit,
                |h| {
                    h.app.dialogs.open(LooseFolderRenameState {
                        kit: active_id(h),
                        rel_path: PathBuf::from("objects/weapons/rifle"),
                        parent_display: "objects/weapons".to_owned(),
                        old_name: "rifle".to_owned(),
                        name_input: "smoke_rifle".to_owned(),
                        focus_input: true,
                        error: None,
                        tag_count: 2,
                        outside_referrers: None,
                    });
                },
                &["Rename Folder"],
            ),
            case(
                "exported_mod",
                &["dialog:ExportedMod"],
                &["mods/exported_mod_window.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ExportedMod {
                        stem: "Smoke_P".to_owned(),
                        directory: PathBuf::from("/no/such/Paks/~mods"),
                        count: 2,
                        skipped: 0,
                    });
                },
                &["Mod exported"],
            ),
            case(
                "mod_export",
                &["dialog:ModExportDialog"],
                &["mods/mod_export_window.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ModExportDialog {
                        kit: active_id(h),
                        review_only: false,
                        snapshot: CampaignProjectSnapshot {
                            game: "haloce_evolved".to_owned(),
                            source_path: PathBuf::new(),
                            selected_identity: None,
                            tabs: Vec::new(),
                            overlays: HashMap::new(),
                            history: Default::default(),
                            folders: Default::default(),
                        },
                        rows: vec![ModExportRow {
                            identity: CE_TAG.to_owned(),
                            display_path: CE_TAG.to_owned(),
                            group_tag: u32::from_be_bytes(*b"weap"),
                            kind: ModExportChange::Modified,
                            include: true,
                            bytes: 1024,
                            reason: None,
                            overridden_by: None,
                        }],
                        name: "Smoke".to_owned(),
                        folder: PathBuf::from("/no/such/Paks/~mods"),
                        overwrite_acknowledged: false,
                        expanded: HashSet::new(),
                        diffs: HashMap::new(),
                        controls_height: 0.0,
                    });
                },
                &["Export Mod", "Smoke_P.utoc"],
            ),
            case(
                "keyword_chooser",
                &["dialog:KeywordChooser"],
                &["browser/keyword_chooser.rs"],
                memory_kit,
                |h| h.app.dialogs.open(KeywordChooser),
                &["Keywords"],
            ),
            case(
                "tsv_paste",
                &["dialog:TsvPasteState"],
                &["editor/tsv_paste_window.rs"],
                |h| {
                    scenario_kit(h);
                    open_scenario(h);
                },
                |h| {
                    h.app.dialogs.open(TsvPasteState {
                        kit: active_id(h),
                        tag_key: fixture::entry_key(SCENARIO),
                        block_path: "skies".to_owned(),
                        block_label: "skies".to_owned(),
                        element_count: 2,
                        text: String::new(),
                        status: None,
                    });
                },
                &["Paste TSV → skies"],
            ),
            case(
                "block_table",
                &["dialog:BlockTableState"],
                &["editor/block_table_window.rs"],
                |h| {
                    scenario_kit(h);
                    open_scenario(h);
                },
                |h| {
                    let kit = &h.app.model.kits[h.app.model.active];
                    let key = fixture::entry_key(SCENARIO);
                    let table = crate::app::editor::block_table_for(
                        kit.id,
                        &key,
                        &kit.parsed_tags[&key],
                        kit.source.as_ref(),
                        &kit.names,
                        crate::app::editor::BlockTableRequest {
                            path: "skies".to_owned(),
                            label: "skies".to_owned(),
                            view_scope: "smoke".to_owned(),
                            selected: 0,
                        },
                    )
                    .expect("the scenario's skies block");
                    h.app.dialogs.open(table);
                },
                &["Block Entry Table - skies"],
            ),
            case(
                "operation_notice",
                &["dialog:OperationNotice"],
                &["shell/operation_notice.rs"],
                welcome,
                |h| {
                    h.app.dialogs.open(OperationNotice {
                        title: "Smoke notice".to_owned(),
                        message: "The smoke operation finished.".to_owned(),
                        failed: false,
                    });
                },
                &["Smoke notice", "The smoke operation finished."],
            ),
            case(
                "extract_target",
                &["dialog:ExtractTargetPrompt"],
                &["export/extract_target_window.rs"],
                memory_kit,
                |h| {
                    h.app.dialogs.open(ExtractTargetPrompt {
                        kit: h.app.model.kits[0].id,
                        key: biped_key(),
                        display_path: "objects/smoke.render_model".to_owned(),
                        kind: ExtractKind::Geometry,
                        source: blam_tags::game::Game::Halo3,
                        target: blam_tags::game::Game::Halo3,
                    });
                },
                &["Extract Geometry"],
            ),
            case(
                "chimp_discard",
                &["dialog:ChimpDiscardPrompt"],
                &["chimp/save.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ChimpDiscardPrompt {
                        kit: active_id(h),
                        packages: vec!["/Game/Smoke/SM_Smoke".to_owned()],
                        pending_action: None,
                        error: None,
                    });
                },
                &["Discard Chimp changes?"],
            ),
            case(
                "chimp_save",
                &["dialog:ChimpSaveDialog"],
                &["chimp/save.rs"],
                container_kit,
                |h| {
                    let kit = h.app.model.active;
                    h.app.open_chimp_save_dialog_for_test(kit);
                },
                &["Save Chimp changes"],
            ),
            case(
                "chimp_mesh_texture_prompt",
                &["dialog:ChimpMeshTexturePrompt"],
                &["chimp/prompts_window.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ChimpMeshTexturePrompt::for_test(
                        active_id(h),
                        "/Game/Smoke/SM_Smoke",
                    ));
                },
                &["Export textures with this mesh?"],
            ),
            case(
                "chimp_texture_export_prompt",
                &["dialog:ChimpTextureExportPrompt"],
                &["chimp/prompts_window.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ChimpTextureExportPrompt::for_test(
                        active_id(h),
                        "/Game/Smoke/T_Smoke",
                    ));
                },
                &["Extract texture"],
            ),
            case(
                "chimp_level_export_prompt",
                &["dialog:ChimpLevelExportPrompt"],
                &["chimp/prompts_window.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(ChimpLevelExportPrompt::for_test(
                        active_id(h),
                        "/Game/Smoke/L_Smoke",
                    ));
                },
                &["Export level"],
            ),
            case(
                "poke_scanning",
                &["dialog:PokeDialog"],
                &["runtime_poke.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(PokeDialog {
                        kit: active_id(h),
                        key: ce_key(),
                        state: PokeDialogState::Scanning,
                    });
                },
                &["Poke Current Tag"],
            ),
            case(
                "poke_error",
                &["dialog:PokeDialog"],
                &["runtime_poke.rs"],
                container_kit,
                |h| {
                    h.app.dialogs.open(PokeDialog {
                        kit: active_id(h),
                        key: ce_key(),
                        state: PokeDialogState::Error("The smoke process is not running".to_owned()),
                    });
                },
                &["Poke Current Tag", "The smoke process is not running"],
            ),
        ];
        cases.sort_by_key(|case| case.name);
        cases
    }

    /// `Baboon` fields shaped like window state (an `Option`, a flag, or a
    /// `…Dialog`/`…Prompt`/`…State` type) that are not a window of their own,
    /// and why. A field of that shape named neither here nor by a case fails
    /// [`every_window_has_a_smoke_case`].
    const NOT_WINDOWS: &[(&str, &str)] = &[
        ("window_state", "native window geometry tracker"),
        ("native_clock", "the clock of the latest input"),
        ("import.native_template_cache", "import cache"),
        ("shell.available_update", "data shown in Settings and the status bar"),
        ("shell.last_update_check", "data shown in Settings"),
        ("export.container_dump_job", "a running job; its progress is in the status bar"),
        ("chimp.chimp_level_job", "a running job; its progress is in the status bar"),
        ("mods.last_mod_export_name", "remembered text"),
        ("chimp.chimp_writes", "running saves"),
        (
            "shell.artwork",
            "texture cache of game and editing-kit artwork",
        ),
        ("poke.last_poke", "undo record"),
        ("poke.poke_direct_running", "running flag"),
        ("poke.poke_undo_running", "running flag"),
        ("kit_tools.editing_kit_path_attention", "highlights a row of the Settings window"),
        ("editor.deferred_file_action", "a queued action"),
        ("shell.restored_active_kit", "session restore bookkeeping"),
        ("browser.reveal_target", "a one-shot browser request"),
        ("search.field_value_searching", "running flag of the field value search"),
        (
            "documents.allow_app_close_once",
            "lets the confirmed second app-close request through",
        ),
        ("kit_tools.kit_tool_drag", "drag-and-drop tracker"),
        ("ce_usmap", "parsed mappings cache"),
        ("export.pending_sound_extract", "a queued request"),
        ("editor.pending_ce_sound_ref", "a queued request"),
        ("references.pending_open", "a queued request"),
        ("kit_tools.pending_tool_import", "a queued request"),
        ("shell.blender_icon", "texture"),
        ("editor.block_clipboard", "clipboard contents"),
        ("references.pending_ref_jump", "a queued navigation"),
        ("search.pending_find_jump", "a queued navigation"),
        ("references.field_nav", "navigation highlight"),
    ];

    // ---------------------------------------------------------------------------
    // Runner
    // ---------------------------------------------------------------------------

    /// Run `steps` on a fresh app, then [`FRAMES`] frames; what the last painted.
    fn painted_after(steps: &[Step]) -> (Vec<String>, String) {
        let mut h = Harness::new();
        for step in steps {
            step(&mut h);
        }
        for _ in 0..FRAMES {
            h.frame(Vec::new());
        }
        for dir in TEMP_DIRS.with(|dirs| std::mem::take(&mut *dirs.borrow_mut())) {
            let _ = std::fs::remove_dir_all(dir);
        }
        (h.painted, h.app.model.status)
    }

    /// Frames run in `secs` of egui time when the app gets one only when it
    /// asks: right away, or after the delay it asked for.
    fn frames_while_idle(h: &mut Harness, secs: f64) -> usize {
        let end = h.time + secs;
        let mut frames = 0;
        while h.repaint_delay != Duration::MAX {
            let step = h.repaint_delay.as_secs_f64().max(1.0 / 60.0);
            if h.time + step > end {
                break;
            }
            // `frame` moves the clock on by a 60 Hz frame itself.
            h.time += step - 1.0 / 60.0;
            h.frame(Vec::new());
            frames += 1;
        }
        frames
    }

    /// Every surface the smoke cases open goes quiet once nothing happens.
    /// Baboon used to redraw at the display's rate for as long as a tag was
    /// open, about 12% of a CPU for a weapon tag: a command the panes send
    /// every frame asked for the next frame. A blinking text cursor or an
    /// animation finishing may still wake it; redrawing every frame may not.
    #[test]
    fn every_surface_goes_quiet_when_nothing_happens() {
        let mut counts = Vec::new();
        for case in cases() {
            let mut h = Harness::new();
            (case.base)(&mut h);
            (case.open)(&mut h);
            for _ in 0..FRAMES {
                h.frame(Vec::new());
            }
            let frames = frames_while_idle(&mut h, 10.0);
            for dir in TEMP_DIRS.with(|dirs| std::mem::take(&mut *dirs.borrow_mut())) {
                let _ = std::fs::remove_dir_all(dir);
            }
            counts.push((case.name, frames));
        }
        // Redrawing every frame is 600 frames in 10 s. The busiest surface
        // that is merely waiting wakes under 100 times: a background check
        // every 0.6 s, a focused box's cursor blinking.
        const BUDGET: usize = 200;
        // Shows a spinner while a background job builds the poke plan, which
        // in the app ends when the job does; the case has no job to end it.
        const SPINNING: &[&str] = &["poke_scanning"];
        let busy: Vec<_> = counts
            .iter()
            .filter(|(name, frames)| *frames > BUDGET && !SPINNING.contains(name))
            .collect();
        assert!(
            busy.is_empty(),
            "redrawing with nothing to do (frames in 10 s): {busy:?}"
        );
        assert!(
            counts
                .iter()
                .any(|(name, frames)| SPINNING.contains(name) && *frames > BUDGET),
            "the spinner case no longer spins, so the test can't tell busy from quiet"
        );
    }

    fn missing(painted: &[String], expect: &[&'static str]) -> Vec<&'static str> {
        expect
            .iter()
            .copied()
            .filter(|needle| !painted.iter().any(|text| text.contains(needle)))
            .collect()
    }

    fn run_case(case: &Case) -> Result<(), String> {
        let (painted, status) = painted_after(&[case.base, case.open]);
        if std::env::var_os("BABOON_SMOKE_DUMP").is_some() {
            eprintln!("[smoke] {}: status `{status}`; painted {painted:?}", case.name);
        }
        let absent = missing(&painted, case.expect);
        if !absent.is_empty() {
            let mut shown = painted;
            shown.truncate(80);
            return Err(format!(
                "not painted: {absent:?}; status `{status}`; painted: {shown:?}"
            ));
        }
        let (control, _) = painted_after(&[case.base]);
        if missing(&control, case.expect).is_empty() {
            return Err(format!(
                "the base alone already paints every expectation {:?}, so they cannot show \
             the window drew",
                case.expect
            ));
        }
        Ok(())
    }

    fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "panicked".to_owned())
    }

    /// Run every `SHARDS`th case from `shard`, each on a fresh app, and report
    /// every failure at once.
    fn run_shard(shard: usize) {
        let only: Vec<String> = std::env::var("BABOON_SMOKE_ONLY")
            .map(|value| value.split(',').map(str::trim).map(str::to_owned).collect())
            .unwrap_or_default();
        let mut failures = Vec::new();
        let mut ran = 0;
        for (index, case) in cases().iter().enumerate() {
            if index % SHARDS != shard
                || (!only.is_empty() && !only.iter().any(|part| case.name.contains(part.as_str())))
            {
                continue;
            }
            ran += 1;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_case(case)))
                .unwrap_or_else(|panic| Err(format!("panicked: {}", panic_message(panic))));
            if let Err(problem) = result {
                failures.push(format!("{}: {problem}", case.name));
            }
        }
        assert!(ran > 0 || !only.is_empty(), "shard {shard} has no cases");
        assert!(
            failures.is_empty(),
            "{} of {ran} case(s) failed:\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }

    /// The cases are split across this many tests, which the harness runs in
    /// parallel: run one after another they take most of a minute.
    const SHARDS: usize = 8;

    macro_rules! smoke_shards {
        ($($name:ident = $shard:literal),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    run_shard($shard);
                }
            )*
            #[test]
            fn every_case_is_in_a_shard() {
                assert_eq!([$($shard),*], std::array::from_fn::<usize, SHARDS, _>(|i| i));
            }
        };
    }

    smoke_shards!(
        windows_draw_shard_0 = 0,
        windows_draw_shard_1 = 1,
        windows_draw_shard_2 = 2,
        windows_draw_shard_3 = 3,
        windows_draw_shard_4 = 4,
        windows_draw_shard_5 = 5,
        windows_draw_shard_6 = 6,
        windows_draw_shard_7 = 7,
    );

    // ---------------------------------------------------------------------------
    // Registry: a window without a case is noticed
    // ---------------------------------------------------------------------------

    /// The `Baboon` struct's fields and their types, read from `src/app/mod.rs`.
    /// A field holding one feature's state (a `…Feature` struct) stands for that
    /// struct's fields, named `field.inner`: they are where its windows live. Each
    /// dialog the host can hold (an `impl Dialog`) is listed too, as
    /// `dialog:Type`.
    fn baboon_fields() -> Vec<(String, String)> {
        let source = include_root_str!("src/app/mod.rs");
        let sources = crate::app::source_scan::app_product_sources();
        let mut out = Vec::new();
        for (name, ty) in struct_fields(source, "pub struct Baboon {") {
            if ty.ends_with("Feature") {
                let header = format!("struct {ty} {{");
                let text = sources
                    .iter()
                    .find(|(_, text)| text.contains(&header))
                    .map(|(_, text)| text.as_str())
                    .unwrap_or_else(|| panic!("`{ty}` is defined under src/app"));
                out.extend(
                    struct_fields(text, &header)
                        .into_iter()
                        .map(|(inner, inner_ty)| (format!("{name}.{inner}"), inner_ty)),
                );
            } else {
                out.push((name, ty));
            }
        }
        // The dialog host's windows are not fields: each `impl Dialog` is one,
        // named `dialog:Type`.
        for (_, text) in &sources {
            for rest in text.split("impl Dialog for ").skip(1) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                out.push((format!("dialog:{name}"), "Dialog".to_owned()));
            }
        }
        out
    }

    /// The fields of the struct whose declaration starts with `header`.
    fn struct_fields(source: &str, header: &str) -> Vec<(String, String)> {
        let body = source
            .split_once(header)
            .unwrap_or_else(|| panic!("no `{header}`"))
            .1;
        let body = &body[..body.find("\n}\n").expect("the struct ends")];
        body.lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("//") && !line.starts_with('#'))
            .map(|line| match line.strip_prefix("pub(") {
                Some(rest) => rest.split_once(") ").map_or(line, |(_, field)| field),
                None => line,
            })
            .filter_map(|line| {
                let (name, ty) = line.split_once(':')?;
                let name = name.trim();
                (!name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
                    .then(|| (name.to_owned(), ty.trim().trim_end_matches(',').to_owned()))
            })
            .collect()
    }

    /// Whether a field has the shape of window state: optional, a flag, or a
    /// type named like a dialog's.
    fn looks_like_a_window(ty: &str) -> bool {
        let leaf = ty.rsplit("::").next().unwrap_or(ty);
        ty.contains("Option<")
            || ty == "bool"
            || ["Dialog", "Prompt", "State", "Confirm", "Popup"]
                .iter()
                .any(|suffix| leaf.ends_with(suffix))
    }

    /// Whether `code` calls `egui::Window::new`, however it is imported, and not
    /// merely a constructor whose name ends in `Window`, like `HelpWindow::new`.
    fn opens_a_window(code: &str) -> bool {
        code.match_indices("Window::new(").any(|(at, _)| {
            !code[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    }

    /// Files under `src/app/` (relative, `/`-separated) whose product code
    /// calls `egui::Window::new`.
    fn window_sources() -> Vec<String> {
        crate::app::source_scan::app_product_sources()
            .into_iter()
            .filter(|(_, text)| opens_a_window(text))
            .map(|(file, _)| file)
            .collect()
    }

    /// Everything the registry is missing or names wrongly, given the struct's
    /// fields and the files that open windows.
    fn registry_problems(
        cases: &[Case],
        fields: &[(String, String)],
        sources: &[String],
    ) -> Vec<String> {
        let named_fields: HashSet<&str> = cases.iter().flat_map(|c| c.fields.iter().copied()).collect();
        let named_sources: HashSet<&str> =
            cases.iter().flat_map(|c| c.sources.iter().copied()).collect();
        let excused: HashSet<&str> = NOT_WINDOWS.iter().map(|(name, _)| *name).collect();
        let mut problems = Vec::new();
        for (name, ty) in fields {
            if looks_like_a_window(ty)
                && !named_fields.contains(name.as_str())
                && !excused.contains(name.as_str())
            {
                problems.push(format!(
                    "`Baboon::{name}: {ty}` has no smoke case: add a row to `cases()` that opens \
                 it, or list it in NOT_WINDOWS with the reason"
                ));
            }
        }
        let field_names: HashSet<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
        for name in named_fields.iter().chain(excused.iter()) {
            if !field_names.contains(name) {
                problems.push(format!("`{name}` is named by the registry but is not a Baboon field"));
            }
        }
        for source in sources {
            if !named_sources.contains(source.as_str()) {
                problems.push(format!(
                    "src/app/{source} opens an egui::Window that no smoke case draws"
                ));
            }
        }
        for source in &named_sources {
            if !sources.iter().any(|s| s == source) {
                problems.push(format!("a case names src/app/{source}, which opens no window"));
            }
        }
        problems
    }

    #[test]
    fn every_window_has_a_smoke_case() {
        let fields = baboon_fields();
        assert!(
            fields.len() > 50
                && fields
                    .iter()
                    .any(|(name, _)| name == "dialog:DeleteConfirm"),
            "the field scan found {} fields; it no longer reads the struct",
            fields.len()
        );
        let sources = window_sources();
        assert!(
            sources.iter().any(|s| s == "tag_ops/delete_confirm.rs")
                && sources.iter().any(|s| s == "shell/settings.rs"),
            "the source scan found {sources:?}; it no longer finds windows"
        );
        let problems = registry_problems(&cases(), &fields, &sources);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// The registry check can fail: a new dialog field, and a new file with a
    /// window in it, are each reported when nothing names them.
    #[test]
    fn the_registry_check_notices_a_window_without_a_case() {
        let mut fields = baboon_fields();
        let mut sources = window_sources();
        assert!(registry_problems(&cases(), &fields, &sources).is_empty());
        fields.push(("smoke_dialog".to_owned(), "Option<SmokeDialog>".to_owned()));
        fields.push(("dialog:SmokeHosted".to_owned(), "Dialog".to_owned()));
        sources.push("shell/frame/dialogs/smoke.rs".to_owned());
        let problems = registry_problems(&cases(), &fields, &sources);
        assert!(problems.iter().any(|p| p.contains("Baboon::smoke_dialog")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("dialog:SmokeHosted")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("shell/frame/dialogs/smoke.rs")), "{problems:?}");
        assert_eq!(problems.len(), 3, "{problems:?}");
    }

    // Every way out of the app to a web page asks the platform to open it.
    // egui only reports the request; eframe opens it only with its `links`
    // feature, which egui 0.29's eframe turned on by itself and 0.36's does not.

    /// Click `text`'s `nth` painting and return every URL that asked to open.
    fn click(h: &mut Harness, text: &str, nth: usize) -> Vec<egui::OpenUrl> {
        h.click(text, nth)
            .into_iter()
            .filter_map(|command| match command {
                egui::OutputCommand::OpenUrl(open) => Some(open),
                _ => None,
            })
            .collect()
    }

    fn urls(opened: &[egui::OpenUrl]) -> Vec<&str> {
        opened.iter().map(|open| open.url.as_str()).collect()
    }

    fn idle(h: &mut Harness) {
        for _ in 0..4 {
            h.frame(Vec::new());
        }
    }

    /// The tests below see the request; only eframe's `links` feature turns it
    /// into an open browser tab, and nothing fails without it but the user's click.
    #[test]
    fn eframe_is_built_with_its_links_feature() {
        let manifest = include_root_str!("Cargo.toml");
        let eframe = manifest
            .lines()
            .find(|line| line.starts_with("eframe = "))
            .expect("Cargo.toml names eframe");
        assert!(eframe.contains("\"links\""), "{eframe}");
    }

    /// The welcome screen's GitHub and Discord buttons.
    #[test]
    fn the_welcome_links_ask_to_open_their_pages() {
        let mut h = Harness::new();
        idle(&mut h);
        assert_eq!(urls(&click(&mut h, "Baboon GitHub", 0)), [BABOON_GITHUB_URL]);
        assert_eq!(
            urls(&click(&mut h, "Halo Mods Discord", 0)),
            ["https://discord.com/invite/4pKEpNW"]
        );
    }

    /// The Help window's source link and a tutorial's "Watch on YouTube".
    #[test]
    fn the_help_window_links_ask_to_open_their_pages() {
        let mut h = Harness::new();
        h.app
            .dialogs
            .open(HelpWindow::new(&h.app.help, HelpPanelTab::About));
        idle(&mut h);
        assert_eq!(urls(&click(&mut h, BABOON_GITHUB_URL, 0)), [BABOON_GITHUB_URL]);

        h.app.dialogs.get_mut::<HelpWindow>().unwrap().tab = HelpPanelTab::Tutorials;
        idle(&mut h);
        let opened = click(&mut h, "Watch on YouTube", 0);
        assert_eq!(opened.len(), 1, "{opened:?}");
        assert!(opened[0].url.starts_with("https://"), "{opened:?}");
        assert!(opened[0].new_tab);
    }

    /// An available update is linked from the status bar and the Help menu.
    #[test]
    fn the_update_links_ask_to_open_the_release() {
        let release = "https://github.com/Zoephie/Baboon/releases/tag/v9.9.9";
        let mut h = Harness::new();
        h.app.shell.available_update = Some(UpdateCheckResult {
            channel: UpdateChannel::Stable,
            latest_tag: "v9.9.9".to_owned(),
            release_url: release.to_owned(),
            commit: "0123456789abcdef".to_owned(),
        });
        idle(&mut h);
        assert_eq!(urls(&click(&mut h, "Update available: v9.9.9", 0)), [release]);

        assert!(click(&mut h, "Help", 0).is_empty(), "opening the menu opens nothing");
        assert_eq!(urls(&click(&mut h, "Update available: v9.9.9...", 0)), [release]);
    }

    // Menus close when an item closes them, or on a click outside — not on every
    // click inside, which is egui 0.36's default and would shut the View menu
    // each time one of its checkboxes was ticked.

    /// Whether the View menu is showing, told by one of its items.
    fn view_menu_open(h: &Harness) -> bool {
        h.painted.iter().any(|text| text == "Expert mode")
    }

    #[test]
    fn view_menu_toggles_keep_it_open_and_an_action_closes_it() {
        let mut h = Harness::new();
        idle(&mut h);
        assert!(!view_menu_open(&h));
        h.click("View", 0);
        idle(&mut h);
        assert!(view_menu_open(&h), "the View menu opened");

        let block_sizes = h.app.model.prefs.show_block_sizes;
        h.click("Show block sizes", 0);
        idle(&mut h);
        assert_eq!(h.app.model.prefs.show_block_sizes, !block_sizes, "the checkbox toggled");
        assert!(view_menu_open(&h), "a toggle leaves the menu open");

        let expert = h.app.model.prefs.expert_mode;
        h.click("Expert mode", 0);
        idle(&mut h);
        assert_eq!(h.app.model.prefs.expert_mode, !expert);
        assert!(view_menu_open(&h), "so does a second one");

        h.click("Tag Groups", 0);
        idle(&mut h);
        assert_eq!(h.app.views[h.app.model.kits[h.app.model.active].id].browser.mode, BrowserMode::Groups);
        assert!(!view_menu_open(&h), "an action closes it");
    }

    #[test]
    fn a_click_outside_a_menu_closes_it() {
        let mut h = Harness::new();
        idle(&mut h);
        h.click("View", 0);
        idle(&mut h);
        assert!(view_menu_open(&h));
        h.click("Recent", 0);
        idle(&mut h);
        assert!(!view_menu_open(&h));
    }

    /// Outside a menu `close_menu` does nothing, as egui 0.29's `Ui::close_menu`
    /// did. egui 0.36's `Ui::close`, which the menus' former `close_menu` calls
    /// would otherwise map to, collapses the header around it instead.
    #[test]
    fn close_menu_outside_a_menu_leaves_its_container_alone() {
        let openness_after = |close: fn(&Ui)| {
            let ctx = egui::Context::default();
            let mut openness = 0.0;
            for frame in 0..4 {
                let input = egui::RawInput {
                    time: Some(f64::from(frame)),
                    ..Default::default()
                };
                let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                    openness = egui::CollapsingHeader::new("Header")
                        .default_open(true)
                        .show(ui, |ui| {
                            if frame == 1 {
                                close(ui);
                            }
                        })
                        .openness;
                });
            }
            openness
        };
        assert_eq!(openness_after(close_menu), 1.0);
        assert_eq!(openness_after(|ui| ui.close()), 0.0, "egui's close collapses it");
    }

    // The Help menu opens Help on the tab it names. The menu sends a command and
    // the window draws from Help's own state, so this crosses the whole path: a
    // click, the queue, the frame applying it, and the next frame's window.

    #[test]
    fn a_help_menu_item_opens_help_on_its_tab() {
        let mut h = Harness::new();
        idle(&mut h);
        assert!(h.app.dialogs.get::<HelpWindow>().is_none());
        assert!(!h.painted.iter().any(|text| text == "Baboon Help"));

        h.click("Help", 0);
        idle(&mut h);
        h.click("Map Names...", 0);
        idle(&mut h);
        let help = h.app.dialogs.get::<HelpWindow>().expect("Help opened");
        assert!(help.tab == HelpPanelTab::MapNames);
        assert!(h.painted.iter().any(|text| text == "Baboon Help"), "the window draws");
    }

    // An open tag's pane hands its edits on every frame, an empty set included,
    // because applying none is what ends the undo step that typing coalesces
    // into. Were the empty frames skipped, every edit after the first would fold
    // into one step that undo could only take back whole.

    fn steps(h: &mut Harness, key: &str) -> usize {
        let kit = h.app.model.active;
        let doc = h.app.model.kits[kit].parsed_tags.get_mut(key).expect("the tag is open");
        let mut count = 0;
        while doc.journal.undo(&doc.tag).is_some() {
            count += 1;
        }
        count
    }

    #[test]
    fn a_drawn_frame_with_no_edits_ends_the_undo_step() {
        let mut h = Harness::new();
        let tag = fixture::synthetic_shader(1);
        let path = "shaders/undo.shader";
        let mut entries = fixture::synthetic_entries(1, 1, 1);
        entries.push(fixture::document_entry(path, &tag));
        fixture::install_kit(&mut h.app, entries);
        let key = fixture::open_document(&mut h.app, path, tag);
        h.frame(Vec::new());

        // Two edits a frame apart, as typing makes them.
        let kit = h.app.model.active;
        let doc = h.app.model.kits[kit].parsed_tags.get_mut(&key).unwrap();
        doc.journal.begin_edit(&doc.tag, "first");
        h.frame(Vec::new());
        let doc = h.app.model.kits[kit].parsed_tags.get_mut(&key).unwrap();
        doc.journal.begin_edit(&doc.tag, "second");
        assert_eq!(steps(&mut h, &key), 2, "the frame between them ended the first");
    }

    /// The check can fail: with no frame drawn between them, the two edits are
    /// one step.
    #[test]
    fn edits_with_no_frame_between_them_are_one_step() {
        let mut h = Harness::new();
        let tag = fixture::synthetic_shader(1);
        let path = "shaders/undo.shader";
        let mut entries = fixture::synthetic_entries(1, 1, 1);
        entries.push(fixture::document_entry(path, &tag));
        fixture::install_kit(&mut h.app, entries);
        let key = fixture::open_document(&mut h.app, path, tag);
        h.frame(Vec::new());

        let kit = h.app.model.active;
        let doc = h.app.model.kits[kit].parsed_tags.get_mut(&key).unwrap();
        doc.journal.begin_edit(&doc.tag, "first");
        doc.journal.begin_edit(&doc.tag, "second");
        assert_eq!(steps(&mut h, &key), 1);
    }

    // Revealing a tag inside folders the loose browser has not loaded yet. Each
    // folder loads once the frame that drew it open is over, so a reveal can only
    // open the next folder down a frame later; it has to stay armed until it
    // reaches its tag rather than be spent on the first frame.

    #[test]
    fn a_reveal_through_unloaded_folders_reaches_its_tag() {
        let kit = LooseKit::new("lazy-reveal", "halo3_mcc");
        kit.write_mcc("objects/weapons/rifle/assault_rifle", "biped", |_| {});
        let mut h = Harness::new();
        kit.install(&mut h.app);
        for _ in 0..2 {
            h.frame(Vec::new());
        }
        let tree = &h.app.model.kits[0].source.as_ref().unwrap().tree;
        assert!(
            tree.children.iter().all(|node| !node.entries_loaded),
            "no folder is loaded before the reveal"
        );

        let key = kit.key("objects/weapons/rifle/assault_rifle.biped");
        h.app.reveal_in_browser(&key);
        // One unloaded folder a frame, three deep, plus the frames a background
        // index job may take to land and rebuild the tree under it: bounded, not
        // fixed, so a slow machine waits rather than fails.
        for _ in 0..60 {
            h.frame(Vec::new());
            if h.painted.iter().any(|text| text.contains("assault_rifle"))
                && h.app.browser.reveal_target.is_none()
            {
                break;
            }
        }
        assert!(
            h.painted.iter().any(|text| text.contains("assault_rifle")),
            "the revealed tag is drawn: {:?}",
            h.painted.iter().filter(|text| text.contains("rifle")).collect::<Vec<_>>()
        );
        assert!(h.app.browser.reveal_target.is_none(), "and the reveal is spent");
    }

    // Settings edits a draft of the preferences and sends it once drawn. A tick
    // has to reach the live preferences that way, and only the setting ticked
    // may change.

    #[test]
    fn a_settings_checkbox_changes_the_live_preference() {
        let mut h = Harness::new();
        h.app.open_settings(Some(SettingsTab::Browser));
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        let before = h.app.model.prefs.clone();
        h.click("Double-click to open tags", 0);
        for _ in 0..2 {
            h.frame(Vec::new());
        }
        let after = &h.app.model.prefs;
        assert_eq!(after.double_click_to_open_tags, !before.double_click_to_open_tags);
        let mut expected = before.clone();
        expected.double_click_to_open_tags = after.double_click_to_open_tags;
        assert!(*after == expected, "nothing else changed");
    }

    /// A draft the commit refuses comes back as its dialog, with the reason.
    #[test]
    fn a_refused_editing_kit_draft_reopens_with_its_reason() {
        let mut app = Baboon::for_test();
        app.commands.send(SettingsCommand::CommitEditingKitDraft(
            CustomEditingKitDraft::new(),
        ));
        app.apply_commands(&egui::Context::default());
        let draft = app
            .dialogs
            .get::<CustomEditingKitDraft>()
            .expect("reopened");
        assert_eq!(draft.error.as_deref(), Some("Enter an editing kit name"));
    }

    // A kit's tag tabs: pressing in a pane focuses its tag, a middle-click
    // leaves a tab alone, and the tab menu's closes reach the right tabs. Each goes through a
    // command sent while the tiles draw, so these drive whole frames.

    const PATHS: [&str; 3] = [
        "folder_00/sub_00/tag_000.biped",
        "folder_00/sub_00/tag_001.biped",
        "folder_00/sub_00/tag_002.biped",
    ];

    /// A kit with the three tags open in one tab group, `tag_002` showing.
    fn three_tabs() -> (Harness, Vec<String>) {
        let mut h = Harness::new();
        fixture::install_kit(&mut h.app, fixture::synthetic_entries(1, 1, 3));
        let keys = PATHS
            .iter()
            .map(|path| fixture::open_document(&mut h.app, path, fixture::new_tag("biped")))
            .collect();
        idle(&mut h);
        (h, keys)
    }

    /// Slide onto the first painting of `text` and press and release `button`
    /// on it, as [`Harness::click`] does with the primary button.
    fn press(h: &mut Harness, text: &str, button: egui::PointerButton) {
        let target = h
            .painted_rects
            .iter()
            .find(|(painted, _)| painted == text)
            .map(|(_, rect)| rect.center())
            .unwrap_or_else(|| panic!("{text:?} is not painted: {:?}", h.painted));
        let from = target - egui::vec2(30.0, 30.0);
        for step in 1..=3 {
            h.frame(vec![egui::Event::PointerMoved(
                from + (target - from) * step as f32 / 3.0,
            )]);
        }
        for pressed in [true, false] {
            h.frame(vec![egui::Event::PointerButton {
                pos: target,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        idle(h);
    }

    /// The kit's open tabs, sorted: they are read off the tile tree, which does
    /// not keep them in any order.
    fn open_tabs(h: &Harness) -> Vec<String> {
        let mut tabs = h.app.model.kits[h.app.model.active].open_tabs.clone();
        tabs.sort();
        tabs
    }

    /// A press inside a pane makes its tag the one the file actions act on.
    #[test]
    fn a_press_in_a_pane_focuses_its_tag() {
        let (mut h, keys) = three_tabs();
        let active = h.app.model.active;
        h.app.model.kits[active].selected_key = Some(keys[0].clone());
        press(&mut h, "runtime object type", egui::PointerButton::Primary);
        assert_eq!(
            h.app.model.kits[active].selected_key.as_ref(),
            Some(&keys[2])
        );
    }

    /// Clicking a tab makes its tag the one the file actions act on, as a
    /// press inside its pane does. It used to only bring the tab forward, so
    /// Save As on the tag in view offered another open tag's type and saved
    /// that tag instead.
    #[test]
    fn clicking_a_tab_focuses_its_tag() {
        let (mut h, keys) = three_tabs();
        let active = h.app.model.active;
        h.app.model.kits[active].selected_key = Some(keys[2].clone());
        press(&mut h, "tag_000.biped", egui::PointerButton::Primary);
        assert_eq!(
            h.app.model.kits[active].selected_key.as_ref(),
            Some(&keys[0])
        );
    }

    /// A middle-click on a tab leaves it open: tabs close only from their
    /// close button or menu.
    #[test]
    fn a_middle_click_leaves_a_tab_open() {
        let (mut h, keys) = three_tabs();
        assert_eq!(open_tabs(&h), keys);
        press(&mut h, "tag_000.biped", egui::PointerButton::Middle);
        assert_eq!(open_tabs(&h), keys);
    }

    /// The tab menu's "Close all but this" keeps the tab it was opened on.
    #[test]
    fn close_all_but_this_keeps_that_tab() {
        let (mut h, keys) = three_tabs();
        press(&mut h, "tag_001.biped", egui::PointerButton::Secondary);
        h.click("Close all but this", 0);
        idle(&mut h);
        assert_eq!(open_tabs(&h), [keys[1].clone()]);
    }

    /// Two panes showing the same tag keep their own keyword drafts. The draft
    /// used to be one field on the app, shared by every pane in every kit.
    #[test]
    fn each_pane_keeps_its_own_keyword_draft() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let mut app = Baboon::for_test();
        let mut draft_ids = Vec::new();
        let frame = |app: &mut Baboon, draft_ids: &mut Vec<egui::Id>| {
            let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    draft_ids.clear();
                    for pane in ["pane a", "pane b"] {
                        ui.push_id(pane, |ui| {
                            let egui = ui.ctx().clone();
                            draw_keyword_bar(&cx!(app, &egui), ui, 0, "file:crate.model");
                            draft_ids
                                .push(ui.make_persistent_id(("keyword_input", "file:crate.model")));
                        });
                    }
                });
            });
        };

        frame(&mut app, &mut draft_ids);
        ctx.data_mut(|data| data.insert_temp(draft_ids[0], "rocket".to_owned()));
        frame(&mut app, &mut draft_ids);

        let draft = |id: egui::Id| {
            ctx.data_mut(|data| data.get_temp::<String>(id))
                .unwrap_or_default()
        };
        assert_eq!(draft(draft_ids[0]), "rocket");
        assert_eq!(draft(draft_ids[1]), "", "the other pane's box is untouched");
    }

    /// An open tag that nobody touches does not keep the window drawing. The
    /// pane and the tile tree send a command every frame; applying one that
    /// changed nothing used to ask for the next frame, which sent them again,
    /// keeping a core busy for as long as any tag was open.
    #[test]
    fn an_open_tag_left_alone_lets_the_window_sleep() {
        let kit = LooseKit::new("idle-repaint", "haloce_mcc");
        kit.write_classic_ce("weapons/rifle", "weapon");
        let mut app = app();
        kit.install(&mut app);
        let key = kit.open(&mut app, "weapons/rifle.weapon");
        assert!(
            app.views[app.model.kits[0].id].tag_tree.tiles.iter().any(
                |(_, tile)| matches!(tile, egui_tiles::Tile::Pane(pane) if *pane == key)
            ),
            "the tag is laid out as a pane, so its frames send the commands"
        );

        let idle = repaint_delay_after(&mut app, Baboon::run_frame, |_| {});
        assert!(
            idle > Duration::from_millis(100),
            "an idle open tag repaints every {idle:?}"
        );

        // The same measurement sees a frame that did change something.
        let changed = repaint_delay_after(&mut app, Baboon::run_frame, |app| {
            app.commands.send(crate::app::context::Command::Status("Changed".to_owned()));
        });
        assert_eq!(changed, Duration::ZERO, "a change applied after drawing is drawn");
    }

    /// Rows out of view are not built, and what shows is what drawing every
    /// row shows, scrolled anywhere. Painted text alone can't tell: egui
    /// skips painting text out of view by itself, so rows built are counted.
    #[test]
    fn rows_out_of_view_are_skipped_and_nothing_in_view_changes() {
        use crate::app::editor::{FIELD_ROWS_BUILT, ROW_CULLING_OFF};
        let kit = LooseKit::new("row-culling", "haloce_mcc");
        kit.write_classic_ce("weapons/b", "weapon");
        let run = |culling: bool| {
            ROW_CULLING_OFF.with(|off| off.set(!culling));
            let mut h = Harness::new();
            kit.install(&mut h.app);
            kit.open(&mut h.app, "weapons/b.weapon");
            settle(&mut h, 8);
            let mut views = Vec::new();
            let mut built = Vec::new();
            for _ in 0..6 {
                FIELD_ROWS_BUILT.with(|rows| rows.set(0));
                h.frame(vec![pointer_at(PANE_POINT), wheel(-600.0)]);
                settle(&mut h, 2);
                FIELD_ROWS_BUILT.with(|rows| rows.set(0));
                h.frame(Vec::new());
                built.push(FIELD_ROWS_BUILT.with(std::cell::Cell::get));
                // What the field area shows: below the pane's tabs and header,
                // above its bottom edge. Rows scrolled above or below it are
                // laid out without culling but clipped, so not seen.
                let mut shown: Vec<String> = h
                    .painted_rects
                    .iter()
                    .filter(|(_, rect)| rect.top() > 200.0 && rect.bottom() < SCREEN.y * 0.8)
                    .map(|(text, rect)| format!("{text}@{:.0},{:.0}", rect.left(), rect.top()))
                    .collect();
                shown.sort();
                views.push(shown);
            }
            ROW_CULLING_OFF.with(|off| off.set(false));
            (views, built)
        };
        let (culled, culled_built) = run(true);
        let (all, all_built) = run(false);
        assert_eq!(culled, all, "the same rows show at every scroll position");
        for (culled, all) in culled_built.iter().zip(&all_built) {
            assert!(culled * 2 < *all, "built {culled} rows of {all}");
        }
    }

    /// A pasted reference is written `path.extension`, and in a Halo CE kit
    /// `.shader` is `shdr`: the paste must not take another game's `shader`.
    #[test]
    fn a_pasted_reference_takes_its_group_from_the_kits_game() {
        let kit = LooseKit::new("tsv-reference", "haloce_mcc");
        kit.write_classic_ce("weapons/b", "weapon");
        let mut h = Harness::new();
        kit.install(&mut h.app);
        let key = kit.open(&mut h.app, "weapons/b.weapon");
        settle(&mut h, 2);
        let active = h.app.model.active;
        let block = "item/object/attachments";
        crate::core::document::apply::add_block_element(
            &mut h.app.model.kits[active]
                .parsed_tags
                .get_mut(&key)
                .unwrap()
                .tag,
            block,
        )
        .unwrap();
        h.app.dialogs.open(TsvPasteState {
            kit: h.app.model.kits[active].id,
            tag_key: key.clone(),
            block_path: block.to_owned(),
            block_label: "attachments".to_owned(),
            element_count: 1,
            text: "type\neffects\\glow.shader\n".to_owned(),
            status: None,
        });
        h.app.apply_tsv_paste();
        let doc = &h.app.model.kits[active].parsed_tags[&key];
        let value = doc
            .tag
            .root()
            .field_path(&format!("{block}[0]/type"))
            .and_then(|field| field.value());
        let Some(TagFieldData::TagReference(reference)) = value else {
            panic!("no reference at {block}[0]/type: {value:?}");
        };
        assert_eq!(
            reference.group_tag_and_name,
            Some((u32::from_be_bytes(*b"shdr"), r"effects\glow".to_owned()))
        );
    }

    /// A Halo CE kit with two weapons open, "b" the tab shown, settled.
    struct TypedEdit {
        h: Harness,
        kit: LooseKit,
        key: String,
    }

    /// Where a field row's text box sits: right of its label.
    fn field_box(h: &Harness, label: &str) -> egui::Pos2 {
        let rect = h
            .painted_rects
            .iter()
            .find(|(text, _)| text == label)
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("{label:?} is not painted"));
        egui::pos2(rect.left() + 310.0, rect.center().y)
    }

    fn painted_at(h: &Harness, label: &str) -> egui::Pos2 {
        h.painted_rects
            .iter()
            .find(|(text, _)| text == label)
            .map(|(_, rect)| rect.center())
            .unwrap_or_else(|| panic!("{label:?} is not painted"))
    }

    /// Slide onto `target` over three frames, then press and release there.
    fn click_point(h: &mut Harness, target: egui::Pos2) {
        let from = target - egui::vec2(30.0, 30.0);
        for step in 1..=3 {
            h.frame(vec![egui::Event::PointerMoved(from + (target - from) * step as f32 / 3.0)]);
        }
        for pressed in [true, false] {
            h.frame(vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
    }

    fn press_key(h: &mut Harness, key: egui::Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            h.frame(vec![egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }]);
        }
    }

    fn settle(h: &mut Harness, frames: usize) {
        for _ in 0..frames {
            h.frame(Vec::new());
        }
    }

    const CTRL: egui::Modifiers = egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE };
    const RADIUS: &str = "item/object/bounding radius#2";

    impl TypedEdit {
        /// Type `5` into "bounding radius" and leave the box focused.
        fn new(name: &str) -> Self {
            let kit = LooseKit::new(name, "haloce_mcc");
            kit.write_classic_ce("weapons/a", "weapon");
            kit.write_classic_ce("weapons/b", "weapon");
            let mut h = Harness::new();
            kit.install(&mut h.app);
            kit.open(&mut h.app, "weapons/a.weapon");
            let key = kit.open(&mut h.app, "weapons/b.weapon");
            settle(&mut h, 8);
            let at = field_box(&h, "bounding radius");
            click_point(&mut h, at);
            settle(&mut h, 1);
            h.frame(vec![egui::Event::Text("5".to_owned())]);
            settle(&mut h, 1);
            let edit = Self { h, kit, key };
            assert_eq!(edit.radius(), Some(0.0), "typing alone commits nothing");
            edit
        }

        fn doc(&self) -> Option<&crate::core::document::TagDocument> {
            self.h.app.model.kits[0].parsed_tags.get(&self.key)
        }

        fn radius(&self) -> Option<f32> {
            real_of(&self.doc()?.tag, RADIUS)
        }

        fn dirty(&self) -> bool {
            self.doc().is_some_and(|doc| doc.dirty.is_set())
        }

        /// The radius the file on disk holds.
        fn saved_radius(&self) -> Option<f32> {
            let bytes = std::fs::read(self.kit.root.join("weapons/b.weapon")).unwrap();
            let tag = crate::core::source::read_tag_from_bytes(
                &bytes,
                GameId::from_id("haloce_mcc"),
                Some(&locate_definitions_root()),
                group_tag("haloce_mcc", "weapon"),
            )
            .unwrap();
            real_of(&tag, RADIUS)
        }
    }

    /// Ctrl+S while still typing saves what was typed. The shortcut used to
    /// give up the field's focus only after the panes had drawn, and the save
    /// ran before the next draw, so the file got the old value and the edit
    /// landed a frame later, leaving the tag modified.
    #[test]
    fn ctrl_s_while_typing_saves_the_typed_value() {
        let mut edit = TypedEdit::new("edit-ctrl-s");
        press_key(&mut edit.h, egui::Key::S, CTRL);
        settle(&mut edit.h, 4);
        assert_eq!(edit.saved_radius(), Some(5.0), "the file holds the typed value");
        assert!(!edit.dirty(), "and the tag is saved");
    }

    /// Ctrl+W while still typing asks to save the typed value rather than
    /// closing the tab and dropping it.
    #[test]
    fn ctrl_w_while_typing_asks_to_save_the_typed_value() {
        let mut edit = TypedEdit::new("edit-ctrl-w");
        press_key(&mut edit.h, egui::Key::W, CTRL);
        settle(&mut edit.h, 4);
        assert!(edit.h.app.dialogs.get::<SaveChangesPrompt>().is_some(), "the close asks first");
        assert_eq!(edit.radius(), Some(5.0));
        assert!(edit.dirty());
    }

    /// Collapsing the section of a field being typed into commits it. The
    /// box stops being drawn on that click, so it never saw itself lose focus,
    /// and kept the typed text to itself while the tag stayed unchanged.
    #[test]
    fn collapsing_the_section_of_a_typed_field_commits_it() {
        let mut edit = TypedEdit::new("edit-collapse");
        let header = painted_at(&edit.h, "OBJECT_BLOCK_STRUCT");
        click_point(&mut edit.h, header);
        settle(&mut edit.h, 3);
        assert!(!edit.h.painted.iter().any(|text| text == "bounding radius"), "the section is collapsed");
        assert_eq!(edit.radius(), Some(5.0));
        assert!(edit.dirty());
    }

    /// The same for switching the pane to its Model Preview sub-tab.
    #[test]
    fn switching_sub_tab_commits_a_typed_field() {
        let mut edit = TypedEdit::new("edit-sub-tab");
        let tab = painted_at(&edit.h, "Model Preview");
        click_point(&mut edit.h, tab);
        settle(&mut edit.h, 3);
        assert_eq!(edit.radius(), Some(5.0));
        assert!(edit.dirty());
    }

    /// Closing the window while it is minimized asks about a field still
    /// being typed in. eframe runs no UI pass for a minimized window, so the
    /// field never committed, the tags looked saved, and the app quit.
    #[test]
    fn closing_while_minimized_asks_about_a_typed_field() {
        let mut edit = TypedEdit::new("edit-minimized");
        let mut commands = Vec::new();
        for (frame, close) in [true, false, false].into_iter().enumerate() {
            let mut input = screen(Vec::new(), 500.0 + frame as f64 / 60.0);
            let viewport = input.viewports.entry(egui::ViewportId::ROOT).or_default();
            viewport.minimized = Some(true);
            if close {
                viewport.events.push(egui::ViewportEvent::Close);
            }
            let app = &mut edit.h.app;
            let output = edit.h.ctx.run_logic(&input, |ctx| app.run_logic(ctx));
            if let Some(sent) = output.viewport_commands.get(&egui::ViewportId::ROOT) {
                commands.extend(sent.iter().cloned());
            }
        }
        assert!(!commands.contains(&egui::ViewportCommand::Close), "the app quit: {commands:?}");
        assert!(edit.h.app.dialogs.get::<SaveChangesPrompt>().is_some(), "it asks first");
        assert_eq!(edit.radius(), Some(5.0));
    }

    /// Escape in a field puts the tag's value back and commits nothing.
    #[test]
    fn escape_restores_the_original_value() {
        let mut edit = TypedEdit::new("edit-escape");
        press_key(&mut edit.h, egui::Key::Escape, egui::Modifiers::NONE);
        settle(&mut edit.h, 3);
        assert_eq!(edit.radius(), Some(0.0));
        assert!(!edit.dirty());
        let row = painted_at(&edit.h, "bounding radius");
        assert!(
            !edit.h.painted_rects.iter().any(|(text, rect)| text == "05" && (rect.center().y - row.y).abs() < 6.0),
            "the box shows the tag's value again"
        );
        // Enter still commits.
        let at = field_box(&edit.h, "bounding radius");
        click_point(&mut edit.h, at);
        edit.h.frame(vec![egui::Event::Text("7".to_owned())]);
        press_key(&mut edit.h, egui::Key::Enter, egui::Modifiers::NONE);
        settle(&mut edit.h, 3);
        assert_eq!(edit.radius(), Some(7.0), "the box held 0 again, so it reads 07");
    }

    /// An undo shows through a multi-part row after its edit committed. The
    /// row's boxes kept the typed text (`05`, shown back as `5`), so the undo
    /// was hidden and clicking in and out of the box applied it again.
    #[test]
    fn an_undo_shows_through_a_committed_component_row() {
        let mut edit = TypedEdit::new("edit-components");
        press_key(&mut edit.h, egui::Key::Escape, egui::Modifiers::NONE);
        settle(&mut edit.h, 2);
        // The first component's box, just right of where a one-box row's
        // box starts.
        let x_box = field_box(&edit.h, "bounding offset") + egui::vec2(10.0, 0.0);
        click_point(&mut edit.h, x_box);
        edit.h.frame(vec![egui::Event::Text("5".to_owned())]);
        press_key(&mut edit.h, egui::Key::Enter, egui::Modifiers::NONE);
        settle(&mut edit.h, 2);
        let offset = |edit: &TypedEdit| {
            edit.doc().and_then(|doc| doc.tag.root().field_path("item/object/bounding offset#2")?.value()).map(|value| format!("{value:?}"))
        };
        let edited = offset(&edit);
        assert!(edit.dirty(), "the component edit committed: {edited:?}");
        press_key(&mut edit.h, egui::Key::Z, CTRL);
        settle(&mut edit.h, 2);
        let undone = offset(&edit);
        assert_ne!(undone, edited, "the undo reverted it");
        // In and out of the box without typing re-applies nothing.
        click_point(&mut edit.h, x_box);
        click_point(&mut edit.h, egui::pos2(1400.0, 900.0));
        settle(&mut edit.h, 2);
        assert_eq!(offset(&edit), undone, "the undo stays undone");
    }
}
