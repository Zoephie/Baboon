//! Campaign Evolved project/recovery persistence.
//!
//! A `.baboon` file is a small SQLite database. Clean tabs are represented by
//! canonical tag identities while modified/new tags also carry their serialized
//! tag bytes, allowing a project to be reopened without modifying the base paks.

use super::*;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

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
    pub(super) bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct CampaignProjectSnapshot {
    pub(super) game: String,
    pub(super) source_path: PathBuf,
    pub(super) selected_identity: Option<String>,
    pub(super) tabs: Vec<CampaignProjectTab>,
    pub(super) overlays: HashMap<String, CampaignProjectOverlay>,
}

impl CampaignProjectSnapshot {
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
            hasher.update(&overlay.bytes);
        }
        hasher.finalize().to_vec()
    }
}

pub(super) struct ActiveCampaignProject {
    pub(super) path: PathBuf,
    pub(super) overlays: HashMap<String, CampaignProjectOverlay>,
    pub(super) last_saved_fingerprint: Vec<u8>,
    pub(super) next_autosave_at: f64,
    pub(super) revision: u64,
    pub(super) save_in_flight: Option<u64>,
    pub(super) write_lock: Arc<Mutex<()>>,
    pub(super) latest_write_revision: Arc<AtomicU64>,
}

impl ActiveCampaignProject {
    pub(super) fn fresh(path: PathBuf, now: f64) -> Self {
        Self {
            path,
            overlays: HashMap::new(),
            last_saved_fingerprint: Vec::new(),
            next_autosave_at: now + CAMPAIGN_PROJECT_AUTOSAVE_SECS,
            revision: 0,
            save_in_flight: None,
            write_lock: Arc::new(Mutex::new(())),
            latest_write_revision: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(super) fn from_snapshot(
        path: PathBuf,
        snapshot: &CampaignProjectSnapshot,
        now: f64,
    ) -> Self {
        Self {
            path,
            overlays: snapshot.overlays.clone(),
            last_saved_fingerprint: snapshot.fingerprint(),
            next_autosave_at: now + CAMPAIGN_PROJECT_AUTOSAVE_SECS,
            revision: 0,
            save_in_flight: None,
            write_lock: Arc::new(Mutex::new(())),
            latest_write_revision: Arc::new(AtomicU64::new(0)),
        }
    }
}

pub(super) struct PendingCampaignProject {
    pub(super) path: PathBuf,
    pub(super) snapshot: CampaignProjectSnapshot,
}

pub(super) fn campaign_recovery_path() -> PathBuf {
    crate::storage::data_path("campaign_evolved_recovery.baboon")
}

pub(super) fn save_campaign_project(
    path: &Path,
    snapshot: &CampaignProjectSnapshot,
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
             );",
        )
        .map_err(|error| format!("Could not initialize project database: {error}"))?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start project transaction: {error}"))?;
    transaction
        .execute("DELETE FROM project", [])
        .and_then(|_| transaction.execute("DELETE FROM tabs", []))
        .and_then(|_| transaction.execute("DELETE FROM overlays", []))
        .map_err(|error| format!("Could not reset project database: {error}"))?;
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
        transaction
            .execute(
                "INSERT INTO overlays
                 (identity, group_tag, logical_path, kind, package, bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    overlay.identity,
                    i64::from(overlay.group_tag),
                    overlay.logical_path,
                    overlay.kind.as_str(),
                    overlay.package,
                    overlay.bytes,
                ],
            )
            .map_err(|error| {
                format!(
                    "Could not write project tag {}: {error}",
                    overlay.logical_path
                )
            })?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Could not commit project database: {error}"))
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
                    bytes,
                },
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;

    Ok(CampaignProjectSnapshot {
        game,
        source_path: PathBuf::from(source_path),
        selected_identity,
        tabs,
        overlays,
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
    let logical_path = logical_path_from_display(&entry.display_path);
    let kind = match &entry.location {
        TagEntryLocation::Container { .. } => CampaignProjectTagKind::Existing,
        TagEntryLocation::NewContainer { .. } => CampaignProjectTagKind::New,
        _ => return None,
    };
    let package = match &entry.location {
        TagEntryLocation::NewContainer { package, .. } => Some(package.clone()),
        _ => None,
    };
    let identity = format!("{:08x}:{logical_path}", entry.group_tag);
    Some((identity, logical_path, kind, package))
}

impl Baboon {
    pub(super) fn current_source_is_campaign_project_capable(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| matches!(source.source, TagSource::IoStoreContainerSet { .. }))
    }

    pub(super) fn campaign_entry_for_identity(&self, identity: &str) -> Option<TagEntry> {
        let source = self.source.as_ref()?;
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

    fn ensure_campaign_project(&mut self, now: f64) {
        if self.current_source_is_campaign_project_capable() && self.campaign_project.is_none() {
            self.campaign_project =
                Some(ActiveCampaignProject::fresh(campaign_recovery_path(), now));
        }
    }

    pub(super) fn capture_campaign_project(
        &mut self,
        now: f64,
    ) -> Result<Option<CampaignProjectSnapshot>, String> {
        let Some(source) = self.source.as_ref() else {
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
        self.ensure_campaign_project(now);
        let mut overlays = self
            .campaign_project
            .as_ref()
            .map(|project| project.overlays.clone())
            .unwrap_or_default();

        for (key, document) in &self.parsed_tags {
            if !document.dirty {
                continue;
            }
            let Some(entry) = self.entry_for_key(key) else {
                continue;
            };
            let Some((identity, logical_path, kind, package)) = campaign_entry_project_parts(entry)
            else {
                continue;
            };
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
                    bytes,
                },
            );
        }

        let floating_order = {
            let mut keys = self.floating_tabs.iter().cloned().collect::<Vec<_>>();
            keys.sort();
            keys
        };
        let mut tabs = Vec::new();
        for key in self.open_tabs.iter().chain(floating_order.iter()) {
            let Some(entry) = self.entry_for_key(key) else {
                continue;
            };
            let Some((identity, logical_path, kind, package)) = campaign_entry_project_parts(entry)
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
                floating: self.floating_tabs.contains(key),
            });
        }
        let selected_identity = self.selected_key.as_ref().and_then(|key| {
            self.entry_for_key(key)
                .and_then(campaign_entry_project_parts)
                .map(|(identity, _, _, _)| identity)
        });
        if let Some(project) = self.campaign_project.as_mut() {
            project.overlays = overlays.clone();
        }
        Ok(Some(CampaignProjectSnapshot {
            game,
            source_path,
            selected_identity,
            tabs,
            overlays,
        }))
    }

    pub(super) fn checkpoint_campaign_project(&mut self, now: f64) -> Result<bool, String> {
        let Some(snapshot) = self.capture_campaign_project(now)? else {
            return Ok(false);
        };
        let fingerprint = snapshot.fingerprint();
        let Some(project) = self.campaign_project.as_mut() else {
            return Ok(false);
        };
        if fingerprint == project.last_saved_fingerprint && project.path.is_file() {
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
        save_campaign_project(&project.path, &snapshot)?;
        project.last_saved_fingerprint = fingerprint;
        project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
        if let Some(session) = self.current_session_state() {
            let _ = save_last_session(&session);
        }
        Ok(true)
    }

    pub(super) fn maybe_autosave_campaign_project(&mut self, ctx: &egui::Context) {
        if !self.current_source_is_campaign_project_capable() {
            return;
        }
        let now = ctx.input(|input| input.time);
        self.ensure_campaign_project(now);
        let due = self
            .campaign_project
            .as_ref()
            .is_some_and(|project| now >= project.next_autosave_at);
        if due {
            let snapshot = match self.capture_campaign_project(now) {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => return,
                Err(error) => {
                    self.status = format!("Campaign project autosave failed: {error}");
                    return;
                }
            };
            let fingerprint = snapshot.fingerprint();
            let Some(project) = self.campaign_project.as_mut() else {
                return;
            };
            if project.save_in_flight.is_some() {
                project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
            } else if fingerprint == project.last_saved_fingerprint && project.path.is_file() {
                project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
            } else {
                project.revision = project.revision.wrapping_add(1);
                let revision = project.revision;
                let path = project.path.clone();
                let write_lock = project.write_lock.clone();
                let latest_write_revision = project.latest_write_revision.clone();
                latest_write_revision.store(revision, Ordering::SeqCst);
                project.save_in_flight = Some(revision);
                project.next_autosave_at = now + CAMPAIGN_PROJECT_AUTOSAVE_SECS;
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
                                save_campaign_project(&path, &snapshot)
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
        let Some(project) = self.campaign_project.as_mut() else {
            return true;
        };
        if project.path != path || project.save_in_flight != Some(revision) {
            return true;
        }
        project.save_in_flight = None;
        match result {
            Ok(()) => {
                project.last_saved_fingerprint = fingerprint;
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
        self.pending_campaign_project = Some(PendingCampaignProject { path, snapshot });
        self.begin_load_folder_path(source_path, ctx);
    }

    pub(super) fn apply_pending_campaign_project(&mut self, now: f64, ctx: &egui::Context) {
        let Some(pending) = self.pending_campaign_project.take() else {
            self.ensure_campaign_project(now);
            return;
        };
        if !self.current_source_is_campaign_project_capable() {
            self.status = "Baboon projects require a Campaign Evolved container source".to_owned();
            return;
        }

        let mut identity_to_key = HashMap::<String, String>::new();
        let mut missing = 0usize;

        // Recreate new project tags first, including ones that are currently
        // closed but must remain part of future exports.
        let new_overlays = pending
            .snapshot
            .overlays
            .values()
            .filter(|overlay| overlay.kind == CampaignProjectTagKind::New)
            .cloned()
            .collect::<Vec<_>>();
        for overlay in new_overlays {
            let Some(group_name) = self.names.name_for(overlay.group_tag).map(str::to_owned) else {
                missing += 1;
                continue;
            };
            let extension = group_tag_to_extension(overlay.group_tag)
                .unwrap_or(group_name.as_str())
                .to_owned();
            let Ok(tag) = TagFile::read_from_bytes(&overlay.bytes) else {
                missing += 1;
                continue;
            };
            let Some((template_container, template_rel)) =
                self.find_container_template(overlay.group_tag)
            else {
                missing += 1;
                continue;
            };
            let package = overlay
                .package
                .clone()
                .unwrap_or_else(|| format!("/Game/Tags/{}-{group_name}", overlay.logical_path));
            let key = format!("newtag:{package}");
            let entry = TagEntry {
                key: key.clone(),
                display_path: format!("{}.{}", overlay.logical_path, extension),
                group_tag: overlay.group_tag,
                group_name: Some(group_name),
                location: TagEntryLocation::NewContainer {
                    template_container,
                    template_rel,
                    package,
                    group_tag: overlay.group_tag,
                },
            };
            self.register_in_memory_tag(entry, tag);
            identity_to_key.insert(overlay.identity.clone(), key);
        }

        for tab in &pending.snapshot.tabs {
            if identity_to_key.contains_key(&tab.identity) {
                continue;
            }
            let Some(entry) = self.campaign_entry_for_identity(&tab.identity) else {
                missing += 1;
                continue;
            };
            identity_to_key.insert(tab.identity.clone(), entry.key);
        }

        self.open_tabs.clear();
        self.floating_tabs.clear();
        self.selected_key = None;
        for tab in &pending.snapshot.tabs {
            let Some(key) = identity_to_key.get(&tab.identity).cloned() else {
                continue;
            };
            if let Some(overlay) = pending.snapshot.overlays.get(&tab.identity) {
                if let Ok(tag) = TagFile::read_from_bytes(&overlay.bytes) {
                    self.parsed_tags
                        .insert(key.clone(), TagDocument::modified(tag));
                } else {
                    missing += 1;
                    continue;
                }
            } else {
                self.ensure_tag_loading(key.clone(), ctx.clone());
            }
            if tab.floating {
                self.floating_tabs.insert(key);
            } else {
                self.open_tabs.push(key);
            }
        }
        self.selected_key = pending
            .snapshot
            .selected_identity
            .as_ref()
            .and_then(|identity| identity_to_key.get(identity))
            .cloned()
            .or_else(|| self.open_tabs.last().cloned());
        self.tab_scroll_target = self
            .selected_key
            .as_ref()
            .filter(|key| self.open_tabs.contains(key))
            .cloned();
        self.campaign_project = Some(ActiveCampaignProject::from_snapshot(
            pending.path,
            &pending.snapshot,
            now,
        ));
        self.status = if missing == 0 {
            format!(
                "Restored Campaign Evolved project ({} tab(s), {} modified tag(s))",
                pending.snapshot.tabs.len(),
                pending.snapshot.overlays.len()
            )
        } else {
            format!(
                "Restored Campaign Evolved project; skipped {missing} missing or incompatible item(s)"
            )
        };
    }

    pub(super) fn load_campaign_overlay_for_key(&mut self, key: &str) -> bool {
        if self.parsed_tags.contains_key(key) {
            return true;
        }
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return false;
        };
        let Some((identity, _, _, _)) = campaign_entry_project_parts(&entry) else {
            return false;
        };
        let Some(overlay) = self
            .campaign_project
            .as_ref()
            .and_then(|project| project.overlays.get(&identity))
            .cloned()
        else {
            return false;
        };
        match TagFile::read_from_bytes(&overlay.bytes) {
            Ok(tag) => {
                self.parsed_tags
                    .insert(key.to_owned(), TagDocument::modified(tag));
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
            bytes: vec![0, 1, 2, 0xff],
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
        };
        save_campaign_project(&path, &snapshot).unwrap();
        let loaded = load_campaign_project(&path).unwrap();
        assert_eq!(loaded.tabs.len(), 1);
        assert_eq!(loaded.tabs[0].identity, overlay.identity);
        assert_eq!(
            loaded.overlays[&loaded.tabs[0].identity].bytes,
            overlay.bytes
        );
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
}
