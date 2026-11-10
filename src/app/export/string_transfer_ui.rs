use super::*;

fn described_option<T: PartialEq>(ui: &mut crate::app::ui_kit::ValueComboUi<'_>, value: &mut T, option: T, title: &str, description: &str) -> egui::Response {
    let mut text = egui::text::LayoutJob::default();
    text.append(title, 0.0, egui::TextFormat::simple(TextStyle::Body.resolve(ui.style()), text_dark()));
    text.append(&format!("\n{description}"), 0.0, egui::TextFormat::simple(TextStyle::Small.resolve(ui.style()), subtle_dark()));
    ui.selectable_value(value, option, text)
}

impl StringTransferWindow {
    fn options(&self) -> StringTransferOptions {
        StringTransferOptions { format: self.format, mode: self.mode, destination: self.destination }
    }

    fn clear_preview(&mut self) {
        self.preview = None;
        self.error = None;
    }
}

impl Dialog for StringTransferWindow {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let Some(index) = cx.model.resolve_kit(self.stamp.kit) else { return false; };
        let kit = &cx.model.kits[index];
        let available = kit.source_session == self.source_session;
        let document = available.then(|| kit.parsed_tags.get(&self.key)).flatten();
        let title = if self.import { "Import Strings Settings" } else { "Extract Strings Settings" };
        let action_icon = if self.import { ButtonIcon::Import } else { ButtonIcon::Export };
        let mut open = true;
        if document.is_none() {
            egui::Window::new(title).id(egui::Id::unique("string_list_transfer")).open(&mut open).show(cx.egui, |ui| {
                ui.label("The source or tag is no longer available. Close this window and reopen it from the intended tag.");
            });
            return open;
        }
        self.stamp.generation = kit.generation;
        let document = document.unwrap();
        let revision = (document.id, document.dirty.revision());
        if self.revision != Some(revision) {
            self.list = strings::read_string_list(&document.tag).map(std::sync::Arc::new);
            self.revision = Some(revision);
            self.clear_preview();
        }
        let Some(list) = self.list.clone() else { return open; };
        let source = kit.source.as_ref();
        let game = source.and_then(|source| source.game);
        let definitions = source.and_then(|source| match &source.source {
            TagSource::LooseFolder { definitions_root, .. } => Some(definitions_root.as_path()), _ => None,
        });
        let syntax = list.storage.text_syntax(crate::app::editor::string_list::text_syntax(definitions, game));
        let layout = cx.model.kit_layout_for(index);
        let tool_command = if list.storage == Storage::HudMessages { Err("Use Baboon to import or export HUD messages.".to_owned()) } else { tool_string_command(game, self.import, &self.relative) };
        let text_format = format!(".{}", list.storage.text_extension());
        let tool_error = tool_command.as_ref().err().cloned().or_else(|| {
            if layout.as_ref().is_none_or(|layout| !layout.root.join("tool.exe").is_file()) { Some("tool.exe was not found in this editing kit".to_owned()) }
            else if cx.model.editing_kit_is_read_only(index) { Some("tool.exe requires a writable editing kit".to_owned()) }
            else { None }
        });
        let tool_dirty = if self.import { kit.parsed_tags.values().any(|document| document.dirty.is_set()) } else { document.dirty.is_set() };
        let mut run = false;
        let mut cancel = false;
        let mut changed = false;
        let mut can_refresh = false;
        let mut browse = false;
        egui::Window::new(title).id(egui::Id::unique("string_list_transfer"))
            .collapsible(false).resizable([true, false]).title_bar(false)
            .frame(egui::Frame::window(&cx.egui.global_style()).inner_margin(egui::Margin {
                left: 20, right: 20, top: 0, bottom: 0,
            }))
            .min_width(window_width(cx.egui, 460.0))
            .default_width(window_width(cx.egui, 500.0)).constrain_to(window_work_area(cx.egui))
            .show(cx.egui, |ui| {
                ui.spacing_mut().window_margin = egui::Margin { left: 20, right: 20, top: 0, bottom: 0 };
                crate::app::search::draw_icon_window_header_without_close(ui, title, action_icon);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add(button_icon_image(ui, ButtonIcon::UnicodeStringList, text_dark(), 16.0));
                    let path = native_display_path(&self.display_path);
                    let (parent, file) = path.rsplit_once('\\').map_or(("", path.as_str()), |(parent, file)| (&path[..parent.len() + 1], file));
                    let mut job = egui::text::LayoutJob::default();
                    let font = TextStyle::Body.resolve(ui.style());
                    job.append(parent, 0.0, egui::TextFormat::simple(font.clone(), subtle_dark()));
                    job.append(file, 0.0, egui::TextFormat::simple(bold_font(font.size), text_dark()));
                    ui.add(egui::Label::new(job).truncate()).on_hover_text(path);
                });
                ui.add_space(12.0);
                let label_font = TextStyle::Body.resolve(ui.style());
                let label_width = ["Method:", "Format:", "Language:", "Destination:", "Import mode:"]
                    .iter().map(|label| ui.painter().layout_no_wrap((*label).to_owned(), label_font.clone(), text_dark()).size().x)
                    .fold(70.0_f32, f32::max).ceil();
                let field_width = (ui.available_width() - label_width - 8.0).max(120.0);
                let mut settings_changed = false;
                egui::Grid::new("string transfer settings").num_columns(2).min_col_width(label_width).spacing(Vec2::new(8.0, 8.0)).show(ui, |ui| {
                    ui.label("Method:");
                    crate::app::ui_kit::ValueComboBox::from_id_salt("strings method").width(field_width)
                        .selected_text(if self.method == StringMethod::Baboon { "Baboon" } else { "tool.exe" }).show_ui(ui, |ui| {
                            settings_changed |= described_option(ui, &mut self.method, StringMethod::Baboon, "Baboon",
                                "Uses the open tag. Supports text and CSV; imports can be undone.").changed();
                            settings_changed |= described_option(ui, &mut self.method, StringMethod::Tool, "tool.exe",
                                "Uses the engine's saved files and language folders. Imports write directly to disk.").changed();
                        });
                    ui.end_row();
                    let baboon = self.method == StringMethod::Baboon;
                    ui.label("Format:");
                    ui.add_enabled_ui(baboon, |ui| {
                        crate::app::ui_kit::ValueComboBox::from_id_salt("strings format").width(field_width)
                            .selected_text(if !baboon || self.format == StringFormat::Text { text_format.as_str() } else { ".csv" }).show_ui(ui, |ui| {
                                settings_changed |= described_option(ui, &mut self.format, StringFormat::Text, &text_format,
                                    "Engine-style text, with a separate file for each language. Exports use UTF-16.").changed();
                                settings_changed |= described_option(ui, &mut self.format, StringFormat::Csv, ".csv",
                                    "Spreadsheet format: one row per string, one column per language. Exports use UTF-8.").changed();
                            }).response.on_hover_text("Text exports use UTF-16. CSV exports use UTF-8 with a BOM. Imports accept UTF-8 and UTF-16.");
                    });
                    ui.end_row();
                    ui.label("Language:");
                    let csv_import = self.import && self.format == StringFormat::Csv;
                    let language_label = if !baboon || csv_import { "All" } else if list.languages.len() == 1 { "Text" }
                        else { self.language.map_or("All", |language| list.languages[language].name.as_str()) };
                    ui.add_enabled_ui(baboon && !csv_import && list.languages.len() > 1, |ui| {
                        crate::app::ui_kit::ValueComboBox::from_id_salt("strings language").width(field_width).selected_text(language_label).show_ui(ui, |ui| {
                            settings_changed |= ui.selectable_value(&mut self.language, None, "All").changed();
                            for (index, language) in list.languages.iter().enumerate() {
                                settings_changed |= ui.selectable_value(&mut self.language, Some(index), &language.name).changed();
                            }
                        }).response.on_disabled_hover_text(if csv_import { "CSV column headers determine which languages are imported." } else { "The engine controls the languages used by tool.exe." });
                    });
                    ui.end_row();
                    ui.label("Destination:");
                    ui.add_enabled_ui(baboon, |ui| {
                        crate::app::ui_kit::ValueComboBox::from_id_salt("strings destination").width(field_width)
                            .selected_text(if !baboon || self.destination == StringDestination::Kit { "Data Folder" }
                                else if self.path.is_some() { "Custom Location" } else { "Select…" }).show_ui(ui, |ui| {
                                ui.add_enabled_ui(layout.is_some(), |ui| {
                                    settings_changed |= ui.selectable_value(&mut self.destination, StringDestination::Kit, "Data Folder").changed();
                                });
                                settings_changed |= ui.selectable_value(&mut self.destination, StringDestination::Custom, "Custom Location…").changed();
                            }).response.on_hover_text(if self.import { "Where to read the imported files." } else { "Where to write the extracted files." });
                    });
                    ui.end_row();
                    if settings_changed { self.path = None; }
                    if baboon && self.destination == StringDestination::Custom {
                        ui.label("Path:");
                        ui.allocate_ui_with_layout(Vec2::new(field_width, BUTTON_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            browse = icon_text_button(ui, ButtonIcon::Browse, "Browse…", true).clicked();
                            let mut path = self.path.as_ref().map_or_else(String::new, |path| path.display().to_string());
                            let response = ui.add_sized(Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                                crate::app::ui_kit::singleline_text_edit(&mut path).hint_text("Select or enter a path…"));
                            if response.changed() {
                                self.path = (!path.is_empty()).then(|| PathBuf::from(path));
                                changed = true;
                            }
                        });
                        ui.end_row();
                    }
                    if baboon && self.import {
                        ui.label("Import mode:");
                        crate::app::ui_kit::ValueComboBox::from_id_salt("strings import mode").width(field_width)
                            .selected_text(match self.mode { None => "Select…", Some(ImportMode::Amend) => "Amend", Some(ImportMode::Replace) => "Replace" }).show_ui(ui, |ui| {
                                changed |= ui.selectable_value(&mut self.mode, Some(ImportMode::Amend), "Amend").changed();
                                changed |= ui.selectable_value(&mut self.mode, Some(ImportMode::Replace), "Replace").changed();
                            });
                        ui.end_row();
                    }
                });
                changed |= settings_changed;
                if changed { self.clear_preview(); }
                let ready;
                if self.method == StringMethod::Tool {
                    ui.add_space(8.0);
                    ui.label("Uses saved files and the engine's native text format and language folders.");
                    if self.import {
                        ui.colored_label(ui.visuals().warn_fg_color, "This command imports a directory and may overwrite sibling tags in every language. It writes directly to disk; Baboon's Amend/Replace modes and undo do not apply.");
                        ui.label("Changed open string lists reload after tool finishes. Any edits made while it runs are preserved.");
                    }
                    if let Ok(command) = &tool_command { ui.monospace(command); }
                    if let Some(error) = &tool_error { ui.colored_label(ui.visuals().error_fg_color, error); }
                    if tool_dirty {
                        ui.label(if self.import { "Save all open tag changes before running a directory import." } else { "Save this tag before exporting with tool.exe." });
                        if ui.button(if self.import { "Save All Open Tags" } else { "Save Tag" }).clicked() {
                            cx.send(ExportCommand::SaveStringTransferTags { stamp: self.stamp, key: self.key.clone(), all: self.import });
                        }
                    }
                    ready = tool_error.is_none() && !tool_dirty;
                } else {
                    if self.import {
                        ui.label(match self.mode {
                            None => "Choose Amend or Replace to continue.",
                            Some(ImportMode::Amend) => "Updates nonblank values and appends new strings. Existing order and blank values are preserved.",
                            Some(ImportMode::Replace) => "Replaces all rows in file order. Omitted IDs are deleted in every language. Blank cells clear translations; absent or entirely blank language columns preserve retained IDs' translations.",
                        });
                        if list.storage == Storage::Classic {
                            ui.label("Halo CE: Amend updates existing Index values; blank or absent Index appends. Replace uses file order and ignores Index.");
                        }
                        if changed { self.clear_preview(); }
                    }
                    if self.destination == StringDestination::Custom {
                        let folder = self.format == StringFormat::Text && self.language.is_none() && list.languages.len() > 1;
                        if browse {
                            let mut dialog = rfd::FileDialog::new().set_title(title);
                            if let Some(directory) = &self.directory { dialog = dialog.set_directory(directory); }
                            let selected = if folder { dialog.pick_folder() } else {
                                let extension = if self.format == StringFormat::Csv { "csv" } else { list.storage.text_extension() };
                                dialog = dialog.add_filter("Strings", &[extension]);
                                if self.import { dialog.pick_file() } else {
                                    let name = if self.format == StringFormat::Csv { format!("{}.csv", self.stem) } else { string_file_name(&self.stem, &list, self.language.unwrap_or(0)) };
                                    dialog.set_file_name(name).save_file()
                                }
                            };
                            if let Some(path) = selected { self.path = Some(path); self.clear_preview(); }
                        }
                        if folder && !self.import { ui.label("Existing files will be replaced."); }
                    }
                    let path = if self.destination == StringDestination::Kit { layout.as_ref().map(|layout| layout.data.clone()) } else { self.path.clone() };
                    let paths = path.as_ref().map(|path| transfer_paths(&list, self.language, &self.stem, &self.relative, path, layout.as_ref(), game, self.options()));
                    if let Some(paths) = &paths {
                        match paths {
                            Ok(paths) => {
                                ui.collapsing("File locations", |ui| {
                                    crate::app::ScrollArea::vertical().id_salt("string paths").max_height(100.0).show(ui, |ui| {
                                        for (_, path) in paths { ui.label(path.display().to_string()); }
                                    });
                                });
                            }
                            Err(error) => { ui.colored_label(ui.visuals().error_fg_color, error); }
                        }
                    }
                    if self.import && self.mode.is_some() && self.preview.is_none() && let Some(Ok(paths)) = &paths {
                        self.preview = Some(transfer_import_plan(&list, paths, self.options(), &syntax));
                    }
                    if let Some(preview) = &self.preview {
                        match preview {
                            Ok(plan) => {
                                ui.label(format!("{} file(s): update {}, add {}, delete {}, clear {} translations, reorder {} rows.", plan.files, plan.updated, plan.added, plan.removed, plan.cleared, plan.reordered));
                                for note in &plan.notes { ui.label(note); }
                            }
                            Err(error) => { ui.colored_label(ui.visuals().error_fg_color, error); }
                        }
                    }
                    can_refresh = paths.is_some();
                    ready = matches!(paths, Some(Ok(_))) && (!self.import || matches!(self.preview, Some(Ok(_)))) && (!self.import || !cx.model.editing_kit_is_read_only(index));
                }
                if let Some(error) = &self.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                ui.add_space(10.0);
                ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 40.0),
                    egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let background = ui.max_rect().expand2(Vec2::new(20.0, 0.0));
                    let mut painter = ui.painter().clone();
                    painter.set_clip_rect(background.intersect(cx.egui.content_rect()));
                    let mut rounding = ui.visuals().window_corner_radius;
                    rounding.nw = 0;
                    rounding.ne = 0;
                    painter.rect_filled(background, rounding, foundation_block_bar());
                    cancel = icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked();
                    run = icon_text_button(ui, action_icon, if self.import { "Import" } else { "Extract" }, ready).clicked();
                    if self.import && self.method == StringMethod::Baboon {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if icon_text_button(ui, ButtonIcon::Refresh, "Refresh File Preview", can_refresh).clicked() {
                                self.clear_preview();
                                cx.egui.request_repaint();
                            }
                        });
                    }
                });
            });
        if changed { cx.egui.request_repaint(); }
        if run {
            if self.method == StringMethod::Tool {
                cx.send(ExportCommand::ToolStringTransfer { stamp: self.stamp, key: self.key.clone(), import: self.import });
            } else {
                cx.send(ExportCommand::StringTransfer {
                    stamp: self.stamp, key: self.key.clone(), import: self.import, language: self.language,
                    path: self.path.clone().or(self.directory.clone()).unwrap_or_default(), options: self.options(),
                });
            }
        }
        open && !cancel
    }
}
