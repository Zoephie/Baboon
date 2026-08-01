//! Chimp: the Campaign Evolved Unreal package workspace.
//!
//! Chimp is deliberately scoped to a loaded Campaign Evolved kit. It shares
//! that kit's Paks root but owns its own package index, documents and editor
//! state; none of those concepts are forced through the editing-kit/tag model.

use super::*;
use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use blam_tags::iostore::asset::texture2d::decode_texture2d_preview;
use blam_tags::iostore::container::writer::{
    PackageOverride, PackageReplacement, overwrite_package_in_place_with,
    overwrite_packages_in_place_with, write_package_mod_container,
};
use blam_tags::iostore::object::archive::ExportContext;
use blam_tags::iostore::object::edit::{
    default_value_for_type, editable_schema_slots, property_type_for_slot, set_property_slot,
    validate_value_for_type,
};
use blam_tags::iostore::object::export::{Export, ExportBlock, read_export_in, write_export_in};
use blam_tags::iostore::object::hand_written as chimp_hw;
use blam_tags::iostore::object::native::{NativeStruct, PerPlatformValue};
use blam_tags::iostore::object::tail_models::{TailContext, parse_texture_chain_tail};
use blam_tags::iostore::object::usmap::PropertyType;
use blam_tags::iostore::object::value::{PropValue, PropertyBlock};
use blam_tags::iostore::package::builder::{read_payloads, write_package};
use blam_tags::iostore::package::ue_types::{FPackageObjectIndex, FPackageObjectIndexType};
use blam_tags::iostore::package::zen::FZenPackageHeader;
use blam_tags::iostore::skeletal_mesh::SkeletalMesh;
use blam_tags::iostore::static_mesh::StaticMesh;
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
    Groups,
    Archives,
    Packages,
    Files,
}

impl KitSurface {
    pub(super) const TABS: [(Self, &'static str, &'static str); 2] = [
        (Self::Tags, "Tags", "Browse and edit Halo tags"),
        (
            Self::Chimp,
            "Chimp",
            "Browse and edit Unreal Engine packages",
        ),
    ];
}

impl ChimpBrowser {
    const TABS: [(Self, &'static str); 5] = [
        (Self::Folders, "Folders"),
        (Self::Groups, "Groups"),
        (Self::Files, "Pak files"),
        (Self::Archives, "Archives"),
        (Self::Packages, "Packages"),
    ];
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpDocumentView {
    #[default]
    Document,
    Texture,
    Mesh,
    Properties,
    Metadata,
}

enum ChimpTreeClick {
    Package(String),
    ExtractTexture(String),
    ExtractMesh(String, ChimpMeshFormat),
    File(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChimpMeshKind {
    Skeletal,
    Static,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChimpMeshFormat {
    Jms,
    Psk,
    Pskx,
}

impl ChimpMeshFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Jms => "jms",
            Self::Psk => "psk",
            Self::Pskx => "pskx",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Jms => "JMS",
            Self::Psk => "ActorX PSK",
            Self::Pskx => "ActorX PSKX",
        }
    }
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
    filtered_archive_for: Option<Option<ChimpArchive>>,
    filtered_packages: Vec<usize>,
    filtered_files: Vec<usize>,
    filtered_groups: BTreeMap<String, Vec<usize>>,
    content_tree: ChimpFolderNode,
    selected_archive: Option<ChimpArchive>,
    package_types: Vec<Option<String>>,
    type_indexing: bool,
    folder_selection: ChimpFolderSelection,
    pub(super) selected_package: Option<String>,
    selected_file: Option<String>,
    pub(super) open_packages: Vec<String>,
    document_tree: Option<egui_tiles::Tree<String>>,
    pub(super) documents: HashMap<String, ChimpDocument>,
    pub(super) loading_packages: HashSet<String>,
    pending_overwrite: Option<String>,
    pending_overwrite_skip_future: bool,
    save_dialog: Option<ChimpSaveDialog>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpSaveMode {
    #[default]
    ExportMod,
    OverwriteSources,
}

struct ChimpSaveDialog {
    mode: ChimpSaveMode,
    name: String,
    folder: PathBuf,
    overwrite_acknowledged: bool,
}

enum ChimpSaveAction {
    Export(PathBuf),
    Overwrite,
}

pub(super) struct ChimpDocument {
    pub(super) package: String,
    pub(super) provider: PackageProvider,
    pub(super) original: Vec<u8>,
    pub(super) header: FZenPackageHeader,
    pub(super) payloads: Vec<Vec<u8>>,
    pub(super) exports: Vec<ChimpExport>,
    texture_previews: Vec<ChimpTexturePreview>,
    mesh_kind: Option<ChimpMeshKind>,
    mesh_preview: Option<Result<ModelPreviewData, String>>,
    mesh_preview_state: ModelPreviewState,
    pub(super) selected_export: usize,
    pub(super) dirty: bool,
    view: ChimpDocumentView,
    document_text: String,
    document_line_numbers: String,
    document_text_dirty: bool,
    metadata_text: String,
    metadata_line_numbers: String,
    metadata_text_dirty: bool,
}

pub(super) struct ChimpTypeIndex {
    package_types: Vec<Option<String>>,
    type_counts: BTreeMap<String, usize>,
    failures: usize,
}

struct ChimpTexturePreview {
    export_index: usize,
    preview: BitmapPreviewState,
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

struct ChimpPaneBehavior<'a> {
    app: &'a mut Baboon,
    kit_index: usize,
    close_requests: Vec<String>,
    focused: Option<String>,
    close_all: bool,
    close_all_but: Option<String>,
    extract_texture: Option<String>,
    extract_mesh: Option<(String, ChimpMeshFormat)>,
}

impl egui_tiles::Behavior<String> for ChimpPaneBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        tile_id: egui_tiles::TileId,
        pane: &mut String,
    ) -> egui_tiles::UiResponse {
        if ui.input(|input| input.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect())
        {
            self.focused = Some(pane.clone());
        }
        self.app.draw_chimp_document_pane(
            ui,
            self.kit_index,
            pane,
            &format!("chimp_tile_{}", tile_id.0),
        );
        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &String) -> egui::WidgetText {
        let dirty = self.app.kits[self.kit_index]
            .chimp
            .documents
            .get(pane)
            .is_some_and(|document| document.dirty);
        let label = pane.rsplit('/').next().unwrap_or(pane);
        RichText::new(if dirty {
            format!("• {label}")
        } else {
            label.to_owned()
        })
        .color(text_dark())
        .into()
    }

    fn is_tab_closable(
        &self,
        _tiles: &egui_tiles::Tiles<String>,
        _tile_id: egui_tiles::TileId,
    ) -> bool {
        true
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        if let Some(egui_tiles::Tile::Pane(package)) = tiles.get(tile_id) {
            self.close_requests.push(package.clone());
        }
        false
    }

    fn on_tab_button(
        &mut self,
        tiles: &egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        let Some(egui_tiles::Tile::Pane(package)) = tiles.get(tile_id) else {
            return button_response;
        };
        let package = package.clone();
        if button_response.clicked() {
            self.focused = Some(package.clone());
        }
        if button_response.middle_clicked() {
            self.close_requests.push(package.clone());
        }
        let has_texture = self.app.kits[self.kit_index]
            .chimp
            .documents
            .get(&package)
            .is_some_and(|document| !document.texture_previews.is_empty());
        let has_mesh = self.app.kits[self.kit_index]
            .chimp
            .documents
            .get(&package)
            .is_some_and(|document| document.mesh_kind.is_some());
        button_response.context_menu(|ui| {
            if ui.button("Close").clicked() {
                self.close_requests.push(package.clone());
                ui.close_menu();
            }
            if ui.button("Close all but this").clicked() {
                self.close_all_but = Some(package.clone());
                ui.close_menu();
            }
            if ui.button("Close all").clicked() {
                self.close_all = true;
                ui.close_menu();
            }
            if has_texture {
                ui.separator();
                if ui.button("Extract Texture2D as TIFF…").clicked() {
                    self.extract_texture = Some(package.clone());
                    ui.close_menu();
                }
            }
            if has_mesh {
                ui.separator();
                ui.menu_button("Extract mesh", |ui| {
                    for format in [
                        ChimpMeshFormat::Jms,
                        ChimpMeshFormat::Psk,
                        ChimpMeshFormat::Pskx,
                    ] {
                        if ui.button(format.label()).clicked() {
                            self.extract_mesh = Some((package.clone(), format));
                            ui.close_menu();
                        }
                    }
                });
            }
        });
        button_response
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        row_type()
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        tiles: &egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        let base = if state.active { menu_bar() } else { row_type() };
        let dirty = matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(package))
            if self.app.kits[self.kit_index]
                .chimp
                .documents
                .get(package)
                .is_some_and(|document| document.dirty));
        if dirty {
            chimp_tint_toward(base, Color32::from_rgb(184, 134, 11), 0.20)
        } else {
            base
        }
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<String>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> Color32 {
        text_dark()
    }
}

impl ChimpState {
    fn ensure_document_tree(&mut self, kit: KitId) -> &mut egui_tiles::Tree<String> {
        self.document_tree
            .get_or_insert_with(|| egui_tiles::Tree::empty(chimp_tree_id(kit)))
    }

    fn open_document_pane(&mut self, kit: KitId, package: &str) {
        let tree = self.ensure_document_tree(kit);
        let existing = tree.tiles.iter().find_map(|(id, tile)| match tile {
            egui_tiles::Tile::Pane(open) if open == package => Some(*id),
            _ => None,
        });
        if let Some(tile_id) = existing {
            tree.make_active(|id, _| id == tile_id);
        } else {
            let tile_id = tree.tiles.insert_pane(package.to_owned());
            match tree.root() {
                Some(root) => {
                    if let Some(egui_tiles::Tile::Container(container)) = tree.tiles.get_mut(root) {
                        container.add_child(tile_id);
                    } else {
                        let tabs = tree.tiles.insert_tab_tile(vec![root, tile_id]);
                        tree.root = Some(tabs);
                    }
                }
                None => tree.root = Some(tile_id),
            }
            tree.make_active(|id, _| id == tile_id);
        }
        self.selected_package = Some(package.to_owned());
        self.sync_open_packages();
    }

    fn close_document_pane(&mut self, package: &str) {
        if let Some(tree) = self.document_tree.as_mut() {
            let tile_id = tree.tiles.iter().find_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(open) if open == package => Some(*id),
                _ => None,
            });
            if let Some(tile_id) = tile_id {
                tree.remove_recursively(tile_id);
            }
        }
        self.sync_open_packages();
    }

    fn sync_open_packages(&mut self) {
        self.open_packages = self
            .document_tree
            .as_ref()
            .map(|tree| {
                tree.tiles
                    .tiles()
                    .filter_map(|tile| match tile {
                        egui_tiles::Tile::Pane(package) => Some(package.clone()),
                        egui_tiles::Tile::Container(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if self
            .selected_package
            .as_ref()
            .is_some_and(|package| !self.open_packages.contains(package))
        {
            self.selected_package = self.open_packages.first().cloned();
        }
    }

    fn filter_is_current(&self, query: &str) -> bool {
        self.filtered_for.as_deref() == Some(query)
            && self.filtered_archive_for == Some(self.selected_archive)
    }

    fn refresh_filter(&mut self, world: &World) {
        let query = self.filter.trim().to_ascii_lowercase();
        if self.filter_is_current(&query) {
            return;
        }
        let selected_archive = self.selected_archive;
        self.filtered_for = Some(query.clone());
        self.filtered_archive_for = Some(selected_archive);
        self.filtered_packages.clear();
        self.filtered_files.clear();
        self.filtered_groups.clear();
        self.filtered_packages.extend(
            world
                .packages()
                .iter()
                .enumerate()
                .filter(|(index, package)| {
                    let archive_matches = match selected_archive {
                        None => true,
                        Some(ChimpArchive::IoStore(container)) => package
                            .providers
                            .iter()
                            .any(|provider| provider.container == container),
                        Some(ChimpArchive::Pak(_)) => false,
                    };
                    let type_matches = self
                        .package_types
                        .get(*index)
                        .and_then(Option::as_deref)
                        .is_some_and(|kind| chimp_contains_query(kind, &query));
                    archive_matches
                        && (query.is_empty()
                            || type_matches
                            || chimp_contains_query(&package.name, &query)
                            || package.providers.iter().any(|provider| {
                                chimp_contains_query(
                                    &world.containers()[provider.container]
                                        .path
                                        .to_string_lossy(),
                                    &query,
                                )
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
                            || chimp_contains_query(&file.path, &query)
                            || file.providers.iter().any(|provider| {
                                chimp_contains_query(
                                    &world.pak_containers()[provider.container]
                                        .path
                                        .to_string_lossy(),
                                    &query,
                                )
                            }))
                })
                .map(|(index, _)| index),
        );
        self.content_tree = ChimpFolderNode::default();
        for &index in &self.filtered_packages {
            self.content_tree
                .insert_package(index, &world.packages()[index].name);
            self.filtered_groups
                .entry(
                    self.package_types
                        .get(index)
                        .and_then(Option::as_deref)
                        .unwrap_or("Unknown")
                        .to_owned(),
                )
                .or_default()
                .push(index);
        }
        for &index in &self.filtered_files {
            self.content_tree
                .insert_file(index, &world.pak_files()[index].path);
        }
    }

    fn reset_filter(&mut self) {
        self.filtered_for = None;
        self.filtered_archive_for = None;
        self.filtered_packages.clear();
        self.filtered_files.clear();
        self.filtered_groups.clear();
        self.content_tree = ChimpFolderNode::default();
    }
}

fn chimp_tree_id(kit: KitId) -> egui::Id {
    egui::Id::new(("chimp_document_tree", kit.0))
}

fn chimp_contains_query(value: &str, query: &str) -> bool {
    let needle = query.as_bytes();
    needle.is_empty()
        || value
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

fn chimp_tint_toward(base: Color32, accent: Color32, amount: f32) -> Color32 {
    let mix =
        |base: u8, accent: u8| (base as f32 + (accent as f32 - base as f32) * amount).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(base.r(), accent.r()),
        mix(base.g(), accent.g()),
        mix(base.b(), accent.b()),
        base.a(),
    )
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
    let exports: Vec<ChimpExport> = header
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
    let texture_previews = decode_chimp_texture_previews(
        world, &provider, &header, &payloads, &names, &resolver, &bulk, &exports,
    );
    let (mesh_kind, mesh_preview, mesh_preview_state) =
        decode_chimp_mesh_preview(world, &provider, &bytes, &header, &exports);
    let initial_view = if !texture_previews.is_empty() {
        ChimpDocumentView::Texture
    } else if mesh_kind.is_some() {
        ChimpDocumentView::Mesh
    } else {
        ChimpDocumentView::default()
    };
    let mut document = ChimpDocument {
        package: header.package_name(),
        provider,
        original: bytes,
        header,
        payloads,
        exports,
        texture_previews,
        mesh_kind,
        mesh_preview,
        mesh_preview_state,
        selected_export: 0,
        dirty: false,
        view: initial_view,
        document_text: String::new(),
        document_line_numbers: String::new(),
        document_text_dirty: true,
        metadata_text: String::new(),
        metadata_line_numbers: String::new(),
        metadata_text_dirty: true,
    };
    refresh_chimp_document_text(&mut document);
    refresh_chimp_metadata_text(&mut document, world);
    Ok(document)
}

fn chimp_material_names(header: &FZenPackageHeader) -> Vec<String> {
    header
        .imported_package_names
        .iter()
        .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
        .filter(|name| name.starts_with("MI_") || name.starts_with("M_"))
        .collect()
}

fn decode_chimp_mesh_preview(
    world: &World,
    provider: &PackageProvider,
    bytes: &[u8],
    header: &FZenPackageHeader,
    exports: &[ChimpExport],
) -> (
    Option<ChimpMeshKind>,
    Option<Result<ModelPreviewData, String>>,
    ModelPreviewState,
) {
    let kind = if exports
        .iter()
        .any(|export| export.class.as_deref() == Some("SkeletalMesh"))
    {
        Some(ChimpMeshKind::Skeletal)
    } else if exports
        .iter()
        .any(|export| export.class.as_deref() == Some("StaticMesh"))
    {
        Some(ChimpMeshKind::Static)
    } else {
        None
    };
    let preview = kind.map(|kind| {
        let header_size = header.summary.header_size as usize;
        let preview = match kind {
            ChimpMeshKind::Skeletal => {
                SkeletalMesh::from_package(bytes, &header.name_map.copy_raw_names(), header_size)
                    .map(chimp_skeletal_mesh_preview)
            }
            ChimpMeshKind::Static => {
                let bulk = world
                    .archives()
                    .get(provider.container)
                    .and_then(|archive| {
                        let chunk = archive.chunk_index_for(&provider.entry_path).ok()?;
                        archive.read_bulk_for(chunk, 0).ok()
                    });
                StaticMesh::from_package_preferring_nanite(bytes, header_size, bulk.as_deref())
                    .map(chimp_static_mesh_preview)
            }
        }
        .map_err(|error| format!("Could not decode mesh geometry: {error:#}"))?;
        Ok(model_preview::standalone_mesh_preview(
            header.package_name(),
            preview,
        ))
    });
    let mut state = ModelPreviewState::default();
    if kind.is_some() {
        state.show_backfaces = true;
        state.region_selections.insert(
            "mesh".to_owned(),
            ModelRegionSelection {
                enabled: true,
                permutation: "default".to_owned(),
            },
        );
    }
    (kind, preview, state)
}

fn chimp_skeletal_mesh_preview(mesh: SkeletalMesh) -> RenderModelPreview {
    let mut preview = chimp_mesh_preview_base(
        mesh.vertices
            .iter()
            .map(|vertex| (vertex.position, vertex.normal)),
        mesh.indices,
    );
    for section in mesh.sections {
        let index_start = section.base_index.min(preview.indices.len() as u32);
        let index_count =
            (section.num_triangles * 3).min(preview.indices.len() as u32 - index_start);
        if index_count > 0 {
            preview.batches.push(RenderModelPreviewBatch {
                region_name: "mesh".to_owned(),
                permutation_name: "default".to_owned(),
                material_index: section.material_index,
                index_start,
                index_count,
            });
        }
    }
    if preview.batches.is_empty() && !preview.indices.is_empty() {
        preview.batches.push(RenderModelPreviewBatch {
            region_name: "mesh".to_owned(),
            permutation_name: "default".to_owned(),
            material_index: 0,
            index_start: 0,
            index_count: preview.indices.len() as u32,
        });
    }
    preview
}

fn chimp_static_mesh_preview(mesh: StaticMesh) -> RenderModelPreview {
    let mut preview = chimp_mesh_preview_base(
        mesh.vertices
            .iter()
            .map(|vertex| (vertex.position, vertex.normal)),
        mesh.indices,
    );
    if !preview.indices.is_empty() {
        preview.batches.push(RenderModelPreviewBatch {
            region_name: "mesh".to_owned(),
            permutation_name: "default".to_owned(),
            material_index: 0,
            index_start: 0,
            index_count: preview.indices.len() as u32,
        });
    }
    preview
}

fn chimp_mesh_preview_base(
    vertices: impl IntoIterator<Item = ([f32; 3], [f32; 3])>,
    indices: Vec<u32>,
) -> RenderModelPreview {
    let mut preview = RenderModelPreview {
        regions: vec![RenderModelPreviewRegion {
            name: "mesh".to_owned(),
            permutations: vec!["default".to_owned()],
        }],
        indices,
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };
    for (position, normal) in vertices {
        for axis in 0..3 {
            preview.bounds_min[axis] = preview.bounds_min[axis].min(position[axis]);
            preview.bounds_max[axis] = preview.bounds_max[axis].max(position[axis]);
        }
        preview
            .vertices
            .push(RenderModelPreviewVertex { position, normal });
    }
    if preview.vertices.is_empty() {
        preview.bounds_min = [-1.0; 3];
        preview.bounds_max = [1.0; 3];
    }
    preview
}

fn decode_chimp_texture_previews(
    world: &World,
    provider: &PackageProvider,
    header: &FZenPackageHeader,
    payloads: &[Vec<u8>],
    names: &[String],
    resolver: &dyn blam_tags::iostore::object::archive::PackageResolver,
    bulk: &[(i64, i64)],
    exports: &[ChimpExport],
) -> Vec<ChimpTexturePreview> {
    let archive = &world.archives()[provider.container];
    let package_chunk = archive.chunk_index_for(&provider.entry_path);
    exports
        .iter()
        .enumerate()
        .filter(|(_, export)| export.class.as_deref() == Some("Texture2D"))
        .map(|(export_index, export)| {
            let decoded = (|| {
                let package_entry = header
                    .export_map
                    .get(export_index)
                    .ok_or_else(|| "Texture export is missing from the package map".to_owned())?;
                let payload = payloads
                    .get(export_index)
                    .ok_or_else(|| "Texture export payload is missing".to_owned())?;
                let export = export.decoded.as_ref().map_err(Clone::clone)?;
                let context = TailContext {
                    bulk_data: bulk,
                    origin: payload.len().saturating_sub(export.tail.len()),
                    usmap: world.usmap(),
                    resolver: Some(resolver),
                    object_flags: package_entry.object_flags,
                };
                let texture = parse_texture_chain_tail(&export.tail, names, context, true)
                    .map_err(|error| error.to_string())?;
                let package_chunk = package_chunk.as_ref().map_err(|error| error.to_string())?;
                decode_texture2d_preview(&texture, |bulk_index| {
                    let entry = header
                        .bulk_data
                        .get(bulk_index.max(0) as usize)
                        .ok_or_else(|| {
                            anyhow::anyhow!("bulk-data index {bulk_index} is out of range")
                        })?;
                    let chunk = archive.read_bulk_for(*package_chunk, entry.cooked_index as u16)?;
                    let start = usize::try_from(entry.serial_offset)
                        .map_err(|_| anyhow::anyhow!("negative bulk-data offset"))?;
                    let size = usize::try_from(entry.serial_size)
                        .map_err(|_| anyhow::anyhow!("negative bulk-data size"))?;
                    chunk
                        .get(start..start.saturating_add(size))
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| {
                            anyhow::anyhow!("bulk-data entry lies outside its sibling chunk")
                        })
                })
                .map_err(|error| error.to_string())
            })();
            let mut preview = BitmapPreviewState::default();
            preview.decoded = Some(decoded.map(|decoded| BitmapPreviewData {
                width: decoded.width,
                height: decoded.height,
                image_count: 1,
                mip_count: 1,
                format_name: decoded.pixel_format,
                type_name: format!("Texture2D • mip {}", decoded.mip_level),
                rgba: decoded.rgba8,
            }));
            ChimpTexturePreview {
                export_index,
                preview,
            }
        })
        .collect()
}

fn chimp_package_type(world: &World, header: &FZenPackageHeader) -> Option<String> {
    let package_name = header.package_name();
    let package_leaf = package_name.rsplit('/').next().unwrap_or_default();
    let classify = |index: usize| {
        let entry = header.export_map.get(index)?;
        let class = world.class_key(header, entry.class_index)?;
        if class.contains('/') || class.contains('#') {
            return None;
        }
        Some(if class.contains("Blueprint") {
            "Blueprint".to_owned()
        } else {
            class
        })
    };
    let primary = header
        .export_map
        .iter()
        .enumerate()
        .find(|(_, export)| {
            export.outer_index.is_null() && header.name_map.get(export.object_name) == package_leaf
        })
        .and_then(|(index, _)| classify(index));
    primary.or_else(|| {
        let roots: Vec<usize> = header
            .export_map
            .iter()
            .enumerate()
            .filter(|(_, export)| export.outer_index.is_null())
            .map(|(index, _)| index)
            .collect();
        roots
            .iter()
            .filter_map(|index| classify(*index))
            .find(|class| class == "Blueprint")
            .or_else(|| roots.into_iter().find_map(classify))
    })
}

fn index_chimp_package_types(world: &World) -> ChimpTypeIndex {
    // Most Zen package headers fit in one 64 KiB IoStore compression block.
    // Only retry with the former 1 MiB window (and finally the whole package)
    // for the uncommon large header. This avoids decompressing sixteen blocks
    // for every package while preserving the existing fallback behavior.
    const HEADER_PREFIXES: [usize; 2] = [64 * 1024, 1024 * 1024];
    index_chimp_package_types_with_prefixes(world, &HEADER_PREFIXES)
}

fn index_chimp_package_types_with_prefixes(
    world: &World,
    header_prefixes: &[usize],
) -> ChimpTypeIndex {
    let mut package_types = Vec::with_capacity(world.packages().len());
    let mut type_counts = BTreeMap::new();
    let mut failures = 0usize;
    for package in world.packages() {
        let result = (|| {
            let provider = package.active_provider()?;
            let archive = world.archives().get(provider.container)?;
            let mut header = None;
            for &max_bytes in header_prefixes {
                let prefix = archive.read_prefix(&provider.entry_path, max_bytes).ok()?;
                if let Ok(decoded) = FZenPackageHeader::deserialize(
                    &mut Cursor::new(&prefix),
                    None,
                    CE_TOC_VERSION,
                    CE_HEADER_VERSION,
                    None,
                ) {
                    header = Some(decoded);
                    break;
                }
                if prefix.len() < max_bytes {
                    break;
                }
            }
            let header = header.or_else(|| {
                world.read_provider(provider).ok().and_then(|bytes| {
                    FZenPackageHeader::deserialize(
                        &mut Cursor::new(bytes),
                        None,
                        CE_TOC_VERSION,
                        CE_HEADER_VERSION,
                        None,
                    )
                    .ok()
                })
            })?;
            chimp_package_type(world, &header)
        })();
        if let Some(class) = &result {
            *type_counts.entry(class.clone()).or_insert(0) += 1;
        } else {
            failures += 1;
        }
        package_types.push(result);
    }
    ChimpTypeIndex {
        package_types,
        type_counts,
        failures,
    }
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
        if let ExportBlock::Reflected(block) = &decoded.block {
            validate_chimp_property_block(class, block, world.usmap()).map_err(|error| {
                format!(
                    "Could not validate {} export {}: {error}",
                    document.package, export.object
                )
            })?;
        }
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

fn validate_chimp_property_block(
    class: &str,
    block: &PropertyBlock,
    usmap: &Usmap,
) -> Result<(), String> {
    for entry in &block.entries {
        let Some(slot) = entry.slot else {
            continue;
        };
        let ty = property_type_for_slot(class, slot, usmap).map_err(|error| error.to_string())?;
        validate_value_for_type(&ty, &entry.value)
            .map_err(|error| format!("{}: {error}", entry.name))?;
        if let (PropertyType::Struct(nested), PropValue::Struct(value)) = (&ty, &entry.value) {
            validate_chimp_property_block(nested, value, usmap)?;
        }
    }
    Ok(())
}

fn load_chimp_usmap(path: Option<&Path>) -> Result<Usmap, String> {
    let Some(path) = path else {
        return Usmap::meteorite()
            .map_err(|error| format!("Could not parse bundled Campaign Evolved USMAP: {error:#}"));
    };
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not read USMAP {}: {error}", path.display()))?;
    Usmap::parse(&bytes)
        .map_err(|error| format!("Could not parse USMAP {}: {error:#}", path.display()))
}

impl Baboon {
    pub(super) fn apply_chimp_usmap_path(&mut self, path: Option<PathBuf>, ctx: egui::Context) {
        if let Err(error) = load_chimp_usmap(path.as_deref()) {
            self.status = error;
            return;
        }
        if self
            .kits
            .iter()
            .any(|kit| kit.chimp.documents.values().any(|document| document.dirty))
        {
            self.status =
                "Build or discard modified Chimp packages before changing the USMAP.".to_owned();
            return;
        }

        self.chimp_usmap_path = path;
        self.chimp_usmap_path_input = self
            .chimp_usmap_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let remount: Vec<usize> = self
            .kits
            .iter()
            .enumerate()
            .filter_map(|(index, kit)| {
                matches!(
                    kit.source.as_ref().map(|source| &source.source),
                    Some(TagSource::IoStoreContainerSet { .. })
                )
                .then_some(index)
            })
            .collect();
        for &index in &remount {
            self.kits[index].chimp = ChimpState::default();
            self.begin_chimp_mount(index, ctx.clone());
        }
        self.status = match &self.chimp_usmap_path {
            Some(path) if remount.is_empty() => {
                format!("Chimp USMAP set to {}", path.display())
            }
            Some(path) => format!("Chimp USMAP set to {}; remounting Chimp", path.display()),
            None if remount.is_empty() => {
                "Chimp will use the bundled Campaign Evolved USMAP".to_owned()
            }
            None => "Using the bundled Campaign Evolved USMAP; remounting Chimp".to_owned(),
        };
    }

    pub(super) fn commit_chimp_usmap_path_input(&mut self, ctx: egui::Context) {
        let trimmed = self.chimp_usmap_path_input.trim();
        let path = (!trimmed.is_empty()).then(|| PathBuf::from(trimmed));
        self.apply_chimp_usmap_path(path, ctx);
    }

    pub(super) fn choose_chimp_usmap_path(&mut self, ctx: egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Select Chimp USMAP")
            .add_filter("Unreal mappings", &["usmap"]);
        if let Some(directory) = self
            .chimp_usmap_path
            .as_ref()
            .and_then(|path| path.parent())
            .filter(|path| path.is_dir())
        {
            dialog = dialog.set_directory(directory);
        }
        if let Some(path) = dialog.pick_file() {
            self.apply_chimp_usmap_path(Some(path), ctx);
        }
    }

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
        let usmap_path = self.chimp_usmap_path.clone();
        self.kits[kit_index].chimp.mount = ChimpMount::Loading;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = (|| {
                let usmap = load_chimp_usmap(usmap_path.as_deref())?;
                // Keep startup to container discovery + the lightweight package
                // index. Generated Blueprint schema recovery is intentionally
                // lazy/future work; doing the whole corpus here would leave the
                // workspace saying "loading" while reading every package.
                let world = World::open(&root, usmap).map_err(|error| error.to_string())?;
                Ok(Arc::new(world))
            })();
            let mounted = result.as_ref().ok().cloned();
            let _ = tx.send(WorkerMessage::ChimpMounted { stamp, result });
            if let Some(world) = mounted {
                let index = index_chimp_package_types(&world);
                let _ = tx.send(WorkerMessage::ChimpTypesIndexed { stamp, index });
            }
            ctx.request_repaint();
        });
    }

    pub(super) fn handle_chimp_mounted(
        &mut self,
        stamp: KitStamp,
        result: Result<Arc<World>, String>,
        ctx: egui::Context,
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
                self.kits[index].chimp.type_indexing = true;
                self.kits[index].chimp.package_types.clear();
                self.status = if diagnostics == 0 {
                    format!("Chimp indexed {packages} Unreal packages and {files} pak files")
                } else {
                    format!(
                        "Chimp indexed {packages} Unreal packages and {files} pak files with {diagnostics} container warning(s)"
                    )
                };
                self.restore_chimp_recovery(index, &world);
                self.finish_pending_chimp_session_restore(index, ctx);
            }
            Err(error) => {
                self.kits[index].chimp.mount = ChimpMount::Failed(error.clone());
                self.status = format!("Chimp could not open: {error}");
            }
        }
        false
    }

    fn finish_pending_chimp_session_restore(&mut self, kit_index: usize, ctx: egui::Context) {
        let packages = std::mem::take(&mut self.kits[kit_index].pending_restore_chimp_packages);
        if packages.is_empty() {
            self.kits[kit_index].pending_restore_active_chimp_package = None;
            return;
        }
        let world = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => return,
        };
        let mut queued = 0usize;
        let mut missing = 0usize;
        for package in packages {
            if world.package(&package).is_none() {
                missing += 1;
                continue;
            }
            if self.kits[kit_index].documents_contains_chimp(&package) {
                let kit = self.kits[kit_index].id;
                self.kits[kit_index].chimp.open_document_pane(kit, &package);
            } else {
                self.begin_chimp_open_package(kit_index, package, ctx.clone());
            }
            queued += 1;
        }
        if self.kits[kit_index].chimp.loading_packages.is_empty()
            && let Some(active) = self.kits[kit_index]
                .pending_restore_active_chimp_package
                .take()
            && self.kits[kit_index].documents_contains_chimp(&active)
        {
            let kit = self.kits[kit_index].id;
            self.kits[kit_index].chimp.selected_package = Some(active.clone());
            self.kits[kit_index].chimp.open_document_pane(kit, &active);
        }
        if queued > 0 || missing > 0 {
            self.status = match (queued, missing) {
                (queued, 0) => format!("Reopening {queued} Chimp package(s)"),
                (0, missing) => format!("Could not find {missing} saved Chimp package(s)"),
                (queued, missing) => format!(
                    "Reopening {queued} Chimp package(s); {missing} saved package(s) are missing"
                ),
            };
        }
    }

    pub(super) fn handle_chimp_types_indexed(
        &mut self,
        stamp: KitStamp,
        type_index: ChimpTypeIndex,
    ) -> bool {
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        let classified = type_index
            .package_types
            .len()
            .saturating_sub(type_index.failures);
        let kinds = type_index.type_counts.len();
        let chimp = &mut self.kits[index].chimp;
        chimp.package_types = type_index.package_types;
        chimp.type_indexing = false;
        chimp.reset_filter();
        self.status =
            format!("Chimp classified {classified} packages into {kinds} Unreal file types");
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
            self.kits[kit_index]
                .chimp
                .documents
                .insert(package.clone(), document);
            let kit_id = self.kits[kit_index].id;
            self.kits[kit_index]
                .chimp
                .open_document_pane(kit_id, &package);
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
            let kit_id = self.kits[kit_index].id;
            self.kits[kit_index]
                .chimp
                .open_document_pane(kit_id, &package);
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
        self.kits[index].chimp.loading_packages.remove(&package);
        match result {
            Ok(document) => {
                self.kits[index]
                    .chimp
                    .documents
                    .insert(package.clone(), document);
                let kit_id = self.kits[index].id;
                self.kits[index].chimp.open_document_pane(kit_id, &package);
            }
            Err(error) => self.status = error,
        }
        if self.kits[index].chimp.loading_packages.is_empty()
            && let Some(active) = self.kits[index].pending_restore_active_chimp_package.take()
            && self.kits[index].documents_contains_chimp(&active)
        {
            let kit_id = self.kits[index].id;
            self.kits[index].chimp.selected_package = Some(active.clone());
            self.kits[index].chimp.open_document_pane(kit_id, &active);
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
        if self.kits[kit_index].chimp.browser == ChimpBrowser::Groups {
            self.draw_chimp_groups(ui, ctx, &world, kit_index);
            return;
        }
        let indices = self.kits[kit_index].chimp.filtered_packages.clone();
        let selected = self.kits[kit_index].chimp.selected_package.clone();
        let mut extract_texture = None;
        let mut extract_mesh = None;
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
                    let is_texture = self.kits[kit_index]
                        .chimp
                        .package_types
                        .get(indices[row])
                        .and_then(Option::as_deref)
                        == Some("Texture2D");
                    let is_mesh = matches!(
                        self.kits[kit_index]
                            .chimp
                            .package_types
                            .get(indices[row])
                            .and_then(Option::as_deref),
                        Some("SkeletalMesh" | "StaticMesh")
                    );
                    if is_texture || is_mesh {
                        response.context_menu(|ui| {
                            if is_texture && ui.button("Extract Texture2D as TIFF…").clicked() {
                                extract_texture = Some(package.name.clone());
                                ui.close_menu();
                            }
                            if is_mesh {
                                chimp_mesh_export_menu(ui, &package.name, &mut extract_mesh);
                            }
                        });
                    }
                    if response.clicked() {
                        self.begin_chimp_open_package(kit_index, package.name.clone(), ctx.clone());
                    }
                }
            });
        if let Some(package) = extract_texture {
            self.begin_extract_chimp_texture_tiff(kit_index, &package, ctx.clone());
        }
        if let Some((package, format)) = extract_mesh {
            self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
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
                                if kind == "Texture2D"
                                    || kind == "SkeletalMesh"
                                    || kind == "StaticMesh"
                                {
                                    response.context_menu(|ui| {
                                        if kind == "Texture2D"
                                            && ui.button("Extract Texture2D as TIFF…").clicked()
                                        {
                                            extract_texture = Some(package.name.clone());
                                            ui.close_menu();
                                        }
                                        if kind == "SkeletalMesh" || kind == "StaticMesh" {
                                            chimp_mesh_export_menu(
                                                ui,
                                                &package.name,
                                                &mut extract_mesh,
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
            self.begin_extract_chimp_texture_tiff(kit_index, &package, ctx.clone());
        }
        if let Some((package, format)) = extract_mesh {
            self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
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
        let package_types = self.kits[kit_index].chimp.package_types.clone();
        let clicked = egui::ScrollArea::vertical()
            .id_salt(("chimp_folders", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                draw_chimp_folder_node(
                    ui,
                    &self.kits[kit_index].chimp.content_tree,
                    world,
                    &package_types,
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
                self.begin_extract_chimp_texture_tiff(kit_index, &package, ctx.clone());
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

    fn draw_chimp_tiles(&mut self, ui: &mut Ui, ctx: &egui::Context, kit_index: usize) {
        let Some(mut tree) = self.kits[kit_index].chimp.document_tree.take() else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a package to inspect it.");
            });
            return;
        };
        if tree.is_empty() {
            self.kits[kit_index].chimp.document_tree = Some(tree);
            ui.centered_and_justified(|ui| {
                ui.label("Select a package to inspect it.");
            });
            return;
        }

        let mut behavior = ChimpPaneBehavior {
            app: self,
            kit_index,
            close_requests: Vec::new(),
            focused: None,
            close_all: false,
            close_all_but: None,
            extract_texture: None,
            extract_mesh: None,
        };
        tree.ui(&mut behavior, ui);
        let close_requests = std::mem::take(&mut behavior.close_requests);
        let focused = behavior.focused.take();
        let close_all = behavior.close_all;
        let close_all_but = behavior.close_all_but.take();
        let extract_texture = behavior.extract_texture.take();
        let extract_mesh = behavior.extract_mesh.take();
        self.kits[kit_index].chimp.document_tree = Some(tree);
        self.kits[kit_index].chimp.sync_open_packages();
        if let Some(package) = focused {
            self.kits[kit_index].chimp.selected_package = Some(package);
        }

        let mut requested = if close_all {
            self.kits[kit_index].chimp.open_packages.clone()
        } else if let Some(keep) = close_all_but {
            self.kits[kit_index]
                .chimp
                .open_packages
                .iter()
                .filter(|package| *package != &keep)
                .cloned()
                .collect()
        } else {
            close_requests
        };
        requested.sort();
        requested.dedup();
        let mut blocked = false;
        for package in requested {
            if self.kits[kit_index]
                .chimp
                .documents
                .get(&package)
                .is_some_and(|document| document.dirty)
            {
                blocked = true;
                continue;
            }
            self.kits[kit_index].chimp.close_document_pane(&package);
            self.kits[kit_index].chimp.documents.remove(&package);
        }
        if blocked {
            self.status = "Build the Chimp mod before closing modified packages.".to_owned();
        }
        if let Some(package) = extract_texture {
            self.begin_extract_chimp_texture_tiff(kit_index, &package, ctx.clone());
        }
        if let Some((package, format)) = extract_mesh {
            self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
        }
    }

    fn draw_chimp_document_pane(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        package: &str,
        scope: &str,
    ) {
        if !self.kits[kit_index].chimp.documents.contains_key(package) {
            ui.label("This package is no longer loaded.");
            return;
        }
        let package = package.to_owned();

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
                    .add_enabled(document.dirty, egui::Button::new("Save Chimp changes…"))
                    .on_hover_text("Save every modified Chimp package in one operation")
                    .clicked();
                extract_package = ui.button("Extract package…").clicked();
                extract_json = ui.button("Export JSON…").clicked();
                extract_export = ui.button("Extract selected export…").clicked();
            });
        }
        if save_mod {
            self.open_chimp_save_dialog(kit_index);
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
        ui.horizontal(|ui| {
            ui.selectable_value(&mut document.view, ChimpDocumentView::Document, "Document")
                .on_hover_text("Readable JSON representation of the complete decoded package");
            if !document.texture_previews.is_empty() {
                ui.selectable_value(&mut document.view, ChimpDocumentView::Texture, "Texture")
                    .on_hover_text("Decoded Texture2D image preview");
            }
            if document.mesh_kind.is_some() {
                ui.selectable_value(&mut document.view, ChimpDocumentView::Mesh, "Mesh")
                    .on_hover_text("Decoded Unreal mesh in Baboon's 3D viewer");
            }
            ui.selectable_value(
                &mut document.view,
                ChimpDocumentView::Properties,
                "Properties",
            )
            .on_hover_text("Inspect exports and edit supported reflected scalar properties");
            ui.selectable_value(&mut document.view, ChimpDocumentView::Metadata, "Metadata")
                .on_hover_text("Package dependencies and physical archive providers");
        });
        ui.separator();

        let changed = match document.view {
            ChimpDocumentView::Document => {
                if document.document_text_dirty {
                    refresh_chimp_document_text(document);
                }
                draw_chimp_json_document(
                    ui,
                    ("chimp_document_text", scope.to_owned(), package.clone()),
                    "Decoded Unreal package document",
                    "Copy JSON",
                    &document.document_text,
                    &document.document_line_numbers,
                );
                false
            }
            ChimpDocumentView::Texture => {
                draw_chimp_texture_preview(ui, document);
                false
            }
            ChimpDocumentView::Mesh => {
                match document.mesh_preview.as_ref() {
                    Some(Ok(preview)) => model_preview::draw_standalone_mesh_preview(
                        ui,
                        preview,
                        &mut document.mesh_preview_state,
                    ),
                    Some(Err(error)) => {
                        ui.colored_label(Color32::from_rgb(150, 56, 44), error);
                    }
                    None => {
                        ui.label(RichText::new("No mesh geometry found.").color(subtle_dark()));
                    }
                }
                false
            }
            ChimpDocumentView::Properties => {
                egui::SidePanel::left(egui::Id::new((
                    "chimp_exports",
                    scope.to_owned(),
                    package.clone(),
                )))
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
                egui::CentralPanel::default()
                    .show_inside(ui, |ui| {
                        draw_chimp_export_editor(ui, document, world.usmap())
                    })
                    .inner
            }
            ChimpDocumentView::Metadata => {
                if document.metadata_text_dirty {
                    refresh_chimp_metadata_text(document, &world);
                }
                draw_chimp_json_document(
                    ui,
                    ("chimp_metadata_text", scope.to_owned(), package.clone()),
                    "Decoded package metadata",
                    "Copy metadata JSON",
                    &document.metadata_text,
                    &document.metadata_line_numbers,
                );
                false
            }
        };
        if changed {
            document.dirty = true;
            document.document_text_dirty = true;
            document.metadata_text_dirty = true;
        }
        let _ = document;
        if changed {
            self.checkpoint_chimp_document(kit_index, &package);
        }
    }

    fn chimp_default_output_folder(&self, kit_index: usize) -> Option<PathBuf> {
        let root = match &self.kits.get(kit_index)?.source.as_ref()?.source {
            TagSource::IoStoreContainerSet { root, .. } => root,
            _ => return None,
        };
        Some(
            self.chimp_output_dir
                .clone()
                .unwrap_or_else(|| root.clone()),
        )
    }

    pub(super) fn open_chimp_save_dialog(&mut self, kit_index: usize) {
        let dirty = self.kits[kit_index]
            .chimp
            .documents
            .values()
            .filter(|document| document.dirty)
            .count();
        if dirty == 0 {
            self.status = "Chimp has no modified packages to save".to_owned();
            return;
        }
        let Some(folder) = self.chimp_default_output_folder(kit_index) else {
            self.status = "Chimp does not have a Paks output folder".to_owned();
            return;
        };
        self.kits[kit_index].chimp.save_dialog = Some(ChimpSaveDialog {
            mode: ChimpSaveMode::ExportMod,
            name: "ChimpMod".to_owned(),
            folder,
            overwrite_acknowledged: false,
        });
    }

    pub(super) fn draw_chimp_save_window(&mut self, ctx: &egui::Context) {
        let Some(kit_index) = self
            .kits
            .iter()
            .position(|kit| kit.chimp.save_dialog.is_some())
        else {
            return;
        };
        let dirty_packages: Vec<String> = self.kits[kit_index]
            .chimp
            .documents
            .values()
            .filter(|document| document.dirty)
            .map(|document| document.package.clone())
            .collect();
        let source_containers: Vec<PathBuf> = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => {
                let mut paths: Vec<_> = dirty_packages
                    .iter()
                    .filter_map(|package| {
                        let document = self.kits[kit_index].chimp.documents.get(package)?;
                        world
                            .containers()
                            .get(document.provider.container)
                            .map(|container| container.path.clone())
                    })
                    .collect();
                paths.sort();
                paths.dedup();
                paths
            }
            _ => Vec::new(),
        };
        let mut close = false;
        let mut action = None;
        let dialog = self.kits[kit_index]
            .chimp
            .save_dialog
            .as_mut()
            .expect("checked above");
        egui::Window::new("Save Chimp changes")
            .id(egui::Id::new("chimp_save_changes"))
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} modified Unreal package(s) will be saved together.",
                    dirty_packages.len()
                ));
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for package in &dirty_packages {
                            ui.label(package);
                        }
                    });
                ui.separator();
                let mode_before = dialog.mode;
                ui.radio_value(
                    &mut dialog.mode,
                    ChimpSaveMode::ExportMod,
                    "Export mod (recommended)",
                );
                ui.radio_value(
                    &mut dialog.mode,
                    ChimpSaveMode::OverwriteSources,
                    "Overwrite source PAKs",
                );
                if dialog.mode != mode_before {
                    dialog.overwrite_acknowledged = false;
                }
                ui.separator();

                let mut can_save;
                match dialog.mode {
                    ChimpSaveMode::ExportMod => {
                        ui.horizontal(|ui| {
                            ui.label("Mod name");
                            ui.text_edit_singleline(&mut dialog.name);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Destination");
                            ui.label(dialog.folder.display().to_string());
                            if ui.button("Browse…").clicked()
                                && let Some(folder) = rfd::FileDialog::new()
                                    .set_title("Choose Chimp mod folder")
                                    .set_directory(&dialog.folder)
                                    .pick_folder()
                            {
                                dialog.folder = folder;
                                dialog.overwrite_acknowledged = false;
                            }
                        });
                        let stem = chimp_mod_stem(&dialog.name);
                        can_save = !sanitize_mod_name(&dialog.name).is_empty();
                        if can_save {
                            ui.label(format!("Output: {stem}.utoc / .ucas / .pak"));
                        } else {
                            ui.colored_label(
                                Color32::from_rgb(210, 120, 80),
                                "Enter a file-safe mod name.",
                            );
                        }
                        let existing =
                            chimp_existing_triplet(&dialog.folder.join(format!("{stem}.utoc")));
                        if !existing.is_empty() {
                            ui.colored_label(
                                Color32::from_rgb(210, 120, 80),
                                format!("This will replace: {}", existing.join(", ")),
                            );
                            ui.checkbox(
                                &mut dialog.overwrite_acknowledged,
                                "Replace the existing mod container",
                            );
                            can_save &= dialog.overwrite_acknowledged;
                        }
                    }
                    ChimpSaveMode::OverwriteSources => {
                        ui.colored_label(
                            Color32::from_rgb(190, 72, 56),
                            "This replaces package indexes in the installed game containers.",
                        );
                        for path in &source_containers {
                            ui.label(path.display().to_string());
                        }
                        ui.checkbox(
                            &mut dialog.overwrite_acknowledged,
                            "I understand these source containers will be modified",
                        );
                        can_save = dialog.overwrite_acknowledged && !source_containers.is_empty();
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    let label = match dialog.mode {
                        ChimpSaveMode::ExportMod => "Export mod",
                        ChimpSaveMode::OverwriteSources => "Overwrite source PAKs",
                    };
                    if ui.add_enabled(can_save, egui::Button::new(label)).clicked() {
                        action = Some(match dialog.mode {
                            ChimpSaveMode::ExportMod => ChimpSaveAction::Export(
                                dialog
                                    .folder
                                    .join(format!("{}.utoc", chimp_mod_stem(&dialog.name))),
                            ),
                            ChimpSaveMode::OverwriteSources => ChimpSaveAction::Overwrite,
                        });
                    }
                });
            });
        if close || action.is_some() {
            self.kits[kit_index].chimp.save_dialog = None;
        }
        match action {
            Some(ChimpSaveAction::Export(output)) => {
                self.chimp_output_dir = output.parent().map(Path::to_path_buf);
                self.export_chimp_mod_to(kit_index, output, ctx.clone());
            }
            Some(ChimpSaveAction::Overwrite) => {
                self.overwrite_all_dirty_chimp_packages(kit_index, ctx.clone());
            }
            None => {}
        }
    }

    fn export_chimp_mod_to(&mut self, kit_index: usize, output: PathBuf, ctx: egui::Context) {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
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
        let validation = (|| -> Result<(), String> {
            let archive = blam_tags::iostore::IoStoreArchive::open(&temporary)
                .map_err(|error| format!("Could not reopen temporary mod: {error}"))?;
            for (_, provider, bytes, _) in &rebuilt {
                let source = &world.archives()[provider.container];
                let source_index = source
                    .chunk_index_for(&provider.entry_path)
                    .map_err(|error| error.to_string())?;
                let chunk_id = source
                    .chunk_id(source_index)
                    .map_err(|error| error.to_string())?;
                let saved_index = archive
                    .find_chunk(&chunk_id)
                    .ok_or_else(|| format!("Temporary mod is missing {}", provider.entry_path))?;
                let saved = archive
                    .read_chunk(saved_index)
                    .map_err(|error| error.to_string())?;
                if saved != *bytes {
                    return Err(format!(
                        "Temporary mod did not preserve {} exactly",
                        provider.entry_path
                    ));
                }
            }
            Ok(())
        })();
        if let Err(error) = validation {
            remove_chimp_triplet(&temporary);
            self.status = error;
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

    fn overwrite_all_dirty_chimp_packages(&mut self, kit_index: usize, ctx: egui::Context) {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
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
            self.status = "Chimp has no modified packages to overwrite".to_owned();
            return;
        }

        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (index, (_, provider, _, _)) in rebuilt.iter().enumerate() {
            groups.entry(provider.container).or_default().push(index);
        }
        let mut originals = BTreeMap::new();
        for &container in groups.keys() {
            let path = world.containers()[container].path.clone();
            match fs::read(&path) {
                Ok(bytes) => {
                    originals.insert(container, (path, bytes));
                }
                Err(error) => {
                    self.status =
                        format!("Could not stage rollback for {}: {error}", path.display());
                    return;
                }
            }
        }

        let mut failure = None;
        for (&container, indices) in &groups {
            let replacements: Vec<_> = indices
                .iter()
                .map(|&index| {
                    let (_, provider, bytes, store) = &rebuilt[index];
                    PackageReplacement {
                        uasset_path: &provider.entry_path,
                        rebuilt_bytes: bytes,
                        store,
                    }
                })
                .collect();
            let path = &originals[&container].0;
            if let Err(error) =
                overwrite_packages_in_place_with(&world.archives()[container], path, &replacements)
            {
                failure = Some(format!("Could not overwrite {}: {error}", path.display()));
                break;
            }
        }

        if let Some(mut error) = failure {
            let mut rollback_failures = Vec::new();
            for (path, bytes) in originals.values() {
                if let Err(rollback_error) = fs::write(path, bytes) {
                    rollback_failures.push(format!("{}: {rollback_error}", path.display()));
                }
            }
            if !rollback_failures.is_empty() {
                error.push_str(&format!(
                    "; rollback also failed for {}",
                    rollback_failures.join(", ")
                ));
            }
            self.status = error;
            drop(world);
            self.begin_chimp_mount(kit_index, ctx);
            return;
        }

        let packages: Vec<String> = rebuilt
            .iter()
            .map(|(package, _, _, _)| package.clone())
            .collect();
        for (package, _, bytes, _) in rebuilt {
            if let Some(document) = self.kits[kit_index].chimp.documents.get_mut(&package) {
                if let Ok(payloads) = read_payloads(&document.header, &bytes) {
                    document.payloads = payloads;
                }
                document.original = bytes;
                document.dirty = false;
            }
        }
        self.clear_chimp_recovery_packages(kit_index, &packages);
        self.status = format!(
            "Overwrote {} modified Unreal package(s) across {} source container(s)",
            packages.len(),
            groups.len()
        );
        drop(world);
        self.begin_chimp_mount(kit_index, ctx);
    }

    fn overwrite_chimp_package(&mut self, kit_index: usize, package: &str, ctx: egui::Context) {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let provider = document.provider.clone();
        let (bytes, store) = match rebuild_chimp_document(&world, document) {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let archive = &world.archives()[provider.container];
        let path = world.containers()[provider.container].path.clone();
        let source_chunk = match archive
            .chunk_index_for(&provider.entry_path)
            .and_then(|index| archive.chunk_id(index))
        {
            Ok(chunk) => chunk,
            Err(error) => {
                self.status = format!("Could not resolve {package} in {}: {error}", path.display());
                return;
            }
        };
        if let Err(error) =
            overwrite_package_in_place_with(archive, &path, &provider.entry_path, &bytes, &store)
        {
            self.status = format!("Could not overwrite {package}: {error}");
            return;
        }
        let verified = blam_tags::iostore::IoStoreArchive::open(&path)
            .and_then(|archive| {
                let index = archive.find_chunk(&source_chunk).ok_or(
                    blam_tags::iostore::IoStoreError::Package(
                        "saved package chunk is absent after reopening",
                    ),
                )?;
                archive.read_chunk(index)
            })
            .is_ok_and(|saved| saved == bytes);
        if !verified {
            self.status = format!(
                "{package} was written, but validation failed; the package remains marked modified"
            );
            return;
        }
        self.accept_chimp_package_save(kit_index, package, bytes);
        self.status = format!("Saved {package} into {}", path.display());
        drop(world);
        self.begin_chimp_mount(kit_index, ctx);
    }

    fn save_chimp_package_to_folder(
        &mut self,
        kit_index: usize,
        package: &str,
        ctx: egui::Context,
    ) {
        let stem = chimp_package_container_stem(package);
        let Some(chosen) = rfd::FileDialog::new()
            .set_title("Save Chimp package container")
            .add_filter("Unreal IoStore container", &["utoc"])
            .set_file_name(format!("{stem}.utoc"))
            .save_file()
        else {
            return;
        };
        let output = chosen.with_extension("utoc");
        let folder = output.parent().unwrap_or(Path::new(".")).to_path_buf();
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let provider = document.provider.clone();
        let (bytes, store) = match rebuild_chimp_document(&world, document) {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if let Err(error) = fs::create_dir_all(&folder) {
            self.status = format!("Could not create {}: {error}", folder.display());
            return;
        }
        let temporary = folder.join(format!("{stem}.building.utoc"));
        let source_archive = &world.archives()[provider.container];
        let source_chunk = match source_archive
            .chunk_index_for(&provider.entry_path)
            .and_then(|index| source_archive.chunk_id(index))
        {
            Ok(chunk) => chunk,
            Err(error) => {
                self.status = format!("Could not resolve {package}: {error}");
                return;
            }
        };
        let override_ = PackageOverride {
            archive: source_archive,
            uasset_path: &provider.entry_path,
            bytes: bytes.clone(),
            store,
        };
        if let Err(error) = write_package_mod_container(&[override_], &temporary) {
            remove_chimp_triplet(&temporary);
            self.status = format!("Could not build {}: {error}", output.display());
            return;
        }
        let validated = blam_tags::iostore::IoStoreArchive::open(&temporary)
            .and_then(|archive| {
                let index = archive.find_chunk(&source_chunk).ok_or(
                    blam_tags::iostore::IoStoreError::Package(
                        "saved package chunk is absent from the new container",
                    ),
                )?;
                archive.read_chunk(index)
            })
            .is_ok_and(|saved| saved == bytes);
        if !validated {
            remove_chimp_triplet(&temporary);
            self.status = format!("Could not validate the rebuilt package container for {package}");
            return;
        }

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
                self.accept_chimp_package_save(kit_index, package, bytes);
                self.status = format!("Saved {package} as {}", output.display());
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

    fn accept_chimp_package_save(&mut self, kit_index: usize, package: &str, bytes: Vec<u8>) {
        if let Some(document) = self.kits[kit_index].chimp.documents.get_mut(package) {
            if let Ok(payloads) = read_payloads(&document.header, &bytes) {
                document.payloads = payloads;
            }
            document.original = bytes;
            document.dirty = false;
        }
        self.clear_chimp_recovery_packages(kit_index, &[package.to_owned()]);
    }

    pub(super) fn draw_chimp_overwrite_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(kit_index) = self
            .kits
            .iter()
            .position(|kit| kit.chimp.pending_overwrite.is_some())
        else {
            return;
        };
        let package = self.kits[kit_index]
            .chimp
            .pending_overwrite
            .clone()
            .expect("checked above");
        let source = self.kits[kit_index]
            .chimp
            .documents
            .get(&package)
            .and_then(|document| match &self.kits[kit_index].chimp.mount {
                ChimpMount::Ready(world) => world
                    .containers()
                    .get(document.provider.container)
                    .map(|container| container.path.clone()),
                _ => None,
            });
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Overwrite Unreal package?")
            .id(egui::Id::new("chimp_overwrite_confirm"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.colored_label(
                    Color32::from_rgb(190, 72, 56),
                    "This modifies the selected game container in place.",
                );
                ui.label(RichText::new(&package).strong());
                if let Some(path) = &source {
                    ui.label(path.display().to_string());
                }
                ui.label("Baboon appends the rebuilt chunks and atomically updates the UTOC.");
                ui.checkbox(
                    &mut self.kits[kit_index].chimp.pending_overwrite_skip_future,
                    "Don't ask again (changeable in Settings)",
                );
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Overwrite package").clicked() {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.kits[kit_index].chimp.pending_overwrite = None;
        } else if confirm {
            if self.kits[kit_index].chimp.pending_overwrite_skip_future {
                self.confirm_container_overwrite = false;
            }
            self.kits[kit_index].chimp.pending_overwrite = None;
            self.overwrite_chimp_package(kit_index, &package, ctx.clone());
        }
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

    fn begin_extract_chimp_texture_tiff(
        &mut self,
        kit_index: usize,
        package: &str,
        ctx: egui::Context,
    ) {
        let suggested = format!("{}.tif", package.rsplit('/').next().unwrap_or("texture"));
        let Some(path) = rfd::FileDialog::new()
            .set_title("Extract Texture2D as TIFF")
            .add_filter("TIFF image", &["tif", "tiff"])
            .set_file_name(&suggested)
            .save_file()
        else {
            return;
        };
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let package = package.to_owned();
        let tx = self.tx.clone();
        self.status = format!("Extracting {package}…");
        thread::spawn(move || {
            let result = write_chimp_texture_tiff(&world, &package, &path);
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    fn begin_extract_chimp_mesh(
        &mut self,
        kit_index: usize,
        package: &str,
        format: ChimpMeshFormat,
        ctx: egui::Context,
    ) {
        let suggested = format!(
            "{}.{}",
            package.rsplit('/').next().unwrap_or("mesh"),
            format.extension()
        );
        let Some(path) = rfd::FileDialog::new()
            .set_title(format!("Extract mesh as {}", format.label()))
            .add_filter(format.label(), &[format.extension()])
            .set_file_name(&suggested)
            .save_file()
        else {
            return;
        };
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let package = package.to_owned();
        let tx = self.tx.clone();
        self.status = format!("Extracting {package} as {}…", format.label());
        thread::spawn(move || {
            let result = write_chimp_mesh(&world, &package, &path, format);
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }
}

fn write_chimp_texture_tiff(world: &World, package: &str, output: &Path) -> Result<String, String> {
    let document = load_chimp_document(world, package)?;
    let data = document
        .texture_previews
        .iter()
        .find_map(|texture| texture.preview.decoded.as_ref()?.as_ref().ok())
        .ok_or_else(|| format!("{package} has no decodable Texture2D image"))?;
    let mut file = fs::File::create(output)
        .map_err(|error| format!("Could not create {}: {error}", output.display()))?;
    // Export the decoder's straight RGBA buffer. Viewer channel masks,
    // backgrounds and colour presentation are deliberately not applied.
    blam_tags::bitmap::tiff::write_rgba8_tiff(&mut file, data.width, data.height, &data.rgba)
        .map_err(|error| format!("Could not encode {}: {error}", output.display()))?;
    Ok(format!("Extracted {package} to {}", output.display()))
}

fn write_chimp_mesh(
    world: &World,
    package: &str,
    output: &Path,
    format: ChimpMeshFormat,
) -> Result<String, String> {
    let document = load_chimp_document(world, package)?;
    let kind = document
        .mesh_kind
        .ok_or_else(|| format!("{package} is not a StaticMesh or SkeletalMesh"))?;
    let materials = chimp_material_names(&document.header);
    let mut writer = std::io::BufWriter::new(
        fs::File::create(output)
            .map_err(|error| format!("Could not create {}: {error}", output.display()))?,
    );
    match kind {
        ChimpMeshKind::Skeletal => {
            let mesh = SkeletalMesh::from_package(
                &document.original,
                &document.header.name_map.copy_raw_names(),
                document.header.summary.header_size as usize,
            )
            .map_err(|error| format!("Could not decode {package}: {error:#}"))?;
            match format {
                ChimpMeshFormat::Jms => chimp_skeletal_mesh_to_jms(&mesh, &materials)
                    .write(&mut writer, 8213)
                    .map_err(|error| error.to_string())?,
                ChimpMeshFormat::Psk => blam_tags::iostore::actorx::write_skeletal_mesh(
                    &mesh,
                    &materials,
                    blam_tags::iostore::actorx::ActorXFormat::Psk,
                    &mut writer,
                )
                .map_err(|error| error.to_string())?,
                ChimpMeshFormat::Pskx => blam_tags::iostore::actorx::write_skeletal_mesh(
                    &mesh,
                    &materials,
                    blam_tags::iostore::actorx::ActorXFormat::Pskx,
                    &mut writer,
                )
                .map_err(|error| error.to_string())?,
            }
        }
        ChimpMeshKind::Static => {
            let archive = &world.archives()[document.provider.container];
            let bulk = archive
                .chunk_index_for(&document.provider.entry_path)
                .ok()
                .and_then(|chunk| archive.read_bulk_for(chunk, 0).ok());
            let mesh = StaticMesh::from_package_preferring_nanite(
                &document.original,
                document.header.summary.header_size as usize,
                bulk.as_deref(),
            )
            .map_err(|error| format!("Could not decode {package}: {error:#}"))?;
            match format {
                ChimpMeshFormat::Jms => chimp_static_mesh_to_jms(&mesh, &materials)
                    .write(&mut writer, 8213)
                    .map_err(|error| error.to_string())?,
                ChimpMeshFormat::Psk => blam_tags::iostore::actorx::write_static_mesh(
                    &mesh,
                    &materials,
                    blam_tags::iostore::actorx::ActorXFormat::Psk,
                    &mut writer,
                )
                .map_err(|error| error.to_string())?,
                ChimpMeshFormat::Pskx => blam_tags::iostore::actorx::write_static_mesh(
                    &mesh,
                    &materials,
                    blam_tags::iostore::actorx::ActorXFormat::Pskx,
                    &mut writer,
                )
                .map_err(|error| error.to_string())?,
            }
        }
    }
    writer
        .flush()
        .map_err(|error| format!("Could not finish {}: {error}", output.display()))?;
    Ok(format!(
        "Extracted {package} as {} to {}",
        format.label(),
        output.display()
    ))
}

/// Canonical Unreal skeletal-mesh to JMS conversion shared by Chimp's direct
/// exporter and Campaign Evolved tag-model extraction.
pub(in crate::app) fn chimp_skeletal_mesh_to_jms(
    mesh: &SkeletalMesh,
    material_names: &[String],
) -> blam_tags::jms::JmsFile {
    blam_tags::iostore::actorx::skeletal_mesh_to_jms(mesh, material_names)
}

/// Canonical Unreal static-mesh to JMS conversion shared by Chimp's direct
/// exporter and Campaign Evolved tag-model extraction.
pub(in crate::app) fn chimp_static_mesh_to_jms(
    mesh: &StaticMesh,
    material_names: &[String],
) -> blam_tags::jms::JmsFile {
    blam_tags::iostore::actorx::static_mesh_to_jms(mesh, material_names)
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
        if matches!(
            package_type,
            Some("Texture2D" | "SkeletalMesh" | "StaticMesh")
        ) {
            response.context_menu(|ui| {
                if package_type == Some("Texture2D")
                    && ui.button("Extract Texture2D as TIFF…").clicked()
                {
                    clicked = Some(ChimpTreeClick::ExtractTexture(package.name.clone()));
                    ui.close_menu();
                }
                if matches!(package_type, Some("SkeletalMesh" | "StaticMesh")) {
                    ui.menu_button("Extract mesh", |ui| {
                        for format in [
                            ChimpMeshFormat::Jms,
                            ChimpMeshFormat::Psk,
                            ChimpMeshFormat::Pskx,
                        ] {
                            if ui.button(format.label()).clicked() {
                                clicked =
                                    Some(ChimpTreeClick::ExtractMesh(package.name.clone(), format));
                                ui.close_menu();
                            }
                        }
                    });
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

fn chimp_mesh_export_menu(
    ui: &mut Ui,
    package: &str,
    requested: &mut Option<(String, ChimpMeshFormat)>,
) {
    ui.menu_button("Extract mesh", |ui| {
        for format in [
            ChimpMeshFormat::Jms,
            ChimpMeshFormat::Psk,
            ChimpMeshFormat::Pskx,
        ] {
            if ui.button(format.label()).clicked() {
                *requested = Some((package.to_owned(), format));
                ui.close_menu();
            }
        }
    });
}

fn triplet(path: &Path) -> [PathBuf; 3] {
    [
        path.with_extension("utoc"),
        path.with_extension("ucas"),
        path.with_extension("pak"),
    ]
}

fn chimp_mod_stem(name: &str) -> String {
    let sanitized = sanitize_mod_name(name);
    if sanitized
        .get(sanitized.len().saturating_sub(2)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case("_p"))
    {
        sanitized
    } else {
        format!("{sanitized}_P")
    }
}

fn chimp_existing_triplet(path: &Path) -> Vec<String> {
    triplet(path)
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("container file")
                .to_owned()
        })
        .collect()
}

fn chimp_package_container_stem(package: &str) -> String {
    let leaf = package
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("Chimp");
    let mut stem = leaf
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if stem.is_empty() {
        stem.push_str("Chimp");
    }
    if !stem.ends_with("_P") {
        stem.push_str("_P");
    }
    stem
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

fn draw_chimp_texture_preview(ui: &mut Ui, document: &mut ChimpDocument) {
    let options: Vec<(usize, String)> = document
        .texture_previews
        .iter()
        .map(|preview| {
            (
                preview.export_index,
                document
                    .exports
                    .get(preview.export_index)
                    .map(|export| export.object.clone())
                    .unwrap_or_else(|| format!("Export {}", preview.export_index)),
            )
        })
        .collect();
    let mut selected_export = if options
        .iter()
        .any(|(index, _)| *index == document.selected_export)
    {
        document.selected_export
    } else {
        options.first().map(|(index, _)| *index).unwrap_or(0)
    };
    if options.len() > 1 {
        let selected_label = options
            .iter()
            .find(|(index, _)| *index == selected_export)
            .map(|(_, label)| label.as_str())
            .unwrap_or("Texture export");
        egui::ComboBox::from_id_salt(("chimp_texture_export", document.package.clone()))
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for (index, label) in &options {
                    ui.selectable_value(&mut selected_export, *index, label);
                }
            });
    }
    document.selected_export = selected_export;
    let Some(preview) = document
        .texture_previews
        .iter_mut()
        .find(|preview| preview.export_index == selected_export)
    else {
        ui.label("This package has no Texture2D export to preview.");
        return;
    };
    let texture_key = format!(
        "chimp_texture_{}_{}",
        document.package, preview.export_index
    );
    let ctx = ui.ctx().clone();
    draw_bitmap_preview_data(ui, &ctx, &texture_key, &mut preview.preview, false);
}

#[derive(Clone, Copy)]
struct ChimpJsonPalette {
    plain: Color32,
    key: Color32,
    string: Color32,
    path: Color32,
    number: Color32,
    literal: Color32,
    punctuation: Color32,
}

impl ChimpJsonPalette {
    fn for_dark_mode(dark_mode: bool) -> Self {
        if dark_mode {
            Self {
                plain: Color32::from_rgb(210, 214, 222),
                key: Color32::from_rgb(244, 191, 92),
                string: Color32::from_rgb(190, 235, 125),
                path: Color32::from_rgb(232, 157, 222),
                number: Color32::from_rgb(242, 139, 130),
                literal: Color32::from_rgb(105, 190, 255),
                punctuation: Color32::from_rgb(105, 210, 225),
            }
        } else {
            Self {
                plain: Color32::from_rgb(45, 50, 58),
                key: Color32::from_rgb(145, 91, 0),
                string: Color32::from_rgb(55, 112, 15),
                path: Color32::from_rgb(154, 52, 136),
                number: Color32::from_rgb(185, 62, 48),
                literal: Color32::from_rgb(0, 104, 178),
                punctuation: Color32::from_rgb(0, 112, 128),
            }
        }
    }
}

#[derive(Default)]
struct ChimpJsonHighlighter;

impl egui::util::cache::ComputerMut<(&egui::FontId, &str, bool), egui::text::LayoutJob>
    for ChimpJsonHighlighter
{
    fn compute(
        &mut self,
        (font_id, text, dark_mode): (&egui::FontId, &str, bool),
    ) -> egui::text::LayoutJob {
        chimp_json_layout_job(text, font_id.clone(), dark_mode)
    }
}

fn chimp_json_highlight(ui: &Ui, text: &str) -> egui::text::LayoutJob {
    type HighlightCache =
        egui::util::cache::FrameCache<egui::text::LayoutJob, ChimpJsonHighlighter>;

    let font_id = TextStyle::Monospace.resolve(ui.style());
    let dark_mode = ui.visuals().dark_mode;
    ui.ctx().memory_mut(|memory| {
        memory
            .caches
            .cache::<HighlightCache>()
            .get((&font_id, text, dark_mode))
    })
}

fn chimp_json_layout_job(
    text: &str,
    font_id: egui::FontId,
    dark_mode: bool,
) -> egui::text::LayoutJob {
    let palette = ChimpJsonPalette::for_dark_mode(dark_mode);
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let bytes = text.as_bytes();
    let mut offset = 0;
    let mut active_key = String::new();

    while offset < bytes.len() {
        let start = offset;
        let (end, color) = match bytes[offset] {
            b'"' => {
                offset += 1;
                while offset < bytes.len() {
                    match bytes[offset] {
                        b'\\' => offset = (offset + 2).min(bytes.len()),
                        b'"' => {
                            offset += 1;
                            break;
                        }
                        _ => offset += 1,
                    }
                }
                let mut after = offset;
                while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                    after += 1;
                }
                if after < bytes.len() && bytes[after] == b':' {
                    if offset >= start + 2 {
                        active_key = text[start + 1..offset - 1].to_owned();
                    }
                    (offset, palette.key)
                } else {
                    let path_value = active_key.to_ascii_lowercase().contains("path")
                        || active_key.to_ascii_lowercase().contains("package");
                    (
                        offset,
                        if path_value {
                            palette.path
                        } else {
                            palette.string
                        },
                    )
                }
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                offset += 1;
                (offset, palette.punctuation)
            }
            b'-' | b'0'..=b'9' => {
                offset += 1;
                while offset < bytes.len()
                    && matches!(
                        bytes[offset],
                        b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'
                    )
                {
                    offset += 1;
                }
                (offset, palette.number)
            }
            _ if text[start..].starts_with("true") => {
                offset += 4;
                (offset, palette.literal)
            }
            _ if text[start..].starts_with("false") => {
                offset += 5;
                (offset, palette.literal)
            }
            _ if text[start..].starts_with("null") => {
                offset += 4;
                (offset, palette.literal)
            }
            byte if byte.is_ascii_whitespace() => {
                offset += 1;
                while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
                    offset += 1;
                }
                (offset, palette.plain)
            }
            _ => {
                offset += text[start..].chars().next().map_or(1, char::len_utf8);
                (offset, palette.plain)
            }
        };
        job.append(
            &text[start..end],
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color,
                ..Default::default()
            },
        );
    }
    job
}

fn chimp_line_numbers(text: &str) -> String {
    let count = text.lines().count().max(1);
    let width = count.to_string().len();
    (1..=count)
        .map(|line| format!("{line:>width$}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn draw_chimp_json_document(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    title: &str,
    copy_label: &str,
    text: &str,
    line_numbers: &str,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().color(subtle_dark()));
        if ui.small_button(copy_label).clicked() {
            ui.output_mut(|output| output.copied_text = text.to_owned());
        }
        ui.label(
            RichText::new(format!("{} lines", text.lines().count().max(1)))
                .small()
                .color(subtle_dark()),
        );
    });
    egui::ScrollArea::both()
        .id_salt(id)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let highlighted = chimp_json_highlight(ui, text);
            let font_id = TextStyle::Monospace.resolve(ui.style());
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal_top(|ui| {
                Frame::none()
                    .fill(ui.visuals().faint_bg_color)
                    .inner_margin(egui::Margin {
                        left: 6.0,
                        right: 8.0,
                        top: 4.0,
                        bottom: 4.0,
                    })
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(line_numbers)
                                    .font(font_id.clone())
                                    .color(ui.visuals().weak_text_color()),
                            )
                            .selectable(false),
                        );
                    });
                Frame::none()
                    .inner_margin(egui::Margin {
                        left: 8.0,
                        right: 8.0,
                        top: 4.0,
                        bottom: 4.0,
                    })
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(highlighted).selectable(true));
                    });
            });
        });
}

fn draw_chimp_export_editor(ui: &mut Ui, document: &mut ChimpDocument, usmap: &Usmap) -> bool {
    let Some(export) = document.exports.get_mut(document.selected_export) else {
        ui.label("This package has no exports.");
        return false;
    };
    ui.heading(&export.object);
    ui.label(
        RichText::new(export.class.as_deref().unwrap_or("Unknown class")).color(subtle_dark()),
    );
    ui.add_space(6.0);
    let class = export.class.clone().unwrap_or_default();
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
                        draw_chimp_property_block(
                            ui,
                            block,
                            &class,
                            &mut document.header.name_map,
                            usmap,
                            0,
                        )
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
    class: &str,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    let mut changed = false;
    let existing: std::collections::BTreeSet<u32> = block
        .entries
        .iter()
        .filter_map(|entry| entry.slot.map(|slot| slot.index))
        .collect();
    if !class.is_empty()
        && let Ok(slots) = editable_schema_slots(class, usmap)
    {
        let omitted: Vec<_> = slots
            .into_iter()
            .filter(|(_, slot, ty)| {
                !existing.contains(&slot.index) && default_value_for_type(ty, usmap).is_ok()
            })
            .collect();
        if !omitted.is_empty() {
            ui.menu_button(
                format!("Add omitted property… ({})", omitted.len()),
                |ui| {
                    ui.set_max_height(360.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (name, slot, ty) in &omitted {
                            let label = if slot.array_index == 0 {
                                name.clone()
                            } else {
                                format!("{name}[{}]", slot.array_index)
                            };
                            if ui.button(label).on_hover_text(format!("{ty:?}")).clicked()
                                && let Ok(value) = default_value_for_type(ty, usmap)
                                && set_property_slot(
                                    block,
                                    class,
                                    name,
                                    slot.array_index,
                                    value,
                                    usmap,
                                )
                                .is_ok()
                            {
                                changed = true;
                                ui.close_menu();
                            }
                        }
                    });
                },
            );
            ui.separator();
        }
    }
    for (index, entry) in block.entries.iter_mut().enumerate() {
        let id = ui.make_persistent_id((depth, index, entry.name.as_ref()));
        let declared = entry
            .slot
            .and_then(|slot| property_type_for_slot(class, slot, usmap).ok());
        let label = match entry.slot.map(|slot| slot.array_index) {
            Some(array_index) if array_index > 0 => {
                format!("{}[{array_index}]", entry.name)
            }
            _ => entry.name.to_string(),
        };
        ui.horizontal_top(|ui| {
            ui.set_min_height(24.0);
            ui.label(RichText::new(label).strong()).on_hover_text(
                declared
                    .as_ref()
                    .map(|ty| format!("{ty:?}"))
                    .unwrap_or_else(|| "Native field without a USMAP slot".to_owned()),
            );
            changed |= chimp_property_value_cell(ui, |ui| {
                draw_chimp_value(
                    ui,
                    id,
                    &mut entry.value,
                    declared.as_ref(),
                    names,
                    usmap,
                    depth,
                )
            })
            .inner;
        });
        ui.separator();
    }
    changed
}

fn chimp_property_value_cell<R>(
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    // `Ui::with_layout` inherits the parent's full remaining cross-axis size.
    // In a vertical ScrollArea that made one compact scalar editor consume the
    // entire viewport and then centered the widget inside it. Start each value
    // cell at one normal row instead; `allocate_ui_with_layout` still expands
    // when a nested struct or container genuinely needs additional height.
    let row_height = ui.spacing().interact_size.y.max(24.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), row_height),
        egui::Layout::right_to_left(egui::Align::Min),
        add_contents,
    )
}

fn draw_chimp_value(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut PropValue,
    declared: Option<&PropertyType>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    if let Some(PropertyType::Optional(inner)) = declared
        && !matches!(value, PropValue::Unset)
    {
        let mut changed = false;
        ui.horizontal(|ui| {
            if ui.small_button("Unset").clicked() {
                *value = PropValue::Unset;
                changed = true;
            } else {
                changed |= draw_chimp_value(
                    ui,
                    id.with("optional"),
                    value,
                    Some(inner),
                    names,
                    usmap,
                    depth,
                );
            }
        });
        return changed;
    }
    match value {
        PropValue::Bool(value) => ui.checkbox(value, "").changed(),
        PropValue::Int(value) => draw_chimp_integer(ui, value, declared, usmap),
        PropValue::Float(value) => ui.add(egui::DragValue::new(value).speed(0.01)).changed(),
        PropValue::Str(value) => {
            let mut text = value.to_string();
            let changed = ui
                .add(egui::TextEdit::singleline(&mut text).id(id))
                .changed();
            if changed {
                value.set_text(text);
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
        PropValue::Object(value) => ui
            .add(egui::DragValue::new(value).prefix("Object "))
            .on_hover_text("FPackageIndex: negative = import, positive = export, zero = None")
            .changed(),
        PropValue::SoftObject(value) => {
            let mut changed = false;
            egui::CollapsingHeader::new("Soft object path")
                .id_salt(id)
                .show(ui, |ui| {
                    changed |= draw_chimp_fname(ui, "Package", &mut value.package, names);
                    changed |= draw_chimp_fname(ui, "Asset", &mut value.asset, names);
                    let mut sub_path = value.sub_path.to_string();
                    ui.horizontal(|ui| {
                        ui.label("Sub-path");
                        if ui.text_edit_singleline(&mut sub_path).changed() {
                            value.sub_path.set_text(sub_path);
                            changed = true;
                        }
                    });
                });
            changed
        }
        PropValue::Struct(block) => {
            let nested_class = match declared {
                Some(PropertyType::Struct(name)) => name.as_str(),
                _ => "",
            };
            egui::CollapsingHeader::new(format!("Struct ({} properties)", block.len()))
                .id_salt(id)
                .show(ui, |ui| {
                    draw_chimp_property_block(ui, block, nested_class, names, usmap, depth + 1)
                })
                .body_returned
                .unwrap_or(false)
        }
        PropValue::Array(values) => draw_chimp_sequence(
            ui,
            id,
            "Array",
            values,
            declared.and_then(|ty| match ty {
                PropertyType::Array(inner) => Some(inner.as_ref()),
                _ => None,
            }),
            names,
            usmap,
            depth,
            true,
        ),
        PropValue::Set(values) => draw_chimp_sequence(
            ui,
            id,
            "Set",
            values,
            declared.and_then(|ty| match ty {
                PropertyType::Set(inner) => Some(inner.as_ref()),
                _ => None,
            }),
            names,
            usmap,
            depth,
            false,
        ),
        PropValue::Map(values) => draw_chimp_map(ui, id, values, declared, names, usmap, depth),
        PropValue::WithRemovals { removals, inner } => {
            let removal_type = match declared {
                Some(PropertyType::Map(key, _)) | Some(PropertyType::Set(key)) => {
                    Some(key.as_ref())
                }
                _ => None,
            };
            let mut changed = false;
            egui::CollapsingHeader::new("Delta-serialized container")
                .id_salt(id)
                .show(ui, |ui| {
                    let mut replace_whole = removals.is_none();
                    if ui
                        .checkbox(&mut replace_whole, "Replace whole container")
                        .changed()
                    {
                        *removals = (!replace_whole).then(Vec::new);
                        changed = true;
                    }
                    if let Some(removals) = removals {
                        changed |= draw_chimp_sequence(
                            ui,
                            id.with("removals"),
                            "Removed values",
                            removals,
                            removal_type,
                            names,
                            usmap,
                            depth + 1,
                            true,
                        );
                    }
                    changed |= draw_chimp_value(
                        ui,
                        id.with("inner"),
                        inner,
                        declared,
                        names,
                        usmap,
                        depth + 1,
                    );
                });
            changed
        }
        PropValue::Native(value) => draw_chimp_native(ui, id, value),
        PropValue::HandWritten(value) => {
            draw_chimp_hand_written(ui, id, value, names, usmap, depth)
        }
        PropValue::Delegate { object, function } => {
            let mut changed = false;
            egui::CollapsingHeader::new("Delegate")
                .id_salt(id)
                .show(ui, |ui| {
                    changed |= ui
                        .add(egui::DragValue::new(object).prefix("Object "))
                        .changed();
                    changed |= draw_chimp_fname(ui, "Function", function, names);
                });
            changed
        }
        PropValue::MulticastDelegate(values) => {
            draw_chimp_multicast_delegate(ui, id, values, names)
        }
        PropValue::FieldPath { path, owner } => {
            let mut changed = ui
                .add(egui::DragValue::new(owner).prefix("Owner "))
                .changed();
            egui::CollapsingHeader::new(format!("Field path ({} segments)", path.len()))
                .id_salt(id)
                .show(ui, |ui| {
                    let mut remove = None;
                    for (index, segment) in path.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            changed |= draw_chimp_fname(ui, &format!("[{index}]"), segment, names);
                            if ui.small_button("−").clicked() {
                                remove = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove {
                        path.remove(index);
                        changed = true;
                    }
                    if ui.small_button("+ segment").clicked() {
                        path.push(blam_tags::iostore::object::value::FName::none());
                        changed = true;
                    }
                });
            changed
        }
        PropValue::Unset => {
            if let Some(PropertyType::Optional(inner)) = declared
                && ui.button("Set value").clicked()
            {
                match default_value_for_type(inner, usmap) {
                    Ok(default) => {
                        *value = default;
                        return true;
                    }
                    Err(error) => {
                        ui.colored_label(Color32::from_rgb(210, 120, 80), error.to_string());
                    }
                }
            } else {
                ui.label("Unset");
            }
            false
        }
        PropValue::Raw(bytes) => {
            ui.colored_label(
                Color32::from_rgb(190, 150, 70),
                format!("{} untyped bytes · preserved read-only", bytes.len()),
            );
            false
        }
    }
}

fn draw_chimp_fname(
    ui: &mut Ui,
    label: &str,
    value: &mut blam_tags::iostore::object::value::FName,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    let mut text = value.to_string();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.text_edit_singleline(&mut text).changed() {
            *value = blam_tags::iostore::object::edit::intern_name(names, &text);
            changed = true;
        }
    });
    changed
}

fn draw_chimp_fstr(
    ui: &mut Ui,
    label: &str,
    value: &mut blam_tags::iostore::object::value::FStr,
) -> bool {
    let mut text = value.to_string();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.text_edit_singleline(&mut text).changed() {
            value.set_text(text);
            changed = true;
        }
    });
    changed
}

fn draw_chimp_integer(
    ui: &mut Ui,
    value: &mut i64,
    declared: Option<&PropertyType>,
    usmap: &Usmap,
) -> bool {
    let ty = match declared {
        Some(PropertyType::Enum { inner, enum_name }) => {
            if let Some(definition) = usmap.enums.iter().find(|item| item.name == *enum_name) {
                let selected = definition
                    .values
                    .iter()
                    .find(|(candidate, _)| *candidate == *value as u64)
                    .map(|(_, name)| name.as_str())
                    .unwrap_or("Unknown");
                let mut changed = false;
                egui::ComboBox::from_id_salt(ui.next_auto_id())
                    .selected_text(format!("{selected} ({})", *value as u64))
                    .show_ui(ui, |ui| {
                        for (candidate, name) in &definition.values {
                            if ui
                                .selectable_label(*value as u64 == *candidate, name)
                                .clicked()
                            {
                                *value = *candidate as i64;
                                changed = true;
                            }
                        }
                    });
                return changed;
            }
            Some(inner.as_ref())
        }
        Some(PropertyType::Byte {
            enum_name: Some(enum_name),
        }) => {
            if let Some(definition) = usmap.enums.iter().find(|item| item.name == *enum_name) {
                let mut changed = false;
                egui::ComboBox::from_id_salt(ui.next_auto_id())
                    .selected_text(
                        definition
                            .values
                            .iter()
                            .find(|(candidate, _)| *candidate == *value as u64)
                            .map(|(_, name)| name.clone())
                            .unwrap_or_else(|| value.to_string()),
                    )
                    .show_ui(ui, |ui| {
                        for (candidate, name) in &definition.values {
                            if ui
                                .selectable_label(*value as u64 == *candidate, name)
                                .clicked()
                            {
                                *value = *candidate as i64;
                                changed = true;
                            }
                        }
                    });
                return changed;
            }
            declared
        }
        other => other,
    };
    match ty {
        Some(PropertyType::UInt64) => {
            let mut unsigned = *value as u64;
            let changed = ui.add(egui::DragValue::new(&mut unsigned)).changed();
            if changed {
                *value = unsigned as i64;
            }
            changed
        }
        Some(PropertyType::UInt32) => ui
            .add(egui::DragValue::new(value).range(0..=u32::MAX as i64))
            .changed(),
        Some(PropertyType::UInt16) => ui
            .add(egui::DragValue::new(value).range(0..=u16::MAX as i64))
            .changed(),
        Some(PropertyType::Int8) => ui
            .add(egui::DragValue::new(value).range(i8::MIN as i64..=i8::MAX as i64))
            .changed(),
        Some(PropertyType::Int16) => ui
            .add(egui::DragValue::new(value).range(i16::MIN as i64..=i16::MAX as i64))
            .changed(),
        Some(PropertyType::Int) => ui
            .add(egui::DragValue::new(value).range(i32::MIN as i64..=i32::MAX as i64))
            .changed(),
        Some(PropertyType::Byte { .. }) => ui
            .add(egui::DragValue::new(value).range(0..=u8::MAX as i64))
            .changed(),
        _ => ui.add(egui::DragValue::new(value)).changed(),
    }
}

#[derive(Clone, Copy)]
enum ChimpListAction {
    Remove(usize),
    Duplicate(usize),
    MoveUp(usize),
    MoveDown(usize),
}

fn draw_chimp_list<T: Clone>(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    values: &mut Vec<T>,
    default: Option<T>,
    mut draw: impl FnMut(&mut Ui, usize, &mut T) -> bool,
) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new(format!("{label} ({})", values.len()))
        .id_salt(id)
        .show(ui, |ui| {
            let mut action = None;
            for (index, value) in values.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.label(format!("[{index}]"));
                        if ui.small_button("↑").clicked() {
                            action = Some(ChimpListAction::MoveUp(index));
                        }
                        if ui.small_button("↓").clicked() {
                            action = Some(ChimpListAction::MoveDown(index));
                        }
                        if ui.small_button("Duplicate").clicked() {
                            action = Some(ChimpListAction::Duplicate(index));
                        }
                        if ui.small_button("Remove").clicked() {
                            action = Some(ChimpListAction::Remove(index));
                        }
                    });
                    changed |= draw(ui, index, value);
                    ui.separator();
                });
            }
            if let Some(default) = default
                && ui.small_button("+ Add").clicked()
            {
                values.push(default);
                changed = true;
            }
            match action {
                Some(ChimpListAction::Remove(index)) => {
                    values.remove(index);
                    changed = true;
                }
                Some(ChimpListAction::Duplicate(index)) => {
                    values.insert(index + 1, values[index].clone());
                    changed = true;
                }
                Some(ChimpListAction::MoveUp(index)) if index > 0 => {
                    values.swap(index, index - 1);
                    changed = true;
                }
                Some(ChimpListAction::MoveDown(index)) if index + 1 < values.len() => {
                    values.swap(index, index + 1);
                    changed = true;
                }
                _ => {}
            }
        });
    changed
}

fn draw_chimp_sequence(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    values: &mut Vec<PropValue>,
    inner_type: Option<&PropertyType>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
    allow_duplicates: bool,
) -> bool {
    let default = inner_type.and_then(|ty| default_value_for_type(ty, usmap).ok());
    let before = values.clone();
    let mut changed = draw_chimp_list(ui, id, label, values, default, |ui, index, value| {
        draw_chimp_value(
            ui,
            id.with(index),
            value,
            inner_type,
            names,
            usmap,
            depth + 1,
        )
    });
    if !allow_duplicates {
        let mut duplicate = false;
        for left in 0..values.len() {
            duplicate |= values[left + 1..]
                .iter()
                .any(|right| values[left].semantic_eq(right));
        }
        if duplicate {
            *values = before;
            changed = false;
            ui.colored_label(
                Color32::from_rgb(210, 120, 80),
                "Sets cannot contain duplicate values.",
            );
        }
    }
    changed
}

fn draw_chimp_map(
    ui: &mut Ui,
    id: egui::Id,
    values: &mut Vec<(PropValue, PropValue)>,
    declared: Option<&PropertyType>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    let (key_type, value_type) = match declared {
        Some(PropertyType::Map(key, value)) => (Some(key.as_ref()), Some(value.as_ref())),
        _ => (None, None),
    };
    let default = key_type
        .and_then(|key| default_value_for_type(key, usmap).ok())
        .zip(value_type.and_then(|value| default_value_for_type(value, usmap).ok()));
    let before = values.clone();
    let mut changed = draw_chimp_list(ui, id, "Map", values, default, |ui, index, pair| {
        let mut changed = false;
        ui.label(RichText::new("Key").strong());
        changed |= draw_chimp_value(
            ui,
            id.with((index, "key")),
            &mut pair.0,
            key_type,
            names,
            usmap,
            depth + 1,
        );
        ui.label(RichText::new("Value").strong());
        changed |= draw_chimp_value(
            ui,
            id.with((index, "value")),
            &mut pair.1,
            value_type,
            names,
            usmap,
            depth + 1,
        );
        changed
    });
    let mut duplicate = false;
    for left in 0..values.len() {
        duplicate |= values[left + 1..]
            .iter()
            .any(|right| values[left].0.semantic_eq(&right.0));
    }
    if duplicate {
        *values = before;
        changed = false;
        ui.colored_label(
            Color32::from_rgb(210, 120, 80),
            "Maps cannot contain duplicate keys.",
        );
    }
    changed
}

fn draw_chimp_multicast_delegate(
    ui: &mut Ui,
    id: egui::Id,
    values: &mut Vec<(i32, blam_tags::iostore::object::value::FName)>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    draw_chimp_list(
        ui,
        id,
        "Multicast delegate",
        values,
        Some((0, blam_tags::iostore::object::value::FName::none())),
        |ui, _, (object, function)| {
            let mut changed = ui
                .add(egui::DragValue::new(object).prefix("Object "))
                .changed();
            changed |= draw_chimp_fname(ui, "Function", function, names);
            changed
        },
    )
}

fn draw_chimp_f64_values(ui: &mut Ui, values: &mut [f64]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(
                    egui::DragValue::new(value)
                        .speed(0.01)
                        .prefix(format!("{index}: ")),
                )
                .changed();
        }
    });
    changed
}

fn draw_chimp_f32_values(ui: &mut Ui, values: &mut [f32]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(
                    egui::DragValue::new(value)
                        .speed(0.01)
                        .prefix(format!("{index}: ")),
                )
                .changed();
        }
    });
    changed
}

fn draw_chimp_i64_values(ui: &mut Ui, values: &mut [i64]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(egui::DragValue::new(value).prefix(format!("{index}: ")))
                .changed();
        }
    });
    changed
}

fn draw_chimp_i32_values(ui: &mut Ui, values: &mut [i32]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(egui::DragValue::new(value).prefix(format!("{index}: ")))
                .changed();
        }
    });
    changed
}

fn draw_chimp_native(ui: &mut Ui, id: egui::Id, value: &mut NativeStruct) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Native struct")
        .id_salt(id)
        .show(ui, |ui| {
            changed |= match value {
                NativeStruct::Vec3d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::Vec4d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::Vec2d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::TwoVec3d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::Vec3f(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::Vec2f(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::Mat4f(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::LinearColor(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::Mat4d(values) => draw_chimp_f64_values(ui, values.as_mut()),
                NativeStruct::Ints(values) => draw_chimp_i64_values(ui, values),
                NativeStruct::Guid(values) => {
                    let mut changed = false;
                    ui.horizontal_wrapped(|ui| {
                        for (index, value) in values.iter_mut().enumerate() {
                            changed |= ui
                                .add(egui::DragValue::new(value).prefix(format!("{index}: ")))
                                .changed();
                        }
                    });
                    changed
                }
                NativeStruct::Color(values) => {
                    let mut changed = false;
                    for (label, value) in ["B", "G", "R", "A"].into_iter().zip(values) {
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix(format!("{label}: ")))
                            .changed();
                    }
                    changed
                }
                NativeStruct::Box3d { min, max, is_valid } => {
                    ui.label("Minimum");
                    let mut changed = draw_chimp_f64_values(ui, min);
                    ui.label("Maximum");
                    changed |= draw_chimp_f64_values(ui, max);
                    changed |= ui
                        .add(egui::DragValue::new(is_valid).prefix("Valid: "))
                        .changed();
                    changed
                }
                NativeStruct::RichCurveKey {
                    interp_mode,
                    tangent_mode,
                    tangent_weight_mode,
                    values,
                } => {
                    let mut changed = ui
                        .add(egui::DragValue::new(interp_mode).prefix("Interpolation: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(tangent_mode).prefix("Tangent: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(tangent_weight_mode).prefix("Weight: "))
                        .changed();
                    changed |= draw_chimp_f32_values(ui, values);
                    changed
                }
                NativeStruct::FontCharacter {
                    start_u,
                    start_v,
                    size_u,
                    size_v,
                    texture_index,
                    vertical_offset,
                } => {
                    let mut changed = false;
                    for (label, value) in [
                        ("Start U", start_u),
                        ("Start V", start_v),
                        ("Size U", size_u),
                        ("Size V", size_v),
                        ("Vertical offset", vertical_offset),
                    ] {
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix(format!("{label}: ")))
                            .changed();
                    }
                    changed |= ui
                        .add(egui::DragValue::new(texture_index).prefix("Texture: "))
                        .changed();
                    changed
                }
                NativeStruct::PackedBits(value) => ui
                    .add(egui::DragValue::new(value).prefix("Bits: "))
                    .changed(),
                NativeStruct::I32(value) => ui.add(egui::DragValue::new(value)).changed(),
                NativeStruct::I64(value) => ui.add(egui::DragValue::new(value)).changed(),
                NativeStruct::FrameRange {
                    lower_kind,
                    lower,
                    upper_kind,
                    upper,
                } => {
                    let mut changed = ui
                        .add(egui::DragValue::new(lower_kind).prefix("Lower kind: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(lower).prefix("Lower: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(upper_kind).prefix("Upper kind: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(upper).prefix("Upper: "))
                        .changed();
                    changed
                }
                NativeStruct::EvaluationKey(values) => {
                    let mut changed = false;
                    for (label, value) in ["Sequence", "Track", "Section"].into_iter().zip(values) {
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix(format!("{label}: ")))
                            .changed();
                    }
                    changed
                }
                NativeStruct::PerPlatform { cooked, value } => {
                    let mut changed = ui.checkbox(cooked, "Cooked").changed();
                    changed |= match value {
                        PerPlatformValue::Int(value) => {
                            ui.add(egui::DragValue::new(value)).changed()
                        }
                        PerPlatformValue::Float(value) => {
                            ui.add(egui::DragValue::new(value).speed(0.01)).changed()
                        }
                        PerPlatformValue::Bool(value) => ui.checkbox(value, "Value").changed(),
                        PerPlatformValue::FrameRate(numerator, denominator) => {
                            let mut inner = ui
                                .add(egui::DragValue::new(numerator).prefix("Numerator: "))
                                .changed();
                            inner |= ui
                                .add(egui::DragValue::new(denominator).prefix("Denominator: "))
                                .changed();
                            inner
                        }
                    };
                    changed
                }
                NativeStruct::EmptySerializer => {
                    ui.label("No serialized fields");
                    false
                }
            };
        });
    changed
}

fn draw_chimp_optional_f32(ui: &mut Ui, label: &str, value: &mut Option<f32>) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, label).changed();
    if present && value.is_none() {
        *value = Some(0.0);
    } else if !present {
        *value = None;
    }
    if let Some(value) = value {
        changed |= ui.add(egui::DragValue::new(value).speed(0.01)).changed();
    }
    changed
}

fn draw_chimp_optional_i32(ui: &mut Ui, label: &str, value: &mut Option<i32>) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, label).changed();
    if present && value.is_none() {
        *value = Some(0);
    } else if !present {
        *value = None;
    }
    if let Some(value) = value {
        changed |= ui.add(egui::DragValue::new(value)).changed();
    }
    changed
}

fn draw_chimp_optional_u64(ui: &mut Ui, label: &str, value: &mut Option<u64>) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, label).changed();
    if present && value.is_none() {
        *value = Some(0);
    } else if !present {
        *value = None;
    }
    if let Some(value) = value {
        changed |= ui.add(egui::DragValue::new(value)).changed();
    }
    changed
}

fn draw_chimp_tree_entry(ui: &mut Ui, value: &mut chimp_hw::TreeEntry) -> bool {
    let mut changed = ui
        .add(egui::DragValue::new(&mut value.start).prefix("Start: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.size).prefix("Size: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.capacity).prefix("Capacity: "))
        .changed();
    changed
}

fn draw_chimp_tree_node(ui: &mut Ui, value: &mut chimp_hw::TreeNode) -> bool {
    let mut changed = false;
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_lower_kind).prefix("Lower kind: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_lower).prefix("Lower: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_upper_kind).prefix("Upper kind: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_upper).prefix("Upper: "))
        .changed();
    for (label, item) in [
        ("Parent children", &mut value.parent_children_handle),
        ("Parent index", &mut value.parent_index),
        ("Children id", &mut value.children_id),
        ("Data id", &mut value.data_id),
    ] {
        changed |= ui
            .add(egui::DragValue::new(item).prefix(format!("{label}: ")))
            .changed();
    }
    changed
}

fn draw_chimp_shader_value(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::ShaderValueType,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    let mut changed = ui
        .add(egui::DragValue::new(&mut value.kind).prefix("Kind: "))
        .changed();
    changed |= ui
        .checkbox(&mut value.is_dynamic_array, "Dynamic array")
        .changed();
    match &mut value.body {
        chimp_hw::ShaderValueTypeBody::Struct { name, elements } => {
            changed |= draw_chimp_fname(ui, "Struct", name, names);
            changed |= draw_chimp_list(
                ui,
                id.with("elements"),
                "Elements",
                elements,
                None,
                |ui, index, (name, value)| {
                    let mut item_changed = draw_chimp_fname(ui, "Name", name, names);
                    item_changed |=
                        draw_chimp_shader_value(ui, id.with(("element", index)), value, names);
                    item_changed
                },
            );
        }
        chimp_hw::ShaderValueTypeBody::Dimension { dimension, counts } => {
            changed |= ui
                .add(egui::DragValue::new(dimension).prefix("Dimension: "))
                .changed();
            changed |= draw_chimp_list(
                ui,
                id.with("counts"),
                "Counts",
                counts,
                Some(0),
                |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
            );
        }
    }
    changed
}

fn draw_chimp_text_argument(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::TextFormatArgument,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    match value {
        chimp_hw::TextFormatArgument::Int(value) => ui.add(egui::DragValue::new(value)).changed(),
        chimp_hw::TextFormatArgument::UInt(value) | chimp_hw::TextFormatArgument::Gender(value) => {
            ui.add(egui::DragValue::new(value)).changed()
        }
        chimp_hw::TextFormatArgument::Float(value) => {
            ui.add(egui::DragValue::new(value).speed(0.01)).changed()
        }
        chimp_hw::TextFormatArgument::Double(value) => {
            ui.add(egui::DragValue::new(value).speed(0.01)).changed()
        }
        chimp_hw::TextFormatArgument::Text(value) => {
            draw_chimp_text(ui, id.with("text"), value, names)
        }
    }
}

fn draw_chimp_text(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::TextValue,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    let mut changed = ui
        .add(egui::DragValue::new(&mut value.flags).prefix("Flags: "))
        .changed();
    match &mut value.history {
        chimp_hw::TextHistory::None { culture_invariant } => {
            let mut present = culture_invariant.is_some();
            if ui
                .checkbox(&mut present, "Culture-invariant string")
                .changed()
            {
                *culture_invariant = present.then(Default::default);
                changed = true;
            }
            if let Some(text) = culture_invariant {
                changed |= draw_chimp_fstr(ui, "Value", text);
            }
        }
        chimp_hw::TextHistory::Base {
            namespace,
            key,
            source,
        } => {
            changed |= draw_chimp_fstr(ui, "Namespace", namespace);
            changed |= draw_chimp_fstr(ui, "Key", key);
            changed |= draw_chimp_fstr(ui, "Source", source);
        }
        chimp_hw::TextHistory::StringTableEntry { table_id, key } => {
            changed |= draw_chimp_fname(ui, "Table", table_id, names);
            changed |= draw_chimp_fstr(ui, "Key", key);
        }
        chimp_hw::TextHistory::OrderedFormat {
            source_fmt,
            arguments,
        } => {
            changed |= draw_chimp_text(ui, id.with("source"), source_fmt, names);
            changed |= draw_chimp_list(
                ui,
                id.with("args"),
                "Arguments",
                arguments,
                None,
                |ui, index, argument| draw_chimp_text_argument(ui, id.with(index), argument, names),
            );
        }
        chimp_hw::TextHistory::NamedFormat {
            kind,
            source_fmt,
            arguments,
        } => {
            changed |= ui
                .add(egui::DragValue::new(kind).prefix("Kind: "))
                .changed();
            changed |= draw_chimp_text(ui, id.with("source"), source_fmt, names);
            changed |= draw_chimp_list(
                ui,
                id.with("args"),
                "Arguments",
                arguments,
                None,
                |ui, index, (name, argument)| {
                    let mut item_changed = draw_chimp_fstr(ui, "Name", name);
                    item_changed |= draw_chimp_text_argument(ui, id.with(index), argument, names);
                    item_changed
                },
            );
        }
        chimp_hw::TextHistory::AsNumber {
            kind,
            currency_code,
            source_value,
            options,
            target_culture,
        } => {
            changed |= ui
                .add(egui::DragValue::new(kind).prefix("Kind: "))
                .changed();
            if let Some(currency) = currency_code {
                changed |= draw_chimp_fstr(ui, "Currency", currency);
            }
            changed |= draw_chimp_text_argument(ui, id.with("value"), source_value, names);
            if let Some(options) = options {
                changed |= ui
                    .checkbox(&mut options.always_sign, "Always sign")
                    .changed();
                changed |= ui
                    .checkbox(&mut options.use_grouping, "Use grouping")
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut options.rounding_mode).prefix("Rounding: "))
                    .changed();
                for (label, field) in [
                    ("Minimum integral", &mut options.minimum_integral_digits),
                    ("Maximum integral", &mut options.maximum_integral_digits),
                    ("Minimum fractional", &mut options.minimum_fractional_digits),
                    ("Maximum fractional", &mut options.maximum_fractional_digits),
                ] {
                    changed |= ui
                        .add(egui::DragValue::new(field).prefix(format!("{label}: ")))
                        .changed();
                }
            }
            changed |= draw_chimp_fstr(ui, "Culture", target_culture);
        }
        chimp_hw::TextHistory::AsDateTime {
            kind,
            source_date_time,
            date_style,
            time_style,
            custom_pattern,
            time_zone,
            target_culture,
        } => {
            changed |= ui
                .add(egui::DragValue::new(kind).prefix("Kind: "))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(source_date_time).prefix("Ticks: "))
                .changed();
            for (label, value) in [("Date style", date_style), ("Time style", time_style)] {
                let mut present = value.is_some();
                if ui.checkbox(&mut present, label).changed() {
                    *value = present.then_some(0);
                    changed = true;
                }
                if let Some(value) = value {
                    changed |= ui.add(egui::DragValue::new(value)).changed();
                }
            }
            if let Some(pattern) = custom_pattern {
                changed |= draw_chimp_fstr(ui, "Pattern", pattern);
            }
            changed |= draw_chimp_fstr(ui, "Time zone", time_zone);
            changed |= draw_chimp_fstr(ui, "Culture", target_culture);
        }
        chimp_hw::TextHistory::Transform {
            source_text,
            transform_type,
        } => {
            changed |= draw_chimp_text(ui, id.with("source"), source_text, names);
            changed |= ui
                .add(egui::DragValue::new(transform_type).prefix("Transform: "))
                .changed();
        }
        chimp_hw::TextHistory::TextGenerator {
            generator_type_id,
            contents,
        } => {
            changed |= draw_chimp_fname(ui, "Generator", generator_type_id, names);
            ui.label(match contents {
                Some(bytes) => format!("{} generator bytes · preserved read-only", bytes.len()),
                None => "No generator payload".to_owned(),
            });
        }
    }
    changed
}

fn draw_chimp_sampler(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::WeightedRandomSampler,
) -> bool {
    let mut changed = draw_chimp_list(
        ui,
        id.with("prob"),
        "Probabilities",
        &mut value.prob,
        Some(0.0),
        |ui, _, value| ui.add(egui::DragValue::new(value).speed(0.01)).changed(),
    );
    changed |= draw_chimp_list(
        ui,
        id.with("alias"),
        "Aliases",
        &mut value.alias,
        Some(0),
        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
    );
    changed |= ui
        .add(egui::DragValue::new(&mut value.total_weight).prefix("Total weight: "))
        .changed();
    changed
}

fn draw_chimp_property_bag_type(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::PropertyBagPropertyType,
) -> bool {
    use chimp_hw::PropertyBagPropertyType as T;
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for candidate in [
                T::None,
                T::Bool,
                T::Byte,
                T::Int32,
                T::Int64,
                T::Float,
                T::Double,
                T::Name,
                T::String,
                T::Text,
                T::Enum,
                T::Struct,
                T::Object,
                T::SoftObject,
                T::Class,
                T::SoftClass,
                T::UInt32,
                T::UInt64,
            ] {
                changed |= ui
                    .selectable_value(value, candidate, format!("{candidate:?}"))
                    .changed();
            }
            if matches!(value, T::Unknown(_)) {
                ui.label("Unknown type is preserved read-only");
            }
        });
    changed
}

fn draw_chimp_property_bag_container(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::PropertyBagContainerType,
) -> bool {
    use chimp_hw::PropertyBagContainerType as T;
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for candidate in [T::None, T::Array, T::Set] {
                changed |= ui
                    .selectable_value(value, candidate, format!("{candidate:?}"))
                    .changed();
            }
            if matches!(value, T::Unknown(_)) {
                ui.label("Unknown container is preserved read-only");
            }
        });
    changed
}

fn draw_chimp_hand_written(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::HandWritten,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Typed Unreal structure")
        .id_salt(id)
        .show(ui, |ui| {
            changed |= match value {
                chimp_hw::HandWritten::MaterialLayersTree(value) => {
                    let mut inner = draw_chimp_list(
                        ui,
                        id.with("nodes"),
                        "Nodes",
                        &mut value.nodes,
                        Some([0; 4]),
                        |ui, _, values| {
                            let mut changed = false;
                            for value in values {
                                changed |= ui.add(egui::DragValue::new(value)).changed();
                            }
                            changed
                        },
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("payloads"),
                        "Payloads",
                        &mut value.payloads,
                        Some([0; 2]),
                        |ui, _, values| {
                            values.iter_mut().fold(false, |changed, value| {
                                ui.add(egui::DragValue::new(value)).changed() || changed
                            })
                        },
                    );
                    inner |= ui
                        .add(egui::DragValue::new(&mut value.root).prefix("Root: "))
                        .changed();
                    inner
                }
                chimp_hw::HandWritten::MovieSceneInlineValue(value) => {
                    let mut inner = draw_chimp_fstr(ui, "Type", &mut value.type_name);
                    if let Some(payload) = &mut value.payload {
                        let class = value.type_name.to_string();
                        inner |=
                            draw_chimp_property_block(ui, payload, &class, names, usmap, depth + 1);
                    }
                    inner
                }
                chimp_hw::HandWritten::EvaluationTree(value) => {
                    let mut inner = draw_chimp_tree_node(ui, &mut value.root);
                    inner |= draw_chimp_list(
                        ui,
                        id.with("child_entries"),
                        "Child entries",
                        &mut value.child_entries,
                        Some(chimp_hw::TreeEntry {
                            start: 0,
                            size: 0,
                            capacity: 0,
                        }),
                        |ui, _, value| draw_chimp_tree_entry(ui, value),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("child_nodes"),
                        "Child nodes",
                        &mut value.child_nodes,
                        None,
                        |ui, _, value| draw_chimp_tree_node(ui, value),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("data_entries"),
                        "Data entries",
                        &mut value.data_entries,
                        Some(chimp_hw::TreeEntry {
                            start: 0,
                            size: 0,
                            capacity: 0,
                        }),
                        |ui, _, value| draw_chimp_tree_entry(ui, value),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("items"),
                        "Items",
                        &mut value.items,
                        None,
                        |ui, _, value| match value {
                            chimp_hw::TreeItem::EntityAndMetaDataIndex { entity, meta_data } => {
                                ui.add(egui::DragValue::new(entity).prefix("Entity: "))
                                    .changed()
                                    | ui.add(egui::DragValue::new(meta_data).prefix("Metadata: "))
                                        .changed()
                            }
                            chimp_hw::TreeItem::SubSequence { sequence_id, flags } => {
                                ui.add(egui::DragValue::new(sequence_id).prefix("Sequence: "))
                                    .changed()
                                    | ui.add(egui::DragValue::new(flags).prefix("Flags: "))
                                        .changed()
                            }
                        },
                    );
                    inner
                }
                chimp_hw::HandWritten::ShaderValueType(value) => {
                    draw_chimp_shader_value(ui, id, value, names)
                }
                chimp_hw::HandWritten::PerQualityLevel(value) => {
                    let mut inner = ui.checkbox(&mut value.cooked, "Cooked").changed();
                    inner |= ui
                        .add(egui::DragValue::new(&mut value.default_bits).prefix("Default bits: "))
                        .changed();
                    inner |= draw_chimp_list(
                        ui,
                        id.with("overrides"),
                        "Overrides",
                        &mut value.overrides,
                        Some((0, 0)),
                        |ui, _, (quality, bits)| {
                            ui.add(egui::DragValue::new(quality).prefix("Quality: "))
                                .changed()
                                | ui.add(egui::DragValue::new(bits).prefix("Bits: "))
                                    .changed()
                        },
                    );
                    inner
                }
                chimp_hw::HandWritten::FontData(value) => {
                    let mut inner = ui
                        .add(
                            egui::DragValue::new(&mut value.font_face_asset).prefix("Face asset: "),
                        )
                        .changed();
                    let mut inline = value.inline_face.is_some();
                    if ui.checkbox(&mut inline, "Inline face").changed() {
                        value.inline_face = inline.then(|| chimp_hw::InlineFontFace {
                            filename: Default::default(),
                            hinting: 0,
                            loading_policy: 0,
                        });
                        inner = true;
                    }
                    if let Some(face) = &mut value.inline_face {
                        inner |= draw_chimp_fstr(ui, "Filename", &mut face.filename);
                        inner |= ui
                            .add(egui::DragValue::new(&mut face.hinting).prefix("Hinting: "))
                            .changed();
                        inner |= ui
                            .add(egui::DragValue::new(&mut face.loading_policy).prefix("Loading: "))
                            .changed();
                    }
                    inner |= ui
                        .add(egui::DragValue::new(&mut value.sub_face_index).prefix("Sub-face: "))
                        .changed();
                    inner
                }
                chimp_hw::HandWritten::MaterialOverrideNanite(value) => {
                    let mut inner = ui.checkbox(&mut value.cooked, "Cooked").changed();
                    inner |= draw_chimp_optional_i32(
                        ui,
                        "Override material",
                        &mut value.override_material,
                    );
                    inner |= draw_chimp_property_block(
                        ui,
                        &mut value.properties,
                        "MaterialOverrideNanite",
                        names,
                        usmap,
                        depth + 1,
                    );
                    inner
                }
                chimp_hw::HandWritten::TimeWarpVariant(value) => match value {
                    chimp_hw::TimeWarpVariant::Literal(value) => {
                        ui.add(egui::DragValue::new(value).speed(0.01)).changed()
                    }
                    chimp_hw::TimeWarpVariant::Typed {
                        kind,
                        object,
                        payload,
                    } => {
                        let mut inner = ui
                            .add(egui::DragValue::new(kind).prefix("Kind: "))
                            .changed();
                        inner |= draw_chimp_optional_i32(ui, "Object", object);
                        if let Some(payload) = payload {
                            inner |=
                                draw_chimp_property_block(ui, payload, "", names, usmap, depth + 1);
                        }
                        inner
                    }
                },
                chimp_hw::HandWritten::LocatorFragment(value) => {
                    let mut inner =
                        draw_chimp_fname(ui, "Fragment type", &mut value.fragment_type, names);
                    if let Some(payload) = &mut value.payload {
                        let class = value.fragment_type.to_string();
                        inner |=
                            draw_chimp_property_block(ui, payload, &class, names, usmap, depth + 1);
                    }
                    inner
                }
                chimp_hw::HandWritten::Text(value) => draw_chimp_text(ui, id, value, names),
                chimp_hw::HandWritten::MovieSceneChannel(value) => {
                    let mut inner = ui
                        .add(
                            egui::DragValue::new(&mut value.pre_infinity_extrap)
                                .prefix("Pre extrapolation: "),
                        )
                        .changed();
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.post_infinity_extrap)
                                .prefix("Post extrapolation: "),
                        )
                        .changed();
                    ui.label(format!("{} time bytes · preserved", value.times.data.len()));
                    ui.label(format!(
                        "{} value bytes · preserved",
                        value.values.data.len()
                    ));
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.default_value)
                                .speed(0.01)
                                .prefix("Default: "),
                        )
                        .changed();
                    inner |= ui
                        .checkbox(&mut value.has_default_value, "Has default")
                        .changed();
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.tick_resolution_numerator)
                                .prefix("Tick numerator: "),
                        )
                        .changed();
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.tick_resolution_denominator)
                                .prefix("Tick denominator: "),
                        )
                        .changed();
                    inner |= ui.checkbox(&mut value.show_curve, "Show curve").changed();
                    inner
                }
                chimp_hw::HandWritten::PcgPoint(value) => {
                    let mut inner = draw_chimp_f64_values(ui, &mut value.transform);
                    inner |= draw_chimp_optional_f32(ui, "Density", &mut value.density);
                    for (label, point) in [
                        ("Bounds minimum", &mut value.bounds_min),
                        ("Bounds maximum", &mut value.bounds_max),
                    ] {
                        let mut present = point.is_some();
                        if ui.checkbox(&mut present, label).changed() {
                            *point = present.then_some([0.0; 3]);
                            inner = true;
                        }
                        if let Some(point) = point {
                            inner |= draw_chimp_f64_values(ui, point);
                        }
                    }
                    let mut color = value.color.is_some();
                    if ui.checkbox(&mut color, "Color").changed() {
                        value.color = color.then_some([0.0; 4]);
                        inner = true;
                    }
                    if let Some(color) = &mut value.color {
                        inner |= draw_chimp_f64_values(ui, color);
                    }
                    inner |= draw_chimp_optional_f32(ui, "Steepness", &mut value.steepness);
                    inner |= draw_chimp_optional_i32(ui, "Seed", &mut value.seed);
                    inner |=
                        draw_chimp_optional_u64(ui, "Metadata entry", &mut value.metadata_entry);
                    inner
                }
                chimp_hw::HandWritten::SkeletalMeshSamplingLod(value) => {
                    draw_chimp_sampler(ui, id, value)
                }
                chimp_hw::HandWritten::SkeletalMeshSamplingRegion(value) => {
                    let mut inner = draw_chimp_list(
                        ui,
                        id.with("triangles"),
                        "Triangle indices",
                        &mut value.triangle_indices,
                        Some(0),
                        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("bones"),
                        "Bone indices",
                        &mut value.bone_indices,
                        Some(0),
                        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
                    );
                    inner |= draw_chimp_sampler(ui, id.with("sampler"), &mut value.sampler);
                    inner |= draw_chimp_list(
                        ui,
                        id.with("vertices"),
                        "Vertices",
                        &mut value.vertices,
                        Some(0),
                        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
                    );
                    inner
                }
                chimp_hw::HandWritten::NiagaraVariable(value) => {
                    let mut inner = draw_chimp_fname(ui, "Name", &mut value.name, names);
                    inner |= draw_chimp_property_block(
                        ui,
                        &mut value.type_def,
                        "NiagaraTypeDefinition",
                        names,
                        usmap,
                        depth + 1,
                    );
                    match &mut value.payload {
                        chimp_hw::NiagaraPayload::None => {}
                        chimp_hw::NiagaraPayload::Offset(value) => {
                            inner |= ui
                                .add(egui::DragValue::new(value).prefix("Offset: "))
                                .changed()
                        }
                        chimp_hw::NiagaraPayload::VarData(bytes) => {
                            ui.label(format!(
                                "{} variable-data bytes · preserved read-only",
                                bytes.len()
                            ));
                        }
                    }
                    inner
                }
                chimp_hw::HandWritten::NiagaraGpuParamInfo(value) => {
                    let mut inner = draw_chimp_fstr(ui, "HLSL symbol", &mut value.hlsl_symbol);
                    inner |= draw_chimp_fstr(ui, "DI class", &mut value.di_class_name);
                    inner |= draw_chimp_list(
                        ui,
                        id.with("functions"),
                        "Generated functions",
                        &mut value.generated_functions,
                        None,
                        |ui, index, function| {
                            let mut item = draw_chimp_fname(
                                ui,
                                "Definition",
                                &mut function.definition_name,
                                names,
                            );
                            item |= draw_chimp_fstr(ui, "Instance", &mut function.instance_name);
                            item |= draw_chimp_list(
                                ui,
                                id.with((index, "specifiers")),
                                "Specifiers",
                                &mut function.specifiers,
                                Some((
                                    blam_tags::iostore::object::value::FName::none(),
                                    blam_tags::iostore::object::value::FName::none(),
                                )),
                                |ui, _, (name, value)| {
                                    draw_chimp_fname(ui, "Name", name, names)
                                        | draw_chimp_fname(ui, "Value", value, names)
                                },
                            );
                            let default_reference = chimp_hw::NiagaraVariableCommonReference {
                                name: blam_tags::iostore::object::value::FName::none(),
                                underlying_type: 0,
                            };
                            item |= draw_chimp_list(
                                ui,
                                id.with((index, "inputs")),
                                "Variadic inputs",
                                &mut function.variadic_inputs,
                                Some(default_reference.clone()),
                                |ui, _, value| {
                                    draw_chimp_fname(ui, "Name", &mut value.name, names)
                                        | ui.add(
                                            egui::DragValue::new(&mut value.underlying_type)
                                                .prefix("Type: "),
                                        )
                                        .changed()
                                },
                            );
                            item |= draw_chimp_list(
                                ui,
                                id.with((index, "outputs")),
                                "Variadic outputs",
                                &mut function.variadic_outputs,
                                Some(default_reference),
                                |ui, _, value| {
                                    draw_chimp_fname(ui, "Name", &mut value.name, names)
                                        | ui.add(
                                            egui::DragValue::new(&mut value.underlying_type)
                                                .prefix("Type: "),
                                        )
                                        .changed()
                                },
                            );
                            item
                        },
                    );
                    inner
                }
                chimp_hw::HandWritten::InstancedPropertyBag(value) => {
                    let mut inner = ui
                        .add(egui::DragValue::new(&mut value.serial_size).prefix("Serial size: "))
                        .changed();
                    if let Some(descriptors) = &mut value.descriptors {
                        let default_descriptor = chimp_hw::PropertyBagDesc {
                            value_type_object: 0,
                            id: Default::default(),
                            name: blam_tags::iostore::object::value::FName::none(),
                            value_type: chimp_hw::PropertyBagPropertyType::None,
                            container_types: Vec::new(),
                        };
                        inner |= draw_chimp_list(
                            ui,
                            id.with("descriptors"),
                            "Descriptors",
                            descriptors,
                            Some(default_descriptor),
                            |ui, index, descriptor| {
                                let mut item = ui
                                    .add(
                                        egui::DragValue::new(&mut descriptor.value_type_object)
                                            .prefix("Type object: "),
                                    )
                                    .changed();
                                item |= draw_chimp_fname(ui, "Name", &mut descriptor.name, names);
                                ui.label("ID");
                                item |= draw_chimp_i32_values(ui, &mut descriptor.id.0);
                                ui.horizontal(|ui| {
                                    ui.label("Type");
                                    item |= draw_chimp_property_bag_type(
                                        ui,
                                        id.with((index, "type")),
                                        &mut descriptor.value_type,
                                    );
                                });
                                item |= draw_chimp_list(
                                    ui,
                                    id.with((index, "containers")),
                                    "Containers",
                                    &mut descriptor.container_types,
                                    Some(chimp_hw::PropertyBagContainerType::None),
                                    |ui, container, value| {
                                        draw_chimp_property_bag_container(
                                            ui,
                                            id.with((index, container)),
                                            value,
                                        )
                                    },
                                );
                                item
                            },
                        );
                    }
                    if let Some(values) = &mut value.values {
                        inner |= draw_chimp_property_block(ui, values, "", names, usmap, depth + 1);
                    }
                    inner
                }
            };
        });
    changed
}

fn chimp_class_display_name(class: Option<&str>) -> &str {
    class
        .and_then(|class| {
            class
                .rsplit(|character| character == '.' || character == '/')
                .find(|segment| !segment.is_empty())
        })
        .unwrap_or("Unknown")
}

fn chimp_object_index_json(
    document: &ChimpDocument,
    world: &World,
    index: FPackageObjectIndex,
) -> Value {
    let object_path = match index.kind() {
        FPackageObjectIndexType::ScriptImport => world
            .class_path(index.raw_index())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("script import #{:016X}", index.raw_index())),
        FPackageObjectIndexType::PackageImport => index
            .package_import()
            .and_then(|reference| {
                let package = document
                    .header
                    .imported_package_names
                    .get(reference.imported_package_index as usize)?;
                let hash = document
                    .header
                    .imported_public_export_hashes
                    .get(reference.imported_public_export_hash_index as usize)?;
                Some(format!("{package}#{hash:016X}"))
            })
            .unwrap_or_else(|| format!("package import #{:016X}", index.raw_index())),
        FPackageObjectIndexType::Export => document
            .header
            .export_map
            .get(index.raw_index() as usize)
            .map(|export| {
                format!(
                    "{}.{}",
                    document.package,
                    document.header.name_map.get(export.object_name)
                )
            })
            .unwrap_or_else(|| format!("export #{}", index.raw_index())),
        FPackageObjectIndexType::Null => "None".to_owned(),
    };
    json!({
        "Kind": format!("{:?}", index.kind()),
        "RawIndex": format!("0x{:016X}", index.raw_index()),
        "ObjectPath": object_path,
    })
}

fn chimp_document_json(document: &ChimpDocument) -> Value {
    json!({
        "Package": document.package,
        "Source": document.provider.entry_path,
        "Summary": {
            "Exports": document.header.export_map.len(),
            "Imports": document.header.import_map.len(),
            "Names": document.header.name_map.copy_raw_names().len(),
            "OriginalSize": document.original.len(),
        },
        "Imports": document.header.imported_package_names,
        "ExternalDependencies": document.header.external_package_dependencies
            .iter()
            .map(|dependency| format!("{dependency:?}"))
            .collect::<Vec<_>>(),
        "Exports": document.exports.iter().map(|export| {
            json!({
                "Type": chimp_class_display_name(export.class.as_deref()),
                "Name": export.object,
                "Class": export.class,
                "Properties": export.decoded.as_ref().ok()
                    .and_then(Export::properties)
                    .map(chimp_block_json),
                "DecodeError": export.decoded.as_ref().err(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn chimp_metadata_json(document: &ChimpDocument, world: &World) -> Value {
    let header = &document.header;
    let summary = &header.summary;
    let record = world.package(&document.package);
    json!({
        "Summary": {
            "Package": document.package,
            "SourcePackage": header.source_package_name(),
            "PackageFlags": format!("0x{:08X}", summary.package_flags),
            "HeaderSize": summary.header_size,
            "CookedHeaderSize": summary.cooked_header_size,
            "OriginalSize": document.original.len(),
            "HasVersioningInfo": summary.has_versioning_info != 0,
            "IsUnversioned": header.is_unversioned,
            "ContainerHeaderVersion": format!("{:?}", header.container_header_version),
            "ExportCount": header.export_map.len(),
            "ImportCount": header.import_map.len(),
            "NameCount": header.name_map.copy_raw_names().len(),
            "BulkDataCount": header.bulk_data.len(),
            "ImportedPackageCount": header.imported_package_names.len(),
            "ExternalDependencyCount": header.external_package_dependencies.len(),
        },
        "Versioning": {
            "ZenVersion": format!("{:?}", header.versioning_info.zen_version),
            "FileVersionUE4": header.versioning_info.package_file_version.file_version_ue4,
            "FileVersionUE5": header.versioning_info.package_file_version.file_version_ue5,
            "LicenseeVersion": header.versioning_info.licensee_version,
            "CustomVersions": header.versioning_info.custom_versions.iter().map(|version| {
                json!({
                    "Key": format!("{:?}", version.key),
                    "Version": version.version,
                })
            }).collect::<Vec<_>>(),
        },
        "SectionOffsets": summary.section_offsets().iter().map(|(section, offset)| {
            json!({
                "Section": section,
                "Offset": offset,
            })
        }).collect::<Vec<_>>(),
        "NameMap": header.name_map.copy_raw_names(),
        "ImportedPackageNames": header.imported_package_names,
        "ImportedPublicExportHashes": header.imported_public_export_hashes.iter()
            .map(|hash| format!("0x{hash:016X}"))
            .collect::<Vec<_>>(),
        "ImportMap": header.import_map.iter().enumerate().map(|(index, object)| {
            json!({
                "Index": index,
                "Object": chimp_object_index_json(document, world, *object),
            })
        }).collect::<Vec<_>>(),
        "ExportMap": header.export_map.iter().enumerate().map(|(index, export)| {
            json!({
                "Index": index,
                "ObjectName": header.name_map.get(export.object_name).to_string(),
                "Class": world.class_key(header, export.class_index),
                "Outer": chimp_object_index_json(document, world, export.outer_index),
                "Super": chimp_object_index_json(document, world, export.super_index),
                "Template": chimp_object_index_json(document, world, export.template_index),
                "SerialOffset": export.cooked_serial_offset,
                "SerialSize": export.cooked_serial_size,
                "PublicExportHash": format!("0x{:016X}", export.public_export_hash),
                "ObjectFlags": format!("0x{:08X}", export.object_flags),
                "FilterFlags": format!("{:?}", export.filter_flags),
            })
        }).collect::<Vec<_>>(),
        "BulkDataMap": header.bulk_data.iter().enumerate().map(|(index, entry)| {
            json!({
                "Index": index,
                "SerialOffset": entry.serial_offset,
                "DuplicateSerialOffset": entry.duplicate_serial_offset,
                "SerialSize": entry.serial_size,
                "Flags": format!("0x{:08X}", entry.flags),
                "CookedIndex": entry.cooked_index,
            })
        }).collect::<Vec<_>>(),
        "ExternalPackageDependencies": header.external_package_dependencies.iter()
            .map(|dependency| {
                json!({
                    "FromPackageId": format!("0x{:016X}", dependency.from_package_id.0),
                    "ExternalArcs": dependency.external_dependency_arcs.iter().map(|arc| {
                        json!({
                            "FromImportIndex": arc.from_import_index,
                            "FromCommandType": format!("{:?}", arc.from_command_type),
                            "ToExportBundleIndex": arc.to_export_bundle_index,
                        })
                    }).collect::<Vec<_>>(),
                    "LegacyArcs": dependency.legacy_dependency_arcs.iter().map(|arc| {
                        json!({
                            "FromExportBundleIndex": arc.from_export_bundle_index,
                            "ToExportBundleIndex": arc.to_export_bundle_index,
                        })
                    }).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        "ShaderMapHashes": header.shader_map_hashes.iter()
            .map(|hash| format!("{hash:?}"))
            .collect::<Vec<_>>(),
        "PhysicalProviders": record.map(|record| {
            record.providers.iter().rev().map(|provider| {
                let container = &world.containers()[provider.container];
                json!({
                    "Active": record.active_provider() == Some(provider),
                    "Container": container.path.display().to_string(),
                    "EntryPath": provider.entry_path,
                    "ReadOrder": provider.read_order,
                    "RecoveredDirectoryIndex": container.recovered_directory_index,
                })
            }).collect::<Vec<_>>()
        }).unwrap_or_default(),
    })
}

fn refresh_chimp_document_text(document: &mut ChimpDocument) {
    document.document_text = serde_json::to_string_pretty(&chimp_document_json(document))
        .unwrap_or_else(|error| format!("Could not render package document: {error}"));
    document.document_line_numbers = chimp_line_numbers(&document.document_text);
    document.document_text_dirty = false;
}

fn refresh_chimp_metadata_text(document: &mut ChimpDocument, world: &World) {
    document.metadata_text = serde_json::to_string_pretty(&chimp_metadata_json(document, world))
        .unwrap_or_else(|error| format!("Could not render package metadata: {error}"));
    document.metadata_line_numbers = chimp_line_numbers(&document.metadata_text);
    document.metadata_text_dirty = false;
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
    fn chimp_scalar_property_row_does_not_consume_the_scroll_viewport() {
        let context = egui::Context::default();
        let mut value = 0_i64;
        let mut row_height = None;
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
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let row = ui.horizontal_top(|ui| {
                            ui.set_min_height(24.0);
                            ui.label("NumReplicatedProperties");
                            chimp_property_value_cell(ui, |ui| {
                                ui.add(egui::DragValue::new(&mut value));
                            });
                        });
                        row_height = Some(row.response.rect.height());
                    });
                });
            },
        );

        let row_height = row_height.expect("the property row was rendered");
        assert!(
            row_height <= 40.0,
            "a scalar property row expanded to {row_height}px"
        );
    }

    #[test]
    fn chimp_mod_names_are_sanitized_and_priority_suffixed() {
        assert_eq!(chimp_mod_stem("My Cool Mod"), "My-Cool-Mod_P");
        assert_eq!(chimp_mod_stem("Already_p"), "Already_p");
        assert_eq!(chimp_mod_stem("../../unsafe"), "unsafe_P");
    }

    #[test]
    fn chimp_mod_stem_rejects_an_empty_name_at_the_dialog_boundary() {
        assert!(sanitize_mod_name(" ! ").is_empty());
        assert_eq!(chimp_mod_stem(" ! "), "_P");
    }

    fn preview_normal_alignment(preview: &ModelPreviewData) -> (f32, f32) {
        // Unreal's source winding is left-handed, so the sign relative to this
        // right-handed cross product is expected to be negative. Magnitude is
        // the useful regression signal: broken packed normals are incoherent.
        let mut signed = 0.0;
        let mut absolute = 0.0;
        let mut count = 0usize;
        for triangle in preview.preview.indices.chunks_exact(3) {
            let Some(a) = preview.preview.vertices.get(triangle[0] as usize) else {
                continue;
            };
            let Some(b) = preview.preview.vertices.get(triangle[1] as usize) else {
                continue;
            };
            let Some(c) = preview.preview.vertices.get(triangle[2] as usize) else {
                continue;
            };
            let [a, b, c] = [a.position, b.position, c.position];
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let face = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let length = (face[0] * face[0] + face[1] * face[1] + face[2] * face[2]).sqrt();
            if length <= 1.0e-6 {
                continue;
            }
            let face = [face[0] / length, face[1] / length, face[2] / length];
            for normal in [
                preview.preview.vertices[triangle[0] as usize].normal,
                preview.preview.vertices[triangle[1] as usize].normal,
                preview.preview.vertices[triangle[2] as usize].normal,
            ] {
                let dot = face[0] * normal[0] + face[1] * normal[1] + face[2] * normal[2];
                signed += dot;
                absolute += dot.abs();
                count += 1;
            }
        }
        if count == 0 {
            return (0.0, 0.0);
        }
        (signed / count as f32, absolute / count as f32)
    }

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
    fn chimp_browser_tabs_follow_the_asset_browsing_order() {
        assert_eq!(
            ChimpBrowser::TABS,
            [
                (ChimpBrowser::Folders, "Folders"),
                (ChimpBrowser::Groups, "Groups"),
                (ChimpBrowser::Files, "Pak files"),
                (ChimpBrowser::Archives, "Archives"),
                (ChimpBrowser::Packages, "Packages"),
            ]
        );
    }

    #[test]
    fn campaign_evolved_surface_tabs_put_tags_before_chimp() {
        assert_eq!(KitSurface::TABS[0].0, KitSurface::Tags);
        assert_eq!(KitSurface::TABS[0].1, "Tags");
        assert_eq!(KitSurface::TABS[1].0, KitSurface::Chimp);
        assert_eq!(KitSurface::TABS[1].1, "Chimp");
    }

    #[test]
    fn chimp_search_matching_is_case_insensitive_without_allocating_per_package() {
        assert!(chimp_contains_query("SM_SpiritDropShip_Body", "spirit"));
        assert!(chimp_contains_query("Texture2D", "texture2d"));
        assert!(!chimp_contains_query("StaticMesh", "skeletal"));
    }

    #[test]
    fn bundled_chimp_usmap_loads_and_invalid_custom_file_is_rejected() {
        assert!(load_chimp_usmap(None).is_ok());
        let path =
            std::env::temp_dir().join(format!("baboon-invalid-{}.usmap", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"not a usmap").unwrap();
        let error = load_chimp_usmap(Some(&path)).err().expect("invalid USMAP");
        assert!(error.contains("Could not parse USMAP"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install plus CE_PAKS and CE_USMAP"]
    fn real_custom_usmap_mounts_and_decodes_a_package() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let path = PathBuf::from(std::env::var_os("CE_USMAP").expect("set CE_USMAP"));
        let world = World::open(root, load_chimp_usmap(Some(&path)).unwrap()).unwrap();
        let package = world
            .packages()
            .iter()
            .find(|package| {
                package
                    .name
                    .to_ascii_lowercase()
                    .contains("sm_spiritdropship_body")
            })
            .unwrap_or_else(|| panic!("SM_SpiritDropShip_Body was not found"));
        let document = load_chimp_document(&world, &package.name).unwrap();
        assert!(!document.exports.is_empty());
        assert_eq!(document.mesh_kind, Some(ChimpMeshKind::Static));
    }

    #[test]
    fn chimp_document_tree_tracks_open_close_and_selection() {
        let mut state = ChimpState::default();
        let kit = KitId(7);
        state.open_document_pane(kit, "/Game/Textures/A");
        state.open_document_pane(kit, "/Game/Textures/B");
        assert_eq!(state.open_packages.len(), 2);
        assert_eq!(state.selected_package.as_deref(), Some("/Game/Textures/B"));
        assert!(
            state
                .document_tree
                .as_ref()
                .is_some_and(|tree| !tree.is_empty())
        );

        state.close_document_pane("/Game/Textures/B");
        assert_eq!(state.open_packages, ["/Game/Textures/A"]);
        assert_eq!(state.selected_package.as_deref(), Some("/Game/Textures/A"));
        state.close_document_pane("/Game/Textures/A");
        assert!(state.open_packages.is_empty());
        assert!(state.selected_package.is_none());
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
    fn readable_documents_are_the_default_view() {
        assert_eq!(ChimpDocumentView::default(), ChimpDocumentView::Document);
        assert_eq!(
            chimp_class_display_name(Some("/Script/Engine.BlueprintGeneratedClass")),
            "BlueprintGeneratedClass"
        );
        assert_eq!(chimp_class_display_name(None), "Unknown");
    }

    #[test]
    fn json_documents_have_line_numbers_and_semantic_colours() {
        let text = "{\n  \"Name\": \"Probe\",\n  \"ObjectPath\": \"/Game/Probe.0\",\n  \"Count\": 3,\n  \"Enabled\": true,\n  \"Missing\": null\n}";
        assert_eq!(chimp_line_numbers(text), "1\n2\n3\n4\n5\n6\n7");
        let job = chimp_json_layout_job(text, egui::FontId::monospace(12.0), true);
        assert_eq!(job.text, text);
        let mut colors = Vec::new();
        for section in &job.sections {
            if !colors.contains(&section.format.color) {
                colors.push(section.format.color);
            }
        }
        assert!(
            colors.len() >= 7,
            "keys, strings, paths, numbers, literals, punctuation, and whitespace need distinct colours"
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
        assert_eq!(document.view, ChimpDocumentView::Document);
        assert!(!document.document_text_dirty);
        assert!(
            !document.exports.is_empty(),
            "the real package should expose at least one readable export"
        );
        let readable: Value =
            serde_json::from_str(&document.document_text).expect("readable document is valid JSON");
        assert_eq!(readable["Package"], package);
        assert_eq!(
            readable["Exports"].as_array().map(Vec::len),
            Some(document.exports.len())
        );
        let export = readable["Exports"]
            .as_array()
            .and_then(|exports| exports.first())
            .expect("the JSON document should contain its first export");
        assert!(export.get("Type").is_some());
        assert!(export.get("Name").is_some());
        assert!(export.get("Properties").is_some());
        assert_eq!(
            document.document_line_numbers.lines().count(),
            document.document_text.lines().count()
        );
        let metadata: Value =
            serde_json::from_str(&document.metadata_text).expect("metadata document is valid JSON");
        assert_eq!(metadata["Summary"]["Package"], package);
        assert_eq!(
            metadata["NameMap"].as_array().map(Vec::len),
            Some(document.header.name_map.copy_raw_names().len())
        );
        assert_eq!(
            metadata["ExportMap"].as_array().map(Vec::len),
            Some(document.header.export_map.len())
        );
        assert!(
            metadata["PhysicalProviders"]
                .as_array()
                .is_some_and(|providers| !providers.is_empty())
        );
        assert_eq!(
            document.metadata_line_numbers.lines().count(),
            document.metadata_text.lines().count()
        );
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

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_fast_type_index_matches_legacy_window() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let fast = index_chimp_package_types_with_prefixes(&world, &[64 * 1024, 1024 * 1024]);
        let legacy = index_chimp_package_types_with_prefixes(&world, &[1024 * 1024]);
        assert_eq!(fast.package_types, legacy.package_types);
        assert_eq!(fast.type_counts, legacy.type_counts);
        assert_eq!(fast.failures, legacy.failures);
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
        let output =
            std::env::temp_dir().join(format!("baboon-chimp-texture-{}.tif", uuid::Uuid::new_v4()));
        write_chimp_texture_tiff(&world, &package, &output).unwrap();
        let bytes = std::fs::read(&output).unwrap();
        assert!(
            bytes.starts_with(b"II*") || bytes.starts_with(b"MM\0*"),
            "Texture2D extraction should produce a TIFF file"
        );
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_meshes_preview_and_extract_to_jms_and_actorx() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let type_index = index_chimp_package_types(&world);

        for (type_name, expected_kind) in [
            ("SkeletalMesh", ChimpMeshKind::Skeletal),
            ("StaticMesh", ChimpMeshKind::Static),
        ] {
            let preferred_suffix = match expected_kind {
                ChimpMeshKind::Skeletal => "/SK_Elite_Common_Body",
                ChimpMeshKind::Static => "",
            };
            let mut candidates = world
                .packages()
                .iter()
                .enumerate()
                .filter(|(package_index, _)| {
                    type_index
                        .package_types
                        .get(*package_index)
                        .and_then(Option::as_deref)
                        == Some(type_name)
                })
                .map(|(_, package)| package)
                .collect::<Vec<_>>();
            candidates.sort_by_key(|package| {
                !package
                    .name
                    .to_ascii_lowercase()
                    .ends_with(&preferred_suffix.to_ascii_lowercase())
            });
            let package = candidates
                .into_iter()
                .find_map(|package| {
                    let document = load_chimp_document(&world, &package.name).ok()?;
                    document.mesh_preview.as_ref()?.as_ref().ok()?;
                    Some(package.name.clone())
                })
                .unwrap_or_else(|| panic!("no decodable {type_name} package"));
            let document = load_chimp_document(&world, &package).unwrap();
            assert_eq!(document.view, ChimpDocumentView::Mesh);
            assert_eq!(document.mesh_kind, Some(expected_kind));
            let preview = document.mesh_preview.as_ref().unwrap().as_ref().unwrap();
            assert!(!preview.preview.vertices.is_empty());
            assert!(!preview.preview.indices.is_empty());
            assert!(!preview.preview.batches.is_empty());
            let (signed_alignment, absolute_alignment) = preview_normal_alignment(preview);
            eprintln!(
                "{package}: winding-signed normal alignment {signed_alignment:.3}, magnitude {absolute_alignment:.3}"
            );
            assert!(
                absolute_alignment > 0.45,
                "{package} normals do not follow the decoded surface ({absolute_alignment:.3})"
            );

            let formats = if expected_kind == ChimpMeshKind::Skeletal
                && preview.preview.vertices.len() <= 65_536
            {
                vec![
                    ChimpMeshFormat::Jms,
                    ChimpMeshFormat::Psk,
                    ChimpMeshFormat::Pskx,
                ]
            } else {
                vec![ChimpMeshFormat::Jms, ChimpMeshFormat::Pskx]
            };
            for format in formats {
                let output = std::env::temp_dir().join(format!(
                    "baboon-chimp-mesh-{}.{}",
                    uuid::Uuid::new_v4(),
                    format.extension()
                ));
                write_chimp_mesh(&world, &package, &output, format).unwrap();
                let bytes = std::fs::read(&output).unwrap();
                match format {
                    ChimpMeshFormat::Jms => assert!(bytes.starts_with(b";### VERSION ###")),
                    ChimpMeshFormat::Psk => {
                        assert!(bytes.windows(8).any(|window| window == b"FACE0000"))
                    }
                    ChimpMeshFormat::Pskx => {
                        assert!(bytes.windows(8).any(|window| window == b"FACE3200"))
                    }
                }
                std::fs::remove_file(output).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_spiritdropship_nanite_export_is_complete() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let package = world
            .packages()
            .iter()
            .find(|package| {
                package
                    .name
                    .to_ascii_lowercase()
                    .contains("sm_spiritdropship_body")
            })
            .unwrap_or_else(|| panic!("SM_SpiritDropShip_Body was not found"));
        let document = load_chimp_document(&world, &package.name).unwrap();
        assert_eq!(document.mesh_kind, Some(ChimpMeshKind::Static));

        let archive = &world.archives()[document.provider.container];
        let chunk = archive
            .chunk_index_for(&document.provider.entry_path)
            .expect("static mesh package has an IoStore chunk");
        let bulk = archive
            .read_bulk_for(chunk, 0)
            .expect("Nanite static mesh has readable bulk data");
        let resources = blam_tags::iostore::nanite::NaniteResources::parse(
            &document.original,
            document.header.summary.header_size as usize,
        )
        .expect("static mesh has Nanite resources");
        let nanite =
            blam_tags::iostore::nanite::decode_nanite(&document.original, &bulk, &resources);
        let mut miswound_triangles = 0usize;
        let mut duplicate_index_triangles = 0usize;
        let mut zero_area_triangles = 0usize;
        for triangle in &nanite.triangles {
            if triangle[0] == triangle[1]
                || triangle[1] == triangle[2]
                || triangle[0] == triangle[2]
            {
                duplicate_index_triangles += 1;
            }
            let [a, b, c] = triangle.map(|index| nanite.positions[index as usize]);
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let face = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            if face[0] * face[0] + face[1] * face[1] + face[2] * face[2] <= 1.0e-12 {
                zero_area_triangles += 1;
            }
            let normal = triangle.iter().fold([0.0; 3], |mut total, index| {
                let normal = nanite.normals[*index as usize];
                total[0] += normal[0];
                total[1] += normal[1];
                total[2] += normal[2];
                total
            });
            if face[0] * normal[0] + face[1] * normal[1] + face[2] * normal[2] > 0.0 {
                miswound_triangles += 1;
            }
        }
        let converted = StaticMesh::from_nanite(&nanite);
        let preview = document
            .mesh_preview
            .as_ref()
            .and_then(|preview| preview.as_ref().ok())
            .expect("Nanite static mesh has a Chimp preview");
        assert_eq!(preview.preview.vertices.len(), converted.vertices.len());
        assert_eq!(preview.preview.indices.len(), converted.indices.len());
        let mut converted_miswound_triangles = 0usize;
        let mut severe_uv_stretch_triangles = 0usize;
        let mut maximum_uv_per_cm = 0.0f32;
        let mut long_uv_edge_triangles = 0usize;
        let mut negative_wrap_span_triangles = 0usize;
        let mut maximum_uv_edge = 0.0f32;
        for triangle in converted.indices.chunks_exact(3) {
            let a = converted.vertices[triangle[0] as usize].position;
            let b = converted.vertices[triangle[1] as usize].position;
            let c = converted.vertices[triangle[2] as usize].position;
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let face = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let normal = triangle.iter().fold([0.0; 3], |mut total, index| {
                let normal = converted.vertices[*index as usize].normal;
                total[0] += normal[0];
                total[1] += normal[1];
                total[2] += normal[2];
                total
            });
            if face[0] * normal[0] + face[1] * normal[1] + face[2] * normal[2] > 0.0 {
                converted_miswound_triangles += 1;
            }
            let mut severe_uv_stretch = false;
            let mut long_uv_edge = false;
            for [left, right] in [[0usize, 1usize], [1, 2], [2, 0]] {
                let left = &converted.vertices[triangle[left] as usize];
                let right = &converted.vertices[triangle[right] as usize];
                let dx = left.position[0] - right.position[0];
                let dy = left.position[1] - right.position[1];
                let dz = left.position[2] - right.position[2];
                let position_distance_squared = dx * dx + dy * dy + dz * dz;
                if position_distance_squared <= 1.0e-12 {
                    continue;
                }
                let du = left.uv[0] - right.uv[0];
                let dv = left.uv[1] - right.uv[1];
                let uv_edge = (du * du + dv * dv).sqrt();
                maximum_uv_edge = maximum_uv_edge.max(uv_edge);
                long_uv_edge |= uv_edge > 2.0;
                let uv_per_cm = ((du * du + dv * dv) / position_distance_squared).sqrt();
                maximum_uv_per_cm = maximum_uv_per_cm.max(uv_per_cm);
                severe_uv_stretch |= uv_per_cm > 10.0;
            }
            severe_uv_stretch_triangles += usize::from(severe_uv_stretch);
            long_uv_edge_triangles += usize::from(long_uv_edge);
            for axis in 0..2 {
                let coordinates = [triangle[0], triangle[1], triangle[2]]
                    .map(|index| converted.vertices[index as usize].uv[axis]);
                let minimum = coordinates.iter().copied().fold(f32::INFINITY, f32::min);
                let maximum = coordinates
                    .iter()
                    .copied()
                    .fold(f32::NEG_INFINITY, f32::max);
                if minimum < -1.0 && maximum < 0.0 && maximum - minimum > 1.0 {
                    negative_wrap_span_triangles += 1;
                    break;
                }
            }
        }
        eprintln!(
            "UV wrap diagnostics: long_edges={long_uv_edge_triangles}, negative_wrap_spans={negative_wrap_span_triangles}, maximum_edge={maximum_uv_edge}"
        );
        let mut position_ids = std::collections::HashMap::<[u32; 3], u32>::new();
        let mut canonical_vertices = Vec::with_capacity(converted.vertices.len());
        for vertex in &converted.vertices {
            let key = vertex
                .position
                .map(|value| if value == 0.0 { 0 } else { value.to_bits() });
            let next = position_ids.len() as u32;
            canonical_vertices.push(*position_ids.entry(key).or_insert(next));
        }
        let mut edges = Vec::with_capacity(converted.indices.len());
        for triangle in converted.indices.chunks_exact(3) {
            let ids = [
                canonical_vertices[triangle[0] as usize],
                canonical_vertices[triangle[1] as usize],
                canonical_vertices[triangle[2] as usize],
            ];
            if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
                continue;
            }
            for [a, b] in [[ids[0], ids[1]], [ids[1], ids[2]], [ids[2], ids[0]]] {
                let [lo, hi] = if a < b { [a, b] } else { [b, a] };
                edges.push((u64::from(lo) << 32) | u64::from(hi));
            }
        }
        edges.sort_unstable();
        let mut boundary_edges = Vec::new();
        let mut cursor = 0usize;
        while cursor < edges.len() {
            let edge = edges[cursor];
            let mut end = cursor + 1;
            while end < edges.len() && edges[end] == edge {
                end += 1;
            }
            if end - cursor == 1 {
                boundary_edges.push(edge);
            }
            cursor = end;
        }
        let mut boundary_adjacency = std::collections::HashMap::<u32, Vec<u32>>::new();
        for edge in &boundary_edges {
            let a = (edge >> 32) as u32;
            let b = *edge as u32;
            boundary_adjacency.entry(a).or_default().push(b);
            boundary_adjacency.entry(b).or_default().push(a);
        }
        let mut visited = std::collections::HashSet::new();
        let mut triangular_boundary_loops = 0usize;
        let mut triangular_hole_edges = std::collections::HashMap::<u64, u32>::new();
        for &start in boundary_adjacency.keys() {
            if !visited.insert(start) {
                continue;
            }
            let mut stack = vec![start];
            let mut vertices = 0usize;
            let mut degree_sum = 0usize;
            let mut component = Vec::new();
            while let Some(vertex) = stack.pop() {
                vertices += 1;
                component.push(vertex);
                let neighbours = &boundary_adjacency[&vertex];
                degree_sum += neighbours.len();
                for &neighbour in neighbours {
                    if visited.insert(neighbour) {
                        stack.push(neighbour);
                    }
                }
            }
            if vertices == 3 && degree_sum == 6 {
                triangular_boundary_loops += 1;
                for index in 0..3 {
                    let a = component[index];
                    let b = component[(index + 1) % 3];
                    let third = component[(index + 2) % 3];
                    let [lo, hi] = if a < b { [a, b] } else { [b, a] };
                    triangular_hole_edges.insert((u64::from(lo) << 32) | u64::from(hi), third);
                }
            }
        }
        let mut paired_degenerate_triangles = 0usize;
        let mut paired_zero_area_holes = std::collections::HashSet::<[u32; 3]>::new();
        for triangle in converted.indices.chunks_exact(3) {
            let ids = [
                canonical_vertices[triangle[0] as usize],
                canonical_vertices[triangle[1] as usize],
                canonical_vertices[triangle[2] as usize],
            ];
            let mut distinct = ids;
            distinct.sort_unstable();
            let distinct_len = if distinct[0] == distinct[2] {
                1
            } else if distinct[0] == distinct[1] || distinct[1] == distinct[2] {
                2
            } else {
                3
            };
            if distinct_len == 2 {
                let a = distinct[0];
                let b = distinct[2];
                let edge = (u64::from(a) << 32) | u64::from(b);
                paired_degenerate_triangles +=
                    usize::from(triangular_hole_edges.contains_key(&edge));
            }

            let positions = [
                converted.vertices[triangle[0] as usize].position,
                converted.vertices[triangle[1] as usize].position,
                converted.vertices[triangle[2] as usize].position,
            ];
            let ab = [
                positions[1][0] - positions[0][0],
                positions[1][1] - positions[0][1],
                positions[1][2] - positions[0][2],
            ];
            let ac = [
                positions[2][0] - positions[0][0],
                positions[2][1] - positions[0][1],
                positions[2][2] - positions[0][2],
            ];
            let cross = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            if cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2] > 1.0e-12 {
                continue;
            }
            for [x, y] in [[ids[0], ids[1]], [ids[1], ids[2]], [ids[2], ids[0]]] {
                if x == y {
                    continue;
                }
                let [lo, hi] = if x < y { [x, y] } else { [y, x] };
                let edge = (u64::from(lo) << 32) | u64::from(hi);
                if let Some(&third) = triangular_hole_edges.get(&edge) {
                    let mut hole = [lo, hi, third];
                    hole.sort_unstable();
                    paired_zero_area_holes.insert(hole);
                }
            }
        }
        eprintln!(
            "{}: input_triangles={}, decoded_triangles={}, duplicate_index_triangles={}, zero_area_triangles={}, miswound_before={}, miswound_after={}, severe_uv_stretch_triangles={}, maximum_uv_per_cm={}, boundary_edges={}, triangular_boundary_loops={}, paired_degenerate_triangles={}, paired_zero_area_holes={}, unresolved_vertices={}",
            package.name,
            resources.num_input_triangles,
            nanite.triangles.len(),
            duplicate_index_triangles,
            zero_area_triangles,
            miswound_triangles,
            converted_miswound_triangles,
            severe_uv_stretch_triangles,
            maximum_uv_per_cm,
            boundary_edges.len(),
            triangular_boundary_loops,
            paired_degenerate_triangles,
            paired_zero_area_holes.len(),
            nanite.unresolved_vertices,
        );
        assert_eq!(nanite.unresolved_vertices, 0);
        assert_eq!(
            nanite.triangles.len(),
            resources.num_input_triangles as usize
        );
        assert!(
            miswound_triangles > 0,
            "fixture should exercise the regression"
        );
        assert_eq!(converted_miswound_triangles, 0);
        assert_eq!(
            severe_uv_stretch_triangles, 0,
            "repaired Nanite faces must remain on their local UV seams"
        );
        assert!(maximum_uv_per_cm < 5.0);
        assert_eq!(
            negative_wrap_span_triangles, 0,
            "negative repeating UV faces should be split at wrap boundaries"
        );
        assert_eq!(long_uv_edge_triangles, 0);
        assert!(
            triangular_boundary_loops <= 1,
            "the Nanite repair should remove the mass triangular-hole pattern"
        );

        let expected_faces = converted
            .indices
            .chunks_exact(3)
            .filter(|triangle| {
                triangle[0] != triangle[1]
                    && triangle[1] != triangle[2]
                    && triangle[0] != triangle[2]
            })
            .count();
        let jms = blam_tags::iostore::actorx::static_mesh_to_jms(&converted, &[]);
        assert_eq!(jms.triangles.len(), expected_faces);
        let mut jms_bytes = Vec::new();
        jms.write(&mut jms_bytes, 8213).unwrap();
        assert!(jms_bytes.starts_with(b";### VERSION ###"));
        let output = std::env::temp_dir().join(format!(
            "baboon-spiritdropship-{}.pskx",
            uuid::Uuid::new_v4()
        ));
        write_chimp_mesh(&world, &package.name, &output, ChimpMeshFormat::Pskx).unwrap();
        let bytes = std::fs::read(&output).unwrap();
        let face_chunk = bytes
            .windows(8)
            .position(|window| window == b"FACE3200")
            .expect("PSKX contains its 32-bit face chunk");
        let face_count =
            i32::from_le_bytes(bytes[face_chunk + 28..face_chunk + 32].try_into().unwrap())
                as usize;
        assert_eq!(face_count, expected_faces);
        std::fs::remove_file(output).unwrap();
    }
}
