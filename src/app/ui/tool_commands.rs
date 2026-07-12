//! Tool-command catalog, argument editor, and launch controls.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

impl Baboon {
    pub(super) fn draw_tool_commands_window(&mut self, ctx: &egui::Context) {
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

    pub(super) fn ensure_tool_commands_loaded(&mut self, game: &str) {
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

    pub(super) fn draw_tool_command_list(&mut self, ui: &mut Ui) {
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

    pub(super) fn draw_selected_tool_command(&mut self, ui: &mut Ui, ctx: &egui::Context) {
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

    pub(super) fn selected_tool_command(&self) -> Option<&ToolCommand> {
        let selected = self.tool_commands.selected.as_deref()?;
        self.tool_commands
            .commands
            .iter()
            .find(|command| command.name == selected)
    }

    pub(super) fn draw_tool_command_arg(
        &mut self,
        ui: &mut Ui,
        command: &ToolCommand,
        arg: &ToolCommandArg,
    ) {
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

    pub(super) fn pick_tool_command_path(&self, kind: ToolCommandArgKind) -> Option<String> {
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
}
