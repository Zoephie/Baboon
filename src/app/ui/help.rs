//! About, documentation, and map-name help window.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

impl Baboon {
    pub(super) fn draw_about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }

        let mut open = self.about_open;
        egui::Window::new("Baboon Help")
            .id(egui::Id::new("baboon_help"))
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .default_width(780.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.help_panel_tab, HelpPanelTab::About, "About");
                    ui.selectable_value(&mut self.help_panel_tab, HelpPanelTab::Doc, "Doc");
                    ui.selectable_value(
                        &mut self.help_panel_tab,
                        HelpPanelTab::MapNames,
                        "Map Names",
                    );
                });
                ui.separator();
                ui.add_space(8.0);
                match self.help_panel_tab {
                    HelpPanelTab::About => draw_about_tab(ui),
                    HelpPanelTab::Doc => draw_doc_tab(ui, &self.help_docs),
                    HelpPanelTab::MapNames => draw_map_names_tab(ui, &mut self.map_names_game_tab),
                }
            });
        self.about_open = open;
    }
}

fn draw_about_tab(ui: &mut Ui) {
    ui.heading(RichText::new("Baboon").color(text_dark()));
    ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(subtle_dark()));
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("blam-tags created by").color(text_dark()));
        ui.label(
            RichText::new("Camden Smallwood")
                .color(foundation_blue())
                .strong(),
        );
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Baboon created by").color(text_dark()));
        ui.label(
            RichText::new("Zoephie Sinyard")
                .color(foundation_blue())
                .strong(),
        );
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Icons by").color(text_dark()));
        ui.label(RichText::new("Paddy Tee").color(foundation_blue()).strong());
    });
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(8.0);
    ui.label(RichText::new("Source").color(text_dark()).strong());
    ui.hyperlink_to(BABOON_GITHUB_URL, BABOON_GITHUB_URL);
}

fn draw_doc_tab(ui: &mut Ui, docs: &HelpDocsState) {
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match docs {
            HelpDocsState::Loaded(docs) => {
                if let Some(tab) = docs.tab("doc") {
                    for section in &tab.sections {
                        doc_section(ui, section);
                    }
                } else {
                    doc_load_error(ui, "Documentation failed to load: missing doc tab.");
                }
            }
            HelpDocsState::Failed(error) => {
                doc_load_error(ui, &format!("Documentation failed to load: {error}"));
            }
        });
}

fn doc_section(ui: &mut Ui, section: &HelpDocSection) {
    ui.label(
        RichText::new(&section.title)
            .color(foundation_blue())
            .font(FontId::proportional(14.0))
            .strong(),
    );
    ui.add_space(4.0);
    for block in &section.blocks {
        match block {
            HelpDocBlock::Paragraph { text } => {
                ui.add(
                    egui::Label::new(RichText::new(text).color(text_dark()))
                        .wrap()
                        .selectable(false),
                );
                ui.add_space(4.0);
            }
            HelpDocBlock::Bullets { items } => {
                for item in items {
                    doc_bullet(ui, item);
                }
            }
        }
    }
    ui.add_space(12.0);
}

fn doc_bullet(ui: &mut Ui, line: &str) {
    ui.horizontal_top(|ui| {
        ui.label(RichText::new("-").color(subtle_dark()));
        ui.add(
            egui::Label::new(RichText::new(line).color(text_dark()))
                .wrap()
                .selectable(false),
        );
    });
}

fn doc_load_error(ui: &mut Ui, message: &str) {
    ui.add(
        egui::Label::new(RichText::new(message).color(text_dark()))
            .wrap()
            .selectable(false),
    );
}
