//! Manage Keywords: every keyword in the user's order, each with its colour,
//! how many tags and folders in this kit carry it, and Search, Rename and
//! Delete. Rename and Delete reach every game's keywords, so a merge or a
//! delete asks first ([`KeywordChangeConfirm`]).
//! The window owns presentation and its drafts; every change is a
//! [`BrowserCommand`].

use super::*;
use crate::core::keyword_catalog::{KeywordColour, keyword_key};

const ROW_HEIGHT: f32 = 34.0;
const PILL_HEIGHT: f32 = 22.0;

/// The Manage Keywords window.
#[derive(Default)]
pub(in crate::app) struct KeywordManager {
    /// The keyword being renamed (its key) and the name typed so far.
    renaming: Option<(String, String)>,
    /// Add New Keyword's name, while it is being typed.
    adding: Option<String>,
    /// A field opened this frame takes the focus once.
    focus_field: bool,
}

impl KeywordManager {
    pub(in crate::app) fn new() -> Self {
        Self::default()
    }
}

/// Drag state: the row picked up, kept in egui's memory while dragging.
#[derive(Clone, Copy)]
struct Dragging(usize);

impl Dialog for KeywordManager {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let catalog = &cx.model.keyword_catalog;
        let store = &cx.model.kits[cx.model.active].keywords;
        // Counts are this kit's: what its search finds.
        let root = cx.model.loaded_tags_root_for(cx.model.active);
        let window_id = egui::Id::unique("keyword_manager");
        let drag_id = window_id.with("dragging");
        egui::Window::new("Manage Keywords")
            .constrain_to(window_work_area(ctx))
            .id(window_id)
            .open(&mut open)
            .collapsible(false)
            .default_width(window_width(ctx, 600.0))
            .show(ctx, |ui| {
                let font = egui::TextStyle::Button.resolve(ui.style());
                let dragging = ui.data(|data| data.get_temp::<Dragging>(drag_id));
                let mut row_rects: Vec<egui::Rect> = Vec::new();
                let mut drag_released = None;
                crate::app::ScrollArea::vertical()
                    .max_height(480.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        if catalog.entries().is_empty() {
                            ui.label(
                                RichText::new(
                                    "No keywords yet. Add one below, or on a tag or folder's Keywords bar.",
                                )
                                .color(subtle_dark()),
                            );
                        }
                        for (index, entry) in catalog.entries().iter().enumerate() {
                            let key = entry.key();
                            let count = store.count_in_kit(&key, root.as_deref());
                            let row = keyword_row_frame(ui).show(ui, |ui| {
                                ui.set_min_height(ROW_HEIGHT - 12.0);
                                ui.horizontal(|ui| {
                                    let handle = drag_handle(ui, window_id.with(("handle", &key)));
                                    if handle.drag_started() {
                                        ui.data_mut(|data| data.insert_temp(drag_id, Dragging(index)));
                                    }
                                    if handle.drag_stopped() {
                                        drag_released = Some(index);
                                    }
                                    self.draw_name(cx, ui, &font, entry, &key);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if icon_button(ui, ButtonIcon::Clear, "Delete keyword", true, text_dark())
                                            .clicked()
                                        {
                                            cx.send(BrowserCommand::DeleteKeyword(key.clone()));
                                        }
                                        if icon_button(ui, ButtonIcon::Edit, "Rename keyword", true, text_dark())
                                            .clicked()
                                        {
                                            self.renaming = Some((key.clone(), entry.name.clone()));
                                            self.adding = None;
                                            self.focus_field = true;
                                        }
                                        if icon_button(
                                            ui,
                                            ButtonIcon::Find,
                                            "Show the tags and folders with this keyword",
                                            true,
                                            text_dark(),
                                        )
                                        .clicked()
                                        {
                                            cx.send(BrowserCommand::SearchKeyword(key.clone()));
                                        }
                                        ui.add_space(6.0);
                                        ui.label(RichText::new(count.to_string()).color(subtle_dark()))
                                            .on_hover_text(format!(
                                                "Tags and folders in this kit with \u{201c}{}\u{201d}",
                                                entry.name
                                            ));
                                    });
                                });
                            });
                            row_rects.push(row.response.rect);
                        }
                        self.draw_add_row(cx, ui);
                    });
                // Where a dragged row would land: before the row under the
                // pointer's upper half, after its lower half.
                if let Some(Dragging(from)) = dragging {
                    let pointer = ui.ctx().pointer_interact_pos();
                    let slot = pointer.map(|pos| {
                        row_rects
                            .iter()
                            .position(|rect| pos.y < rect.center().y)
                            .unwrap_or(row_rects.len())
                    });
                    if let Some(slot) = slot {
                        let y = match (row_rects.get(slot), slot.checked_sub(1).and_then(|at| row_rects.get(at))) {
                            (Some(next), Some(previous)) => (previous.bottom() + next.top()) / 2.0,
                            (Some(next), None) => next.top() - 3.0,
                            (None, Some(previous)) => previous.bottom() + 3.0,
                            (None, None) => ui.min_rect().top(),
                        };
                        if let Some(first) = row_rects.first() {
                            ui.painter().line_segment(
                                [egui::pos2(first.left(), y), egui::pos2(first.right(), y)],
                                Stroke::new(2.0, ui.visuals().selection.stroke.color),
                            );
                        }
                        if drag_released.is_some() {
                            let to = if slot > from { slot - 1 } else { slot };
                            if to != from {
                                cx.send(BrowserCommand::MoveKeyword { from, to });
                            }
                        }
                    }
                    if drag_released.is_some() || !ui.input(|input| input.pointer.any_down()) {
                        ui.data_mut(|data| data.remove::<Dragging>(drag_id));
                    }
                }
            });
        open
    }
}

/// A row's box, as in the mockup: a faint fill and border.
fn keyword_row_frame(ui: &Ui) -> Frame {
    Frame::NONE
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 6))
}

/// Six dots to drag a row by.
fn drag_handle(ui: &mut Ui, id: egui::Id) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(16.0, PILL_HEIGHT), Sense::hover());
    let response = ui.interact(rect, id, Sense::drag()).on_hover_text("Drag to reorder");
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let colour = if response.hovered() || response.dragged() { text_dark() } else { subtle_dark() };
    for column in [-3.0, 3.0] {
        for row in [-6.0, 0.0, 6.0] {
            ui.painter()
                .circle_filled(rect.center() + Vec2::new(column, row), 1.6, colour);
        }
    }
    response
}

impl KeywordManager {
    /// The keyword's pill, which opens its colours, or the rename field.
    fn draw_name(
        &mut self,
        cx: &Ctx,
        ui: &mut Ui,
        font: &FontId,
        entry: &crate::core::keyword_catalog::KeywordInfo,
        key: &str,
    ) {
        if let Some((renaming, draft)) = self.renaming.as_mut()
            && renaming == key
        {
            let field = ui.add(
                crate::app::ui_kit::singleline_text_edit(draft)
                    .desired_width(200.0)
                    .hint_text(placeholder_text("keyword name")),
            );
            if std::mem::take(&mut self.focus_field) {
                field.request_focus();
            }
            let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if entered {
                let new = draft.trim().to_owned();
                if !new.is_empty() && new != entry.name {
                    cx.send(BrowserCommand::RenameKeyword { old: key.to_owned(), new });
                }
                self.renaming = None;
            } else if field.lost_focus() {
                self.renaming = None;
            }
            return;
        }
        let colour = entry.colour;
        let width = crate::app::ui_kit::keyword_pill_width(ui.painter(), &entry.name, font, colour, 0.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, PILL_HEIGHT), Sense::click());
        crate::app::ui_kit::paint_keyword_pill(ui.painter(), rect, &entry.name, font, colour);
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let response = response.on_hover_text("Pick a colour");
        let popup_id = ui.make_persistent_id(("keyword_colour", key));
        if response.clicked() {
            egui::Popup::toggle_id(ui.ctx(), popup_id);
        }
        egui::Popup::from_response(&response)
            .id(popup_id)
            .open_memory(None)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .show(|ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // No colour draws as grey, so Grey stands for both.
                    let picked = colour.unwrap_or(KeywordColour::Grey);
                    for choice in KeywordColour::ALL {
                        let swatch = colour_swatch(ui, choice, picked == choice)
                            .on_hover_text(choice.label());
                        if swatch.clicked() {
                            cx.send(BrowserCommand::SetKeywordColour {
                                keyword: key.to_owned(),
                                colour: Some(choice),
                            });
                        }
                    }
                });
            });
    }

    /// Add New Keyword, or its name field while one is typed.
    fn draw_add_row(&mut self, cx: &Ctx, ui: &mut Ui) {
        keyword_row_frame(ui).show(ui, |ui| {
            ui.set_min_height(ROW_HEIGHT - 12.0);
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                match self.adding.as_mut() {
                    Some(draft) => {
                        let field = ui.add(
                            crate::app::ui_kit::singleline_text_edit(draft)
                                .desired_width(200.0)
                                .hint_text(placeholder_text("new keyword")),
                        );
                        if std::mem::take(&mut self.focus_field) {
                            field.request_focus();
                        }
                        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        if entered {
                            let name = draft.trim().to_owned();
                            if !name.is_empty() {
                                cx.send(BrowserCommand::NewKeyword(name));
                            }
                            self.adding = None;
                        } else if field.lost_focus() {
                            self.adding = None;
                        }
                    }
                    None => {
                        if icon_text_button(ui, ButtonIcon::Add, "Add New Keyword", true).clicked() {
                            self.adding = Some(String::new());
                            self.renaming = None;
                            self.focus_field = true;
                        }
                    }
                }
            });
        });
    }
}

/// One colour in a keyword's colour picker.
fn colour_swatch(ui: &mut Ui, colour: KeywordColour, picked: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    let colours = keyword_pill_colours(Some(colour));
    let circle = rect.shrink(3.0);
    ui.painter().circle(
        circle.center(),
        circle.width() / 2.0,
        colours.dot.unwrap_or(colours.fill),
        Stroke::new(1.0, colours.border.unwrap_or(subtle_dark())),
    );
    if picked || response.hovered() {
        ui.painter().circle_stroke(
            rect.center(),
            rect.width() / 2.0 - 0.5,
            Stroke::new(1.5, text_dark()),
        );
    }
    response
}

/// Asks before a keyword change that reaches every game and can't be undone:
/// deleting a keyword, or renaming it onto another (a merge).
pub(in crate::app) struct KeywordChangeConfirm {
    /// The keyword going (its key) and how it is spelled.
    pub(in crate::app) old: String,
    pub(in crate::app) old_name: String,
    /// The keyword it merges into, spelled; `None` deletes it.
    pub(in crate::app) into: Option<String>,
    /// Tags and folders carrying it in the active editing kit.
    pub(in crate::app) here: usize,
    /// Tags and folders carrying it in every editing kit, of every game.
    pub(in crate::app) everywhere: usize,
}

/// "1 tag or folder", "3 tags and folders".
fn tags_and_folders(count: usize) -> String {
    match count {
        1 => "1 tag or folder".to_owned(),
        count => format!("{count} tags and folders"),
    }
}

impl Dialog for KeywordChangeConfirm {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut decided = None;
        let (title, question, button) = match &self.into {
            None => (
                "Delete Keyword?",
                format!(
                    "Remove \u{201c}{}\u{201d} from every tag and folder that carries it?",
                    self.old_name
                ),
                "Delete Keyword",
            ),
            Some(into) => (
                "Merge Keywords?",
                format!(
                    "Merge \u{201c}{}\u{201d} into \u{201c}{into}\u{201d}? Every tag and folder carrying \u{201c}{}\u{201d} gets \u{201c}{into}\u{201d} instead.",
                    self.old_name, self.old_name
                ),
                "Merge",
            ),
        };
        let here = tags_and_folders(self.here);
        let everywhere = tags_and_folders(self.everywhere);
        let elsewhere = self.everywhere > self.here;
        egui::Window::new(title)
            .id(egui::Id::unique("keyword_change_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(window_width(ctx, 420.0))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new(question).color(text_dark()));
                ui.add_space(6.0);
                egui::Grid::new("keyword_change_counts")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("This editing kit").color(subtle_dark()));
                        ui.label(RichText::new(here).color(text_dark()));
                        ui.end_row();
                        ui.label(RichText::new("All your editing kits").color(subtle_dark()));
                        ui.label(RichText::new(everywhere).strong().color(text_dark()));
                        ui.end_row();
                    });
                if elsewhere {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Other editing kits use it too, and change with this one.")
                            .color(text_dark()),
                    );
                }
                ui.add_space(6.0);
                ui.label(RichText::new("This can't be undone.").color(replace_warning_text()));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(button).clicked() {
                        decided = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decided = Some(false);
                    }
                });
            });
        if decided == Some(true) {
            cx.send(BrowserCommand::ChangeKeywordEverywhere {
                old: self.old.clone(),
                new: self.into.clone(),
            });
        }
        open && decided.is_none()
    }
}

impl Baboon {
    /// Every game's keyword file.
    fn keyword_sidecars() -> Vec<PathBuf> {
        GameId::ALL
            .iter()
            .map(|game| crate::core::source::keywords_path(game.as_str()))
            .collect()
    }

    /// How many of the active kit's tags and folders carry `keyword`.
    fn keywords_here(&self, keyword: &str) -> usize {
        let active = self.model.active;
        self.model.kits[active]
            .keywords
            .count_in_kit(keyword, self.model.loaded_tags_root_for(active).as_deref())
    }

    /// How many tags and folders carry `keyword` in every game: an open
    /// kit's in-memory keywords where one has that game's file, else the
    /// file itself.
    pub(in crate::app) fn keyword_uses_everywhere(&self, keyword: &str) -> usize {
        Self::keyword_sidecars()
            .iter()
            .map(|path| {
                match self.model.kits.iter().find(|kit| kit.keywords.path() == Some(path.as_path())) {
                    Some(kit) => kit.keywords.count_of(keyword),
                    None => crate::core::keywords::count_in_sidecar(path, keyword),
                }
            })
            .sum::<usize>()
            + self
                .model
                .kits
                .iter()
                .filter(|kit| kit.keywords.path().is_none())
                .map(|kit| kit.keywords.count_of(keyword))
                .sum::<usize>()
    }

    /// Rename `old` to `new` (`None` deletes it) on every tag and folder in
    /// every game, and in the keyword list. An open kit changes its own
    /// keywords, which it saves with its other edits; a game with no kit
    /// open has its file rewritten now.
    pub(in crate::app) fn change_keyword_everywhere(&mut self, old: &str, new: Option<&str>) {
        let same_key = new.is_some_and(|new| keyword_key(new) == keyword_key(old));
        let mut problem = None;
        if !same_key {
            for path in Self::keyword_sidecars() {
                let mut open = false;
                for kit in &mut self.model.kits {
                    if kit.keywords.path() == Some(path.as_path()) {
                        kit.keywords.replace_keyword(old, new);
                        open = true;
                    }
                }
                if !open
                    && let Err(error) = crate::core::keywords::replace_in_sidecar(&path, old, new)
                {
                    problem = Some(error);
                }
            }
            // A kit with no game keeps its keywords in memory only.
            for kit in &mut self.model.kits {
                if kit.keywords.path().is_none() {
                    kit.keywords.replace_keyword(old, new);
                }
            }
        }
        // Search boxes holding it follow: renamed, or taken off.
        let old_key = keyword_key(old);
        let new_key = new.map(keyword_key);
        for view in self.views.all_mut() {
            let browser = &mut view.browser;
            for pills in std::iter::once(&mut browser.keyword_terms)
                .chain(browser.folder_browsers.values_mut().map(|pane| &mut pane.keyword_terms))
            {
                if let Some(at) = pills.iter().position(|pill| *pill == old_key) {
                    match &new_key {
                        Some(new_key) if !pills.contains(new_key) => pills[at] = new_key.clone(),
                        _ => {
                            pills.remove(at);
                        }
                    }
                }
            }
        }
        let catalog = &mut self.model.keyword_catalog;
        let old_name = catalog.display(old).to_owned();
        match new {
            Some(new) => {
                let merged = catalog.rename(old, new);
                let into = catalog.display(new).to_owned();
                self.model.status = if merged {
                    format!("Merged \u{201c}{old_name}\u{201d} into \u{201c}{into}\u{201d}")
                } else {
                    format!("Renamed \u{201c}{old_name}\u{201d} to \u{201c}{into}\u{201d}")
                };
            }
            None => {
                catalog.remove(old);
                self.model.status = format!("Deleted keyword \u{201c}{old_name}\u{201d}");
            }
        }
        if let Some(problem) = problem {
            self.model.status = problem;
        }
    }

    /// Manage Keywords' Rename: straight away, unless it lands on another
    /// keyword, which merges them and asks first.
    pub(in crate::app) fn rename_keyword(&mut self, old: &str, new: &str) {
        let catalog = &self.model.keyword_catalog;
        let merges = catalog
            .position(new)
            .is_some_and(|into| Some(into) != catalog.position(old));
        if merges {
            let confirm = KeywordChangeConfirm {
                old: keyword_key(old),
                old_name: catalog.display(old).to_owned(),
                into: Some(catalog.display(new).to_owned()),
                here: self.keywords_here(old),
                everywhere: self.keyword_uses_everywhere(old),
            };
            self.dialogs.open(confirm);
        } else {
            self.change_keyword_everywhere(old, Some(new));
        }
    }

    /// Manage Keywords' Search: the kit's root in a folder window, its search
    /// box holding just this keyword's pill. An open root window is reused.
    pub(in crate::app) fn search_keyword(&mut self, keyword: &str, ctx: &egui::Context) {
        self.handle_browser_action(
            BrowserAction::OpenFolderBrowser {
                rel_path: PathBuf::new(),
                label: "tags".into(),
                open_in_new_tab: false,
            },
            ctx.clone(),
        );
        let panes = &mut self.views[self.model.kits[self.model.active].id].browser.folder_browsers;
        let base = folder_pane_key(Path::new(""));
        let key = if panes.get(&base).is_some_and(|pane| pane.rel_path.as_os_str().is_empty()) {
            Some(base)
        } else {
            let mut roots: Vec<String> = panes
                .iter()
                .filter(|(_, pane)| pane.rel_path.as_os_str().is_empty())
                .map(|(key, _)| key.clone())
                .collect();
            roots.sort();
            roots.into_iter().next()
        };
        if let Some(pane) = key.and_then(|key| panes.get_mut(&key)) {
            pane.filter.clear();
            pane.keyword_terms = vec![keyword_key(keyword)];
            pane.search_scope.keywords = true;
            pane.assets_view = false;
            pane.selection.clear();
            pane.selected_tag = None;
        }
    }

    /// Manage Keywords' Delete: asks first when anything carries it.
    pub(in crate::app) fn delete_keyword(&mut self, keyword: &str) {
        let everywhere = self.keyword_uses_everywhere(keyword);
        if everywhere == 0 {
            self.change_keyword_everywhere(keyword, None);
            return;
        }
        let confirm = KeywordChangeConfirm {
            old: keyword_key(keyword),
            old_name: self.model.keyword_catalog.display(keyword).to_owned(),
            into: None,
            here: self.keywords_here(keyword),
            everywhere,
        };
        self.dialogs.open(confirm);
    }
}
