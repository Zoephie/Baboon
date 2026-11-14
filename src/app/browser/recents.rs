//! Recently closed loose tags, persisted per tags folder and rendered as ordinary browser rows.
use super::*;
use crate::app::prefs::EditingKitRecents;

pub(in crate::app) fn recent_relative_path(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = clean_recent_path(fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()));
    let path = clean_recent_path(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
    path.strip_prefix(root).ok().map(Path::to_path_buf).and_then(clean_favorite_relative_path)
}

impl GuiPrefs {
    /// `paths` is already ordered newest first (active tag first for a bulk close).
    pub(in crate::app) fn remember_closed_tags(&mut self, root: &Path, paths: Vec<PathBuf>) {
        let paths: Vec<_> = paths.into_iter().filter_map(clean_favorite_relative_path)
            .filter(|path| root.join(path).is_file()).collect();
        if paths.is_empty() { return; }
        let index = self.editing_kit_recents.iter().position(|kit| same_recent_path(&kit.tags_root, root))
            .unwrap_or_else(|| {
                self.editing_kit_recents.push(EditingKitRecents { tags_root: root.to_path_buf(), tags: Vec::new() });
                self.editing_kit_recents.len() - 1
            });
        let tags = &mut self.editing_kit_recents[index].tags;
        for path in paths.into_iter().rev() {
            tags.retain(|existing| !same_recent_path(existing, &path));
            tags.insert(0, path);
        }
        tags.truncate(5);
    }
}

impl Baboon {
    pub(in crate::app) fn remember_closed_tag_keys(&mut self, kit_index: usize, mut keys: Vec<String>) {
        let kit = &self.model.kits[kit_index];
        let Some(root) = self.model.loaded_tags_root_for(kit_index) else { return; };
        if let Some(active) = &kit.current_pane {
            if let Some(index) = keys.iter().position(|key| key == active) {
                let active = keys.remove(index);
                keys.insert(0, active);
            }
        }
        let Some(source) = &kit.source else { return; };
        let paths = keys.iter().filter(|key| kit.open_tabs.contains(key)).filter_map(|key| {
            let entry = source.entry_for_key(key)?;
            let TagEntryLocation::LooseFile(path) = &entry.location else { return None; };
            recent_relative_path(&root, path)
        }).collect();
        self.model.prefs.remember_closed_tags(&root, paths);
        self.refresh_recent_entries_for(kit_index);
    }

    pub(in crate::app) fn refresh_recent_entries_for(&mut self, kit_index: usize) {
        let kit = &self.model.kits[kit_index];
        let id = kit.id;
        self.views[id].browser.recent_entries.clear();
        let Some(root) = self.model.loaded_tags_root_for(kit_index) else { return; };
        let Some(index) = self.model.prefs.editing_kit_recents.iter().position(|kit| same_recent_path(&kit.tags_root, &root)) else { return; };
        let tags = &mut self.model.prefs.editing_kit_recents[index].tags;
        tags.retain(|path| root.join(path).is_file());
        for path in tags.iter() {
            if let Ok(Some(entry)) = loose_file_entry(&root, &root.join(path), &kit.names) {
                self.views[id].browser.recent_entries.push(entry);
            }
        }
    }
}

pub(super) fn draw_recents(
    ui: &mut Ui, entries: &[TagEntry], selected: Option<&str>, filter: &str,
    show_prefixes: bool, double_click_to_open: bool, favorite_keys: &HashSet<String>,
    search_scope: BrowserSearchScope, keywords: &std::collections::BTreeMap<String, Vec<String>>,
    game: Option<GameId>,
) -> Option<BrowserAction> {
    let entries: Vec<_> = entries.iter().filter(|entry| scoped_entry_matches(entry, filter, search_scope,
        keywords.get(&entry.key).map(Vec::as_slice).unwrap_or_default())).collect();
    if entries.is_empty() { return None; }
    let mut action = None;
    show_browser_navigation_section(ui, "browser_recents", "Recents", ButtonIcon::Recent, text_dark(), |ui| {
        for entry in entries {
            let row_action = draw_entry(ui, entry, selected, show_prefixes, double_click_to_open,
                None, Some(favorite_keys), true, game);
            if action.is_none() { action = row_action; }
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::*;
    use crate::app::documents::PendingCloseAction;

    fn fixture(name: &str) -> LooseKit {
        let kit = LooseKit::new(name, "halo3_mcc");
        for index in 0..7 { kit.write_mcc(&format!("objects/tag{index}"), "model", |_| {}); }
        kit
    }

    #[test]
    fn deleting_a_recent_tag_removes_the_row_and_persisted_history() {
        let kit = fixture("recents-delete-action");
        let mut app = app();
        kit.install(&mut app);
        let key = kit.open(&mut app, "objects/tag0.model");
        app.close_tab(&key);
        let id = app.model.kits[0].id;
        assert_eq!(app.views[id].browser.recent_entries.len(), 1);
        app.open_delete_tag(&key);
        app.begin_delete_tag(ctx());
        assert!(!kit.path("objects/tag0.model").exists());
        assert!(app.views[id].browser.recent_entries.is_empty());
        assert!(app.model.prefs.editing_kit_recents[0].tags.is_empty());
    }

    #[test]
    fn recents_keep_five_unique_paths_per_kit_and_follow_moves_and_deletions() {
        let kit = fixture("recents-history");
        let other = fixture("recents-other-kit");
        let mut app = app();
        kit.install(&mut app);
        for index in 0..7 {
            let key = kit.open(&mut app, &format!("objects/tag{index}.model"));
            app.close_tab(&key);
        }
        let tags = &app.model.prefs.editing_kit_recents[0].tags;
        assert_eq!(tags, &(2..7).rev().map(|i| PathBuf::from(format!("objects/tag{i}.model"))).collect::<Vec<_>>());
        let key = kit.open(&mut app, "objects/tag3.model");
        assert_eq!(app.model.prefs.editing_kit_recents[0].tags[0], Path::new("objects/tag6.model"), "opening does not reorder history");
        app.close_tab(&key);
        assert_eq!(app.model.prefs.editing_kit_recents[0].tags.len(), 5);
        assert_eq!(app.model.prefs.editing_kit_recents[0].tags[0], Path::new("objects/tag3.model"));
        app.model.prefs.remember_closed_tags(&other.root, vec![PathBuf::from("objects/tag0.model")]);
        assert_eq!(app.model.prefs.editing_kit_recents.len(), 2);
        let from = kit.path("objects/tag3.model");
        let to = kit.path("objects/renamed.model");
        fs::rename(&from, &to).unwrap();
        app.remap_favorites_for_kit(0, &HashMap::from([(file_entry_key(&from), file_entry_key(&to))]), None);
        assert_eq!(app.model.prefs.editing_kit_recents[0].tags[0], Path::new("objects/renamed.model"));
        fs::remove_file(to).unwrap();
        app.refresh_recent_entries_for(0);
        assert_eq!(app.model.prefs.editing_kit_recents[0].tags.len(), 4);
        assert_eq!(app.views[app.model.kits[0].id].browser.recent_entries.len(), 4);
    }

    #[test]
    fn bulk_and_kit_closes_prioritize_the_active_tag_and_ignore_folder_tabs() {
        let kit = fixture("recents-bulk");
        let mut app = app();
        kit.install(&mut app);
        let mut keys = Vec::new();
        for index in 0..7 { keys.push(kit.open(&mut app, &format!("objects/tag{index}.model"))); }
        app.kit_and_view(0).focus_pane(keys[3].clone());
        app.execute_close_action(PendingCloseAction::CloseTabGroup {kit:app.model.active_kit_id(),keys:keys[1..].to_vec(),keep:Some(keys[0].clone())},&ctx());
        assert_eq!(app.model.prefs.editing_kit_recents[0].tags,
            [3, 1, 2, 4, 5].map(|i| PathBuf::from(format!("objects/tag{i}.model"))));
        app.handle_browser_action(BrowserAction::OpenFolderBrowser {
            rel_path: PathBuf::from("objects"), label: "objects".into(), open_in_new_tab: true,
        }, ctx());
        let id = app.model.kits[0].id;
        app.execute_close_action(PendingCloseAction::CloseKit(id), &ctx());
        assert_eq!(app.model.prefs.editing_kit_recents[0].tags[0], Path::new("objects/tag0.model"));
        assert!(app.model.prefs.editing_kit_recents[0].tags.iter().all(|path| path.extension().is_some()));
    }
}
