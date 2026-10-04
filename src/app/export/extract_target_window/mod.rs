//! The Extract Geometry / Extract Animations target window.
//! It owns presentation and the game chosen; the extraction belongs to the controller.

use super::*;

use blam_tags::game::Game;

/// The targets offered, as (generation, name, the formats it gets).
const TARGETS: [(Game, &str, &str); 3] = [
    (
        Game::Halo1,
        "Halo: Combat Evolved",
        "JMS 8200, one file per permutation · JMA 16392",
    ),
    (Game::Halo2, "Halo 2", "JMS 8210 · ASS 2 · JMA 16394"),
    (Game::Halo3, "Halo 3", "JMS 8213 · ASS 7 · JMA 16394"),
];

impl Baboon {
    /// Which game's tools a geometry or animation extraction is for. Choosing
    /// one goes on to the folder picker.
    pub(in crate::app) fn draw_extract_target_window(&mut self, ctx: &egui::Context) {
        let Some(state) = self.export.extract_target.as_mut() else {
            return;
        };
        let title = match state.kind {
            ExtractKind::Geometry => "Extract Geometry",
            ExtractKind::Animation => "Extract Animations",
        };
        let mut open = true;
        let mut extract = false;
        let mut cancel = false;
        egui::Window::new(title)
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("extract_target"))
            .open(&mut open)
            .default_width(window_width(ctx, 520.0))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(native_display_path(&state.display_path))
                        .color(text_dark())
                        .monospace(),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Import with the tools for")
                        .color(subtle_dark())
                        .small(),
                );
                for (game, name, formats) in TARGETS {
                    let label = if game == state.source {
                        format!("{name} (current)")
                    } else {
                        name.to_owned()
                    };
                    ui.radio_value(
                        &mut state.target,
                        game,
                        RichText::new(label).color(text_dark()),
                    );
                    ui.indent(("extract_target_formats", name), |ui| {
                        ui.label(RichText::new(formats).color(subtle_dark()).small());
                    });
                }
                if state.kind == ExtractKind::Geometry && state.target != state.source {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(extract_target_geometry_note(state.source, state.target))
                            .color(subtle_dark())
                            .small(),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Choose Folder…").clicked() {
                        extract = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if extract {
            let state = self.export.extract_target.take().expect("checked above");
            match state.kind {
                ExtractKind::Geometry => {
                    self.begin_extract_geometry(state.key, state.target, ctx.clone())
                }
                ExtractKind::Animation => {
                    self.begin_extract_animation(state.key, state.target, ctx.clone())
                }
            }
        } else if cancel || !open {
            self.export.extract_target = None;
        }
    }
}

/// What changes about geometry exported for another generation's tools.
pub(in crate::app) fn extract_target_geometry_note(source: Game, target: Game) -> &'static str {
    match (source, target) {
        (_, Game::Halo1) => {
            "Halo CE reads a permutation from its file name, so models are split into one JMS \
             per permutation (render in models/, collision in physics/), vertices keep their \
             two heaviest bone weights, and physics models are left out. Level, BSP and \
             particle geometry keep this game's format."
        }
        (Game::Halo1, _) => {
            "Each permutation's file is merged into one JMS whose materials name their \
             permutation and region. Level geometry keeps this game's format."
        }
        _ => "Level and particle geometry keep this game's format.",
    }
}

#[cfg(test)]
mod tests;
