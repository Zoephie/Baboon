//! Modeless Ctrl+F find-in-tag window.

use super::*;

const FIND_LABEL_WIDTH: f32 = 100.0;
const FIND_ROW_GAP: f32 = 8.0;

fn find_row_label(ui: &mut Ui, label: &str) {
    ui.allocate_ui_with_layout(Vec2::new(FIND_LABEL_WIDTH, 25.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(Vec2::new(FIND_LABEL_WIDTH, 25.0));
        ui.label(RichText::new(label).strong().color(text_dark()));
    });
}

/// The modeless Find window. What it searches for and what it found are the
/// Find search's, in [`SearchFeature`]; the window shows them as they stand
/// each frame and sends any change to the query, a step to the next or
/// previous match, or its closing as a command.
pub(in crate::app) struct FindWindow {
    /// Select the query on the next draw, as opening Find again does.
    pub(in crate::app) focus_query: bool,
}

impl Dialog for FindWindow {
    fn show(&mut self, cx: &Ctx, app: &AppReads) -> bool {
        let find = &app.search.find;
        // Closed by the search itself, as when its workspace goes.
        if !find.open {
            return false;
        }
        let mut query = find.query();
        let mut replacement = find.replacement.clone();
        let mut replace_mode = find.replace_mode;
        let mut replacement_changed = false;
        let mut replace_action = None;
        let mut filter_results = find.filter_results;
        let ctx = cx.egui;
        let mut open = true;
        let mut step = 0;
        let mut run_selection = false;
        let mut changed = false;
        let mut filter_changed = false;
        // Enter and Escape belong to the query box. They used to be read from
        // the global input, so committing a field edit with Enter anywhere in
        // the app stepped to the next match, and Escape out of any menu or edit
        // closed Find.
        let mut query_escape = false;
        let default_pos = ctx.content_rect().right_top() + egui::vec2(-618.0, 72.0);
        egui::Window::new("Find")
            .id(egui::Id::unique("find_in_tag"))
            .title_bar(false)
            .collapsible(false)
            .movable(true)
            .resizable(false)
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin {
                left: 12, right: 12, top: 0, bottom: 0,
            }))
            .default_width(window_width(ctx, 600.0))
            .default_pos(default_pos)
            .show(ctx, |ui| {
                ui.spacing_mut().window_margin = egui::Margin { left: 12, right: 12, top: 0, bottom: 0 };
                let mut selected = usize::from(replace_mode);
                if draw_tabbed_window_header(ui, &[("Find", ButtonIcon::Find), ("Replace", ButtonIcon::Replace)],
                    &mut selected, &mut open, Some(("Open Find and Replace documentation", egui::Id::unique("find_replace_help")))) {
                    cx.send(crate::app::help::HelpCommand::ShowDocSection(crate::app::help::FIND_REPLACE_HELP_SECTION));
                }
                replace_mode = selected == 1;
                if replace_mode != find.replace_mode { self.focus_query = true; }
                ui.add_space(8.0);
                ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = FIND_ROW_GAP;
                ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        find_row_label(ui, "Find:");
                        let response = ui.add_sized(
                            [ui.available_width(), 25.0],
                            crate::app::ui_kit::singleline_text_edit(&mut query.text)
                                .id(egui::Id::unique("find_query"))
                                .vertical_align(egui::Align::Center),
                        );
                        if self.focus_query {
                            response.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ctx, response.id) {
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(query.text.chars().count()),
                                    )));
                                state.store(ctx, response.id);
                            }
                            self.focus_query = false;
                        }
                        changed |= response.changed();
                        // A single-line box gives up focus on Enter and Escape,
                        // so the key arrives on the frame it loses focus.
                        let in_query = response.has_focus() || lost_focus_once(&response);
                        let (enter, escape, shift) = ui.input(|input| {
                            (
                                input.key_pressed(egui::Key::Enter),
                                input.key_pressed(egui::Key::Escape),
                                input.modifiers.shift,
                            )
                        });
                        if in_query && enter && !find.searching {
                            if replace_mode || query.within == FindWithin::AllTags || query.strings.as_ref().is_some_and(|options| options.selected_rows) {
                                run_selection = !query.text.is_empty() && (query.strings.as_ref().is_none_or(|options| !options.selected_rows)
                                    || find.string_context.as_ref().is_some_and(|context| !context.selected.is_empty()));
                            } else {
                                step = if shift { -1 } else { 1 };
                            }
                            // Keep the box focused so Enter keeps stepping.
                            response.request_focus();
                        }
                        query_escape = in_query && escape;
                });
                if replace_mode {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        find_row_label(ui, "Replace With:");
                        let response = ui.add_sized([ui.available_width(), 25.0],
                            crate::app::ui_kit::singleline_text_edit(&mut replacement)
                                .id(egui::Id::unique("find_replacement")).vertical_align(egui::Align::Center));
                        replacement_changed = response.changed();
                        let focused = response.has_focus() || lost_focus_once(&response);
                        query_escape |= focused && ui.input(|input| input.key_pressed(egui::Key::Escape));
                    });
                }
                let selection_width = 170.0;
                let options_width = ui.available_width() - selection_width - 16.0;
                let option_width = (options_width - FIND_LABEL_WIDTH - 12.0).max(130.0);
                ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.allocate_ui_with_layout(Vec2::new(options_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::Grid::new("find_options")
                    .num_columns(2)
                    .spacing([12.0, FIND_ROW_GAP])
                    .show(ui, |ui| {
                        find_row_label(ui, "Within:");
                        if let Some(options) = &mut query.strings {
                            crate::app::ui_kit::ValueComboBox::from_id_salt("find_string_within")
                                .selected_text(if options.selected_rows { "Selected Rows" } else { "Current Tag" })
                                .width(option_width).truncate().show_ui(ui, |ui| {
                                    changed |= ui.selectable_value(&mut options.selected_rows, false, "Current Tag").changed();
                                    changed |= ui.selectable_value(&mut options.selected_rows, true, "Selected Rows").changed();
                                });
                        } else {
                        ui.scope(|ui| {
                            crate::app::ui_kit::ValueComboBox::from_id_salt("find_within")
                                .selected_text(query.within.label())
                                .width(option_width)
                                .show_ui(ui, |ui| {
                                    changed |= ui
                                        .selectable_value(
                                            &mut query.within,
                                            FindWithin::CurrentTag,
                                            FindWithin::CurrentTag.label(),
                                        )
                                        .changed();
                                    changed |= ui
                                        .selectable_value(
                                            &mut query.within,
                                            FindWithin::OpenTags,
                                            FindWithin::OpenTags.label(),
                                        )
                                        .changed();
                                    changed |= ui.add_enabled_ui(!replace_mode, |ui| ui
                                        .selectable_value(
                                            &mut query.within,
                                            FindWithin::AllTags,
                                            FindWithin::AllTags.label(),
                                        )
                                        .on_disabled_hover_text("Replace is available in Current Tag and Open Tags")
                                        .changed()).inner;
                                });
                        });
                        }
                        ui.end_row();

                        find_row_label(ui, "Look In:");
                        if let Some(options) = &mut query.strings {
                            let has_ids = find
                                    .string_context
                                    .as_ref()
                                    .is_some_and(|context| context.has_ids);
                            let hud = find.string_context.as_ref().is_some_and(|context| context.tag_key.ends_with(".hud_message_text"));
                            let label = match (has_ids && options.ids, options.text) {
                                (true, true) => if hud { "Names + Text" } else { "String IDs + String Text" },
                                (true, false) => if hud { "Names" } else { "String IDs" },
                                (false, true) => if hud { "Text" } else { "String Text" },
                                (false, false) => "Nothing",
                            };
                            crate::app::ui_kit::ValueComboBox::from_id_salt("find_string_look_in")
                                .selected_text(label)
                                .width(option_width).truncate()
                                .show_ui(ui, |ui| {
                                changed |= ui
                                    .checkbox_enabled(
                                        has_ids,
                                        &mut options.ids, if hud { "Names" } else { "String IDs" },
                                    )
                                    .on_disabled_hover_text(
                                        "Halo CE string lists have no string IDs",
                                    )
                                    .changed();
                                changed |= ui.baboon_checkbox(&mut options.text, if hud { "Text" } else { "String Text" }).changed();
                            });
                        } else {
                            if replace_mode {
                                let (rect, response) = ui.allocate_exact_size(Vec2::new(option_width, 25.0), Sense::hover());
                                ui.painter().rect_stroke(rect, ui.visuals().widgets.inactive.corner_radius,
                                    Stroke::new(1.0, foundation_input_edge()), egui::StrokeKind::Inside);
                                let content = rect.shrink2(ui.spacing().button_padding);
                                ui.scope_builder(egui::UiBuilder::new().max_rect(content)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                                    ui.add(egui::Label::new(RichText::new("Field Values")
                                        .text_style(TextStyle::Button).color(text_dark())).truncate());
                                });
                                response.on_hover_text("Field Values is the only option in Replace mode because only field values can be edited.");
                            } else {
                            crate::app::ui_kit::ValueComboBox::from_id_salt("find_look_in")
                                .selected_text(query.look_in.label())
                                .width(option_width)
                                .show_ui(ui, |ui| {
                                    changed |= ui
                                        .baboon_checkbox(&mut query.look_in.field_names, "Field names")
                                        .changed();
                                    changed |= ui
                                        .baboon_checkbox(&mut query.look_in.field_values, "Field values")
                                        .changed();
                                    changed |=
                                        ui.baboon_checkbox(&mut query.look_in.blocks, "Blocks").changed();
                                });
                            }
                        }

                        ui.end_row();
                        if let (Some(options), Some(context)) =
                            (&mut query.strings, &find.string_context)
                        {
                            find_row_label(ui, "Language:");
                            ui.add_enabled_ui(options.text, |ui| {
                                let label = match options.language {
                                    StringFindLanguage::Current => format!(
                                        "Current Language ({})",
                                        context
                                            .languages
                                            .get(context.language)
                                            .map(String::as_str)
                                            .unwrap_or("English")
                                    ),
                                    StringFindLanguage::All => "All Languages".to_owned(),
                                    StringFindLanguage::Language(index) => context
                                        .languages
                                        .get(index)
                                        .cloned()
                                        .unwrap_or_else(|| "Current Language".to_owned()),
                                };
                                crate::app::ui_kit::ValueComboBox::from_id_salt("find_string_language")
                                    .selected_text(label)
                                    .width(option_width).truncate()
                                    .show_ui(ui, |ui| {
                                        changed |= ui
                                            .selectable_value(
                                                &mut options.language,
                                                StringFindLanguage::Current,
                                                "Current Language",
                                            )
                                            .changed();
                                        changed |= ui
                                            .selectable_value(
                                                &mut options.language,
                                                StringFindLanguage::All,
                                                "All Languages",
                                            )
                                            .changed();
                                        ui.separator();
                                        for (index, language) in
                                            context.languages.iter().enumerate()
                                        {
                                            changed |= ui
                                                .selectable_value(
                                                    &mut options.language,
                                                    StringFindLanguage::Language(index),
                                                    language,
                                                )
                                                .changed();
                                        }
                                    });
                            });
                            ui.end_row();
                        }
                    });
                });
                egui::Frame::new()
                    .stroke(Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color))
                    .corner_radius(4)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width(selection_width - 18.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = FIND_ROW_GAP;
                        // Native checkboxes otherwise reserve a button-height
                        // row around their smaller square, inflating visible gaps.
                        ui.spacing_mut().interact_size.y = ui.spacing().icon_width;
                        ui.label(RichText::new("Selection Options").strong().color(subtle_dark()));
                        changed |= ui.baboon_checkbox(&mut query.match_case, "Match Case").changed();
                        changed |= ui.baboon_checkbox(&mut query.whole_word, "Match Whole Word").changed();
                        });
                    });
                });
                });
                ui.add_space(8.0);
                ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 40.0),
                    egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let background = ui.max_rect().expand2(Vec2::new(12.0, 0.0));
                    let fill = ui.painter().add(egui::Shape::Noop);
                    paint_frame_band(ui, fill, background, ui.visuals().window_corner_radius,
                        ui.visuals().window_stroke, foundation_block_bar(), false);
                    let manual = replace_mode || query.within == FindWithin::AllTags
                        || query.strings.as_ref().is_some_and(|options| options.selected_rows);
                    if manual {
                        let no_selection = query.strings.as_ref().is_some_and(|options| options.selected_rows)
                            && find.string_context.as_ref().is_none_or(|context| context.selected.is_empty());
                        let enabled = !find.searching && !query.text.is_empty() && !no_selection;
                        let hint = if find.searching { "A search is in progress; see the bottom status bar" }
                            else if no_selection { "Select rows in the Strings Editor to search" }
                            else if query.text.is_empty() { "Enter text to find" }
                            else if query.within == FindWithin::AllTags { "Search all tags using the current settings" }
                            else if find.selection_run.as_ref().is_some_and(|run| run.changed) { "Strings changed; click Find to refresh the results" }
                            else { "Search or refresh results using the current scope and settings" };
                        run_selection |= icon_text_button(ui, ButtonIcon::Find, "Find", enabled)
                            .on_hover_text(hint).on_disabled_hover_text(hint).clicked();
                    }
                    if ui.add_enabled_ui(!manual || find.filter_run.as_ref().is_some_and(|run| run.kit == cx.model.active_kit_id()), |ui| selectable_icon_text_button(
                        ui,
                        ButtonIcon::Filter,
                        "Filter Results",
                        filter_results,
                    )
                    .on_hover_text(if query.strings.is_some() {
                        "Show only matching string rows"
                    } else {
                        "Show only matching fields and blocks in the selected scope"
                    })
                    .on_disabled_hover_text("Run Find before filtering its results")
                    .clicked()).inner
                    {
                        filter_results = !filter_results;
                        filter_changed = true;
                    }
                    if replace_mode {
                        let ready = !changed && !find.searching && !query.text.is_empty()
                            && replace_mode == find.replace_mode && query.within != FindWithin::AllTags
                            && !cx.model.editing_kit_is_read_only(cx.model.active)
                            && !find.replace_needs_refresh();
                        if icon_text_button(ui, ButtonIcon::Replace, "Replace", ready && find.active_occurrence().is_some()).clicked() { replace_action = Some(false); }
                        if icon_text_button(ui, ButtonIcon::Replace, "Replace All", ready && !find.occurrences.is_empty()).clicked() { replace_action = Some(true); }
                    }
                    let can_navigate = !changed && !find.occurrences.is_empty() && !find.searching;
                    let counter = find
                        .active
                        .map(|index| format!("{}/{}", index + 1, find.occurrences.len()))
                        .unwrap_or_else(|| format!("0/{}", find.occurrences.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(
                            ui,
                            ButtonIcon::Right,
                            if replace_mode || query.strings.as_ref().is_some_and(|options| options.selected_rows) { "Next match" } else { "Next match (Enter)" },
                            can_navigate,
                            text_dark(),
                        )
                        .clicked()
                        {
                            step = 1;
                        }
                        if icon_button(
                            ui,
                            ButtonIcon::Left,
                            if replace_mode || query.strings.as_ref().is_some_and(|options| options.selected_rows) { "Previous match" } else { "Previous match (Shift+Enter)" },
                            can_navigate,
                            text_dark(),
                        )
                        .clicked()
                        {
                            step = -1;
                        }
                        ui.label(RichText::new(counter).strong().color(subtle_dark()));
                    });
                });
            });
        if changed {
            cx.send(SearchCommand::FindChanged(query));
        }
        if replace_mode != find.replace_mode { cx.send(SearchCommand::FindMode(replace_mode)); }
        if replacement_changed { cx.send(SearchCommand::ReplacementChanged(replacement)); }
        if let Some(all) = replace_action { cx.send(SearchCommand::Replace(all)); }
        if run_selection {
            cx.send(SearchCommand::FindRunSelection);
        }
        if filter_changed {
            cx.send(SearchCommand::FindFilter(filter_results));
            ctx.request_repaint();
        }
        if step != 0 {
            cx.send(SearchCommand::FindStep(step));
        }
        if !open || query_escape {
            cx.send(SearchCommand::FindClose);
            ctx.data_mut(|data| {
                data.remove::<std::sync::Arc<FindRenderSnapshot>>(find_render_snapshot_id())
            });
            return false;
        }
        true
    }
}

/// Compact icon tabs for a floating window. Shares the standard header's
/// height, background, corners and close control; unused space remains draggable.
pub(in crate::app) fn draw_tabbed_window_header(
    ui: &mut Ui, tabs: &[(&str, ButtonIcon)], selected: &mut usize, open: &mut bool,
    help: Option<(&str, egui::Id)>,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
    let margin = ui.spacing().window_margin;
    let background = egui::Rect::from_min_max(
        rect.min - egui::vec2(f32::from(margin.left), f32::from(margin.top)),
        rect.max + egui::vec2(f32::from(margin.right), ui.spacing().item_spacing.y * 0.5));
    let fill = ui.painter().add(egui::Shape::Noop);
    paint_frame_band(ui, fill, background, ui.visuals().window_corner_radius,
        ui.visuals().window_stroke, foundation_block_bar(), true);
    let painter = ui.painter().with_clip_rect(background.intersect(ui.ctx().content_rect()));
    let mut left = background.left();
    for (index, (label, icon)) in tabs.iter().enumerate() {
        const PADDING: f32 = 12.0;
        const ICON_SIZE: f32 = 18.0;
        const GAP: f32 = 6.0;
        let color = text_dark();
        let galley = painter.layout_no_wrap((*label).to_owned(), TextStyle::Button.resolve(ui.style()), color);
        let tab = egui::Rect::from_min_size(egui::pos2(left, rect.top()), egui::vec2(PADDING * 2.0 + ICON_SIZE + GAP + galley.size().x, rect.height()));
        let response = ui.interact(tab, ui.scope_id().with(("window_header_tab", index)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, *selected == index, *label));
        let mut rounding = egui::CornerRadius::same(4);
        if index == 0 { rounding.nw = ui.visuals().window_corner_radius.nw.saturating_sub(ui.visuals().window_stroke.width.ceil() as u8); }
        if *selected == index {
            rounding.sw = 0; rounding.se = 0;
            let active = egui::Rect::from_min_max(tab.min, egui::pos2(tab.right(), background.bottom()));
            painter.rect_filled(active, rounding, ui.visuals().window_fill);
        } else if response.hovered() { painter.rect_filled(tab, rounding, ui.visuals().widgets.hovered.weak_bg_fill); }
        paint_button_icon_at(ui, *icon, egui::Rect::from_center_size(egui::pos2(tab.left() + PADDING + ICON_SIZE * 0.5, background.center().y), Vec2::splat(ICON_SIZE)), color);
        painter.galley(egui::pos2(tab.left() + PADDING + ICON_SIZE + GAP, background.center().y - galley.size().y * 0.5), galley, color);
        if response.clicked() { *selected = index; }
        left = tab.right();
    }
    let close_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 10.0, background.center().y), Vec2::splat(20.0));
    let mut help_clicked = false;
    if let Some((tooltip, id)) = help {
        let help_rect = egui::Rect::from_center_size(egui::pos2(close_rect.left() - 14.0, background.center().y), Vec2::splat(20.0));
        let response = ui.interact(help_rect, id, Sense::click()).on_hover_text(tooltip);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tooltip));
        paint_button_icon_at(ui, ButtonIcon::About, help_rect.shrink(2.0),
            if response.hovered() { text_dark() } else { subtle_dark() });
        help_clicked = response.clicked();
    }
    let close = ui.interact(close_rect, ui.scope_id().with("window_header_close"), Sense::click()).on_hover_text("Close window");
    let cross = close_rect.shrink(5.0);
    let stroke = Stroke::new(1.5_f32, ui.style().interact(&close).fg_stroke.color);
    painter.line_segment([cross.left_top(), cross.right_bottom()], stroke);
    painter.line_segment([cross.right_top(), cross.left_bottom()], stroke);
    if close.clicked() { *open = false; }
    help_clicked
}

pub(in crate::app) fn draw_icon_window_header(
    ui: &mut Ui,
    title: &str,
    icon: ButtonIcon,
    open: &mut bool,
) {
    draw_icon_window_header_impl(ui, title, icon, Some(open));
}

pub(in crate::app) fn draw_icon_window_header_without_close(
    ui: &mut Ui,
    title: &str,
    icon: ButtonIcon,
) {
    draw_icon_window_header_impl(ui, title, icon, None);
}

fn draw_icon_window_header_impl(
    ui: &mut Ui,
    title: &str,
    icon: ButtonIcon,
    open: Option<&mut bool>,
) {
    const HEADER_HEIGHT: f32 = 28.0;
    const TITLE_ICON_SIZE: f32 = 18.0;
    const TITLE_GAP: f32 = 7.0;

    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), HEADER_HEIGHT),
        Sense::hover(),
    );
    let margin = ui.spacing().window_margin;
    let mut background = rect;
    background.min.x -= f32::from(margin.left);
    background.max.x += f32::from(margin.right);
    background.min.y -= f32::from(margin.top);
    background.max.y += ui.spacing().item_spacing.y * 0.5;
    let fill = ui.painter().add(egui::Shape::Noop);
    paint_frame_band(ui, fill, background, ui.visuals().window_corner_radius,
        ui.visuals().window_stroke, foundation_block_bar(), true);
    let font = TextStyle::Heading.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), font, text_dark());
    let group_width = TITLE_ICON_SIZE + TITLE_GAP + galley.size().x;
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.center().x - group_width * 0.5,
            background.center().y - TITLE_ICON_SIZE * 0.5,
        ),
        Vec2::splat(TITLE_ICON_SIZE),
    );
    paint_button_icon_at(ui, icon, icon_rect, text_dark());
    ui.painter().galley(
        egui::pos2(
            icon_rect.right() + TITLE_GAP,
            background.center().y - galley.size().y * 0.5,
        ),
        galley,
        text_dark(),
    );

    if let Some(open) = open {
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 10.0, background.center().y),
            Vec2::splat(20.0),
        );
        let close = ui
            .interact(
                close_rect,
                ui.scope_id().with("window_header_close"),
                Sense::click(),
            )
            .on_hover_text(format!("Close {title}"));
        let cross = close_rect.shrink(5.0);
        let stroke = Stroke::new(1.5_f32, ui.style().interact(&close).fg_stroke.color);
        ui.painter().line_segment([cross.left_top(), cross.right_bottom()], stroke);
        ui.painter().line_segment([cross.right_top(), cross.left_bottom()], stroke);
        if close.clicked() {
            *open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_opens_find_replace_docs_without_closing_or_switching_modes() {
        use crate::app::loose_fixture::PromptDriver;
        for strings in [false, true] {
            for replace in [false, true] {
                let mut driver = PromptDriver::new();
                driver.ctx.set_fonts(foundation_fonts());
                let mut app = Baboon::for_test();
                app.open_find();
                app.set_find_mode(replace, &driver.ctx);
                if strings { app.search.find.strings = Some(StringFindOptions::default()); }
                driver.frame(&mut app, Vec::new());
                driver.frame(&mut app, Vec::new());
                let pos = driver.ctx.read_response(egui::Id::unique("find_replace_help")).unwrap().rect.center();
                driver.frame(&mut app, vec![egui::Event::PointerMoved(pos)]);
                for pressed in [true, false] {
                    driver.frame(&mut app, vec![egui::Event::PointerButton {
                        pos, button: egui::PointerButton::Primary, pressed,
                        modifiers: egui::Modifiers::NONE,
                    }]);
                }
                let help = app.dialogs.get::<crate::app::help::HelpWindow>().expect("About opens Help");
                assert!(help.tab == crate::app::prefs::HelpPanelTab::Doc);
                assert_eq!(help.doc_section.as_deref(), Some(crate::app::help::FIND_REPLACE_HELP_SECTION));
                assert!(app.search.find.open);
                assert_eq!(app.search.find.replace_mode, replace);
            }
        }
    }

    #[test]
    fn selection_options_have_eight_point_visible_checkbox_gaps() {
        for replace in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_global_style(foundation_style());
            ctx.set_fonts(foundation_fonts());
            egui_extras::install_image_loaders(&ctx);
            let mut app = Baboon::for_test();
            app.open_find();
            app.set_find_mode(replace, &ctx);
            let mut boxes = Vec::new();
            for _ in 0..3 {
                let output = crate::app::run_ui_test(&ctx, egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                    ..Default::default()
                }, |_ui| { app.dialogs.draw(&cx!(app, &ctx), &app_reads!(app)); });
                let labels: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if matches!(text.galley.job.text.as_str(), "Match Case" | "Match Whole Word") =>
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size())),
                    _ => None,
                }).collect();
                boxes = output.shapes.iter().filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.brush.is_some() && rect.rect.size() == Vec2::splat(16.0) => Some(rect.rect),
                    egui::Shape::Mesh(mesh) if mesh.calc_bounds().size() == Vec2::splat(16.0) => Some(mesh.calc_bounds()),
                    _ => None,
                }).filter(|rect| labels.iter().any(|label|
                    (rect.center().y - label.center().y).abs() < 1.0
                    && rect.right() <= label.left() && label.left() - rect.right() < 20.0
                )).collect();
            }
            boxes.sort_by(|a, b| a.top().total_cmp(&b.top()));
            assert_eq!(boxes.len(), 2);
            assert_eq!(boxes[1].top() - boxes[0].bottom(), 8.0,
                "measure the painted checkbox squares, not their padded interaction rows");
        }
    }

    #[test]
    fn header_tabs_switch_modes_and_show_the_replacement_field() {
        use crate::app::loose_fixture::PromptDriver;
        let mut app = Baboon::for_test();
        app.open_find();
        let mut driver = PromptDriver::new();
        driver.click(&mut app, "Replace");
        assert!(app.search.find.replace_mode);
        let labels = driver.frame(&mut app, Vec::new());
        assert!(labels.iter().any(|(label, _)| label == "Replace With:"));
        assert!(labels.iter().any(|(label, _)| label == "Replace All"));
        driver.click(&mut app, "Find");
        assert!(!app.search.find.replace_mode);
        assert!(!driver.frame(&mut app, Vec::new()).iter().any(|(label, _)| label == "Replace With:"));
    }

    fn occurrence(field_path: &str) -> FindOccurrence {
        FindOccurrence {
            tag_key: "file:missing".to_owned(),
            field_path: field_path.to_owned(),
            kind: FindTargetKind::Value,
            text: "match".to_owned(),
            range: 0..5,
        }
    }

    /// One frame: a text box standing in for a field editor, and Find.
    fn frame(
        app: &mut Baboon,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        focus: Option<egui::Id>,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(1200.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let _ = crate::app::run_ui_test(&ctx, input, |ui| {
            if let Some(id) = focus {
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
            egui::CentralPanel::default().show(ui, |ui| {
                let mut text = String::from("12");
                ui.add(crate::app::ui_kit::singleline_text_edit(&mut text).id(egui::Id::unique("a_field")));
            });
            app.dialogs.draw(&cx!(app, ctx), &app_reads!(app));
        });
        // What the window asked for runs once drawing is over, as in a frame.
        app.apply_commands(ctx);
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    /// Enter and Escape are Find's only while its query box has focus.
    /// Committing a field with Enter used to step to the next match, and
    /// Escape out of anything closed Find.
    #[test]
    fn find_answers_enter_and_escape_only_in_its_query_box() {
        let ctx = egui::Context::default();
        let mut app = Baboon::for_test();
        app.search.find.open = true;
        app.dialogs.open(FindWindow { focus_query: false });
        app.search.find.occurrences = vec![occurrence("a"), occurrence("b")];
        app.search.find.active = Some(0);
        let field = Some(egui::Id::unique("a_field"));
        let query = Some(egui::Id::unique("find_query"));

        // Lay both out once, then put focus in the field.
        frame(&mut app, &ctx, Vec::new(), None);
        frame(&mut app, &ctx, Vec::new(), field);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)], None);
        assert_eq!(
            app.search.find.active,
            Some(0),
            "Enter in a field must not step Find"
        );
        frame(&mut app, &ctx, Vec::new(), field);
        frame(&mut app, &ctx, vec![key(egui::Key::Escape)], None);
        assert!(
            app.search.find.open,
            "Escape in a field must not close Find"
        );

        // The same keys in the query box.
        frame(&mut app, &ctx, Vec::new(), query);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)], None);
        assert_eq!(
            app.search.find.active,
            Some(1),
            "Enter in the query box steps"
        );
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)], None);
        assert_eq!(app.search.find.active, Some(0), "and keeps stepping");
        frame(&mut app, &ctx, vec![key(egui::Key::Escape)], None);
        assert!(!app.search.find.open, "Escape in the query box closes Find");
    }

    /// What is typed into the query box becomes the search's query, a frame
    /// at a time: the window edits the query as it stands and sends the change.
    #[test]
    fn typing_in_the_query_box_sets_the_search_query() {
        let ctx = egui::Context::default();
        let mut app = Baboon::for_test();
        app.open_find();
        let query = Some(egui::Id::unique("find_query"));
        frame(&mut app, &ctx, Vec::new(), None);
        frame(&mut app, &ctx, Vec::new(), query);
        for letter in ["s", "k", "y"] {
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::Text(letter.to_owned())],
                query,
            );
        }
        assert_eq!(app.search.find.query, "sky");
    }

    #[test]
    fn enter_in_replace_runs_an_explicit_search_in_current_tag() {
        let ctx = egui::Context::default();
        let mut app = Baboon::for_test();
        app.open_find();
        app.set_find_mode(true, &ctx);
        app.search.find.query = "needle".into();
        frame(&mut app, &ctx, Vec::new(), Some(egui::Id::unique("find_query")));
        assert!(app.search.find.replace_run.is_none());
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)], None);
        assert!(app.search.find.replace_run.is_some(), "Enter executes Find rather than advancing stale results");
    }
}
