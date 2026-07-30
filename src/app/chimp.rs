//! Chimp: the Campaign Evolved Unreal package workspace.
//!
//! Chimp is deliberately scoped to a loaded Campaign Evolved kit. It shares
//! that kit's Paks root but owns its own package index, documents and editor
//! state; none of those concepts are forced through the editing-kit/tag model.

use super::*;
use std::collections::BTreeMap;
use std::io::Cursor;

use blam_tags::iostore::container::writer::{PackageOverride, write_package_mod_container};
use blam_tags::iostore::object::archive::ExportContext;
use blam_tags::iostore::object::export::{Export, ExportBlock, read_export_in, write_export_in};
use blam_tags::iostore::object::value::{PropValue, PropertyBlock};
use blam_tags::iostore::package::builder::{read_payloads, write_package};
use blam_tags::iostore::package::zen::FZenPackageHeader;
use blam_tags::iostore::usmap::Usmap;
use blam_tags::iostore::world::{CE_HEADER_VERSION, CE_TOC_VERSION, PackageProvider, World};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum KitSurface {
    #[default]
    Tags,
    Chimp,
}

pub(super) enum ChimpMount {
    Idle,
    Loading,
    Ready(Arc<World>),
    Failed(String),
}

impl Default for ChimpMount {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpBrowser {
    #[default]
    Folders,
    Archives,
    Packages,
    Files,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChimpArchive {
    IoStore(usize),
    Pak(usize),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpFolderSelection {
    #[default]
    Package,
    File,
}

enum ChimpTreeClick {
    Package(String),
    File(String),
}

#[derive(Default)]
struct ChimpFolderNode {
    folders: BTreeMap<String, ChimpFolderNode>,
    packages: Vec<ChimpPackageLeaf>,
    files: Vec<ChimpFileLeaf>,
    package_count: usize,
    file_count: usize,
}

struct ChimpPackageLeaf {
    name: String,
    package: usize,
}

struct ChimpFileLeaf {
    name: String,
    file: usize,
}

impl ChimpFolderNode {
    fn insert_package(&mut self, package: usize, path: &str) {
        let mut segments = path
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .peekable();
        let mut node = self;
        node.package_count += 1;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                node.packages.push(ChimpPackageLeaf {
                    name: segment.to_owned(),
                    package,
                });
                return;
            }
            node = node.folders.entry(segment.to_owned()).or_default();
            node.package_count += 1;
        }
    }

    fn insert_file(&mut self, file: usize, path: &str) {
        let normalized = path.replace('\\', "/");
        let mut segments = normalized
            .split('/')
            .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
            .peekable();
        let mut node = self;
        node.file_count += 1;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                node.files.push(ChimpFileLeaf {
                    name: segment.to_owned(),
                    file,
                });
                return;
            }
            node = node.folders.entry(segment.to_owned()).or_default();
            node.file_count += 1;
        }
    }

    fn entry_count(&self) -> usize {
        self.package_count + self.file_count
    }
}

#[derive(Default)]
pub(super) struct ChimpState {
    pub(super) mount: ChimpMount,
    browser: ChimpBrowser,
    pub(super) filter: String,
    filtered_for: Option<String>,
    filtered_packages: Vec<usize>,
    filtered_files: Vec<usize>,
    content_tree: ChimpFolderNode,
    selected_archive: Option<ChimpArchive>,
    folder_selection: ChimpFolderSelection,
    pub(super) selected_package: Option<String>,
    selected_file: Option<String>,
    pub(super) open_packages: Vec<String>,
    pub(super) documents: HashMap<String, ChimpDocument>,
    pub(super) loading_packages: HashSet<String>,
}

pub(super) struct ChimpDocument {
    pub(super) package: String,
    pub(super) provider: PackageProvider,
    pub(super) original: Vec<u8>,
    pub(super) header: FZenPackageHeader,
    pub(super) payloads: Vec<Vec<u8>>,
    pub(super) exports: Vec<ChimpExport>,
    pub(super) selected_export: usize,
    pub(super) dirty: bool,
}

#[derive(Default, Deserialize, Serialize)]
struct ChimpRecoveryManifest {
    source: String,
    packages: HashMap<String, String>,
}

pub(super) struct ChimpExport {
    pub(super) object: String,
    pub(super) class: Option<String>,
    pub(super) decoded: Result<Export, String>,
}

impl ChimpState {
    fn filter_is_current(&self, query: &str) -> bool {
        self.filtered_for.as_deref() == Some(query)
    }

    fn refresh_filter(&mut self, world: &World) {
        let query = self.filter.trim().to_ascii_lowercase();
        if self.filter_is_current(&query) {
            return;
        }
        let selected_archive = self.selected_archive;
        self.filtered_for = Some(query.clone());
        self.filtered_packages.clear();
        self.filtered_files.clear();
        self.filtered_packages.extend(
            world
                .packages()
                .iter()
                .enumerate()
                .filter(|(_, package)| {
                    let archive_matches = match selected_archive {
                        None => true,
                        Some(ChimpArchive::IoStore(container)) => package
                            .providers
                            .iter()
                            .any(|provider| provider.container == container),
                        Some(ChimpArchive::Pak(_)) => false,
                    };
                    archive_matches
                        && (query.is_empty()
                            || package.name.to_ascii_lowercase().contains(&query)
                            || package.providers.iter().any(|provider| {
                                world.containers()[provider.container]
                                    .path
                                    .to_string_lossy()
                                    .to_ascii_lowercase()
                                    .contains(&query)
                            }))
                })
                .map(|(index, _)| index),
        );
        self.filtered_files.extend(
            world
                .pak_files()
                .iter()
                .enumerate()
                .filter(|(_, file)| {
                    let archive_matches = match selected_archive {
                        None => true,
                        Some(ChimpArchive::Pak(container)) => file
                            .providers
                            .iter()
                            .any(|provider| provider.container == container),
                        Some(ChimpArchive::IoStore(_)) => false,
                    };
                    archive_matches
                        && (query.is_empty()
                            || file.path.to_ascii_lowercase().contains(&query)
                            || file.providers.iter().any(|provider| {
                                world.pak_containers()[provider.container]
                                    .path
                                    .to_string_lossy()
                                    .to_ascii_lowercase()
                                    .contains(&query)
                            }))
                })
                .map(|(index, _)| index),
        );
        self.content_tree = ChimpFolderNode::default();
        for &index in &self.filtered_packages {
            self.content_tree
                .insert_package(index, &world.packages()[index].name);
        }
        for &index in &self.filtered_files {
            self.content_tree
                .insert_file(index, &world.pak_files()[index].path);
        }
    }

    fn reset_filter(&mut self) {
        self.filtered_for = None;
        self.filtered_packages.clear();
        self.filtered_files.clear();
        self.content_tree = ChimpFolderNode::default();
    }
}

pub(super) fn load_chimp_document(world: &World, package: &str) -> Result<ChimpDocument, String> {
    let record = world
        .package(package)
        .ok_or_else(|| format!("{package} is not mounted"))?;
    let provider = record
        .active_provider()
        .cloned()
        .ok_or_else(|| format!("{package} has no active provider"))?;
    let bytes = world
        .read_provider(&provider)
        .map_err(|error| error.to_string())?;
    decode_chimp_document(world, provider, bytes)
}

fn decode_chimp_document(
    world: &World,
    provider: PackageProvider,
    bytes: Vec<u8>,
) -> Result<ChimpDocument, String> {
    let header = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .map_err(|error| format!("Could not parse {}: {error:#}", provider.entry_path))?;
    let payloads = read_payloads(&header, &bytes)
        .map_err(|error| format!("Could not split {}: {error:#}", provider.entry_path))?;
    let names = header.name_map.copy_raw_names();
    let resolver = world.resolver(&header, &bytes, &names);
    let bulk: Vec<(i64, i64)> = header
        .bulk_data
        .iter()
        .map(|entry| (entry.serial_offset, entry.serial_size))
        .collect();
    let context = ExportContext {
        bulk_data: &bulk,
        resolver: Some(&resolver),
    };
    let exports = header
        .export_map
        .iter()
        .zip(&payloads)
        .map(|(entry, payload)| {
            let object = header.name_map.get(entry.object_name).to_string();
            let class = world.class_key(&header, entry.class_index);
            let decoded = class
                .as_deref()
                .ok_or_else(|| "class could not be resolved".to_owned())
                .and_then(|class| {
                    read_export_in(
                        payload,
                        &names,
                        world.usmap(),
                        class,
                        entry.object_flags,
                        &context,
                    )
                    .map_err(|error| error.to_string())
                });
            ChimpExport {
                object,
                class,
                decoded,
            }
        })
        .collect();
    Ok(ChimpDocument {
        package: header.package_name(),
        provider,
        original: bytes,
        header,
        payloads,
        exports,
        selected_export: 0,
        dirty: false,
    })
}

fn rebuild_chimp_document(
    world: &World,
    document: &ChimpDocument,
) -> Result<(Vec<u8>, blam_tags::iostore::container::header::StoreEntry), String> {
    let names = document.header.name_map.copy_raw_names();
    let resolver = world.resolver(&document.header, &document.original, &names);
    let mut payloads = document.payloads.clone();
    for (index, export) in document.exports.iter().enumerate() {
        let (Some(class), Ok(decoded)) = (export.class.as_deref(), &export.decoded) else {
            continue;
        };
        payloads[index] =
            write_export_in(class, decoded, world.usmap(), Some(&resolver)).map_err(|error| {
                format!(
                    "Could not serialize {} export {}: {error:#}",
                    document.package, export.object
                )
            })?;
    }
    write_package(&document.header, &payloads, CE_HEADER_VERSION)
        .map_err(|error| format!("Could not rebuild {}: {error:#}", document.package))
}

impl Baboon {
    pub(super) fn begin_chimp_mount(&mut self, kit_index: usize, ctx: egui::Context) {
        if !self.enable_chimp {
            return;
        }
        let Some(source) = self.kits.get(kit_index).and_then(|kit| kit.source.as_ref()) else {
            return;
        };
        let TagSource::IoStoreContainerSet { root, .. } = &source.source else {
            return;
        };
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        let root = root.clone();
        self.kits[kit_index].chimp.mount = ChimpMount::Loading;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = (|| {
                let usmap = Usmap::meteorite().map_err(|error| error.to_string())?;
                // Keep startup to container discovery + the lightweight package
                // index. Generated Blueprint schema recovery is intentionally
                // lazy/future work; doing the whole corpus here would leave the
                // workspace saying "loading" while reading every package.
                let world = World::open(&root, usmap).map_err(|error| error.to_string())?;
                Ok(Arc::new(world))
            })();
            let _ = tx.send(WorkerMessage::ChimpMounted { stamp, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn handle_chimp_mounted(
        &mut self,
        stamp: KitStamp,
        result: Result<Arc<World>, String>,
    ) -> bool {
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        self.kits[index].chimp.reset_filter();
        match result {
            Ok(world) => {
                let packages = world.packages().len();
                let files = world.pak_files().len();
                let diagnostics = world.diagnostics().len();
                self.kits[index].chimp.mount = ChimpMount::Ready(world.clone());
                self.status = if diagnostics == 0 {
                    format!("Chimp indexed {packages} Unreal packages and {files} pak files")
                } else {
                    format!(
                        "Chimp indexed {packages} Unreal packages and {files} pak files with {diagnostics} container warning(s)"
                    )
                };
                self.restore_chimp_recovery(index, &world);
            }
            Err(error) => {
                self.kits[index].chimp.mount = ChimpMount::Failed(error.clone());
                self.status = format!("Chimp could not open: {error}");
            }
        }
        false
    }

    fn chimp_recovery_dir(&self, kit_index: usize) -> Option<PathBuf> {
        let root = match &self.kits.get(kit_index)?.source.as_ref()?.source {
            TagSource::IoStoreContainerSet { root, .. } => root,
            _ => return None,
        };
        let digest = Sha256::digest(root.to_string_lossy().as_bytes());
        let key: String = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Some(crate::storage::data_path(&format!("chimp-recovery-{key}")))
    }

    fn load_chimp_recovery_manifest(
        &self,
        kit_index: usize,
    ) -> Option<(PathBuf, ChimpRecoveryManifest)> {
        let directory = self.chimp_recovery_dir(kit_index)?;
        let text = fs::read_to_string(directory.join("manifest.json")).ok()?;
        let manifest = serde_json::from_str(&text).ok()?;
        Some((directory, manifest))
    }

    fn restore_chimp_recovery(&mut self, kit_index: usize, world: &Arc<World>) {
        let Some((directory, manifest)) = self.load_chimp_recovery_manifest(kit_index) else {
            return;
        };
        let expected_source = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.root_path().display().to_string())
            .unwrap_or_default();
        if manifest.source != expected_source {
            return;
        }
        let mut restored = 0usize;
        for (package, filename) in manifest.packages {
            let Some(provider) = world
                .package(&package)
                .and_then(|record| record.active_provider())
                .cloned()
            else {
                continue;
            };
            let Ok(bytes) = fs::read(directory.join(filename)) else {
                continue;
            };
            let Ok(mut document) = decode_chimp_document(world, provider, bytes) else {
                continue;
            };
            document.dirty = true;
            if !self.kits[kit_index].chimp.open_packages.contains(&package) {
                self.kits[kit_index]
                    .chimp
                    .open_packages
                    .push(package.clone());
            }
            self.kits[kit_index]
                .chimp
                .documents
                .insert(package.clone(), document);
            self.kits[kit_index].chimp.selected_package = Some(package);
            restored += 1;
        }
        if restored > 0 {
            self.status = format!("Chimp recovered {restored} unsaved package edit(s)");
        }
    }

    fn checkpoint_chimp_document(&mut self, kit_index: usize, package: &str) {
        let Some(directory) = self.chimp_recovery_dir(kit_index) else {
            return;
        };
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let Ok((bytes, _)) = rebuild_chimp_document(world, document) else {
            return;
        };
        if fs::create_dir_all(&directory).is_err() {
            return;
        }
        let digest = Sha256::digest(package.as_bytes());
        let filename = format!(
            "{}.uasset",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        if fs::write(directory.join(&filename), bytes).is_err() {
            return;
        }
        let source = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.root_path().display().to_string())
            .unwrap_or_default();
        let mut manifest = self
            .load_chimp_recovery_manifest(kit_index)
            .map(|(_, manifest)| manifest)
            .unwrap_or_default();
        manifest.source = source;
        manifest.packages.insert(package.to_owned(), filename);
        if let Ok(bytes) = serde_json::to_vec_pretty(&manifest) {
            let _ = fs::write(directory.join("manifest.json"), bytes);
        }
    }

    fn clear_chimp_recovery_packages(&self, kit_index: usize, packages: &[String]) {
        let Some((directory, mut manifest)) = self.load_chimp_recovery_manifest(kit_index) else {
            return;
        };
        for package in packages {
            if let Some(filename) = manifest.packages.remove(package) {
                let _ = fs::remove_file(directory.join(filename));
            }
        }
        if manifest.packages.is_empty() {
            let _ = fs::remove_file(directory.join("manifest.json"));
            let _ = fs::remove_dir(directory);
        } else if let Ok(bytes) = serde_json::to_vec_pretty(&manifest) {
            let _ = fs::write(directory.join("manifest.json"), bytes);
        }
    }

    fn begin_chimp_open_package(&mut self, kit_index: usize, package: String, ctx: egui::Context) {
        if self.kits[kit_index].documents_contains_chimp(&package) {
            self.kits[kit_index].chimp.selected_package = Some(package);
            return;
        }
        self.kits[kit_index].chimp.selected_package = Some(package.clone());
        if !self.kits[kit_index]
            .chimp
            .loading_packages
            .insert(package.clone())
        {
            return;
        }
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = load_chimp_document(&world, &package);
            let _ = tx.send(WorkerMessage::ChimpPackageLoaded {
                stamp,
                package,
                result,
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn handle_chimp_package_loaded(
        &mut self,
        stamp: KitStamp,
        package: String,
        result: Result<ChimpDocument, String>,
    ) -> bool {
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        let chimp = &mut self.kits[index].chimp;
        chimp.loading_packages.remove(&package);
        match result {
            Ok(document) => {
                if !chimp.open_packages.contains(&package) {
                    chimp.open_packages.push(package.clone());
                }
                chimp.documents.insert(package.clone(), document);
                chimp.selected_package = Some(package);
            }
            Err(error) => self.status = error,
        }
        false
    }

    pub(super) fn draw_chimp_workspace(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
    ) {
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
                                    self.draw_chimp_document(ui, kit_index)
                                }
                                ChimpFolderSelection::File => self.draw_chimp_file(ui, kit_index),
                            }
                        }
                        ChimpBrowser::Packages => self.draw_chimp_document(ui, kit_index),
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
            ui.selectable_value(
                &mut self.kits[kit_index].chimp.browser,
                ChimpBrowser::Archives,
                "Archives",
            );
            ui.selectable_value(
                &mut self.kits[kit_index].chimp.browser,
                ChimpBrowser::Folders,
                "Folders",
            );
            ui.selectable_value(
                &mut self.kits[kit_index].chimp.browser,
                ChimpBrowser::Packages,
                "Packages",
            );
            ui.selectable_value(
                &mut self.kits[kit_index].chimp.browser,
                ChimpBrowser::Files,
                "Pak files",
            );
            if matches!(self.kits[kit_index].chimp.mount, ChimpMount::Loading) {
                ui.spinner();
            }
        });
        ui.add_space(4.0);
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.kits[kit_index].chimp.filter)
                .hint_text("Search package or container…")
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
        if !world.diagnostics().is_empty() {
            egui::CollapsingHeader::new(format!(
                "{} container warning(s)",
                world.diagnostics().len()
            ))
            .id_salt(("chimp_diagnostics", self.kits[kit_index].id.0))
            .show(ui, |ui| {
                for diagnostic in world.diagnostics() {
                    ui.colored_label(
                        Color32::from_rgb(210, 150, 70),
                        diagnostic.path.display().to_string(),
                    );
                    ui.label(&diagnostic.message);
                }
            });
        }
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
        let indices = self.kits[kit_index].chimp.filtered_packages.clone();
        let selected = self.kits[kit_index].chimp.selected_package.clone();
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
                    if response.clicked() {
                        self.begin_chimp_open_package(kit_index, package.name.clone(), ctx.clone());
                    }
                }
            });
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
        let clicked = egui::ScrollArea::vertical()
            .id_salt(("chimp_folders", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                draw_chimp_folder_node(
                    ui,
                    &self.kits[kit_index].chimp.content_tree,
                    world,
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
            Some(ChimpTreeClick::File(file)) => {
                let chimp = &mut self.kits[kit_index].chimp;
                chimp.folder_selection = ChimpFolderSelection::File;
                chimp.selected_file = Some(file);
            }
            None => {}
        }
    }

    fn draw_chimp_pak_files(&mut self, ui: &mut Ui, world: &World, kit_index: usize) {
        let indices = self.kits[kit_index].chimp.filtered_files.clone();
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

    fn draw_chimp_document(&mut self, ui: &mut Ui, kit_index: usize) {
        let selected = self.kits[kit_index].chimp.selected_package.clone();
        let open = self.kits[kit_index].chimp.open_packages.clone();
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            for package in &open {
                let dirty = self.kits[kit_index]
                    .chimp
                    .documents
                    .get(package)
                    .is_some_and(|document| document.dirty);
                let label = if dirty {
                    format!("• {}", package.rsplit('/').next().unwrap_or(package))
                } else {
                    package.rsplit('/').next().unwrap_or(package).to_owned()
                };
                if ui
                    .selectable_label(selected.as_deref() == Some(package), label)
                    .clicked()
                {
                    self.kits[kit_index].chimp.selected_package = Some(package.clone());
                }
                if ui.small_button("×").clicked() {
                    close = Some(package.clone());
                }
            }
        });
        ui.separator();
        if let Some(package) = close {
            if self.kits[kit_index]
                .chimp
                .documents
                .get(&package)
                .is_some_and(|document| document.dirty)
            {
                self.status =
                    "Build the Chimp mod before closing this modified package.".to_owned();
                return;
            }
            self.kits[kit_index]
                .chimp
                .open_packages
                .retain(|open| open != &package);
            self.kits[kit_index].chimp.documents.remove(&package);
            self.kits[kit_index].chimp.selected_package =
                self.kits[kit_index].chimp.open_packages.last().cloned();
            return;
        }

        let Some(package) = self.kits[kit_index].chimp.selected_package.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a package to inspect it.");
            });
            return;
        };
        if self.kits[kit_index]
            .chimp
            .loading_packages
            .contains(&package)
        {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Opening {package}…"));
            });
            return;
        }
        if !self.kits[kit_index].chimp.documents.contains_key(&package) {
            ui.heading(&package);
            if ui.button("Open package").clicked() {
                self.begin_chimp_open_package(kit_index, package, ui.ctx().clone());
            }
            return;
        }

        let mut save_mod = false;
        let mut extract_package = false;
        let mut extract_json = false;
        let mut extract_export = false;
        {
            let document = self.kits[kit_index]
                .chimp
                .documents
                .get_mut(&package)
                .expect("checked above");
            ui.horizontal(|ui| {
                ui.heading(&document.package);
                ui.separator();
                save_mod = ui
                    .add_enabled(document.dirty, egui::Button::new("Build Chimp mod"))
                    .clicked();
                extract_package = ui.button("Extract package…").clicked();
                extract_json = ui.button("Property dump…").clicked();
                extract_export = ui.button("Extract selected export…").clicked();
            });
        }
        if save_mod {
            self.build_chimp_mod(kit_index, ui.ctx().clone());
        }
        if extract_package {
            self.extract_chimp_package(kit_index, &package);
        }
        if extract_json {
            self.extract_chimp_json(kit_index, &package);
        }
        if extract_export {
            self.extract_chimp_export(kit_index, &package);
        }

        let world = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => return,
        };
        let document = self.kits[kit_index]
            .chimp
            .documents
            .get_mut(&package)
            .expect("checked above");
        let container = &world.containers()[document.provider.container];
        ui.label(
            RichText::new(format!(
                "{} exports • {} imports • {} bytes • {}",
                document.header.export_map.len(),
                document.header.import_map.len(),
                document.original.len(),
                container.path.display()
            ))
            .color(subtle_dark()),
        );
        egui::CollapsingHeader::new("Package metadata and dependencies")
            .id_salt(("chimp_metadata", package.clone()))
            .show(ui, |ui| {
                ui.label(format!(
                    "Name map: {} names",
                    document.header.name_map.copy_raw_names().len()
                ));
                ui.label(format!(
                    "Imported package references: {}",
                    document.header.imported_package_names.len()
                ));
                for imported in &document.header.imported_package_names {
                    ui.monospace(imported);
                }
                ui.label(format!(
                    "External dependency records: {}",
                    document.header.external_package_dependencies.len()
                ));
                for dependency in &document.header.external_package_dependencies {
                    ui.monospace(format!("{dependency:?}"));
                }
            });
        egui::CollapsingHeader::new(format!(
            "Physical providers ({})",
            world
                .package(&document.package)
                .map_or(0, |record| record.providers.len())
        ))
        .id_salt(("chimp_providers", package.clone()))
        .show(ui, |ui| {
            if let Some(record) = world.package(&document.package) {
                for provider in record.providers.iter().rev() {
                    let active = record.active_provider() == Some(provider);
                    ui.label(format!(
                        "{}{}",
                        if active {
                            "Active — "
                        } else {
                            "Shadowed — "
                        },
                        world.containers()[provider.container].path.display()
                    ));
                    ui.monospace(&provider.entry_path);
                }
            }
        });
        ui.separator();
        egui::SidePanel::left(egui::Id::new(("chimp_exports", package.clone())))
            .resizable(true)
            .default_width(220.0)
            .show_inside(ui, |ui| {
                ui.label(RichText::new("Exports").strong());
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (index, export) in document.exports.iter().enumerate() {
                        let supported = export.decoded.is_ok();
                        let label =
                            format!("{}  {}", if supported { "●" } else { "○" }, export.object);
                        if ui
                            .selectable_label(document.selected_export == index, label)
                            .on_hover_text(export.class.as_deref().unwrap_or("Unknown class"))
                            .clicked()
                        {
                            document.selected_export = index;
                        }
                    }
                });
            });
        let changed = egui::CentralPanel::default()
            .show_inside(ui, |ui| draw_chimp_export_editor(ui, document))
            .inner;
        if changed {
            document.dirty = true;
        }
        let _ = document;
        if changed {
            self.checkpoint_chimp_document(kit_index, &package);
        }
    }

    fn chimp_output_path(&self, kit_index: usize) -> Option<PathBuf> {
        let root = match &self.kits.get(kit_index)?.source.as_ref()?.source {
            TagSource::IoStoreContainerSet { root, .. } => root,
            _ => return None,
        };
        let directory = self
            .chimp_output_dir
            .clone()
            .unwrap_or_else(|| root.join("~mods").join("Chimp"));
        Some(directory.join("Chimp_P.utoc"))
    }

    pub(super) fn build_chimp_mod(&mut self, kit_index: usize, ctx: egui::Context) {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let Some(output) = self.chimp_output_path(kit_index) else {
            return;
        };
        let mut rebuilt = Vec::new();
        for document in self.kits[kit_index]
            .chimp
            .documents
            .values()
            .filter(|document| document.dirty)
        {
            match rebuild_chimp_document(&world, document) {
                Ok((bytes, store)) => rebuilt.push((
                    document.package.clone(),
                    document.provider.clone(),
                    bytes,
                    store,
                )),
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        }
        if rebuilt.is_empty() {
            self.status = "Chimp has no modified packages to build".to_owned();
            return;
        }
        if let Some(parent) = output.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            self.status = format!("Could not create {}: {error}", parent.display());
            return;
        }
        let temporary = output.with_file_name(format!(
            "{}.building.utoc",
            output
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Chimp_P")
        ));
        let overrides: Vec<PackageOverride<'_>> = rebuilt
            .iter()
            .map(|(_, provider, bytes, store)| PackageOverride {
                archive: &world.archives()[provider.container],
                uasset_path: &provider.entry_path,
                bytes: bytes.clone(),
                store: store.clone(),
            })
            .collect();
        let override_count = overrides.len();
        let built_packages: Vec<String> = rebuilt
            .iter()
            .map(|(package, _, _, _)| package.clone())
            .collect();
        let written = write_package_mod_container(&overrides, &temporary);
        drop(overrides);
        if let Err(error) = written {
            remove_chimp_triplet(&temporary);
            self.status = format!("Could not build {}: {error}", output.display());
            return;
        }

        // The active Chimp World (and possibly Baboon's tag mount) can have the
        // existing output memory-mapped. Finish the replacement container at a
        // sibling path, release those mappings, then swap the triplet with a
        // rollback copy. The shipped game containers are never touched.
        drop(world);
        self.kits[kit_index].chimp.mount = ChimpMount::Idle;
        let released = match self.release_export_target_mappings(kit_index, &output) {
            Ok(released) => released,
            Err(error) => {
                remove_chimp_triplet(&temporary);
                self.status = error;
                self.begin_chimp_mount(kit_index, ctx);
                return;
            }
        };
        let replaced = replace_chimp_triplet(&temporary, &output);
        let reopen_failures = self.restore_released_mappings(kit_index, &released);
        match replaced {
            Ok(()) => {
                for (package, _, _, _) in rebuilt {
                    if let Some(document) = self.kits[kit_index].chimp.documents.get_mut(&package) {
                        document.dirty = false;
                    }
                }
                self.clear_chimp_recovery_packages(kit_index, &built_packages);
                self.status = format!(
                    "Built {} modified Unreal package(s) into {}",
                    override_count,
                    output.display()
                );
                if !reopen_failures.is_empty() {
                    self.status.push_str(&format!(
                        "; {} tag mount(s) could not be reopened",
                        reopen_failures.len()
                    ));
                }
            }
            Err(error) => {
                self.status = format!("Could not install {}: {error}", output.display());
            }
        }
        self.begin_chimp_mount(kit_index, ctx);
    }

    fn extract_chimp_package(&mut self, kit_index: usize, package: &str) {
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let bytes = if document.dirty {
            let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
                return;
            };
            match rebuild_chimp_document(world, document) {
                Ok((bytes, _)) => bytes,
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        } else {
            document.original.clone()
        };
        let suggested = format!("{}.uasset", package.rsplit('/').next().unwrap_or("package"));
        let Some(path) = rfd::FileDialog::new()
            .set_title("Extract Unreal package")
            .set_file_name(&suggested)
            .save_file()
        else {
            return;
        };
        match fs::write(&path, bytes) {
            Ok(()) => self.status = format!("Extracted {}", path.display()),
            Err(error) => self.status = format!("Could not write {}: {error}", path.display()),
        }
    }

    fn extract_chimp_export(&mut self, kit_index: usize, package: &str) {
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let index = document
            .selected_export
            .min(document.payloads.len().saturating_sub(1));
        let Some(payload) = document.payloads.get(index) else {
            return;
        };
        let name = document
            .exports
            .get(index)
            .map(|export| export.object.as_str())
            .unwrap_or("export");
        let Some(path) = rfd::FileDialog::new()
            .set_title("Extract raw Unreal export")
            .set_file_name(format!("{name}.bin"))
            .save_file()
        else {
            return;
        };
        match fs::write(&path, payload) {
            Ok(()) => self.status = format!("Extracted {}", path.display()),
            Err(error) => self.status = format!("Could not write {}: {error}", path.display()),
        }
    }

    fn extract_chimp_json(&mut self, kit_index: usize, package: &str) {
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let value = chimp_document_json(document);
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export Unreal property dump")
            .set_file_name(format!(
                "{}.json",
                package.rsplit('/').next().unwrap_or("package")
            ))
            .save_file()
        else {
            return;
        };
        match serde_json::to_vec_pretty(&value)
            .and_then(|bytes| fs::write(&path, bytes).map_err(serde_json::Error::io))
        {
            Ok(()) => self.status = format!("Exported {}", path.display()),
            Err(error) => self.status = format!("Could not write {}: {error}", path.display()),
        }
    }
}

fn draw_chimp_folder_node(
    ui: &mut Ui,
    node: &ChimpFolderNode,
    world: &World,
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
                    draw_chimp_folder_node(ui, child, world, selected_package, selected_file, &path)
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

fn triplet(path: &Path) -> [PathBuf; 3] {
    [
        path.with_extension("utoc"),
        path.with_extension("ucas"),
        path.with_extension("pak"),
    ]
}

fn remove_chimp_triplet(path: &Path) {
    for file in triplet(path) {
        let _ = fs::remove_file(file);
    }
}

fn replace_chimp_triplet(temporary: &Path, output: &Path) -> Result<(), String> {
    let incoming = triplet(temporary);
    let targets = triplet(output);
    let backups = [
        output.with_extension("utoc.previous"),
        output.with_extension("ucas.previous"),
        output.with_extension("pak.previous"),
    ];
    let mut backed_up = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        if !target.exists() {
            continue;
        }
        let _ = fs::remove_file(&backups[index]);
        if let Err(error) = fs::rename(target, &backups[index]) {
            for &done in backed_up.iter().rev() {
                let _ = fs::rename(&backups[done], &targets[done]);
            }
            remove_chimp_triplet(temporary);
            return Err(format!("could not back up {}: {error}", target.display()));
        }
        backed_up.push(index);
    }
    let mut installed = Vec::new();
    for index in 0..incoming.len() {
        if let Err(error) = fs::rename(&incoming[index], &targets[index]) {
            for &done in installed.iter().rev() {
                let _ = fs::remove_file(&targets[done]);
            }
            for &done in backed_up.iter().rev() {
                let _ = fs::rename(&backups[done], &targets[done]);
            }
            remove_chimp_triplet(temporary);
            return Err(format!(
                "could not install {}: {error}",
                targets[index].display()
            ));
        }
        installed.push(index);
    }
    for index in backed_up {
        let _ = fs::remove_file(&backups[index]);
    }
    Ok(())
}

impl Kit {
    fn documents_contains_chimp(&self, package: &str) -> bool {
        self.chimp.documents.contains_key(package)
    }
}

fn draw_chimp_export_editor(ui: &mut Ui, document: &mut ChimpDocument) -> bool {
    let Some(export) = document.exports.get_mut(document.selected_export) else {
        ui.label("This package has no exports.");
        return false;
    };
    ui.heading(&export.object);
    ui.label(
        RichText::new(export.class.as_deref().unwrap_or("Unknown class")).color(subtle_dark()),
    );
    ui.add_space(6.0);
    match &mut export.decoded {
        Ok(decoded) => match &mut decoded.block {
            ExportBlock::Reflected(block) => {
                ui.label(
                    RichText::new(
                        "Filled circle = editable scalar; outlined values are preserved read-only.",
                    )
                    .small()
                    .color(subtle_dark()),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        draw_chimp_property_block(ui, block, &mut document.header.name_map, 0)
                    })
                    .inner
            }
            ExportBlock::NotSerialized => {
                ui.label("This class has no reflected property block.");
                ui.label("Its native payload is preserved byte-for-byte.");
                false
            }
            ExportBlock::Unreflected(block) => {
                ui.label("Reflection data for this class is not available.");
                ui.label(format!(
                    "{} untyped bytes are preserved byte-for-byte.",
                    block.rest.len()
                ));
                false
            }
        },
        Err(error) => {
            ui.colored_label(Color32::from_rgb(210, 120, 80), error);
            ui.label("The raw export remains available for extraction and is never rewritten.");
            false
        }
    }
}

fn draw_chimp_property_block(
    ui: &mut Ui,
    block: &mut PropertyBlock,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    depth: usize,
) -> bool {
    let mut changed = false;
    for (index, entry) in block.entries.iter_mut().enumerate() {
        let id = ui.make_persistent_id((depth, index, entry.name.as_ref()));
        ui.horizontal(|ui| {
            ui.set_min_height(24.0);
            ui.label(RichText::new(entry.name.as_ref()).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                changed |= draw_chimp_value(ui, id, &mut entry.value, names, depth);
            });
        });
        ui.separator();
    }
    changed
}

fn draw_chimp_value(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut PropValue,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    depth: usize,
) -> bool {
    match value {
        PropValue::Bool(value) => ui.checkbox(value, "").changed(),
        PropValue::Int(value) => ui.add(egui::DragValue::new(value)).changed(),
        PropValue::Float(value) => ui.add(egui::DragValue::new(value).speed(0.01)).changed(),
        PropValue::Str(value) => {
            let mut text = value.to_string();
            let changed = ui
                .add(egui::TextEdit::singleline(&mut text).id(id))
                .changed();
            if changed {
                *value = text.into();
            }
            changed
        }
        PropValue::Name(value) => {
            let mut text = value.to_string();
            let changed = ui
                .add(egui::TextEdit::singleline(&mut text).id(id))
                .changed();
            if changed {
                *value = blam_tags::iostore::object::edit::intern_name(names, &text);
            }
            changed
        }
        PropValue::Struct(block) => {
            egui::CollapsingHeader::new(format!("Struct ({} properties)", block.len()))
                .id_salt(id)
                .show(ui, |ui| {
                    draw_chimp_property_block(ui, block, names, depth + 1)
                })
                .body_returned
                .unwrap_or(false)
        }
        PropValue::Array(values) | PropValue::Set(values) => {
            ui.label(format!("{} values (read-only)", values.len()));
            false
        }
        PropValue::Map(values) => {
            ui.label(format!("{} pairs (read-only)", values.len()));
            false
        }
        PropValue::Raw(bytes) => {
            ui.label(format!("{} unknown bytes (preserved)", bytes.len()));
            false
        }
        other => {
            ui.label(format!("{other:?}"));
            false
        }
    }
}

fn chimp_document_json(document: &ChimpDocument) -> Value {
    json!({
        "package": document.package,
        "provider": document.provider.entry_path,
        "imports": document.header.imported_package_names,
        "exports": document.exports.iter().map(|export| {
            json!({
                "object": export.object,
                "class": export.class,
                "properties": export.decoded.as_ref().ok()
                    .and_then(Export::properties)
                    .map(chimp_block_json),
                "decode_error": export.decoded.as_ref().err(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn chimp_block_json(block: &PropertyBlock) -> Value {
    Value::Object(
        block
            .iter()
            .map(|(name, value)| (name.to_owned(), chimp_value_json(value)))
            .collect(),
    )
}

fn chimp_value_json(value: &PropValue) -> Value {
    match value {
        PropValue::Bool(value) => json!(value),
        PropValue::Int(value) => json!(value),
        PropValue::Float(value) => json!(value),
        PropValue::Name(value) => json!(value.to_string()),
        PropValue::Str(value) => json!(value.to_string()),
        PropValue::Object(value) => json!({"object_index": value}),
        PropValue::SoftObject(value) => json!({
            "package": value.package.to_string(),
            "asset": value.asset.to_string(),
            "sub_path": value.sub_path.to_string(),
        }),
        PropValue::Array(values) | PropValue::Set(values) => {
            Value::Array(values.iter().map(chimp_value_json).collect())
        }
        PropValue::Map(values) => Value::Array(
            values
                .iter()
                .map(|(key, value)| {
                    json!({
                        "key": chimp_value_json(key),
                        "value": chimp_value_json(value),
                    })
                })
                .collect(),
        ),
        PropValue::Struct(block) => chimp_block_json(block),
        PropValue::Raw(bytes) => json!({"unknown_bytes": bytes.len()}),
        other => json!({"type": format!("{other:?}")}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chimp_is_idle_and_unfiltered_by_default() {
        let state = ChimpState::default();
        assert!(matches!(state.mount, ChimpMount::Idle));
        assert_eq!(state.browser, ChimpBrowser::Folders);
        assert!(state.filter.is_empty());
        assert!(state.open_packages.is_empty());
        assert!(
            !state.filter_is_current(""),
            "the initial empty query must populate the browser once"
        );
    }

    #[test]
    fn package_tree_groups_every_path_segment_and_counts_descendants() {
        let mut tree = ChimpFolderNode::default();
        tree.insert_package(0, "/Game/UI/Menu");
        tree.insert_package(1, "/Game/UI/Hud");
        tree.insert_package(2, "/Engine/Config");
        tree.insert_file(0, "../../../Meteorite/Content/Audio/menu.bnk");
        assert_eq!(tree.package_count, 3);
        assert_eq!(tree.file_count, 1);
        assert_eq!(tree.entry_count(), 4);
        let game = tree.folders.get("Game").unwrap();
        assert_eq!(game.package_count, 2);
        let ui = game.folders.get("UI").unwrap();
        assert_eq!(ui.package_count, 2);
        assert_eq!(
            ui.packages
                .iter()
                .map(|leaf| leaf.name.as_str())
                .collect::<Vec<_>>(),
            ["Menu", "Hud"]
        );
        let engine = &tree.folders["Engine"];
        assert_eq!(engine.package_count, 1);
        assert_eq!(engine.packages[0].name, "Config");
        assert_eq!(
            tree.folders["Meteorite"].folders["Content"].folders["Audio"].files[0].name,
            "menu.bnk"
        );
    }

    #[test]
    fn raw_values_are_identified_without_embedding_binary_in_json() {
        assert_eq!(
            chimp_value_json(&PropValue::Raw(vec![1, 2, 3])),
            json!({"unknown_bytes": 3})
        );
    }

    #[test]
    fn output_triplet_replacement_replaces_all_files_and_cleans_backups() {
        let directory = std::env::temp_dir().join(format!("baboon-chimp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let incoming = directory.join("incoming.utoc");
        let output = directory.join("Chimp_P.utoc");
        for file in triplet(&incoming) {
            std::fs::write(file, b"new").unwrap();
        }
        for file in triplet(&output) {
            std::fs::write(file, b"old").unwrap();
        }
        replace_chimp_triplet(&incoming, &output).unwrap();
        for file in triplet(&output) {
            assert_eq!(std::fs::read(file).unwrap(), b"new");
        }
        assert!(!output.with_extension("utoc.previous").exists());
        assert!(!output.with_extension("ucas.previous").exists());
        assert!(!output.with_extension("pak.previous").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recovery_manifest_round_trips_package_files() {
        let manifest = ChimpRecoveryManifest {
            source: "Paks".to_owned(),
            packages: HashMap::from([("/Game/UI/Probe".to_owned(), "012345.uasset".to_owned())]),
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let restored: ChimpRecoveryManifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.source, "Paks");
        assert_eq!(
            restored.packages.get("/Game/UI/Probe").map(String::as_str),
            Some("012345.uasset")
        );
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_package_rebuilds_into_a_readable_overlay() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let mut browser = ChimpState::default();
        browser.refresh_filter(&world);
        assert_eq!(browser.filtered_packages.len(), world.packages().len());
        assert_eq!(browser.filtered_files.len(), world.pak_files().len());
        assert_eq!(
            browser.content_tree.package_count,
            browser.filtered_packages.len()
        );
        assert_eq!(
            browser.content_tree.file_count,
            browser.filtered_files.len()
        );
        let container = world
            .containers()
            .iter()
            .find(|container| container.package_count > 0)
            .expect("an IoStore container with packages");
        browser.selected_archive = Some(ChimpArchive::IoStore(container.index));
        browser.reset_filter();
        browser.refresh_filter(&world);
        assert!(!browser.filtered_packages.is_empty());
        assert!(browser.filtered_packages.iter().all(|&index| {
            world.packages()[index]
                .providers
                .iter()
                .any(|provider| provider.container == container.index)
        }));
        let pak = world
            .pak_containers()
            .iter()
            .find(|container| container.file_count > 0)
            .expect("a legacy pak with files");
        browser.selected_archive = Some(ChimpArchive::Pak(pak.index));
        browser.reset_filter();
        browser.refresh_filter(&world);
        assert!(browser.filtered_packages.is_empty());
        assert!(!browser.filtered_files.is_empty());
        assert!(browser.filtered_files.iter().all(|&index| {
            world.pak_files()[index]
                .providers
                .iter()
                .any(|provider| provider.container == pak.index)
        }));
        assert!(
            !world.pak_files().is_empty(),
            "the real mount should index legacy .pak files too"
        );
        let package = world
            .packages()
            .iter()
            .find(|package| package.name.starts_with("/Game/"))
            .expect("a /Game package")
            .name
            .clone();
        let document = load_chimp_document(&world, &package).unwrap();
        let (bytes, store) = rebuild_chimp_document(&world, &document).unwrap();
        FZenPackageHeader::deserialize(
            &mut Cursor::new(&bytes),
            Some(store.clone()),
            CE_TOC_VERSION,
            CE_HEADER_VERSION,
            None,
        )
        .expect("rebuilt package parses");

        let directory = std::env::temp_dir().join(format!("baboon-chimp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("ChimpTest_P.utoc");
        let override_ = PackageOverride {
            archive: &world.archives()[document.provider.container],
            uasset_path: &document.provider.entry_path,
            bytes: bytes.clone(),
            store,
        };
        write_package_mod_container(&[override_], &output).unwrap();
        let mut overlay = blam_tags::iostore::IoStoreArchive::open(&output).unwrap();
        let bases: Vec<&blam_tags::iostore::IoStoreArchive> = world.archives().iter().collect();
        overlay.recover_entries(&bases, Some("Meteorite/Content/"));
        assert_eq!(overlay.read(&document.provider.entry_path).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
