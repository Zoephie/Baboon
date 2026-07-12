//! Reference-path normalization and occurrence navigation helpers.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::ReverseDependenciesBuilt`, rejecting stale source generations.
    pub(super) fn handle_reverse_dependencies_built(
        &mut self,
        generation: u64,
        index: ReverseDependencyIndex,
    ) -> bool {
        self.building_reverse_dependencies = false;
        if generation != self.source_generation {
            return true;
        }
        self.reference_index_progress = None;
        let paired_entry_index_build = self.building_reference_for_entry_index;
        self.building_reference_for_entry_index = false;
        self.show_entry_index_wait_notice = false;
        if let Some(source) = self.source.as_mut() {
            let n = index.len();
            if let (Some(game), TagSource::LooseFolder { root, .. }) =
                (source.game.clone(), &source.source)
            {
                let root = root.clone();
                let to_save = index.clone();
                thread::spawn(move || {
                    if let Err(e) =
                        crate::source::save_reverse_dependency_index(&game, &root, &to_save)
                    {
                        eprintln!("reverse-dependency index save failed: {e}");
                    }
                });
            }
            source.reverse_dependencies = Some(index);
            self.status = if paired_entry_index_build {
                format!("Tag and reference indexes complete: {n} tags")
            } else {
                format!("Reference index complete: {n} tags")
            };
        }
        false
    }

    /// Applies `WorkerMessage::ReferenceIndexProgress`, rejecting stale or inactive builds.
    pub(super) fn handle_reference_index_progress(
        &mut self,
        generation: u64,
        processed: usize,
        total: usize,
        ctx: &egui::Context,
    ) -> bool {
        if generation != self.source_generation || !self.building_reverse_dependencies {
            return true;
        }
        if let Some(progress) = self.reference_index_progress.as_mut() {
            progress.processed = processed;
            progress.total = total;
        }
        ctx.request_repaint();
        false
    }

    /// Applies `WorkerMessage::FolderRefactorProgress` to the visible refactor state.
    pub(super) fn handle_folder_refactor_progress(
        &mut self,
        progress: FolderRefactorProgress,
    ) -> bool {
        self.folder_refactor = Some(FolderRefactorUiState {
            label: progress.label.clone(),
            phase: progress.phase.clone(),
            progress: progress.progress,
        });
        self.status = format!("{}: {}", progress.label, progress.phase);
        false
    }

    /// Applies `WorkerMessage::FolderRefactorFinished` and remaps open state after moves.
    pub(super) fn handle_folder_refactor_finished(
        &mut self,
        result: Result<FolderRefactorFinished, String>,
    ) -> bool {
        self.folder_refactor = None;
        let done = match result {
            Ok(done) => done,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        if let Some(source) = self.source.as_mut() {
            source.entries.clear();
            source.all_entries = done.all_entries;
            source.tree = done.tree;
            source.group_tree = crate::source::build_group_tree(&source.all_entries);
            source.reverse_dependencies = done.reverse_dependencies;
            if let TagSource::LooseFolder { root, .. } = &source.source {
                if !source.all_entries.is_empty()
                    && let Some(game) = source.game.as_deref()
                {
                    let _ = crate::source::save_entry_index(game, root, &source.all_entries);
                }
                if let (Some(game), Some(reverse_dependencies)) =
                    (source.game.as_deref(), source.reverse_dependencies.as_ref())
                {
                    let _ = crate::source::save_reverse_dependency_index(
                        game,
                        root,
                        reverse_dependencies,
                    );
                }
            }
        }
        if done.moved {
            self.remap_current_favorites(&done.old_to_new_keys);
            remap_open_tag_keys(&mut self.open_tabs, &done.old_to_new_keys);
            remap_hashset_keys(&mut self.floating_tabs, &done.old_to_new_keys);
            if let Some(selected) = self.selected_key.clone()
                && let Some(new_key) = done.old_to_new_keys.get(&selected)
            {
                self.selected_key = Some(new_key.clone());
            }
        }
        self.parsed_tags.clear();
        self.loading_tags.clear();
        self.tag_cache_order.clear();
        self.bitmap_previews.clear();
        self.model_previews.clear();
        self.edit_buffers.clear();
        self.field_search.clear();
        self.field_search_applied.clear();
        self.source_generation = self.source_generation.wrapping_add(1);
        self.terminal
            .lines
            .extend(done.lines.into_iter().map(TerminalLineEntry::new));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
        self.status = done.status;
        false
    }
}

pub(super) fn normalize_ref(rel_path: &str) -> String {
    crate::source::normalize_dependency_path(rel_path)
}

pub(super) fn ancestor_block_indices(field_path: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut acc = String::new();
    for segment in field_path.split('/') {
        let (name, index) = match segment.strip_suffix(']').and_then(|s| s.rsplit_once('[')) {
            Some((name, idx)) => (name, idx.parse::<usize>().ok()),
            None => (segment, None),
        };
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

pub(super) fn occurrence_label(field_path: &str) -> String {
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

pub(super) fn dependency_entry_reference_path(
    entry: &TagEntry,
    names: &TagNameIndex,
) -> Option<String> {
    reference_path_without_group_extension(&entry.display_path, entry.group_tag, names)
}

pub(super) fn reference_path_without_group_extension(
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

pub(super) fn dependency_leaf_key(rel_path: &str) -> String {
    rel_path
        .replace('/', "\\")
        .rsplit('\\')
        .next()
        .unwrap_or(rel_path)
        .to_ascii_lowercase()
}

pub(super) fn dependency_target_exists(tags_root: &Path, rel_path: &str, extension: &str) -> bool {
    resolve_tag_path(tags_root, rel_path, extension).is_file()
}
