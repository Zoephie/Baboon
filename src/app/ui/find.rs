//! Modeless Ctrl+F find-in-tag window.

use super::*;

impl Baboon {
    /// Draw the modeless Find window and dispatch query or navigation changes.
    pub(super) fn draw_find_window(&mut self, ctx: &egui::Context) {
        if !self.find.open {
            return;
        }
        let mut open = true;
        let mut step = 0;
        let mut changed = false;
        let mut filter_changed = false;
        let default_pos = ctx.screen_rect().right_top() + egui::vec2(-488.0, 72.0);
        egui::Window::new("Find")
            .id(egui::Id::new("find_in_tag"))
            .title_bar(false)
            .collapsible(false)
            .movable(true)
            .resizable(false)
            .default_width(470.0)
            .default_pos(default_pos)
            .show(ctx, |ui| {
                draw_find_window_header(ui, &mut open);
                ui.separator();
                egui::Grid::new("find_options")
                    .num_columns(3)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("find:").strong().color(text_dark()));
                        let response = ui.add_sized(
                            [290.0, 25.0],
                            egui::TextEdit::singleline(&mut self.find.query)
                                .id(egui::Id::new("find_query"))
                                .vertical_align(egui::Align::Center),
                        );
                        if self.find.focus_query {
                            response.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ctx, response.id) {
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(self.find.query.chars().count()),
                                    )));
                                state.store(ctx, response.id);
                            }
                            self.find.focus_query = false;
                        }
                        changed |= response.changed();
                        ui.end_row();

                        ui.label(RichText::new("within:").strong().color(text_dark()));
                        egui::ComboBox::from_id_salt("find_within")
                            .selected_text(self.find.within.label())
                            .width(190.0)
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(
                                        &mut self.find.within,
                                        FindWithin::CurrentTag,
                                        FindWithin::CurrentTag.label(),
                                    )
                                    .changed();
                                changed |= ui
                                    .selectable_value(
                                        &mut self.find.within,
                                        FindWithin::OpenTags,
                                        FindWithin::OpenTags.label(),
                                    )
                                    .changed();
                                changed |= ui
                                    .selectable_value(
                                        &mut self.find.within,
                                        FindWithin::AllTags,
                                        FindWithin::AllTags.label(),
                                    )
                                    .changed();
                            });
                        changed |= ui
                            .checkbox(&mut self.find.match_case, "match case")
                            .changed();
                        ui.end_row();

                        ui.label(RichText::new("look in:").strong().color(text_dark()));
                        egui::ComboBox::from_id_salt("find_look_in")
                            .selected_text(self.find.look_in.label())
                            .width(190.0)
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .checkbox(&mut self.find.look_in.field_names, "Field names")
                                    .changed();
                                changed |= ui
                                    .checkbox(&mut self.find.look_in.field_values, "Field values")
                                    .changed();
                                changed |= ui
                                    .checkbox(&mut self.find.look_in.blocks, "Blocks")
                                    .changed();
                            });
                        changed |= ui
                            .checkbox(&mut self.find.whole_word, "match whole word")
                            .changed();
                        ui.end_row();
                    });
                ui.add_space(8.0);
                if self.find.searching {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let text = self
                            .find
                            .progress
                            .map(|(done, total)| format!("searching… {done}/{total}"))
                            .unwrap_or_else(|| "preparing all-tag search…".to_owned());
                        ui.label(RichText::new(text).small().color(subtle_dark()));
                    });
                } else if self.find.unreadable > 0 && self.find.within == FindWithin::AllTags {
                    ui.label(
                        RichText::new(format!("{} tag(s) could not be read", self.find.unreadable))
                            .small()
                            .color(subtle_dark()),
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if selectable_icon_text_button(
                        ui,
                        ButtonIcon::Filter,
                        "Filter Results",
                        self.find.filter_results,
                    )
                    .on_hover_text("Show only matching fields and blocks in the selected scope")
                    .clicked()
                    {
                        self.find.filter_results = !self.find.filter_results;
                        filter_changed = true;
                    }
                    let can_navigate = !self.find.occurrences.is_empty() && !self.find.searching;
                    let counter = self
                        .find
                        .active
                        .map(|index| format!("{}/{}", index + 1, self.find.occurrences.len()))
                        .unwrap_or_else(|| format!("0/{}", self.find.occurrences.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(
                            ui,
                            ButtonIcon::Right,
                            "Next match (Enter)",
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
                            "Previous match (Shift+Enter)",
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
                let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                if enter && !self.find.searching {
                    step = if ui.input(|input| input.modifiers.shift) {
                        -1
                    } else {
                        1
                    };
                }
            });
        if changed {
            self.find.active = None;
            self.refresh_find(ctx);
            if let Some(hit) = self.find.active_occurrence().cloned() {
                self.activate_find_occurrence(ctx, hit);
            }
        }
        if filter_changed {
            ctx.request_repaint();
        }
        if step != 0 {
            self.step_find(ctx, step);
        }
        if !open || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.find.close();
            ctx.data_mut(|data| data.remove::<FindRenderSnapshot>(find_render_snapshot_id()));
        }
    }
}

fn draw_find_window_header(ui: &mut Ui, open: &mut bool) {
    const HEADER_HEIGHT: f32 = 28.0;
    const TITLE_ICON_SIZE: f32 = 18.0;
    const TITLE_GAP: f32 = 7.0;

    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), HEADER_HEIGHT),
        Sense::hover(),
    );
    let font = TextStyle::Heading.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap("Find".to_owned(), font, text_dark());
    let group_width = TITLE_ICON_SIZE + TITLE_GAP + galley.size().x;
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.center().x - group_width * 0.5,
            rect.center().y - TITLE_ICON_SIZE * 0.5,
        ),
        Vec2::splat(TITLE_ICON_SIZE),
    );
    paint_button_icon_at(ui, ButtonIcon::Find, icon_rect, text_dark());
    ui.painter().galley(
        egui::pos2(
            icon_rect.right() + TITLE_GAP,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        text_dark(),
    );

    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 10.0, rect.center().y),
        Vec2::splat(20.0),
    );
    let close = ui
        .interact(close_rect, ui.id().with("find_close"), Sense::click())
        .on_hover_text("Close Find");
    let color = ui.style().interact(&close).fg_stroke.color;
    let cross = close_rect.shrink(5.0);
    ui.painter().line_segment(
        [cross.left_top(), cross.right_bottom()],
        Stroke::new(1.5, color),
    );
    ui.painter().line_segment(
        [cross.right_top(), cross.left_bottom()],
        Stroke::new(1.5, color),
    );
    if close.clicked() {
        *open = false;
    }
}
