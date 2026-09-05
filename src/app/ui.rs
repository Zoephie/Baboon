//! Top-level windows, menus, dialogs, and frame composition for [`Baboon`].
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::controller::open_terminal_log;
use super::*;

mod browser_panel;
mod dialogs;
mod find;
mod first_run;
pub(super) mod help;
mod search_windows;
mod recents;
mod settings;
mod shell;
mod kit_tiles;
mod tag_pane;
mod welcome;
mod tag_tiles;
mod tool_commands;
mod blam;

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
            content.right() - EDITING_KIT_MENU_ICON_SIZE * 0.5,
            content.center().y,
        ),
        Vec2::splat(EDITING_KIT_MENU_ICON_SIZE),
    );
    let label_rect = egui::Rect::from_min_max(
        content.min,
        egui::pos2(
            icon_rect.left() - EDITING_KIT_MENU_ICON_GAP,
            content.max.y,
        ),
    );
    EditingKitMenuRowLayout {
        label_rect,
        icon_rect,
    }
}

fn editing_kit_menu_row(
    ui: &mut Ui,
    label: &str,
    fallback: &str,
    texture: Option<&egui::TextureHandle>,
    default_project_icon: bool,
    enabled: bool,
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
    ui.painter()
        .with_clip_rect(layout.label_rect)
        .text(
            layout.label_rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Button.resolve(ui.style()),
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
        ui.painter()
            .with_clip_rect(layout.icon_rect)
            .text(
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
/// Sized to its own text and nothing else. The path used to wrap, and a
/// wrapping label claims the full available width, so the card grew, shrank,
/// and reflowed as the sidebar was dragged.
///
/// It also has to stay out of the panel's minimum width, or the sidebar would
/// refuse to shrink past whichever game had the longest path. A panel is at
/// least as wide as its content's minimum, so the card *allocates* only what
/// is available and *draws* at its natural size into a clipped painter: the
/// layout never sees the overflow, and the card keeps one shape.
fn draw_game_banner_header(
    ui: &mut Ui,
    app: &mut Baboon,
    game: &str,
    path_label: &str,
    profile_id: Option<&str>,
) {
    const EMBLEM: f32 = 72.0;
    const MARGIN: f32 = 8.0;
    const GAP: f32 = 4.0;
    const TITLE_TOP: f32 = 8.0;

    let texture = app.workspace_banner_texture(ui.ctx(), game, profile_id);
    let title = egui::WidgetText::from(
        RichText::new(format!(
            "Tags - {} ({})",
            game_display_name(game),
            game_platform_label(game)
        ))
        .color(text_dark())
        .strong(),
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Body,
    );
    let path = egui::WidgetText::from(RichText::new(path_label).color(subtle_dark()).small())
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            TextStyle::Small,
        );

    let text_width = title.size().x.max(path.size().x);
    let card = Vec2::new(
        MARGIN * 2.0 + EMBLEM + GAP + text_width,
        MARGIN * 2.0 + EMBLEM,
    );
    let (visible, _) = ui.allocate_exact_size(
        Vec2::new(card.x.min(ui.available_width()), card.y),
        Sense::hover(),
    );
    let full = egui::Rect::from_min_size(visible.min, card);

    let painter = ui.painter_at(visible);
    painter.rect_filled(
        full,
        0.0,
        if is_dark_mode() {
            Color32::from_rgb(43, 43, 41)
        } else {
            Color32::from_rgb(235, 235, 230)
        },
    );
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            egui::Rect::from_min_size(
                full.min + Vec2::splat(MARGIN),
                Vec2::splat(EMBLEM),
            ),
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

    /// "Search fields" bar (Guerilla-style): typing a block or field name
    /// collapses the editor to just the matching node(s) and their ancestors.
    pub(super) fn draw_field_search_bar(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        tag_key: &str,
        jump_match_count: usize,
    ) -> bool {
        let mut jump = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Search fields:").color(text_dark()));
            let query = self.kits[kit_index]
                .field_search
                .entry(tag_key.to_owned())
                .or_default();
            let response = foundation_header_text_edit(
                ui,
                egui::TextEdit::singleline(query)
                    .hint_text(placeholder_text("block or field name"))
                    .desired_width(220.0),
            );
            let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
            let submitted = enter && (response.has_focus() || response.lost_focus());
            let jump_tooltip = match jump_match_count {
                0 => "No matching field or block".to_owned(),
                1 => "Jump to match (Enter)".to_owned(),
                count => format!("Jump to next match (Enter)\n{count} matches"),
            };
            let jump_response = icon_button(
                ui,
                ButtonIcon::JumpTo,
                &jump_tooltip,
                jump_match_count > 0,
                Vec2::new(22.0, 22.0),
                text_dark(),
            );
            // Re-resolve matches after drawing when Enter is used: the TextEdit
            // may have changed its query during this same frame.
            jump = submitted || jump_response.clicked();
            if jump {
                // A single-line edit surrenders focus on Enter. Take it back so
                // another Enter advances again without an intervening click.
                response.request_focus();
            }
            if icon_button(
                ui,
                ButtonIcon::Clear,
                "Clear search",
                true,
                Vec2::new(22.0, 22.0),
                text_dark(),
            )
            .clicked()
            {
                query.clear();
            }
            ui.label(
                RichText::new("shows only matches and the blocks/structs that contain them")
                    .color(subtle_dark())
                    .small(),
            );
        });
        ui.add_space(4.0);
        jump
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
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                ui.menu_button("Monitor", |ui| {
                    ui.set_min_width(210.0);
                    for command in commands {
                        if ui.button(*command).clicked() {
                            self.submit_terminal_command(format!("tool {command}"), ctx.clone());
                            ui.close_menu();
                        }
                    }
                })
                .response
            })
            .inner;
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
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                ui.menu_button("Assets", |ui| {
                    ui.set_min_width(210.0);
                    if ui.button("Bitmap Browser").clicked() {
                        self.open_bitmap_library();
                        ui.close_menu();
                    }
                    if ui.button("Model Browser").clicked() {
                        self.open_model_library();
                        ui.close_menu();
                    }
                    // Baboon's own import pipelines only cover Halo 3 so far,
                    // so the entry only appears there.
                    if self.active_kit_is_halo3() && ui.button("Blam!").clicked() {
                        // Re-detect on every open: the data folder may have
                        // changed since the pane was last shown.
                        self.kits[self.active].blam.scanned_path = None;
                        self.kits[self.active].open_tag_pane(BLAM_KEY);
                        ui.close_menu();
                    }
                })
                .response
            })
            .inner;
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
            let (resp, add_clicked) = Frame::none()
                .fill(foundation_input())
                .rounding(egui::Rounding::same(BUTTON_HEIGHT / 2.0))
                .inner_margin(egui::Margin::same(2.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.spacing_mut().interact_size.y = 20.0;
                    ui.set_height(20.0);
                    ui.horizontal(|ui| {
                        let resp = foundation_header_text_edit(
                            ui,
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
                })
                .inner;
            let submitted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (add_clicked || submitted) && !self.keyword_input.trim().is_empty() {
                self.kits[kit_index].keywords.add(tag_key, &self.keyword_input);
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
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(width, BUTTON_HEIGHT),
        Sense::hover(),
    );
    let background = editor_bg();
    let target = if is_dark_mode() {
        Color32::WHITE
    } else {
        Color32::BLACK
    };
    let blend = |base: u8, overlay: u8| {
        (base as f32 + (overlay as f32 - base as f32) * 0.05).round() as u8
    };
    let fill = Color32::from_rgb(
        blend(background.r(), target.r()),
        blend(background.g(), target.g()),
        blend(background.b(), target.b()),
    );
    ui.painter().rect_filled(
        rect,
        egui::Rounding::same(BUTTON_HEIGHT / 2.0),
        fill,
    );
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
