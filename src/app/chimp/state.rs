//! Chimp workspace state: the per-kit browser, document tree and open documents.
//! It owns the types every Chimp view reads and writes; decoding, saving and drawing belong elsewhere.

use super::*;
use crate::core::document::journal::EditJournal;

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

/// A kit's Chimp content: the mounted world, the open package documents and
/// which are open or selected. How the Chimp surface is browsed and laid out
/// is the kit view's [`ChimpView`].
#[derive(Default)]
pub(in crate::app) struct ChimpState {
    pub(in crate::app) mount: ChimpMount,
    /// The generation the mount in flight (or the one that produced the
    /// mounted world) was started at. The world depends only on the Paks
    /// folder, which a tag edit bumping the generation doesn't change; a new
    /// source rebuilds this state, so a mount from the old one finds no
    /// match.
    pub(in crate::app) mount_request: Option<u64>,
    pub(in crate::app) selected_package: Option<String>,
    pub(in crate::app) open_packages: Vec<String>,
    pub(in crate::app) documents: HashMap<String, ChimpDocument>,
    pub(super) loading_packages: HashSet<String>,
}

/// How a kit's Chimp surface is being browsed and laid out: the browser mode,
/// filter and what it matched, the package-type index, selections in the
/// browser, the document tile tree and the save dialog. Held in the kit's
/// view, apart from the [`ChimpState`] content.
#[derive(Default)]
pub(in crate::app) struct ChimpView {
    /// Each open document's pane, by package.
    pub(super) documents: HashMap<String, ChimpDocumentUi>,
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
    pub(super) selected_file: Option<String>,
    pub(super) document_tree: Option<egui_tiles::Tree<String>>,
}

/// One open Unreal package's content: its bytes and decoded header, payloads
/// and exports, and its save and recovery bookkeeping. What its pane shows
/// and drafts is its [`ChimpDocumentUi`] in the kit's [`ChimpView`].
pub(in crate::app) struct ChimpDocument {
    pub(super) package: String,
    pub(super) provider: PackageProvider,
    pub(super) original: Vec<u8>,
    pub(super) header: FZenPackageHeader,
    pub(super) payloads: Vec<Vec<u8>>,
    pub(super) exports: Vec<ChimpExport>,
    pub(super) mesh_kind: Option<ChimpMeshKind>,
    pub(in crate::app) dirty: bool,
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
    /// Undo and redo, as rebuilt packages. See the `edit` module.
    pub(super) journal: EditJournal,
}

/// One open package's pane: which tab and export it shows, the text and
/// usage it derived from the document, header edits being drafted, who
/// references it, and its texture and mesh previews.
pub(in crate::app) struct ChimpDocumentUi {
    pub(super) texture_previews: Vec<ChimpTexturePreview>,
    pub(super) mesh_preview: Option<Result<ModelPreviewData, String>>,
    pub(super) mesh_preview_state: ModelPreviewState,
    pub(super) selected_export: usize,
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
    /// The selected export as the property editor edits it. See
    /// [`ChimpPropertyDraft`].
    pub(super) property_draft: Option<ChimpPropertyDraft>,
    /// A view or export chosen while a text box had focus, taken up on the
    /// next frame. The box is still drawn on the frame of the click, so it
    /// sees itself lose focus and commits; switched at once, it was never
    /// drawn again and the typed name was lost.
    pub(super) pending_switch: Option<ChimpPaneSwitch>,
}

/// What a deferred switch in a package pane goes to.
#[derive(Clone, Copy)]
pub(super) enum ChimpPaneSwitch {
    View(ChimpDocumentView),
    Export(usize),
}

/// One export's values and the name map they intern into, edited in place by
/// the property editor and sent to the document whole when they change.
///
/// Taken afresh whenever the document has moved on since — an undo, a header
/// commit, or this draft's own last edit landing — so what the editor shows
/// is always the document as it stands.
pub(super) struct ChimpPropertyDraft {
    pub(super) export: usize,
    /// The document's [`ChimpDocument::edits`] when this was taken.
    pub(super) edits: u64,
    pub(super) decoded: Export,
    pub(super) name_map: blam_tags::iostore::package::name_map::FNameMap,
}

#[derive(Default)]
pub(super) enum ChimpReferrerState {
    #[default]
    Idle,
    Scanning,
    Done(ChimpReferrerScan),
}

impl ChimpView {
    fn ensure_document_tree(&mut self, kit: KitId) -> &mut egui_tiles::Tree<String> {
        self.document_tree
            .get_or_insert_with(|| egui_tiles::Tree::empty(chimp_tree_id(kit)))
    }

    pub(super) fn open_document_pane(&mut self, chimp: &mut ChimpState, kit: KitId, package: &str) {
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
        chimp.selected_package = Some(package.to_owned());
        self.sync_open_packages(chimp);
    }

    pub(super) fn close_document_pane(&mut self, chimp: &mut ChimpState, package: &str) {
        if let Some(tree) = self.document_tree.as_mut() {
            let tile_id = tree.tiles.iter().find_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(open) if open == package => Some(*id),
                _ => None,
            });
            if let Some(tile_id) = tile_id {
                tree.remove_recursively(tile_id);
            }
        }
        self.sync_open_packages(chimp);
    }

    /// Re-derive the open and selected packages from the tile tree, which
    /// owns the layout. Returns whether the open packages or the selection
    /// changed.
    pub(super) fn sync_open_packages(&mut self, chimp: &mut ChimpState) -> bool {
        let open_packages: Vec<String> = self
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
        let mut changed = open_packages != chimp.open_packages;
        chimp.open_packages = open_packages;
        if chimp
            .selected_package
            .as_ref()
            .is_some_and(|package| !chimp.open_packages.contains(package))
        {
            chimp.selected_package = chimp.open_packages.first().cloned();
            changed = true;
        }
        changed
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

impl Baboon {
    /// Open `package`'s document pane in a kit's Chimp layout and select it.
    pub(super) fn open_chimp_document_pane(&mut self, kit_index: usize, package: &str) {
        let kit = &mut self.model.kits[kit_index];
        self.views[kit.id].chimp.open_document_pane(&mut kit.chimp, kit.id, package);
    }

    /// Add an open document to a kit, with its pane.
    pub(super) fn insert_chimp_document(
        &mut self,
        kit_index: usize,
        package: String,
        document: ChimpDocument,
        pane: ChimpDocumentUi,
    ) {
        let kit = &mut self.model.kits[kit_index];
        self.views[kit.id].chimp.documents.insert(package.clone(), pane);
        kit.chimp.documents.insert(package, document);
    }

    /// Drop an open document from a kit, with its pane.
    pub(super) fn remove_chimp_document(&mut self, kit_index: usize, package: &str) {
        let kit = &mut self.model.kits[kit_index];
        self.views[kit.id].chimp.documents.remove(package);
        kit.chimp.documents.remove(package);
    }

    /// Drop a kit's Chimp content and its view, as on a remount or when
    /// Chimp is turned off.
    pub(in crate::app) fn reset_chimp(&mut self, kit_index: usize) {
        let kit = &mut self.model.kits[kit_index];
        kit.chimp = ChimpState::default();
        self.views[kit.id].chimp = ChimpView::default();
        let id = kit.id;
        self.dialogs.close_where::<ChimpSaveDialog>(|dialog| dialog.kit == id);
    }

    /// Close `package`'s document pane in a kit's Chimp layout.
    pub(super) fn close_chimp_document_pane(&mut self, kit_index: usize, package: &str) {
        let kit = &mut self.model.kits[kit_index];
        self.views[kit.id].chimp.close_document_pane(&mut kit.chimp, package);
    }
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
        let view = ChimpView::default();
        assert!(matches!(state.mount, ChimpMount::Idle));
        assert_eq!(view.browser, ChimpBrowser::Folders);
        assert!(view.filter.is_empty());
        assert!(state.open_packages.is_empty());
        assert!(
            !view.filter_is_current(""),
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
        let mut view = ChimpView::default();
        let kit = KitId(7);
        view.open_document_pane(&mut state, kit, "/Game/Textures/A");
        view.open_document_pane(&mut state, kit, "/Game/Textures/B");
        assert_eq!(state.open_packages.len(), 2);
        assert_eq!(state.selected_package.as_deref(), Some("/Game/Textures/B"));
        assert!(
            view
                .document_tree
                .as_ref()
                .is_some_and(|tree| !tree.is_empty())
        );

        view.close_document_pane(&mut state, "/Game/Textures/B");
        assert_eq!(state.open_packages, ["/Game/Textures/A"]);
        assert_eq!(state.selected_package.as_deref(), Some("/Game/Textures/A"));
        view.close_document_pane(&mut state, "/Game/Textures/A");
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
