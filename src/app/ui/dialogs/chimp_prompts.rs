//! The Chimp export prompts: mesh textures, texture export, and level export.
//! It owns presentation and the options chosen; the exports themselves belong to the controller and `chimp`.

use super::*;

impl Baboon {
    pub(in crate::app::ui) fn draw_chimp_mesh_texture_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.chimp_mesh_texture_prompt.as_ref() else {
            return;
        };
        let package = prompt.package.clone();
        let format_label = prompt.format_label();
        let mesh_path = prompt.path.display().to_string();
        let texture_directory = prompt.texture_directory().display().to_string();
        let subject = prompt.texture_subject();
        let mut texture_export = prompt.texture_export;
        let mut open = true;
        let mut with_textures: Option<ChimpTextureScope> = None;
        let mut cancel = false;
        egui::Window::new("Export textures with this mesh?")
            .id(egui::Id::new("chimp_mesh_texture_prompt"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(600.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("{package} as {format_label}")).color(text_dark()));
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&mesh_path)
                        .color(subtle_dark())
                        .monospace()
                        .small(),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(
                        "Baboon can also export the textures this mesh's materials reference:",
                    )
                    .color(text_dark()),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&texture_directory)
                        .color(subtle_dark())
                        .monospace()
                        .small(),
                );
                ui.add_space(6.0);
                // A material graph is far larger than it looks: exporting one
                // weapon pulled 49 textures, most of them shared inputs the
                // whole game uses. Said plainly and up front, because the cost
                // only becomes obvious once the folder is full.
                ui.label(
                    RichText::new(
                        "Exporting all textures can produce far more than you expect. A \
                         material graph reaches every shared input it touches — master \
                         materials, detail maps, lookup tables — so a single weapon can run to \
                         40–50 files, and decoding them all takes a while.",
                    )
                    .color(egui::Color32::from_rgb(210, 120, 90)),
                );
                if let Some(subject) = &subject {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "This model's textures are the ones named after {subject} — usually \
                             its diffuse, normal, ORM, emissive and decals."
                        ))
                        .color(subtle_dark())
                        .small(),
                    );
                }
                ui.add_space(12.0);
                // The same choice the single-texture export asks for, offered
                // here rather than as a second window: it is one more decision
                // about the same export, and it only applies to the two buttons
                // below that ask for textures at all.
                ui.label(RichText::new("Image format for those textures").color(text_dark()));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    for choice in ChimpTextureFormat::ALL {
                        ui.radio_value(&mut texture_export.format, choice, choice.label())
                            .on_hover_text(choice.summary());
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(texture_export.format.summary())
                        .color(subtle_dark())
                        .small(),
                );
                ui.add_space(6.0);
                ui.checkbox(
                    &mut texture_export.split_udim,
                    "Split UDIM blocks into separate files",
                )
                .on_hover_text(
                    "Off writes each UDIM texture as one stitched image instead, for an \
                     engine with no UDIM support",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if let Some(subject) = &subject
                        && ui
                            .button("Mesh and this model's textures")
                            .on_hover_text(format!("Only textures whose name contains {subject}"))
                            .clicked()
                    {
                        with_textures = Some(ChimpTextureScope::Matching);
                    }
                    if ui
                        .button("Mesh and all textures")
                        .on_hover_text(
                            "Every texture the materials reference, shared ones included",
                        )
                        .clicked()
                    {
                        with_textures = Some(ChimpTextureScope::All);
                    }
                    if ui.button("Mesh only").clicked() {
                        with_textures = Some(ChimpTextureScope::None);
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if let Some(prompt) = self.chimp_mesh_texture_prompt.as_mut() {
            prompt.texture_export = texture_export;
        }
        if let Some(with_textures) = with_textures {
            if let Some(prompt) = self.chimp_mesh_texture_prompt.take() {
                self.start_chimp_mesh_export(prompt, with_textures, ctx.clone());
            }
        } else if cancel || !open {
            self.chimp_mesh_texture_prompt = None;
        }
    }

    /// Ask how a level should be split before exporting it.
    ///
    /// The splitting is the part worth explaining. A whole level is more than an
    /// importer opens, so the export is a folder of regions rather than a file,
    /// and every piece of that — why it splits, what the numbers mean, that the
    /// shared library has to travel with the segments — is invisible from the
    /// files alone.
    pub(in crate::app::ui) fn draw_chimp_texture_export_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.chimp_texture_export_prompt.as_ref() else {
            return;
        };
        let name = prompt.name().to_owned();
        let mut export = prompt.export;
        let mut open = true;
        let mut go = false;
        let mut cancel = false;

        egui::Window::new("Extract texture")
            .id(egui::Id::new("chimp_texture_export_prompt"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new(&name).color(text_dark()));
                ui.add_space(10.0);
                for choice in ChimpTextureFormat::ALL {
                    ui.radio_value(&mut export.format, choice, choice.label());
                    ui.indent(choice.label(), |ui| {
                        ui.label(RichText::new(choice.summary()).color(subtle_dark()).small());
                    });
                    ui.add_space(6.0);
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                ui.checkbox(
                    &mut export.split_udim,
                    "Split UDIM blocks into separate files",
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if export.split_udim {
                        "A virtual texture with more than one UDIM block writes one numbered \
                         file per block beside the name you choose, each at the resolution it \
                         was authored at."
                    } else {
                        "The whole set is written as the single stitched image its tiles were \
                         reassembled into — for an engine with no UDIM support. Blocks \
                         authored smaller are magnified to sit on the same grid, so this \
                         costs some detail that splitting keeps."
                    })
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button("Choose file and export…").clicked() {
                        go = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if let Some(prompt) = self.chimp_texture_export_prompt.as_mut() {
            prompt.export = export;
        }
        if go {
            if let Some(prompt) = self.chimp_texture_export_prompt.take() {
                self.start_chimp_texture_export(prompt, ctx.clone());
            }
        } else if cancel || !open {
            self.chimp_texture_export_prompt = None;
        }
    }

    pub(in crate::app::ui) fn draw_chimp_level_export_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.chimp_level_export_prompt.as_ref() else {
            return;
        };
        let package = prompt.package.clone();
        let name = prompt.name();
        let format_label = prompt.format_label();
        let format_summary = prompt.format_summary();
        let cells = prompt.cells.len();
        let usd = matches!(prompt.format, ChimpLevelFormat::SegmentedUsd);
        let (mut nanite, mut split) = (prompt.nanite, prompt.split);
        let (mut triangles, mut placements) = (prompt.triangles, prompt.placements);
        let mut open = true;
        let mut go = false;
        let mut cancel = false;

        egui::Window::new("Export level")
            .id(egui::Id::new("chimp_level_export_prompt"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(640.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("{package} as {format_label}")).color(text_dark()));
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!("{cells} World Partition cells"))
                        .color(subtle_dark())
                        .small(),
                );
                ui.add_space(8.0);
                ui.label(RichText::new(format_summary).color(text_dark()));
                ui.add_space(12.0);

                ui.checkbox(&mut nanite, "Full Nanite geometry");
                ui.label(
                    RichText::new(
                        "Off exports the cooked fallback instead - a coarse proxy built for \
                         hardware that cannot run Nanite. Far smaller, and visibly rougher.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(10.0);

                ui.checkbox(&mut split, "Split into segments");
                ui.add_space(4.0);
                if split {
                    ui.label(
                        RichText::new(
                            "The level is cut in half at the middle of its widest axis, and \
                             each half again, until every piece fits the limits below. \
                             Segments are regions of the map, not arbitrary slices, and they \
                             share a coordinate system - importing two lines them up exactly. \
                             Dense areas produce more, smaller segments than open ones.",
                        )
                        .color(text_dark()),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut triangles)
                                .speed(250_000.0)
                                .range(100_000..=usize::MAX),
                        );
                        ui.label(
                            RichText::new("triangles, across the distinct meshes a segment uses")
                                .color(subtle_dark())
                                .small(),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut placements)
                                .speed(1_000.0)
                                .range(100..=usize::MAX),
                        );
                        ui.label(RichText::new("placements").color(subtle_dark()).small());
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "Both limits matter, because geometry and object count run out \
                             separately: a stand of foliage can place tens of thousands of \
                             copies of a handful of meshes, and a triangle budget would not \
                             see it.",
                        )
                        .color(subtle_dark())
                        .small(),
                    );
                } else if usd {
                    // Measured, not cautious: this is what the whole level did.
                    ui.label(
                        RichText::new(
                            "One file for the whole level. Campaign Evolved's C10 comes to \
                             8.6 GB and 296,399 placements this way, which Blender does not \
                             open - leave splitting on unless the level is a small one.",
                        )
                        .color(text_dark()),
                    );
                } else {
                    ui.label(
                        RichText::new(
                            "One master placing the whole level. The master holds placements \
                             and links, not geometry, so it stays small - C10 is 20 MB across \
                             296,399 placements - but opening it loads every mesh it uses.",
                        )
                        .color(text_dark()),
                    );
                }

                ui.add_space(12.0);
                ui.label(
                    RichText::new(if usd {
                        "Writes a prototype library holding every mesh once, and a file per \
                         segment that references it. The library has to stay beside the \
                         segments - a segment on its own imports as nothing."
                    } else {
                        "Writes the geometry and a build_blend.py beside it. Baboon cannot \
                         write .blend files, so Blender builds them: open the script and run \
                         it, or blender --background --python build_blend.py."
                    })
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("Choose folder and export…")
                        .on_hover_text(
                            "Reading a level takes minutes; progress shows in the status bar",
                        )
                        .clicked()
                    {
                        go = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if let Some(prompt) = self.chimp_level_export_prompt.as_mut() {
            prompt.nanite = nanite;
            prompt.split = split;
            prompt.triangles = triangles;
            prompt.placements = placements;
        }
        if go {
            if let Some(prompt) = self.chimp_level_export_prompt.take() {
                self.start_chimp_level_export(prompt, ctx.clone());
            }
        } else if cancel || !open {
            self.chimp_level_export_prompt = None;
        }
        let _ = name;
    }
}
