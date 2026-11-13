//! The single tag or folder Rename layout, styled like Bulk Rename: one name
//! field, an Original | Rename row, then the references to fix.
//! The windows that use it own their state and what applying does.

use super::*;
use crate::app::browser::bulk_rename::{
    RenameIcon, ScriptRows, draw_references_box, padded_body, references_box_height, references_to_fix_title, rename_table_header,
    rename_table_row, template,
};

/// Referring tags: display path and tag group.
pub(in crate::app) type ReferenceRows = Vec<(String, u32)>;

/// What one Rename window shows.
pub(in crate::app) struct SingleRename<'a> {
    /// The name being typed. `{name}` stands for `current`; on apply it is
    /// replaced by the expanded name, so the apply reads the final name.
    pub input: &'a mut String,
    /// Focus the field with the whole name selected on the next frame, so
    /// typing replaces it.
    pub focus: &'a mut bool,
    pub field_id: egui::Id,
    /// The current name, without an extension.
    pub current: &'a str,
    /// Where it is now, shown when hovering the original name.
    pub location: String,
    /// The tag's extension, shown after the name in the table; none for a folder.
    pub extension: Option<&'a str>,
    pub icon: RenameIcon,
    pub game: Option<GameId>,
    /// "Tag names" or "Folder names", for validation messages.
    pub noun: &'a str,
    /// A failure from the last apply, shown under the field.
    pub error: Option<&'a str>,
    /// Text about what the rename does, under the table: a sentence, and
    /// optionally a smaller detail.
    pub notes: Option<(String, Option<String>)>,
    /// The known referring tags (display path, tag group) and a note about
    /// the list, or `None` to leave the references box out.
    pub references: Option<(ReferenceRows, Vec<String>)>,
    /// The scripts with paths to fix, listed after the referring tags.
    pub scripts: ScriptRows,
    pub apply_hover: &'a str,
}

/// The name typed, with `{name}` standing for the current name, checked as
/// a valid name.
fn resolve_name(input: &str, current: &str, noun: &str) -> Result<String, String> {
    template(input.trim(), current)
        .and_then(|name| duplicate::validate_leaf_characters(&name, noun, "Enter a name"))
}

/// What the window asked for this frame.
#[derive(Default)]
pub(in crate::app) struct SingleRenameResponse {
    pub apply: bool,
    pub cancel: bool,
}

/// Draw a Rename window's contents, header to footer.
pub(in crate::app) fn draw_single_rename(cx: &Ctx, ui: &mut Ui, open: &mut bool, rename: SingleRename) -> SingleRenameResponse {
    let mut result = SingleRenameResponse::default();
    if crate::app::search::draw_icon_window_header_with_help(ui, "Rename", ButtonIcon::Rename, open,
        ("About Renaming", rename.field_id.with("about"))) {
        cx.send(crate::app::help::HelpCommand::ShowDocSection(crate::app::help::RENAMING_HELP_SECTION));
    }
    let (entered, expanded) = padded_body(ui, |ui| {
        ui.add_space(14.0);
        ui.label("Rename");
        // A tag's extension sits at the end of the field as muted hint text: it
        // is kept, not typed. The field's right margin keeps the name clear of it.
        let suffix = rename.extension.map(|extension| {
            let font = egui::TextStyle::Body.resolve(ui.style());
            ui.painter().layout_no_wrap(format!(".{extension}"), font, text_dark().gamma_multiply(0.5))
        });
        let right = suffix.as_ref().map_or(4.0, |galley| galley.size().x + 16.0).min(127.0) as i8;
        let response = ui.add(
            crate::app::ui_kit::singleline_text_edit(rename.input)
                .id(rename.field_id)
                .desired_width(f32::INFINITY)
                .margin(egui::Margin { left: 4, right, top: 2, bottom: 2 })
                .font(egui::TextStyle::Monospace),
        );
        if let Some(galley) = suffix {
            let at = egui::pos2(response.rect.right() - 8.0 - galley.size().x, response.rect.center().y - galley.size().y * 0.5);
            ui.painter().galley(at, galley, text_dark().gamma_multiply(0.5));
        }
        if std::mem::take(rename.focus) {
            response.request_focus();
            let mut text = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
            text.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(rename.input.chars().count()),
            )));
            text.store(ui.ctx(), response.id);
        }
        let entered = lost_focus_once(&response) && ui.input(|input| input.key_pressed(egui::Key::Enter));

        let expanded = resolve_name(rename.input, rename.current, rename.noun);
        let new_name = expanded.as_ref().ok().filter(|name| name.as_str() != rename.current);
        if let Some(error) = rename.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }

        ui.add_space(12.0);
        let width = ui.available_width();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let column = rename_table_header(ui, width);
            let with_extension = |name: &str| match rename.extension {
                Some(extension) => format!("{name}.{extension}"),
                None => name.to_owned(),
            };
            let original = with_extension(rename.current);
            let (renamed, color) = match &expanded {
                Err(error) => (error.clone(), ui.visuals().error_fg_color),
                Ok(name) if new_name.is_some() => (with_extension(name), foundation_blue()),
                Ok(name) => (with_extension(name), subtle_dark()),
            };
            rename_table_row(ui, width, column, rename.icon, rename.game, &original, &rename.location, &renamed, color);
        });
        if let Some((note, detail)) = &rename.notes {
            ui.add_space(10.0);
            ui.label(RichText::new(note).color(text_dark()));
            if let Some(detail) = detail {
                ui.label(RichText::new(detail).small().color(subtle_dark()));
            }
        }
        if let Some((rows, notes)) = &rename.references {
            ui.add_space(14.0);
            let max = (window_work_area(ui.ctx()).height() * 0.4).clamp(120.0, 320.0);
            let scripts = &rename.scripts;
            let height = references_box_height(scripts.total(rows.len()), notes.len() + scripts.note_lines(), max);
            draw_references_box(ui, &references_to_fix_title(scripts.total(rows.len())), rows, scripts, notes, rename.game, height);
        }

        (entered, expanded)
    });
    let new_name = expanded.as_ref().ok().filter(|name| name.as_str() != rename.current);
    ui.add_space(12.0);
    // Held until the scripts are checked, so the window has said what the
    // rename changes before it can run.
    let checking = rename.scripts.checking;
    let can_apply = new_name.is_some() && !checking;
    popup_footer(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked() {
                result.cancel = true;
            }
            if icon_text_button(ui, ButtonIcon::Rename, "Rename", can_apply)
                .on_hover_text(rename.apply_hover)
                .on_disabled_hover_text(if checking { ScriptRows::WAITING } else { rename.apply_hover })
                .clicked()
            {
                result.apply = true;
            }
        });
    });
    result.apply |= entered;
    result.apply &= can_apply;
    if result.apply {
        *rename.input = new_name.cloned().unwrap_or_default();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_name_token_expands_and_the_result_is_validated() {
        assert_eq!(resolve_name("drone", "bugger", "Tag names").as_deref(), Ok("drone"));
        assert_eq!(resolve_name("{name}_v2", "bugger", "Tag names").as_deref(), Ok("bugger_v2"));
        assert_eq!(resolve_name(" {name} ", "bugger", "Tag names").as_deref(), Ok("bugger"));
        assert!(resolve_name("{typo}", "bugger", "Tag names").is_err());
        assert!(resolve_name("", "bugger", "Tag names").is_err());
        assert!(resolve_name("a/b", "bugger", "Tag names").is_err());
    }
}
