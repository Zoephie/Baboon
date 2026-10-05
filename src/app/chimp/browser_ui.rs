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

/// Draw a kit's Chimp surface: its toolbar, the package browser, and the
/// open packages or the mount's status.
pub(in crate::app) fn draw_chimp_workspace(
    ui: &mut Ui,
    cx: &Ctx,
    chimp: &mut ChimpFeature,
    view: &mut ChimpView,
    kit_index: usize,
) {
    let kit = cx.model.kits[kit_index].id;
    chimp_workspace_toolbar(ui, |ui| {
        let packages = cx.model.chimp_dirty_packages(kit_index);
        let icon = button_icon_image(ui, ButtonIcon::Garbage, text_dark(), 16.0);
        let response = ui.add_enabled(!packages.is_empty(), egui::Button::image(icon));
        if response
            .on_hover_text(
                "Discard every modified Chimp package in this workspace and restore the original source data",
            )
            .on_disabled_hover_text("This workspace has no modified Chimp packages")
            .clicked()
        {
            cx.open_dialog(ChimpDiscardPrompt {
                kit,
                packages,
                pending_action: None,
                error: None,
            });
        }
    });
    draw_chimp_level_progress(ui, chimp, kit);
    ui.add_space(4.0);
    let ready = matches!(cx.model.kits[kit_index].chimp.mount, ChimpMount::Ready(_));
    egui::Panel::left(egui::Id::new(("chimp_package_browser", kit.0)))
        .resizable(true)
        .default_size(360.0)
        .frame(
            Frame::NONE
                .fill(left_panel())
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ui, |ui| {
            draw_chimp_browser(ui, cx, view, kit_index);
        });
    egui::CentralPanel::default()
        .frame(
            Frame::NONE
                .fill(editor_bg())
                .inner_margin(egui::Margin::same(10)),
        )
        .show(ui, |ui| {
            let writing = chimp.chimp_writes.contains_key(&kit);
            if ready {
                match view.browser {
                    ChimpBrowser::Folders => match view.folder_selection {
                        ChimpFolderSelection::Package => {
                            draw_chimp_tiles(ui, cx, view, kit_index, writing)
                        }
                        ChimpFolderSelection::File => draw_chimp_file(ui, cx, view, kit_index),
                    },
                    ChimpBrowser::Groups => draw_chimp_tiles(ui, cx, view, kit_index, writing),
                    ChimpBrowser::Packages => draw_chimp_tiles(ui, cx, view, kit_index, writing),
                    ChimpBrowser::Archives => {
                        crate::app::shell::frame::centered_empty_state(
                            ui,
                            "Select an archive to browse its folder hierarchy.",
                        );
                    }
                    ChimpBrowser::Files => draw_chimp_file(ui, cx, view, kit_index),
                }
            } else {
                draw_chimp_mount_status(ui, cx, kit_index);
            }
        });
}

/// A bar for the export running in this workspace, if one is.
///
/// A level export is minutes of work, and without this the window simply
/// sits there — the one thing a user cannot tell from a frozen-looking
/// screen is the difference between working and stuck.
fn draw_chimp_level_progress(ui: &mut Ui, chimp: &ChimpFeature, kit: KitId) {
    let Some(job) = chimp.chimp_level_job.as_ref().filter(|job| job.kit == kit) else {
        return;
    };
    let phase = job.phase;
    let (done, total) = (job.done, job.total);
    let fraction = job.fraction();
    let remaining = job.remaining();
    let name = job.name.clone();

    ui.add_space(4.0);
    egui::Frame::NONE
        .fill(row_type())
        .inner_margin(egui::Margin::symmetric(8, 6))
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

fn draw_chimp_mount_status(ui: &mut Ui, cx: &Ctx, kit_index: usize) {
    let kit = cx.model.kits[kit_index].id;
    if matches!(cx.model.kits[kit_index].chimp.mount, ChimpMount::Loading) {
        crate::app::shell::loading::centered_loading_state(
            ui,
            "Please wait — Chimp is starting up…",
            "Discovering containers and indexing Unreal packages.",
        );
        return;
    }
    ui.vertical_centered(|ui| {
        ui.add_space(48.0);
        ui.heading("Chimp");
        ui.add_space(8.0);
        match &cx.model.kits[kit_index].chimp.mount {
            ChimpMount::Idle => {
                ui.label("The Unreal package index has not been started.");
                if ui.button("Start Chimp").clicked() {
                    cx.send(ChimpCommand::Mount { kit });
                }
            }
            ChimpMount::Loading => {}
            ChimpMount::Failed(error) => {
                ui.colored_label(Color32::from_rgb(210, 80, 80), error);
                if ui.button("Retry").clicked() {
                    cx.send(ChimpCommand::Mount { kit });
                }
            }
            ChimpMount::Ready(_) => {}
        }
    });
}

fn draw_chimp_browser(ui: &mut Ui, cx: &Ctx, view: &mut ChimpView, kit_index: usize) {
    let kit = cx.model.kits[kit_index].id;
    if matches!(cx.model.kits[kit_index].chimp.mount, ChimpMount::Loading) {
        // Allocate the whole browser body so an otherwise empty loading
        // state cannot collapse the resizable side panel around its icon.
        let available = ui.available_size();
        let (container, _) = ui.allocate_exact_size(available, Sense::hover());
        let spinner_size = 128.0_f32.min(container.width()).min(container.height());
        let top_padding = 48.0_f32.min((container.height() - spinner_size).max(0.0));
        let spinner_rect = egui::Rect::from_min_size(
            egui::pos2(
                container.center().x - spinner_size * 0.5,
                container.top() + top_padding,
            ),
            Vec2::splat(spinner_size),
        );
        crate::app::shell::loading::paint_loading_rings_sized(ui, spinner_rect, spinner_size);
        return;
    }
    ui.horizontal(|ui| {
        for (browser, label) in ChimpBrowser::TABS {
            ui.selectable_value(&mut view.browser, browser, label);
        }
    });
    ui.add_space(4.0);
    let response = ui.add(
        egui::TextEdit::singleline(&mut view.filter)
            .hint_text(placeholder_text("Search package or container…"))
            .desired_width(f32::INFINITY),
    );
    if response.changed() {
        view.reset_filter();
    }
    ui.add_space(4.0);

    let ChimpMount::Ready(world) = &cx.model.kits[kit_index].chimp.mount else {
        draw_chimp_mount_status(ui, cx, kit_index);
        return;
    };
    view.refresh_filter(world);
    // Container diagnostics are not surfaced here. A mount routinely skips
    // archives that carry nothing Chimp reads, and reporting that above the
    // browser on every view described the mount rather than anything the
    // reader can act on. The Archives tab still lists what it could not
    // open, which is where that question is actually being asked.
    if view.browser == ChimpBrowser::Archives {
        draw_chimp_archives(ui, view, world, kit);
        return;
    }
    if view.browser == ChimpBrowser::Files {
        draw_chimp_pak_files(ui, view, world, kit);
        return;
    }
    if view.browser == ChimpBrowser::Folders {
        draw_chimp_folders(ui, cx, view, world, kit_index);
        return;
    }
    if view.browser == ChimpBrowser::Groups {
        draw_chimp_groups(ui, cx, view, world, kit_index);
        return;
    }
    let indices = Arc::clone(&view.filtered_packages);
    let selected = cx.model.kits[kit_index].chimp.selected_package.clone();
    let mut extract_texture = None;
    let mut extract_mesh = None;
    let mut export_level = None;
    egui::ScrollArea::vertical()
        .id_salt(("chimp_packages", kit.0))
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
                    view.package_types
                        .get(indices[row])
                        .and_then(Option::as_deref),
                );
                if actions.any() {
                    context_menu(&response, |ui| {
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
                    cx.send(ChimpCommand::Open {
                        kit,
                        package: package.name.clone(),
                    });
                }
            }
        });
    send_chimp_extractions(cx, kit, extract_texture, extract_mesh, export_level);
}

fn draw_chimp_groups(ui: &mut Ui, cx: &Ctx, view: &ChimpView, world: &World, kit_index: usize) {
    let kit = cx.model.kits[kit_index].id;
    if view.type_indexing {
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

    let groups = &view.filtered_groups;
    let selected = cx.model.kits[kit_index].chimp.selected_package.clone();
    let mut open_package = None;
    let mut extract_texture = None;
    let mut extract_mesh = None;
    let mut export_level = None;
    if groups.is_empty() && !view.type_indexing {
        ui.label(RichText::new("No matching Unreal packages.").color(subtle_dark()));
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt(("chimp_groups", kit.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (kind, indices) in groups {
                egui::CollapsingHeader::new(format!("{kind}  ·  {}", indices.len()))
                    .id_salt(("chimp_group", kit.0, &kind))
                    .default_open(false)
                    .show(ui, |ui| {
                        for &index in indices {
                            let package = &world.packages()[index];
                            let label = package.name.rsplit('/').next().unwrap_or(&package.name);
                            let response = ui
                                .selectable_label(selected.as_deref() == Some(&package.name), label)
                                .on_hover_text(&package.name);
                            let actions =
                                ChimpPackageActions::of(&package.name, Some(kind.as_str()));
                            if actions.any() {
                                context_menu(&response, |ui| {
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
        cx.send(ChimpCommand::Open { kit, package });
    }
    send_chimp_extractions(cx, kit, extract_texture, extract_mesh, export_level);
}

fn draw_chimp_archives(ui: &mut Ui, view: &mut ChimpView, world: &World, kit: KitId) {
    let selected = view.selected_archive;
    egui::ScrollArea::vertical()
        .id_salt(("chimp_archives", kit.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if ui
                .selectable_label(selected.is_none(), "All mounted archives")
                .clicked()
            {
                view.selected_archive = None;
                view.browser = ChimpBrowser::Folders;
                view.folder_selection = ChimpFolderSelection::Package;
                view.filter.clear();
                view.reset_filter();
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
                    view.selected_archive = Some(ChimpArchive::IoStore(container.index));
                    view.browser = ChimpBrowser::Folders;
                    view.folder_selection = ChimpFolderSelection::Package;
                    view.filter.clear();
                    view.reset_filter();
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
                    view.selected_archive = Some(ChimpArchive::Pak(container.index));
                    view.browser = ChimpBrowser::Folders;
                    view.folder_selection = ChimpFolderSelection::File;
                    view.filter.clear();
                    view.reset_filter();
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
    ui: &mut Ui,
    cx: &Ctx,
    view: &mut ChimpView,
    world: &World,
    kit_index: usize,
) {
    let kit = cx.model.kits[kit_index].id;
    let selected_archive = view.selected_archive;
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
                view.selected_archive = None;
                view.reset_filter();
            }
        });
        ui.separator();
    }
    let selected_package = cx.model.kits[kit_index].chimp.selected_package.as_deref();
    // Borrowed, like the tree beside it: this used to clone the type of
    // every mounted package (about 104k strings) every frame the default
    // Folders tab was drawn, only to satisfy the borrow checker.
    let clicked = egui::ScrollArea::vertical()
        .id_salt(("chimp_folders", kit.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_chimp_folder_node(
                ui,
                &view.content_tree,
                world,
                &view.package_types,
                selected_package,
                view.selected_file.as_deref(),
                "",
            )
        })
        .inner;
    match clicked {
        Some(ChimpTreeClick::Package(package)) => {
            view.folder_selection = ChimpFolderSelection::Package;
            cx.send(ChimpCommand::Open { kit, package });
        }
        Some(ChimpTreeClick::ExtractTexture(package)) => {
            send_chimp_extractions(cx, kit, Some(package), None, None);
        }
        Some(ChimpTreeClick::ExportLevel(package, format)) => {
            send_chimp_extractions(cx, kit, None, None, Some((package, format)));
        }
        Some(ChimpTreeClick::ExtractMesh(package, format)) => {
            send_chimp_extractions(cx, kit, None, Some((package, format)), None);
        }
        Some(ChimpTreeClick::File(file)) => {
            view.folder_selection = ChimpFolderSelection::File;
            view.selected_file = Some(file);
        }
        None => {}
    }
}

fn draw_chimp_pak_files(ui: &mut Ui, view: &mut ChimpView, world: &World, kit: KitId) {
    let indices = Arc::clone(&view.filtered_files);
    let selected = view.selected_file.clone();
    egui::ScrollArea::vertical()
        .id_salt(("chimp_pak_files", kit.0))
        .auto_shrink([false, false])
        .show_rows(ui, 22.0, indices.len(), |ui, range| {
            for row in range {
                let file = &world.pak_files()[indices[row]];
                let active = file.active_provider();
                let mut label = file.path.clone();
                if file.providers.len() > 1 {
                    label.push_str("  ⧉");
                }
                let response = ui.selectable_label(selected.as_deref() == Some(&file.path), label);
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
                    view.selected_file = Some(file.path.clone());
                }
            }
        });
}

fn draw_chimp_file(ui: &mut Ui, cx: &Ctx, view: &ChimpView, kit_index: usize) {
    let kit = cx.model.kits[kit_index].id;
    let Some(path) = view.selected_file.clone() else {
        crate::app::shell::frame::centered_empty_state(
            ui,
            "Select a file from a legacy .pak container.",
        );
        return;
    };
    let ChimpMount::Ready(world) = &cx.model.kits[kit_index].chimp.mount else {
        return;
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
        cx.send(ChimpCommand::ExtractPakFile {
            kit,
            path: file.path.clone(),
        });
    }
}

/// Send the extractions a package menu asked for.
pub(super) fn send_chimp_extractions(
    cx: &Ctx,
    kit: KitId,
    texture: Option<String>,
    mesh: Option<(String, ChimpMeshFormat)>,
    level: Option<(String, ChimpLevelFormat)>,
) {
    let extractions = [
        texture.map(|package| (package, ChimpExtraction::Texture)),
        mesh.map(|(package, format)| (package, ChimpExtraction::Mesh(format))),
        level.map(|(package, format)| (package, ChimpExtraction::Level(format))),
    ];
    for (package, what) in extractions.into_iter().flatten() {
        cx.send(ChimpCommand::Extract { kit, package, what });
    }
}

impl Baboon {
    /// Write a legacy-pak file to where the user picks.
    pub(super) fn extract_chimp_pak_file(&mut self, kit_index: usize, path: &str) {
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
            return;
        };
        let world = Arc::clone(world);
        let Some(provider) = world.pak_file(path).and_then(|file| file.active_provider()) else {
            return;
        };
        let suggested = std::path::Path::new(path)
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
            Ok(()) => self.model.status = format!("Extracted {}", output.display()),
            Err(error) => {
                self.model.status = format!("Could not extract {}: {error:#}", output.display())
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
            context_menu(&response, |ui| {
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
        let _ = crate::app::run_ui_test(
            &context,
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

    /// Draw kit 0's Chimp surface and apply what it sent, as a frame does.
    fn draw_workspace(app: &mut Baboon) -> impl FnMut(&mut egui::Ui) + '_ {
        move |ui| {
            let ctx = ui.ctx().clone();
            let kit = app.model.kits[0].id;
            egui::CentralPanel::default().show(ui, |ui| {
                draw_chimp_workspace(
                    ui,
                    &cx!(app, &ctx),
                    &mut app.chimp,
                    &mut app.views[kit].chimp,
                    0,
                );
            });
            app.apply_commands(&ctx);
        }
    }

    /// The folder tree nests packages by path; clicking one opens it beside
    /// the tree.
    #[test]
    fn the_folder_tree_opens_a_package() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[]);
        let mut frames = Frames::new();
        frames.click("Game  ·  2", &mut draw_workspace(&mut app));
        frames.click("Test  ·  2", &mut draw_workspace(&mut app));
        assert!(frames.shows("Other"));
        frames.click_exact("Thing", 0, &mut draw_workspace(&mut app));
        assert_eq!(
            app.views[app.model.kits[0].id].chimp.folder_selection,
            ChimpFolderSelection::Package
        );
        assert_eq!(app.model.kits[0].chimp.selected_package.as_deref(), Some(THING));
        apply_until(&mut app, |app| app.model.kits[0].chimp.documents.contains_key(THING));
        frames.frame(Vec::new(), &mut draw_workspace(&mut app));
        assert!(frames.shows("1 exports • 2 imports •"), "the pane is drawn");
    }

    /// The flat package list opens what is clicked, and the search box
    /// narrows it.
    #[test]
    fn the_package_list_opens_and_filters() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[]);
        let mut frames = Frames::new();
        frames.click_exact("Packages", 0, &mut draw_workspace(&mut app));
        assert_eq!(app.views[app.model.kits[0].id].chimp.browser, ChimpBrowser::Packages);
        assert!(frames.shows(THING) && frames.shows(OTHER));
        frames.click_exact(OTHER, 0, &mut draw_workspace(&mut app));
        apply_until(&mut app, |app| app.model.kits[0].chimp.documents.contains_key(OTHER));

        frames.click("Search package or container…", &mut draw_workspace(&mut app));
        frames.type_text("thing", &mut draw_workspace(&mut app));
        frames.frame(Vec::new(), &mut draw_workspace(&mut app));
        let chimp = &app.model.kits[0].chimp;
        let view = &app.views[app.model.kits[0].id].chimp;
        assert_eq!(view.filter, "thing");
        let ChimpMount::Ready(world) = &chimp.mount else {
            unreachable!()
        };
        assert_eq!(
            view
                .filtered_packages
                .iter()
                .map(|&index| world.packages()[index].name.as_str())
                .collect::<Vec<_>>(),
            [THING]
        );
    }

    /// The archive list names each container with its package count; picking
    /// one scopes the folder tree to it until "Show all".
    #[test]
    fn the_archive_list_scopes_the_tree() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[]);
        let mut frames = Frames::new();
        frames.click_exact("Archives", 0, &mut draw_workspace(&mut app));
        for text in [
            "All mounted archives",
            "pakchunk0-Windows.utoc  ·  2 packages",
            "pakchunk0-Windows.pak  ·  0 files",
            "Unavailable or empty",
            "Select an archive to browse its folder hierarchy.",
        ] {
            assert!(frames.shows(text), "{text}");
        }
        frames.click("pakchunk0-Windows.utoc  ·  2 packages", &mut draw_workspace(&mut app));
        let chimp = &app.views[app.model.kits[0].id].chimp;
        assert_eq!(chimp.selected_archive, Some(ChimpArchive::IoStore(0)));
        assert_eq!(chimp.browser, ChimpBrowser::Folders);
        assert!(frames.shows("Game  ·  2"));
        frames.click("Show all", &mut draw_workspace(&mut app));
        assert_eq!(app.views[app.model.kits[0].id].chimp.selected_archive, None);

        frames.click_exact("Archives", 0, &mut draw_workspace(&mut app));
        frames.click("pakchunk0-Windows.pak  ·  0 files", &mut draw_workspace(&mut app));
        let chimp = &app.views[app.model.kits[0].id].chimp;
        assert_eq!(chimp.selected_archive, Some(ChimpArchive::Pak(0)));
        assert_eq!(chimp.folder_selection, ChimpFolderSelection::File);
        assert!(frames.shows("Select a file from a legacy .pak container."));
        assert!(!frames.shows("Game  ·  2"), "the pak holds no packages");
    }

    /// Packages group by their indexed type; a group opens onto its packages.
    #[test]
    fn the_group_list_opens_a_package_by_type() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[]);
        // Indexed by mount order, which sorts `Other` first.
        app.views[app.model.kits[0].id].chimp.package_types = vec![Some("Texture2D".to_owned()), None];
        let mut frames = Frames::new();
        frames.click_exact("Groups", 0, &mut draw_workspace(&mut app));
        assert!(frames.shows("Texture2D  ·  1"));
        assert!(frames.shows("Unknown  ·  1"));
        frames.click("Texture2D  ·  1", &mut draw_workspace(&mut app));
        frames.click_exact("Other", 0, &mut draw_workspace(&mut app));
        apply_until(&mut app, |app| app.model.kits[0].chimp.documents.contains_key(OTHER));
    }

    /// Before the mount, the workspace offers to start it, waits while it
    /// runs, and browses once it lands; a failed mount offers a retry.
    #[test]
    fn the_mount_status_starts_waits_and_retries() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[]);
        app.model.prefs.enable_chimp = true;
        app.model.kits[0].chimp.mount = ChimpMount::Idle;
        let mut frames = Frames::new();
        frames.frame(Vec::new(), &mut draw_workspace(&mut app));
        assert!(frames.shows("The Unreal package index has not been started."));
        frames.click("Start Chimp", &mut draw_workspace(&mut app));
        assert!(matches!(app.model.kits[0].chimp.mount, ChimpMount::Loading));
        frames.frame(Vec::new(), &mut draw_workspace(&mut app));
        assert!(frames.shows("Please wait — Chimp is starting up…"));
        apply_until(&mut app, |app| {
            matches!(app.model.kits[0].chimp.mount, ChimpMount::Ready(_))
                && !app.views[app.model.kits[0].id].chimp.type_indexing
        });
        frames.frame(Vec::new(), &mut draw_workspace(&mut app));
        assert!(frames.shows("Game  ·  2"));

        app.model.kits[0].chimp.mount = ChimpMount::Failed("no containers".to_owned());
        frames.frame(Vec::new(), &mut draw_workspace(&mut app));
        assert!(frames.shows("no containers"));
        frames.click("Retry", &mut draw_workspace(&mut app));
        assert!(matches!(app.model.kits[0].chimp.mount, ChimpMount::Loading));
        apply_until(&mut app, |app| {
            matches!(app.model.kits[0].chimp.mount, ChimpMount::Ready(_))
                && !app.views[app.model.kits[0].id].chimp.type_indexing
        });
    }

    /// The toolbar's discard button opens the prompt for every modified
    /// package, and does nothing while there are none.
    #[test]
    fn the_toolbar_discard_prompts_for_modified_packages() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        // The toolbar is right-aligned on the first row.
        let button = egui::pos2(VALUE_X - 4.0, 8.0 + 12.0);
        let mut frames = Frames::new();
        frames.frame(Vec::new(), &mut draw_workspace(&mut app));
        frames.click_at(button, &mut draw_workspace(&mut app));
        assert!(
            app.dialogs.get::<ChimpDiscardPrompt>().is_none(),
            "disabled while clean"
        );

        app.model.kits[0].chimp.documents.get_mut(THING).unwrap().dirty = true;
        frames.click_at(button, &mut draw_workspace(&mut app));
        let prompt = app
            .dialogs
            .get::<ChimpDiscardPrompt>()
            .expect("the prompt opened");
        assert_eq!(prompt.packages, [THING]);
        assert!(prompt.pending_action.is_none());
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

        let mut browser = ChimpView {
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
        let (_, pane) = load_chimp_document_with_pane(&world, &package).unwrap();
        assert_eq!(pane.view, ChimpDocumentView::Texture);
        let decoded = pane
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
