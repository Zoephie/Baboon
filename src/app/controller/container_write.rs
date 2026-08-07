//! The write lifecycle for Campaign Evolved IoStore containers: taking a
//! container out of service, unmapping it, swapping its files, and putting it
//! back. Reading and mounting containers belongs to `src/source/`; drawing the
//! outcome belongs to `src/app/ui/`.
//!
//! Windows is the reason this exists. A mounted container's `.ucas` is
//! memory-mapped and the Chimp workspace keeps a live handle on every `.pak`,
//! so a write that *truncates* those files fails at the OS with
//! `ERROR_USER_MAPPED_FILE` (1224) or `ERROR_SHARING_VIOLATION` (32) and
//! nothing connects the error to the mod being installed. Not every write
//! truncates, though: the in-place duplicate/delete writer only appends to the
//! `.ucas` and swaps the `.utoc` by rename, and it *reads through the very
//! mapping* an unmap would take away. So a lease says which kind of write it
//! is, and only the truncating kind unmaps.

use super::*;
use std::path::{Path, PathBuf};

/// What the pending write does to the container's files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ContainerWriteMode {
    /// Export Mod and the Chimp exports: the `.utoc`/`.ucas`/`.pak` are
    /// created fresh, so every mapping and handle on them has to go first.
    Replace,
    /// Duplicate, delete, in-place save: the `.ucas` is appended to and the
    /// `.utoc` is swapped by rename, neither of which Windows blocks on a
    /// mapped file. Nothing is unmapped — the engine's in-place writer reads
    /// chunks through the mapping while it works, so taking it away would
    /// break the write rather than enable it.
    AppendInPlace,
}

/// Whether the leased files ended up different from how they started. Decides
/// whether Chimp has to be remounted: its `World` holds its own parsed copy of
/// every TOC, which a committed write makes stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ContainerWriteOutcome {
    Committed,
    Unchanged,
}

/// Where in the lifecycle something went wrong. Reported verbatim so a bug
/// report says which step failed rather than only that the mod did not install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum LeasePhase {
    /// Taking the container out of service.
    Acquire,
    /// Dropping the mappings and handles on it.
    Unmap,
    /// Building the replacement, before anything of the original is touched.
    Write,
    /// Moving the replacement over the original.
    Swap,
    /// Recording what the write means, after the files are in place.
    Commit,
    /// Putting the original back after a failure.
    Rollback,
    /// Reopening the container and the Chimp workspace over it.
    Remount,
}

impl LeasePhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Acquire => "taking the container out of service",
            Self::Unmap => "releasing the container",
            Self::Write => "building the replacement",
            Self::Swap => "replacing the files",
            Self::Commit => "committing",
            Self::Rollback => "rolling back",
            Self::Remount => "remounting",
        }
    }
}

/// Something Baboon can name that still holds a container open.
///
/// Every variant here is a fact, not a guess: each is established by looking at
/// state Baboon owns. What cannot be established is counted separately rather
/// than attributed to whichever job happens to be running.
#[derive(Clone, Debug)]
pub(in crate::app) enum ContainerHolder {
    /// The Unreal package workspace, which mounts every container under `Paks`
    /// a second time and keeps an open handle on every `.pak`.
    ChimpWorld { workspace: String, doing: String },
    /// The same container mounted in another open workspace.
    OtherWorkspaceMount { workspace: String },
    /// A tracked background job that snapshots the whole source, and therefore
    /// every mounted archive with it.
    Job {
        workspace: String,
        job: &'static str,
    },
    /// Another container write already in flight against the same files.
    ConcurrentWrite,
}

impl std::fmt::Display for ContainerHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChimpWorld { workspace, doing } => {
                write!(f, "the Unreal package workspace in {workspace} is {doing}")
            }
            Self::OtherWorkspaceMount { workspace } => {
                write!(f, "{workspace} has the same container mounted")
            }
            Self::Job { workspace, job } => write!(f, "{job} is running in {workspace}"),
            Self::ConcurrentWrite => {
                write!(f, "another write to these files has not finished")
            }
        }
    }
}

/// Why a leased write could not proceed, in enough detail to act on.
#[derive(Clone, Debug)]
pub(in crate::app) struct ContainerWriteFailure {
    pub(in crate::app) phase: LeasePhase,
    /// The exact file, not the container's label — the label is what the pak
    /// is called, and the message has to name what is on disk.
    pub(in crate::app) file: PathBuf,
    pub(in crate::app) label: String,
    pub(in crate::app) holders: Vec<ContainerHolder>,
    /// Live references to the container this probe could not attribute to
    /// anything it tracks.
    pub(in crate::app) unattributed: usize,
    /// The OS error, when the phase was an actual file operation.
    pub(in crate::app) io: Option<String>,
}

impl ContainerWriteFailure {
    /// A failure at a file operation, where the OS error is the whole story
    /// and nothing is holding anything.
    pub(in crate::app) fn at(phase: LeasePhase, file: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            phase,
            file: file.to_path_buf(),
            label: lease_label(file),
            holders: Vec::new(),
            unattributed: 0,
            io: Some(error.to_string()),
        }
    }
}

impl std::fmt::Display for ContainerWriteFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({}, {})",
            self.file.display(),
            self.label,
            self.phase.as_str()
        )?;
        if let Some(io) = &self.io {
            write!(f, ": {io}")?;
        }
        if !self.holders.is_empty() {
            let named: Vec<String> = self.holders.iter().map(ToString::to_string).collect();
            write!(f, ": {}", named.join("; "))?;
        }
        match (self.holders.is_empty(), self.unattributed) {
            (_, 0) => {}
            // Nothing named at all. Saying "try again in a moment" here would
            // be inviting a retry loop against a hold that may never clear.
            (true, count) => write!(
                f,
                ": {count} background reader(s) still hold it, and Baboon cannot say which — \
                 every job it tracks has already finished. Export under a different name, or \
                 reload this workspace and try again."
            )?,
            (false, count) => write!(f, "; {count} further reader(s) it cannot name")?,
        }
        Ok(())
    }
}

/// What putting a container back accomplished. Never an error: by the time this
/// runs the files are already whatever they are going to be, and there is
/// nothing above it that could do better than say so.
#[derive(Default)]
pub(in crate::app) struct ContainerWriteReport {
    /// Containers that could not be reopened, by label and reason. A workspace
    /// carrying one of these must be reloaded before it is edited again.
    pub(in crate::app) reopen_failures: Vec<String>,
    pub(in crate::app) chimp_remounted: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) struct ContainerLeaseId(u64);

/// A container reserved for writing.
///
/// Parked on [`Baboon`] rather than held by the caller whenever the write runs
/// on a worker, because the restore has to happen on the UI thread and the
/// terminal `WorkerMessage` is what brings control back there.
pub(in crate::app) struct ContainerWriteLease {
    id: ContainerLeaseId,
    mode: ContainerWriteMode,
    /// The `.utoc` being written, and by extension its siblings.
    target_utoc: PathBuf,
    /// Which kit and mounted-container slot this lease unmapped. Spans every
    /// open workspace, not just the one writing: two workspaces on the same
    /// install both map the same `.ucas`, and the other one's mapping blocks
    /// the write just as effectively.
    unmapped: Vec<(KitId, usize)>,
    /// Workspaces whose Chimp `World` this lease dropped and must remount.
    chimp_idled: Vec<KitId>,
    /// Cleared by [`Baboon::release_container_write_lease`]. The `Drop` check
    /// below reads it.
    settled: bool,
}

impl ContainerWriteLease {
    /// Whether this lease took a mapping away from a mounted container — which
    /// is the same question as "was the file being written already mounted".
    pub(in crate::app) fn unmapped_any(&self) -> bool {
        !self.unmapped.is_empty()
    }
}

impl Drop for ContainerWriteLease {
    fn drop(&mut self) {
        // Nothing can be restored from here — that needs `&mut Baboon`. Being
        // loud is the next best thing: a lease dropped without release means a
        // mapping never came back and a Chimp workspace is idle for good.
        debug_assert!(
            self.settled,
            "container write lease for {} dropped without release",
            self.target_utoc.display()
        );
        if !self.settled {
            eprintln!(
                "container write lease for {} dropped without release",
                self.target_utoc.display()
            );
        }
    }
}

/// The three files one container is made of. The `.baboon` sidecar is written
/// by the export separately and is never mapped, so it is not part of the swap.
pub(in crate::app) fn container_triplet(path: &Path) -> [PathBuf; 3] {
    [
        path.with_extension("utoc"),
        path.with_extension("ucas"),
        path.with_extension("pak"),
    ]
}

pub(in crate::app) fn remove_container_triplet(path: &Path) {
    for file in container_triplet(path) {
        let _ = fs::remove_file(file);
    }
}

/// Move a finished triplet over an existing one, keeping the original until
/// every file has landed.
///
/// Each existing file is renamed aside before anything is installed, so a
/// failure part-way through puts back exactly what was there rather than
/// leaving a container with two files from the new mod and one from the old —
/// which the game would mount and then fail to read.
pub(in crate::app) fn swap_container_triplet(
    temporary: &Path,
    output: &Path,
) -> Result<(), ContainerWriteFailure> {
    let incoming = container_triplet(temporary);
    let targets = container_triplet(output);
    let backups = [
        output.with_extension("utoc.previous"),
        output.with_extension("ucas.previous"),
        output.with_extension("pak.previous"),
    ];
    // Putting the originals back. A rollback that itself fails is the one
    // outcome the user has to hear about in its own words: the container is
    // left made of files from two different mods, which the game will mount and
    // then fail to read.
    let roll_back = |done: &[usize]| -> Option<ContainerWriteFailure> {
        let mut stuck = Vec::new();
        for &index in done.iter().rev() {
            if let Err(error) = fs::rename(&backups[index], &targets[index]) {
                stuck.push(format!("{}: {error}", targets[index].display()));
            }
        }
        (!stuck.is_empty()).then(|| {
            ContainerWriteFailure::at(LeasePhase::Rollback, output, stuck.join("; "))
        })
    };

    let mut backed_up = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        if !target.exists() {
            continue;
        }
        let _ = fs::remove_file(&backups[index]);
        if let Err(error) = fs::rename(target, &backups[index]) {
            let failure = roll_back(&backed_up)
                .unwrap_or_else(|| ContainerWriteFailure::at(LeasePhase::Swap, target, error));
            remove_container_triplet(temporary);
            return Err(failure);
        }
        backed_up.push(index);
    }
    let mut installed = Vec::new();
    for index in 0..incoming.len() {
        if let Err(error) = fs::rename(&incoming[index], &targets[index]) {
            for &done in installed.iter().rev() {
                let _ = fs::remove_file(&targets[done]);
            }
            let failure = roll_back(&backed_up).unwrap_or_else(|| {
                ContainerWriteFailure::at(LeasePhase::Swap, &targets[index], error)
            });
            remove_container_triplet(temporary);
            return Err(failure);
        }
        installed.push(index);
    }
    for index in backed_up {
        let _ = fs::remove_file(&backups[index]);
    }
    Ok(())
}

/// Where a replacement container is built before it takes the place of the one
/// on disk.
///
/// A *subdirectory* of the output's own folder, not a differently-named file
/// beside it, for two reasons. The container id the writer stamps into the TOC
/// is CityHash64 of the file stem, so a staging name of `mymod_P.building`
/// would ship a container declaring itself under a name nothing else uses —
/// keeping the stem identical keeps the id right. And staying inside the same
/// folder keeps the final move a same-volume rename rather than a copy.
pub(in crate::app) fn staging_utoc_for(output: &Path) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let file_name = output.file_name().unwrap_or_else(|| "mod.utoc".as_ref());
    output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".baboon-export-{}-{nonce}", std::process::id()))
        .join(file_name)
}

/// Remove a staging container and the directory [`staging_utoc_for`] made for
/// it, leaving nothing behind whether the write succeeded or failed.
pub(in crate::app) fn discard_staging(staging_utoc: &Path) {
    remove_container_triplet(staging_utoc);
    if let Some(directory) = staging_utoc.parent() {
        let _ = fs::remove_dir(directory);
    }
}

impl Baboon {
    /// Reserve a container for writing.
    ///
    /// Refuses a second concurrent write to the same files, and for a
    /// truncating write drops every Chimp `World` whose install covers the
    /// target — that is what closes the `.pak` handles. Nothing is unmapped
    /// yet: the caller still has reads to do through the very archives a write
    /// is about to replace.
    pub(in crate::app) fn acquire_container_write_lease(
        &mut self,
        target_utoc: &Path,
        mode: ContainerWriteMode,
    ) -> Result<ContainerWriteLease, ContainerWriteFailure> {
        let target_utoc = target_utoc.to_path_buf();
        if self
            .container_write_leases
            .values()
            .any(|lease| lease.target_utoc == target_utoc)
        {
            return Err(ContainerWriteFailure {
                phase: LeasePhase::Acquire,
                file: target_utoc.clone(),
                label: lease_label(&target_utoc),
                holders: vec![ContainerHolder::ConcurrentWrite],
                unattributed: 0,
                io: None,
            });
        }
        let id = ContainerLeaseId(self.next_container_lease);
        self.next_container_lease = self.next_container_lease.wrapping_add(1);
        let mut lease = ContainerWriteLease {
            id,
            mode,
            target_utoc: target_utoc.clone(),
            unmapped: Vec::new(),
            chimp_idled: Vec::new(),
            settled: false,
        };
        if mode == ContainerWriteMode::AppendInPlace {
            // The `.utoc` is still swapped by rename, so Chimp's parsed copy of
            // it goes stale — but nothing has to be released to let the write
            // happen, and idling the workspace for it would be gratuitous. It
            // is remounted on commit instead.
            return Ok(lease);
        }
        for kit_index in 0..self.kits.len() {
            if !self.chimp_covers(kit_index, &target_utoc) {
                continue;
            }
            let kit_id = self.kits[kit_index].id;
            match std::mem::replace(&mut self.kits[kit_index].chimp.mount, ChimpMount::Idle) {
                ChimpMount::Ready(world) => {
                    // Sole ownership is the whole point: a clone in a background
                    // job keeps the `.ucas` mapped and the `.pak` open whatever
                    // this does, so dropping ours would only hide the problem.
                    if std::sync::Arc::strong_count(&world) > 1 {
                        let doing = self.chimp_activity(kit_index);
                        self.kits[kit_index].chimp.mount = ChimpMount::Ready(world);
                        let failure = ContainerWriteFailure {
                            phase: LeasePhase::Acquire,
                            file: target_utoc.clone(),
                            label: lease_label(&target_utoc),
                            holders: vec![ContainerHolder::ChimpWorld {
                                workspace: self.workspace_label(kit_index),
                                doing,
                            }],
                            unattributed: 0,
                            io: None,
                        };
                        lease.settled = true;
                        let _ = self.release_container_write_lease_inner(
                            lease,
                            ContainerWriteOutcome::Unchanged,
                        );
                        return Err(failure);
                    }
                    drop(world);
                    lease.chimp_idled.push(kit_id);
                }
                ChimpMount::Loading => {
                    // A mount worker in flight will `World::open` mid-write and
                    // map every `.ucas` again. Put the state back and refuse.
                    self.kits[kit_index].chimp.mount = ChimpMount::Loading;
                    let failure = ContainerWriteFailure {
                        phase: LeasePhase::Acquire,
                        file: target_utoc.clone(),
                        label: lease_label(&target_utoc),
                        holders: vec![ContainerHolder::ChimpWorld {
                            workspace: self.workspace_label(kit_index),
                            doing: "still mounting".to_owned(),
                        }],
                        unattributed: 0,
                        io: None,
                    };
                    lease.settled = true;
                    let _ = self.release_container_write_lease_inner(
                        lease,
                        ContainerWriteOutcome::Unchanged,
                    );
                    return Err(failure);
                }
                // Idle or failed: nothing held, and nothing to put back.
                other => self.kits[kit_index].chimp.mount = other,
            }
        }
        Ok(lease)
    }

    /// Drop every `.ucas` mapping Baboon holds on the leased container, across
    /// every open workspace. A no-op for an appending write.
    ///
    /// Call only once every archive read the write needs has already happened:
    /// a released partition refuses every payload read until it is restored.
    pub(in crate::app) fn unmap_leased_containers(
        &mut self,
        lease: &mut ContainerWriteLease,
    ) -> Result<(), ContainerWriteFailure> {
        if lease.mode == ContainerWriteMode::AppendInPlace {
            return Ok(());
        }
        let target_utoc = lease.target_utoc.clone();
        for kit_index in 0..self.kits.len() {
            let Some(source) = self.kits[kit_index].source.as_ref() else {
                continue;
            };
            let targets = crate::source::mounted_containers_at(&source.source, &target_utoc);
            if targets.is_empty() {
                continue;
            }
            let kit_id = self.kits[kit_index].id;
            for index in targets {
                let (label, holders, unattributed) = {
                    let Some(source) = self.kits[kit_index].source.as_mut() else {
                        continue;
                    };
                    let TagSource::IoStoreContainerSet { containers, .. } = &mut source.source
                    else {
                        continue;
                    };
                    let Some(mounted) = containers.get_mut(index) else {
                        continue;
                    };
                    let label = mounted.chunk_label.clone();
                    match std::sync::Arc::get_mut(&mut mounted.archive) {
                        Some(archive) => {
                            archive.release_partition();
                            lease.unmapped.push((kit_id, index));
                            continue;
                        }
                        None => {
                            let extra = std::sync::Arc::strong_count(&mounted.archive) - 1;
                            (label, Vec::new(), extra)
                        }
                    }
                };
                let (mut holders, unattributed) =
                    self.probe_container_holders(kit_index, &target_utoc, holders, unattributed);
                holders.extend(self.other_workspace_mounts(kit_index, &target_utoc));
                return Err(ContainerWriteFailure {
                    phase: LeasePhase::Unmap,
                    file: target_utoc.with_extension("ucas"),
                    label,
                    holders,
                    unattributed,
                    io: None,
                });
            }
        }
        Ok(())
    }

    /// Put everything the lease took back: reopen what was unmapped, remount
    /// the Chimp workspaces it idled, and settle the lease.
    pub(in crate::app) fn release_container_write_lease(
        &mut self,
        lease: ContainerWriteLease,
        outcome: ContainerWriteOutcome,
        ctx: &egui::Context,
    ) -> ContainerWriteReport {
        let mut report = self.release_container_write_lease_inner(lease, outcome);
        report.chimp_remounted = self.drain_pending_chimp_remounts(ctx);
        report
    }

    /// Restart every Unreal package workspace a finished write idled.
    ///
    /// Separate from the release because the acquire path can idle one
    /// workspace and then refuse on the next, and it has no `egui::Context` to
    /// remount with. A queue nothing drained would leave that workspace idle
    /// for the rest of the session.
    pub(in crate::app) fn drain_pending_chimp_remounts(&mut self, ctx: &egui::Context) -> usize {
        let remount: Vec<usize> = std::mem::take(&mut self.pending_chimp_remounts)
            .into_iter()
            .filter_map(|kit| self.kit_index(kit))
            .collect();
        for &kit_index in &remount {
            self.begin_chimp_mount(kit_index, ctx.clone());
        }
        remount.len()
    }

    /// The part of the release that needs no `egui::Context`, so the acquire
    /// path can undo itself without threading one through. Chimp remounts are
    /// queued for the caller above rather than started here.
    fn release_container_write_lease_inner(
        &mut self,
        mut lease: ContainerWriteLease,
        outcome: ContainerWriteOutcome,
    ) -> ContainerWriteReport {
        let mut report = ContainerWriteReport::default();
        for (kit_id, index) in std::mem::take(&mut lease.unmapped) {
            let Some(kit_index) = self.kit_index(kit_id) else {
                continue;
            };
            if let Some(failure) = self.reopen_unmapped_container(kit_index, index) {
                report.reopen_failures.push(failure);
            }
        }
        // A write that changed nothing leaves Chimp's parsed TOCs valid, but it
        // was still idled to get the handles out of the way, so it goes back
        // either way. The outcome only decides whether a workspace that was
        // *not* idled needs remounting too.
        let mut remount = std::mem::take(&mut lease.chimp_idled);
        if outcome == ContainerWriteOutcome::Committed
            && lease.mode == ContainerWriteMode::AppendInPlace
        {
            let target = lease.target_utoc.clone();
            for kit_index in 0..self.kits.len() {
                let kit_id = self.kits[kit_index].id;
                if remount.contains(&kit_id) {
                    continue;
                }
                if self.chimp_covers(kit_index, &target)
                    && matches!(self.kits[kit_index].chimp.mount, ChimpMount::Ready(_))
                {
                    self.kits[kit_index].chimp.mount = ChimpMount::Idle;
                    remount.push(kit_id);
                }
            }
        }
        self.pending_chimp_remounts.extend(remount);
        self.container_write_leases.remove(&lease.id);
        lease.settled = true;
        report
    }

    /// Park a lease whose write runs on a worker thread. The terminal
    /// `WorkerMessage` carries the id back so the completion handler can find
    /// it again on the UI thread.
    pub(in crate::app) fn park_container_write_lease(
        &mut self,
        lease: ContainerWriteLease,
    ) -> ContainerLeaseId {
        let id = lease.id;
        self.container_write_leases.insert(id, lease);
        id
    }

    pub(in crate::app) fn take_container_write_lease(
        &mut self,
        id: ContainerLeaseId,
    ) -> Option<ContainerWriteLease> {
        self.container_write_leases.remove(&id)
    }

    /// Release every parked lease belonging to a workspace that has gone away,
    /// so a lease can never outlive the kit it was taken for.
    pub(in crate::app) fn sweep_container_write_leases(&mut self, ctx: &egui::Context) {
        // First, anything an acquire idled before refusing: it never reached a
        // release, so this is the only thing that puts it back.
        self.drain_pending_chimp_remounts(ctx);
        let orphaned: Vec<ContainerLeaseId> = self
            .container_write_leases
            .iter()
            .filter(|(_, lease)| {
                lease
                    .unmapped
                    .iter()
                    .map(|(kit, _)| *kit)
                    .chain(lease.chimp_idled.iter().copied())
                    .any(|kit| self.kit_index(kit).is_none())
            })
            .map(|(id, _)| *id)
            .collect();
        for id in orphaned {
            if let Some(lease) = self.container_write_leases.remove(&id) {
                self.release_container_write_lease(lease, ContainerWriteOutcome::Unchanged, ctx);
            }
        }
    }

    /// Reopen one container that was unmapped, returning a description of the
    /// failure when it could not be.
    ///
    /// The container on disk is a different file now, so its index is reopened
    /// rather than the old one remapped: an override container ships no
    /// directory index, and `reopen_container_archive` is what rebuilds its
    /// file list. Remapping is the fallback, which at least leaves the mount
    /// readable.
    fn reopen_unmapped_container(&mut self, kit_index: usize, index: usize) -> Option<String> {
        let (root, containers) = {
            let source = self.kits.get(kit_index)?.source.as_ref()?;
            let TagSource::IoStoreContainerSet {
                root, containers, ..
            } = &source.source
            else {
                return None;
            };
            (root.clone(), containers.clone())
        };
        let reopened = crate::source::reopen_container_archive(&root, &containers, index);
        let source = self.kits.get_mut(kit_index)?.source.as_mut()?;
        let TagSource::IoStoreContainerSet { containers, .. } = &mut source.source else {
            return None;
        };
        let mounted = containers.get_mut(index)?;
        match reopened {
            Ok(archive) => {
                mounted.archive = std::sync::Arc::new(archive);
                None
            }
            Err(error) => {
                let label = mounted.chunk_label.clone();
                if let Some(archive) = std::sync::Arc::get_mut(&mut mounted.archive)
                    && archive.remap_partition().is_ok()
                {
                    return Some(format!("{label}: {error} (reading the file already there)"));
                }
                Some(format!(
                    "{label}: {error} — this workspace cannot read that container until it is \
                     reloaded"
                ))
            }
        }
    }

    /// Whether this workspace's Chimp mount covers `target_utoc`.
    fn chimp_covers(&self, kit_index: usize, target_utoc: &Path) -> bool {
        if matches!(
            self.kits[kit_index].chimp.mount,
            ChimpMount::Idle | ChimpMount::Failed(_)
        ) {
            return false;
        }
        self.kits[kit_index]
            .source
            .as_ref()
            .is_some_and(|source| target_utoc.starts_with(source.source.root_path()))
    }

    fn workspace_label(&self, kit_index: usize) -> String {
        self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.label.clone())
            .unwrap_or_else(|| "this workspace".to_owned())
    }

    /// Name what still holds a mounted container, and count what cannot be
    /// named.
    ///
    /// The count is exact — it comes from the reference count. The names are
    /// not proof: a tracked job that snapshots the source is listed because it
    /// is the likely holder. Anything left over is reported as a number, since
    /// a wrong name here sends the user to close a window that was never the
    /// problem.
    fn probe_container_holders(
        &self,
        kit_index: usize,
        target_utoc: &Path,
        mut holders: Vec<ContainerHolder>,
        unattributed: usize,
    ) -> (Vec<ContainerHolder>, usize) {
        let workspace = self.workspace_label(kit_index);
        let kit_id = self.kits[kit_index].id;
        let mut remaining = unattributed;
        let tracked: [(&'static str, bool); 7] = [
            ("a bulk tag extraction", self.container_dump_job.is_some()),
            (
                "a tag duplicate",
                self.container_duplicate_running.contains(&kit_id),
            ),
            (
                "a tag delete",
                self.container_delete_running.contains(&kit_id),
            ),
            ("a level export", self.chimp_level_job.is_some()),
            ("a runtime poke", self.poke_direct_running),
            (
                "the field-value index build",
                self.kits[kit_index].field_index.is_building(),
            ),
            ("a tag load", !self.kits[kit_index].loading_tags.is_empty()),
        ];
        for (job, running) in tracked {
            // One reference accounted for per running job, and no further: the
            // count is what is known, and naming more jobs than there are
            // references would be inventing holders.
            if running && remaining > 0 {
                holders.push(ContainerHolder::Job {
                    workspace: workspace.clone(),
                    job,
                });
                remaining -= 1;
            }
        }
        // Chimp holds its own separate mapping and `.pak` handle rather than a
        // clone of this archive, so it never accounts for one of the count.
        if self.chimp_covers(kit_index, target_utoc) {
            holders.push(ContainerHolder::ChimpWorld {
                workspace,
                doing: self.chimp_activity(kit_index),
            });
        }
        (holders, remaining)
    }

    /// Other open workspaces mounting the same container file.
    fn other_workspace_mounts(&self, writing: usize, target_utoc: &Path) -> Vec<ContainerHolder> {
        (0..self.kits.len())
            .filter(|&kit_index| kit_index != writing)
            .filter(|&kit_index| {
                self.kits[kit_index].source.as_ref().is_some_and(|source| {
                    !crate::source::mounted_containers_at(&source.source, target_utoc).is_empty()
                })
            })
            .map(|kit_index| ContainerHolder::OtherWorkspaceMount {
                workspace: self.workspace_label(kit_index),
            })
            .collect()
    }
}

fn lease_label(target_utoc: &Path) -> String {
    target_utoc
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("container")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(holders: Vec<ContainerHolder>, unattributed: usize) -> ContainerWriteFailure {
        ContainerWriteFailure {
            phase: LeasePhase::Unmap,
            file: PathBuf::from("D:/Game/Paks/~mods/mymod_P.ucas"),
            label: "mymod_P".to_owned(),
            holders,
            unattributed,
            io: None,
        }
    }

    #[test]
    fn a_named_holder_is_named_and_nothing_is_invented() {
        let text = failure(
            vec![ContainerHolder::ChimpWorld {
                workspace: "Paks (12 packs)".to_owned(),
                doing: "indexing package types".to_owned(),
            }],
            0,
        )
        .to_string();
        assert!(text.contains("mymod_P.ucas"), "{text}");
        assert!(text.contains("releasing the container"), "{text}");
        assert!(text.contains("indexing package types"), "{text}");
        assert!(!text.contains("cannot say"), "{text}");
    }

    #[test]
    fn an_unattributed_hold_says_so_rather_than_guessing() {
        // The one thing this message must never do is send the user to close
        // something Baboon has no evidence about.
        let text = failure(Vec::new(), 2).to_string();
        assert!(text.contains("2 background reader(s)"), "{text}");
        assert!(text.contains("cannot say which"), "{text}");
        assert!(!text.contains("try again in a moment"), "{text}");
    }

    #[test]
    fn a_partial_attribution_reports_both_halves() {
        let text = failure(
            vec![ContainerHolder::Job {
                workspace: "Paks (12 packs)".to_owned(),
                job: "a bulk tag extraction",
            }],
            2,
        )
        .to_string();
        assert!(text.contains("a bulk tag extraction is running"), "{text}");
        assert!(text.contains("2 further reader(s)"), "{text}");
    }

    #[test]
    fn a_concurrent_write_is_refused_by_name() {
        let text = ContainerWriteFailure {
            phase: LeasePhase::Acquire,
            file: PathBuf::from("D:/Game/Paks/~mods/mymod_P.utoc"),
            label: "mymod_P".to_owned(),
            holders: vec![ContainerHolder::ConcurrentWrite],
            unattributed: 0,
            io: None,
        }
        .to_string();
        assert!(text.contains("another write to these files"), "{text}");
    }

    #[test]
    fn staging_keeps_the_stem_so_the_container_id_is_the_real_one() {
        // The writer hashes the file stem into the TOC's container id, so a
        // staging file named anything else ships a container declaring itself
        // under a name nothing will ever look for.
        let output = PathBuf::from("D:/Game/Paks/~mods/mymod_P.utoc");
        let staging = staging_utoc_for(&output);
        assert_eq!(staging.file_name(), output.file_name());
        // One level down, inside the output's own folder, so the swap is a
        // same-volume rename.
        assert_eq!(staging.parent().and_then(Path::parent), output.parent());
        let directory = staging
            .parent()
            .and_then(Path::file_name)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(directory.starts_with(".baboon-export-"), "{directory}");
    }

    /// The invariant the whole lease exists for, stated against the OS.
    ///
    /// Needs no game files: `OverrideContainerWriter` produces a valid
    /// `.utoc`/`.ucas`/`.pak` triplet from nothing. Windows-only because it is
    /// a Windows rule — a file with a mapped section refuses to be truncated
    /// (`ERROR_USER_MAPPED_FILE`, os error 1224), which is exactly what an
    /// export over an installed mod does, and what every other platform
    /// permits. There is no portable way to observe it.
    #[cfg(windows)]
    #[test]
    fn a_mapped_partition_refuses_truncation_until_it_is_released() {
        let scratch = std::env::temp_dir().join(format!(
            "baboon-lease-mapping-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&scratch).expect("scratch dir");
        let utoc = scratch.join("leased_P.utoc");
        let mut writer = blam_tags::iostore::writer::OverrideContainerWriter::new("../../../");
        let mut id = [0u8; 12];
        id[..8].copy_from_slice(&0x0bad_f00d_dead_beefu64.to_le_bytes());
        id[11] = blam_tags::iostore::CHUNK_TYPE_BULK_DATA;
        writer.add_chunk(blam_tags::iostore::FIoChunkId(id), vec![7u8; 2048]);
        writer.write(&utoc).expect("write a container to lease");

        let mut archive = blam_tags::iostore::IoStoreArchive::open(&utoc).expect("open");
        assert!(archive.is_partition_mapped());
        let ucas = utoc.with_extension("ucas");
        let truncate = || {
            std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&ucas)
        };
        assert!(
            truncate().is_err(),
            "a mapped .ucas cannot be replaced — this is os error 1224, the whole reason the \
             lease exists"
        );

        archive.release_partition();

        assert!(!archive.is_partition_mapped());
        assert!(
            truncate().is_ok(),
            "once the mapping is released the file can be replaced"
        );
        drop(archive);
        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_triplet_is_the_three_files_the_engine_loads() {
        let files = container_triplet(Path::new("D:/Game/Paks/~mods/mymod_P.utoc"));
        let names: Vec<String> = files
            .iter()
            .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["mymod_P.utoc", "mymod_P.ucas", "mymod_P.pak"]);
    }
}
