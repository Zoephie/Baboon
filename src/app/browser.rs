//! Tag-browser tree, list, filtering, and context-menu presentation.
//! It owns tag-browser filtering and presentation; source discovery, document loading, and edit application belong elsewhere.

use super::*;

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
}

impl ModifiedTags {
    pub(in crate::app) fn contains_key(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    pub(in crate::app) fn insert(&mut self, entry: &TagEntry) {
        self.keys.insert(entry.key.clone());
    }

    /// Whether anything under this folder is modified.
    ///
    /// Walks the subtree and stops at the first hit, so a folder that contains
    /// an edit answers immediately; only clean folders pay for their whole
    /// subtree, and an empty set skips the walk entirely.
    pub(in crate::app) fn subtree_has_modified(
        &self,
        node: &TagTreeNode,
        entries: &[TagEntry],
    ) -> bool {
        if self.keys.is_empty() {
            return false;
        }
        node.entries.iter().any(|&index| {
            entries
                .get(index)
                .is_some_and(|entry| self.keys.contains(&entry.key))
        }) || node
            .children
            .iter()
            .any(|child| self.subtree_has_modified(child, entries))
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

pub(in crate::app) fn set_browser_game(ui: &Ui, game: Option<String>) {
    ui.data_mut(|data| data.insert_temp(browser_game_id(), game.unwrap_or_default()));
}

pub(in crate::app) fn browser_game_is_campaign_evolved(ui: &Ui) -> bool {
    ui.data(|data| data.get_temp::<String>(browser_game_id()))
        .is_some_and(|game| game == "haloce_evolved")
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
    availability: crate::app::controller::ScenarioLaunchAvailability,
) {
    ui.data_mut(|data| data.insert_temp(scenario_launch_id(), availability));
}

pub(in crate::app) fn browser_scenario_launch(
    ui: &Ui,
) -> crate::app::controller::ScenarioLaunchAvailability {
    ui.data(|data| {
        data.get_temp::<crate::app::controller::ScenarioLaunchAvailability>(scenario_launch_id())
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
