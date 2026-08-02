//! Deleting a tag: loose files to a recoverable trash folder, Campaign Evolved
//! copies out of the container that holds them.
//! It owns the delete workflow and its safety gates; container surgery belongs
//! to `blam-tags`, and browser presentation belongs to the browser modules.
//!
//! The two halves are deliberately asymmetric. A loose tag is a file, so
//! deleting it is a move — reversible by hand, and reversible on purpose. A
//! Campaign Evolved tag lives inside the game's own pak, so deleting it rewrites
//! a shipped file; that is restricted to copies Baboon itself made, and it is
//! Baboon's own records — the duplicate ledger and the backups it leaves beside
//! a container — that say which those are. The container cannot say: a copy in a
//! pak is indistinguishable from a tag the game shipped.

use super::*;

use super::duplicate::{backup_paths_text, create_duplicate_backup};
use std::time::{SystemTime, UNIX_EPOCH};

const DELETED_TAGS_DIR: &str = "deleted-tags";

/// Where a deleted loose tag is moved to, under the installation's own storage.
///
/// A tag is deleted far more often by mistake than by intent, and Baboon has no
/// undo that reaches across a restart — so the file is moved somewhere findable
/// rather than unlinked. The timestamped run folder keeps repeated deletions of
/// the same path from colliding.
fn loose_trash_destination(
    game: Option<&str>,
    display_path: &str,
    now_unix_secs: u64,
) -> Result<PathBuf, String> {
    let relative = display_path.replace('\\', "/");
    let relative = relative.trim_start_matches('/');
    if relative.is_empty() {
        return Err("This tag has no path to delete".to_owned());
    }
    if relative.split('/').any(|part| part == "..") {
        return Err("This tag's path is not safe to move".to_owned());
    }
    let mut destination = crate::storage::data_path(DELETED_TAGS_DIR);
    destination.push(game.unwrap_or("unknown-game"));
    destination.push(now_unix_secs.to_string());
    for part in relative.split('/') {
        destination.push(part);
    }
    Ok(destination)
}

/// Drop every per-kit trace of a tag that no longer exists.
///
/// The generation bump comes last, and is what makes the change visible: the
/// browser's memoised filter and the field-value index are both keyed on it, so
/// without it a search would keep answering with a tag that is gone.
fn forget_tag_in_kit(kit: &mut Kit, key: &str) {
    kit.parsed_tags.remove(key);
    kit.loading_tags.remove(key);
    kit.bitmap_previews.remove(key);
    kit.model_previews.remove(key);
    kit.field_search.remove(key);
    kit.field_search_applied.remove(key);
    kit.edit_buffers.forget_tag(key);
    if kit.selected_key.as_deref() == Some(key) {
        kit.selected_key = None;
    }
    if let Some(source) = kit.source.as_mut() {
        source.entries.retain(|entry| entry.key != key);
        source.all_entries.retain(|entry| entry.key != key);
        source.tree = crate::source::build_tree(&source.entries);
        source.group_tree = crate::source::build_group_tree(&source.entries);
        if let Some(index) = source.reverse_dependencies.as_mut() {
            index.clear_tag(key);
        }
    }
    // Keywords are a sidecar keyed by tag key, so they outlive the tag unless
    // they are dropped here.
    kit.keywords.forget_tag(key);
    kit.keywords.save_if_dirty();
    kit.generation = kit.generation.wrapping_add(1);
    kit.field_index.invalidate();
}

/// Move a deleted tag out of the way, without ever destroying what is already
/// at the destination.
fn move_to_trash(path: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err(format!(
            "{} already exists; move it before deleting this tag again",
            destination.display()
        ));
    }
    // Rename first: on the same volume it is atomic and never touches the
    // bytes. A kit on a different drive from the storage root needs the copy,
    // and only once that has succeeded may the original go.
    if fs::rename(path, destination).is_ok() {
        return Ok(());
    }
    fs::copy(path, destination).map_err(|error| {
        format!(
            "Could not move {} to {}: {error}",
            path.display(),
            destination.display()
        )
    })?;
    fs::remove_file(path).map_err(|error| {
        let _ = fs::remove_file(destination);
        format!("Could not remove {}: {error}", path.display())
    })
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// What a container delete needs to know about one tag: where its chunks live,
/// and the evidence that Baboon is the one that put them there.
#[derive(Clone, Debug)]
pub(in crate::app) struct ContainerDeleteTarget {
    pub(in crate::app) package_path: String,
    pub(in crate::app) uasset_path: String,
    pub(in crate::app) ubulk_path: String,
    pub(in crate::app) minimum_appended_index: Option<u32>,
}

/// The chunk index above which each mounted container's contents were appended
/// by Baboon, resolved once per container because it reads the directory.
pub(in crate::app) fn container_appended_thresholds(
    containers: &[crate::source::MountedContainer],
) -> Vec<Option<u32>> {
    containers
        .iter()
        .map(|container| created_tags::container_original_entry_count(&container.utoc_path))
        .collect()
}

/// Resolve a container tag for deletion, or say why it cannot be.
///
/// Provenance comes from either of two independent records, both Baboon's own:
/// the duplicate ledger, or the immutable backup left beside the container
/// before it was first written to. The backup covers copies made before the
/// ledger existed and survives the ledger being lost or moved between machines;
/// the ledger covers containers whose backups have been cleaned up. Neither is
/// derived from the container itself, because a copy in a pak is
/// indistinguishable from a tag the game shipped.
pub(in crate::app) fn resolve_container_delete_target(
    containers: &[crate::source::MountedContainer],
    thresholds: &[Option<u32>],
    container: usize,
    rel_path: &str,
    ledger: &CreatedTagLedger,
) -> Result<ContainerDeleteTarget, String> {
    let target = containers
        .get(container)
        .ok_or("This tag's container is no longer mounted")?;
    if let Some(record) = ledger.find(&target.utoc_path, rel_path) {
        return Ok(ContainerDeleteTarget {
            package_path: record.package_path.clone(),
            uasset_path: record.uasset_path.clone(),
            ubulk_path: record.ubulk_path.clone(),
            minimum_appended_index: Some(record.container_entry_count_before),
        });
    }
    let threshold = thresholds
        .get(container)
        .copied()
        .flatten()
        .ok_or("Baboon has no record of creating any tag in this container")?;
    let index = target
        .archive
        .chunk_index_for(rel_path)
        .map_err(|_| "This tag is no longer in its container".to_owned())?;
    if index < threshold {
        return Err("Only tags duplicated by Baboon can be deleted".to_owned());
    }
    let uasset_path = rel_path
        .strip_suffix(".ubulk")
        .map(|stem| format!("{stem}.uasset"))
        .ok_or("This tag's container path is not a .ubulk")?;
    let package_path = super::container_rel_to_package_path(rel_path)
        .ok_or("This tag's container path has no package name")?;
    Ok(ContainerDeleteTarget {
        package_path,
        uasset_path,
        ubulk_path: rel_path.to_owned(),
        minimum_appended_index: Some(threshold),
    })
}

/// Whether an entry can be deleted at all, and why not when it cannot.
pub(in crate::app) fn delete_eligibility(
    entry: &TagEntry,
    containers: &[crate::source::MountedContainer],
    thresholds: &[Option<u32>],
    ledger: &CreatedTagLedger,
) -> Result<(), String> {
    match &entry.location {
        TagEntryLocation::LooseFile(path) => {
            if path.exists() {
                Ok(())
            } else {
                Err("This tag's file is no longer on disk".to_owned())
            }
        }
        TagEntryLocation::Container {
            container,
            rel_path,
        } => resolve_container_delete_target(containers, thresholds, *container, rel_path, ledger)
            .map(|_| ()),
        TagEntryLocation::NewContainer { .. } => {
            Err("Discard this unsaved tag instead of deleting it".to_owned())
        }
        TagEntryLocation::Monolithic { .. } => {
            Err("Monolithic cache tags are read-only".to_owned())
        }
    }
}

/// Browser keys for every tag in `source` that may be deleted.
pub(in crate::app) fn deletable_container_keys(
    source: &LoadedSourceData,
    ledger: &CreatedTagLedger,
) -> HashSet<String> {
    let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
        return HashSet::new();
    };
    let thresholds = container_appended_thresholds(containers);
    if thresholds.iter().all(Option::is_none) && ledger.is_empty() {
        return HashSet::new();
    }
    source
        .entries
        .iter()
        .filter(|entry| {
            matches!(&entry.location, TagEntryLocation::Container { container, rel_path }
                if resolve_container_delete_target(
                    containers,
                    &thresholds,
                    *container,
                    rel_path,
                    ledger,
                )
                .is_ok())
        })
        .map(|entry| entry.key.clone())
        .collect()
}

struct ContainerDeleteWorkerInput {
    root: PathBuf,
    containers: Vec<crate::source::MountedContainer>,
    target_container: usize,
    key: String,
    display_path: String,
    group_tag: u32,
    target: ContainerDeleteTarget,
    target_label: String,
    is_mod: bool,
}

impl Baboon {
    /// Bring a workspace's deletable set up to date, if anything has changed
    /// since it was last resolved.
    pub(in crate::app) fn refresh_deletable_keys(&mut self, kit_index: usize) {
        let generation = self.kits[kit_index].generation;
        if self.kits[kit_index].deletable_keys_generation == Some(generation) {
            return;
        }
        let keys = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| deletable_container_keys(source, &self.created_tags))
            .unwrap_or_default();
        self.kits[kit_index].deletable_keys = Arc::new(keys);
        self.kits[kit_index].deletable_keys_generation = Some(generation);
    }

    /// Open the delete confirmation for `key`, resolving everything the dialog
    /// needs to describe exactly what will happen.
    pub(super) fn open_delete_tag(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer in the source".to_owned();
            return;
        };
        let kit = self.active_kit_id();
        if self.container_delete_running.contains(&kit)
            || self.container_duplicate_running.contains(&kit)
        {
            self.status =
                "A Campaign Evolved write is already running for this workspace".to_owned();
            return;
        }
        let containers = self.mounted_containers().unwrap_or_default();
        let thresholds = container_appended_thresholds(&containers);
        if let Err(error) =
            delete_eligibility(&entry, &containers, &thresholds, &self.created_tags)
        {
            self.status = error;
            return;
        }
        let (referrers, referrers_unavailable) = match self.references_to_entry(&entry) {
            Some(list) => (
                list.into_iter().map(|entry| entry.display_path).collect(),
                false,
            ),
            None => (Vec::new(), true),
        };
        let has_unsaved_edits = self.kits[self.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set());
        let kind = match &entry.location {
            TagEntryLocation::Container { container, .. } => {
                let target = containers.get(*container);
                DeleteKind::Container {
                    target_label: target
                        .map(|target| {
                            format!(
                                "{} {} ({})",
                                if target.is_mod {
                                    "mounted mod"
                                } else {
                                    "shipped pak"
                                },
                                target.chunk_label,
                                target.utoc_path.display()
                            )
                        })
                        .unwrap_or_else(|| "an unmounted container".to_owned()),
                }
            }
            _ => DeleteKind::Loose,
        };
        self.delete_confirm = Some(DeleteConfirm {
            kit,
            key: key.to_owned(),
            display_path: entry.display_path.clone(),
            kind,
            referrers,
            referrers_unavailable,
            has_unsaved_edits,
        });
    }

    fn mounted_containers(&self) -> Option<Vec<crate::source::MountedContainer>> {
        match &self.source()?.source {
            TagSource::IoStoreContainerSet { containers, .. } => Some(containers.clone()),
            _ => None,
        }
    }

    /// Apply the confirmed deletion. Loose tags are moved on the spot; container
    /// tags go to a worker, because rewriting a pak's TOC is not a UI-thread job.
    pub(in crate::app) fn begin_delete_tag(&mut self, ctx: egui::Context) {
        let Some(confirm) = self.delete_confirm.take() else {
            return;
        };
        if !self.focus_navigation_kit(confirm.kit) {
            self.status = "The workspace this delete came from is closed".to_owned();
            return;
        }
        // Re-resolved after the dialog, not carried through it: a modeless
        // confirmation outlives the frame that opened it, and the source can
        // have moved on underneath.
        let Some(entry) = self.entry_for_key(&confirm.key).cloned() else {
            self.status = "Tag is no longer in the source".to_owned();
            return;
        };
        let containers = self.mounted_containers().unwrap_or_default();
        let thresholds = container_appended_thresholds(&containers);
        if let Err(error) =
            delete_eligibility(&entry, &containers, &thresholds, &self.created_tags)
        {
            self.status = error;
            return;
        }
        match &entry.location {
            TagEntryLocation::LooseFile(path) => {
                let path = path.clone();
                match self.delete_loose_tag(&entry, &path) {
                    Ok(destination) => {
                        self.status = format!(
                            "Deleted {} — moved to {}",
                            entry.display_path,
                            destination.display()
                        );
                    }
                    Err(error) => self.status = error,
                }
            }
            TagEntryLocation::Container {
                container,
                rel_path,
            } => self.start_container_delete(&entry, *container, rel_path.clone(), ctx),
            _ => self.status = "This tag cannot be deleted".to_owned(),
        }
    }

    fn delete_loose_tag(&mut self, entry: &TagEntry, path: &Path) -> Result<PathBuf, String> {
        let game = self.source().and_then(|source| source.game.clone());
        let destination =
            loose_trash_destination(game.as_deref(), &entry.display_path, now_unix_secs())?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        move_to_trash(path, &destination)?;
        self.forget_deleted_tag(self.active, &entry.key);
        if let Some(source) = self.source_mut() {
            if let TagSource::LooseFolder { root, .. } = &source.source
                && let Ok(tree) = crate::source::build_folder_directory_tree(root)
            {
                source.tree = tree;
            }
            let complete_index = !source.all_entries.is_empty();
            source.group_tree = crate::source::build_group_tree(if complete_index {
                &source.all_entries
            } else {
                &source.entries
            });
            if complete_index
                && let TagSource::LooseFolder { root, .. } = &source.source
                && let Some(game) = source.game.as_deref()
            {
                let _ = crate::source::save_entry_index(game, root, &source.all_entries);
            }
        }
        Ok(destination)
    }

    fn start_container_delete(
        &mut self,
        entry: &TagEntry,
        target_container: usize,
        rel_path: String,
        ctx: egui::Context,
    ) {
        let kit = self.active_kit_id();
        let (root, containers, target_label, is_mod) = {
            let Some(source) = self.source() else {
                self.status = "No source is loaded".to_owned();
                return;
            };
            let TagSource::IoStoreContainerSet {
                root, containers, ..
            } = &source.source
            else {
                self.status = "Source is not a Campaign Evolved container source".to_owned();
                return;
            };
            let Some(target) = containers.get(target_container) else {
                self.status = "Container provenance is stale".to_owned();
                return;
            };
            (
                root.clone(),
                containers.clone(),
                target.chunk_label.clone(),
                target.is_mod,
            )
        };
        let thresholds = container_appended_thresholds(&containers);
        let target = match resolve_container_delete_target(
            &containers,
            &thresholds,
            target_container,
            &rel_path,
            &self.created_tags,
        ) {
            Ok(target) => target,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        self.container_delete_running.insert(kit);
        self.status = format!("Deleting {} from {target_label}…", entry.display_path);
        let input = ContainerDeleteWorkerInput {
            root,
            containers,
            target_container,
            key: entry.key.clone(),
            display_path: entry.display_path.clone(),
            group_tag: entry.group_tag,
            target,
            target_label,
            is_mod,
        };
        let stamp = KitStamp {
            kit,
            generation: self.kits[self.active].generation,
        };
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = run_container_delete(input);
            let _ = tx.send(WorkerMessage::ContainerDeleteFinished { stamp, result });
            ctx.request_repaint();
        });
    }

    pub(in crate::app) fn handle_container_delete_finished(
        &mut self,
        stamp: KitStamp,
        result: Result<ContainerDeleteResult, String>,
    ) -> bool {
        let kit_index = self.kit_index(stamp.kit);
        self.container_delete_running.remove(&stamp.kit);
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.operation_notice = Some(OperationNotice {
                    title: "Delete failed".to_owned(),
                    message: error.clone(),
                    failed: true,
                });
                self.status = error;
                return false;
            }
        };
        // Recorded even if the workspace is gone: the bytes are out of the
        // container either way, and a ledger row pointing at a tag that no
        // longer exists would keep offering to delete it.
        self.created_tags
            .forget(&result.target_utoc, &result.ubulk_path);
        let ledger_error = self.created_tags.save().err();
        let Some(kit_index) = kit_index else {
            return true;
        };
        let Some(target_container) = super::duplicate::container_index_for_utoc(
            self.kits[kit_index].source.as_ref(),
            result.target_container,
            &result.target_utoc,
        ) else {
            self.status = format!(
                "Deleted {} from {}, but this workspace no longer has that container mounted — \
                 reload the source. Backup: {}",
                result.display_path,
                result.target_label,
                backup_paths_text(&result.backup)
            );
            return false;
        };
        let mut result = result;
        result.target_container = target_container;
        {
            let Some(source) = self.kits[kit_index].source.as_mut() else {
                self.status = "Delete completed after its source was unloaded".to_owned();
                return false;
            };
            let TagSource::IoStoreContainerSet {
                containers,
                index,
                packages,
                shipped,
                ..
            } = &mut source.source
            else {
                self.status = "Delete completed against a non-container source".to_owned();
                return false;
            };
            let Some(target) = containers.get_mut(result.target_container) else {
                self.status = "Delete completed with stale container provenance".to_owned();
                return false;
            };
            target.archive = result.archive;
            // Mirrors the duplicate side so an add and a remove address the
            // same row of the index.
            if let Some(key) = super::duplicate::container_duplicate_index_key(
                result.group_tag,
                &result.ubulk_path,
            ) {
                Arc::make_mut(index).remove(&key);
            }
            Arc::make_mut(packages).remove(&result.package.to_ascii_lowercase());
            if !result.is_mod {
                Arc::make_mut(shipped).remove(&result.ubulk_path);
            }
        }
        self.forget_deleted_tag(kit_index, &result.key);
        self.operation_notice = Some(OperationNotice {
            title: "Tag deleted".to_owned(),
            message: format!(
                "{}\n\nRemoved from {}.\nThe UTOC changed; the retired bytes stay in the UCAS as \
                 dead space and the sibling PAK did not change.\nBackup: {}",
                result.display_path,
                result.target_label,
                backup_paths_text(&result.backup)
            ),
            failed: false,
        });
        self.status = match ledger_error {
            Some(error) => format!(
                "Deleted {} from {} (UTOC changed; UCAS bytes retained), but the duplicate \
                 record could not be updated ({error}). Backup: {}",
                result.display_path,
                result.target_label,
                backup_paths_text(&result.backup)
            ),
            None => format!(
                "Deleted {} from {} (UTOC changed; UCAS bytes retained). Backup: {}",
                result.display_path,
                result.target_label,
                backup_paths_text(&result.backup)
            ),
        };
        false
    }

    /// Drop every trace of a tag that no longer exists, for either source kind.
    ///
    /// Ordering matters: the pane closes first so the tab layout is repaired
    /// while the key is still meaningful, and the generation bump comes last so
    /// the memoised browser filter and the field index both rebuild against the
    /// state that is left.
    fn forget_deleted_tag(&mut self, kit_index: usize, key: &str) {
        // First, while the entry is still resolvable: a stashed overlay for a
        // tag that no longer exists would resurrect it on the next project load.
        self.forget_campaign_overlay(kit_index, key);
        self.kits[kit_index].close_tag_pane(key);
        forget_tag_in_kit(&mut self.kits[kit_index], key);
        // Navigation state names tags by key, and this key now names nothing.
        if self
            .reveal_target
            .as_ref()
            .is_some_and(|reveal| reveal.key == key)
        {
            self.reveal_target = None;
        }
    }
}

fn run_container_delete(
    input: ContainerDeleteWorkerInput,
) -> Result<ContainerDeleteResult, String> {
    let target = input
        .containers
        .get(input.target_container)
        .ok_or("Container provenance is stale")?;
    let backup = create_duplicate_backup(&target.utoc_path)?;
    let archive = target.archive.clone();
    let request = blam_tags::iostore::writer::InPlaceTagDeletion {
        package_path: &input.target.package_path,
        // The container records nothing about who wrote a chunk, so this is what
        // keeps the operation off the game's own tags: anything below the count
        // the container had before Baboon first appended to it is not ours.
        minimum_appended_index: input.target.minimum_appended_index,
        expected_uasset_path: Some(&input.target.uasset_path),
        expected_ubulk_path: Some(&input.target.ubulk_path),
    };
    if let Err(error) =
        blam_tags::iostore::writer::delete_tag_in_place_with(&archive, &target.utoc_path, &request)
    {
        return Err(format!(
            "Delete from {} failed: {error}. The container was restored; backup kept at {}",
            input.target_label,
            backup_paths_text(&backup)
        ));
    }
    let reopened = crate::source::reopen_container_archive(
        &input.root,
        &input.containers,
        input.target_container,
    )
    .map_err(|error| {
        format!(
            "Delete rewrote {}, but reopening failed: {error}. Backup kept at {}",
            input.target_label,
            backup_paths_text(&backup)
        )
    })?;
    Ok(ContainerDeleteResult {
        key: input.key,
        target_container: input.target_container,
        target_utoc: target.utoc_path.clone(),
        archive: Arc::new(reopened),
        display_path: input.display_path,
        group_tag: input.group_tag,
        package: input.target.package_path,
        uasset_path: input.target.uasset_path,
        ubulk_path: input.target.ubulk_path,
        target_label: input.target_label,
        is_mod: input.is_mod,
        backup,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger_with(utoc: &str, ubulk: &str) -> CreatedTagLedger {
        let mut ledger = CreatedTagLedger::default();
        ledger.record(CreatedTagRecord {
            utoc_path: utoc.to_owned(),
            chunk_label: "pakchunk240-WinGDK".to_owned(),
            package_path: "/Game/Tags/objects/copy-biped".to_owned(),
            package_id: 7,
            uasset_path: "Meteorite/Content/Tags/objects/copy-biped.uasset".to_owned(),
            ubulk_path: ubulk.to_owned(),
            display_path: "objects/copy.biped".to_owned(),
            group_tag: 0,
            source_display: "objects/original.biped".to_owned(),
            container_entry_count_before: 12,
            created_unix_secs: 1,
        });
        ledger
    }

    fn container_entry(container: usize, rel_path: &str) -> TagEntry {
        TagEntry {
            key: format!("ublock:pakchunk240-WinGDK:{rel_path}"),
            display_path: "objects/copy.biped".to_owned(),
            group_tag: 0,
            group_name: None,
            location: TagEntryLocation::Container {
                container,
                rel_path: rel_path.to_owned(),
            },
        }
    }

    #[test]
    fn only_recorded_container_copies_are_deletable() {
        let ubulk = "Meteorite/Content/Tags/objects/copy-biped.ubulk";
        let ledger = ledger_with("C:/Game/Paks/pakchunk240-WinGDK.utoc", ubulk);
        let empty = CreatedTagLedger::default();

        // Without a live container list there is nothing to check a record
        // against, so nothing is deletable.
        assert!(delete_eligibility(&container_entry(0, ubulk), &[], &[], &ledger).is_err());
        assert!(delete_eligibility(&container_entry(0, ubulk), &[], &[], &empty).is_err());
    }

    #[test]
    fn unsaved_and_read_only_entries_are_never_deletable() {
        let ledger = CreatedTagLedger::default();
        let new_container = TagEntry {
            key: "new:1".to_owned(),
            display_path: "objects/fresh.biped".to_owned(),
            group_tag: 0,
            group_name: None,
            location: TagEntryLocation::NewContainer {
                template_container: 0,
                template_rel: "Tags/template-biped.uasset".to_owned(),
                package: "/Game/Tags/objects/fresh-biped".to_owned(),
                group_tag: 0,
            },
        };
        let monolithic = TagEntry {
            key: "cache:bipd:objects/cached".to_owned(),
            display_path: "objects/cached.biped".to_owned(),
            group_tag: 0,
            group_name: None,
            location: TagEntryLocation::Monolithic {
                name: "objects/cached".to_owned(),
                group_tag: 0,
            },
        };
        assert!(delete_eligibility(&new_container, &[], &[], &ledger).is_err());
        assert!(delete_eligibility(&monolithic, &[], &[], &ledger).is_err());
    }

    #[test]
    fn a_missing_loose_file_is_not_deletable() {
        let ledger = CreatedTagLedger::default();
        let entry = TagEntry {
            key: "file:missing".to_owned(),
            display_path: "objects/missing.biped".to_owned(),
            group_tag: 0,
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(
                "C:/definitely/not/here/missing.biped",
            )),
        };
        assert!(delete_eligibility(&entry, &[], &[], &ledger).is_err());
    }

    #[test]
    fn forgetting_a_tag_clears_its_kit_state_and_bumps_the_generation() {
        let mut kit = Kit::empty(KitId(7), TagNameIndex::default());
        let entry = container_entry(0, "Meteorite/Content/Tags/objects/copy-biped.ubulk");
        let key = entry.key.clone();
        let other = TagEntry {
            key: "keep".to_owned(),
            display_path: "objects/keep.biped".to_owned(),
            ..container_entry(0, "Meteorite/Content/Tags/objects/keep-biped.ubulk")
        };
        let entries = vec![entry.clone(), other.clone()];
        kit.source = Some(LoadedSourceData {
            label: "delete test".to_owned(),
            source: TagSource::IoStoreContainerSet {
                root: PathBuf::from("C:/Game/Paks"),
                containers: Vec::new(),
                index: Arc::new(crate::source::ContainerTagIndex::default()),
                packages: Arc::new(crate::source::ContainerPackageIndex::default()),
                shipped: Arc::new(crate::source::ShippedTagIndex::default()),
            },
            names: TagNameIndex::default(),
            game: None,
            tree: crate::source::build_tree(&entries),
            group_tree: crate::source::build_group_tree(&entries),
            entries,
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
        });
        kit.selected_key = Some(key.clone());
        let generation_before = kit.generation;

        forget_tag_in_kit(&mut kit, &key);

        let source = kit.source.as_ref().unwrap();
        assert_eq!(source.entries.len(), 1);
        assert_eq!(source.entries[0].key, other.key);
        assert!(!kit.parsed_tags.contains_key(&key));
        assert_eq!(kit.selected_key, None);
        assert_ne!(
            kit.generation, generation_before,
            "the browser filter and field index only rebuild on a new generation"
        );
        assert!(
            !source
                .tree
                .children
                .iter()
                .flat_map(|node| node.entries.iter())
                .any(|&index| source.entries.get(index).is_none()),
            "the rebuilt tree must not address entries that no longer exist"
        );
    }

    #[test]
    fn moving_a_deleted_tag_never_overwrites_what_is_already_there() {
        let root = std::env::temp_dir().join(format!(
            "baboon-delete-trash-{}-{}",
            std::process::id(),
            now_unix_secs()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("tag.biped");
        let destination = root.join("moved.biped");
        fs::write(&source, b"tag bytes").unwrap();

        move_to_trash(&source, &destination).unwrap();
        assert!(!source.exists(), "the original is gone once it has moved");
        assert_eq!(fs::read(&destination).unwrap(), b"tag bytes");

        fs::write(&source, b"second tag").unwrap();
        assert!(move_to_trash(&source, &destination).is_err());
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"tag bytes",
            "an occupied destination keeps its own bytes"
        );
        assert!(source.exists(), "a refused move leaves the tag in place");

        let _ = fs::remove_file(&source);
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_dir(&root);
    }

    #[test]
    fn loose_trash_keeps_the_tag_path_and_refuses_to_escape_it() {
        let destination =
            loose_trash_destination(Some("haloce_evolved"), "objects\\weapons\\rifle.weapon", 99)
                .unwrap();
        let tail: Vec<String> = destination
            .components()
            .rev()
            .take(5)
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            tail,
            vec![
                "rifle.weapon".to_owned(),
                "weapons".to_owned(),
                "objects".to_owned(),
                "99".to_owned(),
                "haloce_evolved".to_owned(),
            ]
        );
        assert!(loose_trash_destination(None, "../../escape.weapon", 1).is_err());
        assert!(loose_trash_destination(None, "", 1).is_err());
    }
}
