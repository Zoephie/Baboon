//! The Extract Geometry / Extract Animations target window.
//! It owns presentation and the game chosen; the extraction is
//! [`ExportCommand::Extract`].

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

/// Which game's tools a geometry or animation extraction is for. Choosing
/// one goes on to the folder picker.
impl Dialog for ExtractTargetPrompt {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let state = &mut *self;
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
            cx.send(ExportCommand::Extract {
                kind: self.kind,
                key: self.key.clone(),
                target: self.target,
            });
            return false;
        }
        open && !cancel
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
mod tests {
    //! The Extract Geometry / Animations target window: it opens on the kit's own
    //! game, a click on another game's option is what the extraction will use,
    //! and Cancel drops the request.

    use super::*;

    fn app() -> Baboon {
        Baboon::assemble(
            &egui::Context::default(),
            crate::window_state::WindowStateTracker::for_test(),
            GuiPrefs::default(),
            HashSet::new(),
            None,
            TagNameIndex::default(),
            None,
        )
    }

    fn prompt(source: Game) -> ExtractTargetPrompt {
        ExtractTargetPrompt {
            key: "file:objects/elite.render_model".to_owned(),
            display_path: "objects/elite.render_model".to_owned(),
            kind: ExtractKind::Geometry,
            source,
            target: source,
        }
    }

    /// Draw the window once; where each painted label starting with `text`
    /// landed.
    fn frame(
        app: &mut Baboon,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Rect)> {
        let output = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |_| app.dialogs.draw(&cx!(app, ctx), &app_reads!(app)),
        );
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    fn click(app: &mut Baboon, ctx: &egui::Context, labels: &[(String, egui::Rect)], text: &str) {
        let (_, rect) = labels
            .iter()
            .find(|(label, _)| label.starts_with(text))
            .unwrap_or_else(|| panic!("no `{text}` label drawn; drew {labels:?}"));
        let pos = rect.center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(app, ctx, vec![egui::Event::PointerMoved(pos), button(true)]);
        frame(app, ctx, vec![button(false)]);
    }

    #[test]
    fn the_kit_s_game_is_the_default_and_marked() {
        assert_eq!(GameId::HaloCe.generation(), Game::Halo1);
        assert_eq!(GameId::Halo2.generation(), Game::Halo2);
        for id in [
            "halo3_mcc",
            "halo3odst_mcc",
            "haloreach_mcc",
            "halo4_mcc",
            "halo2amp_mcc",
            "haloce_evolved",
        ] {
            assert_eq!(GameId::from_id(id).unwrap().generation(), Game::Halo3, "{id}");
        }
        let mut app = app();
        app.dialogs.open(prompt(Game::Halo2));
        let ctx = egui::Context::default();
        // A window lays itself out unseen on its first frame.
        frame(&mut app, &ctx, Vec::new());
        let labels = frame(&mut app, &ctx, Vec::new());
        assert!(
            labels.iter().any(|(l, _)| l == "Halo 2 (current)"),
            "{labels:?}"
        );
    }

    #[test]
    fn choosing_another_game_sets_the_target_and_cancel_drops_it() {
        let mut app = app();
        app.dialogs.open(prompt(Game::Halo2));
        let ctx = egui::Context::default();
        // A window lays itself out unseen on its first frame.
        frame(&mut app, &ctx, Vec::new());
        let labels = frame(&mut app, &ctx, Vec::new());
        click(&mut app, &ctx, &labels, "Halo: Combat Evolved");
        let state = app
            .dialogs
            .get::<ExtractTargetPrompt>()
            .expect("still open");
        assert_eq!(state.target, Game::Halo1);
        assert_eq!(
            state.source,
            Game::Halo2,
            "the source must not move with the choice"
        );

        // The Halo CE note only shows once the target leaves the source's side.
        let labels = frame(&mut app, &ctx, Vec::new());
        assert!(
            labels
                .iter()
                .any(|(l, _)| l.starts_with("Halo CE reads a permutation")),
            "{labels:?}"
        );

        click(&mut app, &ctx, &labels, "Cancel");
        assert!(app.dialogs.get::<ExtractTargetPrompt>().is_none());
    }

    /// Choosing a folder hands the extraction on as a command and closes the
    /// window; the folder picker opens when the frame applies it.
    #[test]
    fn choose_folder_sends_the_extraction_and_closes_the_window() {
        let ctx = egui::Context::default();
        let mut app = Baboon::for_test();
        app.dialogs.open(prompt(Game::Halo3));
        let mut labels = frame(&mut app, &ctx, Vec::new());
        for _ in 0..3 {
            labels = frame(&mut app, &ctx, Vec::new());
        }
        assert_eq!(app.commands.len(), 0);
        click(&mut app, &ctx, &labels, "Choose Folder…");
        assert!(app.dialogs.get::<ExtractTargetPrompt>().is_none());
        assert_eq!(app.commands.len(), 1);
    }
}
