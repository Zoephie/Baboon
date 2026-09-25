//! Chimp workspace frame and package browser.
//! It owns drawing the toolbar, mount status and the group, archive, folder and file browsers; documents are drawn in `document_ui`.

use super::*;

/// What a package's context menu offers.
///
/// One answer, used both to decide whether to attach a menu at all and to decide
/// what goes in it. The browser draws packages in four places, and when the two
/// decisions were written out separately at each of them they drifted: the level
/// entry was added to every menu body, but the guard on two of the sites still
/// only admitted textures and meshes, so on those the entry sat inside a menu
/// that was never built. A menu that offers nothing must not open, and anything
/// it would offer must open it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ChimpPackageActions {
    texture: bool,
    mesh: bool,
    level: bool,
}

impl ChimpPackageActions {
    fn of(package: &str, kind: Option<&str>) -> Self {
        Self {
            texture: kind == Some("Texture2D"),
            mesh: matches!(kind, Some("SkeletalMesh" | "StaticMesh")),
            level: chimp_looks_like_level(package),
        }
    }

    fn any(self) -> bool {
        self.texture || self.mesh || self.level
    }
}

impl Baboon {
    pub(in crate::app) fn draw_chimp_workspace(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
    ) {
        chimp_workspace_toolbar(ui, |ui| {
            let packages = self.chimp_dirty_packages(kit_index);
            let icon = button_icon_image(ui, ButtonIcon::Garbage, text_dark(), 16.0);
            let response = ui.add_enabled(!packages.is_empty(), egui::Button::image(icon));
            if response
                .on_hover_text(
                    "Discard every modified Chimp package in this workspace and restore the original source data",
                )
                .on_disabled_hover_text("This workspace has no modified Chimp packages")
                .clicked()
            {
                self.chimp_discard_prompt = Some(ChimpDiscardPrompt {
                    kit: self.kits[kit_index].id,
                    packages,
                    pending_action: None,
                    error: None,
                });
            }
        });
        self.draw_chimp_level_progress(ui, kit_index);
        ui.add_space(4.0);
        let ready = matches!(self.kits[kit_index].chimp.mount, ChimpMount::Ready(_));
        egui::SidePanel::left(egui::Id::new((
            "chimp_package_browser",
            self.kits[kit_index].id.0,
        )))
        .resizable(true)
        .default_width(360.0)
        .frame(
            Frame::none()
                .fill(left_panel())
                .inner_margin(egui::Margin::same(8.0)),
        )
        .show_inside(ui, |ui| {
            self.draw_chimp_browser(ui, ctx, kit_index);
        });
        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(editor_bg())
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show_inside(ui, |ui| {
                if ready {
                    match self.kits[kit_index].chimp.browser {
                        ChimpBrowser::Folders => {
                            match self.kits[kit_index].chimp.folder_selection {
                                ChimpFolderSelection::Package => {
                                    self.draw_chimp_tiles(ui, ctx, kit_index)
                                }
                                ChimpFolderSelection::File => self.draw_chimp_file(ui, kit_index),
                            }
                        }
                        ChimpBrowser::Groups => self.draw_chimp_tiles(ui, ctx, kit_index),
                        ChimpBrowser::Packages => self.draw_chimp_tiles(ui, ctx, kit_index),
                        ChimpBrowser::Archives => {
                            ui.centered_and_justified(|ui| {
                                ui.label("Select an archive to browse its folder hierarchy.");
                            });
                        }
                        ChimpBrowser::Files => self.draw_chimp_file(ui, kit_index),
                    }
                } else {
                    self.draw_chimp_mount_status(ui, kit_index);
                }
            });
    }

    /// A bar for the export running in this workspace, if one is.
    ///
    /// A level export is minutes of work, and without this the window simply
    /// sits there — the one thing a user cannot tell from a frozen-looking
    /// screen is the difference between working and stuck.
    fn draw_chimp_level_progress(&mut self, ui: &mut Ui, kit_index: usize) {
        let kit = self.kits[kit_index].id;
        let Some(job) = self.chimp_level_job.as_ref().filter(|job| job.kit == kit) else {
            return;
        };
        let phase = job.phase;
        let (done, total) = (job.done, job.total);
        let fraction = job.fraction();
        let remaining = job.remaining();
        let name = job.name.clone();

        ui.add_space(4.0);
        egui::Frame::none()
            .fill(row_type())
            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Exporting {name}"))
                            .color(text_dark())
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Only ever an estimate, and said as one.
                        if let Some(remaining) = remaining {
                            ui.label(
                                RichText::new(format!("{} left", format_remaining(remaining)))
                                    .color(subtle_dark())
                                    .small(),
                            );
                        }
                    });
                });
                ui.add_space(3.0);
                ui.add(
                    egui::ProgressBar::new(fraction).desired_height(10.0).text(
                        RichText::new(format!(
                            "{} ({}/3) — {done}/{total}",
                            phase.label(),
                            phase.step()
                        ))
                        .small(),
                    ),
                );
            });
        ui.add_space(2.0);
    }

    fn draw_chimp_mount_status(&mut self, ui: &mut Ui, kit_index: usize) {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.heading("Chimp");
            ui.add_space(8.0);
            match &self.kits[kit_index].chimp.mount {
                ChimpMount::Idle => {
                    ui.label("The Unreal package index has not been started.");
                    if ui.button("Start Chimp").clicked() {
                        self.begin_chimp_mount(kit_index, ui.ctx().clone());
                    }
                }
                ChimpMount::Loading => {
                    ui.spinner();
                    ui.label("Discovering containers and indexing Unreal packages…");
                    ui.label(
                        RichText::new(
                            "Campaign Evolved tag editing remains available while this runs.",
                        )
                        .color(subtle_dark()),
                    );
                }
                ChimpMount::Failed(error) => {
                    ui.colored_label(Color32::from_rgb(210, 80, 80), error);
                    if ui.button("Retry").clicked() {
                        self.begin_chimp_mount(kit_index, ui.ctx().clone());
                    }
                }
                ChimpMount::Ready(_) => {}
            }
        });
    }

    fn draw_chimp_browser(&mut self, ui: &mut Ui, ctx: &egui::Context, kit_index: usize) {
        ui.horizontal(|ui| {
            for (browser, label) in ChimpBrowser::TABS {
                ui.selectable_value(&mut self.kits[kit_index].chimp.browser, browser, label);
            }
            if matches!(self.kits[kit_index].chimp.mount, ChimpMount::Loading) {
                ui.spinner();
            }
        });
        ui.add_space(4.0);
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.kits[kit_index].chimp.filter)
                .hint_text(placeholder_text("Search package or container…"))
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            self.kits[kit_index].chimp.reset_filter();
        }
        ui.add_space(4.0);

        let world = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => {
                self.draw_chimp_mount_status(ui, kit_index);
                return;
            }
        };
        self.kits[kit_index].chimp.refresh_filter(&world);
        // Container diagnostics are not surfaced here. A mount routinely skips
        // archives that carry nothing Chimp reads, and reporting that above the
        // browser on every view described the mount rather than anything the
        // reader can act on. The Archives tab still lists what it could not
        // open, which is where that question is actually being asked.
        if self.kits[kit_index].chimp.browser == ChimpBrowser::Archives {
            self.draw_chimp_archives(ui, &world, kit_index);
            return;
        }
        if self.kits[kit_index].chimp.browser == ChimpBrowser::Files {
            self.draw_chimp_pak_files(ui, &world, kit_index);
            return;
        }
        if self.kits[kit_index].chimp.browser == ChimpBrowser::Folders {
            self.draw_chimp_folders(ui, ctx, &world, kit_index);
            return;
        }
        if self.kits[kit_index].chimp.browser == ChimpBrowser::Groups {
            self.draw_chimp_groups(ui, ctx, &world, kit_index);
            return;
        }
        let indices = Arc::clone(&self.kits[kit_index].chimp.filtered_packages);
        let selected = self.kits[kit_index].chimp.selected_package.clone();
        let mut extract_texture = None;
        let mut extract_mesh = None;
        let mut export_level = None;
        egui::ScrollArea::vertical()
            .id_salt(("chimp_packages", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show_rows(ui, 22.0, indices.len(), |ui, range| {
                for row in range {
                    let package = &world.packages()[indices[row]];
                    let active = package.active_provider();
                    let overridden = package.providers.len() > 1;
                    let mut label = package.name.clone();
                    if overridden {
                        label.push_str("  ⧉");
                    }
                    let response =
                        ui.selectable_label(selected.as_deref() == Some(&package.name), label);
                    let response = if let Some(provider) = active {
                        response.on_hover_text(format!(
                            "{}\n{}\n{} provider(s)",
                            package.name,
                            world.containers()[provider.container].path.display(),
                            package.providers.len()
                        ))
                    } else {
                        response
                    };
                    let actions = ChimpPackageActions::of(
                        &package.name,
                        self.kits[kit_index]
                            .chimp
                            .package_types
                            .get(indices[row])
                            .and_then(Option::as_deref),
                    );
                    if actions.any() {
                        response.context_menu(|ui| {
                            if actions.texture {
                                chimp_texture_export_menu(ui, &package.name, &mut extract_texture);
                            }
                            if actions.mesh {
                                chimp_mesh_export_menu(ui, &package.name, &mut extract_mesh);
                            }
                            if actions.level {
                                chimp_level_export_menu(ui, &package.name, &mut export_level);
                            }
                        });
                    }
                    if response.clicked() {
                        self.begin_chimp_open_package(kit_index, package.name.clone(), ctx.clone());
                    }
                }
            });
        if let Some(package) = extract_texture {
            self.begin_extract_chimp_texture(kit_index, &package);
        }
        if let Some((package, format)) = extract_mesh {
            self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
        }
        if let Some((package, format)) = export_level {
            self.begin_export_chimp_level(kit_index, &package, format);
        }
    }

    fn draw_chimp_groups(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        world: &World,
        kit_index: usize,
    ) {
        if self.kits[kit_index].chimp.type_indexing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Indexing Unreal package types…")
                        .small()
                        .color(subtle_dark()),
                );
            });
            ui.add_space(4.0);
        }

        let groups = &self.kits[kit_index].chimp.filtered_groups;
        let selected = self.kits[kit_index].chimp.selected_package.clone();
        let mut open_package = None;
        let mut extract_texture = None;
        let mut extract_mesh = None;
        let mut export_level = None;
        if groups.is_empty() && !self.kits[kit_index].chimp.type_indexing {
            ui.label(RichText::new("No matching Unreal packages.").color(subtle_dark()));
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt(("chimp_groups", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (kind, indices) in groups {
                    egui::CollapsingHeader::new(format!("{kind}  ·  {}", indices.len()))
                        .id_salt(("chimp_group", self.kits[kit_index].id.0, &kind))
                        .default_open(false)
                        .show(ui, |ui| {
                            for &index in indices {
                                let package = &world.packages()[index];
                                let label =
                                    package.name.rsplit('/').next().unwrap_or(&package.name);
                                let response = ui
                                    .selectable_label(
                                        selected.as_deref() == Some(&package.name),
                                        label,
                                    )
                                    .on_hover_text(&package.name);
                                let actions =
                                    ChimpPackageActions::of(&package.name, Some(kind.as_str()));
                                if actions.any() {
                                    response.context_menu(|ui| {
                                        if actions.texture {
                                            chimp_texture_export_menu(
                                                ui,
                                                &package.name,
                                                &mut extract_texture,
                                            );
                                        }
                                        if actions.mesh {
                                            chimp_mesh_export_menu(
                                                ui,
                                                &package.name,
                                                &mut extract_mesh,
                                            );
                                        }
                                        if actions.level {
                                            chimp_level_export_menu(
                                                ui,
                                                &package.name,
                                                &mut export_level,
                                            );
                                        }
                                    });
                                }
                                if response.clicked() {
                                    open_package = Some(package.name.clone());
                                }
                            }
                        });
                }
            });
        if let Some(package) = open_package {
            self.begin_chimp_open_package(kit_index, package, ctx.clone());
        }
        if let Some(package) = extract_texture {
            self.begin_extract_chimp_texture(kit_index, &package);
        }
        if let Some((package, format)) = extract_mesh {
            self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
        }
        if let Some((package, format)) = export_level {
            self.begin_export_chimp_level(kit_index, &package, format);
        }
    }

    fn draw_chimp_archives(&mut self, ui: &mut Ui, world: &World, kit_index: usize) {
        let selected = self.kits[kit_index].chimp.selected_archive;
        egui::ScrollArea::vertical()
            .id_salt(("chimp_archives", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if ui
                    .selectable_label(selected.is_none(), "All mounted archives")
                    .clicked()
                {
                    let chimp = &mut self.kits[kit_index].chimp;
                    chimp.selected_archive = None;
                    chimp.browser = ChimpBrowser::Folders;
                    chimp.folder_selection = ChimpFolderSelection::Package;
                    chimp.filter.clear();
                    chimp.reset_filter();
                }
                ui.add_space(4.0);
                ui.label(RichText::new("IoStore").strong());
                for container in world.containers() {
                    let name = container
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("container.utoc");
                    let response = ui
                        .selectable_label(
                            selected == Some(ChimpArchive::IoStore(container.index)),
                            format!("{name}  ·  {} packages", container.package_count),
                        )
                        .on_hover_text(format!(
                            "{}\nMount order: {}{}",
                            container.path.display(),
                            container.read_order,
                            if container.recovered_directory_index {
                                "\nRecovered directory index"
                            } else {
                                ""
                            }
                        ));
                    if response.clicked() {
                        let chimp = &mut self.kits[kit_index].chimp;
                        chimp.selected_archive = Some(ChimpArchive::IoStore(container.index));
                        chimp.browser = ChimpBrowser::Folders;
                        chimp.folder_selection = ChimpFolderSelection::Package;
                        chimp.filter.clear();
                        chimp.reset_filter();
                    }
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Legacy pak").strong());
                for container in world.pak_containers() {
                    let name = container
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("container.pak");
                    let response = ui
                        .selectable_label(
                            selected == Some(ChimpArchive::Pak(container.index)),
                            format!("{name}  ·  {} files", container.file_count),
                        )
                        .on_hover_text(format!(
                            "{}\nMount order: {}",
                            container.path.display(),
                            container.read_order
                        ));
                    if response.clicked() {
                        let chimp = &mut self.kits[kit_index].chimp;
                        chimp.selected_archive = Some(ChimpArchive::Pak(container.index));
                        chimp.browser = ChimpBrowser::Folders;
                        chimp.folder_selection = ChimpFolderSelection::File;
                        chimp.filter.clear();
                        chimp.reset_filter();
                    }
                }
                if !world.diagnostics().is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Unavailable or empty").strong());
                    for diagnostic in world.diagnostics() {
                        let name = diagnostic
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("archive");
                        ui.colored_label(Color32::from_rgb(210, 150, 70), name)
                            .on_hover_text(format!(
                                "{}\n{}",
                                diagnostic.path.display(),
                                diagnostic.message
                            ));
                    }
                }
            });
    }

    fn draw_chimp_folders(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        world: &World,
        kit_index: usize,
    ) {
        let selected_archive = self.kits[kit_index].chimp.selected_archive;
        if let Some(archive) = selected_archive {
            ui.horizontal(|ui| {
                let path = match archive {
                    ChimpArchive::IoStore(index) => &world.containers()[index].path,
                    ChimpArchive::Pak(index) => &world.pak_containers()[index].path,
                };
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("archive");
                ui.label(RichText::new(name).strong());
                if ui.small_button("Show all").clicked() {
                    let chimp = &mut self.kits[kit_index].chimp;
                    chimp.selected_archive = None;
                    chimp.reset_filter();
                }
            });
            ui.separator();
        }
        let selected_package = self.kits[kit_index].chimp.selected_package.clone();
        let selected_file = self.kits[kit_index].chimp.selected_file.clone();
        // Borrowed, like the tree beside it: this used to clone the type of
        // every mounted package (about 104k strings) every frame the default
        // Folders tab was drawn, only to satisfy the borrow checker.
        let chimp = &self.kits[kit_index].chimp;
        let clicked = egui::ScrollArea::vertical()
            .id_salt(("chimp_folders", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                draw_chimp_folder_node(
                    ui,
                    &chimp.content_tree,
                    world,
                    &chimp.package_types,
                    selected_package.as_deref(),
                    selected_file.as_deref(),
                    "",
                )
            })
            .inner;
        match clicked {
            Some(ChimpTreeClick::Package(package)) => {
                self.kits[kit_index].chimp.folder_selection = ChimpFolderSelection::Package;
                self.begin_chimp_open_package(kit_index, package, ctx.clone());
            }
            Some(ChimpTreeClick::ExtractTexture(package)) => {
                self.begin_extract_chimp_texture(kit_index, &package);
            }
            Some(ChimpTreeClick::ExportLevel(package, format)) => {
                self.begin_export_chimp_level(kit_index, &package, format);
            }
            Some(ChimpTreeClick::ExtractMesh(package, format)) => {
                self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
            }
            Some(ChimpTreeClick::File(file)) => {
                let chimp = &mut self.kits[kit_index].chimp;
                chimp.folder_selection = ChimpFolderSelection::File;
                chimp.selected_file = Some(file);
            }
            None => {}
        }
    }

    fn draw_chimp_pak_files(&mut self, ui: &mut Ui, world: &World, kit_index: usize) {
        let indices = Arc::clone(&self.kits[kit_index].chimp.filtered_files);
        let selected = self.kits[kit_index].chimp.selected_file.clone();
        egui::ScrollArea::vertical()
            .id_salt(("chimp_pak_files", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show_rows(ui, 22.0, indices.len(), |ui, range| {
                for row in range {
                    let file = &world.pak_files()[indices[row]];
                    let active = file.active_provider();
                    let mut label = file.path.clone();
                    if file.providers.len() > 1 {
                        label.push_str("  ⧉");
                    }
                    let response =
                        ui.selectable_label(selected.as_deref() == Some(&file.path), label);
                    let response = if let Some(provider) = active {
                        response.on_hover_text(format!(
                            "{}\n{}\n{} provider(s)",
                            file.path,
                            world.pak_containers()[provider.container].path.display(),
                            file.providers.len()
                        ))
                    } else {
                        response
                    };
                    if response.clicked() {
                        self.kits[kit_index].chimp.selected_file = Some(file.path.clone());
                    }
                }
            });
    }

    fn draw_chimp_file(&mut self, ui: &mut Ui, kit_index: usize) {
        let Some(path) = self.kits[kit_index].chimp.selected_file.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a file from a legacy .pak container.");
            });
            return;
        };
        let world = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => return,
        };
        let Some(file) = world.pak_file(&path) else {
            return;
        };
        let Some(provider) = file.active_provider() else {
            return;
        };
        let container = &world.pak_containers()[provider.container];
        ui.heading(&file.path);
        ui.label(
            RichText::new(format!(
                "{} • {} provider(s)",
                container.path.display(),
                file.providers.len()
            ))
            .color(subtle_dark()),
        );
        ui.add_space(8.0);
        ui.label("Legacy-pak entries are exposed as raw files.");
        ui.label(
            RichText::new(
                "Wwise banks/media and other staged data can be extracted; Unreal package property editing uses the Packages view.",
            )
            .color(subtle_dark()),
        );
        if ui.button("Extract file…").clicked() {
            let suggested = std::path::Path::new(&file.path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("extracted.bin");
            let Some(output) = rfd::FileDialog::new()
                .set_title("Extract legacy-pak file")
                .set_file_name(suggested)
                .save_file()
            else {
                return;
            };
            match world
                .read_pak_provider(provider)
                .and_then(|bytes| fs::write(&output, bytes).map_err(anyhow::Error::from))
            {
                Ok(()) => self.status = format!("Extracted {}", output.display()),
                Err(error) => {
                    self.status = format!("Could not extract {}: {error:#}", output.display())
                }
            }
        }
    }
}

fn draw_chimp_folder_node(
    ui: &mut Ui,
    node: &ChimpFolderNode,
    world: &World,
    package_types: &[Option<String>],
    selected_package: Option<&str>,
    selected_file: Option<&str>,
    parent: &str,
) -> Option<ChimpTreeClick> {
    let mut clicked = None;
    for (name, child) in &node.folders {
        let path = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}/{name}")
        };
        let child_clicked =
            egui::CollapsingHeader::new(format!("{name}  ·  {}", child.entry_count()))
                .id_salt(("chimp_folder", path.clone()))
                .show(ui, |ui| {
                    draw_chimp_folder_node(
                        ui,
                        child,
                        world,
                        package_types,
                        selected_package,
                        selected_file,
                        &path,
                    )
                })
                .body_returned
                .flatten();
        if clicked.is_none() {
            clicked = child_clicked;
        }
    }
    for leaf in &node.packages {
        let package = &world.packages()[leaf.package];
        let mut label = leaf.name.clone();
        if package.providers.len() > 1 {
            label.push_str("  ⧉");
        }
        let response = ui.selectable_label(selected_package == Some(package.name.as_str()), label);
        let response = if let Some(provider) = package.active_provider() {
            response.on_hover_text(format!(
                "{}\n{}\n{} provider(s)",
                package.name,
                world.containers()[provider.container].path.display(),
                package.providers.len()
            ))
        } else {
            response
        };
        let package_type = package_types.get(leaf.package).and_then(Option::as_deref);
        let actions = ChimpPackageActions::of(&package.name, package_type);
        if actions.any() {
            response.context_menu(|ui| {
                if actions.texture {
                    let mut request = None;
                    chimp_texture_export_menu(ui, &package.name, &mut request);
                    if let Some(name) = request {
                        clicked = Some(ChimpTreeClick::ExtractTexture(name));
                    }
                }
                if actions.mesh {
                    let mut requested = None;
                    chimp_mesh_export_menu(ui, &package.name, &mut requested);
                    if let Some((package, format)) = requested {
                        clicked = Some(ChimpTreeClick::ExtractMesh(package, format));
                    }
                }
                if actions.level {
                    let mut requested = None;
                    chimp_level_export_menu(ui, &package.name, &mut requested);
                    if let Some((package, format)) = requested {
                        clicked = Some(ChimpTreeClick::ExportLevel(package, format));
                    }
                }
            });
        }
        if response.clicked() {
            clicked = Some(ChimpTreeClick::Package(package.name.clone()));
        }
    }
    for leaf in &node.files {
        let file = &world.pak_files()[leaf.file];
        let mut label = leaf.name.clone();
        if file.providers.len() > 1 {
            label.push_str("  ⧉");
        }
        let response = ui.selectable_label(selected_file == Some(file.path.as_str()), label);
        let response = if let Some(provider) = file.active_provider() {
            response.on_hover_text(format!(
                "{}\n{}\n{} provider(s)",
                file.path,
                world.pak_containers()[provider.container].path.display(),
                file.providers.len()
            ))
        } else {
            response
        };
        if response.clicked() {
            clicked = Some(ChimpTreeClick::File(file.path.clone()));
        }
    }
    clicked
}

fn chimp_workspace_toolbar<R>(
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let row_height = ui.spacing().interact_size.y.max(24.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), row_height),
        egui::Layout::right_to_left(egui::Align::Center),
        add_contents,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chimp_workspace_toolbar_does_not_consume_the_editor_viewport() {
        let context = egui::Context::default();
        let mut toolbar_height = None;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_200.0, 800.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let toolbar = chimp_workspace_toolbar(ui, |ui| {
                        let _ = ui.button("Discard");
                    });
                    toolbar_height = Some(toolbar.response.rect.height());
                });
            },
        );

        let toolbar_height = toolbar_height.expect("the Chimp toolbar was rendered");
        assert!(
            toolbar_height <= 40.0,
            "the Chimp toolbar expanded to {toolbar_height}px"
        );
    }

    #[test]
    fn a_level_offers_a_menu_wherever_it_is_browsed() {
        // The bug this exists for: the level entry was added to every menu
        // body, but two of the four sites still only attached a menu for
        // textures and meshes - so on those the entry sat inside a menu that
        // was never built, and right-clicking a level did nothing at all.
        // A level is a `World`, which is neither.
        let level = ChimpPackageActions::of("/Game/Levels/Halo1/Solo/C10/C10", Some("World"));
        assert!(level.level);
        assert!(level.any(), "a level must open a menu");
        assert!(!level.texture && !level.mesh);
    }

    #[test]
    fn anything_the_menu_would_offer_opens_it() {
        // The invariant that keeps the guard and the contents from drifting:
        // `any()` is true exactly when at least one entry would be drawn.
        for (package, kind) in [
            ("/Game/Levels/Halo1/Solo/C10/C10", Some("World")),
            ("/Game/Meshes/SM_Rock", Some("StaticMesh")),
            ("/Game/Characters/SK_Elite", Some("SkeletalMesh")),
            ("/Game/Textures/T_Bark", Some("Texture2D")),
            (
                "/Game/Levels/Halo1/Solo/C10/_Generated_/043ATWPYEEJ",
                Some("World"),
            ),
            ("/Game/Blueprints/BP_Door", Some("Blueprint")),
            ("/Game/Misc/Thing", None),
        ] {
            let actions = ChimpPackageActions::of(package, kind);
            assert_eq!(
                actions.any(),
                actions.texture || actions.mesh || actions.level,
                "{package} would draw entries into a menu that does not open"
            );
        }
    }

    #[test]
    fn a_package_with_nothing_to_offer_opens_no_menu() {
        assert!(!ChimpPackageActions::of("/Game/Blueprints/BP_Door", Some("Blueprint")).any());
        // A cell is a World too, and there are 2,334 of them: offering to
        // export each one as a level would be noise.
        assert!(
            !ChimpPackageActions::of(
                "/Game/Levels/Halo1/Solo/C10/_Generated_/043ATWPYEEJ",
                Some("World")
            )
            .any()
        );
    }

    #[test]
    fn chimp_search_matching_is_case_insensitive_without_allocating_per_package() {
        assert!(contains_ignore_ascii_case(
            "SM_SpiritDropShip_Body",
            "spirit"
        ));
        assert!(contains_ignore_ascii_case("Texture2D", "texture2d"));
        assert!(!contains_ignore_ascii_case("StaticMesh", "skeletal"));
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_file_types_filter_and_texture_preview() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let index = index_chimp_package_types(&world);
        assert_eq!(index.package_types.len(), world.packages().len());
        for expected in ["Blueprint", "SkeletalMesh", "StaticMesh", "Texture2D"] {
            assert!(
                index.type_counts.contains_key(expected),
                "real package index should contain {expected}"
            );
        }

        let mut browser = ChimpState {
            package_types: index.package_types,
            filter: "Texture2D".to_owned(),
            ..Default::default()
        };
        browser.refresh_filter(&world);
        assert!(!browser.filtered_packages.is_empty());
        let textures = &browser.filtered_groups["Texture2D"];
        assert!(!textures.is_empty());
        assert!(
            textures
                .iter()
                .all(|index| { browser.package_types[*index].as_deref() == Some("Texture2D") })
        );

        let target = std::env::var("CE_TEXTURE_PACKAGE").ok();
        let package_index = match target.as_deref() {
            Some(target) => {
                let target = target.to_ascii_lowercase();
                textures
                    .iter()
                    .copied()
                    .find(|index| {
                        world.packages()[*index]
                            .name
                            .to_ascii_lowercase()
                            .contains(&target)
                    })
                    .unwrap_or_else(|| panic!("target Texture2D package {target:?} was not found"))
            }
            None => textures[0],
        };
        let package = world.packages()[package_index].name.clone();
        let document = load_chimp_document(&world, &package).unwrap();
        assert_eq!(document.view, ChimpDocumentView::Texture);
        let decoded = document
            .texture_previews
            .iter()
            .find_map(|preview| {
                preview
                    .preview
                    .decoded
                    .as_ref()
                    .and_then(|decoded| decoded.as_ref().ok())
            })
            .unwrap_or_else(|| panic!("{package} should decode at least one Texture2D preview"));
        assert_eq!(
            decoded.rgba.len(),
            decoded.width as usize * decoded.height as usize * 4
        );
        if package
            .to_ascii_lowercase()
            .ends_with("/t_odst_williams_default_d")
        {
            assert_eq!((decoded.width, decoded.height), (4096, 1024));
            assert!(
                decoded
                    .rgba
                    .chunks_exact(4)
                    .any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2]),
                "target virtual texture should contain decoded colour data"
            );
        }
        // A UDIM set writes one numbered file per block rather than the single
        // path chosen, so check the directory rather than that exact name.
        let directory =
            std::env::temp_dir().join(format!("baboon-chimp-texture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        write_chimp_texture(
            &world,
            &package,
            &directory.join("texture.tif"),
            ChimpTextureExport {
                format: ChimpTextureFormat::Tiff,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let written: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert!(!written.is_empty(), "Texture2D extraction wrote nothing");
        for path in &written {
            let bytes = std::fs::read(path).unwrap();
            assert!(
                bytes.starts_with(b"II*") || bytes.starts_with(b"MM\0*"),
                "{} should be a TIFF file",
                path.display()
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
