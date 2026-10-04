//! The notice window reporting how a container operation ended.
//! It owns presentation only; the operations belong to their owning subsystems.

use super::*;

impl Baboon {
    pub(in crate::app) fn draw_operation_notice_window(&mut self, ctx: &egui::Context) {
        let Some(notice) = self.shell.operation_notice.as_ref() else {
            return;
        };
        let title = notice.title.clone();
        let mut message = notice.message.clone();
        let failed = notice.failed;
        let mut open = true;
        let mut dismiss = false;
        egui::Window::new(title)
            .id(egui::Id::new("operation_notice"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(window_width(ctx, 620.0))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if failed {
                    ui.label(
                        RichText::new("The container was left as it was; nothing was changed.")
                            .color(text_dark()),
                    );
                    ui.add_space(7.0);
                }
                // A read-only multiline edit rather than a label: the message
                // carries paths and a writer error, and it is only useful if it
                // can be selected and copied.
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut message)
                                .desired_width(f32::INFINITY)
                                .font(egui::TextStyle::Monospace)
                                .interactive(true),
                        );
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy").clicked() {
                        ui.copy_text(message.clone());
                    }
                    if ui.button("OK").clicked() {
                        dismiss = true;
                    }
                });
            });
        if dismiss || !open {
            self.shell.operation_notice = None;
        }
    }
}

/// The outcome of a container write, kept on screen until dismissed.
///
/// These operations rewrite the game's own pak and take long enough that the
/// user has looked away, so their result cannot live in the status bar: it is
/// gone before it can be read, and a failure that scrolls past is a failure that
/// gets reported as "nothing happened". The message is selectable and copyable
/// because the useful ones are too long to retype.
pub(in crate::app) struct OperationNotice {
    pub(in crate::app) title: String,
    pub(in crate::app) message: String,
    pub(in crate::app) failed: bool,
}
