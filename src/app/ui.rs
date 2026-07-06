use super::controller::open_terminal_log;
use super::*;

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

fn draw_game_banner_header(ui: &mut Ui, app: &mut Baboon, game: &str, path_label: &str) {
    let texture = app.game_banner_texture(ui.ctx(), game).cloned();
    Frame::none()
        .fill(if is_dark_mode() {
            Color32::from_rgb(43, 43, 41)
        } else {
            Color32::from_rgb(235, 235, 230)
        })
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(texture) = texture {
                    ui.add(
                        egui::Image::new(egui::load::SizedTexture::new(
                            texture.id(),
                            Vec2::splat(72.0),
                        ))
                        .fit_to_exact_size(Vec2::splat(72.0)),
                    );
                }
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("Tags - {} (MCC)", game_display_name(game)))
                            .color(text_dark())
                            .strong(),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(path_label).color(subtle_dark()).small())
                            .wrap(),
                    );
                });
            });
        });
}

fn sidebar_source_path_label(source: &TagSource) -> String {
    match source {
        TagSource::SingleFile { path } => path.display().to_string(),
        TagSource::LooseFolder { root, .. } => root.display().to_string(),
        TagSource::MonolithicCache { root, .. } => root.display().to_string(),
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
mod tests {
    use super::*;

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
            monitor_commands_for_game(Some("halo2_mcc")),
            &[
                "monitor-bitmaps",
                "monitor-bitmaps-data-and-tags",
                "monitor-models",
                "monitor-structures",
            ]
        );
        assert_eq!(
            monitor_commands_for_game(Some("halo4_mcc")),
            &["monitor-bitmaps", "monitor-strings"]
        );
        assert!(monitor_commands_for_game(Some("haloce_mcc")).is_empty());
        assert!(monitor_commands_for_game(None).is_empty());
    }
}

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

fn tab_label_width(ui: &Ui, label: &str, min_width: f32, max_width: f32) -> f32 {
    let width = label.chars().count() as f32 * 7.0 + ui.spacing().button_padding.x * 2.0;
    width.clamp(min_width, max_width)
}

impl Baboon {
    /// "Search fields" bar (Guerilla-style): typing a block or field name
    /// collapses the editor to just the matching node(s) and their ancestors.
    pub(super) fn draw_field_search_bar(&mut self, ui: &mut Ui, tag_key: &str) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Search fields:").color(text_dark()));
            let query = self.field_search.entry(tag_key.to_owned()).or_default();
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
                .on_hover_text("Launch tag_test")
                .clicked()
            {
                self.launch_tag_test();
            }

            let sapien_ready = self
                .kit_tool_path("sapien.exe")
                .is_some_and(|path| path.is_file());
            if launcher_button(ui, self.sapien_icon.as_ref(), "S", sapien_ready)
                .on_hover_text("Launch Sapien")
                .clicked()
            {
                self.launch_sapien();
            }

            ui.separator();
            self.draw_editing_kit_shortcut_buttons(ui);
        });
    }

    fn draw_editing_kit_shortcut_buttons(&mut self, ui: &mut Ui) {
        for shortcut in EDITING_KIT_SHORTCUTS.into_iter().rev() {
            let texture = self.game_banner_texture(ui.ctx(), shortcut.game).cloned();
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
        let game = self
            .source
            .as_ref()
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
    fn draw_keyword_bar(&mut self, ui: &mut Ui, tag_key: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Keywords:").color(subtle_dark()));
            let existing = self.keywords.keywords(tag_key).to_vec();
            let mut remove: Option<String> = None;
            for keyword in &existing {
                if ui
                    .small_button(format!("{keyword}  ✕"))
                    .on_hover_text("Remove keyword")
                    .clicked()
                {
                    remove = Some(keyword.clone());
                }
            }
            if let Some(keyword) = remove {
                self.keywords.remove(tag_key, &keyword);
            }
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.keyword_input)
                    .hint_text("add keyword")
                    .desired_width(120.0),
            );
            let submitted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Add").clicked() || submitted) && !self.keyword_input.trim().is_empty() {
                self.keywords.add(tag_key, &self.keyword_input);
                self.keyword_input.clear();
            }
        });
        ui.add_space(4.0);
    }

    /// Small query window for the background field-value search. Results land in
    /// the shared results window via `FieldValueSearchFinished`.
    /// Rename / move a tag with reference fix-up. The referrer list is the
    /// preview; "Apply" moves the file and rewrites every referencing tag.
    fn draw_rename_tag_window(&mut self, ctx: &egui::Context) {
        if self.rename_tag.is_none() {
            return;
        }
        let mut open = true;
        let mut do_apply = false;
        let mut cancel = false;
        {
            let state = self.rename_tag.as_mut().expect("checked above");
            egui::Window::new("Rename / Move Tag")
                .id(egui::Id::new("rename_tag"))
                .open(&mut open)
                .default_width(560.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Current path").color(subtle_dark()).small());
                    ui.label(
                        RichText::new(&state.old_display)
                            .color(text_dark())
                            .monospace(),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("New path (relative, no extension)")
                            .color(subtle_dark())
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.new_path_input)
                                .desired_width(430.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        ui.label(
                            RichText::new(format!(".{}", state.extension)).color(subtle_dark()),
                        );
                    });
                    ui.add_space(8.0);
                    if state.referrers_unavailable {
                        ui.label(
                            RichText::new(
                                "Reference index unavailable — references are still rewritten on \
                                 apply, but can't be previewed here.",
                            )
                            .color(subtle_dark()),
                        );
                    } else if state.referrers.is_empty() {
                        ui.label(
                            RichText::new("No other tags reference this tag.").color(subtle_dark()),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "{} referring tag(s) will be updated:",
                                state.referrers.len()
                            ))
                            .color(text_dark()),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("rename_referrers")
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for referrer in &state.referrers {
                                    ui.label(RichText::new(referrer).color(subtle_dark()).small());
                                }
                            });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !state.new_path_input.trim().is_empty(),
                                egui::Button::new("Apply"),
                            )
                            .on_hover_text("Move the file on disk and rewrite all references")
                            .clicked()
                        {
                            do_apply = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
        }
        if do_apply {
            // begin_rename_tag clears `rename_tag` on success; on a validation
            // error it leaves the dialog open with a status message.
            self.begin_rename_tag();
        }
        if cancel || !open {
            self.rename_tag = None;
        }
    }

    /// TSV import window: the user pastes tab-separated rows (header = field
    /// names) and applies them onto the target block's existing elements.
    fn draw_tsv_paste_window(&mut self, ctx: &egui::Context) {
        if self.tsv_paste.is_none() {
            return;
        }
        let mut open = true;
        let mut do_apply = false;
        {
            let paste = self.tsv_paste.as_mut().expect("checked above");
            egui::Window::new(format!("Paste TSV → {}", paste.block_label))
                .id(egui::Id::new("tsv_paste"))
                .open(&mut open)
                .default_width(560.0)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Paste tab-separated rows (first row = field names) to overwrite \
                             this block's {} element(s), cell by cell. Extra rows are ignored — \
                             add elements first if you need more.",
                            paste.element_count
                        ))
                        .color(subtle_dark()),
                    );
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut paste.text)
                                    .desired_rows(12)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Monospace)
                                    .hint_text("paste TSV here (Ctrl+V)"),
                            );
                        });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!paste.text.trim().is_empty(), egui::Button::new("Apply"))
                            .clicked()
                        {
                            do_apply = true;
                        }
                        if let Some(status) = &paste.status {
                            ui.label(RichText::new(status).color(subtle_dark()));
                        }
                    });
                });
        }
        if do_apply {
            self.apply_tsv_paste();
        }
        if !open {
            self.tsv_paste = None;
        }
    }

    fn draw_field_value_search_window(&mut self, ctx: &egui::Context) {
        if !self.field_value_search_open {
            return;
        }
        let mut open = true;
        let mut do_search = false;
        let mut do_build = false;
        egui::Window::new("Search Field Values")
            .id(egui::Id::new("field_value_search"))
            .open(&mut open)
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Find tags whose field values contain text — strings, string IDs, tag \
                         references, and enum names.",
                    )
                    .color(subtle_dark()),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let response = ui.add_enabled(
                        !self.field_value_searching,
                        egui::TextEdit::singleline(&mut self.field_value_query)
                            .hint_text("value to find")
                            .desired_width(240.0),
                    );
                    let submitted =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if self.field_value_searching {
                        ui.spinner();
                        ui.label(RichText::new("searching…").color(subtle_dark()));
                    } else if ui.button("Search").clicked() || submitted {
                        do_search = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("group").color(subtle_dark()).small());
                    ui.add(
                        egui::TextEdit::singleline(&mut self.field_value_group)
                            .hint_text("any (e.g. weap / weapon)")
                            .desired_width(180.0),
                    )
                    .on_hover_text("Optional: limit the search to a tag group (four-CC or name).");
                });
                ui.add_space(4.0);
                let indexed = self.field_index.is_ready_for(self.source_generation);
                ui.horizontal(|ui| {
                    if indexed {
                        ui.label(
                            RichText::new("● indexed — searches are instant")
                                .color(Color32::from_rgb(120, 170, 90))
                                .small(),
                        );
                    } else if self.field_index.is_building() {
                        ui.spinner();
                        ui.label(
                            RichText::new("building index…")
                                .color(subtle_dark())
                                .small(),
                        );
                    } else {
                        ui.label(
                            RichText::new("not indexed — first search scans live")
                                .color(subtle_dark())
                                .small(),
                        );
                        if ui.small_button("Build index").clicked() {
                            do_build = true;
                        }
                    }
                });
            });
        if do_search && !self.field_value_query.trim().is_empty() {
            self.begin_field_value_search(ctx.clone());
        }
        if do_build {
            self.begin_build_field_index(ctx.clone());
        }
        self.field_value_search_open = open;
    }

    /// Window listing all keywords (with tag counts); clicking one shows the
    /// tags that carry it in the shared results window.
    fn draw_keyword_chooser_window(&mut self, ctx: &egui::Context) {
        if !self.keyword_chooser_open {
            return;
        }
        let mut open = true;
        let mut chosen: Option<String> = None;
        let all = self.keywords.all_keywords();
        egui::Window::new("Keywords")
            .id(egui::Id::new("keyword_chooser"))
            .open(&mut open)
            .default_width(280.0)
            .show(ctx, |ui| {
                if all.is_empty() {
                    ui.label(
                        RichText::new("No keywords yet — add them on a tag's Keywords bar.")
                            .color(subtle_dark()),
                    );
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (keyword, count) in &all {
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(format!("{keyword}  ({count})"))
                                            .color(text_dark()),
                                    )
                                    .sense(Sense::click()),
                                )
                                .on_hover_text("Show tags with this keyword")
                                .clicked()
                            {
                                chosen = Some(keyword.clone());
                            }
                        }
                    });
            });
        if let Some(keyword) = chosen {
            self.show_tags_with_keyword(&keyword);
        }
        self.keyword_chooser_open = open;
    }

    /// Reference-graph navigator: parents (referenced by) on the left, children
    /// (references) on the right, with the focused tag and back/forward history.
    fn draw_content_explorer_window(&mut self, ctx: &egui::Context) {
        if self.content_explorer.is_none() {
            return;
        }
        enum ExplorerAct {
            Navigate(TagEntry),
            Back,
            Forward,
            Open(String),
            Reveal(String),
        }
        let mut open = true;
        let mut act: Option<ExplorerAct> = None;
        let mut filter = self
            .content_explorer
            .as_ref()
            .map(|explorer| explorer.filter.clone())
            .unwrap_or_default();
        {
            let explorer = self.content_explorer.as_ref().expect("checked above");
            egui::Window::new("Content Explorer")
                .id(egui::Id::new("content_explorer"))
                .open(&mut open)
                .default_width(720.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!explorer.back.is_empty(), egui::Button::new("← Back"))
                            .clicked()
                        {
                            act = Some(ExplorerAct::Back);
                        }
                        if ui
                            .add_enabled(
                                !explorer.forward.is_empty(),
                                egui::Button::new("Forward →"),
                            )
                            .clicked()
                        {
                            act = Some(ExplorerAct::Forward);
                        }
                        ui.separator();
                        if ui.button("Open in editor").clicked() {
                            act = Some(ExplorerAct::Open(explorer.focus.key.clone()));
                        }
                        if ui.button("Reveal in browser").clicked() {
                            act = Some(ExplorerAct::Reveal(explorer.focus.key.clone()));
                        }
                        ui.separator();
                        ui.add(
                            egui::TextEdit::singleline(&mut filter)
                                .hint_text("filter")
                                .desired_width(140.0),
                        );
                    });
                    ui.separator();
                    ui.label(
                        RichText::new(explorer.focus.display_path.replace('\\', "/"))
                            .strong()
                            .color(text_dark()),
                    );
                    if explorer.index_unavailable {
                        let note = if self.building_reverse_dependencies || self.scanning_entries {
                            "Reference index is building — reopen this in a moment."
                        } else {
                            "Reference index unavailable — it builds automatically for \
                             loose-folder sources, or run Tools → Build Reference Index."
                        };
                        ui.label(RichText::new(note).color(subtle_dark()));
                    }
                    ui.separator();
                    let filter_lower = filter.trim().to_ascii_lowercase();
                    let matches = |entry: &TagEntry| {
                        filter_lower.is_empty()
                            || entry
                                .display_path
                                .to_ascii_lowercase()
                                .contains(&filter_lower)
                    };
                    let parents: Vec<&TagEntry> =
                        explorer.parents.iter().filter(|e| matches(e)).collect();
                    let children: Vec<&TagEntry> =
                        explorer.children.iter().filter(|e| matches(e)).collect();
                    let count_label = |shown: usize, total: usize| {
                        if shown == total {
                            format!("({total})")
                        } else {
                            format!("({shown}/{total})")
                        }
                    };
                    ui.columns(2, |cols| {
                        cols[0].label(
                            RichText::new(format!(
                                "Referenced by {}",
                                count_label(parents.len(), explorer.parents.len())
                            ))
                            .strong()
                            .color(text_dark()),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("ce_parents")
                            .max_height(380.0)
                            .show(&mut cols[0], |ui| {
                                if parents.is_empty() {
                                    ui.label(RichText::new("(none)").color(subtle_dark()));
                                }
                                for entry in &parents {
                                    if explorer_entry_row(ui, entry) {
                                        act = Some(ExplorerAct::Navigate((*entry).clone()));
                                    }
                                }
                            });
                        cols[1].label(
                            RichText::new(format!(
                                "References {}",
                                count_label(children.len(), explorer.children.len())
                            ))
                            .strong()
                            .color(text_dark()),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("ce_children")
                            .max_height(380.0)
                            .show(&mut cols[1], |ui| {
                                if children.is_empty() {
                                    ui.label(RichText::new("(none)").color(subtle_dark()));
                                }
                                for entry in &children {
                                    if explorer_entry_row(ui, entry) {
                                        act = Some(ExplorerAct::Navigate((*entry).clone()));
                                    }
                                }
                            });
                    });
                });
        }
        if let Some(explorer) = self.content_explorer.as_mut() {
            explorer.filter = filter;
        }
        match act {
            Some(ExplorerAct::Navigate(entry)) => self.content_explorer_navigate(entry),
            Some(ExplorerAct::Back) => self.content_explorer_back(),
            Some(ExplorerAct::Forward) => self.content_explorer_forward(),
            Some(ExplorerAct::Open(key)) => self.select_entry(key, ctx.clone()),
            Some(ExplorerAct::Reveal(key)) => self.reveal_in_browser(&key),
            None => {}
        }
        if !open {
            self.content_explorer = None;
        }
    }

    /// Floating window listing the results of a tag query (find-references /
    /// unreferenced). Clicking an entry opens it.
    pub(super) fn source_game(&self) -> Option<&str> {
        self.source
            .as_ref()
            .and_then(|source| source.game.as_deref())
    }

    pub(super) fn source_tags_root(&self) -> Option<&std::path::Path> {
        self.source
            .as_ref()
            .and_then(|source| match &source.source {
                TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                _ => None,
            })
    }

    pub(super) fn source_definitions_root(&self) -> Option<&std::path::Path> {
        self.source
            .as_ref()
            .and_then(|source| match &source.source {
                TagSource::LooseFolder {
                    definitions_root, ..
                } => Some(definitions_root.as_path()),
                _ => None,
            })
    }

    fn draw_tag_diff_window(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.tag_diff.take() else {
            return;
        };
        let a_group = self
            .parsed_tags
            .get(&state.a_key)
            .map(|doc| doc.tag.group().tag);
        let mut open = true;
        let mut compute = false;
        let mut browse = false;
        egui::Window::new("Compare Tags")
            .id(egui::Id::new("tag_diff_window"))
            .open(&mut open)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("A:").strong().color(text_dark()));
                    ui.monospace(state.a_key.replace('\\', "/"));
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("B:").strong().color(text_dark()));
                    let selected = state
                        .b_display
                        .clone()
                        .map(|k| k.replace('\\', "/"))
                        .unwrap_or_else(|| "(open tag)".to_owned());
                    let mut keys: Vec<String> = self
                        .parsed_tags
                        .keys()
                        .filter(|k| {
                            **k != state.a_key
                                && self.parsed_tags.get(*k).map(|d| d.tag.group().tag) == a_group
                        })
                        .cloned()
                        .collect();
                    keys.sort();
                    let (_, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt("tag_diff_b")
                            .selected_text(selected)
                            .width(380.0),
                        |ui| {
                            for key in &keys {
                                if ui
                                    .selectable_label(
                                        state.b_key.as_deref() == Some(key.as_str()),
                                        key.replace('\\', "/"),
                                    )
                                    .clicked()
                                {
                                    state.b_key = Some(key.clone());
                                    state.b_display = Some(key.clone());
                                    state.results = None;
                                }
                            }
                        },
                    );
                    if let Some(delta) = wheel_delta {
                        let current = state
                            .b_key
                            .as_ref()
                            .and_then(|selected| keys.iter().position(|key| key == selected))
                            .unwrap_or(0);
                        if let Some(next) = combo_scroll_next_index(current, keys.len(), delta) {
                            let key = keys[next].clone();
                            state.b_key = Some(key.clone());
                            state.b_display = Some(key);
                            state.results = None;
                        }
                    }
                    if ui
                        .add_enabled(state.b_key.is_some(), egui::Button::new("Compare"))
                        .clicked()
                    {
                        compute = true;
                    }
                    if ui
                        .button("Browse file…")
                        .on_hover_text("Pick any tag of the same group from disk")
                        .clicked()
                    {
                        browse = true;
                    }
                });
                ui.label(
                    RichText::new("Compares field-by-field against an open tab or a tag on disk.")
                        .small()
                        .color(subtle_dark()),
                );

                if let Some(results) = &state.results {
                    ui.separator();
                    if results.diffs.is_empty() {
                        ui.label(RichText::new("No differences.").color(subtle_dark()));
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} differing field(s){}",
                                    results.diffs.len(),
                                    if results.truncated { " (capped)" } else { "" }
                                ))
                                .color(subtle_dark())
                                .small(),
                            );
                            if ui
                                .small_button("Copy")
                                .on_hover_text("Copy the diff as tab-separated rows")
                                .clicked()
                            {
                                let text = std::iter::once("field\tA\tB".to_owned())
                                    .chain(
                                        results
                                            .diffs
                                            .iter()
                                            .map(|d| format!("{}\t{}\t{}", d.path, d.a, d.b)),
                                    )
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                ui.output_mut(|output| output.copied_text = text);
                            }
                        });
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .max_height(460.0)
                            .show(ui, |ui| {
                                egui::Grid::new("tag_diff_grid")
                                    .num_columns(3)
                                    .striped(true)
                                    .show(ui, |ui| {
                                        for diff in &results.diffs {
                                            ui.label(RichText::new(&diff.path).monospace().small());
                                            ui.label(RichText::new(&diff.a).color(text_dark()));
                                            ui.label(RichText::new(&diff.b).color(text_dark()));
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                }
            });

        if compute {
            if let Some(b_key) = state.b_key.clone() {
                let names = TagNameIndex::default();
                let diff = match (
                    self.parsed_tags.get(&state.a_key),
                    self.parsed_tags.get(&b_key),
                ) {
                    (Some(a), Some(b)) => Some(diff_tags(&a.tag, &b.tag, &names, 5000)),
                    _ => None,
                };
                if let Some((diffs, truncated)) = diff {
                    state.results = Some(TagDiffResults { diffs, truncated });
                }
            }
        }
        if browse {
            if let Some(group) = a_group {
                let ext = group_tag_to_extension(group).unwrap_or("");
                let mut dialog = rfd::FileDialog::new().set_title("Pick tag B to compare");
                if !ext.is_empty() {
                    dialog = dialog.add_filter(ext, &[ext]);
                }
                if let Some(root) = self.source_tags_root() {
                    dialog = dialog.set_directory(root);
                }
                if let Some(path) = dialog.pick_file() {
                    let game = self.source_game();
                    let definitions_root = self.source_definitions_root();
                    match crate::source::read_tag_at_path(&path, game, definitions_root, group) {
                        Ok(b_tag) => {
                            if let Some(a) = self.parsed_tags.get(&state.a_key) {
                                let names = TagNameIndex::default();
                                let (diffs, truncated) = diff_tags(&a.tag, &b_tag, &names, 5000);
                                state.b_key = None;
                                state.b_display = Some(path.display().to_string());
                                state.results = Some(TagDiffResults { diffs, truncated });
                            }
                        }
                        Err(error) => {
                            self.status =
                                format!("Compare: could not load {}: {error}", path.display());
                        }
                    }
                }
            }
        }
        if open {
            self.tag_diff = Some(state);
        }
    }

    fn draw_query_results_window(&mut self, ctx: &egui::Context) {
        // Walk any expanded-but-uncached referrer rows before we take the results
        // (this reads `self.query_results`).
        self.refresh_ref_jump_occurrences(ctx);
        let Some(results) = self.query_results.take() else {
            return;
        };
        let mut open = true;
        let mut to_open: Option<String> = None;
        let mut to_reveal: Option<String> = None;
        let mut to_toggle: Vec<usize> = Vec::new();
        let mut to_jump: Option<(String, String)> = None;
        let expanded = &self.ref_jump_expanded;
        let occurrences = &self.ref_jump_occurrences;
        egui::Window::new(&results.title)
            .id(egui::Id::new("tag_query_results"))
            .open(&mut open)
            .default_width(440.0)
            .show(ctx, |ui| {
                if let Some(note) = &results.note {
                    ui.label(RichText::new(note).color(subtle_dark()));
                }
                if !results.entries.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} tag(s)", results.entries.len()))
                                .color(subtle_dark())
                                .small(),
                        );
                        if ui
                            .small_button("Copy paths")
                            .on_hover_text("Copy all result tag paths (one per line)")
                            .clicked()
                        {
                            let text = results
                                .entries
                                .iter()
                                .map(|entry| entry.display_path.replace('\\', "/"))
                                .collect::<Vec<_>>()
                                .join("\n");
                            ui.output_mut(|output| output.copied_text = text);
                        }
                    });
                    ui.separator();
                    // A references popup lets each row expand to its per-occurrence
                    // list; other query kinds render a plain clickable row.
                    let expandable = results.ref_target.is_some();
                    egui::ScrollArea::vertical()
                        .max_height(460.0)
                        .show(ui, |ui| {
                            for (index, entry) in results.entries.iter().enumerate() {
                                let path = entry.display_path.replace('\\', "/");
                                let label = match results.annotations.get(index) {
                                    Some(annotation) => format!("{annotation}  —  {path}"),
                                    None => path,
                                };
                                let is_expanded = expanded.contains(&index);
                                let make_row = |ui: &mut egui::Ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(&label).color(text_dark()))
                                            .sense(Sense::click()),
                                    )
                                    .on_hover_text(
                                        "Click to jump to the first reference · right-click to reveal",
                                    )
                                };
                                let row = if expandable {
                                    ui.horizontal(|ui| {
                                        let arrow = if is_expanded { "▼" } else { "▶" };
                                        if ui
                                            .add(
                                                egui::Button::new(RichText::new(arrow).small())
                                                    .frame(false),
                                            )
                                            .on_hover_text("Show every field that references this tag")
                                            .clicked()
                                        {
                                            to_toggle.push(index);
                                        }
                                        make_row(ui)
                                    })
                                    .inner
                                } else {
                                    make_row(ui)
                                };
                                if row.clicked() {
                                    to_open = Some(entry.key.clone());
                                }
                                row.context_menu(|ui| {
                                    if ui.button("Open").clicked() {
                                        to_open = Some(entry.key.clone());
                                        ui.close_menu();
                                    }
                                    if ui.button("Reveal in browser").clicked() {
                                        to_reveal = Some(entry.key.clone());
                                        ui.close_menu();
                                    }
                                });
                                if expandable && is_expanded {
                                    match occurrences.get(&index) {
                                        Some(list) if !list.is_empty() => {
                                            for occ in list {
                                                ui.horizontal(|ui| {
                                                    ui.add_space(22.0);
                                                    if ui
                                                        .add(
                                                            egui::Label::new(
                                                                RichText::new(format!("↳ {}", occ.label))
                                                                    .color(subtle_dark()),
                                                            )
                                                            .sense(Sense::click()),
                                                        )
                                                        .on_hover_text("Jump to this field")
                                                        .clicked()
                                                    {
                                                        to_jump = Some((
                                                            entry.key.clone(),
                                                            occ.field_path.clone(),
                                                        ));
                                                    }
                                                });
                                            }
                                        }
                                        Some(_) => {
                                            ui.horizontal(|ui| {
                                                ui.add_space(22.0);
                                                ui.label(
                                                    RichText::new("no direct field found")
                                                        .italics()
                                                        .color(subtle_dark())
                                                        .small(),
                                                );
                                            });
                                        }
                                        None => {
                                            ui.horizontal(|ui| {
                                                ui.add_space(22.0);
                                                ui.label(
                                                    RichText::new("loading…")
                                                        .italics()
                                                        .color(subtle_dark())
                                                        .small(),
                                                );
                                            });
                                        }
                                    }
                                }
                            }
                        });
                }
            });
        for index in to_toggle {
            if self.ref_jump_expanded.remove(&index) {
                // Collapsed — drop the cache so a re-expand re-reads fresh.
                self.ref_jump_occurrences.remove(&index);
            } else {
                self.ref_jump_expanded.insert(index);
            }
        }
        if let Some((key, field_path)) = to_jump {
            // The referrer is already loaded (we walked it for occurrences), so
            // focus it and navigate to the exact field directly.
            self.select_entry(key.clone(), ctx.clone());
            self.navigate_to_field(ctx, &key, &field_path);
        }
        if let Some(key) = to_open {
            // For a "References to X" result, queue a jump to the exact field in
            // the referrer that points at X (resolved once the tag loads).
            if let Some((group_tag, rel_path)) = &results.ref_target {
                self.pending_ref_jump = Some(PendingRefJump {
                    tag_key: key.clone(),
                    group_tag: *group_tag,
                    rel_path: rel_path.clone(),
                });
            }
            self.select_entry(key, ctx.clone());
        }
        if let Some(key) = to_reveal {
            self.reveal_in_browser(&key);
        }
        // Keep the window's results until it is closed.
        if open {
            self.query_results = Some(results);
        }
    }

    fn draw_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }

        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .id(egui::Id::new("app_settings"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(760.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.settings_tab, SettingsTab::Startup, "Startup");
                    ui.selectable_value(&mut self.settings_tab, SettingsTab::Browser, "Browser");
                    ui.selectable_value(
                        &mut self.settings_tab,
                        SettingsTab::EditingKits,
                        "Editing Kits",
                    );
                    ui.selectable_value(
                        &mut self.settings_tab,
                        SettingsTab::EditingKitAliases,
                        "Editing Kit Aliases",
                    );
                    ui.selectable_value(
                        &mut self.settings_tab,
                        SettingsTab::Appearance,
                        "Appearance",
                    );
                    ui.selectable_value(&mut self.settings_tab, SettingsTab::Tools, "Tools");
                });
                ui.separator();
                ui.add_space(8.0);
                match self.settings_tab {
                    SettingsTab::Startup => self.draw_settings_startup_tab(ui),
                    SettingsTab::Browser => self.draw_settings_browser_tab(ui),
                    SettingsTab::EditingKits => self.draw_settings_editing_kits_tab(ui),
                    SettingsTab::EditingKitAliases => self.draw_settings_aliases_tab(ui),
                    SettingsTab::Appearance => self.draw_settings_appearance_tab(ui),
                    SettingsTab::Tools => self.draw_settings_tools_tab(ui),
                }
            });
        if !open {
            self.pending_ui_scale = self.ui_scale;
        }
        self.settings_open = open;
    }

    fn set_editing_kit_path_input(&mut self, shortcut: EditingKitShortcut, input: String) {
        let trimmed = input.trim().to_owned();
        if trimmed.is_empty() {
            self.editing_kit_paths.remove(shortcut.game);
        } else {
            self.editing_kit_paths
                .insert(shortcut.game.to_owned(), PathBuf::from(&trimmed));
        }
        self.editing_kit_path_inputs
            .insert(shortcut.game.to_owned(), input);
        if self.editing_kit_path_attention.as_deref() == Some(shortcut.game) && !trimmed.is_empty()
        {
            self.editing_kit_path_attention = None;
        }
    }

    fn draw_settings_startup_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Startup").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new("When reopening Baboon with a previous session:").color(text_dark()),
        );
        ui.add_space(2.0);
        ui.radio_value(
            &mut self.session_restore,
            SessionRestore::Ask,
            "Ask which windows to reopen",
        );
        ui.radio_value(
            &mut self.session_restore,
            SessionRestore::Always,
            "Reopen the last session automatically",
        );
        ui.radio_value(
            &mut self.session_restore,
            SessionRestore::Never,
            "Start fresh (never reopen)",
        );
    }

    fn draw_settings_browser_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Browser").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.double_click_to_open_tags,
            "Double-click to open tags",
        );
        ui.checkbox(
            &mut self.folders_before_tags,
            "List subfolders before tags in browser",
        );
    }

    fn draw_settings_editing_kits_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Editing Kits").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new("Configure each editing-kit root or its tags folder for quick loading.")
                .color(subtle_dark()),
        );
        if ui.button("Auto Detect").clicked() {
            self.auto_detect_editing_kit_paths();
        }
        ui.add_space(6.0);

        for shortcut in EDITING_KIT_SHORTCUTS {
            let attention = self.editing_kit_path_attention.as_deref() == Some(shortcut.game);
            let fill = if attention {
                if is_dark_mode() {
                    Color32::from_rgb(62, 45, 39)
                } else {
                    Color32::from_rgb(255, 226, 212)
                }
            } else {
                Color32::TRANSPARENT
            };
            let texture = self.game_banner_texture(ui.ctx(), shortcut.game).cloned();
            let mut input = self
                .editing_kit_path_inputs
                .get(shortcut.game)
                .cloned()
                .unwrap_or_default();
            let mut changed = false;
            let mut browse = false;
            let mut load = false;
            let mut clear = false;

            Frame::none()
                .fill(fill)
                .inner_margin(egui::Margin::same(4.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(texture) = &texture {
                            ui.add(
                                egui::Image::new(egui::load::SizedTexture::new(
                                    texture.id(),
                                    Vec2::splat(20.0),
                                ))
                                .fit_to_exact_size(Vec2::splat(20.0)),
                            );
                        } else {
                            ui.label(RichText::new(shortcut.fallback).color(text_dark()).strong());
                        }
                        ui.add_sized(
                            Vec2::new(72.0, 20.0),
                            egui::Label::new(RichText::new(shortcut.label).color(text_dark())),
                        );
                        changed = ui
                            .add(
                                egui::TextEdit::singleline(&mut input)
                                    .hint_text("editing-kit root or tags folder")
                                    .desired_width(360.0),
                            )
                            .changed();
                        browse = ui.button("Browse...").clicked();
                        load = ui.button("Load").clicked();
                        clear = ui.button("Clear").clicked();
                    });
                });

            if changed {
                self.set_editing_kit_path_input(shortcut, input);
            }
            if browse {
                self.choose_editing_kit_path(shortcut);
            }
            if load {
                self.load_editing_kit_shortcut(shortcut, ui.ctx().clone());
            }
            if clear {
                self.editing_kit_paths.remove(shortcut.game);
                self.editing_kit_path_inputs
                    .insert(shortcut.game.to_owned(), String::new());
                if self.editing_kit_path_attention.as_deref() == Some(shortcut.game) {
                    self.editing_kit_path_attention = None;
                }
                self.status = format!("{} path cleared", shortcut.label);
            }
        }
    }

    fn draw_settings_aliases_tab(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new("Editing Kit Folder Aliases")
                .color(text_dark())
                .strong(),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Map custom kit folder names to a game profile, for example h2rek -> halo2_mcc.",
            )
            .color(subtle_dark()),
        );
        ui.add_space(4.0);
        let mut remove_alias = None;
        let mut aliases_changed = false;
        ui.label(RichText::new("Configured aliases").color(subtle_dark()));
        if self.ek_folder_aliases.is_empty() {
            ui.label(RichText::new("No custom aliases added").color(subtle_dark()));
        }
        for index in 0..self.ek_folder_aliases.len() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Folder").color(subtle_dark()));
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.ek_folder_aliases[index].folder_name)
                            .desired_width(160.0),
                    )
                    .changed()
                {
                    aliases_changed = true;
                }
                let selected_label = ek_game_label(&self.ek_folder_aliases[index].game);
                let (_, wheel_delta) = combo_box_with_scroll(
                    ui,
                    egui::ComboBox::from_id_salt(("ek_folder_alias_game", index))
                        .selected_text(selected_label)
                        .width(210.0),
                    |ui| {
                        for (label, game) in SUPPORTED_EK_GAMES {
                            if ui
                                .selectable_value(
                                    &mut self.ek_folder_aliases[index].game,
                                    (*game).to_owned(),
                                    *label,
                                )
                                .changed()
                            {
                                aliases_changed = true;
                            }
                        }
                    },
                );
                if let Some(delta) = wheel_delta {
                    let current = SUPPORTED_EK_GAMES
                        .iter()
                        .position(|(_, game)| *game == self.ek_folder_aliases[index].game)
                        .unwrap_or(0);
                    if let Some(next) =
                        combo_scroll_next_index(current, SUPPORTED_EK_GAMES.len(), delta)
                    {
                        let game = SUPPORTED_EK_GAMES[next].1.to_owned();
                        self.ek_folder_aliases[index].game = game;
                        aliases_changed = true;
                    }
                }
                ui.label(
                    RichText::new(format!("-> {}", self.ek_folder_aliases[index].game))
                        .color(subtle_dark()),
                );
                if ui.small_button("Remove").clicked() {
                    remove_alias = Some(index);
                }
            });
        }
        if let Some(index) = remove_alias {
            self.ek_folder_aliases.remove(index);
            aliases_changed = true;
            self.status = "Editing kit folder alias removed".to_owned();
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("New").color(subtle_dark()));
            ui.add(
                egui::TextEdit::singleline(&mut self.new_ek_alias_name)
                    .hint_text(
                        RichText::new("example: h2rek")
                            .italics()
                            .color(placeholder_text()),
                    )
                    .desired_width(160.0),
            );
            let (_, wheel_delta) = combo_box_with_scroll(
                ui,
                egui::ComboBox::from_id_salt("new_ek_folder_alias_game")
                    .selected_text(ek_game_label(&self.new_ek_alias_game))
                    .width(210.0),
                |ui| {
                    for (label, game) in SUPPORTED_EK_GAMES {
                        ui.selectable_value(
                            &mut self.new_ek_alias_game,
                            (*game).to_owned(),
                            *label,
                        );
                    }
                },
            );
            if let Some(delta) = wheel_delta {
                let current = SUPPORTED_EK_GAMES
                    .iter()
                    .position(|(_, game)| *game == self.new_ek_alias_game)
                    .unwrap_or(0);
                if let Some(next) =
                    combo_scroll_next_index(current, SUPPORTED_EK_GAMES.len(), delta)
                {
                    self.new_ek_alias_game = SUPPORTED_EK_GAMES[next].1.to_owned();
                }
            }
            if ui.button("Add").clicked() {
                let folder_name = self.new_ek_alias_name.trim().to_owned();
                if folder_name.is_empty() {
                    self.status = "Enter a folder name before adding an alias".to_owned();
                } else if supported_ek_game_id(&self.new_ek_alias_game).is_none() {
                    self.status = "Choose a supported game before adding an alias".to_owned();
                } else if let Some(existing) = self
                    .ek_folder_aliases
                    .iter_mut()
                    .find(|alias| alias.folder_name.trim().eq_ignore_ascii_case(&folder_name))
                {
                    existing.folder_name = folder_name.clone();
                    existing.game = self.new_ek_alias_game.clone();
                    self.new_ek_alias_name.clear();
                    aliases_changed = true;
                    self.status = format!("Updated editing kit alias {folder_name}");
                } else {
                    self.ek_folder_aliases.push(EkFolderAlias {
                        folder_name: folder_name.clone(),
                        game: self.new_ek_alias_game.clone(),
                    });
                    self.new_ek_alias_name.clear();
                    aliases_changed = true;
                    self.status = format!("Added editing kit alias {folder_name}");
                }
            }
        });
        if aliases_changed {
            self.reapply_current_folder_profile();
        }
    }

    fn draw_settings_appearance_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Appearance").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.checkbox(&mut self.dark_mode, "Dark mode");
        ui.horizontal(|ui| {
            ui.label(RichText::new("UI scale").color(subtle_dark()));
            ui.add(
                egui::Slider::new(&mut self.pending_ui_scale, MIN_UI_SCALE..=MAX_UI_SCALE)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            );
            ui.label(
                RichText::new(format!("{:.0}%", self.pending_ui_scale * 100.0))
                    .color(subtle_dark()),
            );
            if ui.button("Reset").clicked() {
                self.pending_ui_scale = DEFAULT_UI_SCALE;
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Model viewport").color(subtle_dark()));
            ui.add(
                egui::Slider::new(
                    &mut self.model_preview_size,
                    MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
                )
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
            );
            ui.label(
                RichText::new(format!("{:.0}%", self.model_preview_size * 100.0))
                    .color(subtle_dark()),
            );
            if ui.button("Reset").clicked() {
                self.model_preview_size = DEFAULT_MODEL_PREVIEW_SIZE;
            }
        });
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Apply UI scale").clicked() {
                self.ui_scale = self.pending_ui_scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
                self.status = "UI scale applied".to_owned();
            }
        });
    }

    fn draw_settings_tools_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Blender").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Path").color(subtle_dark()));
            let path_response = ui
                .add(egui::TextEdit::singleline(&mut self.blender_path_input).desired_width(360.0));
            if path_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                let trimmed = self.blender_path_input.trim();
                self.blender_path = if trimmed.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(trimmed))
                };
                self.status = if let Some(path) = &self.blender_path {
                    format!("Blender path set to {}", path.display())
                } else {
                    "Blender path cleared".to_owned()
                };
            }
            if ui.button("Browse...").clicked() {
                self.choose_blender_path();
            }
        });
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Clear").clicked() {
                self.blender_path = None;
                self.blender_path_input.clear();
                self.status = "Blender path cleared".to_owned();
            }
        });
    }

    fn draw_tool_commands_window(&mut self, ctx: &egui::Context) {
        if !self.tool_commands.open {
            return;
        }
        let game = self
            .source
            .as_ref()
            .and_then(|source| source.game.as_deref())
            .map(str::to_owned);
        if let Some(game) = game.as_deref() {
            self.ensure_tool_commands_loaded(game);
        }

        let mut open = self.tool_commands.open;
        let window_size = self.tool_commands_window_size;
        let mut window_pos = self.tool_commands_window_pos.unwrap_or_else(|| {
            let available = ctx.available_rect();
            egui::pos2(
                available.center().x - window_size.x * 0.5,
                available.center().y - window_size.y * 0.5,
            )
        });
        let mut dragged_window_pos = None;
        let mut close_requested = false;
        let window = egui::Window::new("Tool Commands")
            .id(egui::Id::new("tool_commands"))
            .collapsible(false)
            .title_bar(false)
            .movable(false)
            .resizable(true)
            .drag_to_scroll(false)
            .constrain(false)
            .open(&mut open)
            .current_pos(window_pos)
            .min_size(MIN_TOOL_COMMANDS_WINDOW_SIZE)
            .default_size(self.tool_commands_window_size);
        let response = window.show(ctx, |ui| {
            let title_height = 28.0;
            let (title_rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), title_height),
                Sense::hover(),
            );
            let close_width = 28.0;
            let close_rect = egui::Rect::from_min_max(
                egui::pos2(title_rect.right() - close_width, title_rect.top()),
                title_rect.right_bottom(),
            );
            let drag_rect = egui::Rect::from_min_max(
                title_rect.min,
                egui::pos2(close_rect.left() - 4.0, title_rect.bottom()),
            );
            let title_response = ui.interact(drag_rect, ui.id().with("title_bar"), Sense::drag());
            if title_response.dragged() {
                window_pos += ui.input(|input| input.pointer.delta());
                dragged_window_pos = Some(window_pos);
                ctx.request_repaint();
            }
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(title_rect.shrink2(Vec2::new(4.0, 2.0))),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Tool Commands").color(text_dark()).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("×").clicked() {
                                close_requested = true;
                            }
                        });
                    });
                },
            );
            ui.separator();

            if game.is_none() {
                ui.label(
                    RichText::new("Load an editing-kit folder first to view tool commands.")
                        .color(subtle_dark()),
                );
                return;
            }
            if let Some(error) = self.tool_commands.error.as_ref() {
                ui.label(RichText::new(error).color(material_delete_text()));
                return;
            }
            if self.tool_commands.commands.is_empty() {
                ui.label(
                    RichText::new("No tool commands documented for this game").color(subtle_dark()),
                );
                return;
            }

            let available_width = ui.available_width();
            let available_height = ui
                .available_height()
                .max(MIN_TOOL_COMMANDS_WINDOW_SIZE.y - 80.0);
            let max_left_width = (available_width - 320.0).max(MIN_TOOL_COMMANDS_LEFT_WIDTH);
            self.tool_commands_left_width = self
                .tool_commands_left_width
                .clamp(MIN_TOOL_COMMANDS_LEFT_WIDTH, max_left_width);
            ui.horizontal(|ui| {
                ui.set_height(available_height);
                ui.allocate_ui_with_layout(
                    Vec2::new(self.tool_commands_left_width, available_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(self.tool_commands_left_width);
                        ui.label(RichText::new("Commands").color(text_dark()).strong());
                        ui.separator();
                        let list_height = ui.available_height().max(120.0);
                        egui::ScrollArea::vertical()
                            .id_salt("tool_command_list")
                            .max_height(list_height)
                            .show(ui, |ui| {
                                self.draw_tool_command_list(ui);
                            });
                    },
                );
                let (handle_rect, handle_response) =
                    ui.allocate_exact_size(Vec2::new(7.0, available_height), Sense::drag());
                let handle_color = if handle_response.hovered() || handle_response.dragged() {
                    material_grid_light()
                } else {
                    material_input_edge()
                };
                ui.painter().line_segment(
                    [handle_rect.center_top(), handle_rect.center_bottom()],
                    Stroke::new(2.0, handle_color),
                );
                if handle_response.dragged() {
                    self.tool_commands_left_width = (self.tool_commands_left_width
                        + ui.input(|input| input.pointer.delta().x))
                    .clamp(MIN_TOOL_COMMANDS_LEFT_WIDTH, max_left_width);
                }
                let right_width = ui.available_width().max(300.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(right_width, available_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(300.0);
                        egui::ScrollArea::vertical()
                            .id_salt("tool_command_detail")
                            .max_height(available_height)
                            .show(ui, |ui| {
                                self.draw_selected_tool_command(ui, ctx);
                            });
                    },
                );
            });
        });
        if let Some(response) = response {
            let rect = response.response.rect;
            self.tool_commands_window_pos = dragged_window_pos.or(Some(rect.min));
            self.tool_commands_window_size = rect.size();
        }
        if close_requested {
            open = false;
        }
        self.tool_commands.open = open;
    }

    fn ensure_tool_commands_loaded(&mut self, game: &str) {
        if self.tool_commands.catalog_game.as_deref() == Some(game) {
            return;
        }
        self.tool_commands.catalog_game = Some(game.to_owned());
        match load_tool_commands(game) {
            Ok(commands) => {
                self.tool_commands.error = None;
                self.tool_commands.commands = commands;
                self.tool_commands.selected = self
                    .tool_commands
                    .commands
                    .first()
                    .map(|command| command.name.clone());
                self.tool_commands.values.clear();
                self.tool_commands.optional_open = false;
            }
            Err(error) => {
                self.tool_commands.commands.clear();
                self.tool_commands.selected = None;
                self.tool_commands.values.clear();
                self.tool_commands.error = Some(error);
            }
        }
    }

    fn draw_tool_command_list(&mut self, ui: &mut Ui) {
        let mut categories = Vec::<String>::new();
        for command in &self.tool_commands.commands {
            if !categories
                .iter()
                .any(|category| category == &command.category)
            {
                categories.push(command.category.clone());
            }
        }
        categories.sort_by_key(|category| {
            (
                category.eq_ignore_ascii_case("Advanced / Unknown"),
                category.clone(),
            )
        });

        let header_color = ui.visuals().hyperlink_color;
        for (index, category) in categories.into_iter().enumerate() {
            if index > 0 {
                ui.add_space(6.0);
            }
            let collapsed = self.tool_commands_collapsed_categories.contains(&category);
            let mut toggle_clicked = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (icon_rect, icon_response) =
                    ui.allocate_exact_size(Vec2::new(16.0, 16.0), Sense::click());
                disclosure_triangle_icon(
                    ui,
                    !collapsed,
                    icon_rect.center(),
                    if collapsed {
                        disclosure_triangle_blue()
                    } else {
                        disclosure_triangle_green()
                    },
                );
                let label_response = ui.add(
                    egui::Label::new(
                        RichText::new(&category)
                            .color(header_color)
                            .strong()
                            .size(13.0),
                    )
                    .sense(Sense::click()),
                );
                toggle_clicked = icon_response.clicked() || label_response.clicked();
            });
            if toggle_clicked {
                if collapsed {
                    self.tool_commands_collapsed_categories.remove(&category);
                } else {
                    self.tool_commands_collapsed_categories
                        .insert(category.clone());
                }
            }
            if self.tool_commands_collapsed_categories.contains(&category) {
                continue;
            }
            let commands = self
                .tool_commands
                .commands
                .iter()
                .filter(|command| command.category == category)
                .map(|command| command.name.clone())
                .collect::<Vec<_>>();
            ui.indent(("tool_command_category", &category), |ui| {
                for command_name in commands {
                    let selected =
                        self.tool_commands.selected.as_deref() == Some(command_name.as_str());
                    if ui.selectable_label(selected, &command_name).clicked() {
                        self.tool_commands.selected = Some(command_name);
                        self.tool_commands.values.clear();
                        self.tool_commands.optional_open = false;
                    }
                }
            });
        }
    }

    fn draw_selected_tool_command(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let Some(command) = self.selected_tool_command().cloned() else {
            ui.label(RichText::new("Select a command").color(subtle_dark()));
            return;
        };
        ui.heading(RichText::new(&command.name).color(text_dark()));
        ui.label(RichText::new(&command.category).color(subtle_dark()));
        ui.add_space(4.0);
        if !command.description.is_empty() {
            ui.label(RichText::new(&command.description).color(text_dark()));
        }
        if !command.example.is_empty() {
            ui.label(
                RichText::new(format!("Example: {}", command.example))
                    .color(subtle_dark())
                    .monospace(),
            );
        }
        ui.add_space(10.0);

        let (required, optional): (Vec<_>, Vec<_>) =
            command.args.iter().partition(|arg| arg.required);
        if !required.is_empty() {
            ui.label(RichText::new("Arguments").color(text_dark()).strong());
            ui.add_space(3.0);
            for arg in required {
                self.draw_tool_command_arg(ui, &command, arg);
            }
        }
        if !optional.is_empty() {
            ui.add_space(4.0);
            egui::CollapsingHeader::new("Optional arguments")
                .default_open(self.tool_commands.optional_open)
                .show(ui, |ui| {
                    self.tool_commands.optional_open = true;
                    for arg in optional {
                        self.draw_tool_command_arg(ui, &command, arg);
                    }
                });
        }

        ui.add_space(12.0);
        let preview = tool_command_preview(&command, &self.tool_commands.values);
        ui.label(RichText::new("Preview").color(text_dark()).strong());
        let mut preview_text = preview.clone();
        ui.add(
            egui::TextEdit::singleline(&mut preview_text)
                .desired_width(ui.available_width())
                .font(egui::TextStyle::Monospace)
                .interactive(false),
        );
        ui.add_space(8.0);
        let missing = tool_command_missing_required(&command, &self.tool_commands.values);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    missing.is_none() && !self.terminal.running,
                    egui::Button::new("Run").min_size(Vec2::new(80.0, 24.0)),
                )
                .clicked()
            {
                self.submit_terminal_command(preview.clone(), ctx.clone());
                self.tool_commands.open = false;
            }
            if let Some(missing) = missing {
                ui.label(
                    RichText::new(format!("Required argument missing: {missing}"))
                        .color(material_delete_text()),
                );
            }
        });
    }

    fn selected_tool_command(&self) -> Option<&ToolCommand> {
        let selected = self.tool_commands.selected.as_deref()?;
        self.tool_commands
            .commands
            .iter()
            .find(|command| command.name == selected)
    }

    fn draw_tool_command_arg(&mut self, ui: &mut Ui, command: &ToolCommand, arg: &ToolCommandArg) {
        let key = tool_arg_key("", arg);
        let mut value = self
            .tool_commands
            .values
            .get(&key)
            .cloned()
            .unwrap_or_else(|| {
                if arg.kind == ToolCommandArgKind::Enum {
                    arg.values.first().cloned().unwrap_or_default()
                } else {
                    String::new()
                }
            });
        // Inline validation: a required parameter left empty is flagged before
        // Run (the Run button is also disabled). Enum args always have a value.
        let is_invalid =
            arg.required && arg.kind != ToolCommandArgKind::Enum && value.trim().is_empty();
        let mut browse_clicked = false;
        ui.horizontal(|ui| {
            ui.set_min_height(24.0);
            let required = if arg.required { "" } else { " (optional)" };
            ui.label(
                RichText::new(format!("{}{required}", arg.name))
                    .color(text_dark())
                    .strong(),
            );
            ui.add_space(4.0);
            match arg.kind {
                ToolCommandArgKind::Enum => {
                    let (_, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt(("tool_arg_enum", &command.name, &arg.name))
                            .selected_text(if value.is_empty() {
                                arg.values.first().map(String::as_str).unwrap_or("")
                            } else {
                                value.as_str()
                            })
                            .width(180.0),
                        |ui| {
                            for option in &arg.values {
                                ui.selectable_value(&mut value, option.clone(), option);
                            }
                        },
                    );
                    if let Some(delta) = wheel_delta {
                        let current = arg
                            .values
                            .iter()
                            .position(|option| option == &value)
                            .unwrap_or(0);
                        if let Some(next) =
                            combo_scroll_next_index(current, arg.values.len(), delta)
                        {
                            value = arg.values[next].clone();
                        }
                    }
                }
                _ => {
                    let mut edit = egui::TextEdit::singleline(&mut value)
                        .desired_width(300.0)
                        .font(egui::TextStyle::Monospace);
                    if is_invalid {
                        edit = edit.text_color(Color32::from_rgb(190, 70, 54));
                    }
                    ui.add(edit);
                    if matches!(
                        arg.kind,
                        ToolCommandArgKind::PathData
                            | ToolCommandArgKind::PathTag
                            | ToolCommandArgKind::PathFile
                    ) && ui.small_button("...").clicked()
                    {
                        browse_clicked = true;
                    }
                }
            }
            if is_invalid {
                ui.label(
                    RichText::new("required")
                        .small()
                        .color(Color32::from_rgb(190, 70, 54)),
                );
            }
        });
        if browse_clicked && let Some(path) = self.pick_tool_command_path(arg.kind) {
            value = path;
        }
        self.tool_commands.values.insert(key, value);
        if !arg.description.is_empty() {
            ui.label(RichText::new(&arg.description).color(subtle_dark()));
        }
        if matches!(
            arg.kind,
            ToolCommandArgKind::PathData
                | ToolCommandArgKind::PathTag
                | ToolCommandArgKind::PathFile
        ) {
            ui.label(
                RichText::new(
                    "Use backslashes and paths relative to the EK data or tags folder. Quotes are not needed.",
                )
                .color(subtle_dark()),
            );
        }
        ui.add_space(4.0);
    }

    fn pick_tool_command_path(&self, kind: ToolCommandArgKind) -> Option<String> {
        let kit_root = self.editing_kit_root();
        let data_root = kit_root.as_ref().map(|root| root.join("data"));
        let tags_root = kit_root.as_ref().map(|root| root.join("tags"));
        let start_dir = match kind {
            ToolCommandArgKind::PathData => data_root.as_deref(),
            ToolCommandArgKind::PathTag => tags_root.as_deref(),
            ToolCommandArgKind::PathFile => data_root.as_deref().or(kit_root.as_deref()),
            _ => kit_root.as_deref(),
        };
        let mut dialog = rfd::FileDialog::new();
        if let Some(start_dir) = start_dir.filter(|path| path.is_dir()) {
            dialog = dialog.set_directory(start_dir);
        }
        match kind {
            ToolCommandArgKind::PathData => dialog
                .pick_folder()
                .map(|path| path_arg_from_picker(&path, data_root.as_deref(), false)),
            ToolCommandArgKind::PathTag => dialog
                .pick_folder()
                .map(|path| path_arg_from_picker(&path, tags_root.as_deref(), true)),
            ToolCommandArgKind::PathFile => dialog.pick_file().map(|path| {
                path_arg_from_picker(&path, data_root.as_deref().or(tags_root.as_deref()), false)
            }),
            _ => None,
        }
    }

    fn draw_new_tag_window(&mut self, ctx: &egui::Context) {
        if !self.new_tag_open {
            return;
        }

        let mut open = self.new_tag_open;
        let mut refresh_groups = false;
        let mut create = false;
        let mut close_requested = false;
        egui::Window::new("New Tag")
            .id(egui::Id::new("new_tag_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                if self.loaded_tags_root().is_none() {
                    ui.label(
                        RichText::new(
                            "Load a loose editing-kit tags folder before creating a tag.",
                        )
                        .color(subtle_dark()),
                    );
                    ui.add_space(8.0);
                }

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Game").color(subtle_dark()));
                    let before = self.new_tag_dialog.game.clone();
                    let games = crate::app::controller::available_definition_games();
                    let (_, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt("new_tag_game")
                            .selected_text(&self.new_tag_dialog.game)
                            .width(220.0),
                        |ui| {
                            for game in &games {
                                ui.selectable_value(
                                    &mut self.new_tag_dialog.game,
                                    game.clone(),
                                    game,
                                );
                            }
                        },
                    );
                    if let Some(delta) = wheel_delta {
                        let current = games
                            .iter()
                            .position(|game| game == &self.new_tag_dialog.game)
                            .unwrap_or(0);
                        if let Some(next) = combo_scroll_next_index(current, games.len(), delta) {
                            self.new_tag_dialog.game = games[next].clone();
                        }
                    }
                    if self.new_tag_dialog.game != before {
                        refresh_groups = true;
                    }
                });

                let selected_group_before = self.new_tag_dialog.selected_group;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Group").color(subtle_dark()));
                    let selected = self
                        .new_tag_dialog
                        .groups
                        .get(self.new_tag_dialog.selected_group)
                        .map(|group| {
                            format!("{} ({})", group.name, format_group_tag(group.group_tag))
                        })
                        .unwrap_or_else(|| "No schemas".to_owned());
                    let (_, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt("new_tag_group")
                            .selected_text(selected)
                            .width(320.0),
                        |ui| {
                            for (index, group) in self.new_tag_dialog.groups.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.new_tag_dialog.selected_group,
                                    index,
                                    format!(
                                        "{} ({})",
                                        group.name,
                                        format_group_tag(group.group_tag)
                                    ),
                                );
                            }
                        },
                    );
                    if let Some(delta) = wheel_delta {
                        let current = self.new_tag_dialog.selected_group;
                        if let Some(next) = combo_scroll_next_index(
                            current,
                            self.new_tag_dialog.groups.len(),
                            delta,
                        ) {
                            self.new_tag_dialog.selected_group = next;
                        }
                    }
                });
                if self.new_tag_dialog.selected_group != selected_group_before {
                    self.new_tag_dialog.rel_path.clear();
                    self.new_tag_dialog.output_path = None;
                    self.new_tag_dialog.error = None;
                }

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Location").color(subtle_dark()));
                    let location = if self.new_tag_dialog.rel_path.is_empty() {
                        "No tag selected".to_owned()
                    } else {
                        self.new_tag_dialog.rel_path.clone()
                    };
                    let mut location_text = location;
                    ui.add_enabled(
                        false,
                        egui::TextEdit::singleline(&mut location_text).desired_width(360.0),
                    );
                    if ui
                        .add_enabled(
                            self.loaded_tags_root().is_some()
                                && !self.new_tag_dialog.groups.is_empty(),
                            egui::Button::new("Choose..."),
                        )
                        .clicked()
                    {
                        self.choose_new_tag_output_path();
                    }
                });

                if let Some(group) = self
                    .new_tag_dialog
                    .groups
                    .get(self.new_tag_dialog.selected_group)
                {
                    ui.label(
                        RichText::new(format!(
                            "Creates a .{} tag relative to the loaded tags folder.",
                            group.extension
                        ))
                        .color(subtle_dark())
                        .small(),
                    );
                }

                if let Some(error) = &self.new_tag_dialog.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).color(material_delete_text()));
                }

                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        close_requested = true;
                    }
                    let can_create = self.loaded_tags_root().is_some()
                        && !self.new_tag_dialog.groups.is_empty()
                        && self.new_tag_dialog.output_path.is_some();
                    if ui
                        .add_enabled(can_create, egui::Button::new("Create"))
                        .clicked()
                    {
                        create = true;
                    }
                });
            });

        if refresh_groups {
            self.refresh_new_tag_groups();
        }
        if close_requested {
            open = false;
        }
        self.new_tag_open = open;
        if create {
            self.create_new_tag();
        }
    }

    fn draw_about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }

        let mut open = self.about_open;
        egui::Window::new("Baboon Help")
            .id(egui::Id::new("baboon_help"))
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .default_width(780.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.help_panel_tab, HelpPanelTab::About, "About");
                    ui.selectable_value(&mut self.help_panel_tab, HelpPanelTab::Doc, "Doc");
                    ui.selectable_value(
                        &mut self.help_panel_tab,
                        HelpPanelTab::MapNames,
                        "Map Names",
                    );
                });
                ui.separator();
                ui.add_space(8.0);
                match self.help_panel_tab {
                    HelpPanelTab::About => draw_about_tab(ui),
                    HelpPanelTab::Doc => draw_doc_tab(ui, &self.help_docs),
                    HelpPanelTab::MapNames => draw_map_names_tab(ui, &mut self.map_names_game_tab),
                }
            });
        self.about_open = open;
    }
}

fn draw_about_tab(ui: &mut Ui) {
    ui.heading(RichText::new("Baboon").color(text_dark()));
    ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(subtle_dark()));
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("blam-tags created by").color(text_dark()));
        ui.label(
            RichText::new("Camden Smallwood")
                .color(foundation_blue())
                .strong(),
        );
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Baboon created by").color(text_dark()));
        ui.label(
            RichText::new("Zoephie Sinyard")
                .color(foundation_blue())
                .strong(),
        );
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Icons by").color(text_dark()));
        ui.label(RichText::new("Paddy Tee").color(foundation_blue()).strong());
    });
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(8.0);
    ui.label(RichText::new("Source").color(text_dark()).strong());
    ui.hyperlink_to(BABOON_GITHUB_URL, BABOON_GITHUB_URL);
}

fn draw_doc_tab(ui: &mut Ui, docs: &HelpDocsState) {
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match docs {
            HelpDocsState::Loaded(docs) => {
                if let Some(tab) = docs.tab("doc") {
                    for section in &tab.sections {
                        doc_section(ui, section);
                    }
                } else {
                    doc_load_error(ui, "Documentation failed to load: missing doc tab.");
                }
            }
            HelpDocsState::Failed(error) => {
                doc_load_error(ui, &format!("Documentation failed to load: {error}"));
            }
        });
}

fn doc_section(ui: &mut Ui, section: &HelpDocSection) {
    ui.label(
        RichText::new(&section.title)
            .color(foundation_blue())
            .font(FontId::proportional(14.0))
            .strong(),
    );
    ui.add_space(4.0);
    for block in &section.blocks {
        match block {
            HelpDocBlock::Paragraph { text } => {
                ui.add(
                    egui::Label::new(RichText::new(text).color(text_dark()))
                        .wrap()
                        .selectable(false),
                );
                ui.add_space(4.0);
            }
            HelpDocBlock::Bullets { items } => {
                for item in items {
                    doc_bullet(ui, item);
                }
            }
        }
    }
    ui.add_space(12.0);
}

fn doc_bullet(ui: &mut Ui, line: &str) {
    ui.horizontal_top(|ui| {
        ui.label(RichText::new("-").color(subtle_dark()));
        ui.add(
            egui::Label::new(RichText::new(line).color(text_dark()))
                .wrap()
                .selectable(false),
        );
    });
}

fn doc_load_error(ui: &mut Ui, message: &str) {
    ui.add(
        egui::Label::new(RichText::new(message).color(text_dark()))
            .wrap()
            .selectable(false),
    );
}

impl eframe::App for Baboon {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_worker_messages(ctx);
        ctx.set_zoom_factor(self.ui_scale);
        self.handle_pixels_per_point_change(ctx);
        set_dark_mode(self.dark_mode);
        ctx.set_visuals(foundation_visuals());
        set_combo_scroll_cycle_enabled(ctx, self.scroll_to_cycle_dropdowns);
        self.handle_app_close_request(ctx);
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.save_current_tag();
        }
        // Undo: Ctrl+Z. Redo: Ctrl+Shift+Z or Ctrl+Y.
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Z)) {
            self.undo_current_tag();
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
        }) || ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Y))
        {
            self.redo_current_tag();
        }
        let dropped_paths = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if !dropped_paths.is_empty() {
            self.open_dropped_files(dropped_paths, ctx.clone());
        }

        egui::TopBottomPanel::top("menu")
            .frame(Frame::none().fill(menu_bar()).inner_margin(egui::Margin {
                left: 6.0,
                right: 6.0,
                top: 2.0,
                bottom: 2.0,
            }))
            .show(ctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("New Tag...").clicked() {
                            ui.close_menu();
                            self.open_new_tag_dialog();
                        }
                        if ui.button("Load Tag...").clicked() {
                            ui.close_menu();
                            self.begin_load_single(ctx.clone());
                        }
                        if ui.button("Load Folder...").clicked() {
                            ui.close_menu();
                            self.begin_load_folder(ctx.clone());
                        }
                        if ui.button("Load Monolithic blob_index.dat...").clicked() {
                            ui.close_menu();
                            self.begin_load_monolithic(ctx.clone());
                        }
                        ui.separator();
                        let has_loaded_folder = self.loaded_tags_root().is_some();
                        if ui
                            .add_enabled(has_loaded_folder, egui::Button::new("Open Tags Folder"))
                            .clicked()
                        {
                            ui.close_menu();
                            self.open_loaded_tags_folder();
                        }
                        if ui
                            .add_enabled(has_loaded_folder, egui::Button::new("Open Data Folder"))
                            .clicked()
                        {
                            ui.close_menu();
                            self.open_loaded_data_folder();
                        }
                        ui.menu_button("Recent Folders", |ui| {
                            if self.recent_folders.is_empty() {
                                ui.add_enabled(false, egui::Button::new("No recent folders"));
                            } else {
                                for path in self.recent_folders.clone() {
                                    let full_path = path.display().to_string();
                                    let label = recent_folder_menu_label(&path);
                                    if ui.button(label).on_hover_text(full_path).clicked() {
                                        ui.close_menu();
                                        self.load_recent_folder(path, ctx.clone());
                                    }
                                }
                                ui.separator();
                                if ui.button("Clear Recent Folders").clicked() {
                                    self.recent_folders.clear();
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.separator();
                        if ui.button("Save Current Tag    Ctrl+S").clicked() {
                            ui.close_menu();
                            self.save_current_tag();
                        }
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Save Current Tag As..."),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.save_current_tag_as();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Close Current Tag"),
                            )
                            .clicked()
                        {
                            if let Some(key) = self.selected_key.clone() {
                                self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
                            }
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                !self.open_tabs.is_empty() || !self.floating_tabs.is_empty(),
                                egui::Button::new("Close All Tags"),
                            )
                            .clicked()
                        {
                            self.request_close_action(PendingCloseAction::CloseAllTabs, ctx);
                            ui.close_menu();
                        }
                        ui.separator();
                        let can_fix_dependencies = self.selected_key.is_some()
                            && self.source.as_ref().is_some_and(|source| {
                                matches!(source.source, TagSource::LooseFolder { .. })
                            });
                        if ui
                            .add_enabled(
                                can_fix_dependencies,
                                egui::Button::new("Fix Tag Dependencies"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.fix_current_tag_dependencies();
                        }
                        // Regenerate Index: force a fresh full scan and
                        // overwrite the cached index file.
                        let can_regen = self
                            .source
                            .as_ref()
                            .map(|s| {
                                matches!(s.source, TagSource::LooseFolder { .. })
                                    && s.game.is_some()
                            })
                            .unwrap_or(false);
                        if ui
                            .add_enabled(
                                can_regen && !self.scanning_entries,
                                egui::Button::new("Regenerate Index"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            // Clear cached entries so the scan runs fresh.
                            if let Some(s) = self.source.as_mut() {
                                s.all_entries.clear();
                                s.group_tree = crate::source::build_group_tree(&[]);
                            }
                            self.begin_scan_all_entries(ctx.clone());
                        }
                        ui.separator();
                        if ui.button("Settings...").clicked() {
                            self.settings_open = true;
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui
                            .add_enabled(
                                self.can_undo_current(),
                                egui::Button::new("Undo    Ctrl+Z"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.undo_current_tag();
                        }
                        if ui
                            .add_enabled(
                                self.can_redo_current(),
                                egui::Button::new("Redo    Ctrl+Shift+Z"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.redo_current_tag();
                        }
                    });
                    ui.menu_button("Tools", |ui| {
                        if ui.button("Run Tool...").clicked() {
                            ui.close_menu();
                            self.tool_commands.open = true;
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Find References to Current Tag"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = self.selected_key.clone() {
                                self.show_references_for(&key);
                            }
                        }
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Explore References to Current Tag..."),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = self.selected_key.clone() {
                                self.open_content_explorer(&key);
                            }
                        }
                        if ui.button("Find Unreferenced Tags...").clicked() {
                            ui.close_menu();
                            self.show_unreferenced_tags();
                        }
                        {
                            let is_loose = self.source.as_ref().is_some_and(|source| {
                                matches!(source.source, TagSource::LooseFolder { .. })
                            });
                            let has_index = self
                                .source
                                .as_ref()
                                .is_some_and(|source| source.reverse_dependencies.is_some());
                            let label = if self.building_reverse_dependencies {
                                "Building Reference Index…"
                            } else if has_index {
                                "Rebuild Reference Index"
                            } else {
                                "Build Reference Index"
                            };
                            if ui
                                .add_enabled(
                                    is_loose && !self.building_reverse_dependencies,
                                    egui::Button::new(label),
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.begin_build_reverse_dependencies(ctx.clone(), true);
                            }
                        }
                        if ui.button("List Scenario Map IDs...").clicked() {
                            ui.close_menu();
                            self.show_map_ids();
                        }
                        if ui.button("List Sounds by Class...").clicked() {
                            ui.close_menu();
                            self.show_sounds_by_class();
                        }
                        if ui.button("List Uncompressed Sounds...").clicked() {
                            ui.close_menu();
                            self.show_uncompressed_sounds();
                        }
                        if ui.button("Search Field Values...").clicked() {
                            ui.close_menu();
                            self.field_value_search_open = true;
                        }
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Compare Current Tag With..."),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = self.selected_key.clone() {
                                self.tag_diff = Some(TagDiffState {
                                    a_key: key,
                                    b_key: None,
                                    b_display: None,
                                    results: None,
                                });
                            }
                        }
                        ui.separator();
                        if ui.button("Browse Keywords...").clicked() {
                            ui.close_menu();
                            self.keyword_chooser_open = true;
                        }
                    });
                    ui.menu_button("View", |ui| {
                        if ui
                            .selectable_label(self.browser_mode == BrowserMode::Folders, "Folders")
                            .clicked()
                        {
                            self.browser_mode = BrowserMode::Folders;
                            ui.close_menu();
                        }
                        if ui
                            .selectable_label(
                                self.browser_mode == BrowserMode::Groups,
                                "Tag Groups",
                            )
                            .clicked()
                        {
                            self.browser_mode = BrowserMode::Groups;
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.menu_button(format!("Sort by: {}", self.browser_sort.label()), |ui| {
                            for option in BrowserSort::ALL {
                                if ui
                                    .selectable_label(self.browser_sort == option, option.label())
                                    .clicked()
                                {
                                    self.browser_sort = option;
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.separator();
                        ui.checkbox(&mut self.show_browser_prefixes, "Show [tag]/[folder]");
                        ui.checkbox(&mut self.show_block_sizes, "Show block sizes");
                        ui.checkbox(
                            &mut self.scroll_to_cycle_dropdowns,
                            "Scroll wheel cycles dropdowns",
                        );
                        ui.checkbox(&mut self.expert_mode, "Expert mode");
                        ui.separator();
                        let terminal_enabled = self.terminal_work_dir.is_some();
                        if ui
                            .add_enabled(
                                terminal_enabled,
                                egui::SelectableLabel::new(self.terminal_open, "Terminal"),
                            )
                            .clicked()
                        {
                            self.terminal_open = !self.terminal_open;
                            self.remember_terminal_open_for_game();
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        if ui.button("About...").clicked() {
                            self.help_panel_tab = HelpPanelTab::About;
                            self.about_open = true;
                            ui.close_menu();
                        }
                        if ui.button("Doc...").clicked() {
                            self.help_panel_tab = HelpPanelTab::Doc;
                            self.about_open = true;
                            ui.close_menu();
                        }
                        if ui.button("Map Names...").clicked() {
                            self.help_panel_tab = HelpPanelTab::MapNames;
                            self.about_open = true;
                            ui.close_menu();
                        }
                        if ui.button("Check for updates").clicked() {
                            self.begin_check_for_updates(ctx.clone());
                            ui.close_menu();
                        }
                    });
                    self.draw_tool_launcher_buttons(ui);
                });
            });

        egui::TopBottomPanel::bottom("status")
            .frame(Frame::none().fill(menu_bar()).inner_margin(egui::Margin {
                left: 6.0,
                right: 6.0,
                top: 2.0,
                bottom: 2.0,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Status").strong());
                    ui.separator();
                    ui.label(&self.status);
                    if let Some(progress) = &self.folder_refactor {
                        ui.separator();
                        ui.label(RichText::new(&progress.label).strong());
                        let mut bar = if let Some(value) = progress.progress {
                            egui::ProgressBar::new(value.clamp(0.0, 1.0))
                        } else {
                            egui::ProgressBar::new(0.0).animate(true)
                        };
                        bar = bar
                            .desired_width(180.0)
                            .text(RichText::new(&progress.phase).color(text_dark()));
                        ui.add(bar);
                        ctx.request_repaint();
                    }
                });
            });

        // Terminal panel — rendered AFTER status so it sits above it.
        if self.terminal_open {
            let work_dir_label = self
                .terminal_work_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            egui::TopBottomPanel::bottom("terminal")
                .resizable(true)
                .default_height(180.0)
                .height_range(90.0..=600.0)
                .frame(
                    Frame::none()
                        .fill(foundation_group_bg())
                        .inner_margin(egui::Margin {
                            left: 6.0,
                            right: 6.0,
                            top: 4.0,
                            bottom: 4.0,
                        }),
                )
                .show(ctx, |ui| {
                    // Header pinned to the top of the panel.
                    egui::TopBottomPanel::top("terminal_header")
                        .frame(Frame::none())
                        .show_inside(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(RichText::new("Terminal").color(text_dark()));
                                ui.small(
                                    RichText::new(&work_dir_label)
                                        .color(subtle_dark())
                                        .monospace(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button("×")
                                            .on_hover_text("Close terminal")
                                            .clicked()
                                        {
                                            self.terminal_open = false;
                                            self.remember_terminal_open_for_game();
                                        }
                                        if icon_button(
                                            ui,
                                            ButtonIcon::Clear,
                                            "Clear terminal",
                                            true,
                                            Vec2::new(22.0, 20.0),
                                            text_dark(),
                                        )
                                        .clicked()
                                        {
                                            self.terminal.lines.clear();
                                        }
                                        let open_log_enabled =
                                            self.terminal.last_log_path.is_some();
                                        let mut open_log_button = ui.add_enabled(
                                            open_log_enabled,
                                            egui::Button::new(
                                                RichText::new("Open full log").small(),
                                            ),
                                        );
                                        if let Some(path) = self.terminal.last_log_path.as_ref() {
                                            open_log_button = open_log_button
                                                .on_hover_text(path.display().to_string());
                                        }
                                        if open_log_button.clicked()
                                            && let Some(path) = self.terminal.last_log_path.clone()
                                            && let Err(error) = open_terminal_log(&path)
                                        {
                                            self.status = error;
                                        }
                                        if self.terminal.running {
                                            if self.terminal.process.is_some()
                                                && ui.small_button("Stop").clicked()
                                            {
                                                self.stop_terminal_command();
                                            }
                                            let running_label = self
                                                .terminal
                                                .running_command
                                                .as_deref()
                                                .unwrap_or("running...");
                                            ui.small(
                                                RichText::new(running_label)
                                                    .color(subtle_dark())
                                                    .monospace(),
                                            );
                                        }
                                    },
                                );
                            });
                            ui.add_space(2.0);
                        });

                    // Input row pinned to the bottom of the panel.
                    egui::TopBottomPanel::bottom("terminal_input")
                        .frame(Frame::none())
                        .show_inside(ui, |ui| {
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(">").monospace().color(subtle_dark()));
                                // Reserve a fixed width for the Run button on
                                // the right; the TextEdit fills the rest. (Do
                                // NOT wrap the button in a right_to_left layout
                                // — that consumes all remaining width and leaves
                                // nothing for the input field.)
                                let button_w = 52.0;
                                let text_w = (ui.available_width() - button_w - 8.0).max(40.0);
                                let resp = ui.add_enabled(
                                    !self.terminal.running,
                                    egui::TextEdit::singleline(&mut self.terminal.input)
                                        .desired_width(text_w)
                                        .font(egui::TextStyle::Monospace)
                                        .hint_text("tool <command> …"),
                                );
                                if self.terminal.refocus_input && !self.terminal.running {
                                    resp.request_focus();
                                    self.terminal.refocus_input = false;
                                }
                                let run_clicked = ui
                                    .add_enabled(!self.terminal.running, egui::Button::new("Run"))
                                    .clicked();
                                let enter = resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if resp.has_focus() && !self.terminal.running {
                                    let recall = ui.input(|i| {
                                        if i.key_pressed(egui::Key::ArrowUp) {
                                            -1
                                        } else if i.key_pressed(egui::Key::ArrowDown) {
                                            1
                                        } else {
                                            0
                                        }
                                    });
                                    if recall != 0 {
                                        self.recall_terminal_history(recall);
                                        resp.request_focus();
                                    }
                                }
                                if run_clicked || enter {
                                    self.begin_terminal_command(ctx.clone());
                                    // Refocus the input so the user can keep typing.
                                    resp.request_focus();
                                }
                            });
                        });

                    // Output fills the remaining center space. The CentralPanel
                    // bounds the scroll area exactly, so there's no available_height
                    // feedback to fight the resize handle.
                    egui::CentralPanel::default()
                        .frame(
                            Frame::none()
                                .fill(Color32::from_rgb(24, 24, 23))
                                .inner_margin(egui::Margin {
                                    left: 6.0,
                                    right: 6.0,
                                    top: 4.0,
                                    bottom: 4.0,
                                }),
                        )
                        .show_inside(ui, |ui| {
                            let want_scroll_bottom = self.terminal.scroll_to_bottom;
                            self.terminal.scroll_to_bottom = false;
                            egui::ScrollArea::vertical()
                                .id_salt("terminal_output")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.visuals_mut().override_text_color = None;
                                    ui.set_min_width(ui.available_width());
                                    for line in &self.terminal.lines {
                                        let mut text = RichText::new(&line.text)
                                            .color(terminal_line_color(line.severity));
                                        if terminal_line_is_strong(line.severity) {
                                            text = text.font(bold_font(13.0)).strong();
                                        } else {
                                            text = text.monospace().font(FontId::monospace(13.0));
                                        }
                                        ui.add(egui::Label::new(text).wrap());
                                    }
                                    if want_scroll_bottom {
                                        ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                                    }
                                });
                        });
                });
        }

        egui::SidePanel::left("tag_browser")
            .resizable(true)
            .default_width(330.0)
            .frame(Frame::none().fill(left_panel()).inner_margin(egui::Margin {
                left: 8.0,
                right: 8.0,
                top: 6.0,
                bottom: 6.0,
            }))
            .show(ctx, |ui| {
                let sidebar_header = self.source.as_ref().map(|source| {
                    (
                        source.game.clone(),
                        source.source.origin_label(),
                        sidebar_source_path_label(&source.source),
                    )
                });
                if let Some((Some(game), _origin, path_label)) = sidebar_header.as_ref() {
                    draw_game_banner_header(ui, self, game, path_label);
                } else {
                    ui.heading(RichText::new("Tags").color(text_dark()));
                    if let Some((_, origin, _)) = sidebar_header.as_ref() {
                        ui.small(RichText::new(origin).color(subtle_dark()));
                        ui.add_space(8.0);
                    }
                }

                if let Some(source) = &mut self.source {
                    ui.add_space(8.0);
                    let scanning = self.scanning_entries;
                    // Collect deferred scan-trigger here; execute after borrow ends.
                    let mut need_scan = false;
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.browser_mode,
                            BrowserMode::Folders,
                            "Folders",
                        );
                        let groups_btn = ui.selectable_value(
                            &mut self.browser_mode,
                            BrowserMode::Groups,
                            "Groups",
                        );
                        if groups_btn.clicked()
                            && matches!(source.source, TagSource::LooseFolder { .. })
                            && source.all_entries.is_empty()
                            && !scanning
                        {
                            need_scan = true;
                        }
                    });
                    ui.checkbox(&mut self.show_browser_prefixes, "Show prefixes");
                    ui.add_space(6.0);
                    let prev_filter_empty = self.filter.is_empty();
                    ui.scope(|ui| {
                        ui.visuals_mut().override_text_color = Some(text_dark());
                        let search_bg = if is_dark_mode() {
                            Color32::from_rgb(48, 48, 46)
                        } else {
                            Color32::from_rgb(246, 246, 244)
                        };
                        let search_hover = if is_dark_mode() {
                            Color32::from_rgb(58, 58, 55)
                        } else {
                            Color32::from_rgb(255, 255, 252)
                        };
                        ui.visuals_mut().extreme_bg_color = search_bg;
                        ui.visuals_mut().widgets.inactive.bg_fill = search_bg;
                        ui.visuals_mut().widgets.hovered.bg_fill = search_hover;
                        ui.visuals_mut().widgets.active.bg_fill = search_hover;
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filter)
                                .hint_text("search tags")
                                .desired_width(f32::INFINITY),
                        );
                    });
                    if let Some(warning) = browser::browser_filter_warning(&self.filter) {
                        ui.label(
                            RichText::new(warning)
                                .small()
                                .color(Color32::from_rgb(184, 134, 11)),
                        );
                    }
                    if prev_filter_empty
                        && !self.filter.is_empty()
                        && matches!(source.source, TagSource::LooseFolder { .. })
                        && source.all_entries.is_empty()
                        && !scanning
                    {
                        need_scan = true;
                    }
                    ui.add_space(4.0);
                    let selected = self.selected_key.clone();
                    let filter = self.filter.trim().to_owned();
                    let mode = self.browser_mode;
                    let show_prefixes = self.show_browser_prefixes;
                    let folders_before_tags = self.folders_before_tags;
                    let double_click_to_open = self.double_click_to_open_tags;
                    let mut status_update = None;
                    // Groups and filtered Folders use all_entries (background
                    // scan) so every tag is visible, not just visited folders.
                    let has_all = !source.all_entries.is_empty();
                    let groups_mode = matches!(mode, BrowserMode::Groups);
                    // One-shot "reveal in tree" request (force-open ancestors +
                    // scroll). Borrowed into the Copy `Reveal` for the draw.
                    let reveal_owned = self.reveal_target.take();
                    let reveal = reveal_owned.as_ref().map(|request| Reveal {
                        key: request.key.as_str(),
                        remaining: request.ancestors.as_slice(),
                    });
                    let sort = self.browser_sort;
                    let action = if !filter.is_empty() {
                        // Active search: render a *pruned* tree containing only
                        // the matching tags, with folders collapsed so the user
                        // drills down to find them. The pruned tree is memoized
                        // in `filter_cache` (rebuilt once per keystroke, not per
                        // frame), and collapsed folders don't build their
                        // children — so per-frame cost stays bounded.
                        let entries: &[TagEntry] = if has_all {
                            &source.all_entries
                        } else {
                            &source.entries
                        };
                        if scanning && !has_all {
                            ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new("Indexing tags…")
                                            .color(subtle_dark())
                                            .small(),
                                    );
                                    None
                                })
                                .inner
                        } else {
                            self.filter_cache.refresh(
                                self.source_generation,
                                &filter,
                                entries,
                                has_all,
                                groups_mode,
                            );
                            let cache = &self.filter_cache;
                            ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    if cache.entries.is_empty() {
                                        ui.label(
                                            RichText::new("No matching tags").color(subtle_dark()),
                                        );
                                        return None;
                                    }
                                    // Empty filter → tree renders every (already
                                    // pruned) entry with folders collapsed.
                                    draw_tree(
                                        ui,
                                        &cache.tree,
                                        &cache.entries,
                                        selected.as_deref(),
                                        "",
                                        show_prefixes,
                                        double_click_to_open,
                                        groups_mode,
                                        reveal,
                                        sort,
                                        !groups_mode && folders_before_tags,
                                    )
                                })
                                .inner
                        }
                    } else {
                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| match mode {
                                BrowserMode::Folders => {
                                    if let TagSource::LooseFolder { root, .. } = &source.source {
                                        let root = root.clone();
                                        draw_tree_lazy(
                                            ui,
                                            &mut source.tree,
                                            &mut source.entries,
                                            &mut source.group_tree,
                                            &root,
                                            &source.names,
                                            selected.as_deref(),
                                            &filter,
                                            show_prefixes,
                                            double_click_to_open,
                                            &mut status_update,
                                            reveal,
                                            sort,
                                            folders_before_tags,
                                        )
                                    } else {
                                        draw_tree(
                                            ui,
                                            &source.tree,
                                            &source.entries,
                                            selected.as_deref(),
                                            &filter,
                                            show_prefixes,
                                            double_click_to_open,
                                            false,
                                            reveal,
                                            sort,
                                            folders_before_tags,
                                        )
                                    }
                                }
                                BrowserMode::Groups => {
                                    if scanning && !has_all {
                                        ui.label(
                                            RichText::new("Indexing tags…")
                                                .color(subtle_dark())
                                                .small(),
                                        );
                                        None
                                    } else {
                                        let entries = if has_all {
                                            &source.all_entries[..]
                                        } else {
                                            &source.entries[..]
                                        };
                                        draw_tree(
                                            ui,
                                            &source.group_tree,
                                            entries,
                                            selected.as_deref(),
                                            &filter,
                                            show_prefixes,
                                            double_click_to_open,
                                            true,
                                            reveal,
                                            sort,
                                            false,
                                        )
                                    }
                                }
                            })
                            .inner
                    };
                    if let Some(status) = status_update {
                        self.status = status;
                    }
                    if let Some(action) = action {
                        self.handle_browser_action(action, ctx.clone());
                    }
                    // Deferred: begin_scan_all_entries needs &mut self, so
                    // it must be called after the `source` borrow ends.
                    if need_scan {
                        self.begin_scan_all_entries(ctx.clone());
                    }
                    // Auto-build the reverse-dependency index so find-references
                    // / unreferenced / Content Explorer work without first
                    // running a move/rename. If the full entry list isn't in
                    // yet this kicks the scan first, then builds when it lands.
                    // Idempotent and self-gating (see the method + scan guard).
                    if !self.building_reverse_dependencies
                        && self.source.as_ref().is_some_and(|source| {
                            matches!(source.source, TagSource::LooseFolder { .. })
                                && source.reverse_dependencies.is_none()
                        })
                    {
                        self.begin_build_reverse_dependencies(ctx.clone(), false);
                    }
                } else {
                    ui.label("Use File to load a tag, folder, or monolithic cache.");
                }
            });

        egui::CentralPanel::default()
            .frame(Frame::none().fill(editor_bg()).inner_margin(egui::Margin {
                left: 10.0,
                right: 10.0,
                top: 8.0,
                bottom: 8.0,
            }))
            .show(ctx, |ui| {
                if !self.open_tabs.is_empty() || self.dragging_floating_tab.is_some() {
                    let mut close_key = None;
                    let mut pop_key = None;
                    let mut close_all = false;
                    let mut close_all_but = None;
                    let mut reveal_key = None;
                    let mut rack_rect = None;
                    if self.open_tabs.is_empty() {
                        let response = ui.label(
                            RichText::new("Drop popped tag here")
                                .color(subtle_dark())
                                .strong(),
                        );
                        rack_rect = Some(response.rect);
                    } else {
                        const TAB_BUTTON_SIZE: f32 = 18.0;
                        const TAB_MIN_LABEL_WIDTH: f32 = 48.0;
                        const TAB_MAX_LABEL_WIDTH: f32 = 170.0;
                        const TAB_SIDE_PADDING: f32 = 8.0;
                        const TAB_INNER_GAP: f32 = 3.0;

                        let available_width = ui.available_width().max(120.0);
                        let row_gap = 3.0;
                        // (key, label, active, dirty, label_width, group_tag)
                        let mut rows = Vec::<Vec<(String, String, bool, bool, f32, u32)>>::new();
                        let mut row = Vec::new();
                        let mut row_width = 0.0;

                        for key in self.open_tabs.clone() {
                            let Some(entry) = self.entry_for_key(&key) else {
                                continue;
                            };
                            let active = self.selected_key.as_deref() == Some(key.as_str());
                            let dirty = self
                                .parsed_tags
                                .get(&key)
                                .map(|doc| doc.dirty)
                                .unwrap_or(false);
                            let label = if dirty {
                                format!("● {}", tag_tab_label(entry))
                            } else {
                                tag_tab_label(entry)
                            };
                            let label_width = tab_label_width(
                                ui,
                                &label,
                                TAB_MIN_LABEL_WIDTH,
                                TAB_MAX_LABEL_WIDTH,
                            );
                            let tab_width = TAB_SIDE_PADDING
                                + 16.0
                                + TAB_INNER_GAP
                                + label_width
                                + TAB_INNER_GAP
                                + TAB_BUTTON_SIZE
                                + TAB_INNER_GAP
                                + TAB_BUTTON_SIZE;
                            let next_width = if row.is_empty() {
                                tab_width
                            } else {
                                row_width + row_gap + tab_width
                            };
                            if !row.is_empty() && next_width > available_width {
                                rows.push(row);
                                row = Vec::new();
                                row_width = 0.0;
                            }
                            if !row.is_empty() {
                                row_width += row_gap;
                            }
                            row_width += tab_width;
                            row.push((key, label, active, dirty, label_width, entry.group_tag));
                        }
                        if !row.is_empty() {
                            rows.push(row);
                        }

                        for row in rows {
                            let row_response = ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = row_gap;
                                for (key, label, active, dirty, label_width, group_tag) in row {
                                    let shown_label = truncate_for_cell(&label, label_width);
                                    let base_fill = if active { menu_bar() } else { row_type() };
                                    // Subtle amber tint flags tabs with unsaved edits
                                    // (on top of the ● marker in the label).
                                    let fill = if dirty {
                                        tint_toward(
                                            base_fill,
                                            Color32::from_rgb(184, 134, 11),
                                            0.20,
                                        )
                                    } else {
                                        base_fill
                                    };
                                    let tab_response = Frame::none()
                                        .fill(fill)
                                        .stroke(Stroke::new(1.0, grid_line()))
                                        .inner_margin(egui::Margin {
                                            left: 3.0,
                                            right: 3.0,
                                            top: 2.0,
                                            bottom: 2.0,
                                        })
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = TAB_INNER_GAP;
                                                draw_tag_icon(ui, group_tag, 16.0);
                                                let label_response = ui
                                                    .add_sized(
                                                        Vec2::new(label_width, 18.0),
                                                        egui::SelectableLabel::new(
                                                            active,
                                                            RichText::new(shown_label.clone())
                                                                .color(text_dark())
                                                                .strong(),
                                                        ),
                                                    )
                                                    .on_hover_text(label.clone());
                                                if label_response.clicked() {
                                                    self.selected_key = Some(key.clone());
                                                    self.ensure_tag_loading(
                                                        key.clone(),
                                                        ctx.clone(),
                                                    );
                                                }
                                                if label_response.middle_clicked() {
                                                    close_key = Some(key.clone());
                                                }
                                                label_response.context_menu(|ui| {
                                                    if ui.button("Reveal in browser").clicked() {
                                                        reveal_key = Some(key.clone());
                                                        ui.close_menu();
                                                    }
                                                    ui.separator();
                                                    if ui.button("Close all").clicked() {
                                                        close_all = true;
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Close all but this").clicked() {
                                                        close_all_but = Some(key.clone());
                                                        ui.close_menu();
                                                    }
                                                });
                                                if ui
                                                    .add(
                                                        egui::Button::new("⇱")
                                                            .min_size(Vec2::splat(TAB_BUTTON_SIZE)),
                                                    )
                                                    .on_hover_text("Pop tab out")
                                                    .clicked()
                                                {
                                                    pop_key = Some(key.clone());
                                                }
                                                if ui
                                                    .add(
                                                        egui::Button::new("x")
                                                            .min_size(Vec2::splat(TAB_BUTTON_SIZE)),
                                                    )
                                                    .on_hover_text("Close tab")
                                                    .clicked()
                                                {
                                                    close_key = Some(key.clone());
                                                }
                                            });
                                        });
                                    if tab_response.response.middle_clicked() {
                                        close_key = Some(key.clone());
                                    }
                                }
                            });
                            rack_rect = Some(match rack_rect {
                                Some(rect) => rect.union(row_response.response.rect),
                                None => row_response.response.rect,
                            });
                        }
                    }
                    if close_all {
                        self.request_close_action(PendingCloseAction::CloseAllTabs, ctx);
                    } else if let Some(key) = close_all_but {
                        self.request_close_action(PendingCloseAction::CloseAllButThis(key), ctx);
                    } else if let Some(key) = close_key {
                        self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
                    } else if let Some(key) = pop_key {
                        self.pop_tab(&key);
                    }
                    if let Some(key) = reveal_key {
                        self.reveal_in_browser(&key);
                    }
                    self.tab_rack_rect = rack_rect;
                    ui.add_space(6.0);
                } else {
                    self.tab_rack_rect = None;
                }

                if let Some(entry) = self.selected_entry().cloned() {
                    let selected_key = entry.key.clone();
                    draw_entry_header(ui, &entry, &self.names);
                    self.draw_keyword_bar(ui, &selected_key);

                    // "Search fields" collapses the editor to matching blocks.
                    // Not offered for shader/sound tags (their own surfaces).
                    let supports_field_search = supports_field_search(&entry);
                    if supports_field_search {
                        self.draw_field_search_bar(ui, &selected_key);
                    }

                    let mut bitmap_reimport_request = None;
                    // Documentation overlay (fetched before borrowing parsed_tags).
                    let def_docs = self.def_docs_for_entry(&entry);
                    if let Some(doc) = self.parsed_tags.get_mut(&selected_key) {
                        let mut pending = Vec::new();
                        let mut block_ops = Vec::new();
                        let mut shader_ops = Vec::new();
                        let mut shader_param_ops = Vec::new();
                        let mut h2_shader_param_ops = Vec::new();
                        let mut function_data_ops = Vec::new();
                        let mut model_variant_ops = Vec::new();
                        let mut color_request = None;
                        let mut function_request = None;
                        let mut block_clip_request = None;
                        let mut bitmap_reimport = None;
                        let mut tsv_paste_request = None;
                        let field_filter = compute_pending_field_filter(
                            &doc.tag,
                            supports_field_search,
                            &selected_key,
                            &self.field_search,
                            &mut self.field_search_applied,
                        );
                        let sound_volume = self.audio.volume();
                        let mut edit_context = FieldEditContext {
                            view_scope: "docked",
                            tag_key: &selected_key,
                            group_tag: entry.group_tag,
                            root: Some(doc.tag.root()),
                            game: self
                                .source
                                .as_ref()
                                .and_then(|source| source.game.as_deref()),
                            definitions_root: self.source.as_ref().and_then(|source| match &source
                                .source
                            {
                                TagSource::LooseFolder {
                                    definitions_root, ..
                                } => Some(definitions_root.as_path()),
                                _ => None,
                            }),
                            names: Some(&self.names),
                            definition_group_name: self
                                .names
                                .name_for(entry.group_tag)
                                .or_else(|| group_tag_to_extension(entry.group_tag)),
                            tags_root: self.source.as_ref().and_then(|source| {
                                match &source.source {
                                    TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                                    _ => None,
                                }
                            }),
                            status: Some(&mut self.status),
                            editable: is_editable_tag(&entry, &doc.tag),
                            show_block_sizes: self.show_block_sizes,
                            buffers: &mut self.edit_buffers,
                            pending: &mut pending,
                            block_ops: &mut block_ops,
                            block_confirm: &mut self.block_confirm,
                            open_request: &mut self.pending_open,
                            sound_play_request: &mut self.audio.pending,
                            sound_status: self.audio.status.as_deref(),
                            sound_volume,
                            sound_extract_request: &mut self.pending_sound_extract,
                            sound_language: self.audio.language.as_deref(),
                            tool_import: &mut self.pending_tool_import,
                            bitmap_reimport: &mut bitmap_reimport,
                            shader_ops: &mut shader_ops,
                            shader_param_ops: &mut shader_param_ops,
                            h2_shader_param_ops: &mut h2_shader_param_ops,
                            function_data_ops: &mut function_data_ops,
                            model_variant_ops: &mut model_variant_ops,
                            color_request: &mut color_request,
                            function_request: &mut function_request,
                            docs: def_docs.as_deref(),
                            tsv_paste_request: &mut tsv_paste_request,
                            block_clipboard: self.block_clipboard.as_ref(),
                            block_clip_request: &mut block_clip_request,
                            field_filter: field_filter.as_ref(),
                            field_nav: self.field_nav.as_ref(),
                        };
                        if is_bitmap_tag(&entry) {
                            let preview = self
                                .bitmap_previews
                                .entry(selected_key.clone())
                                .or_default();
                            draw_bitmap_tag(
                                ui,
                                ctx,
                                &doc.tag,
                                &entry,
                                &self.names,
                                &mut self.color_popup,
                                preview,
                                self.expert_mode,
                                &mut edit_context,
                            );
                        } else {
                            let mut local_model_preview;
                            let model_preview = if is_model_group(entry.group_tag, &self.names) {
                                self.model_previews.entry(selected_key.clone()).or_default()
                            } else {
                                local_model_preview = ModelPreviewState::default();
                                &mut local_model_preview
                            };
                            draw_tag(
                                ui,
                                &doc.tag,
                                &entry,
                                &self.names,
                                self.source.as_ref().map(|source| &source.source),
                                &mut self.rmdf_cache,
                                &mut self.rmop_cache,
                                &mut self.color_popup,
                                &mut self.function_popup,
                                model_preview,
                                &mut self.model_preview_size,
                                self.expert_mode,
                                &mut edit_context,
                            );
                        }
                        // Snapshot for undo before a mutating batch. Coalesces
                        // continuous edits into one entry; closes the window on
                        // frames with no edits.
                        if !pending.is_empty()
                            || !block_ops.is_empty()
                            || !shader_ops.is_empty()
                            || !shader_param_ops.is_empty()
                            || !model_variant_ops.is_empty()
                        {
                            doc.journal.begin_edit(&doc.tag, "Edit");
                        } else {
                            doc.journal.end_edit_window();
                        }
                        if let Some(status) =
                            apply_pending_edits(&mut doc.tag, pending, &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_block_ops(&mut doc.tag, block_ops, &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_shader_ops(&mut doc.tag, shader_ops, &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_shader_param_ops(&mut doc.tag, shader_param_ops, &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        if let Some(status) = apply_h2_shader_param_ops(
                            &mut doc.tag,
                            h2_shader_param_ops,
                            &mut doc.dirty,
                        ) {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_function_data_ops(&mut doc.tag, function_data_ops, &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_model_variant_ops(&mut doc.tag, model_variant_ops, &mut doc.dirty)
                        {
                            self.status = status;
                            if let Some(preview) = self.model_previews.get_mut(&selected_key) {
                                preview.loaded_key = None;
                                preview.data = None;
                            }
                        }
                        // A color swatch was clicked: open the shared picker.
                        if let Some(popup) = color_request {
                            self.color_popup = Some(popup);
                        }
                        if let Some(popup) = function_request {
                            self.function_popup = Some(popup);
                        }
                        // Element(s) were copied: stash them on the clipboard.
                        if let Some(clip) = block_clip_request {
                            self.status = format!(
                                "Copied {} '{}' element(s)",
                                clip.elements.len(),
                                clip.label
                            );
                            self.block_clipboard = Some(clip);
                        }
                        // "Paste TSV…" was chosen: open the import window.
                        if let Some(req) = tsv_paste_request {
                            self.tsv_paste = Some(TsvPasteState {
                                tag_key: selected_key.clone(),
                                block_path: req.block_path,
                                block_label: req.block_label,
                                element_count: req.element_count,
                                text: String::new(),
                                status: None,
                            });
                        }
                        bitmap_reimport_request = bitmap_reimport;
                    } else if self.loading_tags.contains(&selected_key) {
                        ui.label("Loading tag data...");
                    } else {
                        ui.label("Select the tag again to load it.");
                    }
                    if let Some(key) = bitmap_reimport_request {
                        self.begin_reimport_bitmap(key, ctx.clone());
                    }
                } else {
                    ui.heading("No tag selected");
                    ui.label("Load a source from File, then select a tag in the browser.");
                }
            });
        self.draw_settings_window(ctx);
        self.draw_tool_commands_window(ctx);
        self.draw_new_tag_window(ctx);
        self.draw_about_window(ctx);
        self.draw_query_results_window(ctx);
        self.draw_tag_diff_window(ctx);
        self.draw_content_explorer_window(ctx);
        self.draw_keyword_chooser_window(ctx);
        self.draw_field_value_search_window(ctx);
        self.draw_tsv_paste_window(ctx);
        self.draw_rename_tag_window(ctx);
        self.persist_prefs_if_changed();
        self.keywords.save_if_dirty();
        self.draw_floating_tabs(ctx);
        self.handle_floating_tab_drop(ctx);
        if let Some(result) = draw_color_popup(
            ctx,
            &mut self.color_popup,
            &mut self.custom_color_swatches,
            &mut self.palette_last_dir,
        ) {
            match result {
                ColorPopupResult::FieldEdit { tag_key, edit } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        doc.journal.begin_edit(&doc.tag, "Edit color");
                        if let Some(status) =
                            apply_pending_edits(&mut doc.tag, vec![edit], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        doc.journal.end_edit_window();
                    }
                }
                ColorPopupResult::ShaderOp { tag_key, op } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        doc.journal.begin_edit(&doc.tag, "Shader edit");
                        if let Some(status) =
                            apply_shader_ops(&mut doc.tag, vec![op], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        doc.journal.end_edit_window();
                    }
                }
                ColorPopupResult::ShaderParamOp { tag_key, op } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        doc.journal.begin_edit(&doc.tag, "Shader parameter");
                        if let Some(status) =
                            apply_shader_param_ops(&mut doc.tag, vec![op], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        doc.journal.end_edit_window();
                    }
                }
                ColorPopupResult::H2ShaderParamOp { tag_key, op } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        doc.journal.begin_edit(&doc.tag, "Shader parameter");
                        if let Some(status) =
                            apply_h2_shader_param_ops(&mut doc.tag, vec![op], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        doc.journal.end_edit_window();
                    }
                }
            }
        }
        if let Some(batch) = draw_function_popup(ctx, &mut self.function_popup) {
            if let Some(doc) = self.parsed_tags.get_mut(&batch.tag_key) {
                if !batch.edits.is_empty() || !batch.data_ops.is_empty() {
                    doc.journal.begin_edit(&doc.tag, "Edit function");
                }
                if let Some(status) = apply_pending_edits(&mut doc.tag, batch.edits, &mut doc.dirty)
                {
                    self.status = status;
                }
                if let Some(status) =
                    apply_function_data_ops(&mut doc.tag, batch.data_ops, &mut doc.dirty)
                {
                    self.status = status;
                }
                doc.journal.end_edit_window();
            }
        }
        self.handle_block_confirm(ctx);
        self.handle_save_changes_prompt(ctx);
        self.handle_last_opened_windows_prompt(ctx);
        self.process_pending_open(ctx);
        self.apply_field_nav(ctx);
        // Drain queued sound-player actions: resolve the permutation against the
        // FMOD banks, decode (cached), and play/stop. Runs every frame so voices
        // are reaped even when idle; the tags root is only cloned when acting.
        let sound_root = if self.audio.pending.is_some() {
            self.source_tags_root().map(std::path::Path::to_path_buf)
        } else {
            None
        };
        self.audio.process(sound_root.as_deref(), ctx);
        // Drain a queued sound extraction (decode + write files off the render
        // hot loop) and a reimport hand-off (opens the tool runner pre-filled).
        if let Some(request) = self.pending_sound_extract.take() {
            self.audio.run_extract(request);
            if let Some(status) = self.audio.status.clone() {
                self.status = status;
            }
        }
        // While the Wwise index builds off-thread, keep repainting so the drain
        // loop polls it (the worker also pings on completion, but this covers
        // the "loading…" status update).
        if self.audio.is_busy() {
            ctx.request_repaint();
        }
        self.process_pending_tool_import(ctx);
    }
}

fn recent_folder_menu_label(path: &Path) -> String {
    const MAX_CHARS: usize = 54;
    let text = path.display().to_string();
    let count = text.chars().count();
    if count <= MAX_CHARS {
        return text;
    }
    let keep = MAX_CHARS.saturating_sub(3);
    let tail = text
        .chars()
        .rev()
        .take(keep)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("...{tail}")
}
