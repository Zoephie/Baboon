//! A kit's view state: what drawing the kit changes as it goes.
//!
//! The kit itself ([`Kit`], in the [`Model`]) is the content: its source, open
//! documents, selection and indices, which every draw reads. Its tab layout,
//! drafts, browser, preview caches, library tabs and panes are changed by the
//! draws that show them, so they live here, apart from the model, where a draw
//! can hold its kit's view mutably while it reads the model through a
//! [`Ctx`]. Commands and job handlers can change either.

use super::*;
use crate::app::kits::terminal::KitTerminal;

/// One kit's view state. Made, replaced and dropped with its kit.
pub(in crate::app) struct KitView {
    /// Layout of this kit's open tags: which panes exist, how they are split,
    /// and which is active in each tab group. The tree is authoritative —
    /// `open_tabs` is re-derived from it, never the other way round — so a
    /// split or a drag survives instead of being overwritten by a mirror.
    pub(in crate::app) tag_tree: egui_tiles::Tree<String>,
    /// In-progress text edits keyed by stable widget/edit identifiers. Drafts
    /// rather than bare strings so a value the user is still typing is not
    /// overwritten by the document underneath it.
    pub(in crate::app) edit_buffers: EditDrafts,
    /// Pending expand/collapse-all requests, keyed by tag. Raised from the tag
    /// tab's menu and consumed by the next draw of that tag's pane.
    pub(in crate::app) pending_expand: HashMap<String, bool>,
    /// Tracks panes that need their normal collapse defaults restored after
    /// Find's visual filter stops applying to them, and caches the filter each
    /// one last applied.
    pub(in crate::app) find_filter_applied: HashMap<String, AppliedFindFilter>,
    /// How this kit's browser lists its tags: mode, order and filter, the
    /// docked folder browsers, and the modified, deletable and favourite sets
    /// it marks, each with what it was built from.
    pub(in crate::app) browser: KitBrowser,
    /// What the editor derives from this kit's documents and keeps between
    /// frames: bitmap and model previews, render-method definitions and options
    /// with the epoch that invalidates the shader grid, Halo 2 templates, and
    /// Campaign Evolved sound bindings.
    pub(in crate::app) caches: EditorCaches,
    /// The Bitmap Library tab's state: its search, its grid size, and its
    /// own bounded thumbnail cache — deliberately not `bitmap_previews`,
    /// which is unbounded and holds full-resolution images.
    pub(in crate::app) bitmap_browser: ThumbnailLibrary<Bitmaps>,
    /// The Model Library tab's state, the same shape for the same reasons.
    pub(in crate::app) model_browser: ThumbnailLibrary<Models>,
    /// Read-only repository history and working-tree browser.
    pub(in crate::app) git_review: GitReviewState,
    /// Where this kit's terminal is: whether its panel is open and the
    /// directory its commands run in.
    pub(in crate::app) terminal: KitTerminal,
    /// CE-only nested surface. Chimp is not an editing kit and does not enter
    /// the top-level kit registry; it lives beside the Tags surface here.
    pub(in crate::app) surface: KitSurface,
    /// Halo 3 only: state of this kit's Blam! import pane ([`BLAM_KEY`] in
    /// `tag_tree`).
    pub(in crate::app) blam: BlamUiState,
}

impl KitView {
    /// The view a kit opens with: nothing laid out, drafted or cached, and
    /// its browser showing as `browser` says.
    pub(in crate::app) fn new(kit: KitId, browser: KitBrowser) -> Self {
        Self {
            tag_tree: egui_tiles::Tree::empty(tag_tree_id(kit)),
            edit_buffers: EditDrafts::default(),
            pending_expand: HashMap::new(),
            find_filter_applied: HashMap::new(),
            browser,
            caches: EditorCaches::default(),
            bitmap_browser: ThumbnailLibrary::default(),
            model_browser: ThumbnailLibrary::default(),
            git_review: GitReviewState::default(),
            terminal: KitTerminal::default(),
            surface: KitSurface::Tags,
            blam: BlamUiState::default(),
        }
    }
}

/// Every open kit's view, keyed by its kit. Kept in step with the model's
/// kits by the places that add, replace and close one; indexing a kit with no
/// view is a bug there, and panics like indexing past the end of `kits`.
#[derive(Default)]
pub(in crate::app) struct KitViews {
    views: HashMap<KitId, KitView>,
}

impl KitViews {
    /// The views of a Baboon that has just started: its one empty kit's.
    pub(in crate::app) fn startup(view: KitView) -> Self {
        let mut views = Self::default();
        views.insert(KitId(0), view);
        views
    }

    /// Give `kit` this view, replacing any it had.
    pub(in crate::app) fn insert(&mut self, kit: KitId, view: KitView) {
        self.views.insert(kit, view);
    }

    pub(in crate::app) fn remove(&mut self, kit: KitId) -> Option<KitView> {
        self.views.remove(&kit)
    }
}

impl std::ops::Index<KitId> for KitViews {
    type Output = KitView;

    fn index(&self, kit: KitId) -> &KitView {
        self.views
            .get(&kit)
            .unwrap_or_else(|| panic!("{kit:?} has no view"))
    }
}

impl std::ops::IndexMut<KitId> for KitViews {
    fn index_mut(&mut self, kit: KitId) -> &mut KitView {
        self.views
            .get_mut(&kit)
            .unwrap_or_else(|| panic!("{kit:?} has no view"))
    }
}

impl KitView {
    /// A fresh view for a kit a test built by hand.
    #[cfg(test)]
    pub(in crate::app) fn for_test(kit: &Kit) -> Self {
        Self::new(kit.id, KitBrowser::default())
    }

    /// Tag keys currently laid out, in tab order. Derived from the tree, which
    /// owns the layout; the kit's `open_tabs` is this, re-derived.
    pub(in crate::app) fn tabs_from_tree(&self) -> Vec<String> {
        self.tag_tree
            .tiles
            .tiles()
            .filter_map(|tile| match tile {
                egui_tiles::Tile::Pane(key) => Some(key.clone()),
                egui_tiles::Tile::Container(_) => None,
            })
            .collect()
    }

    fn tile_for_key(&self, key: &str) -> Option<egui_tiles::TileId> {
        self.tag_tree
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(pane) if pane == key => Some(*id),
                _ => None,
            })
    }
}

/// A kit and its view, borrowed together for what changes both: the tab
/// layout, which the view's tree owns and the kit's `open_tabs` and
/// `selected_key` follow, and forgetting a document along with everything
/// drawn or drafted for it.
pub(in crate::app) struct KitMut<'a> {
    pub(in crate::app) kit: &'a mut Kit,
    pub(in crate::app) view: &'a mut KitView,
}

impl Baboon {
    /// The kit at `index` with its view.
    pub(in crate::app) fn kit_and_view(&mut self, index: usize) -> KitMut<'_> {
        let kit = &mut self.model.kits[index];
        let view = &mut self.views[kit.id];
        KitMut::new(kit, view)
    }
}

impl<'a> KitMut<'a> {
    pub(in crate::app) fn new(kit: &'a mut Kit, view: &'a mut KitView) -> Self {
        Self { kit, view }
    }
    /// Forget everything this kit holds for one open document: the parsed tag,
    /// an in-flight load, its previews, Find filter, edit drafts and, for a
    /// folder pane, its browser state.
    ///
    /// This used to be written out in four places (closing a tab, closing all,
    /// closing all but one, deleting a tag), each clearing a different subset:
    /// only the delete path dropped the model preview, whose geometry and
    /// textures therefore outlived every closed tab.
    pub(in crate::app) fn drop_document(&mut self, key: &str) {
        self.kit.parsed_tags.remove(key);
        self.kit.loading_tags.remove(key);
        self.view.caches.bitmap_previews.remove(key);
        self.view.caches.model_previews.remove(key);
        self.view.find_filter_applied.remove(key);
        self.view.edit_buffers.forget_tag(key);
        self.view.browser.folder_browsers.remove(key);
    }

    /// [`Self::drop_document`] for every document except `keep`.
    pub(in crate::app) fn drop_documents_except(&mut self, keep: Option<&str>) {
        let keys: HashSet<String> = self
            .kit
            .parsed_tags
            .keys()
            .chain(self.kit.loading_tags.iter())
            .chain(self.view.caches.bitmap_previews.keys())
            .chain(self.view.caches.model_previews.keys())
            .chain(self.view.find_filter_applied.keys())
            .chain(self.view.browser.folder_browsers.keys())
            .filter(|key| Some(key.as_str()) != keep)
            .cloned()
            .collect();
        for key in &keys {
            self.drop_document(key);
        }
        // Drafts are keyed "<tag>|<field>", including ones for tags that were
        // never loaded, so they are trimmed by prefix rather than by key.
        match keep {
            None => self.view.edit_buffers.clear(),
            Some(keep) => {
                let prefix = format!("{keep}|");
                self.view
                    .edit_buffers
                    .retain(|draft, _| draft.starts_with(&prefix));
            }
        }
    }

    /// Rewrite every open tag key through `map`, after a move or rename has
    /// changed the keys underneath them.
    ///
    /// The tree is where this has to land: `open_tabs` is re-derived from it
    /// every frame, so remapping only the list is overwritten immediately and
    /// the panes keep pointing at keys their source no longer has.
    pub(in crate::app) fn remap_tag_keys(&mut self, map: &HashMap<String, String>) {
        for (_, tile) in self.view.tag_tree.tiles.iter_mut() {
            if let egui_tiles::Tile::Pane(key) = tile
                && let Some(new_key) = map.get(key)
            {
                *key = new_key.clone();
            }
        }
        if let Some(selected) = self.kit.selected_key.as_ref()
            && let Some(new_key) = map.get(selected)
        {
            self.kit.selected_key = Some(new_key.clone());
        }
        self.sync_open_tabs();
    }

    /// Re-derive `open_tabs` from the tree. Called after anything that can
    /// change the layout: a frame of `tree.ui`, an open, or a close.
    pub(in crate::app) fn sync_open_tabs(&mut self) {
        self.kit.open_tabs = self.view.tabs_from_tree();
        if self
            .kit
            .selected_key
            .as_ref()
            .is_some_and(|key| !self.kit.open_tabs.contains(key))
        {
            self.kit.selected_key = self
                .kit
                .open_tabs
                .iter()
                .find(|key| !is_folder_pane_key(key))
                .cloned();
        }
    }

    /// Add `key` as a pane if it is not already laid out, and select it.
    pub(in crate::app) fn open_tag_pane(&mut self, key: &str) {
        if let Some(tile_id) = self.view.tile_for_key(key) {
            self.view.tag_tree.make_active(|id, _| id == tile_id);
        } else {
            let tree = &mut self.view.tag_tree;
            let tile_id = tree.tiles.insert_pane(key.to_owned());
            match tree.root() {
                Some(root) => {
                    if let Some(egui_tiles::Tile::Container(container)) = tree.tiles.get_mut(root) {
                        container.add_child(tile_id);
                    } else {
                        // A bare pane at the root: wrap both in a tab group.
                        let tabs = tree.tiles.insert_tab_tile(vec![root, tile_id]);
                        tree.root = Some(tabs);
                    }
                }
                None => tree.root = Some(tile_id),
            }
            tree.make_active(|id, _| id == tile_id);
        }
        self.kit.selected_key = Some(key.to_owned());
        self.sync_open_tabs();
    }

    /// Open `key` as a pane split beside the existing layout, rather than as
    /// another tab in the same group. This is what alt-click does — the
    /// successor to tearing a tag out into its own window.
    pub(in crate::app) fn open_tag_pane_beside(&mut self, key: &str) {
        if self.view.tile_for_key(key).is_some() {
            self.open_tag_pane(key);
            return;
        }
        let tree = &mut self.view.tag_tree;
        let tile_id = tree.tiles.insert_pane(key.to_owned());
        match tree.root() {
            Some(root) => {
                let split = tree.tiles.insert_horizontal_tile(vec![root, tile_id]);
                tree.root = Some(split);
            }
            None => tree.root = Some(tile_id),
        }
        self.kit.selected_key = Some(key.to_owned());
        self.sync_open_tabs();
    }

    /// Remove `key`'s pane from the layout.
    pub(in crate::app) fn close_tag_pane(&mut self, key: &str) {
        if let Some(tile_id) = self.view.tile_for_key(key) {
            self.view.tag_tree.remove_recursively(tile_id);
        }
        self.view.browser.folder_browsers.remove(key);
        self.sync_open_tabs();
    }
}

#[cfg(test)]
mod tests;
