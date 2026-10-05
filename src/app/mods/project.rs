//! Campaign Evolved project/recovery persistence.
//!
//! A `.baboon` file is a small SQLite database. Clean tabs are represented by
//! canonical tag identities while modified/new tags also carry their serialized
//! tag bytes, allowing a project to be reopened without modifying the base paks.

use super::*;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(in crate::app) const CAMPAIGN_PROJECT_VERSION: i64 = 1;
pub(in crate::app) const CAMPAIGN_PROJECT_AUTOSAVE_SECS: f64 = 0.75;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum CampaignProjectTagKind {
    Existing,
    New,
}

impl CampaignProjectTagKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Existing => "existing",
            Self::New => "new",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "existing" => Some(Self::Existing),
            "new" => Some(Self::New),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct CampaignProjectTab {
    pub(in crate::app) identity: String,
    pub(in crate::app) label: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) logical_path: String,
    pub(in crate::app) kind: CampaignProjectTagKind,
    pub(in crate::app) package: Option<String>,
    pub(in crate::app) floating: bool,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct CampaignProjectOverlay {
    pub(in crate::app) identity: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) logical_path: String,
    pub(in crate::app) kind: CampaignProjectTagKind,
    pub(in crate::app) package: Option<String>,
    /// Shared, not owned: the overlay map is cloned two or three times per
    /// autosave tick, and a workspace stashing a 105 MiB tag paid a full copy
    /// each time.
    pub(in crate::app) bytes: Arc<Vec<u8>>,
    /// Hashed once, where the bytes are produced. The project fingerprint is
    /// taken over these rather than over the bytes themselves: a workspace
    /// holding a 105 MiB animation graph cost 230 ms a tick to re-hash, twice a
    /// second, purely to learn nothing had changed.
    pub(in crate::app) digest: [u8; 32],
    /// For a copy Save As made of a shipped tag, that tag's identity: the copy
    /// is wrapped in its `.uasset`, and finds it again by this on restore.
    pub(in crate::app) copied_from: Option<String>,
}

/// The identity of the tag `entry` is a Save As copy of, if it is one.
pub(in crate::app) fn copied_from_of(entry: &TagEntry) -> Option<String> {
    match &entry.location {
        TagEntryLocation::NewContainer {
            template: NewContainerTemplate::Copy { source, .. },
            ..
        } => Some(source.clone()),
        _ => None,
    }
}

/// One tag's undo and redo stacks as the session holds them, oldest first.
#[derive(Clone, Debug, Default)]
pub(in crate::app) struct TagHistory {
    pub(in crate::app) undo: Vec<HistoryStep>,
    pub(in crate::app) redo: Vec<HistoryStep>,
    /// The owning journal's change counter when this was captured, so a save
    /// can tell "nothing has happened since" without looking at the snapshots.
    pub(in crate::app) revision: u64,
}

/// One undoable step: what it was called, and the tag bytes it restores.
#[derive(Clone, Debug)]
pub(in crate::app) struct HistoryStep {
    /// The journal snapshot's id; see [`crate::app::Snapshot::id`].
    pub(in crate::app) id: u64,
    pub(in crate::app) label: String,
    pub(in crate::app) bytes: Arc<Vec<u8>>,
}

/// How many steps of one stack a restored session gets back.
///
/// Far below the in-memory limit of 64 on purpose. A step is a whole serialized
/// tag, the recovery file is rewritten as you edit, and the value of persisted
/// history falls off a cliff after the last handful of actions — nobody
/// reopens a workspace to undo their sixtieth-from-last change.
pub(in crate::app) const HISTORY_STEP_LIMIT: usize = 16;

/// The total bytes of history one workspace may write to its recovery file.
///
/// Campaign Evolved ships a 105 MiB animation graph; two of those in a stack
/// would be a quarter-gigabyte written repeatedly while the user edits. The
/// newest steps are kept and the oldest dropped, so what survives is the part
/// anyone would actually reach for.
pub(in crate::app) const HISTORY_BYTE_BUDGET: usize = 64 * 1024 * 1024;

/// Trim captured history to what may be written, newest first.
///
/// Applied across the whole workspace rather than per tag: the budget exists to
/// bound the recovery file, and a per-tag budget multiplies by however many tags
/// happen to be open. Steps are dropped oldest-first, and a stack keeps its
/// order.
pub(in crate::app) fn trim_history_for_disk(
    history: &mut BTreeMap<String, TagHistory>,
    step_limit: usize,
    byte_budget: usize,
) {
    for entry in history.values_mut() {
        for stack in [&mut entry.undo, &mut entry.redo] {
            if stack.len() > step_limit {
                stack.drain(..stack.len() - step_limit);
            }
        }
    }
    // Newest-first across every stack, so what is dropped is the least likely
    // to be wanted. `(identity, stack, index)` keeps this deterministic when two
    // steps are equally old.
    let mut steps: Vec<(String, bool, usize, usize)> = Vec::new();
    for (identity, entry) in history.iter() {
        for (index, step) in entry.undo.iter().enumerate() {
            steps.push((identity.clone(), false, index, step.bytes.len()));
        }
        for (index, step) in entry.redo.iter().enumerate() {
            steps.push((identity.clone(), true, index, step.bytes.len()));
        }
    }
    // Oldest first: lowest index within a stack, then by identity for stability.
    steps.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    let mut total: usize = steps.iter().map(|(_, _, _, len)| *len).sum();
    let mut drop_counts: HashMap<(String, bool), usize> = HashMap::new();
    for (identity, is_redo, _, len) in steps {
        if total <= byte_budget {
            break;
        }
        total -= len;
        *drop_counts.entry((identity, is_redo)).or_default() += 1;
    }
    for ((identity, is_redo), count) in drop_counts {
        let Some(entry) = history.get_mut(&identity) else {
            continue;
        };
        let stack = if is_redo {
            &mut entry.redo
        } else {
            &mut entry.undo
        };
        let count = count.min(stack.len());
        stack.drain(..count);
    }
    history.retain(|_, entry| !entry.undo.is_empty() || !entry.redo.is_empty());
}

pub(in crate::app) fn overlay_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

#[derive(Clone, Debug)]
pub(in crate::app) struct CampaignProjectSnapshot {
    pub(in crate::app) game: String,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) selected_identity: Option<String>,
    pub(in crate::app) tabs: Vec<CampaignProjectTab>,
    pub(in crate::app) overlays: HashMap<String, CampaignProjectOverlay>,
    /// Each open tag's undo/redo stacks, so reopening the workspace reopens the
    /// session rather than just the files. Written to this workspace's own
    /// recovery project and to a project the user names, never to the sidecar
    /// beside an exported mod — that one travels to whoever installs the mod,
    /// and an author's step-by-step editing trail is neither their business nor
    /// something they should have to download.
    pub(in crate::app) history: BTreeMap<String, TagHistory>,
    /// Folders the user made in the container that no tag has landed in yet.
    ///
    /// A pak's directory index cannot encode a directory with no file beneath
    /// it, so these exist only in the workspace and would otherwise be gone on
    /// the next launch. Like history, they are the author's own organisation
    /// rather than mod content, so they are not written to the sidecar beside an
    /// exported mod.
    pub(in crate::app) folders: std::collections::BTreeSet<String>,
}

impl CampaignProjectSnapshot {
    /// Each overlay's digest, by identity — what the overlays table holds once
    /// this snapshot has been written.
    pub(in crate::app) fn digests(&self) -> SavedProjectState {
        SavedProjectState {
            overlays: self
                .overlays
                .iter()
                .map(|(identity, overlay)| (identity.clone(), overlay.digest))
                .collect(),
            history: self.history_digest(),
            history_rows: self
                .history
                .iter()
                .flat_map(|(identity, entry)| {
                    [(false, &entry.undo), (true, &entry.redo)]
                        .into_iter()
                        .flat_map(move |(is_redo, steps)| {
                            steps.iter().enumerate().map(move |(position, step)| {
                                ((identity.clone(), step.id), (is_redo, position))
                            })
                        })
                })
                .collect(),
        }
    }

    /// One digest over the whole session history, so a save can skip the
    /// history altogether when no journal has moved. When one has, only its
    /// new steps are written; see `write_history`.
    pub(in crate::app) fn history_digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for (identity, entry) in &self.history {
            hasher.update(identity.as_bytes());
            // The journal's own change counter, not a hash of the snapshots:
            // hashing them would only move the per-tick cost from the disk to
            // the CPU, which is not a saving.
            hasher.update(entry.revision.to_le_bytes());
            hasher.update((entry.undo.len() as u64).to_le_bytes());
            hasher.update((entry.redo.len() as u64).to_le_bytes());
        }
        hasher.finalize().into()
    }

    pub(in crate::app) fn fingerprint(&self) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(self.game.as_bytes());
        hasher.update(self.source_path.to_string_lossy().as_bytes());
        if let Some(selected) = &self.selected_identity {
            hasher.update(selected.as_bytes());
        }
        for tab in &self.tabs {
            hasher.update(tab.identity.as_bytes());
            hasher.update([tab.floating as u8]);
        }
        let mut overlays = self.overlays.values().collect::<Vec<_>>();
        overlays.sort_by(|a, b| a.identity.cmp(&b.identity));
        for overlay in overlays {
            hasher.update(overlay.identity.as_bytes());
            hasher.update(overlay.digest);
        }
        // Undo history moves with the edits that produced it almost always, but
        // not quite: a redo stack cleared by a fresh edit, or an undo that lands
        // back on bytes already stashed, changes the session without changing
        // any overlay. Folding it in is what stops those going unwritten.
        hasher.update(self.history_digest());
        // Same reasoning as history, and more sharply: making a folder changes
        // no overlay and no tab at all, so without this the autosave would find
        // the fingerprint unchanged, skip the write, and the folder would be
        // gone at the next launch — having looked, all session, like it had
        // been saved. `folders` is a `BTreeSet`, so the order is stable.
        for folder in &self.folders {
            hasher.update(folder.as_bytes());
            hasher.update([0]);
        }
        hasher.finalize().to_vec()
    }
}

/// What a written project left on disk, so the next write can skip what has
/// not changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct SavedProjectState {
    /// Each overlay's digest, by identity — rows whose bytes match are left
    /// alone rather than rewritten.
    pub(in crate::app) overlays: HashMap<String, [u8; 32]>,
    /// One digest over the whole history, so a save can skip it when nothing
    /// moved.
    pub(in crate::app) history: [u8; 32],
    /// Every history row, `(identity, step id) → (is redo, position)`. A save
    /// writes the bytes of a step only when it is not already here, and moves
    /// the rest by updating two integers.
    pub(in crate::app) history_rows: HashMap<(String, u64), (bool, usize)>,
}

/// Which kind of `.baboon` is being written.
///
/// The two are the same format and deliberately not the same content: a session
/// project is this workspace's own state, while a sidecar is published beside a
/// mod for other people.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ProjectScope {
    /// This workspace's recovery file, or a project the user saved.
    Session,
    /// The `.baboon` written next to an exported mod's containers.
    ModSidecar,
}

pub(in crate::app) struct ActiveCampaignProject {
    /// Where this workspace autosaves. Always derived from the mounted source
    /// — never a file the user picked.
    ///
    /// The two paths are kept apart because they were once one field, and
    /// opening a `.baboon` therefore made *that* file the autosave target: an
    /// exported mod's sidecar became a live, self-overwriting file the moment it
    /// was opened, and declining to save at exit deleted the stashed rows out of
    /// it. Autosave now only ever writes the recovery file.
    pub(in crate::app) recovery_path: PathBuf,
    /// The `.baboon` this workspace is associated with: what `File > Open
    /// Baboon Project` opened, or where `Save Baboon Project As...` last wrote.
    /// It is the target of `File > Save Baboon Project`, and nothing else
    /// writes to it.
    pub(in crate::app) project_path: Option<PathBuf>,
    pub(in crate::app) overlays: HashMap<String, CampaignProjectOverlay>,
    /// Document key -> the `Dirty` revision its overlay bytes were written
    /// from, so an untouched document is never serialized twice.
    /// Keyed by `content_stamp()`, not the bare revision: a reloaded
    /// document starts again from revision 0, and a revision alone could take
    /// it for the document it replaced and keep that one's stale bytes.
    pub(in crate::app) captured_revisions: HashMap<String, (u64, u64)>,
    /// What the recovery file holds, by identity, so a save writes only the rows
    /// whose bytes changed instead of replacing every overlay.
    ///
    /// `None` when that is unknown — a fresh workspace, or a project imported
    /// from a file the recovery has never seen — and the next write then
    /// replaces every row rather than merging into whatever was there.
    pub(in crate::app) saved_digests: Option<SavedProjectState>,
    /// What an in-flight write will leave on disk, promoted to `saved_digests`
    /// once it reports success.
    pub(in crate::app) pending_digests: Option<SavedProjectState>,
    pub(in crate::app) last_saved_fingerprint: Vec<u8>,
    pub(in crate::app) next_autosave_at: f64,
    pub(in crate::app) revision: u64,
    pub(in crate::app) save_in_flight: Option<u64>,
    pub(in crate::app) write_lock: Arc<Mutex<()>>,
    pub(in crate::app) latest_write_revision: Arc<AtomicU64>,
    /// Stashed *new* tags adopted from the recovery file that still have no
    /// entry in the browser.
    ///
    /// A new tag exists only in memory, so a recovered overlay is all that is
    /// left of one -- and adopting the file's overlays without recreating those
    /// entries left the tag nowhere: absent from the tree, unresolvable, and
    /// listed in Export Mod as "not in this source" forever, because the
    /// overlays table is written back out every autosave. Held as a queue rather
    /// than adopted on the spot: the recovery file is picked up as soon as the
    /// source mounts, which can be before the names and container templates the
    /// entry needs are loaded.
    pub(in crate::app) pending_new_overlays: Vec<CampaignProjectOverlay>,
}

impl ActiveCampaignProject {
    pub(in crate::app) fn fresh(recovery_path: PathBuf, now: f64) -> Self {
        Self {
            recovery_path,
            project_path: None,
            overlays: HashMap::new(),
            captured_revisions: HashMap::new(),
            // Nothing is known about whatever file may be sitting at the
            // recovery path, so the first write replaces it outright.
            saved_digests: None,
            pending_digests: None,
            last_saved_fingerprint: Vec::new(),
            next_autosave_at: now + CAMPAIGN_PROJECT_AUTOSAVE_SECS,
            revision: 0,
            save_in_flight: None,
            write_lock: Arc::new(Mutex::new(())),
            latest_write_revision: Arc::new(AtomicU64::new(0)),
            pending_new_overlays: Vec::new(),
        }
    }

    /// The recovery file's own contents, picked back up. Its digests are exactly
    /// what is stored there, and it needs no rewrite until something changes.
    pub(in crate::app) fn adopted(
        recovery_path: PathBuf,
        snapshot: &CampaignProjectSnapshot,
        now: f64,
    ) -> Self {
        Self {
            saved_digests: Some(snapshot.digests()),
            last_saved_fingerprint: snapshot.fingerprint(),
            ..Self::from_snapshot_parts(recovery_path, snapshot, now)
        }
    }

    /// A project read from a `.baboon` the user pointed at. The recovery file has
    /// never held these bytes, so neither the digests nor the fingerprint may
    /// claim otherwise: leaving either behind would let the first autosave
    /// conclude there was nothing to write and merge these overlays into
    /// whatever the last workspace left at that path.
    pub(in crate::app) fn imported(
        recovery_path: PathBuf,
        project_path: PathBuf,
        snapshot: &CampaignProjectSnapshot,
        now: f64,
    ) -> Self {
        Self {
            project_path: Some(project_path),
            saved_digests: None,
            last_saved_fingerprint: Vec::new(),
            ..Self::from_snapshot_parts(recovery_path, snapshot, now)
        }
    }

    fn from_snapshot_parts(
        recovery_path: PathBuf,
        snapshot: &CampaignProjectSnapshot,
        now: f64,
    ) -> Self {
        Self {
            recovery_path,
            project_path: None,
            saved_digests: None,
            overlays: snapshot.overlays.clone(),
            captured_revisions: HashMap::new(),
            pending_digests: None,
            last_saved_fingerprint: Vec::new(),
            next_autosave_at: now + CAMPAIGN_PROJECT_AUTOSAVE_SECS,
            revision: 0,
            save_in_flight: None,
            write_lock: Arc::new(Mutex::new(())),
            latest_write_revision: Arc::new(AtomicU64::new(0)),
            pending_new_overlays: snapshot
                .overlays
                .values()
                .filter(|overlay| overlay.kind == CampaignProjectTagKind::New)
                .cloned()
                .collect(),
        }
    }

    /// What to show as this workspace's project, and where its edits actually
    /// live. The recovery file is not something the user named, so it is
    /// described rather than presented as an open document.
    pub(in crate::app) fn label(&self) -> String {
        match self.project_path.as_deref() {
            Some(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            None => "unsaved".to_owned(),
        }
    }
}

pub(in crate::app) struct PendingCampaignProject {
    pub(in crate::app) path: PathBuf,
    /// The project to open once the source mounts. `None` stages the path as
    /// this workspace's save target *without* reading it back in, which is what
    /// a session restore wants: the workspace's recovery file is always the
    /// fresher copy of the same edits, so re-importing the `.baboon` the user
    /// happened to have open would overwrite newer work with older.
    pub(in crate::app) snapshot: Option<CampaignProjectSnapshot>,
}

/// Where a Campaign Evolved kit autosaves its recovery project.
///
/// Derived from the mounted source so two Campaign Evolved kits recover to
/// two files rather than overwriting each other, and so a kit finds its own
/// recovery again on the next launch. `None` keeps the original unqualified
/// name for a kit with no source path to key on.
pub(in crate::app) fn campaign_recovery_path(source_root: Option<&Path>) -> PathBuf {
    let Some(root) = source_root else {
        return crate::core::storage::data_path("campaign_evolved_recovery.baboon");
    };
    let mut hasher = Sha256::new();
    hasher.update(root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let tag = digest[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    crate::core::storage::data_path(&format!("{CAMPAIGN_RECOVERY_STEM}-{tag}.baboon"))
}

pub(in crate::app) const CAMPAIGN_RECOVERY_STEM: &str = "campaign_evolved_recovery";

/// Whether `path` is one of Baboon's own recovery files rather than a `.baboon`
/// the user named. Recovery files are an implementation detail of a workspace —
/// they are not offered as a save target, and a session that recorded one back
/// when the two were the same file must not be read as having a project open.
pub(in crate::app) fn is_campaign_recovery_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(CAMPAIGN_RECOVERY_STEM))
}

/// Bring the history tables in line with `snapshot`, writing only what is
/// not already there. Anything but a session file carries no history.
fn write_history(
    transaction: &rusqlite::Transaction<'_>,
    snapshot: &CampaignProjectSnapshot,
    on_disk: Option<&SavedProjectState>,
    scope: ProjectScope,
) -> Result<(), String> {
    let failed = |error: rusqlite::Error| format!("Could not write project history: {error}");
    // Rows in the table older builds wrote: this file's history is there, in a
    // shape `on_disk` does not describe. Rewritten whole into `history_steps`,
    // and cleared, in this same transaction.
    let legacy = transaction
        .query_row("SELECT EXISTS (SELECT 1 FROM history)", [], |row| {
            row.get::<_, bool>(0)
        })
        .map_err(failed)?;
    let known = on_disk
        .filter(|_| !legacy && scope == ProjectScope::Session)
        .map(|on_disk| &on_disk.history_rows);
    if legacy {
        transaction
            .execute("DELETE FROM history", [])
            .map_err(failed)?;
    }
    let Some(known) = known else {
        transaction
            .execute("DELETE FROM history_steps", [])
            .map_err(failed)?;
        if scope == ProjectScope::Session {
            for ((identity, _), (is_redo, position), step) in history_rows(snapshot) {
                insert_history_step(transaction, identity, step, is_redo, position)?;
            }
        }
        return Ok(());
    };
    let wanted = snapshot.digests().history_rows;
    for (identity, step) in known.keys() {
        if !wanted.contains_key(&(identity.clone(), *step)) {
            transaction
                .execute(
                    "DELETE FROM history_steps WHERE identity = ?1 AND step = ?2",
                    params![identity, *step as i64],
                )
                .map_err(failed)?;
        }
    }
    for ((identity, id), (is_redo, position), step) in history_rows(snapshot) {
        match known.get(&(identity.clone(), id)) {
            None => insert_history_step(transaction, identity, step, is_redo, position)?,
            Some(&place) if place == (is_redo, position) => {}
            Some(_) => {
                transaction
                    .execute(
                        "UPDATE history_steps SET stack = ?3, position = ?4
                         WHERE identity = ?1 AND step = ?2",
                        params![
                            identity,
                            id as i64,
                            history_stack_name(is_redo),
                            position as i64
                        ],
                    )
                    .map_err(failed)?;
            }
        }
    }
    Ok(())
}

/// Every step in `snapshot`, as `((identity, id), (is redo, position), step)`.
fn history_rows(
    snapshot: &CampaignProjectSnapshot,
) -> impl Iterator<Item = ((&String, u64), (bool, usize), &HistoryStep)> {
    snapshot.history.iter().flat_map(|(identity, entry)| {
        [(false, &entry.undo), (true, &entry.redo)]
            .into_iter()
            .flat_map(move |(is_redo, steps)| {
                steps
                    .iter()
                    .enumerate()
                    .map(move |(position, step)| ((identity, step.id), (is_redo, position), step))
            })
    })
}

fn history_stack_name(is_redo: bool) -> &'static str {
    if is_redo { "redo" } else { "undo" }
}

fn insert_history_step(
    transaction: &rusqlite::Transaction<'_>,
    identity: &str,
    step: &HistoryStep,
    is_redo: bool,
    position: usize,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT OR REPLACE INTO history_steps
             (identity, step, stack, position, label, bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                identity,
                step.id as i64,
                history_stack_name(is_redo),
                position as i64,
                step.label,
                step.bytes.as_slice(),
            ],
        )
        .map_err(|error| format!("Could not write project history for {identity}: {error}"))?;
    Ok(())
}

/// Write the project to `path`.
///
/// `on_disk` is what the last successful save left in the overlays table, by
/// identity and digest; overlay rows whose bytes are unchanged are then left
/// alone. Rewriting every overlay meant editing a 4 MiB scenario also rewrote
/// the 105 MiB animation graph stashed beside it. Pass `None` when the file's
/// contents are unknown — every overlay is replaced, as before.
pub(in crate::app) fn save_campaign_project(
    path: &Path,
    snapshot: &CampaignProjectSnapshot,
    on_disk: Option<&SavedProjectState>,
    scope: ProjectScope,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create project folder: {error}"))?;
    }
    let mut connection = Connection::open(path)
        .map_err(|error| format!("Could not open project {}: {error}", path.display()))?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS project (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 version INTEGER NOT NULL,
                 game TEXT NOT NULL,
                 source_path TEXT NOT NULL,
                 selected_identity TEXT
             );
             CREATE TABLE IF NOT EXISTS tabs (
                 position INTEGER PRIMARY KEY,
                 identity TEXT NOT NULL UNIQUE,
                 label TEXT NOT NULL,
                 group_tag INTEGER NOT NULL,
                 logical_path TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 package TEXT,
                 floating INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS overlays (
                 identity TEXT PRIMARY KEY,
                 group_tag INTEGER NOT NULL,
                 logical_path TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 package TEXT,
                 bytes BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS history (
                 identity TEXT NOT NULL,
                 stack TEXT NOT NULL,
                 position INTEGER NOT NULL,
                 label TEXT NOT NULL,
                 bytes BLOB NOT NULL,
                 PRIMARY KEY (identity, stack, position)
             );
             -- Keyed by step rather than position, so a stack shifting by
             -- one updates positions instead of rewriting every step's bytes.
             -- Replaces `history`, which older builds wrote and which is still
             -- read, and cleared by the first save that finds rows in it.
             CREATE TABLE IF NOT EXISTS history_steps (
                 identity TEXT NOT NULL,
                 step INTEGER NOT NULL,
                 stack TEXT NOT NULL,
                 position INTEGER NOT NULL,
                 label TEXT NOT NULL,
                 bytes BLOB NOT NULL,
                 PRIMARY KEY (identity, step)
             );
             CREATE TABLE IF NOT EXISTS folders (
                 path TEXT PRIMARY KEY
             );
             -- What a Save As copy was copied from, by the copy's identity.
             -- A table of its own so older builds, which ignore it, still
             -- read the project.
             CREATE TABLE IF NOT EXISTS overlay_origins (
                 identity TEXT PRIMARY KEY,
                 copied_from TEXT NOT NULL
             );",
        )
        .map_err(|error| format!("Could not initialize project database: {error}"))?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start project transaction: {error}"))?;
    transaction
        .execute("DELETE FROM project", [])
        .and_then(|_| transaction.execute("DELETE FROM tabs", []))
        .map_err(|error| format!("Could not reset project database: {error}"))?;
    // Overlays are reconciled rather than replaced: drop the ones that are gone,
    // rewrite only the ones whose bytes differ.
    match on_disk {
        Some(on_disk) => {
            for identity in on_disk.overlays.keys() {
                if !snapshot.overlays.contains_key(identity) {
                    transaction
                        .execute(
                            "DELETE FROM overlays WHERE identity = ?1",
                            params![identity],
                        )
                        .map_err(|error| {
                            format!("Could not drop project tag {identity}: {error}")
                        })?;
                }
            }
        }
        None => {
            transaction
                .execute("DELETE FROM overlays", [])
                .map_err(|error| format!("Could not reset project overlays: {error}"))?;
        }
    }
    transaction
        .execute(
            "INSERT INTO project (id, version, game, source_path, selected_identity)
             VALUES (1, ?1, ?2, ?3, ?4)",
            params![
                CAMPAIGN_PROJECT_VERSION,
                snapshot.game,
                snapshot.source_path.to_string_lossy(),
                snapshot.selected_identity,
            ],
        )
        .map_err(|error| format!("Could not write project metadata: {error}"))?;
    for (position, tab) in snapshot.tabs.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO tabs
                 (position, identity, label, group_tag, logical_path, kind, package, floating)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    position as i64,
                    tab.identity,
                    tab.label,
                    i64::from(tab.group_tag),
                    tab.logical_path,
                    tab.kind.as_str(),
                    tab.package,
                    tab.floating as i64,
                ],
            )
            .map_err(|error| format!("Could not write project tab {}: {error}", tab.label))?;
    }
    for overlay in snapshot.overlays.values() {
        if on_disk
            .is_some_and(|on_disk| on_disk.overlays.get(&overlay.identity) == Some(&overlay.digest))
        {
            continue;
        }
        transaction
            .execute(
                "INSERT OR REPLACE INTO overlays
                 (identity, group_tag, logical_path, kind, package, bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    overlay.identity,
                    i64::from(overlay.group_tag),
                    overlay.logical_path,
                    overlay.kind.as_str(),
                    overlay.package,
                    overlay.bytes.as_slice(),
                ],
            )
            .map_err(|error| {
                format!(
                    "Could not write project tag {}: {error}",
                    overlay.logical_path
                )
            })?;
    }
    // History is reconciled by step id. A step's bytes never change once it
    // exists, so only steps new since the last save are written; the rest are
    // moved (an undo stack shifts by one on every edit) or dropped. Keyed by
    // position, every save used to rewrite every step, each a whole tag.
    // Skipped entirely when nothing has changed.
    let history_unchanged =
        on_disk.is_some_and(|on_disk| on_disk.history == snapshot.history_digest());
    if !history_unchanged {
        write_history(&transaction, snapshot, on_disk, scope)?;
    }
    // Folders are replaced wholesale: the set is a handful of short strings, so
    // reconciling it would cost more than rewriting it. Session scope only —
    // like history, a folder is the author's own organisation, and the sidecar
    // travels to whoever installs the mod.
    // Replaced wholesale, like folders: a row per copy, and few of those.
    transaction
        .execute("DELETE FROM overlay_origins", [])
        .map_err(|error| format!("Could not reset project tag origins: {error}"))?;
    for overlay in snapshot.overlays.values() {
        if let Some(source) = &overlay.copied_from {
            transaction
                .execute(
                    "INSERT INTO overlay_origins (identity, copied_from) VALUES (?1, ?2)",
                    params![overlay.identity, source],
                )
                .map_err(|error| {
                    format!("Could not write project tag origin {}: {error}", overlay.logical_path)
                })?;
        }
    }
    transaction
        .execute("DELETE FROM folders", [])
        .map_err(|error| format!("Could not reset project folders: {error}"))?;
    if scope == ProjectScope::Session {
        for folder in &snapshot.folders {
            transaction
                .execute("INSERT INTO folders (path) VALUES (?1)", params![folder])
                .map_err(|error| format!("Could not write project folder {folder}: {error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("Could not commit project database: {error}"))
}

/// Each Save As copy's source identity, by the copy's identity; empty for a
/// project written before the table existed.
fn read_overlay_origins(connection: &Connection) -> Vec<(String, String)> {
    let Ok(mut statement) = connection.prepare("SELECT identity, copied_from FROM overlay_origins")
    else {
        return Vec::new();
    };
    statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// Read the pending-folder set, tolerating a project written before the table
/// existed.
///
/// `CAMPAIGN_PROJECT_VERSION` is deliberately not bumped for this: the version
/// check is a strict equality, so raising it would make every `.baboon` already
/// on disk unreadable by the new build — for a change that only adds a table an
/// older build silently ignores.
fn read_project_folders(connection: &Connection) -> std::collections::BTreeSet<String> {
    let Ok(mut statement) = connection.prepare("SELECT path FROM folders") else {
        return Default::default();
    };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else {
        return Default::default();
    };
    rows.filter_map(Result::ok)
        .filter(|path| !path.trim().is_empty())
        .collect()
}

pub(in crate::app) fn load_campaign_project(path: &Path) -> Result<CampaignProjectSnapshot, String> {
    let connection = Connection::open(path)
        .map_err(|error| format!("Could not open project {}: {error}", path.display()))?;
    let (version, game, source_path, selected_identity): (i64, String, String, Option<String>) =
        connection
            .query_row(
                "SELECT version, game, source_path, selected_identity FROM project WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|error| format!("Could not read project metadata: {error}"))?;
    if version != CAMPAIGN_PROJECT_VERSION {
        return Err(format!(
            "Unsupported Baboon project version {version} (expected {CAMPAIGN_PROJECT_VERSION})"
        ));
    }
    if game != GameId::CampaignEvolved.as_str() {
        return Err(format!("Project is for unsupported game '{game}'"));
    }

    let mut tabs_statement = connection
        .prepare(
            "SELECT identity, label, group_tag, logical_path, kind, package, floating
             FROM tabs ORDER BY position",
        )
        .map_err(|error| format!("Could not read project tabs: {error}"))?;
    let tabs = tabs_statement
        .query_map([], |row| {
            let kind: String = row.get(4)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                kind,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)? != 0,
            ))
        })
        .map_err(|error| format!("Could not query project tabs: {error}"))?
        .map(|row| {
            let (identity, label, group_tag, logical_path, kind, package, floating) =
                row.map_err(|error| format!("Could not decode project tab: {error}"))?;
            let kind = CampaignProjectTagKind::from_str(&kind)
                .ok_or_else(|| format!("Unknown project tag kind '{kind}'"))?;
            Ok(CampaignProjectTab {
                identity,
                label,
                group_tag: group_tag as u32,
                logical_path,
                kind,
                package,
                floating,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    drop(tabs_statement);

    let mut overlays_statement = connection
        .prepare("SELECT identity, group_tag, logical_path, kind, package, bytes FROM overlays")
        .map_err(|error| format!("Could not read project tags: {error}"))?;
    let overlays = overlays_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Vec<u8>>(5)?,
            ))
        })
        .map_err(|error| format!("Could not query project tags: {error}"))?
        .map(|row| {
            let (identity, group_tag, logical_path, kind, package, bytes) =
                row.map_err(|error| format!("Could not decode project tag: {error}"))?;
            let kind = CampaignProjectTagKind::from_str(&kind)
                .ok_or_else(|| format!("Unknown project tag kind '{kind}'"))?;
            Ok((
                identity.clone(),
                CampaignProjectOverlay {
                    identity,
                    group_tag: group_tag as u32,
                    logical_path,
                    kind,
                    package,
                    digest: overlay_digest(&bytes),
                    bytes: Arc::new(bytes),
                    copied_from: None,
                },
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let mut overlays = overlays;
    for (identity, source) in read_overlay_origins(&connection) {
        if let Some(overlay) = overlays.get_mut(&identity) {
            overlay.copied_from = Some(source);
        }
    }

    // A project written before history existed simply has no table. That is a
    // session with nothing to undo, not a project that fails to open. One
    // written before steps had ids keeps them in `history`, by position; its
    // steps are given fresh ids, and its next save moves them across.
    let mut history = read_history(
        &connection,
        "SELECT identity, stack, label, bytes, step FROM history_steps
         ORDER BY identity, stack, position",
    )?;
    if history.is_empty() {
        history = read_history(
            &connection,
            "SELECT identity, stack, label, bytes, NULL FROM history
             ORDER BY identity, stack, position",
        )?;
    }

    let folders = read_project_folders(&connection);
    Ok(CampaignProjectSnapshot {
        game,
        source_path: PathBuf::from(source_path),
        selected_identity,
        tabs,
        overlays,
        history,
        folders,
    })
}

/// Read history rows as `(identity, stack, label, bytes, step id or NULL)`.
/// A missing table reads as no history.
fn read_history(
    connection: &Connection,
    query: &str,
) -> Result<BTreeMap<String, TagHistory>, String> {
    let mut history: BTreeMap<String, TagHistory> = BTreeMap::new();
    let Ok(mut statement) = connection.prepare(query) else {
        return Ok(history);
    };
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(|error| format!("Could not query project history: {error}"))?;
    for row in rows {
        let (identity, stack, label, bytes, id) =
            row.map_err(|error| format!("Could not decode project history: {error}"))?;
        let bytes = Arc::new(bytes);
        let snapshot = match id {
            Some(id) => crate::app::Snapshot::restored(id as u64, bytes, label),
            None => crate::app::Snapshot::restored(crate::app::next_snapshot_id(), bytes, label),
        };
        let step = HistoryStep {
            id: snapshot.id,
            label: snapshot.label,
            bytes: snapshot.bytes,
        };
        let entry = history.entry(identity).or_default();
        match stack.as_str() {
            "undo" => entry.undo.push(step),
            "redo" => entry.redo.push(step),
            // A stack name this build does not know is skipped rather than
            // guessed at: restoring a step onto the wrong stack would undo
            // in the wrong direction.
            _ => {}
        }
    }
    Ok(history)
}

/// Take the lock that keeps a project's writes in order.
///
/// It guards no data, only the order of writes, so a writer that panicked
/// while holding it left nothing half-updated behind it: each write is its
/// own transaction. Refusing a poisoned lock, as it used to be, turned one
/// panicking autosave into every later checkpoint failing for the session.
fn lock_campaign_project_writer(lock: &Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One autosave of a project's recovery file, for
/// [`write_campaign_project_in_background`].
struct CampaignProjectWrite {
    revision: u64,
    path: PathBuf,
    fingerprint: Vec<u8>,
    snapshot: CampaignProjectSnapshot,
    on_disk: Option<SavedProjectState>,
    write_lock: Arc<Mutex<()>>,
    latest_write_revision: Arc<AtomicU64>,
}

/// Write a project autosave off the UI thread and report it with
/// `CampaignProjectSaved`, which clears the save in flight.
///
/// It goes through `spawn_worker` so that a write that panics still reports:
/// on a bare thread it sent nothing, and the save in flight that refuses
/// every later autosave was never cleared.
fn write_campaign_project_in_background(
    tx: &std::sync::mpsc::Sender<WorkerMessage>,
    ctx: &egui::Context,
    write: CampaignProjectWrite,
) {
    let CampaignProjectWrite {
        revision,
        path,
        fingerprint,
        snapshot,
        on_disk,
        write_lock,
        latest_write_revision,
    } = write;
    let (panic_path, panic_fingerprint) = (path.clone(), fingerprint.clone());
    spawn_worker(
        tx,
        ctx,
        move || {
            let result = {
                let _guard = lock_campaign_project_writer(&write_lock);
                if latest_write_revision.load(Ordering::SeqCst) != revision {
                    Ok(())
                } else {
                    save_campaign_project(&path, &snapshot, on_disk.as_ref(), ProjectScope::Session)
                }
            };
            WorkerMessage::CampaignProjectSaved {
                revision,
                path,
                fingerprint,
                result,
            }
        },
        move |error| WorkerMessage::CampaignProjectSaved {
            revision,
            path: panic_path,
            fingerprint: panic_fingerprint,
            result: Err(format!("Saving the project failed: {error}")),
        },
    );
}

fn logical_path_from_display(display_path: &str) -> String {
    let normalized = display_path.replace('\\', "/");
    normalized
        .rsplit_once('.')
        .map(|(path, _)| path)
        .unwrap_or(&normalized)
        .trim_matches('/')
        .to_ascii_lowercase()
}

/// The identity an older build gave a container tag whose name has a dot in
/// it, or `None` where it is the same as today's.
///
/// Container tags are named without an extension, and the display path used
/// to be made by cutting at the name's last dot: `levels/v1.2/bitmaps/rock`
/// displayed as `levels/v1.bitmap`, so its identity was `…:levels/v1`. Project
/// files written then still hold that identity.
fn legacy_campaign_identity(entry: &TagEntry) -> Option<String> {
    if !matches!(entry.location, TagEntryLocation::Container { .. }) {
        return None;
    }
    let (_, logical_path, _, _) = campaign_entry_project_parts(entry)?;
    let (stem, _) = logical_path.rsplit_once('.')?;
    let stem = stem.trim_matches('/');
    (!stem.is_empty()).then(|| format!("{:08x}:{stem}", entry.group_tag))
}

pub(in crate::app) fn campaign_entry_project_parts(
    entry: &TagEntry,
) -> Option<(String, String, CampaignProjectTagKind, Option<String>)> {
    campaign_entry_project_parts_with(entry, None)
}

/// Identity, logical path, kind and package for one Campaign Evolved entry.
///
/// `authored_package` is the canonical `/Game/…` path of a copy **Baboon
/// itself** put into a container, taken from the duplicate ledger. Without it
/// a duplicate is indistinguishable from a tag the game shipped — it mounts as
/// an ordinary `TagEntryLocation::Container` and nothing in the container
/// records who wrote it — so it filed as `Existing` and an export built a
/// field override against a package that only exists inside the mod it was
/// copied into. With it, the copy is what it actually is: new content, with
/// its own package identity, that an export writes whole.
pub(in crate::app) fn campaign_entry_project_parts_with(
    entry: &TagEntry,
    authored_package: Option<String>,
) -> Option<(String, String, CampaignProjectTagKind, Option<String>)> {
    let logical_path = logical_path_from_display(&entry.display_path);
    let (kind, package) = match (&entry.location, authored_package) {
        (TagEntryLocation::Container { .. }, Some(package)) => {
            (CampaignProjectTagKind::New, Some(package))
        }
        (TagEntryLocation::Container { .. }, None) => (CampaignProjectTagKind::Existing, None),
        (TagEntryLocation::NewContainer { package, .. }, _) => {
            (CampaignProjectTagKind::New, Some(package.clone()))
        }
        _ => return None,
    };
    let identity = format!("{:08x}:{logical_path}", entry.group_tag);
    Some((identity, logical_path, kind, package))
}

impl Baboon {
    /// The canonical package path of a container entry Baboon authored, or
    /// `None` for one the game ships.
    ///
    /// Answered from the duplicate ledger, which is the only persistent record
    /// that a copy was made and is already what gates deletion. Resolved by the
    /// container's `.utoc` path and the payload's own path, so a copy stays
    /// recognisable across remounts that reorder the container list.
    pub(in crate::app) fn authored_package_for_entry(
        &self,
        kit: usize,
        entry: &TagEntry,
    ) -> Option<String> {
        let TagEntryLocation::Container {
            container,
            rel_path,
        } = &entry.location
        else {
            return None;
        };
        let source = self.model.kits.get(kit)?.source.as_ref()?;
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return None;
        };
        let utoc = &containers.get(*container)?.utoc_path;
        self.tag_ops.created_tags
            .find(utoc, rel_path)
            .map(|record| record.package_path.clone())
    }

    /// Give a freshly restored document the undo history the project kept for
    /// it, if any is still waiting.
    ///
    /// Called wherever a document appears during a restore — the synchronous
    /// path for a stashed edit, and the worker result for a tab that had to be
    /// read back off disk. Taking the history rather than copying it means a
    /// document reopened later in the same session does not get a second, stale
    /// copy of it.
    pub(in crate::app) fn apply_pending_history(&mut self, kit: usize, key: &str) {
        let Some(history) = self.model.kits[kit].restore.pending_history.remove(key) else {
            return;
        };
        let Some(document) = self.model.kits[kit].parsed_tags.get_mut(key) else {
            return;
        };
        let steps = |steps: Vec<HistoryStep>| {
            steps
                .into_iter()
                .map(|step| crate::app::Snapshot::restored(step.id, step.bytes, step.label))
                .collect::<Vec<_>>()
        };
        document
            .journal
            .restore(steps(history.undo), steps(history.redo));
    }

    /// Stash a tag Baboon just wrote into a container as new content.
    ///
    /// A duplicate is registered as a *clean* document — the copy really is on
    /// disk, so flagging it dirty would claim an unsaved change that does not
    /// exist — and only dirty documents are captured. Without this the copy is
    /// invisible to Export Mod until it is edited, and a mod exported under a
    /// different name would silently leave it behind.
    pub(in crate::app) fn stash_authored_tag(
        &mut self,
        kit: usize,
        entry: &TagEntry,
        package: String,
        bytes: Vec<u8>,
        now: f64,
    ) {
        let Some((identity, logical_path, kind, package)) =
            campaign_entry_project_parts_with(entry, Some(package))
        else {
            return;
        };
        self.ensure_campaign_project(kit, now);
        let Some(project) = self.model.kits[kit].project.active.as_mut() else {
            return;
        };
        project.overlays.insert(
            identity.clone(),
            CampaignProjectOverlay {
                identity,
                group_tag: entry.group_tag,
                logical_path,
                kind,
                package,
                digest: overlay_digest(&bytes),
                bytes: Arc::new(bytes),
                copied_from: copied_from_of(entry),
            },
        );
    }

    fn ensure_campaign_project(&mut self, kit: usize, now: f64) {
        if !self.model.current_source_is_campaign_project_capable(kit)
            || self.model.kits[kit].project.active.is_some()
        {
            return;
        }
        let root = self.model.kits[kit]
            .source
            .as_ref()
            .map(|source| source.source.root_path().to_path_buf());
        let path = campaign_recovery_path(root.as_deref());
        // Adopt the recovery file already sitting at this path rather than
        // starting empty. It holds this very source's stashed edits, and it is
        // keyed by a hash of the source root, so a file being there at all
        // means it belongs to this install.
        //
        // Starting fresh made the file write-only: the first autosave, within
        // a second of the source mounting, overwrote everything stashed in
        // earlier sessions. Only a session restore or File > Open Baboon
        // Project ever read one back.
        let restored = match load_campaign_project(&path) {
            // A snapshot recorded for a different source would mean a hash
            // collision; ignore it rather than serve one install's edits to
            // another.
            Ok(snapshot)
                if root
                    .as_deref()
                    .is_none_or(|root| snapshot.source_path == root) =>
            {
                Some(snapshot)
            }
            _ => None,
        };
        self.model.kits[kit].project.active = Some(match &restored {
            Some(snapshot) => ActiveCampaignProject::adopted(path, snapshot, now),
            None => ActiveCampaignProject::fresh(path, now),
        });
        // Folders the user made last session. Nothing else brings them back:
        // they are not in any pak, so the mount cannot re-derive them.
        if let Some(folders) = restored.as_ref().map(|snapshot| snapshot.folders.clone()) {
            self.adopt_project_container_folders(kit, folders);
        }
        if let Some(count) = restored
            .as_ref()
            .map(|snapshot| snapshot.overlays.len())
            .filter(|count| *count > 0)
        {
            self.model.status = format!(
                "Restored {count} stashed modification(s) from this workspace's last session"
            );
        }
    }

    /// Put stashed new tags back into the browser.
    ///
    /// Runs until each queued overlay is either placed or already there. An
    /// overlay is left queued while the source is still loading -- the names and
    /// the template container are read from it -- and dropped once it resolves,
    /// so a tag adopted by `File > Open Baboon Project`, which registers them
    /// itself, is not registered twice.
    ///
    /// Scoped to the focused kit: registration writes through the active source.
    /// A background kit's queue waits until that kit is focused, which is before
    /// anything can be exported from it.
    fn adopt_pending_new_overlays(&mut self, kit: usize) {
        if kit != self.model.active
            || self.model.kits[kit]
                .project.active
                .as_ref()
                .is_none_or(|project| project.pending_new_overlays.is_empty())
        {
            return;
        }
        let queued = self.model.kits[kit]
            .project.active
            .as_ref()
            .map(|project| project.pending_new_overlays.clone())
            .unwrap_or_default();
        let mut adopted = 0usize;
        let mut still_pending = Vec::new();
        let mut failed = Vec::new();
        for overlay in queued {
            if self
                .model.campaign_entry_for_identity(kit, &overlay.identity)
                .is_some()
            {
                continue;
            }
            // Only "not yet" stays queued. A failure used to stay queued too,
            // and this runs every frame: one overlay that could never be placed
            // redid the entry scans and the tag parse every frame, for good.
            // Its bytes stay stashed in the project either way.
            let (entry, tag) = match self.model.new_overlay_entry(kit, &overlay) {
                OverlayAdoption::Ready(entry, tag) => (entry, tag),
                OverlayAdoption::NotYet => {
                    still_pending.push(overlay);
                    continue;
                }
                OverlayAdoption::Failed(reason) => {
                    failed.push(format!("{}: {reason}", overlay.logical_path));
                    continue;
                }
            };
            let key = entry.key.clone();
            self.stash_in_memory_tag(entry, tag);
            // The document was parsed from the overlay's own bytes, so the
            // project already holds its serialization -- recording that spares
            // the next autosave from writing every adopted tag out again.
            if let Some(revision) = self.model.kits[kit]
                .parsed_tags
                .get(&key)
                .map(|document| document.content_stamp())
            {
                if let Some(project) = self.model.kits[kit].project.active.as_mut() {
                    project.captured_revisions.insert(key, revision);
                }
            }
            adopted += 1;
        }
        if let Some(project) = self.model.kits[kit].project.active.as_mut() {
            project.pending_new_overlays = still_pending;
        }
        if adopted > 0 {
            self.model.status =
                format!("Restored {adopted} stashed new tag(s) from this workspace's last session");
        }
        if !failed.is_empty() {
            self.model.status = format!(
                "Could not restore {} stashed new tag(s) (still saved in the project): {}",
                failed.len(),
                failed.join("; ")
            );
        }
    }

    pub(in crate::app) fn capture_campaign_project(
        &mut self,
        kit: usize,
        now: f64,
    ) -> Result<Option<CampaignProjectSnapshot>, String> {
        // This kit's source, not the active one: autosave runs for every kit.
        let Some(source) = self.model.kits[kit].source.as_ref() else {
            return Ok(None);
        };
        let TagSource::IoStoreContainerSet { root, .. } = &source.source else {
            return Ok(None);
        };
        let source_path = root.clone();
        let game = source
            .game
            .unwrap_or(GameId::CampaignEvolved)
            .as_str()
            .to_owned();
        self.ensure_campaign_project(kit, now);
        let mut overlays = self.model.kits[kit]
            .project.active
            .as_ref()
            .map(|project| project.overlays.clone())
            .unwrap_or_default();

        // What each dirty document's bytes were captured from last time, so a
        // document nobody has touched since is carried over instead of being
        // serialized again. Autosave runs twice a second whether or not
        // anything was edited, and a stashed 105 MiB animation graph costs
        // ~100 ms to write out.
        let captured = self.model.kits[kit]
            .project.active
            .as_ref()
            .map(|project| project.captured_revisions.clone())
            .unwrap_or_default();
        let mut now_captured: HashMap<String, (u64, u64)> = HashMap::new();
        for (key, document) in &self.model.kits[kit].parsed_tags {
            if !document.dirty.is_set() {
                continue;
            }
            let Some(entry) = self.model.entry_for_key_in(kit, key) else {
                continue;
            };
            let authored = self.authored_package_for_entry(kit, entry);
            let Some(entry) = self.model.entry_for_key_in(kit, key) else {
                continue;
            };
            let Some((identity, logical_path, kind, package)) =
                campaign_entry_project_parts_with(entry, authored)
            else {
                continue;
            };
            let revision = document.content_stamp();
            now_captured.insert(key.clone(), revision);
            if captured.get(key) == Some(&revision) && overlays.contains_key(&identity) {
                continue;
            }
            let bytes = document
                .tag
                .write_to_bytes()
                .map_err(|error| format!("Could not serialize {}: {error}", entry.display_path))?;
            overlays.insert(
                identity.clone(),
                CampaignProjectOverlay {
                    identity,
                    group_tag: entry.group_tag,
                    logical_path,
                    kind,
                    package,
                    digest: overlay_digest(&bytes),
                    bytes: Arc::new(bytes),
                    copied_from: copied_from_of(entry),
                },
            );
        }

        // Floating tabs are gone with the tab rack — the tiles tree is the
        // whole open set now, so nothing is recorded as floating.
        let floating_order: Vec<String> = Vec::new();
        let mut tabs = Vec::new();
        for key in self.model.kits[kit].open_tabs.iter().chain(floating_order.iter()) {
            let Some(entry) = self.model.entry_for_key_in(kit, key) else {
                continue;
            };
            let authored = self.authored_package_for_entry(kit, entry);
            let Some(entry) = self.model.entry_for_key_in(kit, key) else {
                continue;
            };
            let Some((identity, logical_path, kind, package)) =
                campaign_entry_project_parts_with(entry, authored)
            else {
                continue;
            };
            tabs.push(CampaignProjectTab {
                identity,
                label: entry.display_path.clone(),
                group_tag: entry.group_tag,
                logical_path,
                kind,
                package,
                floating: false,
            });
        }
        let selected_identity = self.model.kits[kit].selected_key.as_ref().and_then(|key| {
            self.model.entry_for_key_in(kit, key)
                .and_then(campaign_entry_project_parts)
                .map(|(identity, _, _, _)| identity)
        });
        // Every open document's undo trail, not just the edited ones: a tag
        // whose edits were undone back to the original is still a tag whose
        // history the user may want on the other side of a restart.
        let mut history: BTreeMap<String, TagHistory> = BTreeMap::new();
        for (key, document) in &self.model.kits[kit].parsed_tags {
            let (undo, redo) = document.journal.stacks();
            if undo.is_empty() && redo.is_empty() {
                continue;
            }
            let Some(entry) = self.model.entry_for_key_in(kit, key) else {
                continue;
            };
            let Some((identity, ..)) = campaign_entry_project_parts(entry) else {
                continue;
            };
            let step = |snapshot: &crate::app::Snapshot| HistoryStep {
                id: snapshot.id,
                label: snapshot.label.clone(),
                // Shared with the journal rather than copied — this runs twice
                // a second.
                bytes: snapshot.bytes.clone(),
            };
            history.insert(
                identity,
                TagHistory {
                    undo: undo.iter().map(step).collect(),
                    redo: redo.iter().map(step).collect(),
                    revision: document.journal.revision(),
                },
            );
        }
        trim_history_for_disk(&mut history, HISTORY_STEP_LIMIT, HISTORY_BYTE_BUDGET);
        if let Some(project) = self.model.kits[kit].project.active.as_mut() {
            project.overlays.clone_from(&overlays);
            project.captured_revisions = now_captured;
        }
        Ok(Some(CampaignProjectSnapshot {
            game,
            source_path,
            selected_identity,
            tabs,
            overlays,
            history,
            folders: self.model.kits[kit].pending_container_folders.clone(),
        }))
    }

    /// Refresh this kit's set of modified tags if anything has changed since it
    /// was last built.
    ///
    /// The signature is the identity of everything that would land in the set:
    /// the dirty documents' keys and the stashed overlays' identities. Both are
    /// small — a handful of entries — so building and comparing it every frame
    /// is far cheaper than the entry lookups the rebuild performs.
    pub(in crate::app) fn refresh_modified_tags(&mut self, kit: usize) {
        let mut signature: Vec<String> = self.model.kits[kit]
            .parsed_tags
            .iter()
            .filter(|(_, document)| document.dirty.is_set())
            .map(|(key, _)| key.clone())
            .collect();
        if let Some(project) = self.model.kits[kit].project.active.as_ref() {
            signature.extend(project.overlays.keys().cloned());
        }
        signature.sort();
        if signature == self.views[self.model.kits[kit].id].browser.modified_signature {
            return;
        }
        let mut modified = ModifiedTags::default();
        let dirty_keys: Vec<String> = self.model.kits[kit]
            .parsed_tags
            .iter()
            .filter(|(_, document)| document.dirty.is_set())
            .map(|(key, _)| key.clone())
            .collect();
        for key in dirty_keys {
            if let Some(entry) = self.model.entry_for_key_in(kit, &key) {
                modified.insert(entry);
            }
        }
        // Stashed tags need not be open, so they are resolved from the project
        // rather than from the open documents.
        let identities: Vec<String> = self.model.kits[kit]
            .project.active
            .as_ref()
            .map(|project| project.overlays.keys().cloned().collect())
            .unwrap_or_default();
        for identity in identities {
            if let Some(entry) = self.model.campaign_entry_for_identity(kit, &identity) {
                modified.insert(&entry);
            }
        }
        self.views[self.model.kits[kit].id].browser.modified_tags = std::sync::Arc::new(modified);
        self.views[self.model.kits[kit].id].browser.modified_signature = signature;
    }

    /// Forget one tag's stashed overlay, so the tag reads as its source has it
    /// again. Returns whether anything was stashed for it.
    ///
    /// Overlays are otherwise only ever inserted: without this, clearing a
    /// document's dirty flag left the edited bytes in the project and reopening
    /// the tag brought them straight back.
    pub(in crate::app) fn forget_campaign_overlay(&mut self, kit: usize, key: &str) -> bool {
        let Some(entry) = self.model.entry_for_key_in(kit, key).cloned() else {
            return false;
        };
        let Some((identity, ..)) = campaign_entry_project_parts(&entry) else {
            return false;
        };
        self.model.kits[kit]
            .project.active
            .as_mut()
            .is_some_and(|project| project.overlays.remove(&identity).is_some())
    }

    /// Forget every stashed overlay in this kit's project, returning how many
    /// tags were carrying one.
    pub(in crate::app) fn forget_all_campaign_overlays(&mut self, kit: usize) -> usize {
        let Some(project) = self.model.kits[kit].project.active.as_mut() else {
            return 0;
        };
        let count = project.overlays.len();
        project.overlays.clear();
        count
    }

    /// Throw away everything this workspace has not written into the game:
    /// every stashed overlay and every unsaved document. The tags then reload
    /// exactly as the game ships them.
    pub(in crate::app) fn clear_campaign_stash(&mut self, kit: usize, ctx: &egui::Context) {
        self.model.active = kit;
        let stashed = self.forget_all_campaign_overlays(kit);
        let open = self.model.kits[kit].open_tabs.clone();
        // Every parsed document goes, not just the dirty ones: a document
        // opened from the project reads clean while still holding the stashed
        // bytes, so keeping it would put the edits straight back.
        {
            let kit_state = &mut self.model.kits[kit];
            let view = &mut self.views[kit_state.id];
            kit_state.parsed_tags.clear();
            kit_state.loading_tags.clear();
            view.caches.bitmap_previews.clear();
            view.caches.model_previews.clear();
            view.find_filter_applied.clear();
            view.edit_buffers.clear();
        }
        let now = ctx.input(|input| input.time);
        if let Err(error) = self.checkpoint_campaign_project(kit, now) {
            self.model.status = format!("Could not update the Campaign Evolved project: {error}");
            return;
        }
        for key in open {
            self.select_entry(key, ctx.clone());
        }
        self.model.status = match stashed {
            0 => "Cleared this workspace's unsaved modifications".to_owned(),
            1 => "Cleared 1 stashed modification".to_owned(),
            n => format!("Cleared {n} stashed modifications"),
        };
    }

    pub(in crate::app) fn checkpoint_campaign_project(
        &mut self,
        kit: usize,
        now: f64,
    ) -> Result<bool, String> {
        let Some(snapshot) = self.capture_campaign_project(kit, now)? else {
            return Ok(false);
        };
        let fingerprint = snapshot.fingerprint();
        let Some(project) = self.model.kits[kit].project.active.as_mut() else {
            return Ok(false);
        };
        if fingerprint == project.last_saved_fingerprint && project.recovery_path.is_file() {
            project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
            return Ok(false);
        }
        project.revision = project.revision.wrapping_add(1);
        let revision = project.revision;
        project
            .latest_write_revision
            .store(revision, Ordering::SeqCst);
        project.save_in_flight = None;
        let write_lock = project.write_lock.clone();
        let _write_guard = lock_campaign_project_writer(&write_lock);
        let on_disk = project.saved_digests.clone();
        save_campaign_project(
            &project.recovery_path,
            &snapshot,
            on_disk.as_ref(),
            ProjectScope::Session,
        )?;
        project.saved_digests = Some(snapshot.digests());
        project.last_saved_fingerprint = fingerprint;
        project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
        if let Some(session) = self.current_session_state() {
            let _ = save_last_session(&session);
        }
        Ok(true)
    }

    /// Autosave every kit's project, not just the focused one — a project left
    /// in a background workspace must keep checkpointing or its edits are the
    /// ones lost to a crash.
    pub(in crate::app) fn maybe_autosave_campaign_projects(&mut self, ctx: &egui::Context) {
        for kit in 0..self.model.kits.len() {
            self.maybe_autosave_campaign_project(kit, ctx);
        }
    }

    fn maybe_autosave_campaign_project(&mut self, kit: usize, ctx: &egui::Context) {
        if !self.model.current_source_is_campaign_project_capable(kit) {
            return;
        }
        let now = ctx.input(|input| input.time);
        self.ensure_campaign_project(kit, now);
        self.adopt_pending_new_overlays(kit);
        let due = self.model.kits[kit]
            .project.active
            .as_ref()
            .is_some_and(|project| now >= project.next_autosave_at);
        // A save already running means this tick's work would be thrown away, so
        // do not do it. The check used to sit *after* the capture, which is the
        // expensive part.
        if due
            && self.model.kits[kit]
                .project.active
                .as_ref()
                .is_some_and(|project| project.save_in_flight.is_some())
        {
            if let Some(project) = self.model.kits[kit].project.active.as_mut() {
                project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(750));
            return;
        }
        if due {
            let snapshot = match self.capture_campaign_project(kit, now) {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => return,
                Err(error) => {
                    self.model.status = format!("Campaign project autosave failed: {error}");
                    return;
                }
            };
            let fingerprint = snapshot.fingerprint();
            let Some(project) = self.model.kits[kit].project.active.as_mut() else {
                return;
            };
            if fingerprint == project.last_saved_fingerprint && project.recovery_path.is_file() {
                project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
                // Nothing changed, and nothing can change without a frame of
                // its own (input, or a worker's message), which checks again.
                // Asking for a wake-up here kept an idle app capturing the
                // project every 0.75 s for as long as it stayed open.
                return;
            } else {
                project.revision = project.revision.wrapping_add(1);
                let revision = project.revision;
                let path = project.recovery_path.clone();
                let write_lock = project.write_lock.clone();
                let latest_write_revision = project.latest_write_revision.clone();
                latest_write_revision.store(revision, Ordering::SeqCst);
                project.save_in_flight = Some(revision);
                project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
                // What this write will leave on disk, held until it succeeds.
                let on_disk = project.saved_digests.clone();
                project.pending_digests = Some(snapshot.digests());
                write_campaign_project_in_background(
                    &self.tx,
                    ctx,
                    CampaignProjectWrite {
                        revision,
                        path,
                        fingerprint,
                        snapshot,
                        on_disk,
                        write_lock,
                        latest_write_revision,
                    },
                );
            }
        }
        // Wake when the next check is due: an edit made in this frame must be
        // saved even if nothing else happens after it.
        if let Some(project) = self.model.kits[kit].project.active.as_ref() {
            let wait = (project.next_autosave_at - now).max(0.0);
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
    }

    pub(in crate::app) fn handle_campaign_project_saved(
        &mut self,
        revision: u64,
        path: PathBuf,
        fingerprint: Vec<u8>,
        result: Result<(), String>,
    ) -> bool {
        // Locate the kit whose project this save belongs to. Matching on the
        // path and the in-flight revision is enough, and it means a save that
        // outlives its kit is dropped instead of landing on another one.
        let Some(kit) = self.model.kits.iter().position(|kit| {
            kit.project.active.as_ref().is_some_and(|project| {
                project.recovery_path == path && project.save_in_flight == Some(revision)
            })
        }) else {
            return true;
        };
        let Some(project) = self.model.kits[kit].project.active.as_mut() else {
            return true;
        };
        project.save_in_flight = None;
        let pending = project.pending_digests.take();
        match result {
            Ok(()) => {
                project.last_saved_fingerprint = fingerprint;
                // Only now is this what the file holds; a failed write leaves
                // the previous belief in place, so the next save reconciles
                // against what actually got there.
                if let Some(digests) = pending {
                    project.saved_digests = Some(digests);
                }
                if let Some(session) = self.current_session_state() {
                    let _ = save_last_session(&session);
                }
            }
            Err(error) => {
                self.model.status = format!("Campaign project autosave failed: {error}");
            }
        }
        false
    }

    pub(in crate::app) fn begin_open_campaign_project(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open Baboon Project")
            .add_filter("Baboon project", &["baboon"])
            .pick_file()
        else {
            return;
        };
        self.begin_open_campaign_project_path(path, ctx);
    }

    /// Stage the `.baboon` a restored session had open as that workspace's save
    /// target, to be attached once the source finishes mounting.
    ///
    /// Deliberately *not* an import: the workspace autosaves to its recovery
    /// file, which is therefore at least as fresh as the project file and
    /// usually fresher. Reading the `.baboon` back in would replace this
    /// session's stashed edits with whatever state the file was last explicitly
    /// saved in.
    pub(in crate::app) fn queue_campaign_project_target(&mut self, kit: usize, path: PathBuf) {
        // Sessions written before the recovery file and the project file were
        // separate recorded the recovery path here. It is not a project the user
        // named, and offering it as a save target would be wrong.
        if is_campaign_recovery_file(&path) {
            return;
        }
        self.model.kits[kit].project.pending = Some(PendingCampaignProject {
            path,
            snapshot: None,
        });
    }

    /// Write this workspace's project to its associated `.baboon`, asking for a
    /// destination when it has none yet.
    pub(in crate::app) fn save_campaign_project_file(&mut self, kit: usize, now: f64) {
        let Some(path) = self.model.kits[kit]
            .project.active
            .as_ref()
            .and_then(|project| project.project_path.clone())
        else {
            self.save_campaign_project_file_as(kit, now);
            return;
        };
        self.write_campaign_project_file(kit, path, now);
    }

    pub(in crate::app) fn save_campaign_project_file_as(&mut self, kit: usize, now: f64) {
        if !self.model.current_source_is_campaign_project_capable(kit) {
            self.model.status = "Baboon projects require a Campaign Evolved container source".to_owned();
            return;
        }
        let current = self.model.kits[kit]
            .project.active
            .as_ref()
            .and_then(|project| project.project_path.clone());
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save Baboon Project As")
            .add_filter("Baboon project", &["baboon"])
            .set_file_name(
                current
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "campaign-evolved.baboon".to_owned()),
            );
        // Next to the project it is replacing, else beside the game it edits.
        if let Some(folder) = current
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| {
                self.model.kits[kit]
                    .source
                    .as_ref()
                    .map(|source| source.source.root_path().to_path_buf())
            })
        {
            dialog = dialog.set_directory(folder);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        // A dialog that returns an extensionless name would otherwise write a
        // project that `Open Baboon Project` cannot see.
        let path = match path.extension() {
            Some(_) => path,
            None => path.with_extension("baboon"),
        };
        self.write_campaign_project_file(kit, path, now);
    }

    fn write_campaign_project_file(&mut self, kit: usize, path: PathBuf, now: f64) {
        let snapshot = match self.capture_campaign_project(kit, now) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.model.status =
                    "Baboon projects require a Campaign Evolved container source".to_owned();
                return;
            }
            Err(error) => {
                self.model.status = format!("Could not save the Baboon project: {error}");
                return;
            }
        };
        // Nothing here knows what is in a file the user named, and it may be an
        // older project entirely, so it is replaced rather than merged into.
        if let Err(error) = save_campaign_project(&path, &snapshot, None, ProjectScope::Session) {
            self.model.status = error;
            return;
        }
        let count = snapshot.overlays.len();
        if let Some(project) = self.model.kits[kit].project.active.as_mut() {
            project.project_path = Some(path.clone());
        }
        // The recovery file stays the live copy, so it is brought level with what
        // was just written out.
        if let Err(error) = self.checkpoint_campaign_project(kit, now) {
            self.model.status = format!(
                "Saved {}, but the recovery file failed: {error}",
                path.display()
            );
            return;
        }
        self.model.status = format!("Saved {count} modified tag(s) to {}", path.display());
    }

    pub(in crate::app) fn begin_open_campaign_project_path(&mut self, path: PathBuf, ctx: egui::Context) {
        let snapshot = match load_campaign_project(&path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.model.status = error;
                return;
            }
        };
        let source_path = if crate::core::source::find_paks_dir(&snapshot.source_path).is_some() {
            snapshot.source_path.clone()
        } else if let Some(configured) = self.model.prefs.editing_kit_paths.get(GameId::CampaignEvolved.as_str())
            && crate::core::source::find_paks_dir(configured).is_some()
        {
            configured.clone()
        } else {
            let Some(selected) = rfd::FileDialog::new()
                .set_title("Locate Campaign Evolved Install or Paks Folder")
                .pick_folder()
            else {
                self.model.status = "Campaign Evolved project source was not found".to_owned();
                return;
            };
            selected
        };
        self.begin_load_folder_path(source_path, ctx);
        // Staged after the load starts: the loader has routed to a kit and
        // left it active, so this lands on the kit the source will mount into.
        self.model.kits[self.model.active].project.pending = Some(PendingCampaignProject {
            path,
            snapshot: Some(snapshot),
        });
    }

    pub(in crate::app) fn apply_pending_campaign_project(
        &mut self,
        kit: usize,
        now: f64,
        ctx: &egui::Context,
    ) {
        let Some(pending) = self.model.kits[kit].project.pending.take() else {
            self.ensure_campaign_project(kit, now);
            return;
        };
        if !self.model.current_source_is_campaign_project_capable(kit) {
            self.model.status = "Baboon projects require a Campaign Evolved container source".to_owned();
            return;
        }
        // A restored session stages its project file as a save target only; the
        // recovery file this workspace has been autosaving to is the live copy,
        // and `ensure_campaign_project` has just picked it back up.
        let Some(snapshot) = pending.snapshot else {
            self.ensure_campaign_project(kit, now);
            if let Some(project) = self.model.kits[kit].project.active.as_mut() {
                project.project_path = Some(pending.path);
            }
            return;
        };
        let project_path = pending.path;
        self.adopt_project_container_folders(kit, snapshot.folders.clone());

        let mut identity_to_key = HashMap::<String, String>::new();
        let mut restored_revisions = HashMap::<String, (u64, u64)>::new();
        let mut missing = 0usize;

        // Recreate new project tags first, including ones that are currently
        // closed but must remain part of future exports.
        let new_overlays = snapshot
            .overlays
            .values()
            .filter(|overlay| overlay.kind == CampaignProjectTagKind::New)
            .cloned()
            .collect::<Vec<_>>();
        for overlay in new_overlays {
            let OverlayAdoption::Ready(entry, tag) = self.model.new_overlay_entry(kit, &overlay) else {
                missing += 1;
                continue;
            };
            let key = entry.key.clone();
            self.register_in_memory_tag(entry, tag);
            identity_to_key.insert(overlay.identity.clone(), key);
        }

        for tab in &snapshot.tabs {
            if identity_to_key.contains_key(&tab.identity) {
                continue;
            }
            let Some(entry) = self.model.campaign_entry_for_identity(kit, &tab.identity) else {
                missing += 1;
                continue;
            };
            identity_to_key.insert(tab.identity.clone(), entry.key);
        }

        // Staged by document key before any tag is opened, so it is already
        // waiting whichever way the document arrives — restored from a stashed
        // edit below, or read back off disk by a worker some frames later.
        self.model.kits[kit].restore.pending_history = snapshot
            .history
            .iter()
            .filter_map(|(identity, history)| {
                identity_to_key
                    .get(identity)
                    .map(|key| (key.clone(), history.clone()))
            })
            .collect();

        // Rebuild the kit's tag layout from the project, rather than the flat
        // tab list the rack used: the tiles tree owns which tags are open.
        let kit_id = self.model.kits[kit].id;
        self.views[self.model.kits[kit].id].tag_tree = egui_tiles::Tree::empty(tag_tree_id(kit_id));
        self.model.kits[kit].open_tabs.clear();
        self.model.kits[kit].selected_key = None;
        for tab in &snapshot.tabs {
            let Some(key) = identity_to_key.get(&tab.identity).cloned() else {
                continue;
            };
            if let Some(overlay) = snapshot.overlays.get(&tab.identity) {
                if let Ok(tag) = TagFile::read_from_bytes(&overlay.bytes) {
                    let document = TagDocument::modified(tag);
                    // The document was parsed from the overlay's own bytes, so
                    // the project already holds its serialization. Recording
                    // that spares the first autosave after a restore from
                    // writing every stashed tag out again.
                    restored_revisions.insert(key.clone(), document.content_stamp());
                    self.model.kits[kit].parsed_tags.insert(key.clone(), document);
                    self.apply_pending_history(kit, &key);
                } else {
                    missing += 1;
                    continue;
                }
            } else {
                self.ensure_tag_loading(key.clone(), ctx.clone());
            }
            self.kit_and_view(kit).open_tag_pane(&key);
        }
        self.model.kits[kit].selected_key = snapshot
            .selected_identity
            .as_ref()
            .and_then(|identity| identity_to_key.get(identity))
            .cloned()
            .or_else(|| self.model.kits[kit].open_tabs.last().cloned());
        // Tiles reveal the active tab themselves, so there is no scroll target
        // to remember; `open_tag_pane` already made each restored tag active.
        if let Some(key) = self.model.kits[kit].selected_key.clone() {
            self.kit_and_view(kit).open_tag_pane(&key);
        }
        let root = self.model.kits[kit]
            .source
            .as_ref()
            .map(|source| source.source.root_path().to_path_buf());
        let recovery_path = campaign_recovery_path(root.as_deref());
        let mut project =
            ActiveCampaignProject::imported(recovery_path, project_path, &snapshot, now);
        project.captured_revisions = restored_revisions;
        let tabs = snapshot.tabs.len();
        let stashed = snapshot.overlays.len();
        self.model.kits[kit].project.active = Some(project);
        // These overlays have only ever existed in the file the user opened. The
        // recovery file is what every later autosave writes and what the next
        // session picks up, so it is brought level with them now rather than at
        // the mercy of whether anything is edited afterwards.
        if let Err(error) = self.checkpoint_campaign_project(kit, now) {
            self.model.status = format!("Opened the project, but its recovery file failed: {error}");
            return;
        }
        self.model.status = if missing == 0 {
            format!("Restored Campaign Evolved project ({tabs} tab(s), {stashed} modified tag(s))")
        } else {
            format!(
                "Restored Campaign Evolved project; skipped {missing} missing or incompatible item(s)"
            )
        };
    }

    pub(in crate::app) fn load_campaign_overlay_for_key(&mut self, kit: usize, key: &str) -> bool {
        if self.model.kits[kit].parsed_tags.contains_key(key) {
            return true;
        }
        let Some(entry) = self.model.entry_for_key_in(kit, key).cloned() else {
            return false;
        };
        let Some((identity, _, _, _)) = campaign_entry_project_parts(&entry) else {
            return false;
        };
        let Some(overlay) = self.model.kits[kit]
            .project.active
            .as_ref()
            .and_then(|project| project.overlays.get(&identity))
            .cloned()
        else {
            return false;
        };
        match TagFile::read_from_bytes(&overlay.bytes) {
            Ok(tag) => {
                // Same as a restore: the stashed bytes are this document's
                // serialization already, so autosave need not redo it.
                let document = TagDocument::modified(tag);
                let revision = document.content_stamp();
                self.model.kits[kit].parsed_tags.insert(key.to_owned(), document);
                if let Some(project) = self.model.kits[kit].project.active.as_mut() {
                    project.captured_revisions.insert(key.to_owned(), revision);
                }
                true
            }
            Err(error) => {
                self.model.status = format!("Could not restore {}: {error}", entry.display_path);
                false
            }
        }
    }
}

/// What adopting one stashed new tag came to.
enum OverlayAdoption {
    Ready(TagEntry, TagFile),
    /// The source it is read against has not loaded yet.
    NotYet,
    /// It cannot be placed, and trying again will not change that.
    Failed(String),
}

/// This kit's Campaign Evolved recovery/project database, and project contents
/// staged until its source finishes mounting.
#[derive(Default)]
pub(in crate::app) struct KitProject {
    /// This kit's Campaign Evolved recovery/project database, if its source
    /// has one. Per kit because a project belongs to a source — two Campaign
    /// Evolved kits are two projects, and one application-wide slot would let
    /// either checkpoint over the other's tags.
    pub(in crate::app) active: Option<ActiveCampaignProject>,
    /// Project contents staged until this kit's source finishes mounting.
    pub(in crate::app) pending: Option<PendingCampaignProject>,
}

impl Model {
    pub(in crate::app) fn current_source_is_campaign_project_capable(&self, kit: usize) -> bool {
        self.kits[kit]
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.source, TagSource::IoStoreContainerSet { .. }))
    }

    /// Whether this kit's project has bytes stashed for `key` — that is, whether
    /// discarding the document would also delete something from disk.
    pub(in crate::app) fn tag_has_stashed_overlay(&self, kit: usize, key: &str) -> bool {
        let Some(entry) = self.entry_for_key_in(kit, key) else {
            return false;
        };
        let Some((identity, ..)) = campaign_entry_project_parts(entry) else {
            return false;
        };
        self.kits[kit]
            .project.active
            .as_ref()
            .is_some_and(|project| project.overlays.contains_key(&identity))
    }

    /// Identities of the tags this kit currently has stashed, as display paths.
    pub(in crate::app) fn stashed_campaign_tags(&self, kit: usize) -> Vec<String> {
        let Some(project) = self.kits[kit].project.active.as_ref() else {
            return Vec::new();
        };
        let mut paths: Vec<String> = project
            .overlays
            .values()
            .map(|overlay| overlay.logical_path.clone())
            .collect();
        paths.sort();
        paths
    }

    pub(in crate::app) fn campaign_entry_for_identity(
        &self,
        kit: usize,
        identity: &str,
    ) -> Option<TagEntry> {
        let source = self.kits[kit].source.as_ref()?;
        let entries = || source.entries.iter().chain(source.all_entries.iter());
        if let Some(entry) = entries().find(|entry| {
            campaign_entry_project_parts(entry)
                .is_some_and(|(candidate, _, _, _)| candidate == identity)
        }) {
            return Some(entry.clone());
        }
        // An identity recorded before dotted names were displayed whole. Taken
        // only when exactly one tag had it, since the old form could collide.
        let mut legacy = entries()
            .filter(|entry| legacy_campaign_identity(entry).as_deref() == Some(identity));
        let entry = legacy.next()?;
        legacy
            .all(|other| other.key == entry.key)
            .then(|| entry.clone())
    }

    /// Rebuild the browser entry for a stashed new tag, and parse its bytes.
    ///
    /// `None` when this kit cannot place it: the group name, the template
    /// container and the parse all have to succeed, and the first two depend on
    /// how far the source has loaded. Shared by both restore paths -- the
    /// recovery file adopted at mount and `File > Open Baboon Project` -- because
    /// the entry a new tag is registered under decides whether it resolves at
    /// export, and two copies of that derivation is how one path came to build it
    /// and the other not to.
    fn new_overlay_entry(&self, kit: usize, overlay: &CampaignProjectOverlay) -> OverlayAdoption {
        // The names and the template come off the source: before it has
        // loaded, this is a "not yet" rather than a "no".
        if self.kits[kit].source.is_none() {
            return OverlayAdoption::NotYet;
        }
        let Some(group_name) = self.kits[kit]
            .names
            .name_for(overlay.group_tag)
            .map(str::to_owned)
        else {
            return OverlayAdoption::Failed(format!(
                "its group {} is not one this game's definitions know",
                format_group_tag(overlay.group_tag)
            ));
        };
        // A stashed tag of a group the game ships none of has no donor to point
        // back at, and recovering it must not depend on finding one — otherwise
        // the tag survives the save and vanishes on reopen.
        let template = match &overlay.copied_from {
            // A copy is wrapped in its source's own `.uasset`, wherever this
            // mount put it. Without the source there is no wrapper that is
            // right for it, so it waits rather than take on a donor's bindings.
            Some(source) => match self
                .campaign_entry_for_identity(kit, source)
                .map(|entry| entry.location)
            {
                Some(TagEntryLocation::Container {
                    container,
                    rel_path,
                }) => match rel_path.strip_suffix(".ubulk") {
                    Some(stem) => NewContainerTemplate::Copy {
                        container,
                        rel_path: format!("{stem}.uasset"),
                        source: source.clone(),
                    },
                    None => {
                        return OverlayAdoption::Failed(format!(
                            "{source}, the tag it was copied from, is not stored as a .ubulk"
                        ));
                    }
                },
                _ => {
                    return OverlayAdoption::Failed(format!(
                        "{source}, the tag it was copied from, is not in the mounted paks"
                    ));
                }
            },
            None => match crate::app::tag_ops::new_tag::new_container_template_for(
                self.find_container_template_in(kit, overlay.group_tag),
                &group_name,
            ) {
                Ok(template) => template,
                Err(error) => return OverlayAdoption::Failed(error),
            },
        };
        let tag = match TagFile::read_from_bytes(&overlay.bytes) {
            Ok(tag) => tag,
            Err(error) => {
                return OverlayAdoption::Failed(format!("its stashed bytes do not parse: {error}"));
            }
        };
        let extension = group_tag_to_extension(overlay.group_tag)
            .unwrap_or(group_name.as_str())
            .to_owned();
        let package = overlay
            .package
            .clone()
            .unwrap_or_else(|| format!("/Game/Tags/{}-{group_name}", overlay.logical_path));
        OverlayAdoption::Ready(
            TagEntry {
                key: crate::core::tag_key::new_tag_entry_key(&package),
                display_path: format!("{}.{}", overlay.logical_path, extension),
                group_tag: overlay.group_tag,
                group_name: Some(group_name),
                location: TagEntryLocation::NewContainer {
                    template,
                    package,
                    group_tag: overlay.group_tag,
                },
            },
            tag,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::documents::{PendingCloseAction, SaveChangesPrompt};
    use crate::app::loose_fixture::*;
    use crate::app::mods::{CampaignProjectSnapshot, CampaignProjectTagKind, campaign_entry_project_parts};
    use crate::core::test_kits::{compat_json, compat_samples, unique_temp_dir};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::time::Duration;

    /// A container tag with a dot in its name has a new identity now that its
    /// display path keeps the dot; a project written before still names it
    /// by the old one, and that still finds it, unless it is ambiguous.
    #[test]
    fn a_dotted_container_tag_answers_to_its_old_identity_too() {
        let bitm = u32::from_be_bytes(*b"bitm");
        let entry = |logical: &str| TagEntry {
            key: format!("ublock:pakchunk0:Meteorite/Content/Tags/{logical}-bitmap.ubulk"),
            // What the container loader now displays it as.
            display_path: format!("{logical}.bitmap"),
            group_tag: bitm,
            group_name: None,
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: format!("Meteorite/Content/Tags/{logical}-bitmap.ubulk"),
            },
        };
        let mut app = Baboon::for_test();
        let source = |entries: Vec<TagEntry>| LoadedSourceData {
            label: "ce".to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("/ce"),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries,
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        };
        app.install_loaded_source(source(vec![entry("levels/v1.2/bitmaps/rock")]));
        let found = |app: &Baboon, identity: &str| {
            app.model.campaign_entry_for_identity(0, identity)
                .map(|entry| entry.display_path)
        };
        assert_eq!(
            found(&app, "6269746d:levels/v1.2/bitmaps/rock").as_deref(),
            Some("levels/v1.2/bitmaps/rock.bitmap")
        );
        assert_eq!(
            found(&app, "6269746d:levels/v1").as_deref(),
            Some("levels/v1.2/bitmaps/rock.bitmap"),
            "the identity an older build recorded"
        );

        // Two tags that both had that old identity: neither is guessed.
        app.install_loaded_source(source(vec![
            entry("levels/v1.2/bitmaps/rock"),
            entry("levels/v1.3/bitmaps/rock"),
        ]));
        assert_eq!(found(&app, "6269746d:levels/v1"), None);
    }

    /// The revision has to keep climbing across a save, or a cache that saw
    /// revision 1, watched the document be saved and then edited again, would
    /// see revision 1 once more and skip work it needed to do.
    #[test]
    fn a_dirty_revision_never_repeats() {
        let mut dirty = Dirty::default();
        assert!(!dirty.is_set());
        dirty.touch();
        let first = dirty.revision();
        dirty.clear();
        assert!(!dirty.is_set());
        dirty.touch();
        assert!(dirty.is_set());
        assert_ne!(dirty.revision(), first);
    }

    fn overlay(identity: &str, bytes: &[u8]) -> CampaignProjectOverlay {
        CampaignProjectOverlay {
            identity: identity.to_owned(),
            group_tag: 0x1234_5678,
            logical_path: identity.to_owned(),
            kind: CampaignProjectTagKind::Existing,
            package: None,
            digest: overlay_digest(bytes),
            bytes: Arc::new(bytes.to_vec()),
            copied_from: None,
        }
    }

    fn history_of(sizes: &[usize]) -> TagHistory {
        TagHistory {
            undo: sizes
                .iter()
                .enumerate()
                .map(|(index, size)| HistoryStep {
                    id: index as u64 + 1,
                    label: format!("edit {index}"),
                    bytes: Arc::new(vec![0; *size]),
                })
                .collect(),
            redo: Vec::new(),
            revision: 1,
        }
    }

    #[test]
    fn history_is_trimmed_to_the_newest_steps() {
        let mut history = BTreeMap::from([("a".to_owned(), history_of(&[1; 20]))]);

        trim_history_for_disk(&mut history, 16, 1024);

        let kept: Vec<&str> = history["a"]
            .undo
            .iter()
            .map(|step| step.label.as_str())
            .collect();
        assert_eq!(kept.len(), 16);
        assert_eq!(
            kept.first().copied(),
            Some("edit 4"),
            "the oldest four steps are the ones dropped"
        );
        assert_eq!(kept.last().copied(), Some("edit 19"));
    }

    #[test]
    fn the_byte_budget_is_shared_across_every_open_tag() {
        // The budget bounds the recovery file, so it cannot be per tag — ten
        // tags each holding "only" their own allowance is ten times the file.
        let mut history = BTreeMap::from([
            ("a".to_owned(), history_of(&[400, 400, 400])),
            ("b".to_owned(), history_of(&[400, 400, 400])),
        ]);

        trim_history_for_disk(&mut history, 16, 1000);

        let total: usize = history
            .values()
            .flat_map(|entry| entry.undo.iter().chain(entry.redo.iter()))
            .map(|step| step.bytes.len())
            .sum();
        assert!(total <= 1000, "kept {total} bytes against a 1000 budget");
        // What survives is the newest of each tag, not one tag's whole stack.
        for identity in ["a", "b"] {
            assert_eq!(
                history[identity]
                    .undo
                    .last()
                    .map(|step| step.label.as_str()),
                Some("edit 2"),
                "{identity} kept its most recent step"
            );
        }
    }

    /// A save writes only the history steps it has not written before: an
    /// unchanged history not at all, and a stack that moved only its new step.
    /// Every step is a whole tag, and saves run twice a second while editing.
    #[test]
    fn a_save_writes_only_new_history_steps() {
        let path = temp_project("history-skip");
        let step = |id: u64, label: &str| HistoryStep {
            id,
            label: label.to_owned(),
            bytes: Arc::new(vec![id as u8; 3]),
        };
        let mut snapshot = snapshot_of(vec![overlay("a", b"one")]);
        snapshot.history = BTreeMap::from([(
            "a".to_owned(),
            TagHistory {
                undo: vec![step(1, "Edit color")],
                redo: Vec::new(),
                revision: 7,
            },
        )]);
        save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();
        let saved = snapshot.digests();

        // Mark the row on disk, so a rewrite of it is detectable.
        let mark = || {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute("UPDATE history_steps SET label = 'sentinel'", [])
                .unwrap();
        };
        mark();

        // The tag was edited again, but the journal did not move.
        let mut later = snapshot_of(vec![overlay("a", b"two")]);
        later.history = snapshot.history.clone();
        save_campaign_project(&path, &later, Some(&saved), ProjectScope::Session).unwrap();
        let loaded = load_campaign_project(&path).unwrap();
        assert_eq!(loaded.history["a"].undo[0].label, "sentinel", "rewritten");
        assert_eq!(*loaded.overlays["a"].bytes, b"two".to_vec());

        // A new edit: the old step moves below it and is not written again.
        let mut moved = later.clone();
        let entry = moved.history.get_mut("a").unwrap();
        entry.undo.push(step(2, "Edit name"));
        entry.undo.push(step(5, "Edit size"));
        entry.revision = 8;
        save_campaign_project(&path, &moved, Some(&later.digests()), ProjectScope::Session)
            .unwrap();
        let loaded = load_campaign_project(&path).unwrap();
        let labels: Vec<&str> = loaded.history["a"]
            .undo
            .iter()
            .map(|step| step.label.as_str())
            .collect();
        assert_eq!(
            labels,
            ["sentinel", "Edit name", "Edit size"],
            "only the new steps are written"
        );

        // A new edit pushes the two oldest past the budget and one is then
        // undone: step 5 moves from the top to the bottom, beneath a new
        // step, and the file must read back in the snapshot's order.
        let mut shifted = moved.clone();
        let entry = shifted.history.get_mut("a").unwrap();
        entry.undo = vec![step(5, "Edit size"), step(7, "Edit scale")];
        entry.redo = vec![step(8, "Edit scale")];
        entry.revision = 9;
        save_campaign_project(
            &path,
            &shifted,
            Some(&moved.digests()),
            ProjectScope::Session,
        )
        .unwrap();
        let loaded = load_campaign_project(&path).unwrap();
        let ids = |steps: &[HistoryStep]| steps.iter().map(|step| step.id).collect::<Vec<_>>();
        assert_eq!(ids(&loaded.history["a"].undo), [5, 7]);
        assert_eq!(ids(&loaded.history["a"].redo), [8]);

        let _ = fs::remove_file(&path);
    }

    /// A recovery file from before steps had ids keeps its history in the old
    /// table. It still opens with its history, and the next save moves it
    /// across rather than trusting a row map that does not describe it.
    #[test]
    fn history_in_the_old_table_is_read_and_moved_across() {
        let path = temp_project("history-legacy");
        let snapshot = snapshot_of(vec![overlay("a", b"one")]);
        save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS history (
                     identity TEXT NOT NULL, stack TEXT NOT NULL, position INTEGER NOT NULL,
                     label TEXT NOT NULL, bytes BLOB NOT NULL,
                     PRIMARY KEY (identity, stack, position));
                 INSERT INTO history VALUES ('a', 'undo', 0, 'Old edit', x'0102');",
            )
            .unwrap();
        drop(connection);

        let loaded = load_campaign_project(&path).unwrap();
        assert_eq!(loaded.history["a"].undo[0].label, "Old edit");
        // What the app believes is on disk after adopting the file.
        let adopted = loaded.digests();
        let mut next = loaded.clone();
        next.history.get_mut("a").unwrap().revision += 1;
        save_campaign_project(&path, &next, Some(&adopted), ProjectScope::Session).unwrap();

        let connection = Connection::open(&path).unwrap();
        let legacy: i64 = connection
            .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(legacy, 0, "the old table is cleared");
        drop(connection);
        let reloaded = load_campaign_project(&path).unwrap();
        assert_eq!(
            reloaded.history["a"].undo[0].label, "Old edit",
            "and nothing lost"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_tag_trimmed_to_nothing_leaves_no_row_behind() {
        let mut history = BTreeMap::from([("a".to_owned(), TagHistory::default())]);
        trim_history_for_disk(&mut history, 16, 1000);
        assert!(history.is_empty());
    }

    fn snapshot_of(overlays: Vec<CampaignProjectOverlay>) -> CampaignProjectSnapshot {
        CampaignProjectSnapshot {
            game: "haloce_evolved".to_owned(),
            source_path: PathBuf::from("Paks"),
            selected_identity: None,
            tabs: Vec::new(),
            overlays: overlays
                .into_iter()
                .map(|overlay| (overlay.identity.clone(), overlay))
                .collect(),
            history: BTreeMap::new(),
            folders: Default::default(),
        }
    }

    /// A save must rewrite only the overlays whose bytes changed. Stashing a
    /// 105 MiB animation graph alongside a 4 MiB scenario meant every edit to
    /// the scenario rewrote both.
    #[test]
    fn a_save_rewrites_only_the_overlays_whose_bytes_changed() {
        let path = std::env::temp_dir().join(format!(
            "baboon-project-diff-{}-{}.baboon",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = snapshot_of(vec![overlay("a", b"one"), overlay("b", b"two")]);
        save_campaign_project(&path, &first, None, ProjectScope::Session).unwrap();

        // "a" is unchanged, "b" is gone, "c" is new. The claim that "a" is
        // already on disk is honoured by digest, so passing different bytes
        // under its old digest proves the row really was skipped.
        let mut stale = overlay("a", b"REWRITTEN");
        stale.digest = overlay_digest(b"one");
        let second = snapshot_of(vec![stale, overlay("c", b"three")]);
        save_campaign_project(
            &path,
            &second,
            Some(&first.digests()),
            ProjectScope::Session,
        )
        .unwrap();

        let loaded = load_campaign_project(&path).unwrap();
        let mut identities: Vec<&String> = loaded.overlays.keys().collect();
        identities.sort();
        assert_eq!(identities, vec!["a", "c"], "b was dropped, c was added");
        assert_eq!(
            loaded.overlays["a"].bytes.as_slice(),
            b"one",
            "a's row was left alone"
        );
        assert_eq!(loaded.overlays["c"].bytes.as_slice(), b"three");
        let _ = std::fs::remove_file(&path);
    }

    /// The fingerprint is what autosave compares to decide whether to write, so
    /// it has to notice a changed overlay while never touching its bytes.
    #[test]
    fn the_fingerprint_follows_the_digest() {
        let before = snapshot_of(vec![overlay("a", b"one")]);
        let same = snapshot_of(vec![overlay("a", b"one")]);
        let changed = snapshot_of(vec![overlay("a", b"other")]);
        assert_eq!(before.fingerprint(), same.fingerprint());
        assert_ne!(before.fingerprint(), changed.fingerprint());
    }

    fn temp_project(name: &str) -> PathBuf {
        crate::core::test_kits::unique_temp_path(name).with_extension("baboon")
    }

    fn identities_in(path: &Path) -> Vec<String> {
        let mut identities: Vec<String> = load_campaign_project(path)
            .unwrap()
            .overlays
            .into_keys()
            .collect();
        identities.sort();
        identities
    }

    /// A `.baboon` the user opened is not what the workspace writes to. It was,
    /// and so an exported mod's sidecar became a live file the moment it was
    /// opened: autosave rewrote it, and declining to save at exit deleted the
    /// stashed rows straight out of it — which is how an exported mod's project
    /// came back empty two sessions later.
    #[test]
    fn an_opened_project_is_never_the_autosave_target() {
        let recovery = temp_project("recovery");
        let opened = temp_project("opened");
        let snapshot = snapshot_of(vec![overlay("a", b"one")]);
        let project =
            ActiveCampaignProject::imported(recovery.clone(), opened.clone(), &snapshot, 0.0);
        assert_eq!(project.recovery_path, recovery);
        assert_eq!(project.project_path, Some(opened.clone()));
        assert_eq!(
            project.label(),
            opened.file_name().unwrap().to_string_lossy(),
            "the workspace is labelled with the project the user opened"
        );
    }

    /// An imported project's overlays have never been in the recovery file, so
    /// neither the digests nor the fingerprint may claim they have. Either one
    /// would let the first autosave decide there was nothing to write — leaving
    /// the workspace's live copy as whatever the last session left there.
    #[test]
    fn an_imported_project_replaces_a_stale_recovery_file() {
        let recovery = temp_project("stale-recovery");
        // What an earlier session of this workspace left behind.
        save_campaign_project(
            &recovery,
            &snapshot_of(vec![overlay("old", b"x")]),
            None,
            ProjectScope::Session,
        )
        .unwrap();

        let imported = snapshot_of(vec![overlay("new", b"y")]);
        let project = ActiveCampaignProject::imported(
            recovery.clone(),
            temp_project("opened"),
            &imported,
            0.0,
        );
        assert!(
            project.saved_digests.is_none(),
            "nothing is known about the recovery file, so it must be replaced whole"
        );
        assert_ne!(
            project.last_saved_fingerprint,
            imported.fingerprint(),
            "the recovery file does not hold these bytes yet, so a write is due"
        );
        // Exactly what `checkpoint_campaign_project` then does with them.
        save_campaign_project(
            &recovery,
            &imported,
            project.saved_digests.as_ref(),
            ProjectScope::Session,
        )
        .unwrap();
        assert_eq!(
            identities_in(&recovery),
            vec!["new"],
            "the stale row was replaced, not merged into"
        );
        let _ = fs::remove_file(&recovery);
    }

    fn autosave_of(project: &ActiveCampaignProject, revision: u64) -> CampaignProjectWrite {
        let snapshot = snapshot_of(vec![overlay("a", b"one")]);
        CampaignProjectWrite {
            revision,
            path: project.recovery_path.clone(),
            fingerprint: snapshot.fingerprint(),
            snapshot,
            on_disk: None,
            write_lock: project.write_lock.clone(),
            latest_write_revision: project.latest_write_revision.clone(),
        }
    }

    /// An autosave that panicked sent nothing, so its save stayed in flight and
    /// every later autosave was skipped for the session.
    #[test]
    fn an_autosave_that_panics_is_no_longer_in_flight() {
        let mut app = Baboon::for_test();
        let recovery = temp_project("panicking-autosave");
        let mut project = ActiveCampaignProject::adopted(recovery.clone(), &snapshot_of(Vec::new()), 0.0);
        project.save_in_flight = Some(3);
        project.latest_write_revision.store(3, Ordering::SeqCst);
        let write = autosave_of(&project, 3);
        app.model.kits[0].project.active = Some(project);

        let ctx = egui::Context::default();
        crate::app::shell::with_panicking_workers(|| {
            write_campaign_project_in_background(&app.tx, &ctx, write)
        });
        assert!(
            crate::app::shell::apply_next_worker_message(&mut app),
            "the autosave answered"
        );
        let project = app.model.kits[0].project.active.as_ref().unwrap();
        assert_eq!(project.save_in_flight, None);
        assert!(app.model.status.contains("crashed"), "{}", app.model.status);
        let _ = fs::remove_file(&recovery);
    }

    /// The writer lock guards only the order of writes. A writer that panicked
    /// while holding it poisoned it, and every later checkpoint then failed.
    #[test]
    fn a_poisoned_writer_lock_does_not_stop_later_saves() {
        let recovery = temp_project("poisoned-lock");
        let project = ActiveCampaignProject::adopted(recovery.clone(), &snapshot_of(Vec::new()), 0.0);
        project.latest_write_revision.store(1, Ordering::SeqCst);
        let lock = project.write_lock.clone();
        let _ = std::thread::spawn(move || {
            let _guard = lock.lock().unwrap();
            panic!("a writer fell over while holding the lock");
        })
        .join();
        assert!(project.write_lock.is_poisoned());

        let (tx, rx) = std::sync::mpsc::channel();
        write_campaign_project_in_background(&tx, &egui::Context::default(), autosave_of(&project, 1));
        let Ok(WorkerMessage::CampaignProjectSaved { result, .. }) =
            rx.recv_timeout(std::time::Duration::from_secs(10))
        else {
            panic!("the autosave did not answer");
        };
        let identities = load_campaign_project(&recovery).map(|loaded| loaded.overlays.len());
        let _ = fs::remove_file(&recovery);
        assert_eq!(result, Ok(()));
        assert_eq!(identities.ok(), Some(1), "the write reached the file");
    }

    /// The recovery file the workspace autosaves to is picked back up as-is, so
    /// it needs neither a rewrite nor a full replace until something changes.
    #[test]
    fn an_adopted_recovery_file_is_believed() {
        let recovery = temp_project("adopted");
        let snapshot = snapshot_of(vec![overlay("a", b"one")]);
        let project = ActiveCampaignProject::adopted(recovery, &snapshot, 0.0);
        assert_eq!(project.saved_digests, Some(snapshot.digests()));
        assert_eq!(project.last_saved_fingerprint, snapshot.fingerprint());
        assert_eq!(project.project_path, None);
        assert_eq!(
            project.label(),
            "unsaved",
            "a recovery file is not a project the user named"
        );
    }

    /// Baboon's own recovery files are not save targets, and a session that
    /// recorded one — every session written while the two were the same file —
    /// must not come back reading as though the user had a project open.
    #[test]
    fn recovery_files_are_recognized_as_baboons_own() {
        assert!(is_campaign_recovery_file(&campaign_recovery_path(Some(
            Path::new("/games/evolved/Paks")
        ))));
        assert!(is_campaign_recovery_file(&campaign_recovery_path(None)));
        assert!(!is_campaign_recovery_file(Path::new(
            "/games/evolved/Paks/~mods/mymod_P.baboon"
        )));
    }

    /// Making a folder changes no overlay, no tab and no history, so if the
    /// autosave fingerprint ignored it the write would be skipped and the
    /// folder would be gone at the next launch — after looking, all session,
    /// exactly as though it had been saved.
    #[test]
    fn the_autosave_fingerprint_notices_a_folder_only_change() {
        let base = snapshot_of(Vec::new());
        let mut with_folder = base.clone();
        with_folder.folders = ["objects/vehicles".to_owned()].into_iter().collect();
        assert_ne!(base.fingerprint(), with_folder.fingerprint());

        // And a different folder is a different session.
        let mut other = base.clone();
        other.folders = ["objects/characters".to_owned()].into_iter().collect();
        assert_ne!(with_folder.fingerprint(), other.fingerprint());

        // Same set, same fingerprint — otherwise every tick would rewrite.
        let mut repeat = base.clone();
        repeat.folders = ["objects/vehicles".to_owned()].into_iter().collect();
        assert_eq!(with_folder.fingerprint(), repeat.fingerprint());
    }

    /// A `.baboon` written before folders existed must still open.
    ///
    /// This is why `CAMPAIGN_PROJECT_VERSION` is not bumped for the new table:
    /// the version check is a strict equality, so raising it would reject every
    /// project already on disk rather than migrate it. The table is purely
    /// additive, so an older build ignores it and a newer one reads its absence
    /// as "no folders".
    #[test]
    fn a_project_written_before_the_folders_table_still_opens() {
        let path = std::env::temp_dir().join(format!(
            "baboon-project-v1-{}-{}.baboon",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // The v1 schema, written literally — no `folders` table.
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE project (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     version INTEGER NOT NULL,
                     game TEXT NOT NULL,
                     source_path TEXT NOT NULL,
                     selected_identity TEXT
                 );
                 CREATE TABLE tabs (
                     position INTEGER PRIMARY KEY,
                     identity TEXT NOT NULL UNIQUE,
                     label TEXT NOT NULL,
                     group_tag INTEGER NOT NULL,
                     logical_path TEXT NOT NULL,
                     kind TEXT NOT NULL,
                     package TEXT,
                     floating INTEGER NOT NULL
                 );
                 CREATE TABLE overlays (
                     identity TEXT PRIMARY KEY,
                     group_tag INTEGER NOT NULL,
                     logical_path TEXT NOT NULL,
                     kind TEXT NOT NULL,
                     package TEXT,
                     bytes BLOB NOT NULL
                 );
                 CREATE TABLE history (
                     identity TEXT NOT NULL,
                     stack TEXT NOT NULL,
                     position INTEGER NOT NULL,
                     label TEXT NOT NULL,
                     bytes BLOB NOT NULL,
                     PRIMARY KEY (identity, stack, position)
                 );
                 INSERT INTO project (id, version, game, source_path, selected_identity)
                 VALUES (1, 1, 'haloce_evolved', 'Paks', NULL);",
            )
            .unwrap();
        drop(connection);

        let loaded = load_campaign_project(&path).expect("a v1 project still opens");
        assert!(loaded.folders.is_empty());

        // And saving it forward adds the table without disturbing anything.
        let mut forward = loaded.clone();
        forward.folders = ["objects/vehicles".to_owned()].into_iter().collect();
        save_campaign_project(&path, &forward, None, ProjectScope::Session).unwrap();
        let reloaded = load_campaign_project(&path).unwrap();
        assert_eq!(reloaded.folders, forward.folders);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn campaign_project_round_trips_binary_overlays_and_tab_order() {
        let path = std::env::temp_dir().join(format!(
            "baboon-project-{}-{}.baboon",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let overlay = CampaignProjectOverlay {
            identity: "12345678:objects/test".to_owned(),
            group_tag: 0x1234_5678,
            logical_path: "objects/test".to_owned(),
            kind: CampaignProjectTagKind::Existing,
            package: None,
            digest: overlay_digest(&[0, 1, 2, 0xff]),
            bytes: Arc::new(vec![0, 1, 2, 0xff]),
            copied_from: None,
        };
        let snapshot = CampaignProjectSnapshot {
            game: "haloce_evolved".to_owned(),
            source_path: PathBuf::from("Paks"),
            selected_identity: Some(overlay.identity.clone()),
            tabs: vec![CampaignProjectTab {
                identity: overlay.identity.clone(),
                label: "objects/test.weapon".to_owned(),
                group_tag: overlay.group_tag,
                logical_path: overlay.logical_path.clone(),
                kind: overlay.kind,
                package: None,
                floating: false,
            }],
            overlays: HashMap::from([(overlay.identity.clone(), overlay.clone())]),
            history: BTreeMap::from([(
                overlay.identity.clone(),
                TagHistory {
                    undo: vec![HistoryStep {
                        id: 1,
                        label: "Edit color".to_owned(),
                        bytes: Arc::new(vec![7, 7, 7]),
                    }],
                    redo: vec![HistoryStep {
                        id: 2,
                        label: "Block edit".to_owned(),
                        bytes: Arc::new(vec![9]),
                    }],
                    revision: 3,
                },
            )]),
            folders: ["objects/vehicles".to_owned(), "sound/new".to_owned()]
                .into_iter()
                .collect(),
        };
        save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();
        let loaded = load_campaign_project(&path).unwrap();
        assert_eq!(loaded.tabs.len(), 1);
        assert_eq!(loaded.tabs[0].identity, overlay.identity);
        assert_eq!(
            loaded.overlays[&loaded.tabs[0].identity].bytes,
            overlay.bytes
        );
        // The session, not just the files: which tags were open, what was
        // edited, and the steps that got there.
        let restored = &loaded.history[&overlay.identity];
        assert_eq!(restored.undo.len(), 1);
        assert_eq!(restored.undo[0].label, "Edit color");
        assert_eq!(*restored.undo[0].bytes, vec![7, 7, 7]);
        assert_eq!(restored.redo.len(), 1);
        assert_eq!(restored.redo[0].label, "Block edit");
        assert_eq!(*restored.redo[0].bytes, vec![9]);
        // A folder no tag has landed in exists nowhere but the workspace, so
        // without this it is gone on the next launch.
        assert_eq!(loaded.folders, snapshot.folders);

        // The same snapshot written as a mod's sidecar carries the tags and
        // nothing about how they were arrived at: that file is downloaded by
        // whoever installs the mod.
        let sidecar = path.with_extension("sidecar.baboon");
        save_campaign_project(&sidecar, &snapshot, None, ProjectScope::ModSidecar).unwrap();
        let published = load_campaign_project(&sidecar).unwrap();
        assert!(
            published.history.is_empty(),
            "an exported mod must not ship the author's undo history"
        );
        assert!(
            published.folders.is_empty(),
            "an exported mod must not ship the author's workspace folders"
        );
        assert_eq!(published.overlays.len(), snapshot.overlays.len());
        let _ = fs::remove_file(&sidecar);

        let connection = Connection::open(&path).unwrap();
        connection
            .execute("UPDATE project SET version = 99 WHERE id = 1", [])
            .unwrap();
        drop(connection);
        assert!(
            load_campaign_project(&path)
                .unwrap_err()
                .contains("Unsupported Baboon project version")
        );
        let _ = fs::remove_file(path);
    }

    /// Export resolves each stashed overlay back to a tag by identity string,
    /// taking the first entry that produces a match. Two tags sharing an
    /// identity would therefore send one tag's edited bytes to the other's
    /// path in the container -- a mod that builds and does the wrong thing, or
    /// nothing.
    #[test]
    fn container_tag_identities_are_unique() {
        static PAKS: std::sync::LazyLock<&'static str> =
            std::sync::LazyLock::new(|| crate::core::test_kits::leak(crate::core::test_kits::ce_paks()));
        if !std::path::Path::new(*PAKS).exists() {
            eprintln!("skipping: Campaign Evolved not present");
            return;
        }
        let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = crate::core::format::TagNameIndex::load_from_definitions(&defs);
        let loaded = crate::core::source::load_iostore_container_set(
            std::path::PathBuf::from(*PAKS),
            &names,
            &defs,
        )
        .expect("mount container set");

        let mut seen: HashMap<String, String> = HashMap::new();
        let mut collisions = Vec::new();
        let mut identified = 0usize;
        for entry in loaded.entries.iter().chain(loaded.all_entries.iter()) {
            let Some((identity, ..)) = campaign_entry_project_parts(entry) else {
                continue;
            };
            identified += 1;
            let location = match &entry.location {
                TagEntryLocation::Container {
                    container,
                    rel_path,
                } => format!("container {container}: {rel_path}"),
                TagEntryLocation::NewContainer { package, .. } => format!("new: {package}"),
                _ => "other".to_owned(),
            };
            match seen.get(&identity) {
                Some(existing) if *existing != location => {
                    collisions.push(format!("{identity}: {existing} vs {location}"));
                }
                Some(_) => {}
                None => {
                    seen.insert(identity, location);
                }
            }
        }
        eprintln!("{identified} identified tag(s), {} distinct", seen.len());
        assert!(
            collisions.is_empty(),
            "{} identity collision(s):\n{}",
            collisions.len(),
            collisions
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    // Every saved format Baboon reads, fed through the real readers from the
    // synthetic samples in `testdata/compat` (see its README; regenerate with
    // `gen_samples.py`). Old files must keep loading, files a newer build wrote
    // must not be destroyed by this one, and the cases a reader refuses are
    // pinned beside the ones it accepts, so a reader that accepted everything
    // would fail here too.

    #[test]
    fn compat_projects() {
        use crate::app::mods::project::{
            ProjectScope, is_campaign_recovery_file, load_campaign_project, save_campaign_project,
        };
        let project = compat_samples().join("project");
        let recovery = project.join("campaign_evolved_recovery-46ec1ffb674b.baboon");
        assert!(is_campaign_recovery_file(&recovery));
        assert!(!is_campaign_recovery_file(
            &project.join("user_project.history_table_only.baboon")
        ));
        let snap = load_campaign_project(&recovery).expect("recovery");
        assert_eq!(snap.game, "haloce_evolved");
        assert_eq!(snap.tabs.len(), 4);
        assert_eq!(snap.overlays.len(), 2);
        assert_eq!(snap.folders.len(), 2);
        let marine = "62697064:objects/characters/marine/marine";
        assert_eq!(snap.selected_identity.as_deref(), Some(marine));
        let history = &snap.history[marine];
        assert_eq!(
            (history.undo.len(), history.redo.len()),
            (2, 1),
            "a stack name this build does not know is skipped"
        );
        // The recovery file is named by sha256(source_path); the loader compares
        // source_path exactly, so renormalizing the root orphans the file.
        let expected = {
            use sha2::Digest;
            let digest = sha2::Sha256::digest(snap.source_path.to_string_lossy().as_bytes());
            digest[..6]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        assert_eq!(expected, "46ec1ffb674b");

        let scratch = unique_temp_dir("project");
        let out = scratch.join("roundtrip.baboon");
        save_campaign_project(&out, &snap, None, ProjectScope::Session).unwrap();
        let back = load_campaign_project(&out).unwrap();
        let identities =
            |snap: &CampaignProjectSnapshot| snap.tabs.iter().map(|tab| tab.identity.clone()).collect::<Vec<_>>();
        assert_eq!(identities(&back), identities(&snap));
        assert_eq!(back.folders, snap.folders);
        let _ = std::fs::remove_dir_all(&scratch);

        let legacy = load_campaign_project(&project.join("user_project.history_table_only.baboon"))
            .expect("history table only");
        assert_eq!(legacy.history[marine].undo.len(), 1);
        let original = load_campaign_project(&project.join("user_project.original_v1_schema.baboon"))
            .expect("original schema");
        assert!(original.history.is_empty() && original.folders.is_empty());
        for rejected in [
            "rejected.version2.baboon",
            "rejected.game_halo3_mcc.baboon",
            "rejected.unknown_kind.baboon",
        ] {
            assert!(
                load_campaign_project(&project.join(rejected)).is_err(),
                "{rejected} must be refused"
            );
        }
    }

    /// A Campaign Evolved tag's project identity is `{group:08x}:{logical path}`,
    /// the display path lowered without its extension. Dotted names keep their
    /// dots (bb6315b); an identity written before that, cut at the last dot, still
    /// finds its tag when only one tag had it.
    #[test]
    fn compat_campaign_identities() {
        let container = |logical: &str, group: &[u8; 4], extension: &str| TagEntry {
            key: crate::core::source::container_entry_key(
                "pakchunk0-WinGDK",
                &format!("Meteorite/Content/Tags/{logical}-{extension}.ubulk"),
            ),
            display_path: format!("{logical}.{extension}"),
            group_tag: u32::from_be_bytes(*group),
            group_name: Some(extension.to_owned()),
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: format!("Meteorite/Content/Tags/{logical}-{extension}.ubulk"),
            },
        };
        let identity = |entry: &TagEntry| campaign_entry_project_parts(entry).unwrap().0;
        assert_eq!(
            identity(&container("objects/characters/marine/marine", b"bipd", "biped")),
            "62697064:objects/characters/marine/marine"
        );
        assert_eq!(
            identity(&container("Levels/V1.2/Bitmaps/Rock", b"bitm", "bitmap")),
            "6269746d:levels/v1.2/bitmaps/rock"
        );
        assert_eq!(
            identity(&container("sound/machines/piston_close2.l", b"snd!", "sound")),
            "736e6421:sound/machines/piston_close2.l"
        );
        let package = "/Game/Tags/objects/foo/bar-camera_track";
        assert_eq!(
            crate::core::tag_key::new_tag_entry_key(package),
            compat_json("tag_keys.json")["newtag_ce"].as_str().unwrap()
        );
        let authored = TagEntry {
            key: crate::core::tag_key::new_tag_entry_key(package),
            display_path: "objects/foo/bar.camera_track".to_owned(),
            group_tag: u32::from_be_bytes(*b"trak"),
            group_name: Some("camera_track".to_owned()),
            location: TagEntryLocation::NewContainer {
                template: crate::core::source::NewContainerTemplate::Derived {
                    group: "camera_track".to_owned(),
                },
                package: package.to_owned(),
                group_tag: u32::from_be_bytes(*b"trak"),
            },
        };
        let (new_identity, _, kind, new_package) = campaign_entry_project_parts(&authored).unwrap();
        assert_eq!(new_identity, "7472616b:objects/foo/bar");
        assert_eq!(kind, CampaignProjectTagKind::New);
        assert_eq!(new_package.as_deref(), Some(package));

        // The project file holding both spellings, against a mounted source with
        // one dotted tag in it.
        let snap = crate::app::mods::project::load_campaign_project(
            &compat_samples().join("project/user_project.dotted_identities.baboon"),
        )
        .expect("dotted identities");
        let tabs: Vec<&str> = snap.tabs.iter().map(|tab| tab.identity.as_str()).collect();
        assert_eq!(
            tabs,
            [
                "6269746d:levels/v1.2/bitmaps/rock",
                "6269746d:levels/v1",
                "736e6421:sound/machines/piston_close2.l",
            ]
        );
        let mut app = Baboon::for_test();
        let mounted = |entries: Vec<TagEntry>| LoadedSourceData {
            label: "ce".to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("/ce"),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries,
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        };
        let rock = container("levels/v1.2/bitmaps/rock", b"bitm", "bitmap");
        app.install_loaded_source(mounted(vec![rock.clone()]));
        for tab in &tabs[..2] {
            assert_eq!(
                app.model.campaign_entry_for_identity(0, tab).map(|entry| entry.key),
                Some(rock.key.clone()),
                "{tab}"
            );
        }
        assert!(app.model.campaign_entry_for_identity(0, tabs[2]).is_none());
        // Two tags that had the same old identity: neither is guessed.
        app.install_loaded_source(mounted(vec![
            rock.clone(),
            container("levels/v1.3/bitmaps/rock", b"bitm", "bitmap"),
        ]));
        assert!(app.model.campaign_entry_for_identity(0, tabs[1]).is_none());
        assert!(app.model.campaign_entry_for_identity(0, tabs[0]).is_some());
    }

    /// A stashed new tag that can never be placed leaves the retry queue and
    /// says why. It used to stay queued, and adoption runs every frame, so it
    /// redid the entry scans and the parse every frame for the whole session.
    #[test]
    fn an_overlay_that_cannot_be_placed_is_not_retried_every_frame() {
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
        let mut project = ActiveCampaignProject::fresh(PathBuf::from("recovery.baboon"), 0.0);
        project.pending_new_overlays.push(CampaignProjectOverlay {
            identity: "tag:unknown".to_owned(),
            group_tag: u32::from_be_bytes(*b"zzzz"),
            logical_path: "objects/unknown".to_owned(),
            kind: CampaignProjectTagKind::New,
            package: None,
            bytes: Arc::new(Vec::new()),
            digest: [0; 32],
            copied_from: None,
        });
        app.model.kits[0].project.active = Some(project);

        app.adopt_pending_new_overlays(0);

        let queue = &app.model.kits[0]
            .project.active
            .as_ref()
            .unwrap()
            .pending_new_overlays;
        assert!(queue.is_empty(), "dropped from the retry queue");
        assert!(app.model.status.contains("Could not restore 1"), "{}", app.model.status);
    }

    // Characterization of Campaign Evolved project persistence and mod-export
    // review, without an install.
    //
    // The source is a container set with no containers mounted: its browser
    // entries are container tags, and their documents are built from the
    // Campaign Evolved definitions. That reaches everything that works off the
    // entries and documents -- capture, the recovery file, autosave, the stash,
    // the close prompt's stash and discard, the export review -- and stops where
    // a real `.utoc` would be read or written: building a mod container, and
    // telling a stashed tag apart from the shipped one, need an install
    // (`mod_override_tests.rs` covers those against `BLAM_TEST_CE`).

    const GROUP: &str = "point_physics";
    const FRICTION: &str = "air friction";

    /// A stand-in `Paks` directory, and the recovery file Baboon keys off it.
    struct CeKit {
        root: PathBuf,
    }

    impl CeKit {
        fn new(name: &str) -> Self {
            let root = fs::canonicalize(crate::core::test_kits::unique_temp_dir(name)).unwrap();
            Self { root }
        }

        fn recovery(&self) -> PathBuf {
            campaign_recovery_path(Some(&self.root))
        }

        fn entry(&self, path: &str) -> TagEntry {
            TagEntry {
                key: format!("ublock:pakchunk0:{path}"),
                display_path: format!("{path}.{GROUP}"),
                group_tag: group_tag("haloce_evolved", GROUP),
                group_name: Some(GROUP.to_owned()),
                location: TagEntryLocation::Container {
                    container: 0,
                    rel_path: format!("Tags/{path}-{GROUP}.ubulk"),
                },
            }
        }

        /// A container tag's project identity: its group, then its path without
        /// the extension.
        fn identity(&self, path: &str) -> String {
            format!("{:08x}:{path}", group_tag("haloce_evolved", GROUP))
        }

        /// An app with this source installed and `paths` open, the first one
        /// selected.
        fn app(&self, paths: &[&str]) -> Baboon {
            let entries: Vec<TagEntry> = paths.iter().map(|path| self.entry(path)).collect();
            let mut app = Baboon::for_test();
            app.install_loaded_source(LoadedSourceData {
                label: "Campaign Evolved".to_owned(),
                source: TagSource::IoStoreContainerSet {
                    root: self.root.clone(),
                    containers: Vec::new(),
                    index: Default::default(),
                    packages: Default::default(),
                    shipped: Default::default(),
                },
                names: TagNameIndex::load_game(&locate_definitions_root(), GameId::CampaignEvolved)
                    .unwrap(),
                game: Some(GameId::CampaignEvolved),
                entries: entries.clone(),
                tree: TagTree::default(),
                group_tree: TagTree::default(),
                all_entries: entries.clone(),
                reverse_dependencies: None,
                initial_tag: None,
                key_hints: Default::default(),
                complete_scan: false,
                chosen_kit_layout: None,
            });
            for entry in entries.iter().rev() {
                let tag = TagFile::new(definition("haloce_evolved", GROUP)).unwrap();
                app.model.kits[0]
                    .parsed_tags
                    .insert(entry.key.clone(), TagDocument::clean(tag));
                app.kit_and_view(0).open_tag_pane(&entry.key);
            }
            app.model.kits[0].selected_key = Some(entries[0].key.clone());
            app
        }
    }

    impl Drop for CeKit {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.recovery());
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn key(path: &str) -> String {
        format!("ublock:pakchunk0:{path}")
    }

    fn friction_in(bytes: &[u8]) -> Option<f32> {
        real_of(&TagFile::read_from_bytes(bytes).ok()?, FRICTION)
    }

    /// One frame at `time` running the autosave.
    fn autosave_at(app: &mut Baboon, ctx: &egui::Context, time: f64) {
        let _ = crate::app::run_ui_test(&ctx, screen(Vec::new(), time), |ui| {
            app.maybe_autosave_campaign_projects(ui.ctx())
        });
    }

    fn project(app: &Baboon) -> &ActiveCampaignProject {
        app.model.kits[0].project.active.as_ref().expect("a project")
    }

    fn step(id: u64, label: &str, bytes: &[u8]) -> HistoryStep {
        HistoryStep {
            id,
            label: label.to_owned(),
            bytes: Arc::new(bytes.to_vec()),
        }
    }

    fn kind_overlay(identity: &str, kind: CampaignProjectTagKind, bytes: &[u8]) -> CampaignProjectOverlay {
        let (_, logical_path) = identity.split_once(':').unwrap();
        CampaignProjectOverlay {
            identity: identity.to_owned(),
            group_tag: 0x7070_6879,
            logical_path: logical_path.to_owned(),
            kind,
            package: (kind == CampaignProjectTagKind::New)
                .then(|| format!("/Game/Tags/{logical_path}")),
            digest: overlay_digest(bytes),
            bytes: Arc::new(bytes.to_vec()),
            copied_from: None,
        }
    }

    /// `snapshot` as it reads back: history revisions are not stored.
    fn without_revisions(snapshot: &CampaignProjectSnapshot) -> CampaignProjectSnapshot {
        let mut snapshot = snapshot.clone();
        for history in snapshot.history.values_mut() {
            history.revision = 0;
        }
        snapshot
    }

    /// Everything a project holds, written and read back.
    #[test]
    fn a_saved_project_loads_back_field_for_field() {
        let kit = CeKit::new("project-round-trip");
        let existing = "70706879:objects/rock.point_physics";
        let new = "70706879:objects/mine/pebble.point_physics";
        let binary: Vec<u8> = (0..=255).chain([0, 0, 255]).collect();
        let snapshot = CampaignProjectSnapshot {
            game: "haloce_evolved".to_owned(),
            source_path: kit.root.clone(),
            selected_identity: Some(new.to_owned()),
            tabs: vec![
                CampaignProjectTab {
                    identity: new.to_owned(),
                    label: "objects/mine/pebble.point_physics".to_owned(),
                    group_tag: 0x7070_6879,
                    logical_path: "objects/mine/pebble.point_physics".to_owned(),
                    kind: CampaignProjectTagKind::New,
                    package: Some("/Game/Tags/objects/mine/pebble-point_physics".to_owned()),
                    floating: false,
                },
                CampaignProjectTab {
                    identity: existing.to_owned(),
                    label: "objects/rock.point_physics".to_owned(),
                    group_tag: 0x7070_6879,
                    logical_path: "objects/rock.point_physics".to_owned(),
                    kind: CampaignProjectTagKind::Existing,
                    package: None,
                    floating: false,
                },
            ],
            overlays: HashMap::from([
                (
                    existing.to_owned(),
                    kind_overlay(existing, CampaignProjectTagKind::Existing, &binary),
                ),
                (
                    new.to_owned(),
                    kind_overlay(new, CampaignProjectTagKind::New, b"new tag bytes"),
                ),
            ]),
            history: BTreeMap::from([(
                existing.to_owned(),
                TagHistory {
                    undo: vec![step(1, "Edit", b"first"), step(2, "Edit", b"second")],
                    redo: vec![step(3, "Block edit", b"undone")],
                    revision: 7,
                },
            )]),
            folders: BTreeSet::from(["objects/mine".to_owned(), "levels/new".to_owned()]),
        };
        let path = kit.root.join("project.baboon");

        save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();
        let loaded = load_campaign_project(&path).unwrap();

        assert_eq!(loaded.game, "haloce_evolved");
        assert_eq!(loaded.source_path, kit.root);
        assert_eq!(loaded.selected_identity.as_deref(), Some(new));
        let tabs = |snapshot: &CampaignProjectSnapshot| {
            snapshot
                .tabs
                .iter()
                .map(|tab| {
                    (
                        tab.identity.clone(),
                        tab.label.clone(),
                        tab.group_tag,
                        tab.logical_path.clone(),
                        tab.kind,
                        tab.package.clone(),
                        tab.floating,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(tabs(&loaded), tabs(&snapshot), "tabs, in order");
        assert_eq!(loaded.overlays.len(), 2);
        for (identity, expected) in &snapshot.overlays {
            let got = &loaded.overlays[identity];
            assert_eq!(*got.bytes, *expected.bytes, "{identity}");
            assert_eq!(got.digest, expected.digest);
            assert_eq!(got.kind, expected.kind);
            assert_eq!(got.package, expected.package);
            assert_eq!(got.logical_path, expected.logical_path);
            assert_eq!(got.group_tag, expected.group_tag);
        }
        let history = &loaded.history[existing];
        let steps = |steps: &[HistoryStep]| {
            steps
                .iter()
                .map(|step| (step.id, step.label.clone(), step.bytes.to_vec()))
                .collect::<Vec<_>>()
        };
        assert_eq!(steps(&history.undo), steps(&snapshot.history[existing].undo));
        assert_eq!(steps(&history.redo), steps(&snapshot.history[existing].redo));
        // QUIRK: the journal revision is not stored; it reads back as zero.
        assert_eq!(history.revision, 0);
        assert_eq!(loaded.folders, snapshot.folders);
        // The fingerprint covers the revision too, so it is the one difference.
        assert_eq!(loaded.fingerprint(), without_revisions(&snapshot).fingerprint());
        assert_ne!(loaded.fingerprint(), snapshot.fingerprint());

        // A mod's sidecar project carries no history.
        let sidecar = kit.root.join("sidecar.baboon");
        save_campaign_project(&sidecar, &snapshot, None, ProjectScope::ModSidecar).unwrap();
        let loaded = load_campaign_project(&sidecar).unwrap();
        assert!(loaded.history.is_empty());
        assert_eq!(loaded.overlays.len(), 2);
    }

    /// What the app captures: an overlay per dirty document, the open tabs, the
    /// selection, each document's undo trail and the folders made this session;
    /// a checkpoint writes exactly that to the recovery file.
    #[test]
    fn a_capture_holds_the_workspace_and_a_checkpoint_writes_it() {
        let _session = session_file_lock();
        let kit = CeKit::new("project-capture");
        let mut app = kit.app(&["objects/rock", "objects/stone"]);
        edit_field(&mut app, &key("objects/rock"), FRICTION, "0.25");
        edit_field(&mut app, &key("objects/rock"), FRICTION, "0.5");
        app.model.kits[0]
            .pending_container_folders
            .insert("objects/mine".to_owned());

        let snapshot = app.capture_campaign_project(0, 1.0).unwrap().expect("a snapshot");

        assert_eq!(snapshot.source_path, kit.root);
        assert_eq!(
            snapshot.overlays.keys().cloned().collect::<Vec<_>>(),
            vec![kit.identity("objects/rock")],
            "only the dirty document"
        );
        let rock = &snapshot.overlays[&kit.identity("objects/rock")];
        assert_eq!(rock.kind, CampaignProjectTagKind::Existing);
        assert_eq!(rock.package, None);
        assert_eq!(friction_in(&rock.bytes), Some(0.5));
        assert_eq!(
            *rock.bytes,
            app.model.kits[0].parsed_tags[&key("objects/rock")].tag.write_to_bytes().unwrap()
        );
        let open: Vec<String> = app.model.kits[0]
            .open_tabs
            .iter()
            .map(|key| kit.identity(&key["ublock:pakchunk0:".len()..]))
            .collect();
        assert_eq!(
            snapshot.tabs.iter().map(|tab| tab.identity.clone()).collect::<Vec<_>>(),
            open,
            "in open-tab order"
        );
        assert_eq!(
            snapshot.selected_identity.as_deref(),
            Some(kit.identity("objects/rock").as_str())
        );
        assert_eq!(snapshot.history.len(), 1, "only the edited document has history");
        let history = &snapshot.history[&kit.identity("objects/rock")];
        assert_eq!(history.undo.len(), 2);
        assert!(history.redo.is_empty());
        assert_eq!(friction_in(&history.undo[1].bytes), Some(0.25), "the state before the step");
        assert_eq!(snapshot.folders, BTreeSet::from(["objects/mine".to_owned()]));
        assert!(app.model.tag_has_stashed_overlay(0, &key("objects/rock")));
        assert!(!app.model.tag_has_stashed_overlay(0, &key("objects/stone")));
        assert_eq!(app.model.stashed_campaign_tags(0), vec!["objects/rock".to_owned()]);

        assert_eq!(app.checkpoint_campaign_project(0, 1.0), Ok(true));
        let written = load_campaign_project(&kit.recovery()).unwrap();
        assert_eq!(written.fingerprint(), without_revisions(&snapshot).fingerprint());
        assert_eq!(
            app.checkpoint_campaign_project(0, 2.0),
            Ok(false),
            "nothing changed, nothing written"
        );
        assert_eq!(project(&app).next_autosave_at, 2.0 + CAMPAIGN_PROJECT_AUTOSAVE_SECS);

        // A loose kit has no project to capture.
        let loose = LooseKit::new("project-capture-loose", "halo3_mcc");
        let mut app = Baboon::for_test();
        loose.install(&mut app);
        assert!(matches!(app.capture_campaign_project(0, 1.0), Ok(None)));
        assert!(app.model.kits[0].project.active.is_none());
    }

    /// When the autosave writes: not before it is due, once something changed,
    /// never twice for the same state, and not while a write is in flight.
    #[test]
    fn autosave_writes_only_when_due_and_changed() {
        let _session = session_file_lock();
        let kit = CeKit::new("project-autosave");
        let mut app = kit.app(&["objects/rock"]);
        let ctx = egui::Context::default();

        autosave_at(&mut app, &ctx, 10.0);
        assert_eq!(project(&app).next_autosave_at, 10.0 + CAMPAIGN_PROJECT_AUTOSAVE_SECS);
        assert_eq!(project(&app).revision, 0, "not due yet");
        assert!(project(&app).project_path.is_none(), "autosave never names a project");

        // Due, and never written: even an empty workspace is written once.
        autosave_at(&mut app, &ctx, 11.0);
        assert_eq!(project(&app).revision, 1);
        assert_eq!(project(&app).save_in_flight, Some(1));
        pump_until(&mut app, "the autosave", |app| {
            project(app).save_in_flight.is_none()
        });
        assert!(project(&app).saved_digests.is_some());
        assert!(load_campaign_project(&kit.recovery()).unwrap().overlays.is_empty());

        // Due again, nothing changed: no write.
        autosave_at(&mut app, &ctx, 12.0);
        assert_eq!(project(&app).revision, 1);
        assert_eq!(project(&app).save_in_flight, None);
        assert_eq!(project(&app).next_autosave_at, 12.0 + CAMPAIGN_PROJECT_AUTOSAVE_SECS);

        // An edit is written at the next due tick, and not before.
        edit_field(&mut app, &key("objects/rock"), FRICTION, "0.75");
        autosave_at(&mut app, &ctx, 12.5);
        assert_eq!(project(&app).revision, 1, "not due");
        autosave_at(&mut app, &ctx, 13.0);
        assert_eq!(project(&app).revision, 2);
        pump_until(&mut app, "the second autosave", |app| {
            project(app).save_in_flight.is_none()
        });
        let written = load_campaign_project(&kit.recovery()).unwrap();
        assert_eq!(
            friction_in(&written.overlays[&kit.identity("objects/rock")].bytes),
            Some(0.75)
        );

        // While a write is in flight a due tick only pushes the next one back.
        edit_field(&mut app, &key("objects/rock"), FRICTION, "1");
        app.model.kits[0].project.active.as_mut().unwrap().save_in_flight = Some(99);
        autosave_at(&mut app, &ctx, 14.0);
        assert_eq!(project(&app).revision, 2);
        assert_eq!(project(&app).next_autosave_at, 14.0 + CAMPAIGN_PROJECT_AUTOSAVE_SECS);
        assert_eq!(
            friction_in(&project(&app).overlays[&kit.identity("objects/rock")].bytes),
            Some(0.75),
            "not even captured"
        );
        assert!(app.rx.recv_timeout(Duration::from_millis(100)).is_err());

        // A loose kit never gets a project.
        let loose = LooseKit::new("project-autosave-loose", "halo3_mcc");
        let mut app = Baboon::for_test();
        loose.install(&mut app);
        autosave_at(&mut app, &ctx, 20.0);
        assert!(app.model.kits[0].project.active.is_none());
    }

    /// A recovery file left by an earlier session is adopted, not overwritten:
    /// the next session starts with its stash.
    #[test]
    fn a_new_session_adopts_the_recovery_file() {
        let _session = session_file_lock();
        let kit = CeKit::new("project-adopt");
        let mut app = kit.app(&["objects/rock"]);
        edit_field(&mut app, &key("objects/rock"), FRICTION, "0.5");
        app.checkpoint_campaign_project(0, 1.0).unwrap();

        let mut next = kit.app(&["objects/rock"]);
        next.model.kits[0].parsed_tags.clear();
        autosave_at(&mut next, &egui::Context::default(), 1.0);

        assert!(next.model.tag_has_stashed_overlay(0, &key("objects/rock")));
        assert_eq!(
            next.model.status,
            "Restored 1 stashed modification(s) from this workspace's last session"
        );
        assert!(project(&next).saved_digests.is_some(), "believed, so not rewritten");
        // Opening the tag serves it from the stash.
        assert!(next.load_campaign_overlay_for_key(0, &key("objects/rock")));
        assert_eq!(
            real_of(&next.model.kits[0].parsed_tags[&key("objects/rock")].tag, FRICTION),
            Some(0.5)
        );
    }

    #[test]
    fn clearing_the_stash_forgets_every_overlay_and_document() {
        let _session = session_file_lock();
        let kit = CeKit::new("project-clear-stash");
        let mut app = kit.app(&["objects/rock", "objects/stone"]);
        edit_field(&mut app, &key("objects/rock"), FRICTION, "0.5");
        app.checkpoint_campaign_project(0, 1.0).unwrap();
        assert_eq!(load_campaign_project(&kit.recovery()).unwrap().overlays.len(), 1);

        app.clear_campaign_stash(0, &ctx());

        assert_eq!(app.model.status, "Cleared 1 stashed modification");
        assert!(project(&app).overlays.is_empty());
        assert!(app.model.kits[0].parsed_tags.is_empty(), "documents dropped, dirty or not");
        assert!(load_campaign_project(&kit.recovery()).unwrap().overlays.is_empty());
        // The open tabs are asked for again, from the (absent) containers.
        for path in ["objects/rock", "objects/stone"] {
            assert!(app.model.kits[0].loading_tags.contains(&key(path)), "{path}");
        }
        drain_messages(&mut app, Duration::from_millis(200));

        app.clear_campaign_stash(0, &ctx());
        assert_eq!(app.model.status, "Cleared this workspace's unsaved modifications");
    }

    /// Write the user's own project, named, through the app's Save Project.
    fn save_user_project(app: &mut Baboon, path: &Path) {
        // The project exists from the first autosave on.
        app.capture_campaign_project(0, 1.0).unwrap();
        app.model.kits[0].project.active.as_mut().unwrap().project_path = Some(path.to_path_buf());
        app.save_campaign_project_file(0, 2.0);
        assert_eq!(
            app.model.status,
            format!("Saved 1 modified tag(s) to {}", path.display())
        );
    }

    /// "Don't Save" on a stashing workspace deletes the stashed copy from the
    /// recovery file, and never touches the `.baboon` the user saved.
    #[test]
    fn discarding_never_writes_the_user_s_project() {
        let _session = session_file_lock();
        let kit = CeKit::new("project-discard");
        let mut app = kit.app(&["objects/rock"]);
        let rock = key("objects/rock");
        edit_field(&mut app, &rock, FRICTION, "0.5");
        app.checkpoint_campaign_project(0, 1.0).unwrap();
        let user = kit.root.join("mine.baboon");
        save_user_project(&mut app, &user);
        let user_bytes = fs::read(&user).unwrap();

        app.request_close_action(PendingCloseAction::CloseTab(rock.clone()), &ctx());
        let prompt = app
            .dialogs
            .get::<SaveChangesPrompt>()
            .expect("the prompt is up");
        assert!(prompt.can_stash);
        assert_eq!(prompt.stashed, 1);
        assert_eq!(prompt.stash_file.as_deref(), Some(kit.recovery().as_path()));

        let mut driver = PromptDriver::new();
        driver.click(&mut app, "Discard...");
        assert!(
            app.dialogs
                .get::<SaveChangesPrompt>()
                .unwrap()
                .confirm_discard,
            "the first click arms"
        );
        assert!(app.dialogs.get::<SaveChangesPrompt>().is_some());
        assert!(app.model.kits[0].open_tabs.contains(&rock));
        driver.click(&mut app, "Delete Stashed Edits");

        assert!(app.dialogs.get::<SaveChangesPrompt>().is_none());
        assert!(!app.model.kits[0].open_tabs.contains(&rock));
        assert!(!app.model.tag_has_stashed_overlay(0, &rock));
        assert!(load_campaign_project(&kit.recovery()).unwrap().overlays.is_empty());
        assert_eq!(fs::read(&user).unwrap(), user_bytes, "the user's project is untouched");
        assert_eq!(load_campaign_project(&user).unwrap().overlays.len(), 1);
    }

    /// "Stash for Mod" keeps the edit in the recovery file and closes; the
    /// user's project keeps what was last saved into it.
    #[test]
    fn stashing_for_mod_keeps_the_edit_out_of_the_user_s_project() {
        let _session = session_file_lock();
        let kit = CeKit::new("project-stash");
        let mut app = kit.app(&["objects/rock"]);
        let rock = key("objects/rock");
        edit_field(&mut app, &rock, FRICTION, "0.5");
        let user = kit.root.join("mine.baboon");
        save_user_project(&mut app, &user);
        let user_bytes = fs::read(&user).unwrap();
        edit_field(&mut app, &rock, FRICTION, "0.75");

        app.request_close_action(PendingCloseAction::CloseTab(rock.clone()), &ctx());
        PromptDriver::new().click(&mut app, "Stash for Mod");

        assert_eq!(
            app.model.status,
            format!(
                "Stashed for Export Mod. {} is unchanged until you save it",
                user.display()
            )
        );
        assert!(app.dialogs.get::<SaveChangesPrompt>().is_none());
        assert!(!app.model.kits[0].open_tabs.contains(&rock), "the close went ahead");
        let stashed = load_campaign_project(&kit.recovery()).unwrap();
        assert_eq!(
            friction_in(&stashed.overlays[&kit.identity("objects/rock")].bytes),
            Some(0.75)
        );
        assert_eq!(fs::read(&user).unwrap(), user_bytes);
        assert_eq!(
            friction_in(
                &load_campaign_project(&user).unwrap().overlays[&kit.identity("objects/rock")].bytes
            ),
            Some(0.5)
        );
    }

    /// The prompt's Save routes a container tag to the in-place overwrite rather
    /// than the loose-file writer. With its container not mounted the overwrite
    /// refuses before taking a lease, and the prompt stays up saying why.
    #[test]
    fn the_prompt_s_save_routes_a_container_tag_into_its_pak() {
        let _session = session_file_lock();
        let kit = CeKit::new("project-prompt-save");
        let mut app = kit.app(&["objects/rock"]);
        let rock = key("objects/rock");
        edit_field(&mut app, &rock, FRICTION, "0.5");
        app.request_close_action(PendingCloseAction::CloseTab(rock.clone()), &ctx());

        PromptDriver::new().click(&mut app, "Save");

        let prompt = app
            .dialogs
            .get::<SaveChangesPrompt>()
            .expect("the prompt is up");

        assert_eq!(
            prompt.error.as_deref(),
            Some("Save failed: objects/rock.point_physics: Container provenance is stale")
        );
        assert!(app.mods.container_write_leases.is_empty(), "no lease was taken");
        assert!(app.model.kits[0].open_tabs.contains(&rock));
        assert!(app.model.kits[0].parsed_tags[&rock].dirty.is_set());
        assert!(fs::read_dir(&kit.root).unwrap().next().is_none(), "nothing written");
    }

    /// The export review lists every stashed tag, and what the writer does with
    /// a selection it cannot build.
    #[test]
    fn the_export_review_lists_the_stash_and_refuses_what_it_cannot_write() {
        let _session = session_file_lock();
        let kit = CeKit::new("project-export");
        let mut app = kit.app(&["objects/rock"]);
        edit_field(&mut app, &key("objects/rock"), FRICTION, "0.5");
        app.capture_campaign_project(0, 1.0).unwrap();
        let orphan = "70706879:objects/gone";
        app.model.kits[0]
            .project.active
            .as_mut()
            .unwrap()
            .overlays
            .insert(
                orphan.to_owned(),
                kind_overlay(orphan, CampaignProjectTagKind::Existing, b"orphaned"),
            );

        app.review_changes();
        let review = app.dialogs.get::<ModExportDialog>().expect("the review opened");
        assert!(review.review_only);
        assert!(!kit.root.join("~mods").exists(), "a review creates nothing");

        app.export_mod();
        let dialog = app.dialogs.get::<ModExportDialog>().expect("the export opened");
        assert!(!dialog.review_only);
        assert_eq!(dialog.name, "mymod");
        assert_eq!(dialog.folder, kit.root.join("~mods"));
        assert!(dialog.folder.is_dir(), "the default destination is made");
        let mut rows: Vec<_> = dialog.rows.iter().collect();
        rows.sort_by(|a, b| a.identity.cmp(&b.identity));
        let rock = rows
            .iter()
            .find(|row| row.identity == kit.identity("objects/rock"))
            .expect("the edited tag");
        assert!(matches!(rock.kind, ModExportChange::Modified));
        assert!(rock.include);
        assert_eq!(rock.reason, None);
        assert_eq!(rock.display_path, "objects/rock");
        let gone = rows.iter().find(|row| row.identity == orphan).expect("the orphan");
        assert!(matches!(gone.kind, ModExportChange::Unresolved));
        assert!(!gone.include, "an unresolved tag is not offered");
        assert_eq!(gone.reason.as_deref(), Some("not in this source"));
        assert_eq!(dialog.rows.len(), 2);

        let snapshot = app.dialogs.get::<ModExportDialog>().unwrap().snapshot.clone();
        let output = kit.root.join("~mods/mymod_P.utoc");
        let write = |app: &mut Baboon, included: &[&str], output: &Path| {
            let included = included.iter().map(|id| (*id).to_owned()).collect();
            app.write_reviewed_mod(&snapshot, &included, output.to_path_buf(), &ctx());
            app.model.status.clone()
        };
        assert_eq!(write(&mut app, &[], &output), "Nothing selected to export");
        assert_eq!(write(&mut app, &[orphan], &output), "Nothing selected to export");
        // An existing tag whose container is not mounted is passed over the same
        // way (QUIRK: reported as nothing selected rather than as unwritable).
        let rock_identity = kit.identity("objects/rock");
        assert_eq!(
            write(&mut app, &[rock_identity.as_str()], &output),
            "Nothing selected to export"
        );
        assert!(!output.exists());
        // A destination whose folder cannot be made is refused first.
        fs::write(kit.root.join("blocker"), b"").unwrap();
        let status = write(&mut app, &[rock_identity.as_str()], &kit.root.join("blocker/x.utoc"));
        assert!(status.starts_with("Could not create "), "{status}");

        // And a loose kit is not a Campaign Evolved source at all.
        let loose = LooseKit::new("project-export-loose", "halo3_mcc");
        let mut app = Baboon::for_test();
        loose.install(&mut app);
        assert_eq!(
            write(&mut app, &[rock_identity.as_str()], &loose.base.join("out/x.utoc")),
            "Export Mod is only for Campaign Evolved containers"
        );
        app.export_mod();
        assert!(app.dialogs.get::<ModExportDialog>().is_none());
        assert_eq!(app.model.status, "Export Mod is only for Campaign Evolved containers");
    }

    /// A Save As copy remembers which shipped tag it was copied from across
    /// the project file. A project an older build wrote has no table for it,
    /// and loads with no origins rather than failing.
    #[test]
    fn a_copy_s_origin_round_trips_through_the_project() {
        let path = unique_temp_dir("copy-origin").join("project.baboon");
        let copy = CampaignProjectOverlay {
            kind: CampaignProjectTagKind::New,
            copied_from: Some("6374726b:objects/props/crate".to_owned()),
            ..overlay("6374726b:objects/props/crate_copy", &[1, 2, 3])
        };
        let plain = overlay("6374726b:objects/props/barrel", &[4]);
        let snapshot = CampaignProjectSnapshot {
            game: "haloce_evolved".to_owned(),
            source_path: PathBuf::from("Paks"),
            selected_identity: None,
            tabs: Vec::new(),
            overlays: HashMap::from([
                (copy.identity.clone(), copy.clone()),
                (plain.identity.clone(), plain.clone()),
            ]),
            history: BTreeMap::new(),
            folders: BTreeSet::new(),
        };
        save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();

        let loaded = load_campaign_project(&path).unwrap();
        assert_eq!(loaded.overlays[&copy.identity].copied_from, copy.copied_from);
        assert_eq!(loaded.overlays[&plain.identity].copied_from, None);

        Connection::open(&path)
            .unwrap()
            .execute_batch("DROP TABLE overlay_origins")
            .unwrap();
        let older = load_campaign_project(&path).unwrap();
        assert_eq!(older.overlays[&copy.identity].copied_from, None);
        assert_eq!(older.overlays.len(), 2);
    }

    /// A stashed copy comes back wrapped in its source's `.uasset`, wherever
    /// this mount put the source. With the source gone it is not restored with
    /// some other tag's wrapper, which would bind it to that tag's assets.
    #[test]
    fn a_restored_copy_is_wrapped_in_its_source_or_not_at_all() {
        let definitions = crate::core::bundled::locate_definitions_root();
        let tag = TagFile::new(definitions.join("haloce_evolved/camera_track.json")).unwrap();
        let group_tag = tag.header.group_tag;
        let source = TagEntry {
            key: "ublock:pakchunk3:objects/props/crate".to_owned(),
            display_path: "objects/props/crate.camera_track".to_owned(),
            group_tag,
            group_name: Some("camera_track".to_owned()),
            location: TagEntryLocation::Container {
                container: 3,
                rel_path: "Tags/objects/props/crate-camera_track.ubulk".to_owned(),
            },
        };
        let (source_identity, ..) = campaign_entry_project_parts(&source).unwrap();
        let bytes = tag.write_to_bytes().unwrap();
        let copy = CampaignProjectOverlay {
            identity: format!("{group_tag:08x}:objects/props/crate_copy"),
            group_tag,
            logical_path: "objects/props/crate_copy".to_owned(),
            kind: CampaignProjectTagKind::New,
            package: Some("/Game/Tags/objects/props/crate_copy-camera_track".to_owned()),
            digest: overlay_digest(&bytes),
            bytes: Arc::new(bytes),
            copied_from: Some(source_identity.clone()),
        };
        let mounted = |entries: Vec<TagEntry>| {
            let mut app = Baboon::for_test();
            app.install_loaded_source(LoadedSourceData {
                label: "copy origin".to_owned(),
                source: TagSource::IoStoreContainerSet {
                    root: PathBuf::from("C:/copy-origin/Paks"),
                    containers: Vec::new(),
                    index: Arc::new(crate::core::source::ContainerTagIndex::default()),
                    packages: Arc::new(crate::core::source::ContainerPackageIndex::default()),
                    shipped: Arc::new(crate::core::source::ShippedTagIndex::default()),
                },
                names: TagNameIndex::load_game(&definitions, GameId::CampaignEvolved).unwrap(),
                game: Some(GameId::CampaignEvolved),
                tree: crate::core::source::build_tree(&entries),
                group_tree: crate::core::source::build_group_tree(&entries),
                entries,
                all_entries: Vec::new(),
                reverse_dependencies: None,
                initial_tag: None,
                key_hints: Default::default(),
                complete_scan: false,
                chosen_kit_layout: None,
            });
            app
        };

        let app = mounted(vec![source]);
        match app.model.new_overlay_entry(0, &copy) {
            OverlayAdoption::Ready(entry, _) => match entry.location {
                TagEntryLocation::NewContainer {
                    template:
                        NewContainerTemplate::Copy {
                            container,
                            rel_path,
                            source,
                        },
                    ..
                } => {
                    assert_eq!(container, 3, "found where this mount put it");
                    assert_eq!(rel_path, "Tags/objects/props/crate-camera_track.uasset");
                    assert_eq!(source, source_identity);
                }
                _ => panic!("restored without its source's wrapper"),
            },
            OverlayAdoption::Failed(reason) => panic!("not restored: {reason}"),
            _ => panic!("not restored"),
        }

        let app = mounted(Vec::new());
        match app.model.new_overlay_entry(0, &copy) {
            OverlayAdoption::Failed(reason) => {
                assert!(reason.contains("not in the mounted paks"), "{reason}")
            }
            _ => panic!("a copy without its source must not be restored"),
        }
    }
}
