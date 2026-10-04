//! The tag browser: the folder and group trees with their filter and menus,
//! the browser panel, what its rows and menus do (opening, copying,
//! favourites, revealing), and the bitmap and model browsers with their
//! thumbnails.

use super::*;
use crate::app::compare::{CompareCommand, GIT_REVIEW_TITLE};
use crate::app::editor::{
    build_bitmap_preview, format_tag_reference_input, geometry_import_verb_for_group_name,
    truncate_for_cell,
};
use crate::app::export::is_scenario_group;

mod filter;
mod tree;

pub(super) use filter::*;
pub(super) use tree::*;

/// Which tags in a workspace carry edits that are not written into the game.
///
/// Resolved once per change rather than per frame: mapping a tag key to its
/// entry is a linear scan of the source, so doing it for every modified tag
/// every frame would cost far more than the handful of lookups it represents.
#[derive(Default)]
pub(in crate::app) struct ModifiedTags {
    keys: HashSet<String>,
    /// Every folder above a modified tag, as a lowercased `/`-separated path.
    folders: HashSet<String>,
    /// The Groups-view node of every modified tag.
    groups: HashSet<String>,
}

/// A tree node's or display path's folder in the form `folders` keys use.
fn folder_key(path: &str) -> String {
    path.replace('\\', "/")
        .trim_matches('/')
        .to_ascii_lowercase()
}

impl ModifiedTags {
    pub(in crate::app) fn contains_key(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    pub(in crate::app) fn insert(&mut self, entry: &TagEntry) {
        self.keys.insert(entry.key.clone());
        let path = folder_key(&entry.display_path);
        let mut folder = path.as_str();
        while let Some((parent, _)) = folder.rsplit_once('/') {
            self.folders.insert(parent.to_owned());
            folder = parent;
        }
        self.groups.insert(crate::core::source::group_tree_label(entry));
    }

    /// Whether anything under this folder (or Groups-view node) is modified.
    ///
    /// Answered from the ancestors recorded when the set was built, which
    /// happens only when it changes. It used to walk the node's subtree, every
    /// frame for every folder header drawn: the top-level folders together
    /// cover the whole source, so any unsaved edit cost a walk of every entry
    /// per frame. It also missed a modified tag inside a folder whose contents
    /// had not been loaded yet.
    pub(in crate::app) fn subtree_has_modified(&self, node: &TagTreeNode) -> bool {
        if self.keys.is_empty() {
            return false;
        }
        self.folders
            .contains(&folder_key(&node.rel_path.to_string_lossy()))
            || self.groups.contains(&node.label)
    }
}

/// egui-memory slot holding the browser's modified set for the kit currently
/// being drawn. Kept here rather than threaded through the tree's dozen
/// drawing functions: the browsers draw one after another, so the value in
/// memory while a kit's tree is drawn is that kit's own — the same mechanism
/// the Find highlighter uses to reach individual field cells.
pub(in crate::app) fn modified_tags_id() -> egui::Id {
    egui::Id::new("browser_modified_tags")
}

pub(in crate::app) fn set_browser_modified_tags(ui: &Ui, modified: std::sync::Arc<ModifiedTags>) {
    ui.data_mut(|data| data.insert_temp(modified_tags_id(), modified));
}

pub(in crate::app) fn browser_modified_tags(ui: &Ui) -> Option<std::sync::Arc<ModifiedTags>> {
    ui.data(|data| data.get_temp::<std::sync::Arc<ModifiedTags>>(modified_tags_id()))
}

fn favorite_folders_id() -> egui::Id {
    egui::Id::new("browser_favorite_folders")
}

pub(in crate::app) fn set_browser_favorite_folders(
    ui: &Ui,
    folders: Option<std::sync::Arc<Vec<PathBuf>>>,
) {
    ui.data_mut(|data| data.insert_temp(favorite_folders_id(), folders));
}

pub(in crate::app) fn browser_favorite_folders(ui: &Ui) -> Option<std::sync::Arc<Vec<PathBuf>>> {
    ui.data(|data| data.get_temp::<Option<std::sync::Arc<Vec<PathBuf>>>>(favorite_folders_id()))
        .flatten()
}

fn folder_pane_browser_id() -> egui::Id {
    egui::Id::new("browser_is_folder_pane")
}

pub(in crate::app) fn set_browser_is_folder_pane(ui: &Ui, is_folder_pane: bool) {
    ui.data_mut(|data| data.insert_temp(folder_pane_browser_id(), is_folder_pane));
}

pub(in crate::app) fn browser_is_folder_pane(ui: &Ui) -> bool {
    ui.data(|data| data.get_temp(folder_pane_browser_id()).unwrap_or(false))
}

/// Browser keys of the container tags this installation created by duplicating,
/// published the same way and for the same reason as the modified set.
///
/// Only these may be deleted: once a copy is in the pak it is indistinguishable
/// from a tag the game shipped, so the enablement has to come from Baboon's own
/// ledger rather than from anything in the container.
fn deletable_keys_id() -> egui::Id {
    egui::Id::new("browser_deletable_keys")
}

pub(in crate::app) fn set_browser_deletable_keys(ui: &Ui, keys: std::sync::Arc<HashSet<String>>) {
    ui.data_mut(|data| data.insert_temp(deletable_keys_id(), keys));
}

pub(in crate::app) fn browser_deletable_keys(ui: &Ui) -> Option<std::sync::Arc<HashSet<String>>> {
    ui.data(|data| data.get_temp::<std::sync::Arc<HashSet<String>>>(deletable_keys_id()))
}

/// Game id of the kit currently being drawn, published the same way and for the
/// same reason as the modified set: menu items that only apply to one game need
/// it, and the tree's drawing functions have no other route to it.
fn browser_game_id() -> egui::Id {
    egui::Id::new("browser_game_id")
}

pub(in crate::app) fn set_browser_game(ui: &Ui, game: Option<GameId>) {
    ui.data_mut(|data| data.insert_temp(browser_game_id(), game));
}

fn browser_game(ui: &Ui) -> Option<GameId> {
    ui.data(|data| data.get_temp::<Option<GameId>>(browser_game_id()))
        .flatten()
}

pub(in crate::app) fn browser_game_is_campaign_evolved(ui: &Ui) -> bool {
    browser_game(ui).is_some_and(GameId::is_campaign_evolved)
}

/// Whether the loaded game's bitmap tags keep the source image they were
/// imported from. Halo CE and Halo 2 keep it as a compressed color plate (1817 of
/// the 1818 stock CE bitmaps, 4067 of the 4184 H2 ones). Halo 3 onward has a
/// `source data` field instead, empty in all 11161 stock Halo 3 bitmaps, so
/// there is nothing to recover there.
pub(in crate::app) fn bitmaps_keep_source_images(game: GameId) -> bool {
    game.is_classic()
}

/// [`bitmaps_keep_source_images`] for the game the browser is drawing.
pub(in crate::app) fn browser_game_keeps_bitmap_sources(ui: &Ui) -> bool {
    browser_game(ui).is_some_and(bitmaps_keep_source_images)
}

fn browser_sound_language_id() -> egui::Id {
    egui::Id::new("browser_sound_language")
}

/// Publish the shared sound player's last-selected language for browser menus.
/// `None` is the game's preferred English bank; name it rather than exposing
/// the implementation term "Default" in an extraction command.
pub(in crate::app) fn set_browser_sound_language(
    ui: &Ui,
    game: Option<GameId>,
    language: Option<&str>,
) {
    let label = sound_language_label(game, language);
    ui.data_mut(|data| data.insert_temp(browser_sound_language_id(), label));
}

fn sound_language_label(game: Option<GameId>, language: Option<&str>) -> String {
    match language {
        Some(language) => crate::app::editor::language_label(language),
        None if matches!(game, Some(GameId::Halo4) | Some(GameId::Halo2Amp)) => {
            "English (US)".to_owned()
        }
        None => "English".to_owned(),
    }
}

pub(in crate::app) fn browser_sound_language(ui: &Ui) -> String {
    ui.data(|data| data.get_temp::<String>(browser_sound_language_id()))
        .unwrap_or_else(|| "English".to_owned())
}

fn browser_sound_available_languages_id() -> egui::Id {
    egui::Id::new("browser_sound_available_languages")
}

/// Publish how many localized bank sets are actually installed. `None` means
/// the source does not use an external per-language bank family.
pub(in crate::app) fn set_browser_sound_available_languages(
    ui: &Ui,
    game: Option<GameId>,
    tags_root: Option<&Path>,
) {
    let count = tags_root.and_then(|root| match game {
        Some(GameId::Halo3) | Some(GameId::Halo3Odst) | Some(GameId::HaloReach) => {
            Some(blam_tags::audio::SoundBanks::available_languages(root).len())
        }
        Some(GameId::Halo4) | Some(GameId::Halo2Amp) => {
            Some(blam_tags::audio::WwiseBanks::available_languages(root).len())
        }
        _ => None,
    });
    ui.data_mut(|data| data.insert_temp(browser_sound_available_languages_id(), count));
}

pub(in crate::app) fn browser_sound_available_languages(ui: &Ui) -> Option<usize> {
    ui.data(|data| data.get_temp(browser_sound_available_languages_id()))
        .flatten()
}

fn browser_entries_scanning_id() -> egui::Id {
    egui::Id::new("browser_entries_scanning")
}

pub(in crate::app) fn set_browser_entries_scanning(ui: &Ui, scanning: bool) {
    ui.data_mut(|data| data.insert_temp(browser_entries_scanning_id(), scanning));
}

pub(in crate::app) fn browser_entries_scanning(ui: &Ui) -> bool {
    ui.data(|data| {
        data.get_temp(browser_entries_scanning_id())
            .unwrap_or(false)
    })
}

fn browser_loose_source_id() -> egui::Id {
    egui::Id::new("browser_loose_source")
}

pub(in crate::app) fn set_browser_loose_source(ui: &Ui, loose: bool) {
    ui.data_mut(|data| data.insert_temp(browser_loose_source_id(), loose));
}

pub(in crate::app) fn browser_loose_source(ui: &Ui) -> bool {
    ui.data(|data| data.get_temp(browser_loose_source_id()).unwrap_or(false))
}

/// What the kit currently being drawn can launch a scenario in, published the
/// same way and for the same reason as the modified set: the row menu offers
/// Sapien and tag_test, and the tree's drawing functions have no other route to
/// the executables on disk.
fn scenario_launch_id() -> egui::Id {
    egui::Id::new("browser_scenario_launch")
}

pub(in crate::app) fn set_browser_scenario_launch(
    ui: &Ui,
    availability: crate::app::kits::scenario_launch::ScenarioLaunchAvailability,
) {
    ui.data_mut(|data| data.insert_temp(scenario_launch_id(), availability));
}

pub(in crate::app) fn browser_scenario_launch(
    ui: &Ui,
) -> crate::app::kits::scenario_launch::ScenarioLaunchAvailability {
    ui.data(|data| {
        data.get_temp::<crate::app::kits::scenario_launch::ScenarioLaunchAvailability>(scenario_launch_id())
    })
    .unwrap_or_default()
}

/// Colour for a tag that this workspace created, with no counterpart in the
/// game. Paired with a `+` marker wherever it is used, so the meaning does not
/// rest on colour alone.
pub(in crate::app) fn added_text() -> Color32 {
    Color32::from_rgb(126, 186, 108)
}

/// Background wash behind the shipped side of a diff, and behind the edited
/// side. Kept dark: the editor draws its own widgets on top, and a strong fill
/// would fight them rather than frame them.
pub(in crate::app) fn removed_wash() -> Color32 {
    Color32::from_rgb(52, 30, 30)
}

pub(in crate::app) fn added_wash() -> Color32 {
    Color32::from_rgb(28, 46, 32)
}

#[cfg(test)]
mod sound_language_tests;

/// Colour for a value or element that is going away. Paired with a `-` marker,
/// so red and green never carry the meaning by themselves.
pub(in crate::app) fn removed_text() -> Color32 {
    Color32::from_rgb(214, 106, 106)
}

/// Colour for a tag or folder holding edits that are not written into the game.
/// The same goldenrod the workspace tab is tinted with.
pub(in crate::app) fn modified_text() -> Color32 {
    Color32::from_rgb(214, 168, 46)
}

#[cfg(test)]
mod modified_tags_tests;
pub(in crate::app) mod panel;
pub(in crate::app) use panel::{draw_folder_browser_pane, draw_kit_browser};
pub(in crate::app) mod actions;
pub(in crate::app) mod bitmap_browser;
pub(in crate::app) use bitmap_browser::*;
pub(in crate::app) mod model_browser;
pub(in crate::app) use model_browser::*;
pub(in crate::app) mod thumbnail_library;
pub(in crate::app) use thumbnail_library::*;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;
pub(in crate::app) mod keyword_chooser;
pub(in crate::app) use keyword_chooser::KeywordChooser;

/// The browser: a tag waiting to be revealed.
pub(in crate::app) struct BrowserFeature {
    pub(in crate::app) reveal_target: Option<RevealRequest>,
}

/// How this kit's browser lists its tags: mode, order and filter, the docked
/// folder browsers, and the modified, deletable and favourite sets it marks,
/// each with what it was built from.
#[derive(Default)]
pub(in crate::app) struct KitBrowser {
    /// How this kit's browser lists tags, and in what order. Per kit because
    /// the useful view differs by game — a folder-organized editing kit reads
    /// best as Folders while a container source reads best as Groups — and two
    /// browsers are on screen at once in a split. New kits start from the
    /// saved [`Baboon::default_browser_mode`], so a single workspace behaves
    /// exactly as it did when this was one application-wide setting.
    pub(in crate::app) mode: BrowserMode,
    pub(in crate::app) sort: BrowserSort,
    pub(in crate::app) filter: String,
    pub(in crate::app) filter_cache: FilterCache,
    /// Docked folder browsers, keyed by their synthetic tag-tree pane key.
    pub(in crate::app) folder_browsers: HashMap<String, FolderBrowserState>,
    /// Which tags the browser should mark as modified, and the signature the
    /// set was built from. Rebuilt only when that signature changes: resolving
    /// a tag key to its entry is a linear scan of the source, so doing it for
    /// every dirty tag every frame would cost far more than the handful of
    /// lookups it represents.
    pub(in crate::app) modified_tags: std::sync::Arc<ModifiedTags>,
    pub(in crate::app) modified_signature: Vec<String>,
    /// Browser keys this workspace may delete, and the generation they were
    /// resolved at. Recomputed only when the generation moves: answering it
    /// walks every entry and stats each container's backups, which is far too
    /// much to repeat for every frame the browser draws.
    pub(in crate::app) deletable_keys: std::sync::Arc<HashSet<String>>,
    pub(in crate::app) deletable_keys_generation: Option<u64>,
}

impl KitBrowser {
    /// A browser opening in this view, with nothing filtered or cached yet.
    pub(in crate::app) fn new(mode: BrowserMode, sort: BrowserSort) -> Self {
        Self {
            mode,
            sort,
            ..Self::default()
        }
    }
}

/// What the browser can be asked to do.
pub(in crate::app) enum BrowserCommand {
    /// A browser action raised in `kit`'s workspace, which becomes active
    /// first: the action addresses the active kit.
    Action { kit: KitId, action: BrowserAction },
    /// Give the tag at `key` in `kit` a keyword.
    AddKeyword {
        kit: KitId,
        key: String,
        keyword: String,
    },
    /// Take a keyword off the tag at `key` in `kit`.
    RemoveKeyword {
        kit: KitId,
        key: String,
        keyword: String,
    },
    /// List the active kit's tags carrying a keyword.
    ShowTagsWithKeyword(String),
    /// Load the folders at `paths` in one of `kit`'s lazy loose-folder trees,
    /// which a browser drew open before they had loaded.
    LoadFolders {
        kit: KitId,
        tree: LazyTree,
        paths: Vec<PathBuf>,
    },
    /// Index every tag in `kit`'s loose folder, for a view that needs them all.
    ScanAllEntries { kit: KitId },
    /// What a cell of one of `kit`'s thumbnail libraries asked for.
    LibraryCell {
        kit: KitId,
        library: Library,
        action: CellAction,
    },
}

impl Baboon {
    pub(in crate::app) fn apply_browser_command(&mut self, command: BrowserCommand, ctx: &egui::Context) {
        match command {
            BrowserCommand::Action { kit, action } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.model.active = index;
                    self.handle_browser_action(action, ctx.clone());
                }
            }
            BrowserCommand::AddKeyword { kit, key, keyword } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.model.kits[index].keywords.add(&key, &keyword);
                }
            }
            BrowserCommand::ShowTagsWithKeyword(keyword) => self.show_tags_with_keyword(&keyword),
            BrowserCommand::LoadFolders { kit, tree, paths } => self.load_browser_folders(kit, tree, &paths),
            BrowserCommand::ScanAllEntries { kit } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.model.active = index;
                    self.begin_scan_all_entries(ctx.clone());
                }
            }
            BrowserCommand::LibraryCell { kit, library, action } => {
                let Some(index) = self.model.kit_index(kit) else {
                    return;
                };
                // Each acts on the active kit, so this one first.
                self.model.active = index;
                match (library, action) {
                    (Library::Bitmaps, CellAction::Open(key)) => self.select_entry(key, ctx.clone()),
                    // Opens a native folder picker, which blocks until it is
                    // answered: not something to do part-way through a draw.
                    (Library::Bitmaps, CellAction::MenuAction(key)) => {
                        self.begin_extract_bitmap(key, ctx.clone())
                    }
                    // A model opens the `.model` that owns it, or the render
                    // model itself when the kit has none; the right-click item
                    // opens the clicked tag with no resolution.
                    (Library::Models, CellAction::Open(key)) => {
                        let open = self.model.resolve_model_browser_open(index, &key);
                        self.select_entry(open, ctx.clone());
                    }
                    (Library::Models, CellAction::MenuAction(key)) => self.select_entry(key, ctx.clone()),
                }
            }
            BrowserCommand::RemoveKeyword { kit, key, keyword } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.model.kits[index].keywords.remove(&key, &keyword);
                }
            }
        }
    }
}

/// Which of a kit's lazy loose-folder trees a folder belongs to.
pub(in crate::app) enum LazyTree {
    /// The sidebar's, the source's own tree.
    Source,
    /// A docked folder pane's, by its pane key.
    Pane(String),
}

impl Baboon {
    /// Load folders a browser drew open before they had loaded.
    fn load_browser_folders(&mut self, kit: KitId, tree: LazyTree, paths: &[PathBuf]) {
        let Some(index) = self.model.kit_index(kit) else {
            return;
        };
        let kit = &mut self.model.kits[index];
        let view = &mut self.views[kit.id];
        let Some(source) = kit.source.as_mut() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return;
        };
        let root = root.clone();
        let names = source.names.clone();
        let status = match tree {
            LazyTree::Source => {
                // The Groups view's tree is kept in step only while nothing
                // better exists: see `load_lazy_folders`.
                let group_tree = source.all_entries.is_empty().then_some(&mut source.group_tree);
                load_lazy_folders(&mut source.tree, &mut source.entries, group_tree, &root, &names, paths)
            }
            // A pane's Groups view is rebuilt from the full index; nothing to
            // keep in step.
            LazyTree::Pane(key) => match view.browser.folder_browsers.get_mut(&key) {
                Some(pane) => load_lazy_folders(&mut pane.tree, &mut source.entries, None, &root, &names, paths),
                None => None,
            },
        };
        if let Some(status) = status {
            self.model.status = status;
        }
    }
}
