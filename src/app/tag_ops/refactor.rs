//! Renaming, moving and duplicating tags and loose folders, as jobs that
//! rewrite every reference to what moved.

use super::*;
use crate::app::documents::saving::lexical_normalize_path;

impl Baboon {
    /// Starts a filesystem refactoring transaction from a captured source snapshot.
    /// Progress and the final replacement tree are applied only through worker messages.
    pub(in crate::app) fn begin_refactor_loose_folder(
        &mut self,
        rel_path: PathBuf,
        label: String,
        move_folder: bool,
    ) {
        if self.tag_ops.folder_refactor.is_some() {
            self.model.status = "A folder move/copy is already running".to_owned();
            return;
        }
        if self.model.kits[self.model.active]
            .parsed_tags
            .values()
            .any(|doc| doc.dirty.is_set())
        {
            self.model.status = "Save or close dirty tags before moving/copying folders".to_owned();
            return;
        }
        let Some(root) = self.model.loaded_tags_root() else {
            self.model.status = "Folder move/copy requires a loaded tags folder".to_owned();
            return;
        };
        let title = if move_folder {
            format!("Move {label} To")
        } else {
            format!("Copy {label} To")
        };
        let Some(destination_parent) = rfd::FileDialog::new()
            .set_title(title)
            .set_directory(&root)
            .pick_folder()
        else {
            return;
        };
        let job_label = if move_folder {
            format!("Moving {label}")
        } else {
            format!("Copying {label}")
        };
        self.spawn_folder_refactor(
            root,
            rel_path,
            destination_parent,
            None,
            move_folder,
            job_label,
        );
    }

    /// Run the folder move/copy job on a worker, with the progress state that
    /// locks the app set before it starts.
    ///
    /// `new_name` replaces the folder's leaf at the destination; `None` keeps it.
    /// A rename is a move into the folder's own parent with a new leaf.
    pub(in crate::app) fn spawn_folder_refactor(
        &mut self,
        root: PathBuf,
        rel_path: PathBuf,
        destination_parent: PathBuf,
        new_name: Option<String>,
        move_folder: bool,
        job_label: String,
    ) {
        let names = self.model.names().clone();
        let existing_all_entries = self
            .model.source()
            .map(|source| source.all_entries.clone())
            .unwrap_or_default();
        let existing_reverse_dependencies = self
            .model.source()
            .and_then(|source| source.reverse_dependencies.clone());
        let game = self.model.source().and_then(|source| source.game.clone());
        // Routed back to the kit the refactor was started in, not
        // whichever one is focused when it lands.
        let stamp = self.model.kit_stamp();
        let tx = self.tx.clone();
        self.tag_ops.folder_refactor = Some(FolderRefactorUiState {
            label: job_label.clone(),
            phase: "Preparing".to_owned(),
            progress: None,
        });
        self.model.status = format!("{job_label}: Preparing");
        self.spawn_job(
            move || WorkerMessage::FolderRefactorFinished {
                stamp,
                result: run_folder_refactor_job(
                    root,
                    rel_path,
                    destination_parent,
                    new_name,
                    move_folder,
                    job_label,
                    names,
                    game,
                    existing_all_entries,
                    existing_reverse_dependencies,
                    &tx,
                ),
            },
            move |_| WorkerMessage::FolderRefactorFinished {
                stamp,
                result: Err("Folder move/copy worker crashed".to_owned()),
            },
        );
    }

    /// Tags that reference `entry` (its "parents"), via the reverse-dependency
    /// index. `None` when no index is available (non-folder source or not yet
    /// scanned).
    /// Open the rename/move dialog for a tag, pre-listing the tags that
    /// reference it (which will be rewritten on apply).
    pub(in crate::app) fn open_rename_tag(&mut self, key: &str) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        self.open_name_operation(key, TagNameOperation::Rename);
    }

    pub(in crate::app) fn open_container_duplicate(&mut self, key: &str) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        self.open_name_operation(key, TagNameOperation::SaveAsOverlay);
    }

    pub(in crate::app) fn open_duplicate_tag(&mut self, key: &str) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        self.open_name_operation(key, TagNameOperation::Duplicate);
    }

    pub(in crate::app) fn open_name_operation(&mut self, key: &str, operation: TagNameOperation) {
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            return;
        };
        let is_new_container = matches!(entry.location, TagEntryLocation::NewContainer { .. });
        let is_container =
            is_new_container || matches!(entry.location, TagEntryLocation::Container { .. });
        let supported = match operation {
            TagNameOperation::Duplicate => {
                // Two writers on one container's UTOC would race, and each
                // validates against a handle the other is invalidating.
                !self
                    .tag_ops.container_duplicate_running
                    .contains(&self.model.active_kit_id())
                    && !self
                        .tag_ops.container_delete_running
                        .contains(&self.model.active_kit_id())
                    && matches!(
                        entry.location,
                        TagEntryLocation::LooseFile(_) | TagEntryLocation::Container { .. }
                    )
            }
            TagNameOperation::Rename | TagNameOperation::SaveAsOverlay => {
                is_container || matches!(entry.location, TagEntryLocation::LooseFile(_))
            }
        };
        if !supported {
            self.model.status = match operation {
                TagNameOperation::Duplicate => {
                    "Only loose-file and Campaign Evolved container tags can be duplicated"
                        .to_owned()
                }
                TagNameOperation::Rename => {
                    "Only loose-folder or container tags can be renamed".to_owned()
                }
                TagNameOperation::SaveAsOverlay => {
                    "Save As is only available for writable tag sources".to_owned()
                }
            };
            return;
        }
        let display = entry.display_path.replace('\\', "/");
        let (stem, extension) = match display.rsplit_once('.') {
            Some((stem, ext)) => (stem.to_owned(), ext.to_owned()),
            None => (display.clone(), String::new()),
        };
        let leaf = stem.rsplit(['/', '\\']).next().unwrap_or(&stem).to_owned();
        let fixed_parent = stem
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_owned())
            .unwrap_or_default();
        let duplicate_parts = duplicate::duplicate_dialog_parts(&display);
        // A new tag edits its whole path (rename and move are the same in-memory
        // operation for it); everything else edits the leaf name only.
        // Any tag that lives in a pak edits its whole path, whether it got here
        // through Rename or through Move. Inside a container those are one
        // operation -- a move is a rename to a different parent -- and the two
        // dialogs are indistinguishable on screen, so splitting the behaviour
        // between them only produced "use Move to choose a folder" from a dialog
        // that looked exactly like the one being recommended.
        let whole_path_editable = is_container;
        // Resolved here rather than at apply time, so the text the user reads
        // and the branch that runs come from one answer. It depends on Baboon's
        // ledger, which cannot change while the dialog is open.
        let in_place_pak = if operation == TagNameOperation::Rename {
            let containers = self.model.mounted_containers().unwrap_or_default();
            rename_in_place::container_rename_eligibility(&entry, &containers, &self.tag_ops.created_tags)
                .ok()
                .and_then(|_| match &entry.location {
                    TagEntryLocation::Container { container, .. } => containers
                        .get(*container)
                        .map(|target| target.chunk_label.clone()),
                    _ => None,
                })
        } else {
            None
        };
        let name = match operation {
            TagNameOperation::Duplicate => duplicate_parts.prefill.clone(),
            TagNameOperation::Rename | TagNameOperation::SaveAsOverlay => {
                if whole_path_editable {
                    stem.clone()
                } else {
                    leaf
                }
            }
        };
        let (referrers, referrers_unavailable) = match self.model.references_to_entry(&entry) {
            Some(list) => (
                list.iter()
                    .map(|e| e.display_path.replace('\\', "/"))
                    .collect(),
                false,
            ),
            None => (Vec::new(), true),
        };
        self.dialogs.open(RenameTagState {
            kit: self.model.active_kit_id(),
            key: entry.key.clone(),
            old_display: display,
            extension: if operation == TagNameOperation::Duplicate {
                duplicate_parts.extension
            } else {
                extension
            },
            operation,
            new_path_input: name,
            fixed_parent: if operation == TagNameOperation::Duplicate {
                duplicate_parts.fixed_parent
            } else {
                fixed_parent
            },
            focus_input: matches!(operation, TagNameOperation::Duplicate),
            referrers,
            referrers_unavailable,
            is_container,
            is_new_container,
            whole_path_editable,
            in_place_pak,
        });
    }

    /// Apply the active name operation. Duplicate is routed to its own
    /// non-destructive copy/confirmation workflow; Rename and SaveAsOverlay
    /// retain their established paths below.
    /// `ctx` is only needed by the in-place container route, which starts a
    /// worker and has to ask for a repaint when it finishes.
    pub(in crate::app) fn begin_rename_tag(&mut self, ctx: &egui::Context) {
        // Everything below resolves against the active kit's tags root or
        // container set, so return to the workspace the dialog was opened for.
        // A closed workspace drops the rename rather than moving a file in
        // whichever game is focused now.
        let Some(kit) = self.dialogs.get::<RenameTagState>().map(|state| state.kit) else {
            return;
        };
        if !self.focus_navigation_kit(kit) {
            self.dialogs.close::<RenameTagState>();
            self.model.status = "The workspace this rename came from is closed".to_owned();
            return;
        }
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        let Some((
            key,
            old_display,
            new_name_raw,
            operation,
            is_container,
            is_new_container,
            whole_path_editable,
            in_place_pak,
        )) = self.dialogs.get::<RenameTagState>().map(|s| {
            (
                s.key.clone(),
                s.old_display.clone(),
                s.new_path_input.clone(),
                s.operation,
                s.is_container,
                s.is_new_container,
                s.whole_path_editable,
                s.in_place_pak.clone(),
            )
        })
        else {
            return;
        };
        if duplicate::name_operation_route(operation)
            == duplicate::NameOperationRoute::InPlaceDuplicateConfirmation
        {
            self.begin_duplicate_tag();
            return;
        }
        let new_name = new_name_raw.trim().to_owned();
        if new_name.is_empty() {
            self.model.status = "Enter a new tag name".to_owned();
            return;
        }
        // Where the whole path is editable a separator is the move half of the
        // operation rather than a mistake. That is every tag in a pak, not just
        // an unsaved one — the field says so, and this has to agree with it or
        // the dialog invites a path and then refuses it.
        if !whole_path_editable && new_name.contains(['/', '\\']) {
            self.model.status = "Enter a name only; use Move to choose a folder".to_owned();
            return;
        }
        if new_name.contains('.') {
            self.model.status = "Enter a name without an extension".to_owned();
            return;
        }
        let new_rel = if whole_path_editable {
            normalize_container_tag_rel(&new_name)
        } else {
            let parent = old_display
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("");
            if parent.is_empty() {
                new_name
            } else {
                format!("{parent}/{new_name}")
            }
        };

        // A brand-new tag has no container to override — it exists only as the
        // open document, so both rename and duplicate are in-memory edits.
        if is_new_container {
            self.dialogs.close::<RenameTagState>();
            match self.apply_new_container_rename(
                &key,
                &new_rel,
                matches!(operation, TagNameOperation::SaveAsOverlay),
            ) {
                Ok(message) => self.model.status = message,
                Err(error) => self.model.status = error,
            }
            return;
        }

        // A tag Baboon put in a pak is moved inside that pak, which is what
        // "rename" ought to have meant all along. Only Baboon's own: moving one
        // the game shipped would take it out from under everything referencing
        // it, and the pak format cannot forward those references — see
        // `container_rename_eligibility`. Everything else keeps the overlay
        // route, which copies rather than moves and so breaks nothing.
        if in_place_pak.is_some() {
            self.dialogs.close::<RenameTagState>();
            self.begin_container_rename_in_place(&key, &new_rel, ctx.clone());
            return;
        }

        // Container tags: write an override container (rename adds a redirect,
        // duplicate does not) instead of moving a loose file.
        if is_container {
            self.dialogs.close::<RenameTagState>();
            let redirect = matches!(operation, TagNameOperation::Rename);
            match self.export_container_override(&key, Some((new_rel, redirect))) {
                Ok(Some(path)) => {
                    let what = if redirect { "renamed tag" } else { "tag copy" };
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("mod");
                    self.model.status = format!(
                        "Exported {what} → {stem}.utoc/.ucas/.pak — copy all three into \
                         Meteorite/Content/Paks/ (base game unchanged)"
                    );
                }
                Ok(None) => {}
                Err(e) => self.model.status = format!("Export failed: {e}"),
            }
            return;
        }

        // Loose folder: move the file on disk + rewrite references.
        if self.tag_ops.folder_refactor.is_some() {
            self.model.status = "A move/rename is already running".to_owned();
            return;
        }
        if self.model.kits[self.model.active]
            .parsed_tags
            .values()
            .any(|doc| doc.dirty.is_set())
        {
            self.model.status = "Save or close dirty tags before renaming".to_owned();
            return;
        }
        let Some(root) = self.model.loaded_tags_root() else {
            self.model.status = "Rename requires a loaded tags folder".to_owned();
            return;
        };
        let Some(entry) = self.model.entry_for_key(&key).cloned() else {
            self.model.status = "Tag no longer exists".to_owned();
            return;
        };
        self.dialogs.close::<RenameTagState>();
        self.start_tag_rename_job(root, entry, new_rel, "Renaming tag");
    }

    /// Starts a filesystem refactoring transaction from a captured source snapshot.
    /// Progress and the final replacement tree are applied only through worker messages.
    pub(in crate::app) fn begin_move_tag(&mut self, key: &str) {
        // Nothing inside a pak has a folder to browse to, so the folder picker
        // below has nothing to show either way. Both container cases edit the
        // whole path in the rename dialog instead: for a brand-new tag that is
        // an in-memory edit, and for one already written it is a move inside
        // the pak that holds it — the same primitive as a rename, since a move
        // *is* a rename to a different parent.
        if matches!(
            self.model.entry_for_key(key).map(|entry| &entry.location),
            Some(TagEntryLocation::NewContainer { .. } | TagEntryLocation::Container { .. })
        ) {
            self.open_rename_tag(key);
            return;
        }
        if self.tag_ops.folder_refactor.is_some() {
            self.model.status = "A move/rename is already running".to_owned();
            return;
        }
        if self.model.kits[self.model.active]
            .parsed_tags
            .values()
            .any(|doc| doc.dirty.is_set())
        {
            self.model.status = "Save or close dirty tags before moving".to_owned();
            return;
        }
        let Some(root) = self.model.loaded_tags_root() else {
            self.model.status = "Move requires a loaded tags folder".to_owned();
            return;
        };
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            self.model.status = "Tag no longer exists".to_owned();
            return;
        };
        if !matches!(entry.location, TagEntryLocation::LooseFile(_)) {
            self.model.status = "Only loose-folder tags can be moved".to_owned();
            return;
        }
        let Some(destination_parent) = rfd::FileDialog::new()
            .set_title("Move Tag To")
            .set_directory(&root)
            .pick_folder()
        else {
            return;
        };
        let root = lexical_normalize_path(&root);
        let destination_parent = lexical_normalize_path(&destination_parent);
        if !destination_parent.starts_with(&root) {
            self.model.status = "Choose a destination inside the loaded tags folder".to_owned();
            return;
        }
        let folder_rel = destination_parent
            .strip_prefix(&root)
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
        let stem = entry
            .display_path
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .and_then(|name| name.rsplit_once('.').map(|(stem, _)| stem))
            .unwrap_or(&entry.display_path)
            .to_owned();
        let new_rel = if folder_rel.is_empty() {
            stem
        } else {
            format!("{folder_rel}/{stem}")
        };
        self.start_tag_rename_job(root, entry, new_rel, "Moving tag");
    }

    pub(in crate::app) fn start_tag_rename_job(
        &mut self,
        root: PathBuf,
        entry: TagEntry,
        new_rel: String,
        job_label: &str,
    ) {
        let names = self.model.names().clone();
        let game = self.model.source().and_then(|source| source.game.clone());
        let all_entries = self
            .model.source()
            .map(|source| source.all_entries.clone())
            .unwrap_or_default();
        let reverse_dependencies = self
            .model.source()
            .and_then(|source| source.reverse_dependencies.clone());
        // Routed back to the kit the refactor was started in, not
        // whichever one is focused when it lands.
        let stamp = self.model.kit_stamp();
        let tx = self.tx.clone();
        let job_label = job_label.to_owned();
        self.tag_ops.folder_refactor = Some(FolderRefactorUiState {
            label: job_label.clone(),
            phase: "Preparing".to_owned(),
            progress: None,
        });
        self.model.status = format!("{job_label}: Preparing");
        self.spawn_job(
            move || WorkerMessage::FolderRefactorFinished {
                stamp,
                result: run_tag_rename_job(
                    root,
                    entry,
                    new_rel,
                    job_label,
                    names,
                    game,
                    all_entries,
                    reverse_dependencies,
                    &tx,
                ),
            },
            move |_| WorkerMessage::FolderRefactorFinished {
                stamp,
                result: Err("Tag move worker crashed".to_owned()),
            },
        );
    }
}

#[derive(Default)]
pub(in crate::app) struct ReferenceRewriteResult {
    references_changed: usize,
    tags_changed: usize,
    changed_keys: Vec<String>,
    /// Tags that hold a reference to rewrite but could not be read, parsed or
    /// written, with why: `(display path, reason)`. They still point at the
    /// old path.
    failed: Vec<(String, String)>,
}

/// Terminal lines naming each tag a reference rewrite could not update.
pub(in crate::app) fn rewrite_failure_lines(failed: &[(String, String)]) -> Vec<String> {
    if failed.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "Warning: {} tag(s) could not be updated and may still reference the old path:",
        failed.len()
    )];
    lines.extend(
        failed
            .iter()
            .map(|(path, reason)| format!("Warning: not updated: {path} ({reason})")),
    );
    lines
}

#[allow(clippy::too_many_arguments)]
/// Rename/move a SINGLE tag and rewrite every reference to it, mirroring
/// [`run_folder_refactor_job`] for one file with an explicit new relative path
/// (no extension). Reuses the same reference-rewrite + key-remap machinery and
/// returns a [`FolderRefactorFinished`] so the existing finish handler applies
/// the in-memory update.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn run_tag_rename_job(
    root: PathBuf,
    entry: TagEntry,
    new_rel: String,
    job_label: String,
    names: TagNameIndex,
    game: Option<GameId>,
    all_entries_before: Vec<TagEntry>,
    existing_reverse_dependencies: Option<ReverseDependencyIndex>,
    tx: &Sender<WorkerMessage>,
) -> Result<FolderRefactorFinished, String> {
    let label = job_label;
    send_folder_refactor_progress(tx, &label, "Preparing", None);
    let root = lexical_normalize_path(&root);

    let TagEntryLocation::LooseFile(old_path) = &entry.location else {
        return Err("Only loose-folder tags can be renamed".to_owned());
    };
    let old_path = lexical_normalize_path(old_path);
    if !old_path.is_file() {
        return Err(format!("Source tag not found: {}", old_path.display()));
    }
    let extension = old_path
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| "Tag file has no extension".to_owned())?
        .to_owned();

    // Compute the destination absolute path from the (extension-less) new rel.
    let new_rel_norm = new_rel.replace('\\', "/");
    if new_rel_norm
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err("Destination path is not a valid relative path".to_owned());
    }
    let new_path = lexical_normalize_path(&root.join(format!("{new_rel_norm}.{extension}")));
    if !new_path.starts_with(&root) {
        return Err("Destination escapes the tags folder".to_owned());
    }
    if new_path == old_path {
        return Err("New path is the same as the current one".to_owned());
    }
    // Tag paths ignore case, so a change of case alone renames nothing: no
    // reference would change. Refused by name, as a folder rename is, rather
    // than left to the file system — a case-insensitive one refuses it only
    // because the destination "exists" (it is the tag itself), and a
    // case-sensitive one would go ahead and rewrite every referrer.
    if new_path
        .to_string_lossy()
        .eq_ignore_ascii_case(&old_path.to_string_lossy())
    {
        return Err(
            "Tag paths ignore case, so changing only the case would not change any reference"
                .to_owned(),
        );
    }
    if new_path.exists() {
        return Err(format!(
            "A tag already exists at the destination: {}",
            new_path.display()
        ));
    }

    // The one rewrite: old reference path → new reference path (same group).
    let old_ref = reference_path_from_abs_file(&root, &old_path, entry.group_tag, &names)
        .ok_or_else(|| "Could not resolve the tag's reference path".to_owned())?;
    let new_ref = reference_path_from_abs_file(&root, &new_path, entry.group_tag, &names)
        .ok_or_else(|| "Could not resolve the destination reference path".to_owned())?;
    let mut rewrites = HashMap::new();
    rewrites.insert((entry.group_tag, old_ref.to_ascii_lowercase()), new_ref);

    // Ensure a reverse-dependency index so we only rewrite actual referrers.
    let mut reverse_dependencies = existing_reverse_dependencies.or_else(|| {
        game.and_then(|game| crate::core::source::load_reverse_dependency_index(game.as_str(), &root))
    });
    if let Some(index) = reverse_dependencies.as_ref()
        && index.len() != all_entries_before.len()
    {
        reverse_dependencies = None; // stale → rebuild below
    }
    let dependency_source = TagSource::LooseFolder {
        root: root.clone(),
        game: game.clone(),
        definitions_root: locate_definitions_root(),
    };
    if reverse_dependencies.is_none() {
        reverse_dependencies = Some(build_reverse_dependency_index(
            &root,
            &dependency_source,
            &all_entries_before,
            &label,
            tx,
        ));
    }
    let dependency_schema_path = game
        .map(|game| {
            locate_definitions_root()
                .join(game.as_str())
                .join("tag_dependency_list.json")
        })
        .filter(|path| path.is_file());

    // The moved entry, post-rename.
    let new_display = new_path
        .strip_prefix(&root)
        .unwrap_or(&new_path)
        .to_string_lossy()
        .replace('\\', "/");
    let new_entry = TagEntry {
        key: file_entry_key(&new_path),
        display_path: new_display,
        group_tag: entry.group_tag,
        group_name: entry.group_name.clone(),
        location: TagEntryLocation::LooseFile(new_path.clone()),
    };
    let old_entries = vec![entry.clone()];
    let new_entries = vec![new_entry.clone()];

    // Move the file on disk.
    if let Some(parent) = new_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    fs::rename(&old_path, &new_path).map_err(|error| {
        format!(
            "Could not move {} to {}: {error}",
            old_path.display(),
            new_path.display()
        )
    })?;

    // Rewrite references in the affected (referring) tags.
    let rewrite_entries = affected_move_rewrite_entries(
        &all_entries_before,
        &old_entries,
        &new_entries,
        &rewrites,
        reverse_dependencies.as_ref(),
    );
    send_folder_refactor_progress(
        tx,
        &label,
        &format!("Rewriting {} affected tag(s)", rewrite_entries.len()),
        None,
    );
    let rewrite_result = rewrite_references_in_entries(
        &dependency_source,
        &rewrite_entries,
        &rewrites,
        &label,
        tx,
        dependency_schema_path.as_deref(),
    )?;
    let references_changed = rewrite_result.references_changed;
    let tags_changed = rewrite_result.tags_changed;

    // Rebuild browser tree + entry set + key map.
    send_folder_refactor_progress(tx, &label, "Refreshing browser", None);
    let tree = crate::core::source::build_folder_directory_tree(&root).map_err(|e| e.to_string())?;
    let all_entries =
        merge_refactored_entries(all_entries_before, &old_entries, &new_entries, true);
    let mut old_to_new_keys = HashMap::new();
    old_to_new_keys.insert(entry.key.clone(), new_entry.key.clone());
    if let Some(index) = reverse_dependencies.as_mut() {
        refresh_reverse_dependency_index_after_refactor(
            index,
            &dependency_source,
            true,
            &old_entries,
            &new_entries,
            &rewrite_result.changed_keys,
            &all_entries,
        );
    }

    let verb = if label.starts_with("Moving") {
        "Moved"
    } else {
        "Renamed"
    };
    let mut status =
        format!("{verb} tag, updated {references_changed} reference(s) in {tags_changed} tag(s)");
    if !rewrite_result.failed.is_empty() {
        status.push_str(&format!(
            "; {} tag(s) could NOT be updated and may still reference the old path (see terminal)",
            rewrite_result.failed.len()
        ));
    }
    let mut lines = vec![
        format!(
            "{verb}: {} -> {}",
            entry.display_path, new_entry.display_path
        ),
        format!("Updated {references_changed} reference(s) in {tags_changed} tag(s)"),
    ];
    lines.extend(rewrite_failure_lines(&rewrite_result.failed));
    Ok(FolderRefactorFinished {
        status,
        lines,
        tree,
        all_entries,
        reverse_dependencies,
        old_to_new_keys,
        moved: true,
        moved_folder: None,
    })
}

pub(in crate::app) fn run_folder_refactor_job(
    root: PathBuf,
    rel_path: PathBuf,
    destination_parent: PathBuf,
    new_name: Option<String>,
    move_folder: bool,
    label: String,
    names: TagNameIndex,
    game: Option<GameId>,
    existing_all_entries: Vec<TagEntry>,
    existing_reverse_dependencies: Option<ReverseDependencyIndex>,
    tx: &Sender<WorkerMessage>,
) -> Result<FolderRefactorFinished, String> {
    send_folder_refactor_progress(tx, &label, "Preparing", None);
    let root = lexical_normalize_path(&root);
    let source_rel = validate_relative_folder_path(&rel_path)?;
    let source = lexical_normalize_path(&root.join(&source_rel));
    if !source.is_dir() {
        return Err(format!("Folder not found: {}", source.display()));
    }
    let destination_parent = lexical_normalize_path(&destination_parent);
    if !destination_parent.starts_with(&root) {
        return Err("Choose a destination inside the loaded tags folder".to_owned());
    }
    let folder_name = source
        .file_name()
        .ok_or_else(|| "Cannot move/copy the tags root itself".to_owned())?;
    let folder_name = match new_name.as_deref() {
        Some(name) => std::ffi::OsStr::new(name),
        None => folder_name,
    };
    let destination = lexical_normalize_path(&destination_parent.join(folder_name));
    if destination == source {
        return Err("Source and destination are the same folder".to_owned());
    }
    if destination.starts_with(&source) {
        return Err("Cannot move/copy a folder into itself".to_owned());
    }
    // A case-insensitive file system answers `exists` for a sibling that
    // differs only in case, which is also a conflict as far as tag paths go:
    // they ignore case, so both folders would claim the same references.
    if destination.exists() || sibling_differing_in_case(&destination).is_some() {
        return Err(format!(
            "Destination already exists: {}",
            destination.display()
        ));
    }

    send_folder_refactor_progress(tx, &label, "Scanning selected folder", None);
    let old_entries =
        scan_folder_subtree_entries(&root, &source_rel, &names).map_err(|e| e.to_string())?;
    if old_entries.is_empty() {
        return Err("No tags found in that folder".to_owned());
    }
    let rewrites =
        build_folder_reference_rewrites(&root, &source, &destination, &old_entries, &names);
    let all_entries_before = if move_folder && existing_all_entries.is_empty() {
        send_folder_refactor_progress(tx, &label, "Building tag database", None);
        scan_folder_subtree_entries(&root, Path::new(""), &names).map_err(|e| e.to_string())?
    } else {
        existing_all_entries.clone()
    };
    let mut reverse_dependencies = existing_reverse_dependencies.or_else(|| {
        game.and_then(|game| crate::core::source::load_reverse_dependency_index(game.as_str(), &root))
    });
    if move_folder
        && let Some(index) = reverse_dependencies.as_ref()
        && index.len() != all_entries_before.len()
    {
        let _ = tx.send(WorkerMessage::TerminalLine(format!(
            "Dependency index is stale ({} indexed tag(s), {} current tag(s)); rebuilding",
            index.len(),
            all_entries_before.len()
        )));
        reverse_dependencies = None;
    }
    if move_folder && reverse_dependencies.is_none() {
        let dependency_source = TagSource::LooseFolder {
            root: root.clone(),
            game: game.clone(),
            definitions_root: locate_definitions_root(),
        };
        reverse_dependencies = Some(build_reverse_dependency_index(
            &root,
            &dependency_source,
            &all_entries_before,
            &label,
            tx,
        ));
    }
    let dependency_schema_path = game
        .map(|game| {
            locate_definitions_root()
                .join(game.as_str())
                .join("tag_dependency_list.json")
        })
        .filter(|path| path.is_file());
    let rewrite_source = TagSource::LooseFolder {
        root: root.clone(),
        game: game.clone(),
        definitions_root: locate_definitions_root(),
    };

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    if move_folder {
        send_folder_refactor_progress(tx, &label, "Moving files", Some(0.15));
        fs::rename(&source, &destination).map_err(|error| {
            format!(
                "Could not move {} to {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
    } else {
        copy_folder_recursive_progress(&source, &destination, &label, tx)?;
    }

    let new_entries = transform_folder_entries(&root, &source, &old_entries, &destination);
    let rewrite_result = if move_folder {
        let rewrite_entries = affected_move_rewrite_entries(
            &all_entries_before,
            &old_entries,
            &new_entries,
            &rewrites,
            reverse_dependencies.as_ref(),
        );
        send_folder_refactor_progress(
            tx,
            &label,
            &format!("Rewriting {} affected tag(s)", rewrite_entries.len()),
            None,
        );
        rewrite_references_in_entries(
            &rewrite_source,
            &rewrite_entries,
            &rewrites,
            &label,
            tx,
            dependency_schema_path.as_deref(),
        )?
    } else {
        send_folder_refactor_progress(tx, &label, "Rewriting copied references", None);
        rewrite_references_in_entries(
            &rewrite_source,
            &new_entries,
            &rewrites,
            &label,
            tx,
            dependency_schema_path.as_deref(),
        )?
    };
    let references_changed = rewrite_result.references_changed;
    let tags_changed = rewrite_result.tags_changed;
    let failed = rewrite_result.failed.clone();

    send_folder_refactor_progress(tx, &label, "Refreshing browser", None);
    let tree = crate::core::source::build_folder_directory_tree(&root).map_err(|e| e.to_string())?;
    let all_entries = if move_folder {
        merge_refactored_entries(all_entries_before, &old_entries, &new_entries, true)
    } else if existing_all_entries.is_empty() {
        Vec::new()
    } else {
        merge_refactored_entries(
            existing_all_entries,
            &old_entries,
            &new_entries,
            move_folder,
        )
    };
    let old_to_new_keys = if move_folder {
        moved_key_map(&root, &source, &old_entries, &destination)
    } else {
        HashMap::new()
    };
    if let Some(index) = reverse_dependencies.as_mut() {
        let dependency_source = TagSource::LooseFolder {
            root: root.clone(),
            game: game.clone(),
            definitions_root: locate_definitions_root(),
        };
        refresh_reverse_dependency_index_after_refactor(
            index,
            &dependency_source,
            move_folder,
            &old_entries,
            &new_entries,
            &rewrite_result.changed_keys,
            &all_entries,
        );
    }

    let action = match (move_folder, new_name.is_some()) {
        (true, true) => "Renamed",
        (true, false) => "Moved",
        (false, _) => "Copied",
    };
    let mut status = format!(
        "{action} {} tag(s), updated {} reference(s) in {} tag(s)",
        old_entries.len(),
        references_changed,
        tags_changed
    );
    if !failed.is_empty() {
        status.push_str(&format!(
            "; {} tag(s) could NOT be updated and may still reference the old path (see terminal)",
            failed.len()
        ));
    }
    let mut lines = vec![format!(
        "{action} folder: {} -> {}",
        source.strip_prefix(&root).unwrap_or(&source).display(),
        destination
            .strip_prefix(&root)
            .unwrap_or(&destination)
            .display()
    )];
    lines.push(format!(
        "Updated {references_changed} reference(s) in {tags_changed} tag(s)"
    ));
    lines.extend(rewrite_failure_lines(&failed));

    Ok(FolderRefactorFinished {
        status,
        lines,
        tree,
        all_entries,
        reverse_dependencies,
        old_to_new_keys,
        moved: move_folder,
        moved_folder: move_folder.then(|| {
            (
                source_rel.clone(),
                destination
                    .strip_prefix(&root)
                    .unwrap_or(&destination)
                    .to_path_buf(),
            )
        }),
    })
}

pub(in crate::app) fn send_folder_refactor_progress(
    tx: &Sender<WorkerMessage>,
    label: &str,
    phase: &str,
    progress: Option<f32>,
) {
    let _ = tx.send(WorkerMessage::FolderRefactorProgress(
        FolderRefactorProgress {
            label: label.to_owned(),
            phase: phase.to_owned(),
            progress,
        },
    ));
}

pub(in crate::app) fn validate_relative_folder_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("Choose a folder inside the loaded tags folder".to_owned());
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        )
    }) {
        return Err("Folder path cannot contain .. or a drive prefix".to_owned());
    }
    Ok(path.to_path_buf())
}

pub(in crate::app) fn copy_folder_recursive_progress(
    source: &Path,
    destination: &Path,
    label: &str,
    tx: &Sender<WorkerMessage>,
) -> Result<(), String> {
    let items = walkdir::WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let file_total = items
        .iter()
        .filter(|item| item.file_type().is_file())
        .count();
    let mut copied = 0usize;
    for item in items {
        let rel = item
            .path()
            .strip_prefix(source)
            .map_err(|error| error.to_string())?;
        let target = destination.join(rel);
        if item.file_type().is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| format!("Could not create {}: {error}", target.display()))?;
        } else if item.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
            }
            fs::copy(item.path(), &target).map_err(|error| {
                format!(
                    "Could not copy {} to {}: {error}",
                    item.path().display(),
                    target.display()
                )
            })?;
            copied += 1;
            if copied == 1 || copied % 25 == 0 || copied == file_total {
                let progress = if file_total == 0 {
                    None
                } else {
                    Some(copied as f32 / file_total as f32)
                };
                send_folder_refactor_progress(
                    tx,
                    label,
                    &format!("Copying files {copied}/{file_total}"),
                    progress,
                );
            }
        }
    }
    Ok(())
}

pub(in crate::app) fn build_folder_reference_rewrites(
    tags_root: &Path,
    source: &Path,
    destination: &Path,
    old_entries: &[TagEntry],
    names: &TagNameIndex,
) -> HashMap<(u32, String), String> {
    let mut rewrites = HashMap::new();
    for entry in old_entries {
        let TagEntryLocation::LooseFile(old_path) = &entry.location else {
            continue;
        };
        let Some(old_ref) =
            reference_path_from_abs_file(tags_root, old_path, entry.group_tag, names)
        else {
            continue;
        };
        let Ok(inner_rel) = old_path.strip_prefix(source) else {
            continue;
        };
        let new_path = destination.join(inner_rel);
        let Some(new_ref) =
            reference_path_from_abs_file(tags_root, &new_path, entry.group_tag, names)
        else {
            continue;
        };
        rewrites.insert((entry.group_tag, old_ref.to_ascii_lowercase()), new_ref);
    }
    rewrites
}

pub(in crate::app) fn transform_folder_entries(
    tags_root: &Path,
    source: &Path,
    old_entries: &[TagEntry],
    destination: &Path,
) -> Vec<TagEntry> {
    old_entries
        .iter()
        .filter_map(|entry| {
            let TagEntryLocation::LooseFile(old_path) = &entry.location else {
                return None;
            };
            let inner_rel = old_path.strip_prefix(source).ok()?;
            let new_path = destination.join(inner_rel);
            let display_path = new_path
                .strip_prefix(tags_root)
                .unwrap_or(&new_path)
                .to_string_lossy()
                .replace('\\', "/");
            Some(TagEntry {
                key: file_entry_key(&new_path),
                display_path,
                group_tag: entry.group_tag,
                group_name: entry.group_name.clone(),
                location: TagEntryLocation::LooseFile(new_path),
            })
        })
        .collect()
}

pub(in crate::app) fn merge_refactored_entries(
    mut all_entries: Vec<TagEntry>,
    old_entries: &[TagEntry],
    new_entries: &[TagEntry],
    moved: bool,
) -> Vec<TagEntry> {
    let old_keys = old_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<HashSet<_>>();
    if moved {
        all_entries.retain(|entry| !old_keys.contains(&entry.key));
    }
    let existing = all_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<HashSet<_>>();
    all_entries.extend(
        new_entries
            .iter()
            .filter(|entry| !existing.contains(&entry.key))
            .cloned(),
    );
    all_entries.sort_by(|a, b| a.display_path.cmp(&b.display_path));
    all_entries
}

pub(in crate::app) fn affected_move_rewrite_entries(
    all_entries: &[TagEntry],
    old_entries: &[TagEntry],
    new_entries: &[TagEntry],
    rewrites: &HashMap<(u32, String), String>,
    reverse_dependencies: Option<&ReverseDependencyIndex>,
) -> Vec<TagEntry> {
    let old_keys = old_entries
        .iter()
        .map(|entry| entry.key.as_str())
        .collect::<HashSet<_>>();
    let mut entries_by_key = all_entries
        .iter()
        .map(|entry| (entry.key.clone(), entry.clone()))
        .collect::<HashMap<_, _>>();
    for entry in new_entries {
        entries_by_key.insert(entry.key.clone(), entry.clone());
    }

    let mut affected = new_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<HashSet<_>>();
    if let Some(index) = reverse_dependencies {
        for ((group_tag, old_ref), _) in rewrites {
            for dependent_key in index.dependents_for(*group_tag, old_ref) {
                if !old_keys.contains(dependent_key.as_str()) {
                    affected.insert(dependent_key.clone());
                }
            }
        }
    } else {
        affected.extend(all_entries.iter().map(|entry| entry.key.clone()));
    }

    let mut entries = affected
        .into_iter()
        .filter_map(|key| entries_by_key.get(&key).cloned())
        .collect::<Vec<_>>();
    entries.sort_by_cached_key(|entry| crate::core::source::natural_key(&entry.display_path));
    entries
}

pub(in crate::app) fn rewrite_references_in_entries(
    source: &TagSource,
    entries: &[TagEntry],
    rewrites: &HashMap<(u32, String), String>,
    label: &str,
    tx: &Sender<WorkerMessage>,
    dependency_schema_path: Option<&Path>,
) -> Result<ReferenceRewriteResult, String> {
    let mut result = ReferenceRewriteResult::default();
    let needles = rewrite_reference_needles(rewrites);
    if needles.is_empty() {
        return Ok(result);
    }
    let total = entries.len();
    for (index, entry) in entries.iter().enumerate() {
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            continue;
        };
        if index == 0 || (index + 1) % 25 == 0 || index + 1 == total {
            let progress = if total == 0 {
                None
            } else {
                Some((index + 1) as f32 / total as f32)
            };
            send_folder_refactor_progress(
                tx,
                label,
                &format!("Rewriting affected references {}/{}", index + 1, total),
                progress,
            );
        }
        // A tag that cannot be read, parsed or written is recorded and the
        // rest are still rewritten. The files have already moved by now, so
        // stopping here would leave every later referrer broken too, and the
        // error would name only the first.
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                result.failed.push((
                    entry.display_path.clone(),
                    format!("could not read: {error}"),
                ));
                continue;
            }
        };
        if !bytes_contain_any_ascii_case_insensitive(&bytes, &needles) {
            continue;
        }
        send_folder_refactor_progress(
            tx,
            label,
            &format!(
                "Rewriting {}",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("tag")
            ),
            None,
        );
        let mut tag = match read_entry(source, entry) {
            Ok(tag) => tag,
            Err(error) => {
                result.failed.push((
                    entry.display_path.clone(),
                    format!("could not parse: {error}"),
                ));
                continue;
            }
        };
        let changed = rewrite_references_in_tag(&mut tag, rewrites);
        if changed == 0 {
            continue;
        }
        if tag.classic_engine().is_none()
            && let Some(schema_path) = dependency_schema_path
            && let Err(error) = tag.rebuild_dependency_list(schema_path)
        {
            let _ = tx.send(WorkerMessage::TerminalLine(format!(
                "Warning: could not rebuild dependency list for {}: {error}",
                entry.display_path
            )));
        }
        if let Err(error) = tag.write_atomic(path) {
            result.failed.push((
                entry.display_path.clone(),
                format!("could not write: {error}"),
            ));
            continue;
        }
        result.references_changed += changed;
        result.tags_changed += 1;
        result.changed_keys.push(entry.key.clone());
    }
    Ok(result)
}

pub(in crate::app) fn refresh_reverse_dependency_index_after_refactor(
    index: &mut ReverseDependencyIndex,
    source: &TagSource,
    moved: bool,
    old_entries: &[TagEntry],
    new_entries: &[TagEntry],
    changed_keys: &[String],
    all_entries: &[TagEntry],
) {
    if moved {
        for entry in old_entries {
            index.clear_tag(&entry.key);
        }
    }
    let entries_by_key = all_entries
        .iter()
        .map(|entry| (entry.key.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let mut refresh_keys = new_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<HashSet<_>>();
    refresh_keys.extend(changed_keys.iter().cloned());
    for key in refresh_keys {
        let Some(entry) = entries_by_key.get(key.as_str()) else {
            continue;
        };
        if let Ok(deps) = read_entry_dependencies(source, entry) {
            index.set_tag_dependencies(entry.key.clone(), deps);
        }
    }
}

pub(in crate::app) fn rewrite_reference_needles(rewrites: &HashMap<(u32, String), String>) -> Vec<Vec<u8>> {
    let mut seen = HashSet::new();
    rewrites
        .keys()
        .filter_map(|(_, old_ref)| {
            let lowered = old_ref.replace('/', "\\").to_ascii_lowercase().into_bytes();
            (!lowered.is_empty() && seen.insert(lowered.clone())).then_some(lowered)
        })
        .collect()
}

pub(in crate::app) fn bytes_contain_any_ascii_case_insensitive(bytes: &[u8], needles: &[Vec<u8>]) -> bool {
    if needles.is_empty() || bytes.is_empty() {
        return false;
    }
    let lowered = bytes.to_ascii_lowercase();
    needles
        .iter()
        .any(|needle| contains_subslice(&lowered, needle.as_slice()))
}

pub(in crate::app) fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

pub(in crate::app) fn rewrite_references_in_tag(
    tag: &mut TagFile,
    rewrites: &HashMap<(u32, String), String>,
) -> usize {
    let mut refs = Vec::new();
    collect_tag_references(tag.root(), "", &mut refs);
    let mut changed = 0usize;
    for reference in refs {
        let key = (reference.group_tag, reference.rel_path.to_ascii_lowercase());
        let Some(new_path) = rewrites.get(&key) else {
            continue;
        };
        if new_path.eq_ignore_ascii_case(&reference.rel_path) {
            continue;
        }
        let mut root = tag.root_mut();
        let Some(mut field) = root.field_path_mut(&reference.field_path) else {
            continue;
        };
        if field
            .set(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((reference.group_tag, new_path.clone())),
            }))
            .is_ok()
        {
            changed += 1;
        }
    }
    changed
}

pub(in crate::app) fn moved_key_map(
    tags_root: &Path,
    source: &Path,
    old_entries: &[TagEntry],
    destination: &Path,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entry in old_entries {
        let TagEntryLocation::LooseFile(old_path) = &entry.location else {
            continue;
        };
        let Ok(inner_rel) = old_path.strip_prefix(source) else {
            continue;
        };
        let new_path = destination.join(inner_rel);
        if new_path.starts_with(tags_root) {
            map.insert(entry.key.clone(), file_entry_key(&new_path));
        }
    }
    map
}

#[cfg(test)]
mod loose_refactor_jobs_tests {
    //! Characterization of the loose-kit refactors -- tag rename and move,
    //! folder rename, move and copy, duplicate, delete -- run to completion on a
    //! temporary tree whose tags reference each other.
    //!
    //! Each job runs as it does in the app: the dialog state is set the way the
    //! dialog leaves it, the apply entry point starts the worker, and the worker's
    //! messages are applied until it settles. The assertions read the results
    //! back off disk (files moved, references rewritten in the referrers' bytes)
    //! and out of the app (tabs, favorites and keys remapped, failures reported).

    use crate::app::loose_fixture::*;
    use super::*;
    use crate::app::browser::BrowserAction;

    const RENDER: &str = "objects/props/crate.render_model";
    const MODEL: &str = "objects/props/crate.model";
    /// Outside the folder, referencing the render model inside it.
    const USER: &str = "levels/test/crate_user.model";
    const BARREL: &str = "objects/props/barrel.model";

    /// `objects/props` holds a render model, a model referencing it and an
    /// unrelated model; `levels/test` holds a second model referencing it from
    /// outside the folder.
    fn kit(name: &str) -> LooseKit {
        let kit = LooseKit::new(name, "halo3_mcc");
        let mode = group_tag("halo3_mcc", "render_model");
        kit.write_mcc("objects/props/crate", "render_model", |_| {});
        for rel in ["objects/props/crate", "levels/test/crate_user"] {
            kit.write_mcc(rel, "model", |tag| {
                set_reference(tag, "render model", mode, "objects\\props\\crate");
            });
        }
        kit.write_mcc("objects/props/barrel", "model", |_| {});
        kit
    }

    fn render_reference(kit: &LooseKit, rel: &str) -> Option<String> {
        let tag = TagFile::read(kit.root.join(rel)).expect("the referrer reads");
        reference_of(&tag, "render model")
    }

    /// The terminal's text, one line each.
    fn terminal(app: &Baboon) -> Vec<String> {
        app.kit_tools.terminal.lines.iter().map(|line| line.text.clone()).collect()
    }

    fn settle(app: &mut Baboon, what: &str) {
        pump_until(app, what, |app| app.tag_ops.folder_refactor.is_none());
    }

    /// The paths of every tag in the kit, relative and with `/`.
    fn tree(kit: &LooseKit) -> Vec<String> {
        let mut paths: Vec<String> = walkdir::WalkDir::new(&kit.root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|item| item.file_type().is_file())
            .map(|item| {
                item.path()
                    .strip_prefix(&kit.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn renaming_a_tag_moves_it_and_rewrites_every_referrer() {
        let kit = kit("refactor-rename-tag");
        let mut app = app();
        kit.install_indexed(&mut app);
        let old_key = kit.key(RENDER);
        let model_key = kit.key(MODEL);
        kit.open(&mut app, RENDER);
        kit.open(&mut app, MODEL);
        app.handle_browser_action(BrowserAction::ToggleFavorite(old_key.clone()), ctx());
        let generation = app.model.kits[0].generation;

        app.handle_browser_action(BrowserAction::RenameTag(old_key.clone()), ctx());
        {
            let state = app
                .dialogs
                .get_mut::<RenameTagState>()
                .expect("the dialog opened");
            assert_eq!(state.referrers, vec![USER.to_owned(), MODEL.to_owned()]);
            state.new_path_input = "crate_renamed".to_owned();
        }
        app.begin_rename_tag(&ctx());
        assert!(
            app.dialogs.get::<RenameTagState>().is_none(),
            "the dialog closed"
        );
        assert!(
            app.tag_ops.folder_refactor.is_some(),
            "the app is locked while it runs"
        );
        settle(&mut app, "the rename");

        let new_rel = "objects/props/crate_renamed.render_model";
        let new_key = kit.key(new_rel);
        assert_eq!(
            app.model.status,
            "Renamed tag, updated 2 reference(s) in 2 tag(s)"
        );
        assert!(!kit.root.join(RENDER).exists());
        assert!(kit.root.join(new_rel).is_file());
        for referrer in [MODEL, USER] {
            assert_eq!(
                render_reference(&kit, referrer).as_deref(),
                Some("objects\\props\\crate_renamed"),
                "{referrer}"
            );
        }
        // Open state follows the tag to its new key.
        let tabs = &app.model.kits[0].open_tabs;
        assert!(tabs.contains(&new_key) && !tabs.contains(&old_key), "{tabs:?}");
        assert!(tabs.contains(&model_key));
        assert_eq!(app.model.kits[0].selected_key.as_deref(), Some(model_key.as_str()));
        // Every document is dropped, edited or not, to be read again.
        assert!(app.model.kits[0].parsed_tags.is_empty());
        assert_ne!(app.model.kits[0].generation, generation);
        // Favorites follow it too.
        assert_eq!(
            app.model.prefs.editing_kit_favorites[0].tags,
            vec![PathBuf::from(new_rel)]
        );
        // The browser and the reference index know the tag by its new path.
        let source = app.model.kits[0].source.as_ref().unwrap();
        assert!(source.all_entries.iter().any(|entry| entry.key == new_key));
        assert!(!source.all_entries.iter().any(|entry| entry.key == old_key));
        let index = source.reverse_dependencies.as_ref().expect("the index survives");
        let mut dependents = index
            .dependents_for(group_tag("halo3_mcc", "render_model"), "objects\\props\\crate_renamed")
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        dependents.sort();
        let mut expected = vec![kit.key(MODEL), kit.key(USER)];
        expected.sort();
        assert_eq!(dependents, expected);
        assert!(terminal(&app).contains(&format!("Renamed: {RENDER} -> {new_rel}")));
        assert!(terminal(&app).contains(&"Updated 2 reference(s) in 2 tag(s)".to_owned()));
    }

    #[test]
    fn a_rename_waits_for_unsaved_edits_and_for_a_running_refactor() {
        let kit = kit("refactor-rename-refused");
        let mut app = app();
        kit.install(&mut app);
        let key = kit.open(&mut app, MODEL);
        edit_field(&mut app, &key, "disappear distance", "4");

        app.handle_browser_action(BrowserAction::RenameTag(kit.key(BARREL)), ctx());
        app.dialogs
            .get_mut::<RenameTagState>()
            .unwrap()
            .new_path_input = "keg".to_owned();
        app.begin_rename_tag(&ctx());
        assert_eq!(app.model.status, "Save or close dirty tags before renaming");
        assert!(
            app.dialogs.get::<RenameTagState>().is_some(),
            "the dialog stays open"
        );
        assert!(kit.root.join(BARREL).is_file());

        app.model.kits[0].parsed_tags.get_mut(&key).unwrap().dirty.clear();
        app.tag_ops.folder_refactor = Some(FolderRefactorUiState {
            label: "Moving".to_owned(),
            phase: "Preparing".to_owned(),
            progress: None,
        });
        app.begin_rename_tag(&ctx());
        assert_eq!(app.model.status, "A move/rename is already running");

        // The name itself is checked before either.
        app.tag_ops.folder_refactor = None;
        for (input, refusal) in [
            ("", "Enter a new tag name"),
            ("sub/keg", "Enter a name only; use Move to choose a folder"),
            ("keg.model", "Enter a name without an extension"),
        ] {
            app.dialogs
                .get_mut::<RenameTagState>()
                .unwrap()
                .new_path_input = input.to_owned();
            app.begin_rename_tag(&ctx());
            assert_eq!(app.model.status, refusal, "{input:?}");
        }
    }

    /// Moving a tag is the rename job with a new folder: the file lands there and
    /// the referrers follow.
    #[test]
    fn moving_a_tag_into_another_folder_rewrites_its_referrers() {
        let kit = kit("refactor-move-tag");
        let entries = kit.entries();
        let entry = entries
            .iter()
            .find(|entry| entry.display_path == RENDER)
            .unwrap()
            .clone();
        let (tx, _rx) = std::sync::mpsc::channel();

        let done = run_tag_rename_job(
            kit.root.clone(),
            entry,
            "objects/moved/crate".to_owned(),
            "Moving tag".to_owned(),
            kit.names(),
            Some(GameId::Halo3),
            entries.clone(),
            Some(kit.index()),
            &tx,
        )
        .expect("the move runs");

        assert_eq!(done.status, "Moved tag, updated 2 reference(s) in 2 tag(s)");
        assert!(done.moved);
        assert!(kit.root.join("objects/moved/crate.render_model").is_file());
        assert!(!kit.root.join(RENDER).exists());
        assert_eq!(
            render_reference(&kit, USER).as_deref(),
            Some("objects\\moved\\crate")
        );
        assert_eq!(done.old_to_new_keys.len(), 1);
        assert_eq!(done.all_entries.len(), entries.len());
        assert!(
            done.tree
                .children
                .iter()
                .any(|node| node.label == "objects"),
            "the browser tree is rebuilt from disk"
        );
    }

    /// Tag paths ignore case, so a rename that only changes case is no rename at
    /// all: it is refused by name on every file system. It used to be refused only
    /// where the file system ignores case (the destination "existed": it was the
    /// tag itself), and on a case-sensitive one the job went ahead and rewrote
    /// every reference to the new spelling.
    #[test]
    fn a_case_only_tag_rename_is_refused_on_every_file_system() {
        let kit = kit("refactor-case-only");
        let entries = kit.entries();
        let entry = entries
            .iter()
            .find(|entry| entry.display_path == RENDER)
            .unwrap()
            .clone();
        let (tx, _rx) = std::sync::mpsc::channel();

        let result = run_tag_rename_job(
            kit.root.clone(),
            entry,
            "objects/props/Crate".to_owned(),
            "Renaming tag".to_owned(),
            kit.names(),
            Some(GameId::Halo3),
            entries,
            Some(kit.index()),
            &tx,
        );

        let error = result.err().expect("refused");
        assert!(
            error.starts_with("Tag paths ignore case"),
            "refused by name, not by the file system: {error}"
        );
        assert_eq!(
            render_reference(&kit, USER).as_deref(),
            Some("objects\\props\\crate"),
            "nothing rewritten"
        );
        assert!(kit.root.join(RENDER).is_file(), "the tag was not moved");

        // Renaming a folder to a case variant is refused by name, everywhere.
        assert_eq!(
            validate_loose_folder_rename_for_test("Props", "props"),
            Err("Tag paths ignore case, so changing only the case would not change any reference"
                .to_owned())
        );
    }

    fn validate_loose_folder_rename_for_test(raw: &str, old: &str) -> Result<String, String> {
        super::folder_rename::validate_loose_folder_rename(raw, old, &[old.to_owned()])
    }

    /// A referrer that cannot be read does not stop the rename: the others are
    /// rewritten, and the one left behind is named in the status and terminal.
    #[test]
    fn a_referrer_that_cannot_be_rewritten_is_reported_and_passed_over() {
        let kit = kit("refactor-failure");
        let mode = group_tag("halo3_mcc", "render_model");
        kit.write_mcc("levels/test/broken", "model", |tag| {
            set_reference(tag, "render model", mode, "objects\\props\\crate");
        });
        let entries = kit.entries();
        // Indexed while it still read, then corrupted: trailing bytes fail the
        // read, and the reference it holds is still there to be found.
        let index = kit.index();
        let broken = kit.root.join("levels/test/broken.model");
        let mut bytes = fs::read(&broken).unwrap();
        bytes.extend_from_slice(b"not part of the tag");
        fs::write(&broken, &bytes).unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.display_path == RENDER)
            .unwrap()
            .clone();
        let (tx, _rx) = std::sync::mpsc::channel();

        let done = run_tag_rename_job(
            kit.root.clone(),
            entry,
            "objects/props/crate2".to_owned(),
            "Renaming tag".to_owned(),
            kit.names(),
            Some(GameId::Halo3),
            entries,
            Some(index),
            &tx,
        )
        .expect("the rename itself succeeds");

        assert_eq!(
            done.status,
            "Renamed tag, updated 2 reference(s) in 2 tag(s); 1 tag(s) could NOT be updated and \
         may still reference the old path (see terminal)"
        );
        assert!(kit.root.join("objects/props/crate2.render_model").is_file());
        assert_eq!(
            render_reference(&kit, USER).as_deref(),
            Some("objects\\props\\crate2")
        );
        assert_eq!(fs::read(&broken).unwrap(), bytes, "the broken tag is untouched");
        assert!(
            done.lines
                .iter()
                .any(|line| line.starts_with("Warning: not updated: levels/test/broken.model (")),
            "{:?}",
            done.lines
        );
        assert!(done.lines.contains(
            &"Warning: 1 tag(s) could not be updated and may still reference the old path:".to_owned()
        ));
    }

    #[test]
    fn renaming_a_folder_moves_its_tags_and_rewrites_referrers_outside_it() {
        let kit = kit("refactor-rename-folder");
        let mut app = app();
        kit.install_indexed(&mut app);
        let old_key = kit.key(RENDER);
        kit.open(&mut app, RENDER);
        app.handle_browser_action(BrowserAction::ToggleFavorite(old_key.clone()), ctx());
        app.handle_browser_action(
            BrowserAction::ToggleFolderFavorite(PathBuf::from("objects/props")),
            ctx(),
        );

        app.handle_browser_action(
            BrowserAction::RenameLooseFolder {
                rel_path: PathBuf::from("objects/props"),
                label: "props".to_owned(),
            },
            ctx(),
        );
        {
            let state = app
                .dialogs
                .get_mut::<LooseFolderRenameState>()
                .expect("the dialog opened");
            assert_eq!(state.tag_count, 3);
            assert_eq!(state.outside_referrers.as_deref(), Some(&[USER.to_owned()][..]));
            state.name_input = "Props".to_owned();
        }
        assert!(!app.apply_loose_folder_rename(), "a case-only rename keeps the dialog open");
        assert_eq!(
            app.dialogs
                .get::<LooseFolderRenameState>()
                .unwrap()
                .error
                .as_deref(),
            Some("Tag paths ignore case, so changing only the case would not change any reference")
        );
        app.dialogs
            .get_mut::<LooseFolderRenameState>()
            .unwrap()
            .name_input = "crates".to_owned();
        assert!(app.apply_loose_folder_rename(), "accepted");
        assert!(app.tag_ops.folder_refactor.is_some());
        settle(&mut app, "the folder rename");

        assert!(app.model.status.starts_with("Renamed"), "{}", app.model.status);
        assert!(!app.model.status.contains("NOT"), "{}", app.model.status);
        assert_eq!(
            tree(&kit),
            vec![
                "levels/test/crate_user.model",
                "objects/crates/barrel.model",
                "objects/crates/crate.model",
                "objects/crates/crate.render_model",
            ]
        );
        for referrer in [USER, "objects/crates/crate.model"] {
            assert_eq!(
                render_reference(&kit, referrer).as_deref(),
                Some("objects\\crates\\crate"),
                "{referrer}"
            );
        }
        let new_key = kit.key("objects/crates/crate.render_model");
        assert!(app.model.kits[0].open_tabs.contains(&new_key));
        assert!(!app.model.kits[0].open_tabs.contains(&old_key));
        assert_eq!(
            app.model.prefs.editing_kit_favorites[0].tags,
            vec![PathBuf::from("objects/crates/crate.render_model")]
        );
        // A favorited folder follows the rename, as its tags do. It used to be
        // left at the old path and then pruned as missing, losing the favorite.
        assert_eq!(
            app.model.prefs.editing_kit_favorites[0].folders,
            vec![PathBuf::from("objects/crates")]
        );
        assert_eq!(app.model.kits[0].active_favorite_folders.len(), 1);
    }

    #[test]
    fn moving_a_folder_carries_its_tags_and_their_references() {
        let kit = kit("refactor-move-folder");
        let (tx, _rx) = std::sync::mpsc::channel();

        let done = run_folder_refactor_job(
            kit.root.clone(),
            PathBuf::from("objects/props"),
            kit.root.join("levels"),
            None,
            true,
            "Moving props".to_owned(),
            kit.names(),
            Some(GameId::Halo3),
            kit.entries(),
            Some(kit.index()),
            &tx,
        )
        .expect("the move runs");

        assert!(done.moved);
        assert!(!done.status.contains("NOT"), "{}", done.status);
        assert_eq!(
            tree(&kit),
            vec![
                "levels/props/barrel.model",
                "levels/props/crate.model",
                "levels/props/crate.render_model",
                "levels/test/crate_user.model",
            ]
        );
        assert_eq!(
            render_reference(&kit, USER).as_deref(),
            Some("levels\\props\\crate")
        );
        assert_eq!(
            render_reference(&kit, "levels/props/crate.model").as_deref(),
            Some("levels\\props\\crate")
        );
        assert_eq!(done.old_to_new_keys.len(), 3);
        assert_eq!(
            done.old_to_new_keys.get(&kit.key(USER)),
            None,
            "a referrer outside the folder keeps its key"
        );
    }

    /// A copy leaves the original and its referrers alone; the copied tags
    /// reference the copies, not the originals.
    #[test]
    fn copying_a_folder_leaves_the_original_and_points_the_copy_at_itself() {
        let kit = kit("refactor-copy-folder");
        let (tx, _rx) = std::sync::mpsc::channel();

        let done = run_folder_refactor_job(
            kit.root.clone(),
            PathBuf::from("objects/props"),
            kit.root.join("levels"),
            None,
            false,
            "Copying props".to_owned(),
            kit.names(),
            Some(GameId::Halo3),
            kit.entries(),
            Some(kit.index()),
            &tx,
        )
        .expect("the copy runs");

        assert!(!done.moved);
        assert!(done.old_to_new_keys.is_empty() || !done.moved);
        assert_eq!(
            tree(&kit),
            vec![
                "levels/props/barrel.model",
                "levels/props/crate.model",
                "levels/props/crate.render_model",
                "levels/test/crate_user.model",
                "objects/props/barrel.model",
                "objects/props/crate.model",
                "objects/props/crate.render_model",
            ]
        );
        assert_eq!(
            render_reference(&kit, USER).as_deref(),
            Some("objects\\props\\crate"),
            "the outside referrer keeps the original"
        );
        assert_eq!(
            render_reference(&kit, MODEL).as_deref(),
            Some("objects\\props\\crate")
        );
        assert_eq!(
            render_reference(&kit, "levels/props/crate.model").as_deref(),
            Some("levels\\props\\crate")
        );
        assert_eq!(done.all_entries.len(), 7);
    }

    #[test]
    fn duplicating_a_tag_copies_its_bytes_beside_it() {
        let kit = kit("refactor-duplicate");
        let mut app = app();
        kit.install(&mut app);
        let key = kit.key(MODEL);

        app.handle_browser_action(BrowserAction::DuplicateTag(key.clone()), ctx());
        app.dialogs
            .get_mut::<RenameTagState>()
            .unwrap()
            .new_path_input = "crate_copy".to_owned();
        app.begin_rename_tag(&ctx());

        let copy = kit.root.join("objects/props/crate_copy.model");
        assert_eq!(fs::read(&copy).unwrap(), fs::read(kit.root.join(MODEL)).unwrap());
        assert_eq!(
            app.model.status,
            format!("Duplicated {MODEL} → {}", copy.display())
        );
        let copy_key = kit.key("objects/props/crate_copy.model");
        assert!(app.model.entry_for_key(&copy_key).is_some(), "registered in the browser");
        assert!(app.model.kits[0].parsed_tags.contains_key(&copy_key), "and opened clean");
        assert!(!app.model.kits[0].parsed_tags[&copy_key].dirty.is_set());
        assert!(app.dialogs.get::<RenameTagState>().is_none());

        // A name already taken is refused, and nothing is written.
        let barrel = fs::read(kit.root.join(BARREL)).unwrap();
        app.handle_browser_action(BrowserAction::DuplicateTag(key.clone()), ctx());
        app.dialogs
            .get_mut::<RenameTagState>()
            .unwrap()
            .new_path_input = "barrel".to_owned();
        app.begin_rename_tag(&ctx());
        assert!(
            app.dialogs.get::<RenameTagState>().is_some(),
            "the dialog stays open"
        );
        assert_eq!(fs::read(kit.root.join(BARREL)).unwrap(), barrel);
        assert_eq!(app.model.status, "A tag with that name already exists in this source");
    }

    /// A dirty tag is duplicated with its edit; the original file is not saved.
    #[test]
    fn duplicating_an_edited_tag_copies_the_edit_not_the_file() {
        let kit = kit("refactor-duplicate-dirty");
        let mut app = app();
        kit.install(&mut app);
        let key = kit.open(&mut app, MODEL);
        edit_field(&mut app, &key, "disappear distance", "9");
        let original = fs::read(kit.root.join(MODEL)).unwrap();

        app.handle_browser_action(BrowserAction::DuplicateTag(key.clone()), ctx());
        app.dialogs
            .get_mut::<RenameTagState>()
            .unwrap()
            .new_path_input = "crate_edited".to_owned();
        app.begin_rename_tag(&ctx());

        let copy = TagFile::read(kit.root.join("objects/props/crate_edited.model")).unwrap();
        assert_eq!(real_of(&copy, "disappear distance"), Some(9.0));
        assert_eq!(fs::read(kit.root.join(MODEL)).unwrap(), original);
        assert!(app.model.kits[0].parsed_tags[&key].dirty.is_set(), "the original stays dirty");
    }

    #[test]
    fn deleting_a_tag_moves_it_to_the_trash_and_forgets_it() {
        let kit = kit("refactor-delete");
        // A leaf no other test deletes: the trash is shared by the test process
        // and keyed by the second.
        let doomed = "objects/props/doomed_by_delete_test.model";
        kit.write_mcc("objects/props/doomed_by_delete_test", "model", |_| {});
        let bytes = fs::read(kit.root.join(doomed)).unwrap();
        let mut app = app();
        kit.install(&mut app);
        let key = kit.open(&mut app, doomed);
        let generation = app.model.kits[0].generation;

        app.handle_browser_action(BrowserAction::DeleteTag(key.clone()), ctx());
        assert!(app.dialogs.get::<DeleteConfirm>().is_some());
        app.begin_delete_tag(ctx());

        assert!(!kit.root.join(doomed).exists());
        let destination = app
            .model.status
            .strip_prefix(&format!("Deleted {doomed} — moved to "))
            .map(PathBuf::from)
            .unwrap_or_else(|| panic!("{}", app.model.status));
        assert_eq!(fs::read(&destination).unwrap(), bytes, "moved, not destroyed");
        assert!(destination.ends_with(doomed));
        assert!(
            destination
                .components()
                .any(|part| part.as_os_str() == "halo3_mcc"),
            "{}",
            destination.display()
        );
        assert!(app.model.entry_for_key(&key).is_none());
        assert!(!app.model.kits[0].parsed_tags.contains_key(&key));
        assert_eq!(app.model.kits[0].selected_key, None);
        assert_ne!(app.model.kits[0].generation, generation);
        assert!(app.dialogs.get::<DeleteConfirm>().is_none());
        let _ = fs::remove_file(destination);
    }
}
