//! Saving a Campaign Evolved tag by overwriting its container in place, as a
//! job that holds the container's write lease.

use super::*;
use crate::app::tag_ops::new_tag::new_container_template_bytes;
use crate::app::tag_ops::new_tag::container_rel_to_package_path;

impl Baboon {
    pub(in crate::app) fn current_source_is_container(&self) -> bool {
        matches!(
            self.source().map(|s| &s.source),
            Some(TagSource::IoStoreContainerSet { .. })
        )
    }

    /// Export a container tag as a higher-priority override container. The base
    /// game is never modified.
    /// - `rename_to == None`: same-name override (Save) — replaces this tag's
    ///   chunk(s), with the `.uasset` SerialSize patched on a size change.
    /// - `Some((new_rel, redirect))`: a new tag at `/Game/Tags/<new_rel>-<group>`
    ///   (Save As / Rename); `redirect` adds an old→new package redirect so
    ///   existing references resolve to the renamed tag.
    /// Returns the output path, or `None` if the save dialog was cancelled.
    pub(in crate::app) fn export_container_override(
        &mut self,
        key: &str,
        rename_to: Option<(String, bool)>,
    ) -> Result<Option<PathBuf>, String> {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return Err("Tag is no longer in the source".to_owned());
        };
        let TagEntryLocation::Container {
            container,
            rel_path,
        } = &entry.location
        else {
            return Err("Not a Campaign Evolved container tag".to_owned());
        };
        let Some(source) = self.source() else {
            return Err("No source loaded".to_owned());
        };
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return Err("Source is not a container".to_owned());
        };
        let archive = containers
            .get(*container)
            .ok_or("container provenance is stale")?
            .archive
            .clone();
        let rel_path = rel_path.clone();
        let group = entry.group_name.clone().unwrap_or_default();
        // What the `.uasset` about to be reused is *to* this tag. It is read
        // back out of this tag's own package below, so its bindings are this
        // tag's bindings -- `wrapper_origin_for` is the one place that call is
        // made, so a Save As cannot start disagreeing with an Export Mod.
        let wrapper_origin =
            wrapper_origin_for(&entry.location).ok_or("Not a Campaign Evolved container tag")?;

        // Tag content: current edited bytes if the tag is loaded, else the
        // original `.ubulk`.
        let tag_bytes = if let Some(doc) = self.model.kits[self.model.active].parsed_tags.get(key) {
            doc.tag
                .write_to_bytes()
                .map_err(|e| format!("serialize tag: {e}"))?
        } else {
            archive
                .read(&rel_path)
                .map_err(|e| format!("read tag: {e}"))?
        };

        match rename_to {
            None => {
                let stem = rel_path
                    .rsplit('/')
                    .next()
                    .and_then(|f| f.strip_suffix(".ubulk"))
                    .unwrap_or("tag");
                let Some(output) = pick_override_utoc(&format!("{stem}_P.utoc")) else {
                    return Ok(None);
                };
                ensure_mod_output_dir(&output)?;
                blam_tags::iostore::writer::write_tag_override(
                    &archive, &rel_path, &tag_bytes, &output,
                )
                .map_err(|e| format!("write override: {e}"))?;
                Ok(Some(output))
            }
            Some((new_rel, redirect)) => {
                let ua_path = rel_path
                    .strip_suffix(".ubulk")
                    .map(|s| format!("{s}.uasset"))
                    .ok_or("source is not a .ubulk")?;
                let template = archive
                    .read(&ua_path)
                    .map_err(|e| format!("read template .uasset: {e}"))?;
                let old_pkg = container_rel_to_package_path(&rel_path)
                    .ok_or("could not derive source package path")?;
                let new_pkg = format!("/Game/Tags/{new_rel}-{group}");
                let leaf = new_rel.rsplit('/').next().unwrap_or("tag");
                let Some(output) = pick_override_utoc(&format!("{leaf}-{group}_P.utoc")) else {
                    return Ok(None);
                };
                ensure_mod_output_dir(&output)?;
                blam_tags::iostore::writer::write_new_tag_container(
                    &template,
                    &tag_bytes,
                    &new_pkg,
                    if redirect {
                        Some(old_pkg.as_str())
                    } else {
                        None
                    },
                    wrapper_origin,
                    &output,
                )
                .map_err(|e| format!("write container: {e}"))?;
                Ok(Some(output))
            }
        }
    }

    /// Overwrite the current container tag inside its own pak, in place, and
    /// wait for it. **Destructive** — modifies the shipped game files.
    ///
    /// For the close prompt's Save, which reads the result off the document's
    /// dirty flag before it lets the app or workspace close. Everything else
    /// uses [`Self::begin_overwrite_current_tag_in_place`].
    pub(in crate::app) fn overwrite_current_tag_in_place(&mut self, key: &str) {
        let Some((job, lease)) = self.prepare_in_place_overwrite(key) else {
            return;
        };
        let written = run_in_place_overwrite(&job);
        self.release_in_place_lease(lease, written.outcome());
        self.finish_in_place_overwrite(job, written);
    }

    /// The same, with the write and the pak reopen on a worker. Only reached
    /// after the user confirms the overwrite, or has turned that off.
    pub(in crate::app) fn begin_overwrite_current_tag_in_place(&mut self, key: &str, ctx: &egui::Context) {
        let Some((job, lease)) = self.prepare_in_place_overwrite(key) else {
            return;
        };
        let lease = self.park_container_write_lease(lease);
        self.model.status = format!("Saving into {}…", job.utoc_path.display());
        let panic_job = job.clone();
        spawn_worker(
            &self.tx,
            ctx,
            move || {
                let written = run_in_place_overwrite(&job);
                WorkerMessage::InPlaceOverwriteFinished {
                    job: Box::new(job),
                    lease,
                    written,
                }
            },
            move |error| WorkerMessage::InPlaceOverwriteFinished {
                job: Box::new(panic_job),
                lease,
                // A panic may have come after the write: remount rather than
                // trust the TOCs.
                written: InPlaceOverwrite {
                    write: Err(error),
                    reopened: None,
                    touched: true,
                },
            },
        );
    }

    /// Applies `WorkerMessage::InPlaceOverwriteFinished`.
    pub(in crate::app) fn handle_in_place_overwrite_finished(
        &mut self,
        job: InPlaceOverwriteJob,
        lease: ContainerLeaseId,
        written: InPlaceOverwrite,
    ) -> bool {
        if let Some(lease) = self.take_container_write_lease(lease) {
            self.release_in_place_lease(lease, written.outcome());
        }
        self.finish_in_place_overwrite(job, written);
        false
    }

    /// Everything the in-place overwrite needs from the UI thread: the tag
    /// serialized (a `TagFile` cannot be cloned), the container, and the write
    /// lease Duplicate, Rename and Delete take on the same files.
    pub(in crate::app) fn prepare_in_place_overwrite(
        &mut self,
        key: &str,
    ) -> Option<(InPlaceOverwriteJob, ContainerWriteLease)> {
        if self.refuse_read_only_edit(self.model.active) {
            return None;
        }
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.model.status = "Tag is no longer in the source".to_owned();
            return None;
        };
        let TagEntryLocation::Container {
            container,
            rel_path,
        } = &entry.location
        else {
            self.model.status = "Not a Campaign Evolved container tag".to_owned();
            return None;
        };
        let container_idx = *container;
        let rel_path = rel_path.clone();
        let Some(doc) = self.model.kits[self.model.active].parsed_tags.get(key) else {
            self.model.status = "Load the tag before saving".to_owned();
            return None;
        };
        let dirty_revision = doc.dirty.revision();
        let bytes = match doc.tag.write_to_bytes() {
            Ok(b) => b,
            Err(e) => {
                self.model.status = format!("Failed to serialize tag: {e}");
                return None;
            }
        };
        let (root, containers) = {
            let Some(source) = self.source() else {
                self.model.status = "No source loaded".to_owned();
                return None;
            };
            let TagSource::IoStoreContainerSet {
                root, containers, ..
            } = &source.source
            else {
                self.model.status = "Source is not a container".to_owned();
                return None;
            };
            (root.clone(), containers.clone())
        };
        let Some(utoc_path) = containers.get(container_idx).map(|m| m.utoc_path.clone()) else {
            self.model.status = "Container provenance is stale".to_owned();
            return None;
        };
        // The same lease Duplicate, Rename and Delete take: it refuses a second
        // write to this container while one is in flight (from this workspace
        // or another on the same install).
        let lease = match self
            .acquire_container_write_lease(&utoc_path, ContainerWriteMode::AppendInPlace)
        {
            Ok(lease) => lease,
            Err(failure) => {
                self.model.status = failure.to_string();
                return None;
            }
        };
        Some((
            InPlaceOverwriteJob {
                stamp: self.kit_stamp(),
                key: key.to_owned(),
                dirty_revision,
                root,
                containers,
                container_idx,
                utoc_path,
                rel_path,
                bytes,
            },
            lease,
        ))
    }

    /// Install the reopened pak and report. The document is marked clean only
    /// if it was not edited while the write ran: the bytes on disk are the
    /// ones serialized before it started.
    pub(in crate::app) fn finish_in_place_overwrite(&mut self, job: InPlaceOverwriteJob, written: InPlaceOverwrite) {
        if let Err(e) = written.write {
            // A mod exported by an older build carries the tag alone, so there
            // is no `.uasset` chunk to rewrite the declared length into and
            // nothing can be added to a container in place.
            let hint = if e.contains("no paired .uasset") {
                " — export this mod again instead of saving into it"
            } else {
                ""
            };
            self.model.status = format!("Overwrite failed: {e}{hint}");
            return;
        }
        let Some(kit) = self.resolve_stamp(job.stamp) else {
            self.model.status = format!(
                "Saved into {}, but the workspace changed meanwhile; reload it to see the tag",
                job.utoc_path.display()
            );
            return;
        };
        let mut reload_error = None;
        match written.reopened {
            Some(Ok(archive)) => {
                // Only onto the container the write was for.
                if let Some(source) = self.model.kits[kit].source.as_mut()
                    && let TagSource::IoStoreContainerSet { containers, .. } = &mut source.source
                    && let Some(m) = containers.get_mut(job.container_idx)
                    && m.utoc_path == job.utoc_path
                {
                    m.archive = std::sync::Arc::new(archive);
                }
            }
            Some(Err(error)) => reload_error = Some(error),
            None => {}
        }
        if let Some(doc) = self.model.kits[kit].parsed_tags.get_mut(&job.key)
            && doc.dirty.revision() == job.dirty_revision
        {
            doc.dirty.clear();
        }
        self.model.status = match reload_error {
            Some(e) => format!(
                "Saved into {}, but reloading the pak failed: {e}",
                job.utoc_path.display()
            ),
            None => format!(
                "Saved into {} (game files modified)",
                job.utoc_path.display()
            ),
        };
    }

    /// Save a brand-new (in-memory) container tag as a new `_P` override
    /// container. A new tag has no baseline in the paks to overwrite, so this
    /// writes a standalone override package via `write_new_tag_container`,
    /// seeded with a same-group tag's `.uasset` or, for a group the game ships
    /// none of, one derived from the group. The base game is untouched; the
    /// user copies the emitted `.utoc`/`.ucas`/`.pak` into `Paks/`.
    pub(in crate::app) fn save_new_container_tag(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.model.status = "Tag is no longer in the source".to_owned();
            return;
        };
        let TagEntryLocation::NewContainer {
            template,
            package,
            group_tag,
        } = &entry.location
        else {
            self.model.status = "Not a new container tag".to_owned();
            return;
        };
        let Some(doc) = self.model.kits[self.model.active].parsed_tags.get(key) else {
            self.model.status = "Load the tag before saving".to_owned();
            return;
        };
        let bytes = match doc.tag.write_to_bytes() {
            Ok(b) => b,
            Err(e) => {
                self.model.status = format!("Failed to serialize tag: {e}");
                return;
            }
        };
        let template = {
            let Some(source) = self.source() else {
                self.model.status = "No source loaded".to_owned();
                return;
            };
            let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
                self.model.status = "Source is not a container".to_owned();
                return;
            };
            match new_container_template_bytes(
                template,
                containers,
                package,
                bytes.len() as u64,
                || self.find_container_template(*group_tag),
            ) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.model.status = error;
                    return;
                }
            }
        };
        // An authored tag's wrapper came from an unrelated donor, so the
        // writer has to strip the donor's bindings rather than carry them.
        let Some(wrapper_origin) = wrapper_origin_for(&entry.location) else {
            self.model.status = "Not a new container tag".to_owned();
            return;
        };
        let leaf = package.rsplit('/').next().unwrap_or("tag");
        let Some(output) = pick_override_utoc(&format!("{leaf}_P.utoc")) else {
            return;
        };
        if let Err(error) = ensure_mod_output_dir(&output) {
            self.model.status = error;
            return;
        }
        match blam_tags::iostore::writer::write_new_tag_container(
            &template,
            &bytes,
            package,
            None,
            wrapper_origin,
            &output,
        ) {
            Ok(()) => {
                if let Some(doc) = self.model.kits[self.model.active].parsed_tags.get_mut(key) {
                    doc.dirty.clear();
                }
                let stem = output.file_stem().and_then(|s| s.to_str()).unwrap_or("mod");
                self.model.status = format!(
                    "Saved new tag → {stem}.utoc/.ucas/.pak — copy all three into \
                     Meteorite/Content/Paks/ (base game unchanged)"
                );
            }
            Err(e) => self.model.status = format!("Save failed: {e}"),
        }
    }
}

/// Prompt for an override `.utoc` output path, defaulting to `default_name`.
///
/// The chosen path names the mod; where it goes is decided by
/// [`mod_output_path`], so every mod Baboon writes is laid out the same way.
pub(in crate::app) fn pick_override_utoc(default_name: &str) -> Option<PathBuf> {
    let mut output = rfd::FileDialog::new()
        .set_title("Export Override Container")
        .set_file_name(default_name)
        .add_filter("IoStore TOC", &["utoc"])
        .save_file()?;
    if output.extension().is_none() {
        output.set_extension("utoc");
    }
    Some(mod_output_path(ensure_priority_suffix(output)))
}

/// An in-place container overwrite, as the worker needs it.
#[derive(Clone)]
pub(in crate::app) struct InPlaceOverwriteJob {
    stamp: KitStamp,
    key: String,
    /// The document's dirty revision when it was serialized.
    dirty_revision: u64,
    root: PathBuf,
    containers: Vec<crate::core::source::MountedContainer>,
    container_idx: usize,
    utoc_path: PathBuf,
    rel_path: String,
    bytes: Vec<u8>,
}

/// What an in-place overwrite did.
pub(in crate::app) struct InPlaceOverwrite {
    write: Result<(), String>,
    /// The container reopened after a successful write, so later reads see it.
    reopened: Option<Result<blam_tags::iostore::IoStoreArchive, String>>,
    /// Whether the container's files may have changed.
    touched: bool,
}

impl InPlaceOverwrite {
    fn outcome(&self) -> ContainerWriteOutcome {
        if self.touched {
            ContainerWriteOutcome::Committed
        } else {
            ContainerWriteOutcome::Unchanged
        }
    }
}

/// Write the tag into its container and reopen it. No UI state: runs on a
/// worker, or inline for the close prompt.
pub(in crate::app) fn run_in_place_overwrite(job: &InPlaceOverwriteJob) -> InPlaceOverwrite {
    // Resolve against the MOUNTED archive, not a fresh handle: an override
    // container (an exported mod the user then reloaded) ships no directory
    // index, and only the mounted handle has the rebuilt file list that can
    // name `rel_path`.
    let archive = &job.containers[job.container_idx].archive;
    let write = blam_tags::iostore::writer::overwrite_tag_in_place_with(
        archive,
        &job.utoc_path,
        &job.rel_path,
        &job.bytes,
    )
    .map_err(|error| error.to_string());
    let touched = write.is_ok();
    let reopened = touched.then(|| {
        crate::core::source::reopen_container_archive(&job.root, &job.containers, job.container_idx)
            .map_err(|error| error.to_string())
    });
    InPlaceOverwrite {
        write,
        reopened,
        touched,
    }
}

/// What Save does with an edited Campaign Evolved container tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ContainerSaveRoute {
    /// Show what shipping the change looks like. The edit is already carried in
    /// the workspace's stash, so nothing is lost by not writing.
    ExportReview,
    /// Overwrite the tag inside the game's own pak, after confirming.
    ConfirmOverwriteInPlace,
    /// The same overwrite, for a user who has turned the confirmation off.
    OverwriteInPlace,
}

/// Route Save for a container tag.
///
/// Writing back into the game's own paks edits the installed game, which is not
/// how a change should be shipped and is not something a user should reach by
/// pressing Save. It is an expert-mode route; everyone else is sent to the
/// export, which is the supported one.
pub(in crate::app) fn container_save_route(expert_mode: bool, confirm: bool) -> ContainerSaveRoute {
    match (expert_mode, confirm) {
        (false, _) => ContainerSaveRoute::ExportReview,
        (true, true) => ContainerSaveRoute::ConfirmOverwriteInPlace,
        (true, false) => ContainerSaveRoute::OverwriteInPlace,
    }
}

#[cfg(test)]
mod in_place_overwrite_tests;
