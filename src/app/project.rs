//! Campaign Evolved project/recovery persistence.
//!
//! A `.baboon` file is a small SQLite database. Clean tabs are represented by
//! canonical tag identities while modified/new tags also carry their serialized
//! tag bytes, allowing a project to be reopened without modifying the base paks.

use super::*;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) const CAMPAIGN_PROJECT_VERSION: i64 = 1;
pub(super) const CAMPAIGN_PROJECT_AUTOSAVE_SECS: f64 = 0.75;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CampaignProjectTagKind {
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
pub(super) struct CampaignProjectTab {
    pub(super) identity: String,
    pub(super) label: String,
    pub(super) group_tag: u32,
    pub(super) logical_path: String,
    pub(super) kind: CampaignProjectTagKind,
    pub(super) package: Option<String>,
    pub(super) floating: bool,
}

#[derive(Clone, Debug)]
pub(super) struct CampaignProjectOverlay {
    pub(super) identity: String,
    pub(super) group_tag: u32,
    pub(super) logical_path: String,
    pub(super) kind: CampaignProjectTagKind,
    pub(super) package: Option<String>,
    /// Shared, not owned: the overlay map is cloned two or three times per
    /// autosave tick, and a workspace stashing a 105 MiB tag paid a full copy
    /// each time.
    pub(super) bytes: Arc<Vec<u8>>,
    /// Hashed once, where the bytes are produced. The project fingerprint is
    /// taken over these rather than over the bytes themselves: a workspace
    /// holding a 105 MiB animation graph cost 230 ms a tick to re-hash, twice a
    /// second, purely to learn nothing had changed.
    pub(super) digest: [u8; 32],
}

/// One tag's undo and redo stacks as the session holds them, oldest first.
#[derive(Clone, Debug, Default)]
pub(super) struct TagHistory {
    pub(super) undo: Vec<HistoryStep>,
    pub(super) redo: Vec<HistoryStep>,
    /// The owning journal's change counter when this was captured, so a save
    /// can tell "nothing has happened since" without looking at the snapshots.
    pub(super) revision: u64,
}

/// One undoable step: what it was called, and the tag bytes it restores.
#[derive(Clone, Debug)]
pub(super) struct HistoryStep {
    pub(super) label: String,
    pub(super) bytes: Arc<Vec<u8>>,
}

/// How many steps of one stack a restored session gets back.
///
/// Far below the in-memory limit of 64 on purpose. A step is a whole serialized
/// tag, the recovery file is rewritten as you edit, and the value of persisted
/// history falls off a cliff after the last handful of actions — nobody
/// reopens a workspace to undo their sixtieth-from-last change.
pub(super) const HISTORY_STEP_LIMIT: usize = 16;

/// The total bytes of history one workspace may write to its recovery file.
///
/// Campaign Evolved ships a 105 MiB animation graph; two of those in a stack
/// would be a quarter-gigabyte written repeatedly while the user edits. The
/// newest steps are kept and the oldest dropped, so what survives is the part
/// anyone would actually reach for.
pub(super) const HISTORY_BYTE_BUDGET: usize = 64 * 1024 * 1024;

/// Trim captured history to what may be written, newest first.
///
/// Applied across the whole workspace rather than per tag: the budget exists to
/// bound the recovery file, and a per-tag budget multiplies by however many tags
/// happen to be open. Steps are dropped oldest-first, and a stack keeps its
/// order.
pub(super) fn trim_history_for_disk(
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

pub(super) fn overlay_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

#[derive(Clone, Debug)]
pub(super) struct CampaignProjectSnapshot {
    pub(super) game: String,
    pub(super) source_path: PathBuf,
    pub(super) selected_identity: Option<String>,
    pub(super) tabs: Vec<CampaignProjectTab>,
    pub(super) overlays: HashMap<String, CampaignProjectOverlay>,
    /// Each open tag's undo/redo stacks, so reopening the workspace reopens the
    /// session rather than just the files. Written to this workspace's own
    /// recovery project and to a project the user names, never to the sidecar
    /// beside an exported mod — that one travels to whoever installs the mod,
    /// and an author's step-by-step editing trail is neither their business nor
    /// something they should have to download.
    pub(super) history: BTreeMap<String, TagHistory>,
    /// Folders the user made in the container that no tag has landed in yet.
    ///
    /// A pak's directory index cannot encode a directory with no file beneath
    /// it, so these exist only in the workspace and would otherwise be gone on
    /// the next launch. Like history, they are the author's own organisation
    /// rather than mod content, so they are not written to the sidecar beside an
    /// exported mod.
    pub(super) folders: std::collections::BTreeSet<String>,
}

impl CampaignProjectSnapshot {
    /// Each overlay's digest, by identity — what the overlays table holds once
    /// this snapshot has been written.
    pub(super) fn digests(&self) -> SavedProjectState {
        SavedProjectState {
            overlays: self
                .overlays
                .iter()
                .map(|(identity, overlay)| (identity.clone(), overlay.digest))
                .collect(),
            history: self.history_digest(),
        }
    }

    /// One digest over the whole session history.
    ///
    /// The history table is replaced wholesale on every write — an undo stack
    /// shifts by one on every edit, so nearly every row's position changes and
    /// there is nothing to reconcile row by row. That makes *skipping* the
    /// write when nothing changed the only thing standing between this feature
    /// and rewriting tens of megabytes twice a second while someone drags a
    /// slider.
    pub(super) fn history_digest(&self) -> [u8; 32] {
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

    pub(super) fn fingerprint(&self) -> Vec<u8> {
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
pub(super) struct SavedProjectState {
    /// Each overlay's digest, by identity — rows whose bytes match are left
    /// alone rather than rewritten.
    pub(super) overlays: HashMap<String, [u8; 32]>,
    /// One digest over the whole history table, which is replaced wholesale or
    /// not at all.
    pub(super) history: [u8; 32],
}

/// Which kind of `.baboon` is being written.
///
/// The two are the same format and deliberately not the same content: a session
/// project is this workspace's own state, while a sidecar is published beside a
/// mod for other people.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectScope {
    /// This workspace's recovery file, or a project the user saved.
    Session,
    /// The `.baboon` written next to an exported mod's containers.
    ModSidecar,
}

pub(super) struct ActiveCampaignProject {
    /// Where this workspace autosaves. Always derived from the mounted source
    /// — never a file the user picked.
    ///
    /// The two paths are kept apart because they were once one field, and
    /// opening a `.baboon` therefore made *that* file the autosave target: an
    /// exported mod's sidecar became a live, self-overwriting file the moment it
    /// was opened, and declining to save at exit deleted the stashed rows out of
    /// it. Autosave now only ever writes the recovery file.
    pub(super) recovery_path: PathBuf,
    /// The `.baboon` this workspace is associated with: what `File > Open
    /// Baboon Project` opened, or where `Save Baboon Project As...` last wrote.
    /// It is the target of `File > Save Baboon Project`, and nothing else
    /// writes to it.
    pub(super) project_path: Option<PathBuf>,
    pub(super) overlays: HashMap<String, CampaignProjectOverlay>,
    /// Document key -> the `Dirty` revision its overlay bytes were written
    /// from, so an untouched document is never serialized twice.
    pub(super) captured_revisions: HashMap<String, u64>,
    /// What the recovery file holds, by identity, so a save writes only the rows
    /// whose bytes changed instead of replacing every overlay.
    ///
    /// `None` when that is unknown — a fresh workspace, or a project imported
    /// from a file the recovery has never seen — and the next write then
    /// replaces every row rather than merging into whatever was there.
    pub(super) saved_digests: Option<SavedProjectState>,
    /// What an in-flight write will leave on disk, promoted to `saved_digests`
    /// once it reports success.
    pub(super) pending_digests: Option<SavedProjectState>,
    pub(super) last_saved_fingerprint: Vec<u8>,
    pub(super) next_autosave_at: f64,
    pub(super) revision: u64,
    pub(super) save_in_flight: Option<u64>,
    pub(super) write_lock: Arc<Mutex<()>>,
    pub(super) latest_write_revision: Arc<AtomicU64>,
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
    pub(super) pending_new_overlays: Vec<CampaignProjectOverlay>,
}

impl ActiveCampaignProject {
    pub(super) fn fresh(recovery_path: PathBuf, now: f64) -> Self {
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
    pub(super) fn adopted(
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
    pub(super) fn imported(
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
    pub(super) fn label(&self) -> String {
        match self.project_path.as_deref() {
            Some(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            None => "unsaved".to_owned(),
        }
    }
}

pub(super) struct PendingCampaignProject {
    pub(super) path: PathBuf,
    /// The project to open once the source mounts. `None` stages the path as
    /// this workspace's save target *without* reading it back in, which is what
    /// a session restore wants: the workspace's recovery file is always the
    /// fresher copy of the same edits, so re-importing the `.baboon` the user
    /// happened to have open would overwrite newer work with older.
    pub(super) snapshot: Option<CampaignProjectSnapshot>,
}

/// Where a Campaign Evolved kit autosaves its recovery project.
///
/// Derived from the mounted source so two Campaign Evolved kits recover to
/// two files rather than overwriting each other, and so a kit finds its own
/// recovery again on the next launch. `None` keeps the original unqualified
/// name for a kit with no source path to key on.
pub(super) fn campaign_recovery_path(source_root: Option<&Path>) -> PathBuf {
    let Some(root) = source_root else {
        return crate::storage::data_path("campaign_evolved_recovery.baboon");
    };
    let mut hasher = Sha256::new();
    hasher.update(root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let tag = digest[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    crate::storage::data_path(&format!("{CAMPAIGN_RECOVERY_STEM}-{tag}.baboon"))
}

pub(super) const CAMPAIGN_RECOVERY_STEM: &str = "campaign_evolved_recovery";

/// Whether `path` is one of Baboon's own recovery files rather than a `.baboon`
/// the user named. Recovery files are an implementation detail of a workspace —
/// they are not offered as a save target, and a session that recorded one back
/// when the two were the same file must not be read as having a project open.
pub(super) fn is_campaign_recovery_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(CAMPAIGN_RECOVERY_STEM))
}

/// Write the project to `path`.
///
/// `on_disk` is what the last successful save left in the overlays table, by
/// identity and digest; overlay rows whose bytes are unchanged are then left
/// alone. Rewriting every overlay meant editing a 4 MiB scenario also rewrote
/// the 105 MiB animation graph stashed beside it. Pass `None` when the file's
/// contents are unknown — every overlay is replaced, as before.
pub(super) fn save_campaign_project(
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
             CREATE TABLE IF NOT EXISTS folders (
                 path TEXT PRIMARY KEY
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
    // History is replaced wholesale rather than reconciled. Undo stacks shift
    // by one on every edit — a step pushed at the top, the oldest dropped — so
    // almost every row's position changes and there is nothing to spare.
    // Keeping it small is what makes that affordable, which is what the budget
    // above is for.
    // Skipped entirely when nothing has changed, which is what keeps an active
    // edit session from rewriting the whole table twice a second.
    let history_unchanged =
        on_disk.is_some_and(|on_disk| on_disk.history == snapshot.history_digest());
    if !history_unchanged {
        transaction
            .execute("DELETE FROM history", [])
            .map_err(|error| format!("Could not reset project history: {error}"))?;
    }
    if scope == ProjectScope::Session && !history_unchanged {
        for (identity, entry) in &snapshot.history {
            for (stack, steps) in [("undo", &entry.undo), ("redo", &entry.redo)] {
                for (position, step) in steps.iter().enumerate() {
                    transaction
                        .execute(
                            "INSERT INTO history (identity, stack, position, label, bytes)
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            params![
                                identity,
                                stack,
                                position as i64,
                                step.label,
                                step.bytes.as_slice(),
                            ],
                        )
                        .map_err(|error| {
                            format!("Could not write project history for {identity}: {error}")
                        })?;
                }
            }
        }
    }
    // Folders are replaced wholesale: the set is a handful of short strings, so
    // reconciling it would cost more than rewriting it. Session scope only —
    // like history, a folder is the author's own organisation, and the sidecar
    // travels to whoever installs the mod.
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

pub(super) fn load_campaign_project(path: &Path) -> Result<CampaignProjectSnapshot, String> {
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
    if game != "haloce_evolved" {
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
                },
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;

    // A project written before history existed simply has no table. That is a
    // session with nothing to undo, not a project that fails to open.
    let mut history: BTreeMap<String, TagHistory> = BTreeMap::new();
    if let Ok(mut statement) = connection
        .prepare("SELECT identity, stack, label, bytes FROM history ORDER BY identity, stack, position")
    {
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })
            .map_err(|error| format!("Could not query project history: {error}"))?;
        for row in rows {
            let (identity, stack, label, bytes) =
                row.map_err(|error| format!("Could not decode project history: {error}"))?;
            let step = HistoryStep {
                label,
                bytes: Arc::new(bytes),
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

fn logical_path_from_display(display_path: &str) -> String {
    let normalized = display_path.replace('\\', "/");
    normalized
        .rsplit_once('.')
        .map(|(path, _)| path)
        .unwrap_or(&normalized)
        .trim_matches('/')
        .to_ascii_lowercase()
}

pub(super) fn campaign_entry_project_parts(
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
pub(super) fn campaign_entry_project_parts_with(
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
    pub(super) fn authored_package_for_entry(
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
        let source = self.kits.get(kit)?.source.as_ref()?;
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return None;
        };
        let utoc = &containers.get(*container)?.utoc_path;
        self.created_tags
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
    pub(super) fn apply_pending_history(&mut self, kit: usize, key: &str) {
        let Some(history) = self.kits[kit].pending_history.remove(key) else {
            return;
        };
        let Some(document) = self.kits[kit].parsed_tags.get_mut(key) else {
            return;
        };
        let steps = |steps: Vec<HistoryStep>| {
            steps
                .into_iter()
                .map(|step| crate::app::Snapshot {
                    bytes: step.bytes,
                    label: step.label,
                })
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
    pub(super) fn stash_authored_tag(
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
        let Some(project) = self.kits[kit].campaign_project.as_mut() else {
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
            },
        );
    }

    pub(super) fn current_source_is_campaign_project_capable(&self, kit: usize) -> bool {
        self.kits[kit]
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.source, TagSource::IoStoreContainerSet { .. }))
    }

    pub(super) fn campaign_entry_for_identity(
        &self,
        kit: usize,
        identity: &str,
    ) -> Option<TagEntry> {
        let source = self.kits[kit].source.as_ref()?;
        source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .find(|entry| {
                campaign_entry_project_parts(entry)
                    .is_some_and(|(candidate, _, _, _)| candidate == identity)
            })
            .cloned()
    }

    fn ensure_campaign_project(&mut self, kit: usize, now: f64) {
        if !self.current_source_is_campaign_project_capable(kit)
            || self.kits[kit].campaign_project.is_some()
        {
            return;
        }
        let root = self.kits[kit]
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
        self.kits[kit].campaign_project = Some(match &restored {
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
            self.status = format!(
                "Restored {count} stashed modification(s) from this workspace's last session"
            );
        }
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
    fn new_overlay_entry(
        &self,
        kit: usize,
        overlay: &CampaignProjectOverlay,
    ) -> Option<(TagEntry, TagFile)> {
        let group_name = self.kits[kit]
            .names
            .name_for(overlay.group_tag)
            .map(str::to_owned)?;
        // A stashed tag of a group the game ships none of has no donor to point
        // back at, and recovering it must not depend on finding one — otherwise
        // the tag survives the save and vanishes on reopen.
        let template = super::controller::new_container_template_for(
            self.find_container_template_in(kit, overlay.group_tag),
            &group_name,
        )
        .ok()?;
        let tag = TagFile::read_from_bytes(&overlay.bytes).ok()?;
        let extension = group_tag_to_extension(overlay.group_tag)
            .unwrap_or(group_name.as_str())
            .to_owned();
        let package = overlay
            .package
            .clone()
            .unwrap_or_else(|| format!("/Game/Tags/{}-{group_name}", overlay.logical_path));
        Some((
            TagEntry {
                key: format!("newtag:{package}"),
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
        ))
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
        if kit != self.active
            || self.kits[kit]
                .campaign_project
                .as_ref()
                .is_none_or(|project| project.pending_new_overlays.is_empty())
        {
            return;
        }
        let queued = self.kits[kit]
            .campaign_project
            .as_ref()
            .map(|project| project.pending_new_overlays.clone())
            .unwrap_or_default();
        let mut adopted = 0usize;
        let mut still_pending = Vec::new();
        for overlay in queued {
            if self
                .campaign_entry_for_identity(kit, &overlay.identity)
                .is_some()
            {
                continue;
            }
            let Some((entry, tag)) = self.new_overlay_entry(kit, &overlay) else {
                // The names and the template come off a source that may still be
                // loading, so this is a "not yet" rather than a "no".
                still_pending.push(overlay);
                continue;
            };
            let key = entry.key.clone();
            self.stash_in_memory_tag(entry, tag);
            // The document was parsed from the overlay's own bytes, so the
            // project already holds its serialization -- recording that spares
            // the next autosave from writing every adopted tag out again.
            if let Some(revision) = self.kits[kit]
                .parsed_tags
                .get(&key)
                .map(|document| document.dirty.revision())
            {
                if let Some(project) = self.kits[kit].campaign_project.as_mut() {
                    project.captured_revisions.insert(key, revision);
                }
            }
            adopted += 1;
        }
        if let Some(project) = self.kits[kit].campaign_project.as_mut() {
            project.pending_new_overlays = still_pending;
        }
        if adopted > 0 {
            self.status =
                format!("Restored {adopted} stashed new tag(s) from this workspace's last session");
        }
    }

    pub(super) fn capture_campaign_project(
        &mut self,
        kit: usize,
        now: f64,
    ) -> Result<Option<CampaignProjectSnapshot>, String> {
        // This kit's source, not the active one: autosave runs for every kit.
        let Some(source) = self.kits[kit].source.as_ref() else {
            return Ok(None);
        };
        let TagSource::IoStoreContainerSet { root, .. } = &source.source else {
            return Ok(None);
        };
        let source_path = root.clone();
        let game = source
            .game
            .clone()
            .unwrap_or_else(|| "haloce_evolved".to_owned());
        self.ensure_campaign_project(kit, now);
        let mut overlays = self.kits[kit]
            .campaign_project
            .as_ref()
            .map(|project| project.overlays.clone())
            .unwrap_or_default();

        // What each dirty document's bytes were captured from last time, so a
        // document nobody has touched since is carried over instead of being
        // serialized again. Autosave runs twice a second whether or not
        // anything was edited, and a stashed 105 MiB animation graph costs
        // ~100 ms to write out.
        let captured = self.kits[kit]
            .campaign_project
            .as_ref()
            .map(|project| project.captured_revisions.clone())
            .unwrap_or_default();
        let mut now_captured: HashMap<String, u64> = HashMap::new();
        for (key, document) in &self.kits[kit].parsed_tags {
            if !document.dirty.is_set() {
                continue;
            }
            let Some(entry) = self.entry_for_key_in(kit, key) else {
                continue;
            };
            let authored = self.authored_package_for_entry(kit, entry);
            let Some(entry) = self.entry_for_key_in(kit, key) else {
                continue;
            };
            let Some((identity, logical_path, kind, package)) =
                campaign_entry_project_parts_with(entry, authored)
            else {
                continue;
            };
            let revision = document.dirty.revision();
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
                },
            );
        }

        // Floating tabs are gone with the tab rack — the tiles tree is the
        // whole open set now, so nothing is recorded as floating.
        let floating_order: Vec<String> = Vec::new();
        let mut tabs = Vec::new();
        for key in self.kits[kit].open_tabs.iter().chain(floating_order.iter()) {
            let Some(entry) = self.entry_for_key_in(kit, key) else {
                continue;
            };
            let authored = self.authored_package_for_entry(kit, entry);
            let Some(entry) = self.entry_for_key_in(kit, key) else {
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
        let selected_identity = self.kits[kit].selected_key.as_ref().and_then(|key| {
            self.entry_for_key_in(kit, key)
                .and_then(campaign_entry_project_parts)
                .map(|(identity, _, _, _)| identity)
        });
        // Every open document's undo trail, not just the edited ones: a tag
        // whose edits were undone back to the original is still a tag whose
        // history the user may want on the other side of a restart.
        let mut history: BTreeMap<String, TagHistory> = BTreeMap::new();
        for (key, document) in &self.kits[kit].parsed_tags {
            let (undo, redo) = document.journal.stacks();
            if undo.is_empty() && redo.is_empty() {
                continue;
            }
            let Some(entry) = self.entry_for_key_in(kit, key) else {
                continue;
            };
            let Some((identity, ..)) = campaign_entry_project_parts(entry) else {
                continue;
            };
            let step = |snapshot: &crate::app::Snapshot| HistoryStep {
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
        if let Some(project) = self.kits[kit].campaign_project.as_mut() {
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
            folders: self.kits[kit].pending_container_folders.clone(),
        }))
    }

    /// Refresh this kit's set of modified tags if anything has changed since it
    /// was last built.
    ///
    /// The signature is the identity of everything that would land in the set:
    /// the dirty documents' keys and the stashed overlays' identities. Both are
    /// small — a handful of entries — so building and comparing it every frame
    /// is far cheaper than the entry lookups the rebuild performs.
    pub(super) fn refresh_modified_tags(&mut self, kit: usize) {
        let mut signature: Vec<String> = self.kits[kit]
            .parsed_tags
            .iter()
            .filter(|(_, document)| document.dirty.is_set())
            .map(|(key, _)| key.clone())
            .collect();
        if let Some(project) = self.kits[kit].campaign_project.as_ref() {
            signature.extend(project.overlays.keys().cloned());
        }
        signature.sort();
        if signature == self.kits[kit].modified_signature {
            return;
        }
        let mut modified = ModifiedTags::default();
        let dirty_keys: Vec<String> = self.kits[kit]
            .parsed_tags
            .iter()
            .filter(|(_, document)| document.dirty.is_set())
            .map(|(key, _)| key.clone())
            .collect();
        for key in dirty_keys {
            if let Some(entry) = self.entry_for_key_in(kit, &key) {
                modified.insert(entry);
            }
        }
        // Stashed tags need not be open, so they are resolved from the project
        // rather than from the open documents.
        let identities: Vec<String> = self.kits[kit]
            .campaign_project
            .as_ref()
            .map(|project| project.overlays.keys().cloned().collect())
            .unwrap_or_default();
        for identity in identities {
            if let Some(entry) = self.campaign_entry_for_identity(kit, &identity) {
                modified.insert(&entry);
            }
        }
        self.kits[kit].modified_tags = std::sync::Arc::new(modified);
        self.kits[kit].modified_signature = signature;
    }

    /// Forget one tag's stashed overlay, so the tag reads as its source has it
    /// again. Returns whether anything was stashed for it.
    ///
    /// Overlays are otherwise only ever inserted: without this, clearing a
    /// document's dirty flag left the edited bytes in the project and reopening
    /// the tag brought them straight back.
    pub(super) fn forget_campaign_overlay(&mut self, kit: usize, key: &str) -> bool {
        let Some(entry) = self.entry_for_key_in(kit, key).cloned() else {
            return false;
        };
        let Some((identity, ..)) = campaign_entry_project_parts(&entry) else {
            return false;
        };
        self.kits[kit]
            .campaign_project
            .as_mut()
            .is_some_and(|project| project.overlays.remove(&identity).is_some())
    }

    /// Whether this kit's project has bytes stashed for `key` — that is, whether
    /// discarding the document would also delete something from disk.
    pub(super) fn tag_has_stashed_overlay(&self, kit: usize, key: &str) -> bool {
        let Some(entry) = self.entry_for_key_in(kit, key) else {
            return false;
        };
        let Some((identity, ..)) = campaign_entry_project_parts(entry) else {
            return false;
        };
        self.kits[kit]
            .campaign_project
            .as_ref()
            .is_some_and(|project| project.overlays.contains_key(&identity))
    }

    /// Forget every stashed overlay in this kit's project, returning how many
    /// tags were carrying one.
    pub(super) fn forget_all_campaign_overlays(&mut self, kit: usize) -> usize {
        let Some(project) = self.kits[kit].campaign_project.as_mut() else {
            return 0;
        };
        let count = project.overlays.len();
        project.overlays.clear();
        count
    }

    /// Identities of the tags this kit currently has stashed, as display paths.
    pub(super) fn stashed_campaign_tags(&self, kit: usize) -> Vec<String> {
        let Some(project) = self.kits[kit].campaign_project.as_ref() else {
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

    /// Throw away everything this workspace has not written into the game:
    /// every stashed overlay and every unsaved document. The tags then reload
    /// exactly as the game ships them.
    pub(super) fn clear_campaign_stash(&mut self, kit: usize, ctx: &egui::Context) {
        self.active = kit;
        let stashed = self.forget_all_campaign_overlays(kit);
        let open = self.kits[kit].open_tabs.clone();
        // Every parsed document goes, not just the dirty ones: a document
        // opened from the project reads clean while still holding the stashed
        // bytes, so keeping it would put the edits straight back.
        {
            let kit_state = &mut self.kits[kit];
            kit_state.parsed_tags.clear();
            kit_state.loading_tags.clear();
            kit_state.bitmap_previews.clear();
            kit_state.model_previews.clear();
            kit_state.field_search.clear();
            kit_state.field_search_applied.clear();
            kit_state.edit_buffers.clear();
        }
        let now = ctx.input(|input| input.time);
        if let Err(error) = self.checkpoint_campaign_project(kit, now) {
            self.status = format!("Could not update the Campaign Evolved project: {error}");
            return;
        }
        for key in open {
            self.select_entry(key, ctx.clone());
        }
        self.status = match stashed {
            0 => "Cleared this workspace's unsaved modifications".to_owned(),
            1 => "Cleared 1 stashed modification".to_owned(),
            n => format!("Cleared {n} stashed modifications"),
        };
    }

    pub(super) fn checkpoint_campaign_project(
        &mut self,
        kit: usize,
        now: f64,
    ) -> Result<bool, String> {
        let Some(snapshot) = self.capture_campaign_project(kit, now)? else {
            return Ok(false);
        };
        let fingerprint = snapshot.fingerprint();
        let Some(project) = self.kits[kit].campaign_project.as_mut() else {
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
        let _write_guard = write_lock
            .lock()
            .map_err(|_| "Campaign project writer lock was poisoned".to_owned())?;
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
    pub(super) fn maybe_autosave_campaign_projects(&mut self, ctx: &egui::Context) {
        for kit in 0..self.kits.len() {
            self.maybe_autosave_campaign_project(kit, ctx);
        }
    }

    fn maybe_autosave_campaign_project(&mut self, kit: usize, ctx: &egui::Context) {
        if !self.current_source_is_campaign_project_capable(kit) {
            return;
        }
        let now = ctx.input(|input| input.time);
        self.ensure_campaign_project(kit, now);
        self.adopt_pending_new_overlays(kit);
        let due = self.kits[kit]
            .campaign_project
            .as_ref()
            .is_some_and(|project| now >= project.next_autosave_at);
        // A save already running means this tick's work would be thrown away, so
        // do not do it. The check used to sit *after* the capture, which is the
        // expensive part.
        if due
            && self.kits[kit]
                .campaign_project
                .as_ref()
                .is_some_and(|project| project.save_in_flight.is_some())
        {
            if let Some(project) = self.kits[kit].campaign_project.as_mut() {
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
                    self.status = format!("Campaign project autosave failed: {error}");
                    return;
                }
            };
            let fingerprint = snapshot.fingerprint();
            let Some(project) = self.kits[kit].campaign_project.as_mut() else {
                return;
            };
            if fingerprint == project.last_saved_fingerprint && project.recovery_path.is_file() {
                project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
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
                let tx = self.tx.clone();
                let repaint = ctx.clone();
                thread::spawn(move || {
                    let result = write_lock
                        .lock()
                        .map_err(|_| "Campaign project writer lock was poisoned".to_owned())
                        .and_then(|_guard| {
                            if latest_write_revision.load(Ordering::SeqCst) != revision {
                                Ok(())
                            } else {
                                save_campaign_project(&path, &snapshot, on_disk.as_ref(), ProjectScope::Session)
                            }
                        });
                    let _ = tx.send(WorkerMessage::CampaignProjectSaved {
                        revision,
                        path,
                        fingerprint,
                        result,
                    });
                    repaint.request_repaint();
                });
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(750));
    }

    pub(super) fn handle_campaign_project_saved(
        &mut self,
        revision: u64,
        path: PathBuf,
        fingerprint: Vec<u8>,
        result: Result<(), String>,
    ) -> bool {
        // Locate the kit whose project this save belongs to. Matching on the
        // path and the in-flight revision is enough, and it means a save that
        // outlives its kit is dropped instead of landing on another one.
        let Some(kit) = self.kits.iter().position(|kit| {
            kit.campaign_project.as_ref().is_some_and(|project| {
                project.recovery_path == path && project.save_in_flight == Some(revision)
            })
        }) else {
            return true;
        };
        let Some(project) = self.kits[kit].campaign_project.as_mut() else {
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
                self.status = format!("Campaign project autosave failed: {error}");
            }
        }
        false
    }

    pub(super) fn begin_open_campaign_project(&mut self, ctx: egui::Context) {
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
    pub(super) fn queue_campaign_project_target(&mut self, kit: usize, path: PathBuf) {
        // Sessions written before the recovery file and the project file were
        // separate recorded the recovery path here. It is not a project the user
        // named, and offering it as a save target would be wrong.
        if is_campaign_recovery_file(&path) {
            return;
        }
        self.kits[kit].pending_campaign_project = Some(PendingCampaignProject {
            path,
            snapshot: None,
        });
    }

    /// Write this workspace's project to its associated `.baboon`, asking for a
    /// destination when it has none yet.
    pub(super) fn save_campaign_project_file(&mut self, kit: usize, now: f64) {
        let Some(path) = self.kits[kit]
            .campaign_project
            .as_ref()
            .and_then(|project| project.project_path.clone())
        else {
            self.save_campaign_project_file_as(kit, now);
            return;
        };
        self.write_campaign_project_file(kit, path, now);
    }

    pub(super) fn save_campaign_project_file_as(&mut self, kit: usize, now: f64) {
        if !self.current_source_is_campaign_project_capable(kit) {
            self.status = "Baboon projects require a Campaign Evolved container source".to_owned();
            return;
        }
        let current = self.kits[kit]
            .campaign_project
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
                self.kits[kit]
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
                self.status =
                    "Baboon projects require a Campaign Evolved container source".to_owned();
                return;
            }
            Err(error) => {
                self.status = format!("Could not save the Baboon project: {error}");
                return;
            }
        };
        // Nothing here knows what is in a file the user named, and it may be an
        // older project entirely, so it is replaced rather than merged into.
        if let Err(error) = save_campaign_project(&path, &snapshot, None, ProjectScope::Session) {
            self.status = error;
            return;
        }
        let count = snapshot.overlays.len();
        if let Some(project) = self.kits[kit].campaign_project.as_mut() {
            project.project_path = Some(path.clone());
        }
        // The recovery file stays the live copy, so it is brought level with what
        // was just written out.
        if let Err(error) = self.checkpoint_campaign_project(kit, now) {
            self.status = format!(
                "Saved {}, but the recovery file failed: {error}",
                path.display()
            );
            return;
        }
        self.status = format!("Saved {count} modified tag(s) to {}", path.display());
    }

    pub(super) fn begin_open_campaign_project_path(&mut self, path: PathBuf, ctx: egui::Context) {
        let snapshot = match load_campaign_project(&path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let source_path = if crate::source::find_paks_dir(&snapshot.source_path).is_some() {
            snapshot.source_path.clone()
        } else if let Some(configured) = self.editing_kit_paths.get("haloce_evolved")
            && crate::source::find_paks_dir(configured).is_some()
        {
            configured.clone()
        } else {
            let Some(selected) = rfd::FileDialog::new()
                .set_title("Locate Campaign Evolved Install or Paks Folder")
                .pick_folder()
            else {
                self.status = "Campaign Evolved project source was not found".to_owned();
                return;
            };
            selected
        };
        self.begin_load_folder_path(source_path, ctx);
        // Staged after the load starts: the loader has routed to a kit and
        // left it active, so this lands on the kit the source will mount into.
        self.kits[self.active].pending_campaign_project = Some(PendingCampaignProject {
            path,
            snapshot: Some(snapshot),
        });
    }

    pub(super) fn apply_pending_campaign_project(
        &mut self,
        kit: usize,
        now: f64,
        ctx: &egui::Context,
    ) {
        let Some(pending) = self.kits[kit].pending_campaign_project.take() else {
            self.ensure_campaign_project(kit, now);
            return;
        };
        if !self.current_source_is_campaign_project_capable(kit) {
            self.status = "Baboon projects require a Campaign Evolved container source".to_owned();
            return;
        }
        // A restored session stages its project file as a save target only; the
        // recovery file this workspace has been autosaving to is the live copy,
        // and `ensure_campaign_project` has just picked it back up.
        let Some(snapshot) = pending.snapshot else {
            self.ensure_campaign_project(kit, now);
            if let Some(project) = self.kits[kit].campaign_project.as_mut() {
                project.project_path = Some(pending.path);
            }
            return;
        };
        let project_path = pending.path;
        self.adopt_project_container_folders(kit, snapshot.folders.clone());

        let mut identity_to_key = HashMap::<String, String>::new();
        let mut restored_revisions = HashMap::<String, u64>::new();
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
            let Some((entry, tag)) = self.new_overlay_entry(kit, &overlay) else {
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
            let Some(entry) = self.campaign_entry_for_identity(kit, &tab.identity) else {
                missing += 1;
                continue;
            };
            identity_to_key.insert(tab.identity.clone(), entry.key);
        }

        // Staged by document key before any tag is opened, so it is already
        // waiting whichever way the document arrives — restored from a stashed
        // edit below, or read back off disk by a worker some frames later.
        self.kits[kit].pending_history = snapshot
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
        let kit_id = self.kits[kit].id;
        self.kits[kit].tag_tree = egui_tiles::Tree::empty(tag_tree_id(kit_id));
        self.kits[kit].open_tabs.clear();
        self.kits[kit].selected_key = None;
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
                    restored_revisions.insert(key.clone(), document.dirty.revision());
                    self.kits[kit].parsed_tags.insert(key.clone(), document);
                    self.apply_pending_history(kit, &key);
                } else {
                    missing += 1;
                    continue;
                }
            } else {
                self.ensure_tag_loading(key.clone(), ctx.clone());
            }
            self.kits[kit].open_tag_pane(&key);
        }
        self.kits[kit].selected_key = snapshot
            .selected_identity
            .as_ref()
            .and_then(|identity| identity_to_key.get(identity))
            .cloned()
            .or_else(|| self.kits[kit].open_tabs.last().cloned());
        // Tiles reveal the active tab themselves, so there is no scroll target
        // to remember; `open_tag_pane` already made each restored tag active.
        if let Some(key) = self.kits[kit].selected_key.clone() {
            self.kits[kit].open_tag_pane(&key);
        }
        let root = self.kits[kit]
            .source
            .as_ref()
            .map(|source| source.source.root_path().to_path_buf());
        let recovery_path = campaign_recovery_path(root.as_deref());
        let mut project =
            ActiveCampaignProject::imported(recovery_path, project_path, &snapshot, now);
        project.captured_revisions = restored_revisions;
        let tabs = snapshot.tabs.len();
        let stashed = snapshot.overlays.len();
        self.kits[kit].campaign_project = Some(project);
        // These overlays have only ever existed in the file the user opened. The
        // recovery file is what every later autosave writes and what the next
        // session picks up, so it is brought level with them now rather than at
        // the mercy of whether anything is edited afterwards.
        if let Err(error) = self.checkpoint_campaign_project(kit, now) {
            self.status = format!("Opened the project, but its recovery file failed: {error}");
            return;
        }
        self.status = if missing == 0 {
            format!("Restored Campaign Evolved project ({tabs} tab(s), {stashed} modified tag(s))")
        } else {
            format!(
                "Restored Campaign Evolved project; skipped {missing} missing or incompatible item(s)"
            )
        };
    }

    pub(super) fn load_campaign_overlay_for_key(&mut self, kit: usize, key: &str) -> bool {
        if self.kits[kit].parsed_tags.contains_key(key) {
            return true;
        }
        let Some(entry) = self.entry_for_key_in(kit, key).cloned() else {
            return false;
        };
        let Some((identity, _, _, _)) = campaign_entry_project_parts(&entry) else {
            return false;
        };
        let Some(overlay) = self.kits[kit]
            .campaign_project
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
                let revision = document.dirty.revision();
                self.kits[kit].parsed_tags.insert(key.to_owned(), document);
                if let Some(project) = self.kits[kit].campaign_project.as_mut() {
                    project.captured_revisions.insert(key.to_owned(), revision);
                }
                true
            }
            Err(error) => {
                self.status = format!("Could not restore {}: {error}", entry.display_path);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }

    fn history_of(sizes: &[usize]) -> TagHistory {
        TagHistory {
            undo: sizes
                .iter()
                .enumerate()
                .map(|(index, size)| HistoryStep {
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
                history[identity].undo.last().map(|step| step.label.as_str()),
                Some("edit 2"),
                "{identity} kept its most recent step"
            );
        }
    }

    /// The history table is replaced wholesale, so a save that cannot tell it
    /// is unchanged rewrites every step. During a drag that is tens of
    /// megabytes twice a second.
    #[test]
    fn an_unchanged_history_is_not_rewritten() {
        let path = temp_project("history-skip");
        let mut snapshot = snapshot_of(vec![overlay("a", b"one")]);
        snapshot.history = BTreeMap::from([(
            "a".to_owned(),
            TagHistory {
                undo: vec![HistoryStep {
                    label: "Edit color".to_owned(),
                    bytes: Arc::new(vec![1, 2, 3]),
                }],
                redo: Vec::new(),
                revision: 7,
            },
        )]);
        save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();
        let saved = snapshot.digests();

        // Reach into the file and mark the row, so a rewrite is detectable.
        let connection = Connection::open(&path).unwrap();
        connection
            .execute("UPDATE history SET label = 'sentinel'", [])
            .unwrap();
        drop(connection);

        // The tag was edited again, but the journal did not move — the same
        // undo steps, the same revision.
        let mut later = snapshot_of(vec![overlay("a", b"two")]);
        later.history = snapshot.history.clone();
        save_campaign_project(&path, &later, Some(&saved), ProjectScope::Session).unwrap();

        let loaded = load_campaign_project(&path).unwrap();
        assert_eq!(
            loaded.history["a"].undo[0].label, "sentinel",
            "an unchanged history was rewritten"
        );
        assert_eq!(*loaded.overlays["a"].bytes, b"two".to_vec());

        // A journal that did move is written again.
        let mut moved = later.clone();
        moved.history.get_mut("a").unwrap().revision = 8;
        save_campaign_project(&path, &moved, Some(&later.digests()), ProjectScope::Session).unwrap();
        let loaded = load_campaign_project(&path).unwrap();
        assert_eq!(loaded.history["a"].undo[0].label, "Edit color");

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
        save_campaign_project(&path, &second, Some(&first.digests()), ProjectScope::Session).unwrap();

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
        std::env::temp_dir().join(format!(
            "baboon-{name}-{}-{}.baboon",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
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
                        label: "Edit color".to_owned(),
                        bytes: Arc::new(vec![7, 7, 7]),
                    }],
                    redo: vec![HistoryStep {
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
        const PAKS: &str = "/Users/camden/Halo/halo-campaign-evolved_pc/Meteorite/Content/Paks";
        if !std::path::Path::new(PAKS).exists() {
            eprintln!("skipping: Campaign Evolved not present");
            return;
        }
        let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = crate::format::TagNameIndex::load_from_definitions(&defs);
        let loaded = crate::source::load_iostore_container_set(
            std::path::PathBuf::from(PAKS),
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
}
