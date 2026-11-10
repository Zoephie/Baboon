//! Merge orchestration: complete reference checks, review, revalidate, then one
//! journalled filesystem/reference operation. No writes happen during review.
use super::folder_merge_review::{self as review, MergeControl, MergeUi, ReviewKind};
use super::*;
use crate::core::{
    folder_merge::{self as disk, Choice, Inventory, Transaction},
    recovery::{self, RecoveryOrigin},
};
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::Ordering;

pub(in crate::app) struct MergeContext {
    pub recovery: Result<(PathBuf, RecoveryOrigin), String>,
    pub control: Arc<MergeControl>,
}

impl Baboon {
    pub(in crate::app) fn save_before_folder_operation(&mut self) {
        let Some(request) = self
            .dialogs
            .get::<review::SaveBeforeFolderOperation>()
            .cloned()
        else {
            return;
        };
        if self.model.resolve_stamp(request.kit) != Some(self.model.active) {
            self.dialogs.close::<review::SaveBeforeFolderOperation>();
            self.model.status = "The workspace changed; start the folder operation again".into();
            return;
        }
        let keys: Vec<_> = self.model.kits[self.model.active]
            .parsed_tags
            .iter()
            .filter(|(_, doc)| doc.dirty.is_set())
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            if let Err(error) = self.save_tag_by_key(&key) {
                if let Some(dialog) = self.dialogs.get_mut::<review::SaveBeforeFolderOperation>() {
                    dialog.error = Some(error);
                }
                return;
            }
        }
        self.dialogs.close::<review::SaveBeforeFolderOperation>();
        self.begin_refactor_loose_folder(request.rel_path, request.label, request.moving);
    }
    pub(in crate::app) fn new_merge_context(&mut self) -> MergeContext {
        let control = Arc::new(MergeControl::default());
        self.tag_ops.folder_merge = Some(MergeUi {
            control: control.clone(),
            review: None,
        });
        MergeContext {
            recovery: self.loose_recovery_context(self.model.active),
            control,
        }
    }
}

fn references(
    path: &Path,
    entries: &[TagEntry],
    index: &ReverseDependencyIndex,
    names: &TagNameIndex,
) -> Option<usize> {
    let entry = entries
        .iter()
        .find(|entry| matches!(&entry.location, TagEntryLocation::LooseFile(p) if p == path))?;
    let reference = dependency_entry_reference_path(entry, names)?;
    Some(
        index
            .dependents_for(entry.group_tag, &reference)
            .into_iter()
            .filter(|key| *key != &entry.key)
            .collect::<HashSet<_>>()
            .len(),
    )
}

/// Kept separate from the legacy rename path: renaming into an existing folder
/// still reports a name error, while Move and Copy explicitly merge.
pub(in crate::app) fn run(
    root: PathBuf,
    source: PathBuf,
    destination: PathBuf,
    moving: bool,
    label: String,
    names: TagNameIndex,
    game: Option<GameId>,
    previous_entries: Vec<TagEntry>,
    previous_index: Option<ReverseDependencyIndex>,
    context: MergeContext,
    tx: &Sender<WorkerMessage>,
) -> Result<FolderRefactorFinished, String> {
    let cancelled = || context.control.cancelled();
    let source_kind = TagSource::LooseFolder {
        root: root.clone(),
        game,
        definitions_root: locate_definitions_root(),
    };
    let mut previous_index = previous_index;
    loop {
        if cancelled() {
            return Err("Folder operation cancelled; no files changed".into());
        }
        send_folder_refactor_progress(tx, &label, "Scanning folders and checking references", None);
        let inventory = Inventory::scan(&source, &destination, &cancelled)?;
        let (all_before, mut index, mut index_changes) =
            refactor::prepare_tag_refactor_index_cancellable(
                &root,
                &source_kind,
                game,
                &names,
                &previous_entries,
                previous_index.take(),
                &label,
                tx,
                &cancelled,
            )?;
        if let Some(entry) = all_before
            .iter()
            .find(|entry| !index.contains_tag(&entry.key))
        {
            return Err(format!(
                "Cannot merge until references can be checked for {}. No files changed.",
                entry.display_path
            ));
        }
        // Inventory already hashes the reviewed source/destination trees. Only
        // their possible referrers need content hashes, not every tag payload
        // in the kit (which can total many GB). Metadata still catches edits
        // elsewhere that could introduce a new reference while review is open.
        let mut referrers = HashSet::new();
        if moving {
            for entry in &all_before {
                if matches!(&entry.location, TagEntryLocation::LooseFile(path) if path.starts_with(&source))
                {
                    if let Some(path) = dependency_entry_reference_path(entry, &names) {
                        referrers
                            .extend(index.dependents_for(entry.group_tag, &path).iter().cloned());
                    }
                }
            }
        }
        send_folder_refactor_progress(tx, &label, "Checking files before review", None);
        let reference_files = all_before
            .iter()
            .enumerate()
            .filter_map(|(number, entry)| match &entry.location {
                TagEntryLocation::LooseFile(path) => Some(if cancelled() {
                    Err("Folder operation cancelled; no files changed".into())
                } else {
                    if number % 100 == 0 {
                        send_folder_refactor_progress(
                            tx,
                            &label,
                            &format!(
                                "Checking files before review: {}/{}",
                                number,
                                all_before.len()
                            ),
                            Some(number as f32 / all_before.len().max(1) as f32),
                        );
                    }
                    let contents = referrers.contains(&entry.key);
                    let result = if contents {
                        disk::fingerprint_cancellable(path, &cancelled)
                    } else {
                        disk::metadata_fingerprint(path)
                    };
                    result.map(|info| (path.clone(), contents, info))
                }),
                _ => None,
            })
            .collect::<Result<Vec<_>, _>>()?;
        review::ask(
            tx,
            &context.control,
            ReviewKind::Summary {
                source: source.clone(),
                destination: destination.clone(),
                files: inventory.files(),
                conflicts: inventory.conflicts(&cancelled)?,
            },
        )?;
        let mut remaining = None;
        let plan = inventory.plan(
            |conflict| {
                if let Some(choice) = remaining
                    && (choice != Choice::Replace || conflict.replace_allowed())
                {
                    return Ok(choice);
                }
                let incoming_references =
                    references(&conflict.incoming, &all_before, &index, &names);
                let existing_references =
                    references(&conflict.existing, &all_before, &index, &names);
                let answer = review::ask(
                    tx,
                    &context.control,
                    ReviewKind::Conflict {
                        conflict,
                        incoming_references,
                        existing_references,
                    },
                )?;
                let choice = answer.choice.ok_or("No conflict choice selected")?;
                if answer.apply_remaining {
                    remaining = Some(choice);
                }
                Ok(choice)
            },
            &cancelled,
        )?;
        send_folder_refactor_progress(tx, &label, "Verifying reviewed files", None);
        let current_entries =
            scan_folder_subtree_entries(&root, Path::new(""), &names).map_err(|e| e.to_string())?;
        let current_keys: HashSet<_> = current_entries.iter().map(|entry| &entry.key).collect();
        let previous_keys: HashSet<_> = all_before.iter().map(|entry| &entry.key).collect();
        let mut changed_reference_keys = Vec::new();
        for (path, contents, info) in &reference_files {
            if cancelled() {
                return Err("Folder operation cancelled; no files changed".into());
            }
            let current = if *contents {
                disk::fingerprint_cancellable(path, &cancelled)
            } else {
                disk::metadata_fingerprint(path)
            };
            if !current.is_ok_and(|current| &current == info) {
                changed_reference_keys.push(file_entry_key(path));
            }
        }
        let unchanged = current_keys == previous_keys
            && plan.vacant_targets()?
            && inventory.unchanged(&cancelled)?
            && changed_reference_keys.is_empty();
        if !unchanged {
            send_folder_refactor_progress(
                tx,
                &label,
                "Files changed during review; rebuilding review",
                None,
            );
            for key in changed_reference_keys {
                index.clear_tag(&key);
            }
            previous_index = Some(index);
            continue;
        }
        if cancelled() {
            return Err("Folder operation cancelled; no files changed".into());
        }

        let targets: HashMap<_, _> = plan
            .files
            .iter()
            .map(|file| (file.source.clone(), file.target.clone()))
            .collect();
        let old_entries: Vec<_> = all_before.iter().filter(|entry| matches!(&entry.location, TagEntryLocation::LooseFile(path) if targets.contains_key(path))).cloned().collect();
        let new_entries: Vec<_> = old_entries
            .iter()
            .map(|old| {
                let TagEntryLocation::LooseFile(path) = &old.location else {
                    unreachable!()
                };
                let target = &targets[path];
                TagEntry {
                    key: file_entry_key(target),
                    display_path: tag_path::relative_tag_path(target.strip_prefix(&root).unwrap()),
                    location: TagEntryLocation::LooseFile(target.clone()),
                    ..old.clone()
                }
            })
            .collect();
        let mut rewrites = HashMap::new();
        for (old, new) in old_entries.iter().zip(&new_entries) {
            if let (Some(from), Some(to)) = (
                dependency_entry_reference_path(old, &names),
                dependency_entry_reference_path(new, &names),
            ) {
                rewrites.insert((old.group_tag, from.to_ascii_lowercase()), to);
            }
        }
        let rewrite_entries = if moving {
            affected_move_rewrite_entries(
                &all_before,
                &old_entries,
                &new_entries,
                &rewrites,
                Some(&index),
            )
        } else {
            new_entries.clone()
        };
        let schema = game
            .map(|g| {
                locate_definitions_root()
                    .join(g.as_str())
                    .join("tag_dependency_list.json")
            })
            .filter(|p| p.is_file());
        context.control.begin_apply()?;
        send_folder_refactor_progress(
            tx,
            &label,
            "Applying folder merge (Cancel unavailable)",
            None,
        );
        let mut transaction = Transaction::new()?;
        let mut recovery_paths = Vec::new();
        let mut recovery_tree = None;
        let applied = (|| {
            // Protect every original referrer before any copied tag can replace it.
            for entry in &rewrite_entries {
                if let TagEntryLocation::LooseFile(path) = &entry.location {
                    transaction.remember(path)?;
                }
            }
            transaction.apply(&plan, |path| {
                // Reserve lazily, once per operation. A batch for each target
                // scattered one merge's originals across timestamp suffixes.
                if recovery_tree.is_none() {
                    let (base, origin) = context.recovery.as_ref().map_err(Clone::clone)?;
                    recovery::prepare_custom_location(
                        &crate::core::storage::data_path("deleted-tags"),
                        base,
                    )?;
                    let relative = destination.strip_prefix(&root).map_err(|e| e.to_string())?;
                    recovery_tree = Some(recovery::prepare_batch(
                        base,
                        origin,
                        relative,
                        "replace during folder merge",
                        recovery::now_secs(),
                    )?);
                }
                let relative = path.strip_prefix(&destination).map_err(|e| e.to_string())?;
                let saved = recovery_tree.as_ref().unwrap().join(relative);
                fs::create_dir_all(saved.parent().ok_or("Invalid recovery path")?)
                    .map_err(|e| format!("Could not create recovery subfolder: {e}"))?;
                disk::copy_backup(path, &saved).map_err(|e| {
                    format!(
                        "Could not preserve overwritten file {}: {e}",
                        path.display()
                    )
                })?;
                recovery_paths.push(saved);
                Ok(())
            })?;
            let rewritten = rewrite_transactional(
                &source_kind,
                &rewrite_entries,
                &rewrites,
                schema.as_deref(),
                tx,
                &label,
                &mut transaction,
            )?;
            if moving {
                transaction.remove_sources(&plan)?;
            }
            let tree = crate::core::source::build_folder_directory_tree(&root)
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((rewritten, tree))
        })();
        let (rewritten, tree) = match applied {
            Ok(done) => done,
            Err(error) => {
                let error = transaction.rollback(error);
                let _ = tx.send(WorkerMessage::TerminalLine(error.clone()));
                return Err(error);
            }
        };
        transaction.commit();
        let mut all: HashMap<_, _> = all_before
            .into_iter()
            .map(|entry| (entry.key.clone(), entry))
            .collect();
        if moving {
            for entry in &old_entries {
                all.remove(&entry.key);
                index_changes.removed.push(entry.key.clone());
            }
        }
        for entry in &new_entries {
            all.insert(entry.key.clone(), entry.clone());
        }
        let mut all_entries: Vec<_> = all.into_values().collect();
        all_entries.sort_by_cached_key(|e| crate::core::source::natural_key(&e.display_path));
        let mut index = index;
        refresh_reverse_dependency_index_after_refactor(
            &mut index,
            &source_kind,
            moving,
            &old_entries,
            &new_entries,
            &rewritten.changed_keys,
            &all_entries,
        );
        index_changes
            .touched
            .extend(new_entries.iter().map(|e| e.key.clone()));
        index_changes
            .touched
            .extend(rewritten.changed_keys.iter().cloned());
        refactor::persist_refactor_indexes(
            &root,
            game,
            &all_entries,
            Some(&index),
            &index_changes,
            tx,
        );
        let old_to_new_keys = if moving {
            old_entries
                .iter()
                .zip(&new_entries)
                .map(|(old, new)| (old.key.clone(), new.key.clone()))
                .collect()
        } else {
            HashMap::new()
        };
        let action = if moving { "Moved" } else { "Copied" };
        let status = format!(
            "{action} {} file(s), updated {} reference(s) in {} tag(s)",
            plan.files.len(),
            rewritten.references_changed,
            rewritten.tags_changed
        );
        let mut lines = vec![
            format!(
                "Merged folder: {} -> {}",
                source.display(),
                destination.display()
            ),
            status.clone(),
        ];
        lines.extend(
            recovery_paths
                .iter()
                .map(|p| format!("Replaced file preserved at {}", p.display())),
        );
        return Ok(FolderRefactorFinished {
            status,
            lines,
            tree,
            all_entries,
            reverse_dependencies: Some(index),
            old_to_new_keys,
            moved: moving,
            moved_folder: (moving && !source.exists()).then(|| {
                (
                    source.strip_prefix(&root).unwrap().to_owned(),
                    plan.directories
                        .first()
                        .unwrap_or(&destination)
                        .strip_prefix(&root)
                        .unwrap()
                        .to_owned(),
                )
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::*;

    fn kit() -> LooseKit {
        let kit = LooseKit::new("folder-merge", "halo3_mcc");
        let group = group_tag(kit.game, "render_model");
        kit.write_mcc("objects\\props\\crate", "render_model", |_| {});
        kit.write_mcc("levels\\props\\crate", "render_model", |_| {});
        for path in ["objects\\props\\crate", "levels\\test\\user"] {
            kit.write_mcc(path, "model", |tag| {
                set_reference(tag, "render model", group, "objects\\props\\crate")
            });
        }
        kit.write_mcc("levels\\props\\observer", "model", |tag| {
            set_reference(tag, "render model", group, "levels\\props\\crate")
        });
        kit
    }
    fn run_review(
        kit: &LooseKit,
        moving: bool,
        mut answer: impl FnMut(&ReviewKind) -> Option<review::MergeAnswer> + Send + 'static,
    ) -> Result<FolderRefactorFinished, String> {
        let (tx, rx) = mpsc::channel();
        let responder = std::thread::spawn(move || {
            while let Ok(message) = rx.recv() {
                if let WorkerMessage::FolderMergeReview(request) = message {
                    let _ = request.answer.send(answer(&request.kind));
                }
            }
        });
        let result = run(
            kit.root.clone(),
            kit.path("objects/props"),
            kit.path("levels/props"),
            moving,
            "Merging props".into(),
            kit.names(),
            Some(GameId::Halo3),
            kit.entries(),
            Some(kit.index()),
            MergeContext {
                recovery: Ok((
                    kit.base.join("recovery"),
                    RecoveryOrigin {
                        source: recovery::RecoverySource::TagsFolder,
                        tags_root: kit.root.clone(),
                        game: Some(kit.game.into()),
                        kit_id: None,
                        kit_name: None,
                    },
                )),
                control: Arc::new(MergeControl::default()),
            },
            &tx,
        );
        drop(tx);
        responder.join().unwrap();
        result
    }
    fn choosing(choice: Choice) -> impl FnMut(&ReviewKind) -> Option<review::MergeAnswer> + Send {
        move |kind| {
            Some(review::MergeAnswer {
                choice: matches!(kind, ReviewKind::Conflict { .. }).then_some(choice),
                apply_remaining: false,
            })
        }
    }
    fn reference(kit: &LooseKit, path: &str) -> Option<String> {
        reference_of(&TagFile::read(kit.path(path)).unwrap(), "render model")
    }

    #[test]
    fn moving_keep_both_rewrites_incoming_references_and_preserves_destination_identity() {
        let kit = kit();
        let done = run_review(&kit, true, |kind| {
            if let ReviewKind::Conflict {
                incoming_references,
                existing_references,
                ..
            } = kind
            {
                assert_eq!(*incoming_references, Some(2));
                assert_eq!(*existing_references, Some(1));
            }
            Some(review::MergeAnswer {
                choice: matches!(kind, ReviewKind::Conflict { .. }).then_some(Choice::KeepBoth),
                apply_remaining: true,
            })
        })
        .unwrap();
        assert_eq!(
            reference(&kit, "levels/props/crate.model").as_deref(),
            Some("levels\\props\\crate (1)")
        );
        assert_eq!(
            reference(&kit, "levels/test/user.model").as_deref(),
            Some("levels\\props\\crate (1)")
        );
        assert_eq!(
            reference(&kit, "levels/props/observer.model").as_deref(),
            Some("levels\\props\\crate")
        );
        assert!(kit.path("levels/props/crate.render_model").exists());
        assert!(kit.path("levels/props/crate (1).render_model").exists());
        assert!(!kit.path("objects/props").exists());
        assert_eq!(done.old_to_new_keys.len(), 2);
    }
    #[test]
    fn move_skip_keeps_source_and_its_referrers_while_moving_other_files() {
        let kit = kit();
        let done = run_review(&kit, true, choosing(Choice::Skip)).unwrap();
        assert!(kit.path("objects/props/crate.render_model").exists());
        assert!(!kit.path("objects/props/crate.model").exists());
        assert_eq!(
            reference(&kit, "levels/test/user.model").as_deref(),
            Some("objects\\props\\crate")
        );
        assert_eq!(
            reference(&kit, "levels/props/crate.model").as_deref(),
            Some("objects\\props\\crate")
        );
        assert_eq!(done.old_to_new_keys.len(), 1);
        assert!(done.moved_folder.is_none());
    }
    #[test]
    fn copy_replace_preserves_recovery_and_only_rewrites_copied_tags() {
        let kit = kit();
        kit.write_mcc("objects/props/nested/shared", "model", |_| {});
        kit.write_mcc("levels/props/nested/shared", "model", |_| {});
        let old = fs::read(kit.path("levels/props/crate.render_model")).unwrap();
        let old_nested = fs::read(kit.path("levels/props/nested/shared.model")).unwrap();
        let done = run_review(&kit, false, |kind| {
            Some(review::MergeAnswer {
                choice: matches!(kind, ReviewKind::Conflict { .. }).then_some(Choice::Replace),
                apply_remaining: true,
            })
        })
        .unwrap();
        assert_eq!(
            reference(&kit, "objects/props/crate.model").as_deref(),
            Some("objects\\props\\crate")
        );
        assert_eq!(
            reference(&kit, "levels/test/user.model").as_deref(),
            Some("objects\\props\\crate")
        );
        assert_eq!(
            reference(&kit, "levels/props/crate.model").as_deref(),
            Some("levels\\props\\crate")
        );
        let backup = walkdir::WalkDir::new(kit.base.join("recovery"))
            .into_iter()
            .filter_map(Result::ok)
            .find(|e| e.file_name() == "crate.render_model")
            .unwrap();
        assert_eq!(fs::read(backup.path()).unwrap(), old);
        let batches: Vec<_> = fs::read_dir(kit.base.join("recovery"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(
            batches.len(),
            1,
            "one merge must preserve all replacements in one recovery batch"
        );
        assert_eq!(
            fs::read(batches[0].join("files/levels/props/crate.render_model")).unwrap(),
            old
        );
        assert_eq!(
            fs::read(batches[0].join("files/levels/props/nested/shared.model")).unwrap(),
            old_nested
        );
        assert!(done.old_to_new_keys.is_empty());
    }
    #[test]
    fn cancelling_review_leaves_files_and_recovery_untouched() {
        let kit = kit();
        let source = fs::read(kit.path("objects/props/crate.model")).unwrap();
        assert!(
            run_review(&kit, true, |_| None)
                .err()
                .unwrap()
                .contains("cancelled")
        );
        assert_eq!(
            fs::read(kit.path("objects/props/crate.model")).unwrap(),
            source
        );
        assert!(!kit.path("levels/props/crate.model").exists());
        assert!(!kit.base.join("recovery").exists());
    }
    #[test]
    fn failed_move_rolls_back_already_rewritten_referrers_and_copied_files() {
        let kit = kit();
        let source = kit.path("objects/props/crate.render_model");
        let original_permissions = fs::metadata(&source).unwrap().permissions();
        let mut readonly = original_permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&source, readonly).unwrap();
        let result = run_review(&kit, true, choosing(Choice::KeepBoth));
        fs::set_permissions(&source, original_permissions).unwrap();
        let error = result.err().expect("read-only source cannot be removed");
        assert!(error.contains("rolled back"), "{error}");
        assert_eq!(
            reference(&kit, "levels/test/user.model").as_deref(),
            Some("objects\\props\\crate")
        );
        assert!(!kit.path("levels/props/crate.model").exists());
        assert!(!kit.path("levels/props/crate (1).render_model").exists());
        assert!(kit.path("objects/props/crate.model").exists());
    }

    #[test]
    fn changed_review_is_rebuilt_before_applying_anything() {
        let kit = kit();
        let source = kit.path("objects/props/new-note.txt");
        let summaries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = summaries.clone();
        run_review(&kit, false, move |kind| {
            if matches!(kind, ReviewKind::Summary { .. })
                && count.fetch_add(1, Ordering::SeqCst) == 0
            {
                fs::write(&source, b"added during review").unwrap();
            }
            Some(review::MergeAnswer {
                choice: matches!(kind, ReviewKind::Conflict { .. }).then_some(Choice::KeepBoth),
                apply_remaining: false,
            })
        })
        .unwrap();
        assert_eq!(summaries.load(Ordering::SeqCst), 2);
        assert_eq!(
            fs::read(kit.path("levels/props/new-note.txt")).unwrap(),
            b"added during review"
        );
    }

    #[test]
    fn a_new_reference_outside_the_merge_rebuilds_review_and_follows_the_move() {
        let kit = kit();
        let path = kit.write_mcc("elsewhere\\new_user", "model", |_| {});
        let summaries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = summaries.clone();
        let group = group_tag(kit.game, "render_model");
        run_review(&kit, true, move |kind| {
            if matches!(kind, ReviewKind::Summary { .. })
                && count.fetch_add(1, Ordering::SeqCst) == 0
            {
                let mut tag = TagFile::read(&path).unwrap();
                set_reference(&mut tag, "render model", group, "objects\\props\\crate");
                tag.write(&path).unwrap();
            }
            Some(review::MergeAnswer {
                choice: matches!(kind, ReviewKind::Conflict { .. }).then_some(Choice::KeepBoth),
                apply_remaining: false,
            })
        })
        .unwrap();
        assert_eq!(summaries.load(Ordering::SeqCst), 2);
        assert_eq!(
            reference(&kit, "elsewhere/new_user.model").as_deref(),
            Some("levels\\props\\crate (1)")
        );
    }

    #[test]
    fn a_referrer_content_edit_invalidates_review_even_with_unchanged_metadata() {
        let kit = kit();
        let path = kit.path("levels/test/user.model");
        let summaries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = summaries.clone();
        let group = group_tag(kit.game, "render_model");
        run_review(&kit, true, move |kind| {
            if matches!(kind, ReviewKind::Summary { .. })
                && count.fetch_add(1, Ordering::SeqCst) == 0
            {
                let metadata = fs::metadata(&path).unwrap();
                let mut tag = TagFile::read(&path).unwrap();
                set_reference(&mut tag, "render model", group, "levels_\\props\\crate");
                tag.write(&path).unwrap();
                assert_eq!(fs::metadata(&path).unwrap().len(), metadata.len());
                fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
                    .unwrap();
            }
            Some(review::MergeAnswer {
                choice: matches!(kind, ReviewKind::Conflict { .. }).then_some(Choice::KeepBoth),
                apply_remaining: false,
            })
        })
        .unwrap();
        assert_eq!(summaries.load(Ordering::SeqCst), 2);
        assert_eq!(
            reference(&kit, "levels/test/user.model").as_deref(),
            Some("levels_\\props\\crate")
        );
    }
}

fn rewrite_transactional(
    source: &TagSource,
    entries: &[TagEntry],
    rewrites: &HashMap<(u32, String), String>,
    schema: Option<&Path>,
    tx: &Sender<WorkerMessage>,
    label: &str,
    transaction: &mut Transaction,
) -> Result<refactor::ReferenceRewriteResult, String> {
    let mut result = refactor::ReferenceRewriteResult::default();
    let needles = rewrite_reference_needles(rewrites);
    if needles.is_empty() {
        return Ok(result);
    }
    for entry in entries {
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            continue;
        };
        let bytes =
            fs::read(path).map_err(|e| format!("Could not read {}: {e}", entry.display_path))?;
        if !bytes_contain_any_ascii_case_insensitive(&bytes, &needles) {
            continue;
        }
        send_folder_refactor_progress(
            tx,
            label,
            &format!("Rewriting {}", entry.display_path),
            None,
        );
        let mut tag = read_entry(source, entry)
            .map_err(|e| format!("Could not parse {}: {e}", entry.display_path))?;
        let changed = rewrite_references_in_tag(&mut tag, rewrites);
        if changed == 0 {
            continue;
        }
        if tag.classic_engine().is_none()
            && let Some(schema) = schema
        {
            tag.rebuild_dependency_list(schema).map_err(|e| {
                format!(
                    "Could not rebuild dependencies for {}: {e}",
                    entry.display_path
                )
            })?;
        }
        transaction.will_modify(path)?;
        tag.write_atomic(path)
            .map_err(|e| format!("Could not rewrite {}: {e}", entry.display_path))?;
        result.references_changed += changed;
        result.tags_changed += 1;
        result.changed_keys.push(entry.key.clone());
    }
    Ok(result)
}
