//! Reference-path normalization and occurrence navigation helpers.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;
use crate::app::kits::terminal::trim_terminal_lines;

impl Baboon {
    /// Applies `WorkerMessage::ReverseDependenciesBuilt`, rejecting stale source generations.
    pub(in crate::app) fn handle_reverse_dependencies_built(
        &mut self,
        stamp: KitStamp,
        index: ReverseDependencyIndex,
        missing: usize,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            return true;
        };
        // The build is over whether or not its result is still wanted.
        self.model.kits[kit_index].index_jobs.reference_progress = None;
        self.model.kits[kit_index].index_jobs.building_references = false;
        let paired_entry_index_build =
            std::mem::take(&mut self.model.kits[kit_index].index_jobs.references_for_entry_index);
        let changed_during_build = std::mem::take(
            &mut self.model.kits[kit_index]
                .index_jobs
                .references_changed_during_build,
        );
        self.dialogs.close::<IndexingNotice>();
        if self.model.resolve_stamp(stamp).is_none() {
            return true;
        }
        // The build read every tag before it started returning; a tag saved,
        // refreshed, created or deleted since then was read as it was. Those
        // changes are laid over the result, or replacing the index (and the
        // saved one) with it would undo them, and since the saved tag's
        // fingerprint is current, no refresh would ever bring them back.
        let mut index = index;
        for (key, references) in changed_during_build {
            match references {
                Some(references) => index.set_tag_dependencies(key, references),
                None => index.clear_tag(&key),
            }
        }
        if let Some(source) = self.model.kits[kit_index].source.as_mut() {
            let n = index.len();
            // An incomplete index is used for this session but not saved: saved,
            // it would load back next time as complete.
            if let (0, Some(game), TagSource::LooseFolder { root, .. }) =
                (missing, source.game.clone(), &source.source)
            {
                let root = root.clone();
                let to_save = index.clone();
                spawn_background("reverse-dependency index save", move || {
                    if let Err(e) =
                        crate::core::source::save_reverse_dependency_index(game.as_str(), &root, &to_save)
                    {
                        eprintln!("reverse-dependency index save failed: {e}");
                    }
                });
            }
            source.reverse_dependencies = Some(index);
            self.model.status = if missing > 0 {
                format!(
                    "Reference index built without {missing} tag(s): a reader crashed on them. \
                     Rebuild it to try again."
                )
            } else if paired_entry_index_build {
                format!("Tag and reference indexes complete: {n} tags")
            } else {
                format!("Reference index complete: {n} tags")
            };
        }
        false
    }

    /// Applies `WorkerMessage::ReferenceIndexProgress`, rejecting stale or inactive builds.
    pub(in crate::app) fn handle_reference_index_progress(
        &mut self,
        stamp: KitStamp,
        processed: usize,
        total: usize,
        ctx: &egui::Context,
    ) -> bool {
        // Drives the global progress bar only; the stamp is checked purely so
        // a closed or reloaded kit's progress stops updating it.
        let Some(kit_index) = self.model.resolve_stamp(stamp) else {
            return true;
        };
        if !self.model.kits[kit_index].index_jobs.building_references {
            return true;
        }
        if let Some(progress) = self.model.kits[kit_index].index_jobs.reference_progress.as_mut() {
            progress.processed = processed;
            progress.total = total;
        }
        ctx.request_repaint();
        false
    }

    /// Applies `WorkerMessage::FolderRefactorProgress` to the visible refactor state.
    pub(in crate::app) fn handle_folder_refactor_progress(
        &mut self,
        progress: FolderRefactorProgress,
    ) -> bool {
        self.tag_ops.folder_refactor = Some(FolderRefactorUiState {
            label: progress.label.clone(),
            phase: progress.phase.clone(),
            progress: progress.progress,
        });
        self.model.status = format!("{}: {}", progress.label, progress.phase);
        false
    }

    /// Applies `WorkerMessage::FolderRefactorFinished` and remaps open state after moves.
    ///
    /// Everything here lands on the kit the refactor was started in. It used to
    /// land on the active kit, so a move that finished after the user switched
    /// workspaces rebuilt the *other* game's browser from these results and
    /// dropped its open documents and unsaved edit buffers along the way.
    pub(in crate::app) fn handle_folder_refactor_finished(
        &mut self,
        stamp: KitStamp,
        result: Result<FolderRefactorFinished, String>,
    ) -> bool {
        self.tag_ops.folder_refactor = None;
        let done = match result {
            Ok(done) => done,
            Err(error) => {
                self.model.status = error;
                return false;
            }
        };
        // The kit was closed or reloaded while the job ran: the work on disk is
        // done, but there is no longer anything here to apply it to.
        let Some(kit_index) = self.model.resolve_stamp(stamp) else {
            self.model.status = done.status;
            return false;
        };
        if let Some(source) = self.model.kits[kit_index].source.as_mut() {
            source.entries.clear();
            source.all_entries = done.all_entries;
            source.tree = done.tree;
            source.group_tree = crate::core::source::build_group_tree(&source.all_entries);
            source.reverse_dependencies = done.reverse_dependencies;
            if let TagSource::LooseFolder { root, .. } = &source.source {
                if !source.all_entries.is_empty()
                    && let Some(game) = source.game.map(GameId::as_str)
                {
                    let _ = crate::core::source::save_entry_index(game, root, &source.all_entries);
                }
                if let (Some(game), Some(reverse_dependencies)) =
                    (source.game.map(GameId::as_str), source.reverse_dependencies.as_ref())
                {
                    let _ = crate::core::source::save_reverse_dependency_index(
                        game,
                        root,
                        reverse_dependencies,
                    );
                }
            }
        }
        if done.moved {
            let moved_folder = done
                .moved_folder
                .as_ref()
                .map(|(from, to)| (from.as_path(), to.as_path()));
            self.remap_favorites_for_kit(kit_index, &done.old_to_new_keys, moved_folder);
            self.kit_and_view(kit_index).remap_tag_keys(&done.old_to_new_keys);
            // Keywords are filed by tag key: they move with the tags.
            let keywords = &mut self.model.kits[kit_index].keywords;
            for (old, new) in &done.old_to_new_keys {
                keywords.rekey_tag(old, new);
            }
            keywords.save_if_dirty();
            // A docked browser on the folder, or inside it, follows it.
            if let Some((from, to)) = moved_folder {
                let kit_id = self.model.kits[kit_index].id;
                self.views[kit_id].browser.remap_folder_browser_paths(from, to);
            }
        }
        let kit = &mut self.model.kits[kit_index];
        let view = &mut self.views[kit.id];
        kit.parsed_tags.clear();
        kit.loading_tags.clear();
        view.caches.bitmap_previews.clear();
        view.caches.model_previews.clear();
        view.edit_buffers.clear();
        view.find_filter_applied.clear();
        kit.generation = kit.generation.wrapping_add(1);
        // Every tree and search result names paths that just moved.
        view.browser.filter_cache = FilterCache::default();
        for pane in view.browser.folder_browsers.values_mut() {
            pane.cached_generation = u64::MAX;
            pane.cached_source_len = usize::MAX;
            pane.tree = TagTree::default();
            pane.group_tree = TagTree::default();
            pane.group_tree_for = None;
            pane.filter_cache = FilterCache::default();
            pane.date_cache = FolderDateCache::default();
        }
        self.kit_tools.terminal
            .lines
            .extend(done.lines.into_iter().map(TerminalLineEntry::new));
        trim_terminal_lines(&mut self.kit_tools.terminal.lines);
        self.kit_tools.terminal.scroll_to_bottom = true;
        self.model.status = done.status;
        false
    }
}

pub(in crate::app) fn normalize_ref(rel_path: &str) -> String {
    crate::core::source::normalize_dependency_path(rel_path)
}

pub(in crate::app) fn ancestor_block_indices(field_path: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut acc = String::new();
    for segment in field_path.split('/') {
        let (name, index) = match segment.strip_suffix(']').and_then(|s| s.rsplit_once('[')) {
            Some((name, idx)) => (name, idx.parse::<usize>().ok()),
            None => (segment, None),
        };
        // Foundation omits ordinals from inherited Unit/Object wrapper IDs.
        // Reference-jump paths can still arrive with those schema ordinals, so
        // normalize them at this shared navigation boundary as well as accepting
        // the canonical plain-wrapper paths produced by Find.
        let name = name
            .rsplit_once('#')
            .filter(|(base, _)| is_inherited_parent_name(base))
            .map_or(name, |(base, _)| base);
        let node_path = if acc.is_empty() {
            name.to_owned()
        } else {
            format!("{acc}/{name}")
        };
        match index {
            Some(index) => {
                out.push((node_path.clone(), index));
                acc = format!("{node_path}[{index}]");
            }
            None => acc = node_path,
        }
    }
    out
}

pub(in crate::app) fn occurrence_label(field_path: &str) -> String {
    field_path
        .split('/')
        .map(|segment| match segment.split_once('[') {
            Some((name, rest)) => {
                format!("{}[{rest}", clean_field_name(strip_ordinal_token(name)))
            }
            None => clean_field_name(strip_ordinal_token(segment)).to_string(),
        })
        .collect::<Vec<_>>()
        .join(" › ")
}

/// Drop a trailing `#ordinal` positional token from a path segment's name
/// part, leaving the display name (`Mapping#5` → `Mapping`).
fn strip_ordinal_token(name: &str) -> &str {
    name.split('#').next().unwrap_or(name)
}

pub(in crate::app) fn dependency_entry_reference_path(
    entry: &TagEntry,
    names: &TagNameIndex,
) -> Option<String> {
    reference_path_without_group_extension(&entry.display_path, entry.group_tag, names)
}

pub(in crate::app) fn normalized_reference_lookup_path(
    path: &str,
    group_tag: u32,
    names: &TagNameIndex,
) -> String {
    let mut path = sanitize_ref_path(path).replace('/', "\\");
    if let Some(extension) = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
    {
        let suffix = format!(".{extension}");
        if path
            .to_ascii_lowercase()
            .ends_with(&suffix.to_ascii_lowercase())
        {
            path.truncate(path.len().saturating_sub(suffix.len()));
        }
    }
    normalize_ref(&path)
}

pub(in crate::app) fn container_entry_for_reference<'a>(
    entries: &'a [TagEntry],
    group_tag: u32,
    rel_path: &str,
    names: &TagNameIndex,
) -> Option<&'a TagEntry> {
    let target = normalized_reference_lookup_path(rel_path, group_tag, names);
    entries.iter().find(|entry| {
        // A tag created this session is a legitimate reference target — it is
        // addressed by the same logical path a saved one is, and "Open
        // referenced tag" reported it missing while it was excluded here.
        matches!(
            &entry.location,
            TagEntryLocation::Container { .. } | TagEntryLocation::NewContainer { .. }
        ) && entry.group_tag == group_tag
            && normalized_reference_lookup_path(&entry.display_path, entry.group_tag, names)
                == target
    })
}

pub(in crate::app) fn reference_path_without_group_extension(
    path: &str,
    group_tag: u32,
    names: &TagNameIndex,
) -> Option<String> {
    let extension = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag));
    let mut path = path.replace('/', "\\");
    if let Some(extension) = extension {
        let suffix = format!(".{extension}");
        if path
            .to_ascii_lowercase()
            .ends_with(&suffix.to_ascii_lowercase())
        {
            let keep = path.len().saturating_sub(suffix.len());
            path.truncate(keep);
            return Some(path);
        }
    }
    Path::new(&path)
        .with_extension("")
        .to_str()
        .map(|path| path.replace('/', "\\"))
}

pub(in crate::app) fn dependency_leaf_key(rel_path: &str) -> String {
    rel_path
        .replace('/', "\\")
        .rsplit('\\')
        .next()
        .unwrap_or(rel_path)
        .to_ascii_lowercase()
}

pub(in crate::app) fn dependency_target_exists(tags_root: &Path, rel_path: &str, extension: &str) -> bool {
    resolve_tag_path(tags_root, rel_path, extension).is_file()
}

#[cfg(test)]
mod incomplete_index_tests {
    use super::*;

    /// A build that lost tags to a crashed reader says so, rather than
    /// reporting a complete index.
    #[test]
    fn an_incomplete_reference_index_is_reported_as_such() {
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::SingleFile {
                path: PathBuf::from("a.model"),
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
        });
        let stamp = app.model.kit_stamp();

        app.handle_reverse_dependencies_built(stamp, ReverseDependencyIndex::default(), 3);

        assert!(app.model.status.contains("without 3 tag"), "{}", app.model.status);
    }
}

#[cfg(test)]
mod reference_path_tests {

    use super::*;

    #[test]
    fn ancestor_block_indices_splits_indexed_path() {
        // Nested blocks: each pair's path is the drawn `path_prefix` (parent
        // indices kept, own index dropped).
        assert_eq!(
            ancestor_block_indices("custom references[3]/sounds[1]/melee sound"),
            vec![
                ("custom references".to_owned(), 3),
                ("custom references[3]/sounds".to_owned(), 1),
            ],
        );
        // A plain struct segment between blocks carries no selection.
        assert_eq!(
            ancestor_block_indices("weapon[2]/melee/damage sound"),
            vec![("weapon".to_owned(), 2)],
        );
        // A top-level (unindexed) reference field has no ancestor blocks.
        assert_eq!(
            ancestor_block_indices("havok cleanup resources"),
            Vec::<(String, usize)>::new(),
        );
        assert_eq!(
            ancestor_block_indices("custom references#5[3]/sounds#2[1]/melee sound#4"),
            vec![
                ("custom references#5".to_owned(), 3),
                ("custom references#5[3]/sounds#2".to_owned(), 1),
            ],
        );
        // Foundation renders inherited wrappers without ordinals, so selector
        // IDs beneath Unit/Object must preserve those plain wrapper segments.
        assert_eq!(
            ancestor_block_indices("unit/object/functions#25[2]/import name#3"),
            vec![("unit/object/functions#25".to_owned(), 2)],
        );
        // Reference-jump paths may retain schema ordinals on inherited wrappers;
        // normalize those to the same selector ID as canonical Find paths.
        assert_eq!(
            ancestor_block_indices("unit#0/object#0/functions#25[2]/import name#3"),
            vec![("unit/object/functions#25".to_owned(), 2)],
        );
    }

    #[test]
    fn occurrence_label_keeps_indices_and_cleans_names() {
        assert_eq!(
            occurrence_label("custom references[3]/melee sound"),
            "custom references[3] › melee sound",
        );
        assert_eq!(
            occurrence_label("havok cleanup resources"),
            "havok cleanup resources"
        );
        assert_eq!(
            occurrence_label("custom references#5[3]/melee sound#4"),
            "custom references[3] › melee sound",
        );
    }

    #[test]
    fn normalize_ref_matches_dependency_key_form() {
        assert_eq!(
            normalize_ref("Sound/Materials/Hard/Human_Weap_Melee"),
            normalize_ref("sound\\materials\\hard\\human_weap_melee"),
        );
    }
}

#[cfg(test)]
mod folder_browser_integration_tests {
    use super::*;
    use crate::app::browser::{BrowserAction, BrowserSearchScope, folder_pane_key};

    /// A folder renamed while docked browsers show it, or folders inside
    /// it: the panes follow with their view choices, the tags' keywords move
    /// with them, and another workspace's keywords are left alone.
    #[test]
    fn folder_rename_preserves_views_and_keywords_in_the_originating_workspace() {
        let mut app = Baboon::for_test();
        let root = PathBuf::from("C:/test-tags");
        app.install_loaded_source(LoadedSourceData {
            label: "test".into(),
            source: TagSource::LooseFolder {
                root: root.clone(),
                game: None,
                definitions_root: locate_definitions_root(),
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
            complete_scan: true,
            chosen_kit_layout: None,
        });
        for path in [
            "objects/brute",
            "objects/brute/bitmaps",
            "objects/brute_other",
        ] {
            app.handle_browser_action(
                BrowserAction::OpenFolderBrowser {
                    rel_path: path.into(),
                    label: path.rsplit('/').next().unwrap().into(),
                    open_in_new_tab: true,
                },
                egui::Context::default(),
            );
        }
        let pane_key = folder_pane_key(Path::new("objects/brute"));
        let kit0 = app.model.kits[0].id;
        let pane = app.views[kit0].browser.folder_browsers.get_mut(&pane_key).unwrap();
        pane.assets_view = true;
        pane.asset_bitmaps = false;
        pane.filter = "armor".into();
        pane.search_scope = BrowserSearchScope {
            tags: true,
            folders: false,
            keywords: true,
        };
        let old_key = format!("file:{}", root.join("objects/brute/armor.bitmap").display());
        let new_key = format!("file:{}", root.join("objects/elite/armor.bitmap").display());
        app.model.kits[0].keywords.add(&old_key, "wip");
        app.kit_and_view(0).open_tag_pane(&old_key);
        let stamp = app.model.kit_stamp();
        app.add_kit();
        app.model.kits[1].keywords.add(&old_key, "other kit");
        app.handle_folder_refactor_finished(
            stamp,
            Ok(FolderRefactorFinished {
                status: "Renamed".into(),
                lines: Vec::new(),
                tree: TagTree::default(),
                all_entries: Vec::new(),
                reverse_dependencies: None,
                old_to_new_keys: HashMap::from([(old_key.clone(), new_key.clone())]),
                moved: true,
                moved_folder: Some(("objects/brute".into(), "objects/elite".into())),
            }),
        );
        assert_eq!(app.model.active, 1);
        let kit = &app.model.kits[0];
        let view = &app.views[kit0];
        let pane = &view.browser.folder_browsers[&pane_key];
        assert_eq!(pane.rel_path, Path::new("objects/elite"));
        assert_eq!(pane.label, "elite");
        assert!(pane.assets_view && !pane.asset_bitmaps);
        assert_eq!(pane.filter, "armor");
        assert!(pane.search_scope.keywords);
        assert_eq!(pane.cached_generation, u64::MAX);
        assert!(kit.open_tabs.contains(&pane_key));
        assert!(kit.open_tabs.contains(&new_key));
        assert_eq!(kit.keywords.keywords(&new_key), &["wip"]);
        assert!(kit.keywords.keywords(&old_key).is_empty());
        assert_eq!(
            view.browser.folder_browsers[&folder_pane_key(Path::new("objects/brute/bitmaps"))].rel_path,
            Path::new("objects/elite/bitmaps")
        );
        assert_eq!(
            view.browser.folder_browsers[&folder_pane_key(Path::new("objects/brute_other"))].rel_path,
            Path::new("objects/brute_other")
        );
        assert_eq!(app.model.kits[1].keywords.keywords(&old_key), &["other kit"]);
    }
}
