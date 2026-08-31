//! Blam! workspace surface: asset folder, per-pipeline tick boxes, and the import request.
//! It owns immediate-mode presentation and request collection; folder detection lives in `app/blam.rs` and the pipelines themselves will live in `blam-tags`.

use super::*;

/// Import stays legible as a filled button in both themes: dark green fill,
/// white text.
fn blam_import_fill() -> Color32 {
    Color32::from_rgb(40, 122, 62)
}

impl Baboon {
    /// The Blam! surface only covers Halo 3 pipelines for now, so both the
    /// Tools menu entry and the surface strip answer to the kit's game.
    pub(super) fn active_kit_is_halo3(&self) -> bool {
        self.kit_is_halo3(self.active)
    }

    pub(super) fn kit_is_halo3(&self, kit_index: usize) -> bool {
        self.kits[kit_index]
            .source
            .as_ref()
            .is_some_and(|source| source.game.as_deref() == Some("halo3_mcc"))
    }

    /// The Blam! pane: a [`BLAM_KEY`] tile in the kit's tag tree, so it drags,
    /// splits, and resizes like any open tag.
    pub(super) fn draw_blam_pane(&mut self, ui: &mut Ui, kit_index: usize) {
        let kit_id = self.kits[kit_index].id;
        let data_root = self
            .editing_kit_root_for(kit_index)
            .map(|root| root.join("data"));

        // Re-detect when the asset path changes (or a rescan was forced), not
        // every frame — the ticks follow the typed path without hammering disk.
        let trimmed = self.kits[kit_index].blam.asset_path.trim().to_owned();
        if self.kits[kit_index].blam.scanned_path.as_deref() != Some(trimmed.as_str()) {
            let blam = &mut self.kits[kit_index].blam;
            if let Some(data_root) = data_root.as_ref().filter(|_| !trimmed.is_empty()) {
                let asset_folder = data_root.join(trimmed.replace('\\', "/"));
                blam.rescan(&asset_folder);
            } else {
                blam.scan = BlamFolderScan::default();
                blam.import_render = false;
                blam.import_collision = false;
                blam.import_physics = false;
                blam.import_structure = false;
                blam.scanned_path = Some(trimmed.clone());
            }
        }

        egui::TopBottomPanel::bottom(egui::Id::new(("blam_status_bar", kit_id.0)))
            .frame(Frame::none().fill(menu_bar()).inner_margin(egui::Margin {
                left: 10.0,
                right: 10.0,
                top: 4.0,
                bottom: 4.0,
            }))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Status:").color(subtle_dark()));
                    ui.label(
                        RichText::new(&self.kits[kit_index].blam.status)
                            .color(text_dark())
                            .monospace(),
                    );
                });
            });

        let mut browse_clicked = false;
        let mut rescan_clicked = false;
        let mut import_clicked = false;
        egui::CentralPanel::default()
            .frame(Frame::none().fill(editor_bg()).inner_margin(egui::Margin {
                left: 14.0,
                right: 14.0,
                top: 10.0,
                bottom: 10.0,
            }))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("blam_pane", kit_id.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.heading(RichText::new("Blam!").color(text_dark()));
                        ui.label(
                            RichText::new(
                                "Runs Baboon's own import pipelines over the asset's tool source \
                         folders, without the kit's tool.exe.",
                            )
                            .color(subtle_dark()),
                        );
                        ui.label(
                            RichText::new(
                                "Preview: the pipelines themselves have not landed yet, so the \
                                 Import button does not import anything in this build.",
                            )
                            .color(ui.visuals().warn_fg_color)
                            .strong(),
                        );
                        ui.add_space(10.0);

                        ui.label(
                            RichText::new("Asset data folder")
                                .color(text_dark())
                                .strong(),
                        );
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(
                                    &mut self.kits[kit_index].blam.asset_path,
                                )
                                .desired_width(320.0)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("objects\\weapons\\rifle\\assault_rifle"),
                            );
                            if ui.small_button("...").clicked() {
                                browse_clicked = true;
                            }
                            if ui
                                .small_button("⟳")
                                .on_hover_text("Re-detect the source folders")
                                .clicked()
                            {
                                rescan_clicked = true;
                            }
                        });
                        ui.label(
                            RichText::new(
                                "Relative to the kit's data folder, like a tool command path.",
                            )
                            .color(subtle_dark()),
                        );
                        ui.add_space(12.0);

                        ui.label(RichText::new("Pipelines").color(text_dark()).strong());
                        let scan = self.kits[kit_index].blam.scan;
                        let blam = &mut self.kits[kit_index].blam;
                        ui.add_enabled(
                            scan.render,
                            egui::Checkbox::new(&mut blam.import_render, "Render (render_model)"),
                        )
                        .on_disabled_hover_text("No render folder in this asset's data folder");
                        ui.indent("blam_prt_indent", |ui| {
                            ui.add_enabled(
                                scan.render && blam.import_render,
                                egui::Checkbox::new(&mut blam.import_prt, "Calculate PRT data"),
                            )
                            .on_disabled_hover_text("PRT is calculated as part of a render import");
                        });
                        ui.add_enabled(
                            scan.collision,
                            egui::Checkbox::new(
                                &mut blam.import_collision,
                                "Collision (collision_model)",
                            ),
                        )
                        .on_disabled_hover_text("No collision folder in this asset's data folder");
                        ui.add_enabled(
                            scan.physics,
                            egui::Checkbox::new(
                                &mut blam.import_physics,
                                "Physics (physics_model)",
                            ),
                        )
                        .on_disabled_hover_text("No physics folder in this asset's data folder");
                        ui.add_enabled(
                            scan.structure,
                            egui::Checkbox::new(
                                &mut blam.import_structure,
                                "Structure (structure_bsp)",
                            ),
                        )
                        .on_disabled_hover_text("No structure folder in this asset's data folder");
                        ui.add_space(14.0);

                        let importable = !trimmed.is_empty() && blam.anything_selected();
                        if ui
                            .add_enabled(
                                importable,
                                egui::Button::new(
                                    RichText::new("Import")
                                        .color(Color32::WHITE)
                                        .strong()
                                        .size(16.0),
                                )
                                .fill(blam_import_fill())
                                .min_size(Vec2::new(340.0, 38.0)),
                            )
                            .on_disabled_hover_text(
                                "Pick an asset folder with at least one source folder ticked",
                            )
                            .clicked()
                        {
                            import_clicked = true;
                        }
                    });
            });
        if browse_clicked
            && let Some(path) = self.pick_tool_command_path(ToolCommandArgKind::PathData)
        {
            let blam = &mut self.kits[kit_index].blam;
            blam.asset_path = path;
            blam.scanned_path = None;
        }
        if rescan_clicked {
            self.kits[kit_index].blam.scanned_path = None;
        }
        if import_clicked {
            self.request_blam_import(kit_index);
        }
    }

    /// Collects the ticked pipelines into a status line. This is where the
    /// blam-tags import pipeline gets called once it lands; until then the
    /// surface's status bar just answers what would run.
    fn request_blam_import(&mut self, kit_index: usize) {
        let blam = &mut self.kits[kit_index].blam;
        let mut pipelines = Vec::new();
        if blam.import_render {
            pipelines.push(if blam.import_prt {
                "render + PRT"
            } else {
                "render"
            });
        }
        if blam.import_collision {
            pipelines.push("collision");
        }
        if blam.import_physics {
            pipelines.push("physics");
        }
        if blam.import_structure {
            pipelines.push("structure");
        }
        blam.status = format!(
            "Import requested ({}) — the blam-tags pipeline is not hooked up yet.",
            pipelines.join(", ")
        );
    }
}
