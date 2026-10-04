//! Chimp workspace state: the per-kit browser, document tree and open documents.
//! It owns the types every Chimp view reads and writes; decoding, saving and drawing belong elsewhere.

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum KitSurface {
    #[default]
    Tags,
    Chimp,
}

pub(in crate::app) enum ChimpMount {
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
pub(super) enum ChimpBrowser {
    #[default]
    Folders,
    Groups,
    Archives,
    Packages,
    Files,
}

impl KitSurface {
    pub(in crate::app) const TABS: [(Self, &'static str, &'static str); 2] = [
        (Self::Tags, "Tags", "Browse and edit Halo tags"),
        (
            Self::Chimp,
            "Chimp",
            "Browse and edit Unreal Engine packages",
        ),
    ];
}

impl ChimpBrowser {
    pub(super) const TABS: [(Self, &'static str); 5] = [
        (Self::Folders, "Folders"),
        (Self::Groups, "Groups"),
        (Self::Files, "Pak files"),
        (Self::Archives, "Archives"),
        (Self::Packages, "Packages"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChimpArchive {
    IoStore(usize),
    Pak(usize),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ChimpFolderSelection {
    #[default]
    Package,
    File,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ChimpDocumentView {
    #[default]
    Document,
    Texture,
    Mesh,
    Properties,
    Header,
    Metadata,
}

pub(super) enum ChimpTreeClick {
    Package(String),
    ExtractTexture(String),
    ExtractMesh(String, ChimpMeshFormat),
    ExportLevel(String, ChimpLevelFormat),
    File(String),
}

#[derive(Default)]
pub(super) struct ChimpFolderNode {
    pub(super) folders: BTreeMap<String, ChimpFolderNode>,
    pub(super) packages: Vec<ChimpPackageLeaf>,
    pub(super) files: Vec<ChimpFileLeaf>,
    pub(super) package_count: usize,
    pub(super) file_count: usize,
}

pub(super) struct ChimpPackageLeaf {
    pub(super) name: String,
    pub(super) package: usize,
}

pub(super) struct ChimpFileLeaf {
    pub(super) name: String,
    pub(super) file: usize,
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

    pub(super) fn entry_count(&self) -> usize {
        self.package_count + self.file_count
    }
}

#[derive(Default)]
pub(in crate::app) struct ChimpState {
    pub(in crate::app) mount: ChimpMount,
    pub(super) browser: ChimpBrowser,
    pub(super) filter: String,
    pub(super) filtered_for: Option<String>,
    pub(super) filtered_archive_for: Option<Option<ChimpArchive>>,
    /// Shared, not owned: the package and file lists are drawn every frame
    /// by closures that also mutate the app, so they took a copy of up to
    /// ~104k indices per frame. Rebuilt in place through `Arc::make_mut`.
    pub(super) filtered_packages: Arc<Vec<usize>>,
    pub(super) filtered_files: Arc<Vec<usize>>,
    pub(super) filtered_groups: BTreeMap<String, Vec<usize>>,
    pub(super) content_tree: ChimpFolderNode,
    pub(super) selected_archive: Option<ChimpArchive>,
    pub(super) package_types: Vec<Option<String>>,
    pub(super) type_indexing: bool,
    pub(super) folder_selection: ChimpFolderSelection,
    pub(in crate::app) selected_package: Option<String>,
    pub(super) selected_file: Option<String>,
    pub(in crate::app) open_packages: Vec<String>,
    pub(super) document_tree: Option<egui_tiles::Tree<String>>,
    pub(in crate::app) documents: HashMap<String, ChimpDocument>,
    pub(super) loading_packages: HashSet<String>,
    pub(super) save_dialog: Option<ChimpSaveDialog>,
}

pub(in crate::app) struct ChimpDocument {
    pub(super) package: String,
    pub(super) provider: PackageProvider,
    pub(super) original: Vec<u8>,
    pub(super) header: FZenPackageHeader,
    pub(super) payloads: Vec<Vec<u8>>,
    pub(super) exports: Vec<ChimpExport>,
    pub(super) texture_previews: Vec<ChimpTexturePreview>,
    pub(super) mesh_kind: Option<ChimpMeshKind>,
    pub(super) mesh_preview: Option<Result<ModelPreviewData, String>>,
    pub(super) mesh_preview_state: ModelPreviewState,
    pub(super) selected_export: usize,
    pub(in crate::app) dirty: bool,
    pub(super) view: ChimpDocumentView,
    pub(super) document_text: String,
    pub(super) document_lines: ChimpJsonLines,
    pub(super) document_text_dirty: bool,
    pub(super) metadata_text: String,
    pub(super) metadata_lines: ChimpJsonLines,
    pub(super) metadata_text_dirty: bool,
    /// Who references each name-map entry and each import slot.
    ///
    /// Cached because it walks every decoded export, and invalidated with the
    /// metadata text — the two go stale together, on any header change.
    pub(super) header_usage: Option<ChimpHeaderUsage>,
    pub(super) header_name_filter: String,
    /// The name-map row being edited, if any. Header edits commit explicitly
    /// rather than per keystroke: a rename walks every decoded export and then
    /// rebuilds the whole package for the recovery checkpoint, which is not
    /// something to do between two letters of a word.
    pub(super) header_name_edit: Option<ChimpNameEdit>,
    pub(super) header_import_edit: Option<ChimpImportEdit>,
    pub(super) header_export_edit: Option<ChimpExportEdit>,
    pub(super) header_identity_edit: Option<ChimpIdentityEdit>,
    pub(super) header_error: Option<String>,
    /// Who imports this package. Not derived at load: there is no reverse index
    /// in the paks, so answering it means reading every mounted header.
    pub(super) referrers: ChimpReferrerState,
    /// The mounted containers no longer provide this package, so `provider` no
    /// longer describes anything and nothing may be written back through it.
    /// The document keeps its bytes, so reading and extraction still work.
    pub(super) orphaned: bool,
    /// When (egui time) this document's recovery checkpoint is due. Set by an
    /// edit and pushed back by the next one, so a burst of edits checkpoints
    /// once, after it stops.
    pub(super) checkpoint_due: Option<f64>,
    /// Counts edits. A save records it when it rebuilds the package and, when
    /// it finishes, clears `dirty` only if no edit landed while it ran.
    pub(super) edits: u64,
}

#[derive(Default)]
pub(super) enum ChimpReferrerState {
    #[default]
    Idle,
    Scanning,
    Done(ChimpReferrerScan),
}

impl ChimpState {
    fn ensure_document_tree(&mut self, kit: KitId) -> &mut egui_tiles::Tree<String> {
        self.document_tree
            .get_or_insert_with(|| egui_tiles::Tree::empty(chimp_tree_id(kit)))
    }

    pub(super) fn open_document_pane(&mut self, kit: KitId, package: &str) {
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

    pub(super) fn close_document_pane(&mut self, package: &str) {
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

    pub(super) fn sync_open_packages(&mut self) {
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

    pub(super) fn refresh_filter(&mut self, world: &World) {
        let query = self.filter.trim().to_ascii_lowercase();
        if self.filter_is_current(&query) {
            return;
        }
        let selected_archive = self.selected_archive;
        self.filtered_for = Some(query.clone());
        self.filtered_archive_for = Some(selected_archive);
        Arc::make_mut(&mut self.filtered_packages).clear();
        Arc::make_mut(&mut self.filtered_files).clear();
        self.filtered_groups.clear();
        Arc::make_mut(&mut self.filtered_packages).extend(
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
                        .is_some_and(|kind| contains_ignore_ascii_case(kind, &query));
                    archive_matches
                        && (query.is_empty()
                            || type_matches
                            || contains_ignore_ascii_case(&package.name, &query)
                            || package.providers.iter().any(|provider| {
                                contains_ignore_ascii_case(
                                    &world.containers()[provider.container]
                                        .path
                                        .to_string_lossy(),
                                    &query,
                                )
                            }))
                })
                .map(|(index, _)| index),
        );
        Arc::make_mut(&mut self.filtered_files).extend(
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
                            || contains_ignore_ascii_case(&file.path, &query)
                            || file.providers.iter().any(|provider| {
                                contains_ignore_ascii_case(
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
        for &index in self.filtered_packages.iter() {
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
        for &index in self.filtered_files.iter() {
            self.content_tree
                .insert_file(index, &world.pak_files()[index].path);
        }
    }

    pub(super) fn reset_filter(&mut self) {
        self.filtered_for = None;
        self.filtered_archive_for = None;
        Arc::make_mut(&mut self.filtered_packages).clear();
        Arc::make_mut(&mut self.filtered_files).clear();
        self.filtered_groups.clear();
        self.content_tree = ChimpFolderNode::default();
    }
}

fn chimp_tree_id(kit: KitId) -> egui::Id {
    egui::Id::new(("chimp_document_tree", kit.0))
}

impl Kit {
    pub(super) fn documents_contains_chimp(&self, package: &str) -> bool {
        self.chimp.documents.contains_key(package)
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
}
