//! Selection actions: one preflight, one review, one rollback journal per job.
use super::*;
use crate::app::references::dependency_entry_reference_path;
use crate::app::shell::{FolderRefactorFinished, FolderRefactorUiState, WorkerMessage};
use crate::app::tag_ops::{folder_merge_ops, folder_merge_review as review};
use crate::core::{folder_merge as disk, recovery};

#[derive(Clone)]
pub(in crate::app) struct BulkRequest {
    pub stamp: KitStamp,
    pub pane: String,
    pub keys: Vec<String>,
    pub action: BulkAction,
    /// Where a Move or Copy goes when it is already chosen (a drop onto a
    /// folder); `None` asks with a folder picker.
    pub destination: Option<PathBuf>,
}
pub(in crate::app) struct BulkPrompt {
    pub request: BulkRequest,
    pub message: String,
}
impl Dialog for BulkPrompt {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let mut open = true;
        let modal = egui::Modal::new(egui::Id::unique("bulk_browser_prompt"))
            .frame(egui::Frame::window(&cx.egui.global_style()))
            .show(cx.egui, |ui| {
                ui.set_width(window_width(cx.egui, 480.0));
                crate::app::search::draw_icon_window_header_without_close(
                    ui,
                    "Confirm Selection Action",
                    ButtonIcon::FolderClosed,
                );
                ui.add_space(12.0);
                ui.label(&self.message);
                ui.add_space(12.0);
                popup_footer(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked() {
                            open = false;
                        }
                        if icon_text_button(
                            ui,
                            ButtonIcon::Confirm,
                            if self.message.starts_with("Save all") {
                                "Save All and Continue"
                            } else {
                                "Continue"
                            },
                            true,
                        )
                        .clicked()
                        {
                            cx.send(BrowserCommand::BulkConfirmed(self.request.clone()));
                            open = false;
                        }
                    });
                });
            });
        open && !modal.should_close()
    }
}
pub(in crate::app) struct BulkOutcome {
    pub done: FolderRefactorFinished,
    pub pane: String,
    pub completed: Vec<String>,
    pub deleted: Vec<String>,
    pub deleted_folders: Vec<PathBuf>,
    pub renamed: bool,
}

impl Baboon {
    pub(in crate::app) fn begin_browser_bulk(
        &mut self,
        pane: String,
        keys: Vec<String>,
        action: BulkAction,
        ctx: &egui::Context,
    ) {
        self.run_browser_bulk(
            BulkRequest {
                stamp: self.model.kit_stamp(),
                pane,
                keys,
                action,
                destination: None,
            },
            false,
            ctx,
        );
    }
    /// Items dropped onto `folder` (relative to the tags folder): the same
    /// Move or Copy as the selection menu's, with the folder already chosen.
    pub(in crate::app) fn begin_browser_drop(
        &mut self,
        pane: String,
        keys: Vec<String>,
        copy: bool,
        folder: PathBuf,
        ctx: &egui::Context,
    ) {
        let Some(root) = self.model.loaded_tags_root_for(self.model.active) else {
            return;
        };
        self.run_browser_bulk(
            BulkRequest {
                stamp: self.model.kit_stamp(),
                pane,
                keys,
                action: if copy {
                    BulkAction::Copy
                } else {
                    BulkAction::Move
                },
                destination: Some(root.join(folder)),
            },
            false,
            ctx,
        );
    }
    pub(in crate::app) fn run_browser_bulk(
        &mut self,
        request: BulkRequest,
        confirmed: bool,
        ctx: &egui::Context,
    ) {
        let Some(kit_index) = self.model.resolve_stamp(request.stamp) else {
            self.model.status = "The workspace changed; select the items again".into();
            return;
        };
        if self.tag_ops.folder_refactor.is_some() {
            self.model.status = "A file operation is already running".into();
            return;
        }
        self.focus_kit(kit_index);
        if request.keys.is_empty() {
            return;
        }
        if request.action == BulkAction::Rename {
            self.open_bulk_rename(request);
            return;
        }
        let Some(source) = self.model.source() else {
            return;
        };
        let entries = selection_entries(&request.keys, source.full_entry_set());
        if !matches!(
            request.action,
            BulkAction::Open | BulkAction::Move | BulkAction::Copy | BulkAction::Delete
        ) {
            let keys = export_keys(request.action, &entries);
            if keys.is_empty() {
                self.model.status = "No compatible tags in this selection".into();
                return;
            }
            let action = match request.action {
                BulkAction::Bitmaps => BrowserAction::ExtractBitmapFolder(keys),
                BulkAction::Sounds => BrowserAction::ExtractSound {
                    keys,
                    all_languages: false,
                },
                BulkAction::Shaders => BrowserAction::ExtractMaterialShaderSourceFolder(keys),
                BulkAction::Includes => BrowserAction::ExtractHlslIncludeFolder(keys),
                BulkAction::Raw => BrowserAction::ExtractContainerFolderTags {
                    label: "Selected Tags".into(),
                    keys,
                },
                _ => BrowserAction::DumpLoadedFolderJson(keys),
            };
            self.handle_browser_action(action, ctx.clone());
            return;
        }
        if request.action == BulkAction::Open {
            let new_tags = request
                .keys
                .iter()
                .filter(|key| {
                    !is_folder_pane_key(key) && !self.model.kits[kit_index].open_tabs.contains(key)
                })
                .count();
            let new_folders = request
                .keys
                .iter()
                .filter_map(|key| key.strip_prefix(FOLDER_PANE_PREFIX))
                .filter(|path| {
                    !self.views[request.stamp.kit]
                        .browser
                        .folder_browsers
                        .values()
                        .any(|pane| {
                            tag_path::relative_tag_path(&pane.rel_path).eq_ignore_ascii_case(path)
                        })
                })
                .count();
            if !confirmed && new_tags + new_folders >= 10 {
                self.dialogs.open(BulkPrompt {
                    message: format!("Open {new_tags} tag tabs and {new_folders} folder tabs?"),
                    request,
                });
                return;
            }
            for key in &request.keys {
                if let Some(path) = key.strip_prefix(FOLDER_PANE_PREFIX) {
                    let path = tag_path::to_relative(path);
                    let label = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    if request.keys.len() == 1 {
                        let kit = self.model.active_kit_id();
                        if let Some(pane) = self.views[kit]
                            .browser
                            .folder_browsers
                            .get_mut(&request.pane)
                        {
                            crate::app::shell::frame::navigate_folder_browser(pane, path, label);
                        }
                    } else {
                        self.handle_browser_action(
                            BrowserAction::OpenFolderBrowser {
                                rel_path: path,
                                label,
                                open_in_new_tab: false,
                            },
                            ctx.clone(),
                        );
                    }
                } else {
                    self.select_entry(key.clone(), ctx.clone());
                }
            }
            return;
        }
        if self.model.editing_kit_is_read_only(kit_index) {
            self.model.status = "This editing kit is read-only".into();
            return;
        }
        let TagSource::LooseFolder { root, .. } = &source.source else {
            self.model.status = "This selection cannot be changed as loose files".into();
            return;
        };
        let root = root.clone();
        let mut paths = Vec::new();
        for key in &request.keys {
            let path = if let Some(path) = key.strip_prefix(FOLDER_PANE_PREFIX) {
                root.join(tag_path::to_relative(path))
            } else {
                let Some(entry) = source.entry_for_key(key) else {
                    self.model.status = "A selected tag no longer exists".into();
                    return;
                };
                let TagEntryLocation::LooseFile(path) = &entry.location else {
                    self.model.status = "Every selected item must support this action".into();
                    return;
                };
                path.clone()
            };
            if !path.starts_with(&root) || path == root || !path.exists() {
                self.model.status = "A selected path is invalid or missing".into();
                return;
            }
            paths.push((key.clone(), path));
        }
        let dirty = self.model.kits[kit_index]
            .parsed_tags
            .values()
            .any(|doc| doc.dirty.is_set());
        if dirty && !confirmed {
            self.dialogs.open(BulkPrompt {
                message: "Save all unsaved tags in this workspace and continue?".into(),
                request,
            });
            return;
        }
        if dirty {
            let keys: Vec<_> = self.model.kits[kit_index]
                .parsed_tags
                .iter()
                .filter(|(_, doc)| doc.dirty.is_set())
                .map(|(key, _)| key.clone())
                .collect();
            for key in keys {
                if let Err(error) = self.save_tag_by_key(&key) {
                    self.model.status = error;
                    return;
                }
            }
        }
        let destination = if request.action == BulkAction::Delete {
            None
        } else {
            let start = paths[0].1.parent().unwrap_or(&root);
            let chosen = request.destination.clone().or_else(|| {
                rfd::FileDialog::new()
                    .set_title(if request.action == BulkAction::Move {
                        "Move Selected Items To"
                    } else {
                        "Copy Selected Items To"
                    })
                    .set_directory(start)
                    .pick_folder()
            });
            let Some(parent) = chosen else {
                return;
            };
            let resolved = match recovery::resolved_path(&parent) {
                Ok(path) => path,
                Err(error) => {
                    self.model.status = error;
                    return;
                }
            };
            if !resolved
                .starts_with(recovery::resolved_path(&root).unwrap_or_else(|_| root.clone()))
            {
                self.model.status = "Choose a destination inside the loaded tags folder".into();
                return;
            }
            Some(parent)
        };
        let source = self.model.source().unwrap();
        let previous = source.full_entry_set().to_vec();
        let index = source.reverse_dependencies.clone();
        let names = self.model.names().clone();
        let game = source.game;
        let context = self.new_merge_context();
        let reviewed = match (&request.action, &destination) {
            (BulkAction::Delete, _) => Some(review::ItemsAction::Delete),
            (BulkAction::Move, Some(folder)) => Some(review::ItemsAction::Move {
                destination: crate::app::browser::bulk_rename::folder_display(folder, &root),
            }),
            _ => None,
        };
        if let Some(action) = reviewed {
            // Open the review at once with what is selected; the check fills
            // in the rest.
            let roots: Vec<_> = paths.iter().map(|(_, path)| path.clone()).collect();
            if let Some(merge) = &mut self.tag_ops.folder_merge {
                merge.items_preview = Some(review::ItemsPreview {
                    action,
                    selected: selection_rows(&root, &roots, &previous),
                    game,
                });
            }
        }
        let stamp = request.stamp;
        let tx = self.tx.clone();
        // A delete first only checks what it would remove, so it is shown as
        // the question it leads to rather than as a delete in progress.
        self.tag_ops.folder_refactor = Some(FolderRefactorUiState {
            label: match request.action {
                BulkAction::Delete => review::DELETE_REVIEW_TITLE.into(),
                BulkAction::Move => "Moving Items".into(),
                _ => "Copying Items".into(),
            },
            phase: if request.action == BulkAction::Copy {
                "Preparing"
            } else {
                "Checking references"
            }
            .into(),
            progress: None,
        });
        self.spawn_job(
            move || WorkerMessage::BrowserBulkFinished {
                stamp,
                result: run_files(
                    root,
                    paths,
                    destination,
                    request,
                    names,
                    game,
                    previous,
                    index,
                    context,
                    &tx,
                ),
            },
            move |_| WorkerMessage::BrowserBulkFinished {
                stamp,
                result: Err("Selection operation worker crashed".into()),
            },
        );
    }
    pub(in crate::app) fn handle_browser_bulk_finished(
        &mut self,
        stamp: KitStamp,
        result: Result<BulkOutcome, String>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(index) = self.model.resolve_stamp(stamp) else {
            self.tag_ops.folder_refactor = None;
            self.tag_ops.folder_merge = None;
            return true;
        };
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => return self.handle_folder_refactor_finished(stamp, Err(error), ctx),
        };
        for key in &outcome.deleted {
            self.forget_campaign_overlay(index, key);
            self.model.kits[index].keywords.forget_tag(key);
            let mut both = self.kit_and_view(index);
            both.close_tag_pane(key);
            both.drop_document(key);
        }
        self.model.kits[index].keywords.save_if_dirty();
        let kit = stamp.kit;
        let close_folders: Vec<_> = self.views[kit]
            .browser
            .folder_browsers
            .iter()
            .filter(|(_, pane)| {
                outcome
                    .deleted_folders
                    .iter()
                    .any(|folder| pane.rel_path.starts_with(folder))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in close_folders {
            self.kit_and_view(index).close_tag_pane(&key);
        }
        let rename_map = outcome.renamed.then(|| {
            let mut map = outcome.done.old_to_new_keys.clone();
            for (from, to) in &outcome.done.moved_folders {
                map.insert(folder_pane_key(from), folder_pane_key(to));
            }
            map
        });
        let completed = outcome.completed;
        let pane = outcome.pane;
        self.handle_folder_refactor_finished(stamp, Ok(outcome.done), ctx);
        self.refresh_favorite_entries_for(index);
        self.model.kits[index].field_index.invalidate();
        if let Some(pane) = self.views[kit].browser.folder_browsers.get_mut(&pane) {
            if let Some(map) = rename_map {
                let remap = |key: String| map.get(&key).cloned().unwrap_or(key);
                pane.selection.keys = std::mem::take(&mut pane.selection.keys)
                    .into_iter()
                    .map(remap)
                    .collect();
                pane.selection.anchor = pane.selection.anchor.take().map(remap);
                pane.selection.focus = pane.selection.focus.take().map(remap);
                pane.selected_tag = pane.selected_tag.take().map(remap);
                for (key, _) in Arc::make_mut(&mut pane.selection.rows) {
                    if let Some(new) = map.get(key) {
                        *key = new.clone();
                    }
                }
            }
            pane.selection.keys.retain(|key| !completed.contains(key));
            if pane
                .selected_tag
                .as_ref()
                .is_some_and(|key| completed.contains(key))
            {
                pane.selected_tag = None;
            }
        }
        false
    }
}

fn resolved_item(path: &Path) -> Result<PathBuf, String> {
    if path.try_exists().map_err(|e| e.to_string())? {
        path.canonicalize().map_err(|e| e.to_string())
    } else {
        recovery::resolved_path(path)
    }
}

#[allow(clippy::too_many_arguments)]
fn run_files(
    root: PathBuf,
    paths: Vec<(String, PathBuf)>,
    destination: Option<PathBuf>,
    request: BulkRequest,
    names: TagNameIndex,
    game: Option<GameId>,
    previous: Vec<TagEntry>,
    index: Option<ReverseDependencyIndex>,
    context: folder_merge_ops::MergeContext,
    tx: &Sender<WorkerMessage>,
) -> Result<BulkOutcome, String> {
    let moving = request.action == BulkAction::Move;
    let mut roots = Vec::new();
    let mut names_seen = HashSet::new();
    let mut reserved = HashSet::new();
    for (_, path) in &paths {
        let resolved = resolved_item(path)?;
        if !resolved.starts_with(recovery::resolved_path(&root)?) {
            return Err("Selected links cannot escape the tags folder".into());
        }
        if request.action != BulkAction::Delete
            && !names_seen.insert(
                path.file_name()
                    .ok_or("Invalid name")?
                    .to_string_lossy()
                    .to_ascii_lowercase(),
            )
        {
            return Err("Two incoming items have the same destination name. Rename or deselect one before moving/copying; no files changed.".into());
        }
        if let Some(parent) = &destination {
            let mut target = parent.join(path.file_name().unwrap());
            let resolved_target = resolved_item(&target)?;
            if resolved_target == resolved {
                if moving {
                    continue;
                }
                target = disk::unused_name(&target, path.is_dir(), &reserved)?;
            } else if resolved_target.starts_with(&resolved) {
                return Err("Cannot move/copy a folder into itself".into());
            }
            reserved.insert(target.to_string_lossy().to_ascii_lowercase());
            roots.push((path.clone(), target));
        }
    }
    let folder_roots: Vec<_> = paths
        .iter()
        .filter(|(_, path)| path.is_dir())
        .map(|(_, path)| path.clone())
        .collect();
    let deleted = if request.action == BulkAction::Delete {
        selection_entries(&request.keys, &previous)
            .iter()
            .map(|entry| entry.key.clone())
            .collect()
    } else {
        Vec::new()
    };
    let done = if request.action == BulkAction::Delete {
        delete_files(&root, &paths, &names, game, previous, index, &context, tx)?
    } else if roots.is_empty() {
        return Err("All selected items are already at the destination; no files changed".into());
    } else {
        folder_merge_ops::run_many(
            root.clone(),
            roots.clone(),
            moving,
            if moving {
                "Moving Items"
            } else {
                "Copying Items"
            }
            .into(),
            names,
            game,
            previous,
            index,
            context,
            tx,
            true,
        )?
    };
    let completed = if request.action == BulkAction::Copy {
        Vec::new()
    } else {
        paths
            .iter()
            .filter(|(_, path)| !path.exists())
            .map(|(key, _)| key.clone())
            .collect()
    };

    Ok(BulkOutcome {
        done,
        pane: request.pane,
        renamed: false,
        completed,
        deleted,
        deleted_folders: if request.action == BulkAction::Delete {
            folder_roots
                .iter()
                .map(|path| path.strip_prefix(&root).unwrap().to_owned())
                .collect()
        } else {
            Vec::new()
        },
    })
}

/// Loose tags by file path, for looking many up at once.
fn entries_by_path(entries: &[TagEntry]) -> HashMap<&Path, &TagEntry> {
    entries
        .iter()
        .filter_map(|entry| match &entry.location {
            TagEntryLocation::LooseFile(path) => Some((path.as_path(), entry)),
            _ => None,
        })
        .collect()
}

/// The selected items of a bulk delete or move as review rows: folders with a folder
/// icon, tags with their group's icon, other files with the generic icon.
pub(in crate::app) fn selection_rows(
    root: &Path,
    roots: &[PathBuf],
    entries: &[TagEntry],
) -> Vec<(String, crate::app::browser::bulk_rename::RenameIcon)> {
    use crate::app::browser::bulk_rename::RenameIcon;
    let relative =
        |path: &Path| tag_path::relative_tag_path(path.strip_prefix(root).unwrap_or(path));
    let by_path = entries_by_path(entries);
    let mut selected: Vec<_> = roots
        .iter()
        .map(|path| match by_path.get(path.as_path()) {
            _ if path.is_dir() => (relative(path), RenameIcon::Folder),
            Some(entry) => (
                entry.display_path.clone(),
                RenameIcon::Tag(Some(entry.group_tag)),
            ),
            None => (relative(path), RenameIcon::Tag(None)),
        })
        .collect();
    selected.sort_by_cached_key(|(path, _)| crate::core::source::natural_key(path));
    selected
}

/// The bulk delete review: each selected item with its icon, what the
/// selection holds, and every tag outside it that references a deleted tag.
#[allow(clippy::too_many_arguments)]
fn delete_review(
    root: &Path,
    roots: &[PathBuf],
    snapshot: &std::collections::BTreeMap<PathBuf, disk::Fingerprint>,
    entries: &[TagEntry],
    deleting: &HashSet<String>,
    index: &ReverseDependencyIndex,
    names: &TagNameIndex,
    game: Option<GameId>,
    cancelled: &dyn Fn() -> bool,
) -> review::ReviewKind {
    let by_path = entries_by_path(entries);
    let entry_at = |path: &Path| by_path.get(path).copied();
    let selected = selection_rows(root, roots, entries);
    let mut referrer_keys = HashSet::new();
    let (mut tags, mut files) = (0, 0);
    for (path, info) in snapshot {
        if info.directory {
            continue;
        }
        let Some(entry) = entry_at(path) else {
            files += 1;
            continue;
        };
        tags += 1;
        if let Some(target) = dependency_entry_reference_path(entry, names) {
            referrer_keys.extend(
                index
                    .dependents_for(entry.group_tag, &target)
                    .iter()
                    .filter(|key| !deleting.contains(*key)),
            );
        }
    }
    let mut referrers: Vec<_> = entries
        .iter()
        .filter(|entry| referrer_keys.contains(&entry.key))
        .map(|entry| (entry.display_path.clone(), entry.group_tag))
        .collect();
    referrers.sort_by_cached_key(|(path, _)| crate::core::source::natural_key(path));
    let changes = crate::core::script_paths::ScriptChanges::deleting(
        entries
            .iter()
            .filter(|entry| deleting.contains(&entry.key))
            .map(|entry| entry.display_path.as_str()),
    );
    let report = crate::app::tag_ops::script_fixes::check(root, game, entries, &changes, cancelled);
    review::ReviewKind::Delete {
        selected,
        referrers,
        scripts: super::bulk_rename::ScriptRows::new(game, changes.kind(), Some(&report)),
        game,
        tags,
        files,
        folders: snapshot.values().filter(|info| info.directory).count(),
    }
}

fn delete_files(
    root: &Path,
    paths: &[(String, PathBuf)],
    names: &TagNameIndex,
    game: Option<GameId>,
    mut previous: Vec<TagEntry>,
    index: Option<ReverseDependencyIndex>,
    context: &folder_merge_ops::MergeContext,
    tx: &Sender<WorkerMessage>,
) -> Result<FolderRefactorFinished, String> {
    use crate::app::tag_ops::refactor;
    let source = TagSource::LooseFolder {
        root: root.to_owned(),
        game,
        definitions_root: locate_definitions_root(),
    };
    let cancelled = || context.control.cancelled();
    let roots: Vec<_> = paths.iter().map(|(_, path)| path.clone()).collect();
    let mut previous_index = index;
    // Reviews rebuilt in a row because files changed meanwhile, and why the
    // last one was.
    let mut rebuilds = 0;
    let mut rebuilt: Option<String> = None;
    loop {
        let (entries, mut index, mut changes) = refactor::prepare_tag_refactor_index_cancellable(
            root,
            &source,
            game,
            names,
            &previous,
            previous_index.take(),
            review::DELETE_REVIEW_TITLE,
            tx,
            &cancelled,
        )?;
        if entries.iter().any(|entry| !index.contains_tag(&entry.key)) {
            return Err(
                "Cannot verify all reference records; rebuild the reference index before deleting"
                    .into(),
            );
        }
        let snapshot = disk::snapshot(&roots, &cancelled)?;
        let metadata = entries
            .iter()
            .filter_map(|entry| {
                if let TagEntryLocation::LooseFile(path) = &entry.location {
                    Some(disk::metadata_fingerprint(path).map(|info| (path.clone(), info)))
                } else {
                    None
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let deleting:HashSet<_>=entries.iter().filter(|entry|matches!(&entry.location,TagEntryLocation::LooseFile(path) if snapshot.contains_key(path))).map(|entry|entry.key.clone()).collect();
        let review = delete_review(
            root, &roots, &snapshot, &entries, &deleting, &index, names, game, &cancelled,
        );
        let (tags, other_files) = match &review {
            review::ReviewKind::Delete { tags, files, .. } => (*tags, *files),
            _ => (0, 0),
        };
        review::ask_again(tx, &context.control, review, rebuilt.take())?;
        let current_entries =
            scan_folder_subtree_entries(root, Path::new(""), names).map_err(|e| e.to_string())?;
        let shown = |path: &Path| {
            path.strip_prefix(root)
                .map(tag_path::relative_tag_path)
                .unwrap_or_else(|_| path.display().to_string())
        };
        let mut reasons = Vec::new();
        // Compared as files: letter case and separators aside.
        let now: HashSet<_> = current_entries
            .iter()
            .map(|entry| crate::core::tag_key::key_identity(&entry.key))
            .collect();
        let before: HashSet<_> = entries
            .iter()
            .map(|entry| crate::core::tag_key::key_identity(&entry.key))
            .collect();
        if now != before {
            let new: Vec<_> = now.difference(&before).collect();
            let gone: Vec<_> = before.difference(&now).collect();
            let example = new
                .first()
                .or(gone.first())
                .map(|key| key.strip_prefix("file:").unwrap_or(key))
                .map(|path| shown(Path::new(path)))
                .unwrap_or_default();
            reasons.push(format!(
                "the kit's tags changed ({} new, {} gone, such as {example})",
                new.len(),
                gone.len()
            ));
        }
        let mut edited = Vec::new();
        for (path, info) in metadata {
            if cancelled() {
                return Err("Folder operation cancelled; no files changed".into());
            }
            if !disk::metadata_fingerprint(&path).is_ok_and(|current| current == info) {
                index.clear_tag(&crate::core::source::file_entry_key(&path));
                edited.push(path);
            }
        }
        if let Some(first) = edited.first() {
            reasons.push(format!(
                "{} tag(s) were edited, such as {}",
                edited.len(),
                shown(first)
            ));
        }
        if snapshot != disk::snapshot(&roots, &cancelled)? {
            reasons.push("the selected items changed".to_owned());
        }
        if !reasons.is_empty() {
            let reason = reasons.join("; ");
            rebuilds += 1;
            if rebuilds >= 3 {
                return Err(format!(
                    "Files kept changing while the review was open ({reason}). Nothing was deleted; try again, or reload the kit if it repeats."
                ));
            }
            let _ = tx.send(WorkerMessage::TerminalLine(format!(
                "Rebuilding the review: {reason}"
            )));
            rebuilt = Some(format!(
                "Files changed while this was open, so it was checked again: {reason}."
            ));
            previous_index = Some(index);
            // From what is on disk now, so a stale remembered tag can't make
            // every rebuild disagree again.
            previous = current_entries;
            continue;
        }
        context.control.begin_apply()?;
        let (base, origin) = context.recovery.as_ref().map_err(Clone::clone)?;
        recovery::prepare_custom_location(&crate::core::storage::data_path("deleted-tags"), base)?;
        let relative = roots
            .iter()
            .map(|path| path.strip_prefix(root).map(Path::to_path_buf))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let files = recovery::prepare_batch_paths(
            base,
            origin,
            &relative,
            "delete selected items",
            recovery::now_secs(),
        )?;
        for (path, info) in &snapshot {
            if info.directory {
                std::fs::create_dir_all(
                    files.join(path.strip_prefix(root).map_err(|e| e.to_string())?),
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let _ = tx.send(WorkerMessage::FolderRefactorProgress(
            crate::app::shell::FolderRefactorProgress {
                label: "Deleting Items".into(),
                phase: "Preserving and deleting files (Cancel unavailable)".into(),
                progress: None,
            },
        ));
        let mut transaction = disk::Transaction::for_kit(root)?;
        let result = (|| {
            transaction.delete_items(&roots, |path| {
                let target = files.join(path.strip_prefix(root).map_err(|e| e.to_string())?);
                std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
                disk::copy_backup(path, &target)
            })?;
            crate::core::source::build_folder_directory_tree(root).map_err(|e| e.to_string())
        })();
        let tree = match result {
            Ok(tree) => tree,
            Err(error) => return Err(transaction.rollback(error)),
        };
        transaction.commit();
        for key in &deleting {
            index.clear_tag(key);
            changes.removed.push(key.clone());
        }
        let all_entries = entries
            .into_iter()
            .filter(|entry| !deleting.contains(&entry.key))
            .collect::<Vec<_>>();
        refactor::persist_refactor_indexes(root, game, &all_entries, Some(&index), &changes, tx);
        return Ok(FolderRefactorFinished {
            status: format!("Deleted {tags} tags and {other_files} other files"),
            lines: vec![format!("Deleted items preserved at {}", files.display())],
            tree,
            all_entries,
            reverse_dependencies: Some(index),
            old_to_new_keys: HashMap::new(),
            moved: false,
            moved_folder: None,
            moved_folders: Vec::new(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::*;
    fn context(kit: &LooseKit) -> folder_merge_ops::MergeContext {
        folder_merge_ops::MergeContext {
            recovery: Ok((
                kit.base.join("recovery"),
                recovery::RecoveryOrigin {
                    source: recovery::RecoverySource::TagsFolder,
                    tags_root: kit.root.clone(),
                    game: Some(kit.game.into()),
                    kit_id: None,
                    kit_name: None,
                },
            )),
            control: Arc::new(review::MergeControl::default()),
        }
    }
    fn run(
        kit: &LooseKit,
        paths: Vec<(String, PathBuf)>,
        action: BulkAction,
        answer: bool,
    ) -> Result<BulkOutcome, String> {
        run_to(kit, paths, action, answer, kit.path("dest"))
    }
    fn run_to(
        kit: &LooseKit,
        paths: Vec<(String, PathBuf)>,
        action: BulkAction,
        answer: bool,
        destination: PathBuf,
    ) -> Result<BulkOutcome, String> {
        let (tx, rx) = mpsc::channel();
        let responder = std::thread::spawn(move || {
            while let Ok(message) = rx.recv() {
                if let WorkerMessage::FolderMergeReview(request) = message {
                    let choice = matches!(&request.kind, review::ReviewKind::Conflict { .. })
                        .then_some(disk::Choice::KeepBoth);
                    let _ = request.answer.send(answer.then_some(review::MergeAnswer {
                        choice,
                        apply_remaining: true,
                    }));
                }
            }
        });
        let req = BulkRequest {
            destination: None,
            stamp: KitStamp {
                kit: KitId(0),
                generation: 0,
            },
            pane: "test".into(),
            keys: paths.iter().map(|(key, _)| key.clone()).collect(),
            action,
        };
        let result = run_files(
            kit.root.clone(),
            paths,
            (action != BulkAction::Delete).then_some(destination),
            req,
            kit.names(),
            GameId::from_id(kit.game),
            kit.entries(),
            Some(kit.index()),
            context(kit),
            &tx,
        );
        drop(tx);
        responder.join().unwrap();
        result
    }
    fn mixed_kit() -> LooseKit {
        let kit = LooseKit::new("bulk-mixed-files", "halo3_mcc");
        let group = group_tag(kit.game, "render_model");
        kit.write_mcc("from/a", "render_model", |_| {});
        kit.write_mcc("from/group/b", "render_model", |_| {});
        kit.write_mcc("outside/uses_a", "model", |tag| {
            set_reference(tag, "render model", group, "from\\a")
        });
        kit.write_mcc("outside/uses_b", "model", |tag| {
            set_reference(tag, "render model", group, "from\\group\\b")
        });
        std::fs::create_dir_all(kit.path("dest")).unwrap();
        std::fs::create_dir_all(kit.path("from/group/empty")).unwrap();
        std::fs::write(kit.path("from/group/note.txt"), b"note").unwrap();
        kit
    }
    #[test]
    fn a_mixed_batch_moves_once_and_rewrites_cross_selection_references() {
        let kit = mixed_kit();
        let paths = vec![
            (
                kit.key("from/a.render_model"),
                kit.path("from/a.render_model"),
            ),
            (
                folder_pane_key(Path::new("from/group")),
                kit.path("from/group"),
            ),
        ];
        let outcome = run(&kit, paths, BulkAction::Move, true).unwrap();
        assert_eq!(outcome.completed.len(), 2);
        assert_eq!(
            outcome.done.moved_folders,
            vec![(PathBuf::from("from/group"), PathBuf::from("dest/group"))]
        );
        assert!(kit.path("dest/a.render_model").exists());
        assert!(kit.path("dest/group/b.render_model").exists());
        assert!(kit.path("dest/group/empty").is_dir());
        assert_eq!(
            std::fs::read(kit.path("dest/group/note.txt")).unwrap(),
            b"note"
        );
        assert_eq!(
            reference_of(
                &TagFile::read(kit.path("outside/uses_a.model")).unwrap(),
                "render model"
            )
            .as_deref(),
            Some("dest\\a")
        );
        assert_eq!(
            reference_of(
                &TagFile::read(kit.path("outside/uses_b.model")).unwrap(),
                "render model"
            )
            .as_deref(),
            Some("dest\\group\\b")
        );
    }
    #[test]
    fn copying_keeps_originals_and_external_references_unchanged() {
        let kit = mixed_kit();
        let outcome = run(
            &kit,
            vec![
                (
                    kit.key("from/a.render_model"),
                    kit.path("from/a.render_model"),
                ),
                (
                    folder_pane_key(Path::new("from/group")),
                    kit.path("from/group"),
                ),
            ],
            BulkAction::Copy,
            true,
        )
        .unwrap();
        assert!(outcome.completed.is_empty());
        assert!(kit.path("from/a.render_model").exists());
        assert!(kit.path("from/group/b.render_model").exists());
        assert_eq!(
            reference_of(
                &TagFile::read(kit.path("outside/uses_a.model")).unwrap(),
                "render model"
            )
            .as_deref(),
            Some("from\\a")
        );
    }
    #[test]
    fn incoming_name_collisions_stop_without_touching_any_source() {
        let kit = mixed_kit();
        kit.write_mcc("other/a", "render_model", |_| {});
        let error = run(
            &kit,
            vec![
                (
                    kit.key("from/a.render_model"),
                    kit.path("from/a.render_model"),
                ),
                (
                    kit.key("other/a.render_model"),
                    kit.path("other/a.render_model"),
                ),
            ],
            BulkAction::Move,
            true,
        )
        .err()
        .unwrap();
        assert!(error.contains("same destination name"));
        assert!(kit.path("from/a.render_model").exists());
        assert!(kit.path("other/a.render_model").exists());
        assert!(!kit.path("dest/a.render_model").exists());
    }
    #[test]
    fn deleting_duplicate_leaf_names_and_empty_folders_uses_one_recoverable_tree() {
        let kit = LooseKit::new("bulk-delete-shared-names", "halo3_mcc");
        kit.write_mcc("a/shared", "model", |_| {});
        kit.write_mcc("b/shared", "model", |_| {});
        std::fs::create_dir_all(kit.path("empty/nested")).unwrap();
        let paths = vec![
            (kit.key("a/shared.model"), kit.path("a/shared.model")),
            (kit.key("b/shared.model"), kit.path("b/shared.model")),
            (folder_pane_key(Path::new("empty")), kit.path("empty")),
        ];
        let outcome = run(&kit, paths, BulkAction::Delete, true).unwrap();
        assert_eq!(outcome.deleted.len(), 2);
        assert_eq!(outcome.completed.len(), 3);
        let batches: Vec<_> = std::fs::read_dir(kit.base.join("recovery"))
            .unwrap()
            .map(|item| item.unwrap().path())
            .collect();
        assert_eq!(batches.len(), 1);
        assert!(batches[0].join("files/a/shared.model").exists());
        assert!(batches[0].join("files/b/shared.model").exists());
        assert!(batches[0].join("files/empty/nested").is_dir());
        assert!(!kit.path("a/shared.model").exists());
        assert!(!kit.path("empty").exists());
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(batches[0].join("recovery-info.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["original_relative_paths"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
    #[test]
    fn the_delete_review_lists_the_selection_and_every_tag_left_with_missing_references() {
        let kit = mixed_kit();
        let paths = vec![
            (
                kit.key("from/a.render_model"),
                kit.path("from/a.render_model"),
            ),
            (
                folder_pane_key(Path::new("from/group")),
                kit.path("from/group"),
            ),
        ];
        let (tx, rx) = mpsc::channel();
        let responder = std::thread::spawn(move || {
            let mut seen = None;
            while let Ok(message) = rx.recv() {
                if let WorkerMessage::FolderMergeReview(request) = message {
                    if let review::ReviewKind::Delete {
                        selected,
                        referrers,
                        tags,
                        files,
                        folders,
                        ..
                    } = &request.kind
                    {
                        let selected: Vec<_> = selected
                            .iter()
                            .map(|(path, icon)| {
                                (
                                    path.replace('\\', "/"),
                                    matches!(
                                        icon,
                                        crate::app::browser::bulk_rename::RenameIcon::Folder
                                    ),
                                )
                            })
                            .collect();
                        let referrers: Vec<_> = referrers
                            .iter()
                            .map(|(path, _)| path.replace('\\', "/"))
                            .collect();
                        seen = Some((selected, referrers, *tags, *files, *folders));
                    }
                    let _ = request.answer.send(None);
                }
            }
            seen
        });
        let req = BulkRequest {
            destination: None,
            stamp: KitStamp {
                kit: KitId(0),
                generation: 0,
            },
            pane: "test".into(),
            keys: paths.iter().map(|(key, _)| key.clone()).collect(),
            action: BulkAction::Delete,
        };
        let result = run_files(
            kit.root.clone(),
            paths,
            None,
            req,
            kit.names(),
            GameId::from_id(kit.game),
            kit.entries(),
            Some(kit.index()),
            context(&kit),
            &tx,
        );
        drop(tx);
        assert!(result.is_err(), "cancelling the review deletes nothing");
        let (selected, referrers, tags, files, folders) =
            responder.join().unwrap().expect("a delete review");
        assert_eq!(
            selected,
            vec![
                ("from/a.render_model".to_owned(), false),
                ("from/group".to_owned(), true)
            ]
        );
        assert_eq!(
            referrers,
            vec![
                "outside/uses_a.model".to_owned(),
                "outside/uses_b.model".to_owned()
            ]
        );
        assert_eq!((tags, files, folders), (2, 1, 2));
        assert!(kit.path("from/a.render_model").exists());
    }
    #[test]
    fn the_move_review_lists_the_selection_its_destination_and_every_reference_to_fix() {
        let kit = mixed_kit();
        let paths = vec![
            (
                kit.key("from/a.render_model"),
                kit.path("from/a.render_model"),
            ),
            (
                folder_pane_key(Path::new("from/group")),
                kit.path("from/group"),
            ),
        ];
        let (tx, rx) = mpsc::channel();
        let responder = std::thread::spawn(move || {
            let mut seen = None;
            while let Ok(message) = rx.recv() {
                if let WorkerMessage::FolderMergeReview(request) = message {
                    if let review::ReviewKind::Move {
                        selected,
                        referrers,
                        destination,
                        conflicts,
                        ..
                    } = &request.kind
                    {
                        let selected: Vec<_> = selected
                            .iter()
                            .map(|(path, icon)| {
                                (
                                    path.replace('\\', "/"),
                                    matches!(
                                        icon,
                                        crate::app::browser::bulk_rename::RenameIcon::Folder
                                    ),
                                )
                            })
                            .collect();
                        let referrers: Vec<_> = referrers
                            .iter()
                            .map(|(path, _)| path.replace('\\', "/"))
                            .collect();
                        seen = Some((selected, referrers, destination.clone(), *conflicts));
                    }
                    let _ = request.answer.send(None);
                }
            }
            seen
        });
        let req = BulkRequest {
            destination: None,
            stamp: KitStamp {
                kit: KitId(0),
                generation: 0,
            },
            pane: "test".into(),
            keys: paths.iter().map(|(key, _)| key.clone()).collect(),
            action: BulkAction::Move,
        };
        let result = run_files(
            kit.root.clone(),
            paths,
            Some(kit.path("dest")),
            req,
            kit.names(),
            GameId::from_id(kit.game),
            kit.entries(),
            Some(kit.index()),
            context(&kit),
            &tx,
        );
        drop(tx);
        assert!(result.is_err(), "cancelling the review moves nothing");
        let (selected, referrers, destination, conflicts) =
            responder.join().unwrap().expect("a move review");
        assert_eq!(
            selected,
            vec![
                ("from/a.render_model".to_owned(), false),
                ("from/group".to_owned(), true)
            ]
        );
        assert_eq!(
            referrers,
            vec![
                "outside/uses_a.model".to_owned(),
                "outside/uses_b.model".to_owned()
            ]
        );
        assert_eq!((destination.as_str(), conflicts), ("dest", 0));
        assert!(kit.path("from/a.render_model").exists());
    }
    #[test]
    fn cancelling_a_batch_review_leaves_sources_and_recovery_untouched() {
        let kit = mixed_kit();
        let path = kit.path("from/a.render_model");
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            run(
                &kit,
                vec![(kit.key("from/a.render_model"), path.clone())],
                BulkAction::Delete,
                false
            )
            .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert!(!kit.base.join("recovery").exists());
    }
    #[test]
    fn copying_to_the_same_folder_keeps_both_and_moving_there_does_nothing() {
        let kit = mixed_kit();
        let path = kit.path("from/a.render_model");
        let paths = || vec![(kit.key("from/a.render_model"), path.clone())];
        let before = std::fs::read(&path).unwrap();
        let outcome = run_to(&kit, paths(), BulkAction::Copy, true, kit.path("from")).unwrap();
        assert!(outcome.completed.is_empty());
        assert_eq!(
            std::fs::read(kit.path("from/a (1).render_model")).unwrap(),
            before
        );
        let error = run_to(&kit, paths(), BulkAction::Move, true, kit.path("from"))
            .err()
            .unwrap();
        assert!(error.contains("already at the destination"));
        assert_eq!(std::fs::read(path).unwrap(), before);
        assert!(!kit.base.join("recovery").exists());
    }
    #[test]
    fn ten_new_tabs_require_confirmation_and_already_open_tabs_do_not_count() {
        let kit = LooseKit::new("bulk-open-warning", "halo3_mcc");
        for i in 0..10 {
            kit.write_mcc(&format!("item{i}"), "model", |_| {});
        }
        let mut app = app();
        kit.install(&mut app);
        let ctx = ctx();
        let keys: Vec<_> = (0..10)
            .map(|i| kit.key(&format!("item{i}.model")))
            .collect();
        app.begin_browser_bulk("test".into(), keys.clone(), BulkAction::Open, &ctx);
        let prompt = app
            .dialogs
            .get::<BulkPrompt>()
            .expect("ten new tabs must be reviewed");
        assert!(prompt.message.contains("10 tag tabs"));
        assert!(app.model.kits[0].open_tabs.is_empty());
        let request = prompt.request.clone();
        app.dialogs.close::<BulkPrompt>();
        app.run_browser_bulk(request, true, &ctx);
        assert_eq!(app.model.kits[0].open_tabs.len(), 10);
        app.begin_browser_bulk("test".into(), keys, BulkAction::Open, &ctx);
        assert!(app.dialogs.get::<BulkPrompt>().is_none());
        assert_eq!(app.model.kits[0].open_tabs.len(), 10);
    }

    /// Run a bulk Move or Delete of `folders` and `tags` in the script
    /// fixture, answering its review with `go`. Returns the result and the
    /// script rows the review showed.
    fn run_scripts(
        kit: &LooseKit,
        folders: &[&str],
        tags: &[&str],
        destination: Option<&str>,
        go: bool,
    ) -> (
        Result<FolderRefactorFinished, String>,
        Option<super::bulk_rename::ScriptRows>,
    ) {
        let mut paths: Vec<(String, PathBuf)> = folders
            .iter()
            .map(|folder| (folder_pane_key(Path::new(folder)), kit.path(folder)))
            .collect();
        paths.extend(tags.iter().map(|tag| (kit.key(tag), kit.path(tag))));
        let (tx, rx) = mpsc::channel();
        let responder = std::thread::spawn(move || {
            let mut seen = None;
            while let Ok(message) = rx.recv() {
                if let WorkerMessage::FolderMergeReview(request) = message {
                    match &request.kind {
                        review::ReviewKind::Move { scripts, .. }
                        | review::ReviewKind::Delete { scripts, .. } => {
                            seen = Some(scripts.clone());
                        }
                        _ => {}
                    }
                    let _ = request.answer.send(go.then_some(review::MergeAnswer {
                        choice: None,
                        apply_remaining: false,
                    }));
                }
            }
            seen
        });
        let req = BulkRequest {
            destination: None,
            stamp: KitStamp {
                kit: KitId(0),
                generation: 0,
            },
            pane: "test".into(),
            keys: paths.iter().map(|(key, _)| key.clone()).collect(),
            action: if destination.is_some() {
                BulkAction::Move
            } else {
                BulkAction::Delete
            },
        };
        let result = run_files(
            kit.root.clone(),
            paths,
            destination.map(|folder| kit.path(folder)),
            req,
            kit.names(),
            GameId::from_id(kit.game),
            kit.entries(),
            Some(kit.index()),
            context(kit),
            &tx,
        );
        drop(tx);
        (
            result.map(|outcome| outcome.done),
            responder.join().unwrap(),
        )
    }

    #[test]
    fn a_bulk_move_lists_the_script_and_fixes_it_where_the_scenario_now_is() {
        use crate::app::tag_ops::script_fixes::fixture::*;
        let kit = kit("bulk-move-scripts");
        fs::create_dir_all(kit.path("moved")).unwrap();
        // The scenario moves too: its saved copy is fixed where it lands.
        let (result, scripts) = run_scripts(
            &kit,
            &["objects/characters/grunt", "scenarios/solo/03a"],
            &[],
            Some("moved"),
            true,
        );
        let done = result.unwrap();
        let scripts = scripts.expect("a move review");
        assert_eq!(scripts.rows.len(), 1);
        assert_eq!(scripts.rows[0].count, "5 paths");
        assert_eq!(scripts.rows[0].problem, None);
        assert!(scripts.notes[0].starts_with("Scripts are updated"));
        let expected = SCRIPT.replace(OLD, r"moved\grunt\grunt");
        assert_eq!(data_text(&kit), expected);
        assert_eq!(saved_copy(&kit, "moved/03a/03a.scenario"), expected);
        assert!(
            done.status.contains("5 path(s) in 1 script(s)"),
            "{}",
            done.status
        );
    }

    #[test]
    fn a_drop_opens_the_move_review_for_its_folder_without_asking_where() {
        let kit = mixed_kit();
        fs::create_dir_all(kit.path("dest")).unwrap();
        let mut app = app();
        kit.install_indexed(&mut app);
        let key = kit.key("from/a.render_model");
        app.handle_browser_action(
            BrowserAction::DropItems {
                pane: "test".into(),
                keys: vec![key],
                copy: false,
                folder: PathBuf::from("dest"),
            },
            ctx(),
        );
        let merge = app.tag_ops.folder_merge.as_ref().expect("a review");
        let preview = merge
            .items_preview
            .as_ref()
            .expect("the review opens at once");
        assert_eq!(
            preview.action,
            review::ItemsAction::Move {
                destination: "dest".into()
            }
        );
        // Nothing moves until the review is answered; cancelling leaves it.
        merge.control.cancel();
        pump_until(&mut app, "the cancelled move to finish", |app| {
            app.tag_ops.folder_refactor.is_none()
        });
        assert!(kit.path("from/a.render_model").exists());
        assert!(!kit.path("dest/a.render_model").exists());
    }

    #[test]
    fn a_script_that_cannot_be_written_rolls_back_the_whole_bulk_move() {
        use crate::app::tag_ops::script_fixes::fixture::*;
        let kit = kit("bulk-move-scripts-read-only");
        fs::create_dir_all(kit.path("moved")).unwrap();
        let data = data_script(&kit);
        let mut permissions = fs::metadata(&data).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&data, permissions.clone()).unwrap();
        let (result, _) = run_scripts(
            &kit,
            &["objects/characters/grunt", "scenarios/solo/03a"],
            &[],
            Some("moved"),
            true,
        );
        let error = result.err().expect("the read-only script stops the move");
        assert!(error.contains("read-only"), "{error}");
        assert!(
            kit.path("objects/characters/grunt/grunt.biped").exists(),
            "the tags are back"
        );
        assert!(!kit.path("moved/grunt").exists());
        assert_eq!(saved_copy(&kit, SCENARIO), SCRIPT);
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&data, permissions).unwrap();
        assert_eq!(data_text(&kit), SCRIPT);
    }

    #[test]
    fn the_bulk_delete_review_lists_scripts_that_will_lose_their_tags() {
        use crate::app::tag_ops::script_fixes::fixture::*;
        let kit = kit("bulk-delete-scripts");
        let (result, scripts) = run_scripts(
            &kit,
            &[],
            &["objects/characters/grunt/grunt.biped"],
            None,
            false,
        );
        assert!(result.is_err(), "cancelled");
        let scripts = scripts.expect("a delete review");
        assert_eq!(scripts.rows[0].count, "2 paths");
        assert_eq!(scripts.rows[0].problem.as_deref(), Some("· 1 may break"));
        assert!(scripts.notes[0].starts_with("These scripts won't compile"));
        assert_eq!(data_text(&kit), SCRIPT, "deleting never rewrites scripts");
    }
}
