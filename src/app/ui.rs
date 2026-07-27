//! Top-level windows, menus, dialogs, and frame composition for [`Baboon`].
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::controller::open_terminal_log;
use super::*;

mod browser_panel;
mod dialogs;
mod find;
mod first_run;
mod help;
mod search_windows;
mod recents;
mod settings;
mod shell;
mod kit_tiles;
mod tag_pane;
mod welcome;
mod tag_tiles;
mod tool_commands;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LauncherButtonVisual {
    Normal,
    Muted,
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
    launcher_button_with_visual(ui, icon, fallback, enabled, LauncherButtonVisual::Normal)
}

fn launcher_button_with_visual(
    ui: &mut Ui,
    icon: Option<&egui::TextureHandle>,
    fallback: &str,
    enabled: bool,
    visual: LauncherButtonVisual,
) -> egui::Response {
    let tint = match visual {
        LauncherButtonVisual::Normal => Color32::WHITE,
        LauncherButtonVisual::Muted => ui.visuals().weak_text_color(),
    };

    match icon {
        Some(texture) => ui.add_enabled(
            enabled,
            egui::ImageButton::new(
                egui::Image::new(egui::load::SizedTexture::new(
                    texture.id(),
                    Vec2::splat(20.0),
                ))
                .tint(tint),
            ),
        ),
        None => ui.add_enabled(
            enabled,
            egui::Button::new(RichText::new(fallback).color(tint)).min_size(Vec2::splat(22.0)),
        ),
    }
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
fn draw_game_banner_header(ui: &mut Ui, app: &mut Baboon, game: &str, path_label: &str) {
    const EMBLEM: f32 = 72.0;
    const MARGIN: f32 = 8.0;
    const GAP: f32 = 4.0;
    const TITLE_TOP: f32 = 8.0;

    let texture = app.game_banner_texture(ui.ctx(), game).cloned();
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

fn ek_game_label(game: &str) -> &str {
    SUPPORTED_EK_GAMES
        .iter()
        .find_map(|(label, id)| (*id == game).then_some(*label))
        .unwrap_or(game)
}

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
        ui.horizontal(|ui| {
            ui.label(RichText::new("Open scenario in:").color(subtle_dark()));
            let sapien_ready = self.can_launch_scenario_in_sapien(kit_index, entry);
            if launcher_button(ui, self.sapien_icon.as_ref(), "S", sapien_ready)
                .on_hover_text("Save if needed, then launch this scenario in Sapien")
                .clicked()
            {
                self.active = kit_index;
                self.launch_scenario_in_sapien(&key);
            }
            let tag_test_ready = self.can_launch_scenario_in_tag_test(kit_index, entry);
            if launcher_button(ui, self.tag_test_icon.as_ref(), "T", tag_test_ready)
                .on_hover_text("Save if needed, then launch this scenario in tag_test")
                .clicked()
            {
                self.active = kit_index;
                self.launch_scenario_in_tag_test(&key);
            }
        });
        ui.add_space(4.0);
    }

    /// "Search fields" bar (Guerilla-style): typing a block or field name
    /// collapses the editor to just the matching node(s) and their ancestors.
    pub(super) fn draw_field_search_bar(&mut self, ui: &mut Ui, kit_index: usize, tag_key: &str) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Search fields:").color(text_dark()));
            let query = self.kits[kit_index]
                .field_search
                .entry(tag_key.to_owned())
                .or_default();
            ui.add(
                egui::TextEdit::singleline(query)
                    .hint_text("block or field name")
                    .desired_width(220.0),
            );
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
    }

    fn draw_tool_launcher_buttons(&mut self, ui: &mut Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if launcher_button(ui, self.blender_icon.as_ref(), "B", true)
                .on_hover_text("Launch Blender")
                .clicked()
            {
                self.launch_blender();
            }

            self.draw_monitor_menu_button(ui);

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
                ui.separator();
            }
            self.draw_editing_kit_shortcut_buttons(ui);
        });
    }

    fn draw_editing_kit_shortcut_buttons(&mut self, ui: &mut Ui) {
        for shortcut in EDITING_KIT_SHORTCUTS.into_iter().rev() {
            let texture = self.game_emblem_texture(ui.ctx(), shortcut.game).cloned();
            let configured_path = self.editing_kit_paths.get(shortcut.game);
            let tooltip = configured_path
                .map(|path| format!("Load {} from {}", shortcut.label, path.display()))
                .unwrap_or_else(|| format!("Set {} path in Settings", shortcut.label));
            let visual = if configured_path.is_some() {
                LauncherButtonVisual::Normal
            } else {
                LauncherButtonVisual::Muted
            };
            if launcher_button_with_visual(ui, texture.as_ref(), shortcut.fallback, true, visual)
                .on_hover_text(tooltip)
                .clicked()
            {
                self.load_editing_kit_shortcut(shortcut, ui.ctx().clone());
            }
        }
    }

    fn draw_monitor_menu_button(&mut self, ui: &mut Ui) {
        let game = self.source()
            .and_then(|source| source.game.as_deref());
        let commands = monitor_commands_for_game(game);
        if commands.is_empty() {
            launcher_button(ui, self.monitor_icon.as_ref(), "M", false)
                .on_hover_text("No monitor commands available for this game");
            return;
        }

        let ctx = ui.ctx().clone();
        let monitor_texture = self.monitor_icon.as_ref().map(|texture| texture.id());
        let add_commands = |ui: &mut Ui| {
            ui.set_min_width(210.0);
            for command in commands {
                if ui.button(*command).clicked() {
                    self.submit_terminal_command(format!("tool {command}"), ctx.clone());
                    ui.close_menu();
                }
            }
        };
        if let Some(texture_id) = monitor_texture {
            ui.menu_image_button(
                egui::load::SizedTexture::new(texture_id, Vec2::splat(20.0)),
                add_commands,
            )
            .response
            .on_hover_text("Run monitor command");
        } else {
            ui.menu_button("M", add_commands)
                .response
                .on_hover_text("Run monitor command");
        }
    }

    /// Per-tag keyword chips (add via Enter/Add, remove via the chip button).
    /// Keywords live in an external sidecar, not the tag binary.
    fn draw_keyword_bar(&mut self, ui: &mut Ui, kit_index: usize, tag_key: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Keywords:").color(subtle_dark()));
            let existing = self.kits[kit_index].keywords.keywords(tag_key).to_vec();
            let mut remove: Option<String> = None;
            for keyword in &existing {
                if ui
                    .small_button(format!("{keyword}  ×"))
                    .on_hover_text("Remove keyword")
                    .clicked()
                {
                    remove = Some(keyword.clone());
                }
            }
            if let Some(keyword) = remove {
                self.kits[kit_index].keywords.remove(tag_key, &keyword);
            }
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.keyword_input)
                    .hint_text("add keyword")
                    .desired_width(120.0),
            );
            let submitted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Add").clicked() || submitted) && !self.keyword_input.trim().is_empty() {
                self.kits[kit_index].keywords.add(tag_key, &self.keyword_input);
                self.keyword_input.clear();
            }
        });
        ui.add_space(4.0);
    }
}

impl eframe::App for Baboon {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.draw_root_ui(ctx, frame);
        self.run_deferred_file_action(ctx);
        self.maybe_autosave_campaign_projects(ctx);
    }
}
