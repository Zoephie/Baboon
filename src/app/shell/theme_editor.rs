//! The Theme Editor: every token of the theme in use, grouped as the token
//! table groups them, each with its colour, where the colour comes from and a
//! reset. Edits are drawn live; Cancel puts back what was drawn when the
//! editor opened. Its Theme menu chooses another theme to use and edit, as
//! Settings' does, asking first when there are edits to lose. Save writes a
//! theme file back; Save As writes a new theme that extends the one edited and
//! holds only what changed. The draft and its files are
//! `ui_kit::themes::draft`.

use super::*;
use crate::app::editor::{MaterialColorPopup, draw_color_picker_editor};
use crate::app::prefs::ColorPaletteSwatch;
use crate::app::ui_kit::themes::draft::{RowState, SaveError, ThemeDraft};
use crate::app::ui_kit::themes::library::ThemeEntry;
use crate::app::ui_kit::themes::{self, Token, state::with_state};

/// A colour being picked for one row, and what the row held before, for the
/// picker's Cancel.
struct Picking {
    token: Token,
    popup: MaterialColorPopup,
    held: Option<String>,
}

/// The Theme Editor window.
pub(in crate::app) struct ThemeEditorWindow {
    /// The draft, or why the theme in use cannot be edited.
    draft: Result<ThemeDraft, String>,
    search: String,
    picking: Option<Picking>,
    swatches: Vec<Option<ColorPaletteSwatch>>,
    /// The name typed for Save As, while it is being asked.
    save_as: Option<String>,
    /// Save found the file changed on disk, and asks before overwriting.
    confirm_overwrite: bool,
    /// The last save's outcome, or why it failed.
    message: Option<String>,
    /// A theme chosen from the Theme menu while there are edits, until
    /// switching to it is confirmed.
    switching: Option<ThemeEntry>,
    /// The Theme menu is previewing the theme under its pointer.
    previewing: bool,
}

impl ThemeEditorWindow {
    /// An editor on the theme in use.
    pub(in crate::app) fn for_active_theme() -> Self {
        let draft = with_state(|state| {
            let id = state
                .applied_id()
                .unwrap_or(themes::BuiltIn::Light.id())
                .to_owned();
            ThemeDraft::open(state.current_library().clone(), &id, themes::installed())
        })
        .map_err(|report| format!("The theme in use cannot be edited: {report}"));
        Self {
            draft,
            search: String::new(),
            picking: None,
            swatches: Vec::new(),
            save_as: None,
            confirm_overwrite: false,
            message: None,
            switching: None,
            previewing: false,
        }
    }

    /// Chooses `entry` as the theme in use and edits it in place of the
    /// draft, whose edits are dropped.
    fn switch_to(&mut self, cx: &Ctx, entry: ThemeEntry) {
        let draft = with_state(|state| {
            ThemeDraft::open(
                state.current_library().clone(),
                &entry.id,
                themes::installed(),
            )
        })
        .map(ThemeDraft::chosen)
        .map_err(|report| format!("{} cannot be edited: {report}", entry.name));
        // Drawn now, before the preferences apply it on a later frame.
        if let Ok(draft) = &draft {
            themes::install(draft.colors());
        }
        cx.set_status(format!("Switched to the theme {}", entry.name));
        cx.edit_prefs(move |prefs| super::settings::choose_theme(prefs, &entry));
        self.draft = draft;
        self.picking = None;
        self.save_as = None;
        self.confirm_overwrite = false;
        self.message = None;
        self.switching = None;
    }
}

/// The Theme menu: every theme that can be applied, the one edited chosen.
/// Returns the theme picked, and sets `hovered` to the one under the pointer.
fn draw_theme_menu(
    ui: &mut Ui,
    entries: &[ThemeEntry],
    editing: Option<&str>,
    hovered: &mut Option<String>,
) -> Option<ThemeEntry> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label("Theme");
        let selected = editing.map_or_else(
            || "None".to_owned(),
            |id| super::settings::theme_label_of(entries, id),
        );
        crate::app::ui_kit::ValueComboBox::from_id_salt("theme_editor_theme")
            .width(240.0)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for entry in entries {
                    ui.add_enabled_ui(!entry.broken, |ui| {
                        let chosen = editing == Some(entry.id.as_str());
                        let response =
                            ui.selectable_label(chosen, super::settings::theme_label(entry));
                        if response.hovered() {
                            *hovered = Some(entry.id.clone());
                        }
                        if response.clicked() && !chosen {
                            picked = Some(entry.clone());
                        }
                    });
                }
            });
    });
    picked
}

impl Dialog for ThemeEditorWindow {
    fn show(&mut self, cx: &Ctx, _app: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut cancel = false;
        // The draft is what is drawn while the editor is open, unless the
        // Theme menu is previewing another: installing an unchanged theme
        // does nothing.
        if !self.previewing
            && let Ok(draft) = &self.draft
        {
            themes::install(draft.colors());
        }
        // The folder is read again at most once a second, so a theme added
        // while the editor is open appears in its menu.
        let entries = with_state(|state| state.entries(std::time::Instant::now()));
        let mut hovered = None;
        let mut picked = None;
        egui::Window::new("Theme Editor")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::unique("theme_editor"))
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_width(window_width(ctx, 720.0))
            .default_height(window_height(ctx, 820.0, false))
            .show(ctx, |ui| {
                let title = match &self.draft {
                    Ok(draft) => format!("Theme Editor - {}", draft.name()),
                    Err(_) => "Theme Editor".to_owned(),
                };
                crate::app::search::draw_icon_window_header(
                    ui,
                    &title,
                    ButtonIcon::ColorPicker,
                    &mut open,
                );
                ui.add_space(6.0);
                let editing = match &self.draft {
                    Ok(draft) => Some(draft.id().to_owned()),
                    Err(_) => with_state(|state| state.applied_id().map(str::to_owned)),
                };
                picked = draw_theme_menu(ui, &entries, editing.as_deref(), &mut hovered);
                if let Some(entry) = self.switching.clone() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Switching to {} drops your unsaved edits.",
                                entry.name
                            ))
                            .color(unapplied_value_text()),
                        );
                        if ui.button("Drop Edits and Switch").clicked() {
                            picked = Some(entry.clone());
                        }
                        if ui.button("Keep Editing").clicked() {
                            self.switching = None;
                        }
                    });
                }
                ui.add_space(4.0);
                match &mut self.draft {
                    Err(error) => {
                        ui.label(RichText::new(error.as_str()).color(error_text()));
                    }
                    Ok(draft) => {
                        cancel = draw_editor(
                            cx,
                            ui,
                            draft,
                            &mut self.search,
                            &mut self.picking,
                            &mut self.swatches,
                            &mut self.save_as,
                            &mut self.confirm_overwrite,
                            &mut self.message,
                            &mut open,
                        );
                    }
                }
            });
        // A theme picked while there are edits waits to be confirmed; picking
        // another from the menu meanwhile waits in its place.
        if let Some(entry) = picked {
            let dirty = self.draft.as_ref().is_ok_and(ThemeDraft::is_dirty);
            if dirty && self.switching.as_ref() != Some(&entry) {
                self.switching = Some(entry);
            } else {
                self.switch_to(cx, entry);
            }
        }
        // The theme under the menu's pointer is drawn until it leaves, then
        // the draft again.
        match hovered {
            Some(id) => {
                with_state(|state| state.preview(Some(&id)));
                self.previewing = true;
            }
            None if self.previewing => {
                with_state(|state| state.preview(None));
                self.previewing = false;
                if let Ok(draft) = &self.draft {
                    themes::install(draft.colors());
                }
            }
            None => {}
        }
        // Closing without saving is Cancel.
        if (!open || cancel) && self.previewing {
            with_state(|state| state.preview(None));
        }
        if (!open || cancel)
            && let Ok(draft) = &self.draft
        {
            themes::install(draft.before());
            if draft.is_dirty() {
                cx.set_status("Theme edits cancelled");
            }
        }
        open && !cancel
    }
}

/// The editor's body. Returns whether Cancel was pressed.
#[allow(clippy::too_many_arguments)]
fn draw_editor(
    cx: &Ctx,
    ui: &mut Ui,
    draft: &mut ThemeDraft,
    search: &mut String,
    picking: &mut Option<Picking>,
    swatches: &mut Vec<Option<ColorPaletteSwatch>>,
    save_as: &mut Option<String>,
    confirm_overwrite: &mut bool,
    message: &mut Option<String>,
    open: &mut bool,
) -> bool {
    let mut cancel = false;
    if draft.is_built_in() {
        ui.label(
            RichText::new("A built-in theme is kept with Save As, as a new theme that extends it.")
                .color(subtle_dark()),
        );
    }
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.add(
            egui::TextEdit::singleline(search)
                .hint_text("id, description or alias")
                .desired_width(260.0),
        );
    });
    let warnings = themes::contrast::warnings(draft.colors());
    if !warnings.is_empty() {
        ui.label(RichText::new("Hard to read:").color(settings_warning_text()));
        for warning in &warnings {
            ui.label(RichText::new(format!("  {warning}")).color(settings_warning_text()));
        }
    }
    ui.add_space(4.0);

    if let Some(pick) = picking {
        let mut close = None;
        super::settings::settings_card(ui, |ui| {
            ui.label(RichText::new(pick.token.id()).strong());
            draw_color_picker_editor(ui, &mut pick.popup, swatches);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    close = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    close = Some(false);
                }
            });
        });
        let color = pick.popup.color32();
        if color != draft.color(pick.token) && close != Some(false) {
            draft.set(pick.token, color);
        }
        match close {
            Some(true) => *picking = None,
            Some(false) => {
                draft.write(pick.token, pick.held.clone());
                *picking = None;
            }
            None => {}
        }
        ui.add_space(4.0);
    }

    let footer = 64.0;
    egui::ScrollArea::vertical()
        .id_salt("theme_editor_rows")
        .max_height((ui.available_height() - footer).max(120.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let needle = search.trim().to_lowercase();
            let mut group = "";
            for token in Token::ALL {
                if !token_matches(*token, &needle) {
                    continue;
                }
                if token.group() != group {
                    group = token.group();
                    ui.add_space(6.0);
                    ui.label(RichText::new(group).strong());
                    ui.separator();
                }
                draw_row(ui, draft, *token, picking);
            }
        });

    ui.separator();
    if let Some(text) = message.as_deref() {
        ui.label(RichText::new(text).color(subtle_dark()));
    }
    if *confirm_overwrite {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("The file changed on disk since the editor read it.")
                    .color(unapplied_value_text()),
            );
            if ui.button("Overwrite").clicked() {
                *confirm_overwrite = false;
                *message = Some(save(draft, true, confirm_overwrite));
            }
            if ui.button("Keep the file").clicked() {
                *confirm_overwrite = false;
            }
        });
    } else if let Some(name) = save_as {
        let mut done = false;
        ui.horizontal(|ui| {
            ui.label("New theme's name");
            let response = ui.add(egui::TextEdit::singleline(name).desired_width(220.0));
            let enter =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button("Save").clicked() || enter {
                let folder = themes::state::themes_folder();
                match draft.save_as(&folder, name) {
                    Ok((id, path)) => {
                        let dark = draft.colors().appearance.is_dark();
                        with_state(|state| state.reload(std::time::Instant::now()));
                        cx.edit_prefs(move |prefs| {
                            if dark {
                                prefs.dark_theme = id;
                            } else {
                                prefs.light_theme = id;
                            }
                            if !prefs.follow_system_theme {
                                prefs.dark_mode = dark;
                            }
                        });
                        cx.set_status(format!(
                            "Saved the theme as {} and chose it",
                            path.display()
                        ));
                        // The new theme is now the one in use, as drawn.
                        *open = false;
                        done = true;
                    }
                    Err(error) => *message = Some(format!("Could not save the theme: {error}")),
                }
            }
            if ui.button("Back").clicked() {
                done = true;
            }
        });
        if done {
            *save_as = None;
        }
    } else {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    draft.can_save() && draft.is_dirty(),
                    egui::Button::new("Save"),
                )
                .clicked()
            {
                *message = Some(save(draft, false, confirm_overwrite));
            }
            if ui.button("Save As...").clicked() {
                *save_as = Some(format!("{} (custom)", draft.name()));
            }
            if ui
                .button("Gallery")
                .on_hover_text("One of everything, in the theme as edited")
                .clicked()
            {
                cx.send(crate::app::context::Command::OpenDialog(Box::new(
                    super::ThemeGalleryWindow::default(),
                )));
            }
            let label = if draft.is_dirty() { "Cancel" } else { "Close" };
            if ui.button(label).clicked() {
                cancel = true;
            }
        });
    }
    cancel
}

/// Saves the draft, and says what came of it.
fn save(draft: &mut ThemeDraft, overwrite: bool, confirm_overwrite: &mut bool) -> String {
    match draft.save(overwrite) {
        Ok(path) => {
            // The theme in use was read from the file just written.
            with_state(|state| state.reload(std::time::Instant::now()));
            format!("Saved {}", path.display())
        }
        Err(SaveError::ChangedOnDisk(_)) => {
            *confirm_overwrite = true;
            String::new()
        }
        Err(error) => format!("Could not save the theme: {error}"),
    }
}

/// One token: its swatch, id, where its colour comes from, its value and a
/// reset.
fn draw_row(ui: &mut Ui, draft: &mut ThemeDraft, token: Token, picking: &mut Option<Picking>) {
    let state = draft.state(token);
    let color = draft.color(token);
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(36.0, 18.0), egui::Sense::click());
        ui.painter().rect_filled(rect, 3.0, color);
        ui.painter().rect_stroke(
            rect,
            3.0,
            ui.visuals().widgets.inactive.fg_stroke,
            egui::StrokeKind::Inside,
        );
        let response = response.on_hover_text("Pick a colour");
        if response.clicked() {
            let [r, g, b, a] = color
                .to_srgba_unmultiplied()
                .map(|channel| f32::from(channel) / 255.0);
            *picking = Some(Picking {
                token,
                popup: MaterialColorPopup::new(token.id(), r, g, b, a),
                held: match &state {
                    RowState::Set(value) => Some(value.clone()),
                    _ => None,
                },
            });
        }
        let aliases: Vec<&str> = themes::aliases::of(token).collect();
        let hover = if aliases.is_empty() {
            token.description().to_owned()
        } else {
            format!("{}\nUsed as: {}", token.description(), aliases.join(", "))
        };
        cell(
            ui,
            250.0,
            egui::Label::new(RichText::new(token.id()).monospace()).truncate(),
        )
        .on_hover_text(hover);
        let (where_from, value) = match &state {
            RowState::Set(value) => ("set".to_owned(), value.clone()),
            RowState::Following(rule) => (
                format!("follows {rule}"),
                themes::export::straight_hex(color),
            ),
            RowState::Inherited => ("inherited".to_owned(), themes::export::straight_hex(color)),
        };
        cell(ui, 90.0, egui::Label::new(RichText::new(value).monospace()));
        cell(
            ui,
            220.0,
            egui::Label::new(RichText::new(&where_from).color(subtle_dark())).truncate(),
        )
        .on_hover_text(where_from);
        let set = matches!(state, RowState::Set(_));
        if ui
            .add_enabled(set, egui::Button::new("Reset"))
            .on_hover_text("Inherit this colour again, or follow what it is derived from")
            .clicked()
        {
            draft.reset(token);
        }
    });
}

/// Whether `token` matches the lowercased search `needle`: by id,
/// description, group or alias. An empty search matches everything.
fn token_matches(token: Token, needle: &str) -> bool {
    needle.is_empty()
        || token.id().contains(needle)
        || token.description().to_lowercase().contains(needle)
        || token.group().to_lowercase().contains(needle)
        || themes::aliases::of(token).any(|alias| alias.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_a_token_by_its_alias() {
        let found: Vec<Token> = Token::ALL
            .iter()
            .copied()
            .filter(|token| token_matches(*token, "save_error"))
            .collect();
        assert_eq!(found, [Token::StatusError]);
        assert!(!token_matches(Token::Text, "save_error"));
    }
}

#[cfg(test)]
impl ThemeEditorWindow {

    /// Sets `token` to `color` and opens the picker on `picked`, as using the
    /// window would: for screenshots.
    pub(in crate::app) fn edit_for_shot(
        &mut self,
        token: Token,
        color: egui::Color32,
        picked: Token,
    ) {
        let Ok(draft) = &mut self.draft else { return };
        draft.set(token, color);
        let [r, g, b, a] = draft
            .color(picked)
            .to_srgba_unmultiplied()
            .map(|channel| f32::from(channel) / 255.0);
        self.picking = Some(Picking {
            token: picked,
            popup: MaterialColorPopup::new(picked.id(), r, g, b, a),
            held: None,
        });
    }
}

/// `widget` at the left of a column `width` wide.
fn cell(ui: &mut Ui, width: f32, widget: impl egui::Widget) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 18.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.add(widget)
        },
    )
    .inner
}
