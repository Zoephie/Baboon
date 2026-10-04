//! Modeless Ctrl+F find-in-tag window.

use super::*;

/// Draw the modeless Find window; a changed query or a step to the next
/// or previous match is sent as a command.
pub(in crate::app) fn draw_find_window(cx: &Ctx, search: &mut SearchFeature) {
    if !search.find.open {
        return;
    }
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
                        egui::TextEdit::singleline(&mut search.find.query)
                            .id(egui::Id::new("find_query"))
                            .vertical_align(egui::Align::Center),
                    );
                    if search.find.focus_query {
                        response.request_focus();
                        if let Some(mut state) = egui::TextEdit::load_state(ctx, response.id) {
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(search.find.query.chars().count()),
                                )));
                            state.store(ctx, response.id);
                        }
                        search.find.focus_query = false;
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
                    if in_query && enter && !search.find.searching {
                        step = if shift { -1 } else { 1 };
                        // Keep the box focused so Enter keeps stepping.
                        response.request_focus();
                    }
                    query_escape = in_query && escape;
                    ui.end_row();

                    ui.label(RichText::new("within:").strong().color(text_dark()));
                    egui::ComboBox::from_id_salt("find_within")
                        .selected_text(search.find.within.label())
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(
                                    &mut search.find.within,
                                    FindWithin::CurrentTag,
                                    FindWithin::CurrentTag.label(),
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut search.find.within,
                                    FindWithin::OpenTags,
                                    FindWithin::OpenTags.label(),
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut search.find.within,
                                    FindWithin::AllTags,
                                    FindWithin::AllTags.label(),
                                )
                                .changed();
                        });
                    changed |= ui
                        .checkbox(&mut search.find.match_case, "match case")
                        .changed();
                    ui.end_row();

                    ui.label(RichText::new("look in:").strong().color(text_dark()));
                    egui::ComboBox::from_id_salt("find_look_in")
                        .selected_text(search.find.look_in.label())
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .checkbox(&mut search.find.look_in.field_names, "Field names")
                                .changed();
                            changed |= ui
                                .checkbox(&mut search.find.look_in.field_values, "Field values")
                                .changed();
                            changed |= ui
                                .checkbox(&mut search.find.look_in.blocks, "Blocks")
                                .changed();
                        });
                    changed |= ui
                        .checkbox(&mut search.find.whole_word, "match whole word")
                        .changed();
                    ui.end_row();
                });
            ui.add_space(8.0);
            if search.find.searching {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let text = search
                        .find
                        .progress
                        .map(|(done, total)| format!("searching… {done}/{total}"))
                        .unwrap_or_else(|| "preparing all-tag search…".to_owned());
                    ui.label(RichText::new(text).small().color(subtle_dark()));
                });
            } else if search.find.unreadable > 0 && search.find.within == FindWithin::AllTags {
                ui.label(
                    RichText::new(format!("{} tag(s) could not be read", search.find.unreadable))
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
                    search.find.filter_results,
                )
                .on_hover_text("Show only matching fields and blocks in the selected scope")
                .clicked()
                {
                    search.find.filter_results = !search.find.filter_results;
                    filter_changed = true;
                }
                let can_navigate = !search.find.occurrences.is_empty() && !search.find.searching;
                let counter = search
                    .find
                    .active
                    .map(|index| format!("{}/{}", index + 1, search.find.occurrences.len()))
                    .unwrap_or_else(|| format!("0/{}", search.find.occurrences.len()));
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
        cx.send(SearchCommand::FindChanged);
    }
    if filter_changed {
        ctx.request_repaint();
    }
    if step != 0 {
        cx.send(SearchCommand::FindStep(step));
    }
    if !open || query_escape {
        search.find.close();
        ctx.data_mut(|data| {
            data.remove::<std::sync::Arc<FindRenderSnapshot>>(find_render_snapshot_id())
        });
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
mod key_scope_tests;
