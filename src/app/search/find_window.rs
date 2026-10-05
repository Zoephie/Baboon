//! Modeless Ctrl+F find-in-tag window.

use super::*;

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
        let mut filter_results = find.filter_results;
        let ctx = cx.egui;
        let mut open = true;
        let mut step = 0;
        let mut changed = false;
        let mut filter_changed = false;
        // Enter and Escape belong to the query box. They used to be read from
        // the global input, so committing a field edit with Enter anywhere in
        // the app stepped to the next match, and Escape out of any menu or edit
        // closed Find.
        let mut query_escape = false;
        let default_pos = ctx.content_rect().right_top() + egui::vec2(-488.0, 72.0);
        egui::Window::new("Find")
            .id(egui::Id::new("find_in_tag"))
            .title_bar(false)
            .collapsible(false)
            .movable(true)
            .resizable(false)
            .default_width(window_width(ctx, 470.0))
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
                            egui::TextEdit::singleline(&mut query.text)
                                .id(egui::Id::new("find_query"))
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
                            step = if shift { -1 } else { 1 };
                            // Keep the box focused so Enter keeps stepping.
                            response.request_focus();
                        }
                        query_escape = in_query && escape;
                        ui.end_row();

                        ui.label(RichText::new("within:").strong().color(text_dark()));
                        egui::ComboBox::from_id_salt("find_within")
                            .selected_text(query.within.label())
                            .width(190.0)
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
                                changed |= ui
                                    .selectable_value(
                                        &mut query.within,
                                        FindWithin::AllTags,
                                        FindWithin::AllTags.label(),
                                    )
                                    .changed();
                            });
                        changed |= ui.checkbox(&mut query.match_case, "match case").changed();
                        ui.end_row();

                        ui.label(RichText::new("look in:").strong().color(text_dark()));
                        egui::ComboBox::from_id_salt("find_look_in")
                            .selected_text(query.look_in.label())
                            .width(190.0)
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .checkbox(&mut query.look_in.field_names, "Field names")
                                    .changed();
                                changed |= ui
                                    .checkbox(&mut query.look_in.field_values, "Field values")
                                    .changed();
                                changed |=
                                    ui.checkbox(&mut query.look_in.blocks, "Blocks").changed();
                            });
                        changed |= ui
                            .checkbox(&mut query.whole_word, "match whole word")
                            .changed();
                        ui.end_row();
                    });
                ui.add_space(8.0);
                if find.searching {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let text = find
                            .progress
                            .map(|(done, total)| format!("searching… {done}/{total}"))
                            .unwrap_or_else(|| "preparing all-tag search…".to_owned());
                        ui.label(RichText::new(text).small().color(subtle_dark()));
                    });
                } else if find.unreadable > 0 && find.within == FindWithin::AllTags {
                    ui.label(
                        RichText::new(format!("{} tag(s) could not be read", find.unreadable))
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
                        filter_results,
                    )
                    .on_hover_text("Show only matching fields and blocks in the selected scope")
                    .clicked()
                    {
                        filter_results = !filter_results;
                        filter_changed = true;
                    }
                    let can_navigate = !find.occurrences.is_empty() && !find.searching;
                    let counter = find
                        .active
                        .map(|index| format!("{}/{}", index + 1, find.occurrences.len()))
                        .unwrap_or_else(|| format!("0/{}", find.occurrences.len()));
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
            });
        if changed {
            cx.send(SearchCommand::FindChanged(query));
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

fn draw_find_window_header(ui: &mut Ui, open: &mut bool) {
    draw_icon_window_header(ui, "Find", ButtonIcon::Find, open);
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
    let mut painter = ui.painter().clone();
    painter.set_clip_rect(background.intersect(ui.ctx().content_rect()));
    let mut rounding = ui.visuals().window_corner_radius;
    rounding.sw = 0;
    rounding.se = 0;
    painter.rect_filled(background, rounding, foundation_block_bar());
    let font = TextStyle::Heading.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), font, text_dark());
    let group_width = TITLE_ICON_SIZE + TITLE_GAP + galley.size().x;
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.center().x - group_width * 0.5,
            rect.center().y - TITLE_ICON_SIZE * 0.5,
        ),
        Vec2::splat(TITLE_ICON_SIZE),
    );
    paint_button_icon_at(ui, icon, icon_rect, text_dark());
    ui.painter().galley(
        egui::pos2(
            icon_rect.right() + TITLE_GAP,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        text_dark(),
    );

    if let Some(open) = open {
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 10.0, rect.center().y),
            Vec2::splat(20.0),
        );
        let close = ui
            .interact(
                close_rect,
                ui.id().with("window_header_close"),
                Sense::click(),
            )
            .on_hover_text(format!("Close {title}"));
        let color = ui.style().interact(&close).fg_stroke.color;
        let cross = close_rect.shrink(5.0);
        ui.painter().line_segment(
            [cross.left_top(), cross.right_bottom()],
            Stroke::new(1.5_f32, color),
        );
        ui.painter().line_segment(
            [cross.right_top(), cross.left_bottom()],
            Stroke::new(1.5_f32, color),
        );
        if close.clicked() {
            *open = false;
        }
    }
}

#[cfg(test)]
mod key_scope_tests {
    use super::*;

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
                ui.add(egui::TextEdit::singleline(&mut text).id(egui::Id::new("a_field")));
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
        let field = Some(egui::Id::new("a_field"));
        let query = Some(egui::Id::new("find_query"));

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
        assert!(app.search.find.open, "Escape in a field must not close Find");

        // The same keys in the query box.
        frame(&mut app, &ctx, Vec::new(), query);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)], None);
        assert_eq!(app.search.find.active, Some(1), "Enter in the query box steps");
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
        let query = Some(egui::Id::new("find_query"));
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
}
