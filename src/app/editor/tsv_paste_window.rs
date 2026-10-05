//! The TSV paste window: tab-separated rows pasted into a block.
//! It owns presentation and the pasted text; applying the rows belongs elsewhere.

use super::*;

/// TSV import window: the user pastes tab-separated rows (header = field
/// names) and applies them onto the target block's existing elements.
impl Dialog for TsvPasteState {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut do_apply = false;
        {
            let paste = &mut *self;
            egui::Window::new(format!("Paste TSV → {}", paste.block_label))
                .constrain_to(window_work_area(ctx))
                .id(egui::Id::new("tsv_paste"))
                .open(&mut open)
                .default_width(window_width(ctx, 560.0))
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
                                    .hint_text(placeholder_text("paste TSV here (Ctrl+V)")),
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
            cx.send(EditorCommand::ApplyTsvPaste);
        }
        open
    }
}
