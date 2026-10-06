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
    /// How the kit's Chimp surface is browsed and laid out.
    pub(in crate::app) chimp: ChimpView,
    /// Each tag pane's field row heights, by pane scope and tag key.
    pub(in crate::app) row_heights: HashMap<String, crate::app::editor::RowHeights>,
}

impl KitView {
    /// Forget the field row heights measured for `key`, in every pane scope.
    pub(in crate::app) fn forget_row_heights(&mut self, key: &str) {
        let suffix = format!("\u{1f}{key}");
        self.row_heights.retain(|pane, _| !pane.ends_with(&suffix));
    }

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
            chimp: ChimpView::default(),
            row_heights: HashMap::new(),
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
    ///
    /// Walked from the root rather than read off the tile store, which is a
    /// hash map: its order is arbitrary, and the selection used to fall back
    /// to whichever tag came first in it.
    pub(in crate::app) fn tabs_from_tree(&self) -> Vec<String> {
        let tiles = &self.tag_tree.tiles;
        let mut keys = Vec::new();
        let mut stack: Vec<egui_tiles::TileId> = self.tag_tree.root().into_iter().collect();
        while let Some(id) = stack.pop() {
            match tiles.get(id) {
                Some(egui_tiles::Tile::Pane(key)) => keys.push(key.clone()),
                Some(egui_tiles::Tile::Container(container)) => {
                    stack.extend(container.children_vec().into_iter().rev());
                }
                None => {}
            }
        }
        // A pane the root no longer reaches is still laid out until the tree
        // is simplified; keep it listed rather than dropping its document.
        for tile in tiles.tiles() {
            if let egui_tiles::Tile::Pane(key) = tile
                && !keys.contains(key)
            {
                keys.push(key.clone());
            }
        }
        keys
    }

    /// The panes on screen under `from`, in layout order: every pane of a
    /// split, and the tab in front of each tab group.
    fn shown_panes_under(&self, from: egui_tiles::TileId) -> Vec<String> {
        let tiles = &self.tag_tree.tiles;
        let mut keys = Vec::new();
        let mut stack = vec![from];
        while let Some(id) = stack.pop() {
            match tiles.get(id) {
                Some(egui_tiles::Tile::Pane(key)) => keys.push(key.clone()),
                Some(egui_tiles::Tile::Container(container)) => {
                    let children: Vec<_> = container.active_children(tiles).collect();
                    stack.extend(children.into_iter().rev());
                }
                None => {}
            }
        }
        keys
    }

    /// Whether `tile` is drawn: every container above it shows it.
    fn is_shown(&self, tile: egui_tiles::TileId) -> bool {
        self.tag_tree.active_tiles().contains(&tile)
    }

    /// What is selected when the user is looking at `front`: a tag, or nothing
    /// for a folder pane, which no tag action applies to.
    fn selection_for_front(front: Option<&String>) -> Option<String> {
        front.filter(|key| !is_folder_pane_key(key)).cloned()
    }

    /// The selection once `tile` is no longer in front: whatever is now shown
    /// in the nearest group above it that is on screen.
    fn selection_covering(&self, tile: egui_tiles::TileId) -> Option<String> {
        let mut node = tile;
        while let Some(parent) = self.tag_tree.tiles.parent_of(node) {
            if self.is_shown(parent) {
                return Self::selection_for_front(self.shown_panes_under(parent).first());
            }
            node = parent;
        }
        self.first_shown_selection()
    }

    /// The selection when there is no better place for it: the first tab on
    /// screen.
    fn first_shown_selection(&self) -> Option<String> {
        let shown = self.tag_tree.root().map(|root| self.shown_panes_under(root)).unwrap_or_default();
        shown.into_iter().find(|key| !is_folder_pane_key(key))
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
        self.view.forget_row_heights(key);
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
    /// Returns whether the open tabs or the selection changed.
    ///
    /// The selection is what Save, Undo, Ctrl+W and Find's "Current tag" act
    /// on, so it is kept on a tab the user can see. A selected tab that is
    /// covered — by a click on another tab, a drag, or a tab opened in front
    /// of it — hands the selection to whatever covers it.
    pub(in crate::app) fn sync_open_tabs(&mut self) -> bool {
        self.sync_open_tabs_after_close(None)
    }

    /// [`Self::sync_open_tabs`], after closing a pane that was in `closed_from`:
    /// a closed selection moves to the tab that group now shows, which is the
    /// one the user is looking at.
    fn sync_open_tabs_after_close(&mut self, closed_from: Option<egui_tiles::TileId>) -> bool {
        let open_tabs = self.view.tabs_from_tree();
        let mut changed = open_tabs != self.kit.open_tabs;
        self.kit.open_tabs = open_tabs;
        let Some(selected) = self.kit.selected_key.as_deref() else {
            return changed;
        };
        let next = match self.view.tile_for_key(selected) {
            Some(tile) if self.view.is_shown(tile) => return changed,
            Some(tile) => self.view.selection_covering(tile),
            None => match closed_from {
                Some(group) if self.view.is_shown(group) => {
                    KitView::selection_for_front(self.view.shown_panes_under(group).first())
                }
                _ => self.view.first_shown_selection(),
            },
        };
        if next != self.kit.selected_key {
            self.kit.selected_key = next;
            changed = true;
        }
        changed
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
        // A folder pane in front selects no tag, the same as clicking it.
        self.kit.selected_key = (!is_folder_pane_key(key)).then(|| key.to_owned());
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
        self.kit.selected_key = (!is_folder_pane_key(key)).then(|| key.to_owned());
        self.sync_open_tabs();
    }

    /// Remove `key`'s pane from the layout.
    pub(in crate::app) fn close_tag_pane(&mut self, key: &str) {
        let mut closed_from = None;
        if let Some(tile_id) = self.view.tile_for_key(key) {
            closed_from = self.view.tag_tree.tiles.parent_of(tile_id);
            self.view.tag_tree.remove_recursively(tile_id);
        }
        self.view.browser.folder_browsers.remove(key);
        self.sync_open_tabs_after_close(closed_from);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::browser::{BrowserMode, BrowserSort, folder_pane_key};

    fn empty_source() -> LoadedSourceData {
        LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("/nonexistent-baboon-view-test"),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        }
    }

    /// Every open kit has a view and every view belongs to an open kit.
    fn assert_in_step(app: &Baboon) {
        for kit in &app.model.kits {
            assert!(app.views.views.contains_key(&kit.id), "{:?} has no view", kit.id);
        }
        assert_eq!(app.views.views.len(), app.model.kits.len(), "a closed kit kept its view");
    }

    #[test]
    fn adding_and_closing_kits_keeps_their_views_in_step() {
        let mut app = Baboon::for_test();
        assert_in_step(&app);
        let first = app.model.kits[0].id;
        let second = app.add_kit();
        assert_in_step(&app);
        app.remove_kit(first);
        assert_in_step(&app);
        assert!(!app.views.views.contains_key(&first));
        // Closing the last kit leaves a fresh empty one, with its own view.
        app.remove_kit(second);
        assert_in_step(&app);
        assert_ne!(app.model.kits[0].id, second);
    }

    /// A new kit's browser opens in the view the prefs say, as before the split.
    #[test]
    fn a_new_kit_opens_its_browser_as_the_prefs_say() {
        let mut app = Baboon::for_test();
        app.model.prefs.browser_mode = BrowserMode::Groups;
        app.model.prefs.browser_sort = BrowserSort::Type;
        let id = app.add_kit();
        assert_eq!(app.views[id].browser.mode, BrowserMode::Groups);
        assert_eq!(app.views[id].browser.sort, BrowserSort::Type);
    }

    /// Loading a source into a kit starts its view over but keeps how its browser
    /// lists tags, which belongs to the workspace rather than the source.
    #[test]
    fn a_reload_starts_the_view_over_but_keeps_the_browser_mode() {
        let mut app = Baboon::for_test();
        let id = app.model.kits[0].id;
        app.views[id].browser.mode = BrowserMode::Groups;
        app.views[id].browser.filter = "warthog".to_owned();
        app.views[id].pending_expand.insert("tag".to_owned(), true);
        app.install_loaded_source(empty_source());
        assert_in_step(&app);
        assert_eq!(app.views[id].browser.mode, BrowserMode::Groups);
        assert!(app.views[id].browser.filter.is_empty());
        assert!(app.views[id].pending_expand.is_empty());
    }

    // The selection is what Save, Undo and Ctrl+W act on. It has to stay on a
    // tab the user can see, whatever changed the layout under it.

    fn tag(n: usize) -> String {
        format!("file:/tags/objects/t{n:02}.weapon")
    }

    /// The tags on screen, read off the tree the way it draws.
    fn shown(view: &KitView) -> Vec<String> {
        view.tag_tree.root().map(|root| view.shown_panes_under(root)).unwrap_or_default()
    }

    #[test]
    fn open_tabs_follow_tab_order() {
        let mut kit = Kit::empty(KitId(0), Default::default());
        let mut view = KitView::for_test(&kit);
        let mut both = KitMut::new(&mut kit, &mut view);
        for n in 0..12 {
            both.open_tag_pane(&tag(n));
        }
        assert_eq!(kit.open_tabs, (0..12).map(tag).collect::<Vec<_>>());
    }

    #[test]
    fn closing_the_selected_tab_selects_the_tab_shown_in_its_place() {
        let mut kit = Kit::empty(KitId(0), Default::default());
        let mut view = KitView::for_test(&kit);
        let mut both = KitMut::new(&mut kit, &mut view);
        for n in 0..12 {
            both.open_tag_pane(&tag(n));
        }
        both.open_tag_pane(&tag(7));
        both.close_tag_pane(&tag(7));
        assert_eq!(shown(both.view), vec![tag(0)], "the group falls back to its first tab");
        assert_eq!(both.kit.selected_key, Some(tag(0)), "and the selection with it");
    }

    #[test]
    fn a_covered_tab_hands_the_selection_to_the_one_in_front() {
        let mut kit = Kit::empty(KitId(0), Default::default());
        let mut view = KitView::for_test(&kit);
        let mut both = KitMut::new(&mut kit, &mut view);
        both.open_tag_pane(&tag(0));
        both.open_tag_pane(&tag(1));
        // What a drag or a hover over the tab bar does: the tree brings another
        // tab forward with no click to focus it.
        let front = both.view.tile_for_key(&tag(0)).unwrap();
        both.view.tag_tree.make_active(|id, _| id == front);
        both.sync_open_tabs();
        assert_eq!(both.kit.selected_key, Some(tag(0)));
    }

    #[test]
    fn a_folder_pane_in_front_selects_no_tag() {
        let mut kit = Kit::empty(KitId(0), Default::default());
        let mut view = KitView::for_test(&kit);
        let mut both = KitMut::new(&mut kit, &mut view);
        both.open_tag_pane(&tag(0));
        both.open_tag_pane(&folder_pane_key(Path::new("objects")));
        assert_eq!(both.kit.selected_key, None, "not the tag behind it");
        both.open_tag_pane(&tag(0));
        assert_eq!(both.kit.selected_key, Some(tag(0)));
    }

    #[test]
    fn a_selection_in_one_half_of_a_split_survives_the_other_half_changing() {
        let mut kit = Kit::empty(KitId(0), Default::default());
        let mut view = KitView::for_test(&kit);
        let mut both = KitMut::new(&mut kit, &mut view);
        both.open_tag_pane(&tag(0));
        both.open_tag_pane(&tag(1));
        both.open_tag_pane_beside(&tag(2));
        both.open_tag_pane(&tag(1));
        assert_eq!(shown(both.view), vec![tag(1), tag(2)]);
        assert_eq!(both.kit.selected_key, Some(tag(1)));
        // The other half's tab group changes; the selected tag is still shown.
        let other = both.view.tile_for_key(&tag(2)).unwrap();
        both.view.tag_tree.make_active(|id, _| id == other);
        both.sync_open_tabs();
        assert_eq!(both.kit.selected_key, Some(tag(1)));
    }
}
