//! The keyword chooser window.
//! It owns presentation and the keyword picked; showing the matching tags belongs to the browser.

use super::*;

impl Baboon {
    pub(in crate::app::ui) fn draw_keyword_chooser_window(&mut self, ctx: &egui::Context) {
        if !self.keyword_chooser_open {
            return;
        }
        let mut open = true;
        let mut chosen: Option<String> = None;
        let all = self.kits[self.active].keywords.all_keywords();
        egui::Window::new("Keywords")
            .id(egui::Id::new("keyword_chooser"))
            .open(&mut open)
            .default_width(280.0)
            .show(ctx, |ui| {
                if all.is_empty() {
                    ui.label(
                        RichText::new("No keywords yet — add them on a tag's Keywords bar.")
                            .color(subtle_dark()),
                    );
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (keyword, count) in &all {
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(format!("{keyword}  ({count})"))
                                            .color(text_dark()),
                                    )
                                    .sense(Sense::click()),
                                )
                                .on_hover_text("Show tags with this keyword")
                                .clicked()
                            {
                                chosen = Some(keyword.clone());
                            }
                        }
                    });
            });
        if let Some(keyword) = chosen {
            self.show_tags_with_keyword(&keyword);
        }
        self.keyword_chooser_open = open;
    }
}
