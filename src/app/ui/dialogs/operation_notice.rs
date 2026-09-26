//! The notice window reporting how a container operation ended.
//! It owns presentation only; the operations belong to their owning subsystems.

use super::*;

impl Baboon {
    pub(in crate::app::ui) fn draw_operation_notice_window(&mut self, ctx: &egui::Context) {
        let Some(notice) = self.operation_notice.as_ref() else {
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
            .default_width(620.0)
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
                        ui.output_mut(|out| out.copied_text = message.clone());
                    }
                    if ui.button("OK").clicked() {
                        dismiss = true;
                    }
                });
            });
        if dismiss || !open {
            self.operation_notice = None;
        }
    }
}
