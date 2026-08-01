//! Application actions and asynchronous workflow coordination for [`Baboon`].
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

mod updates;
use updates::*;
mod terminal;
pub(super) use terminal::open_terminal_log;
#[cfg(test)]
use terminal::terminal_log_timestamp;
use terminal::{
    TerminalStopResult, append_terminal_log_path, create_terminal_log_file,
    run_terminal_command_for_reimport, send_terminal_line, stop_terminal_process,
    stream_terminal_output, trim_terminal_lines,
};
mod tools;
use tools::*;
mod scenario_launch;
use scenario_launch::*;
mod queries;
use queries::*;
mod saving;
pub(super) use saving::{
    available_definition_games, load_new_tag_groups, new_tag_output_path_from_dialog,
};
use saving::{
    entries_for_keys, lexical_normalize_path, ordered_unique_keys,
    register_saved_copy_in_loaded_source, save_as_extension, save_as_file_name, save_as_start_dir,
};
mod documents;
mod loading;
#[cfg(test)]
use loading::loaded_source_status;
mod references;
use anyhow::Context as _;
use references::*;

const TERMINAL_VISIBLE_LINE_LIMIT: usize = 20_000;
const TERMINAL_VISIBLE_LINE_TRIM_TARGET: usize = 18_000;
const ENTRY_INDEX_REFRESH_INTERVAL_SECS: f64 = 30.0;

/// Map a container `.ubulk` path to the UE package path the runtime hashes.
/// `Meteorite/Content/Tags/objects/.../foo-biped.ubulk` → `/Game/Tags/objects/.../foo-biped`.
/// Normalize a user-entered container tag path: lowercase, `\`→`/`, collapse
/// repeated slashes, and trim leading/trailing slashes and any tag extension.
/// Yields the container-relative logical path (e.g. `objects/foo/bar`).
/// Walk up from a resolved `Paks` directory to the folder a user would have
/// picked to open it.
///
/// Sessions written before the chosen folder was recorded hold the inner path,
/// and restoring from it remembers *that* as a recent folder -- so "Paks"
/// reappeared after every restart however often it was removed. Walking up
/// while the parent still resolves to the same directory undoes that without
/// assuming a particular layout.
fn install_root_for_paks(paks_dir: &Path) -> PathBuf {
    // The two layouts `find_paks_dir` looks for directly, in its own order.
    // Probing it instead would walk too far: it also *searches* four levels
    // down, so distant ancestors resolve to this same directory and the walk
    // would climb out of the install entirely.
    for suffix in [
        ["Meteorite", "Content", "Paks"].as_slice(),
        ["Content", "Paks"].as_slice(),
    ] {
        let mut candidate = paks_dir;
        let matched = suffix.iter().rev().all(|expected| {
            let hit = candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(expected));
            if hit && let Some(parent) = candidate.parent() {
                candidate = parent;
            }
            hit
        });
        if matched {
            return candidate.to_path_buf();
        }
    }
    paks_dir.to_path_buf()
}

fn normalize_container_tag_rel(input: &str) -> String {
    let lowered = input.trim().replace('\\', "/").to_ascii_lowercase();
    let mut segments: Vec<&str> = lowered
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    // Drop a trailing tag extension on the leaf (`foo.biped` → `foo`).
    if let Some(last) = segments.last_mut()
        && let Some((stem, _ext)) = last.rsplit_once('.')
        && !stem.is_empty()
    {
        *last = stem;
    }
    segments.join("/")
}

/// The UE package path a brand-new tag will be written at, from its normalized
/// container-relative path and group name (`objects/foo/bar` + `camera_track`
/// → `/Game/Tags/objects/foo/bar-camera_track`).
///
/// Shared by creation and rename on purpose: the entry key derives from this,
/// and a rename that derived either differently would produce an entry the save
/// and project-overlay paths no longer recognize as the same tag.
fn new_container_package(logical: &str, group_name: &str) -> String {
    format!("/Game/Tags/{logical}-{group_name}")
}

/// The browser/document key for a new tag at `package`. Prefixed so it cannot
/// collide with a mounted container tag's key.
fn new_container_key(package: &str) -> String {
    format!("newtag:{package}")
}

fn container_rel_to_package_path(rel: &str) -> Option<String> {
    let no_ext = rel.strip_suffix(".ubulk").unwrap_or(rel);
    let after = no_ext.strip_prefix("Meteorite/Content/").unwrap_or(no_ext);
    Some(format!("/Game/{after}"))
}

/// Prompt for an override `.utoc` output path, defaulting to `default_name`.
fn pick_override_utoc(default_name: &str) -> Option<PathBuf> {
    let mut output = rfd::FileDialog::new()
        .set_title("Export Override Container")
        .set_file_name(default_name)
        .add_filter("IoStore TOC", &["utoc"])
        .save_file()?;
    if output.extension().is_none() {
        output.set_extension("utoc");
    }
    Some(ensure_priority_suffix(output))
}

/// Force the `_P` suffix onto a mod's file name.
///
/// It is what gives an override container priority over the game's own
/// containers; without it the mod mounts alongside them and the base tag wins,
/// so the mod builds correctly and does nothing. That is not a naming
/// preference to be respected -- a mod without it is simply broken -- and it is
/// exactly what a user renaming the default to something meaningful drops.
///
/// Confirmed against the game's own mount path: it compares the last six
/// characters to `_P.pak` case-insensitively and adds `100 × version` to the
/// pak order, where the version defaults to 1 and only rises if the name
/// carries `_<digits>_` before the suffix. A base pak scores 4, so any `_P`
/// mod at 104 outranks it, and `_p` is accepted just as readily.
fn ensure_priority_suffix(path: PathBuf) -> PathBuf {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return path;
    };
    if stem.len() >= 2 && stem[stem.len() - 2..].eq_ignore_ascii_case("_p") {
        return path;
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("utoc");
    path.with_file_name(format!("{stem}_P.{extension}"))
}

#[cfg(any(windows, test))]
fn explorer_select_args(path: &Path) -> [std::ffi::OsString; 2] {
    [
        std::ffi::OsString::from("/select,"),
        path.as_os_str().to_owned(),
    ]
}

/// What a stashed overlay is, for the export review.
///
/// Pure so the rule can be tested without a mounted game, which is what
/// reproducing the report needed: a tag stashed as modified whose bytes turn
/// out to equal the game's own copy.
pub(in crate::app) fn classify_overlay(
    resolvable: bool,
    kind: CampaignProjectTagKind,
    matches_shipped: bool,
) -> ModExportChange {
    match (resolvable, kind) {
        (false, _) => ModExportChange::Unresolved,
        (true, CampaignProjectTagKind::New) => ModExportChange::New,
        (true, CampaignProjectTagKind::Existing) if matches_shipped => ModExportChange::Unchanged,
        (true, CampaignProjectTagKind::Existing) => ModExportChange::Modified,
    }
}


impl Baboon {
    /// Resolve (and cache) the Wwise media a Campaign Evolved `sound` tag binds
    /// to. Returns `None` for every other game and source kind.
    ///
    /// Resolution walks package imports and parses several cooked packages, so
    /// the result is memoized per tag key — the sound panel asks for it every
    /// frame. An unbound tag caches an empty binding, so stubs are not re-walked.
    pub(in crate::app) fn ce_sound_binding(
        &mut self,
        kit_index: usize,
        tag_key: &str,
        entry: &TagEntry,
    ) -> Option<std::sync::Arc<crate::source::ce_audio::CeSoundBinding>> {
        use crate::source::ce_audio;

        if !crate::app::editor::is_sound_group(entry.group_tag) {
            return None;
        }
        if let Some(hit) = self.kits[kit_index].ce_sound_bindings.get(tag_key) {
            return Some(hit.clone());
        }

        let TagEntryLocation::Container { rel_path, .. } = &entry.location else {
            return None;
        };
        let package = ce_audio::tag_package_for_rel_path(rel_path)?;
        self.ce_binding_for_package(kit_index, tag_key, &package)
    }

    /// The Wwise binding of a `sound` tag reached by *reference* rather than by
    /// browser entry — a `sound_looping` track's component, a dialogue
    /// vocalization. Resolves the reference against the mounted containers and
    /// then follows the same walk, memoized under the same per-kit cache.
    pub(in crate::app) fn ce_sound_binding_for_ref(
        &mut self,
        kit_index: usize,
        group_tag: u32,
        reference: &str,
    ) -> Option<std::sync::Arc<crate::source::ce_audio::CeSoundBinding>> {
        use crate::source::ce_audio;

        let Some(TagSource::IoStoreContainerSet { index, .. }) =
            self.kits[kit_index].source.as_ref().map(|s| &s.source)
        else {
            return None;
        };
        let (_, rel_path) = index.lookup(group_tag, reference)?;
        let package = ce_audio::tag_package_for_rel_path(rel_path)?;
        let cache_key = format!("ref:{package}");
        self.ce_binding_for_package(kit_index, &cache_key, &package)
    }

    /// Walk one cooked package out to its Wwise media, memoized under
    /// `cache_key` in this kit. Shared by both entry points above.
    fn ce_binding_for_package(
        &mut self,
        kit_index: usize,
        cache_key: &str,
        package: &str,
    ) -> Option<std::sync::Arc<crate::source::ce_audio::CeSoundBinding>> {
        use crate::source::ce_audio;

        if let Some(hit) = self.kits[kit_index].ce_sound_bindings.get(cache_key) {
            return Some(hit.clone());
        }

        // Checked before the usmap is parsed, as it always has been: a non-CE
        // source must bail out without paying for the bundled reflection data.
        // The `matches!` ends its borrow immediately, which the destructure
        // below could not do across the `ce_usmap` assignment.
        if !matches!(
            self.kits[kit_index].source.as_ref().map(|s| &s.source),
            Some(TagSource::IoStoreContainerSet { .. })
        ) {
            return None;
        }
        if self.ce_usmap.is_none() {
            match blam_tags::iostore::usmap::Usmap::meteorite() {
                Ok(u) => self.ce_usmap = Some(std::sync::Arc::new(u)),
                Err(err) => {
                    eprintln!("campaign evolved: could not parse bundled usmap: {err}");
                    return None;
                }
            }
        }
        let usmap = self.ce_usmap.clone()?;

        let Some(TagSource::IoStoreContainerSet {
            root,
            containers,
            packages,
            ..
        }) = self.kits[kit_index].source.as_ref().map(|s| &s.source)
        else {
            return None;
        };

        // `kits` and `audio` are disjoint fields, so the pak set can be handed
        // to the walk while the container borrow is live. It is needed for
        // events whose media is cooked inside a SoundBank.
        let binding = std::sync::Arc::new(ce_audio::resolve_sound_binding(
            containers,
            packages,
            &usmap,
            package,
            Some((root.as_path(), &mut self.audio.ce_media)),
        ));
        self.kits[kit_index]
            .ce_sound_bindings
            .insert(cache_key.to_owned(), binding.clone());
        Some(binding)
    }

    /// Drain a queued referenced-sound click from a container source: resolve
    /// the reference's own Wwise binding, then queue the same playback or
    /// extraction the primary sound player would.
    pub(super) fn process_ce_sound_ref(&mut self) {
        let Some((kit_id, request)) = self.pending_ce_sound_ref.take() else {
            return;
        };
        let Some(kit_index) = self.kit_index(kit_id) else {
            return;
        };
        let paks_root = match self.kits[kit_index].source.as_ref().map(|s| &s.source) {
            Some(TagSource::IoStoreContainerSet { root, .. }) => root.clone(),
            _ => return,
        };
        let Some(binding) =
            self.ce_sound_binding_for_ref(kit_index, request.group_tag, &request.reference)
        else {
            self.status = format!("{} not found in mounted containers", request.label);
            return;
        };
        if binding.is_empty() {
            self.status = format!("{} has no audio bound", request.label);
            return;
        }

        let language = binding.language_to_show(self.audio.language.as_deref());
        let media: Vec<crate::source::ce_audio::CeSoundMedia> = binding
            .media_for_language(&language)
            .into_iter()
            .cloned()
            .collect();
        if !request.extract {
            // Play the first permutation, matching the loose-folder player.
            let Some(first) = media.into_iter().next() else {
                return;
            };
            self.audio.pending = Some(crate::app::audio::SoundAction::PlayCeMedia {
                paks_root,
                label: format!("{} \u{00B7} {}", request.label, first.display_name()),
                media: Box::new(first),
            });
            return;
        }
        let Some(base) = rfd::FileDialog::new()
            .set_title(format!("Extract {}", request.label))
            .pick_folder()
        else {
            return;
        };
        let items = media
            .into_iter()
            .map(|m| crate::app::sound_extract::ExtractItem {
                out_path: base.join(format!(
                    "{}.wav",
                    crate::app::sound_extract::sanitize_component(&m.display_name())
                )),
                source: crate::app::sound_extract::ExtractSource::CeMedia {
                    paks_root: paks_root.clone(),
                    media: Box::new(m),
                },
            })
            .collect();
        self.pending_sound_extract = Some(crate::app::sound_extract::ExtractRequest {
            items,
            tags_root: None,
            label: request.label,
        });
    }

    fn push_terminal_line(&mut self, line: String) {
        self.terminal.lines.push(TerminalLineEntry::new(line));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
    }

    pub(super) fn process_worker_messages(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            let stale = match message {
                WorkerMessage::TerminalLine(line) => self.handle_terminal_line(line),
                WorkerMessage::TerminalLogError(error) => self.handle_terminal_log_error(error),
                WorkerMessage::TerminalDone { run_id } => self.handle_terminal_done(run_id),
                WorkerMessage::UpdateCheckFinished { silent, result } => {
                    self.handle_update_check_finished(silent, result)
                }
                WorkerMessage::FieldValueSearchFinished {
                    stamp,
                    query,
                    result,
                } => self.handle_field_value_search_finished(stamp, query, result),
                WorkerMessage::FieldIndexBuilt { stamp, blobs } => {
                    self.handle_field_index_built(stamp, blobs)
                }
                WorkerMessage::FindAllProgress {
                    stamp,
                    request_id,
                    processed,
                    total,
                } => {
                    if self.resolve_stamp(stamp).is_some() && request_id == self.find.all_request_id
                    {
                        self.find.progress = Some((processed, total));
                    }
                    false
                }
                WorkerMessage::FindAllFinished {
                    stamp,
                    request_id,
                    occurrences,
                    unreadable,
                } => {
                    if self.resolve_stamp(stamp).is_some() && request_id == self.find.all_request_id
                    {
                        self.find.all_closed_occurrences = occurrences;
                        self.find.unreadable = unreadable;
                        self.find.searching = false;
                        self.find.progress = None;
                    }
                    false
                }
                WorkerMessage::ReverseDependenciesBuilt { stamp, index } => {
                    self.handle_reverse_dependencies_built(stamp, index)
                }
                WorkerMessage::ReferenceIndexProgress {
                    stamp,
                    processed,
                    total,
                } => self.handle_reference_index_progress(stamp, processed, total, ctx),
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path,
                } => self.handle_source_loaded(kit, result, recent_path, ctx),
                WorkerMessage::ChimpMounted { stamp, result } => {
                    self.handle_chimp_mounted(stamp, result, ctx.clone())
                }
                WorkerMessage::ChimpTypesIndexed { stamp, index } => {
                    self.handle_chimp_types_indexed(stamp, index)
                }
                WorkerMessage::ChimpPackageLoaded {
                    stamp,
                    package,
                    result,
                } => self.handle_chimp_package_loaded(stamp, package, result),
                WorkerMessage::TagLoaded { kit, key, result } => {
                    self.handle_tag_loaded(kit, key, result)
                }
                WorkerMessage::BitmapReimportFinished { kit, key, result } => {
                    self.handle_bitmap_reimport_finished(kit, key, result)
                }
                WorkerMessage::ExportFinished(result) => self.handle_export_finished(result),
                WorkerMessage::PokePreflightFinished { kit, key, result } => {
                    self.handle_poke_preflight(kit, key, result);
                    false
                }
                WorkerMessage::PokeWriteFinished { kit, key, result } => {
                    self.handle_poke_write(kit, key, result);
                    false
                }
                WorkerMessage::PokeDirectFinished { kit, key, result } => {
                    self.handle_poke_direct(kit, key, result);
                    false
                }
                WorkerMessage::PokeUndoFinished { result } => {
                    self.handle_poke_undo(result);
                    false
                }
                WorkerMessage::CampaignProjectSaved {
                    revision,
                    path,
                    fingerprint,
                    result,
                } => self.handle_campaign_project_saved(revision, path, fingerprint, result),
                WorkerMessage::FolderRefactorProgress(progress) => {
                    self.handle_folder_refactor_progress(progress)
                }
                WorkerMessage::FolderRefactorFinished { stamp, result } => {
                    self.handle_folder_refactor_finished(stamp, result)
                }
                WorkerMessage::FolderConversionProgress(progress) => {
                    self.handle_folder_conversion_progress(progress)
                }
                WorkerMessage::FolderConversionFinished(report) => {
                    self.handle_folder_conversion_finished(report)
                }
                WorkerMessage::AllEntriesScanned { stamp, result } => {
                    self.handle_all_entries_scanned(stamp, result, ctx)
                }
                WorkerMessage::EntryIndexScanProgress {
                    stamp,
                    processed,
                    total,
                    matched,
                } => self.handle_entry_index_scan_progress(stamp, processed, total, matched, ctx),
                WorkerMessage::EntryIndexRefreshed { stamp, result } => {
                    self.handle_entry_index_refreshed(stamp, result, ctx)
                }
                WorkerMessage::EntryIndexSaved {
                    stamp,
                    path,
                    result,
                } => self.handle_entry_index_saved(stamp, path, result),
            };
            if stale {
                continue;
            }
        }
    }

    pub(super) fn begin_load_single(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new().set_title("Load Tag").pick_file() else {
            return;
        };
        self.begin_load_single_path(path, ctx);
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(super) fn begin_load_single_path(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.open_kit_for(&path) {
            self.status = format!("Switched to {}", path.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.active_kit_id();
        let names = self.default_names.clone();
        self.status = format!("Loading {}", path.display());
        thread::spawn(move || {
            let result = load_single_file(path, &names).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::SourceLoaded {
                kit,
                result,
                recent_path: None,
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_load_folder(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Load Folder")
            .pick_folder()
        else {
            return;
        };
        self.begin_load_folder_path(path, ctx);
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(super) fn begin_load_folder_path(&mut self, path: PathBuf, ctx: egui::Context) {
        // A UE5 `Paks` directory (Halo: Campaign Evolved) is mounted as a
        // container set rather than walked as loose files.
        if let Some(paks) = crate::source::find_paks_dir(&path) {
            // Remember the folder the user picked, not the container directory
            // found inside it — the same way a loose kit remembers its root
            // rather than the `tags/` subfolder it actually scans.
            self.begin_load_iostore_container_set_path(paks, path, ctx);
            return;
        }
        if self.open_kit_for(&path) {
            self.status = format!("Switched to {}", path.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.active_kit_id();
        let names = self.default_names.clone();
        let definitions_root = locate_definitions_root();
        let ek_folder_aliases = self.ek_folder_aliases.clone();
        let folder_info = match resolve_folder_root(&path, &ek_folder_aliases) {
            Ok(info) => info,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        self.status = match folder_info.game {
            Some(game) => format!("Indexing {} as {game}", folder_info.scan_root.display()),
            None => format!("Indexing {}", folder_info.scan_root.display()),
        };
        let recent_path = clean_recent_path(path.clone());
        thread::spawn(move || {
            let result = load_folder(path, &names, &definitions_root, &ek_folder_aliases)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::SourceLoaded {
                kit,
                result,
                recent_path: Some(recent_path),
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_load_monolithic(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Load Monolithic blob_index.dat")
            .add_filter("blob index", &["dat"])
            .pick_file()
        else {
            return;
        };
        self.begin_load_monolithic_path(path, ctx);
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(super) fn begin_load_monolithic_path(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.open_kit_for(&path) {
            self.status = format!("Switched to {}", path.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.active_kit_id();
        let names = self.default_names.clone();
        self.status = format!("Opening {}", path.display());
        let recent_path = clean_recent_path(path.clone());
        thread::spawn(move || {
            let result = load_monolithic_blob_index(path, &names).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::SourceLoaded {
                kit,
                result,
                recent_path: Some(recent_path),
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_load_iostore_container(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open Halo: Campaign Evolved container (.utoc)")
            .add_filter("IoStore TOC", &["utoc"])
            .pick_file()
        else {
            return;
        };
        self.begin_load_iostore_container_path(path, ctx);
    }

    /// The `Paks` directory of the Campaign Evolved install this session is
    /// working with — an already-mounted container set's own root, else the
    /// install configured in Settings. `None` when neither is known, which is
    /// the only case where a container has to be mounted on its own.
    fn campaign_evolved_pak_root(&self) -> Option<PathBuf> {
        let mounted = self.kits.iter().find_map(|kit| {
            match kit.source.as_ref().map(|source| &source.source) {
                Some(TagSource::IoStoreContainerSet { root, .. }) => Some(root.clone()),
                _ => None,
            }
        });
        mounted.or_else(|| {
            let configured = self.editing_kit_paths.get("haloce_evolved")?;
            crate::source::find_paks_dir(configured)
        })
    }

    /// Mounts a single IoStore container (`.utoc`) off the UI thread; completion
    /// is reported through `WorkerMessage::SourceLoaded` like the other loaders.
    pub(super) fn begin_load_iostore_container_path(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.open_kit_for(&path) {
            self.status = format!("Switched to {}", path.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.active_kit_id();
        let names = self.default_names.clone();
        let definitions_root = locate_definitions_root();
        // Mount the container against the install's `Paks` directory. A mod
        // installed in `Paks/~mods` carries no directory index of its own, and
        // only the base containers it overrides can name its chunks.
        let pak_root = self.campaign_evolved_pak_root();
        self.status = format!("Mounting {}", path.display());
        let recent_path = clean_recent_path(path.clone());
        thread::spawn(move || {
            let result = load_iostore_container(path, pak_root, &names, &definitions_root)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::SourceLoaded {
                kit,
                result,
                recent_path: Some(recent_path),
            });
            ctx.request_repaint();
        });
    }

    /// Mounts every container in a `Paks` directory as one merged set.
    /// Mount every container in `paks_dir`. `requested` is the folder the user
    /// actually picked — usually the game's install root, with `paks_dir`
    /// discovered inside it — and is what the kit is remembered and matched by,
    /// so reopening the install switches to it instead of adding a second kit.
    pub(super) fn begin_load_iostore_container_set_path(
        &mut self,
        paks_dir: PathBuf,
        requested: PathBuf,
        ctx: egui::Context,
    ) {
        if self.open_kit_for(&requested) {
            self.status = format!("Switched to {}", requested.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.active_kit_id();
        let names = self.default_names.clone();
        let definitions_root = locate_definitions_root();
        self.status = format!("Mounting containers in {}", paks_dir.display());
        let recent_path = clean_recent_path(requested);
        thread::spawn(move || {
            let result = load_iostore_container_set(paks_dir, &names, &definitions_root)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::SourceLoaded {
                kit,
                result,
                recent_path: Some(recent_path),
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn load_recent_folder(&mut self, path: PathBuf, ctx: egui::Context) {
        if !path.exists() {
            self.status = format!("Folder not found: {}", path.display());
            self.remove_recent_folder(&path);
            return;
        }
        if path.is_dir() {
            self.begin_load_folder_path(path, ctx);
        } else {
            self.begin_load_monolithic_path(path, ctx);
        }
    }

    pub(super) fn remember_recent_folder(&mut self, path: PathBuf) {
        let path = clean_recent_path(path);
        self.recent_folders
            .retain(|existing| !same_recent_path(existing, &path));
        self.recent_folders.insert(0, path);
        self.recent_folders.truncate(MAX_RECENT_FOLDERS);
    }

    pub(super) fn remove_recent_folder(&mut self, path: &Path) {
        self.recent_folders
            .retain(|existing| !same_recent_path(existing, path));
    }

    pub(super) fn open_new_tag_dialog(&mut self) {
        let default_game = self
            .source()
            .and_then(|source| source.game.as_deref())
            .unwrap_or("halo3_mcc")
            .to_owned();
        self.new_tag_dialog = NewTagDialog {
            kit: Some(self.active_kit_id()),
            game: default_game,
            rel_path: String::new(),
            output_path: None,
            groups: Vec::new(),
            selected_group: 0,
            error: None,
        };
        self.refresh_new_tag_groups();
        self.new_tag_open = true;
    }

    /// Open the New Tag dialog pre-filled with a container folder (from a
    /// right-clicked folder node), leaving the leaf name for the user to type.
    pub(super) fn open_new_tag_dialog_in_folder(&mut self, folder_rel: Option<String>) {
        self.open_new_tag_dialog();
        if let Some(folder) = folder_rel.filter(|f| !f.is_empty()) {
            // Pre-fill the path field with the folder + a trailing slash.
            self.new_tag_dialog.rel_path = format!("{}/", folder.trim_end_matches('/'));
        }
    }

    pub(super) fn refresh_new_tag_groups(&mut self) {
        match load_new_tag_groups(&self.new_tag_dialog.game) {
            Ok(groups) if groups.is_empty() => {
                self.new_tag_dialog.groups = groups;
                self.new_tag_dialog.selected_group = 0;
                self.new_tag_dialog.error = Some(format!(
                    "No tag schemas found for {}",
                    self.new_tag_dialog.game
                ));
            }
            Ok(groups) => {
                self.new_tag_dialog.groups = groups;
                self.new_tag_dialog.selected_group = self
                    .new_tag_dialog
                    .selected_group
                    .min(self.new_tag_dialog.groups.len() - 1);
                self.new_tag_dialog.rel_path.clear();
                self.new_tag_dialog.output_path = None;
                self.new_tag_dialog.error = None;
            }
            Err(error) => {
                self.new_tag_dialog.groups.clear();
                self.new_tag_dialog.selected_group = 0;
                self.new_tag_dialog.rel_path.clear();
                self.new_tag_dialog.output_path = None;
                self.new_tag_dialog.error = Some(error);
            }
        }
    }

    pub(super) fn choose_new_tag_output_path(&mut self) {
        let Some(root) = self.loaded_tags_root() else {
            self.new_tag_dialog.error =
                Some("Load a loose editing-kit tags folder before creating a tag".to_owned());
            return;
        };
        let Some(group) = self
            .new_tag_dialog
            .groups
            .get(self.new_tag_dialog.selected_group)
            .cloned()
        else {
            self.new_tag_dialog.error = Some("Choose a tag group".to_owned());
            return;
        };

        let mut dialog = rfd::FileDialog::new()
            .set_title(format!("Create New {}", group.name))
            .set_directory(&root)
            .set_file_name(format!("new_tag.{}", group.extension))
            .add_filter(
                format!("{} tag", group.extension),
                &[group.extension.as_str()],
            );
        if let Some(output) = self.new_tag_dialog.output_path.as_ref()
            && let Some(parent) = output.parent()
        {
            dialog = dialog.set_directory(parent);
        }
        let Some(picked) = dialog.save_file() else {
            return;
        };
        match new_tag_output_path_from_dialog(&root, &picked, &group.extension) {
            Ok((output, rel_path)) => {
                self.new_tag_dialog.output_path = Some(output);
                self.new_tag_dialog.rel_path = rel_path;
                self.new_tag_dialog.error = None;
            }
            Err(error) => {
                self.new_tag_dialog.output_path = None;
                self.new_tag_dialog.rel_path.clear();
                self.new_tag_dialog.error = Some(error);
            }
        }
    }

    pub(super) fn create_new_tag(&mut self) {
        // The tag is written into the active kit's source, and nothing below
        // names a workspace, so without this the tag is created in whichever
        // game was focused when Create was pressed rather than the one the
        // dialog was opened for.
        let dialog_kit = self.new_tag_dialog.kit;
        if !dialog_kit.is_some_and(|kit| self.focus_navigation_kit(kit)) {
            self.new_tag_dialog.error =
                Some("The workspace this tag was being created in is closed".to_owned());
            return;
        }
        // Campaign Evolved containers have no loose tags folder to write into —
        // create the tag purely in memory and let Save / Export Mod write it.
        if self.current_source_is_container() {
            self.create_new_container_tag();
            return;
        }
        let Some(root) = self.loaded_tags_root() else {
            self.new_tag_dialog.error =
                Some("Load a loose editing-kit tags folder before creating a tag".to_owned());
            return;
        };
        let Some(group) = self
            .new_tag_dialog
            .groups
            .get(self.new_tag_dialog.selected_group)
            .cloned()
        else {
            self.new_tag_dialog.error = Some("Choose a tag group".to_owned());
            return;
        };
        let Some(output) = self.new_tag_dialog.output_path.clone() else {
            self.new_tag_dialog.error = Some("Choose a tag name and location".to_owned());
            return;
        };
        let output = match new_tag_output_path_from_dialog(&root, &output, &group.extension) {
            Ok((output, rel_path)) => {
                self.new_tag_dialog.rel_path = rel_path;
                output
            }
            Err(error) => {
                self.new_tag_dialog.error = Some(error);
                return;
            }
        };
        if output.exists() {
            self.new_tag_dialog.error = Some(format!("{} already exists", output.display()));
            return;
        }
        let tag = match TagFile::new(&group.schema_path) {
            Ok(mut tag) => {
                if CONVERSION_GAMES.contains(&self.new_tag_dialog.game.as_str())
                    && let Err(error) =
                        apply_editing_kit_mcc_header(&mut tag, &self.new_tag_dialog.game)
                {
                    self.new_tag_dialog.error = Some(error);
                    return;
                }
                tag
            }
            Err(error) => {
                self.new_tag_dialog.error = Some(format!("Could not create tag: {error}"));
                return;
            }
        };
        if let Some(parent) = output.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            self.new_tag_dialog.error =
                Some(format!("Could not create {}: {error}", parent.display()));
            return;
        }
        if let Err(error) = tag.write_atomic(&output) {
            self.new_tag_dialog.error =
                Some(format!("Could not write {}: {error}", output.display()));
            return;
        }

        let display_path = output
            .strip_prefix(&root)
            .unwrap_or(output.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let key = display_path.clone();
        let entry = TagEntry {
            key: key.clone(),
            display_path,
            group_tag: group.group_tag,
            group_name: Some(group.name.clone()),
            location: TagEntryLocation::LooseFile(output.clone()),
        };
        self.register_created_tag(entry, tag);
        self.new_tag_open = false;
        self.status = format!("Created {}", output.display());
    }

    /// Create a brand-new Campaign Evolved tag in memory (no pak write). The tag
    /// is a defaults-initialized `TagFile::new` from the group schema, registered
    /// dirty at the dialog's container-relative path; Save / Export Mod then write
    /// it via `write_new_tag_container`.
    fn create_new_container_tag(&mut self) {
        let Some(group) = self
            .new_tag_dialog
            .groups
            .get(self.new_tag_dialog.selected_group)
            .cloned()
        else {
            self.new_tag_dialog.error = Some("Choose a tag group".to_owned());
            return;
        };
        let rel = normalize_container_tag_rel(&self.new_tag_dialog.rel_path);
        if rel.is_empty() {
            self.new_tag_dialog.error = Some("Enter a tag path (e.g. objects/foo/bar)".to_owned());
            return;
        }
        let tag = match TagFile::new(&group.schema_path) {
            Ok(tag) => tag,
            Err(error) => {
                self.new_tag_dialog.error = Some(format!("Could not create tag: {error}"));
                return;
            }
        };
        match self.add_new_container_tag(&rel, group.group_tag, &group.name, &group.extension, tag)
        {
            Ok(()) => {
                self.new_tag_open = false;
                self.status = format!("Created {rel}.{} (unsaved)", group.extension);
            }
            Err(error) => self.new_tag_dialog.error = Some(error),
        }
    }

    /// Register a brand-new in-memory container tag (shared by New Tag and
    /// Import-of-a-new-path). `logical` is the normalized container-relative path
    /// (no extension). Fails if the path is empty, no same-group template tag
    /// exists to seed the package, or a new tag already occupies that path.
    pub(super) fn add_new_container_tag(
        &mut self,
        logical: &str,
        group_tag: u32,
        group_name: &str,
        extension: &str,
        tag: TagFile,
    ) -> Result<(), String> {
        if logical.is_empty() {
            return Err("Enter a tag path (e.g. objects/foo/bar)".to_owned());
        }
        // A brand-new package needs an existing same-group tag's `.uasset` as its
        // structural template (see `write_new_tag_container`).
        let Some((template_container, template_rel)) = self.find_container_template(group_tag)
        else {
            return Err(format!(
                "No existing {group_name} tag in the mounted paks to use as a template"
            ));
        };
        let package = new_container_package(logical, group_name);
        let key = new_container_key(&package);
        if self.kits[self.active].parsed_tags.contains_key(&key)
            || self.source().is_some_and(|s| {
                s.entries
                    .iter()
                    .chain(s.all_entries.iter())
                    .any(|e| e.key == key)
            })
        {
            return Err(format!("A new tag already exists at {logical}"));
        }
        let entry = TagEntry {
            key,
            display_path: format!("{logical}.{extension}"),
            group_tag,
            group_name: Some(group_name.to_owned()),
            location: TagEntryLocation::NewContainer {
                template_container,
                template_rel,
                package,
                group_tag,
            },
        };
        self.register_in_memory_tag(entry, tag);
        Ok(())
    }

    /// Rename/move (`duplicate == false`) or copy (`duplicate == true`) a
    /// brand-new container tag to `new_rel`. Nothing is written: a new tag lives
    /// only in its document until Save/Export Mod, so this rewrites the entry
    /// (and re-homes the document under the new key) in memory. Returns the
    /// status line to show.
    fn apply_new_container_rename(
        &mut self,
        key: &str,
        new_rel: &str,
        duplicate: bool,
    ) -> Result<String, String> {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return Err("Tag is no longer in the source".to_owned());
        };
        let TagEntryLocation::NewContainer {
            template_container,
            template_rel,
            group_tag,
            ..
        } = &entry.location
        else {
            return Err("Not a new Campaign Evolved tag".to_owned());
        };
        let (template_container, template_rel, group_tag) =
            (*template_container, template_rel.clone(), *group_tag);
        if new_rel.is_empty() {
            return Err("Enter a tag path (e.g. objects/foo/bar)".to_owned());
        }
        let group_name = entry
            .group_name
            .clone()
            .unwrap_or_else(|| format_group_tag(entry.group_tag));
        let extension = entry
            .display_path
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_owned())
            .unwrap_or_else(|| group_name.clone());

        if duplicate {
            // `TagFile` is not `Clone`; a round-trip through its own bytes is
            // how a document is copied, and it is exactly what Save would have
            // written anyway.
            let bytes = self.kits[self.active]
                .parsed_tags
                .get(key)
                .ok_or("Load the tag before copying it")?
                .tag
                .write_to_bytes()
                .map_err(|error| format!("Could not serialize the tag: {error}"))?;
            let copy = TagFile::read_from_bytes(&bytes)
                .map_err(|error| format!("Could not re-read the copied tag: {error}"))?;
            self.add_new_container_tag(new_rel, group_tag, &group_name, &extension, copy)?;
            return Ok(format!("Copied to {new_rel}.{extension} (unsaved)"));
        }

        let package = new_container_package(new_rel, &group_name);
        let new_key = new_container_key(&package);
        if new_key == key {
            return Ok(format!("{} is already at that path", entry.display_path));
        }
        if self.kits[self.active].parsed_tags.contains_key(&new_key)
            || self.source().is_some_and(|source| {
                source
                    .entries
                    .iter()
                    .chain(source.all_entries.iter())
                    .any(|existing| existing.key == new_key)
            })
        {
            return Err(format!("A tag already exists at {new_rel}"));
        }
        let Some(document) = self.kits[self.active].parsed_tags.remove(key) else {
            return Err("Load the tag before renaming it".to_owned());
        };
        // The project stashes overlays under the package path, so the old
        // identity has to go — otherwise the checkpoint keeps a copy of the tag
        // at its previous path and restores it as a second tag next session.
        let kit = self.active;
        self.forget_campaign_overlay(kit, key);
        self.forget_new_container_entry(kit, key);
        let old_display = entry.display_path.clone();
        self.register_in_memory_tag(
            TagEntry {
                key: new_key,
                display_path: format!("{new_rel}.{extension}"),
                group_tag: entry.group_tag,
                group_name: Some(group_name),
                location: TagEntryLocation::NewContainer {
                    template_container,
                    template_rel,
                    package,
                    group_tag,
                },
            },
            document.tag,
        );
        Ok(format!(
            "Renamed {old_display} → {new_rel}.{extension} (unsaved)"
        ))
    }

    /// Open the "Import tag" dialog: pick a self-describing MCC/Reach tag file,
    /// parse it, validate its schema against our JSON, and seed the dialog.
    /// `folder_rel` pre-fills the destination folder (from a right-clicked node).
    pub(super) fn begin_import_tag(&mut self, folder_rel: Option<String>) {
        if !self.current_source_is_container() {
            self.status = "Import tag is only for Campaign Evolved containers".to_owned();
            return;
        }
        let Some(picked) = rfd::FileDialog::new().set_title("Import Tag").pick_file() else {
            return;
        };
        let bytes = match fs::read(&picked) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.status = format!("Could not read {}: {error}", picked.display());
                return;
            }
        };
        let tag = match TagFile::read_from_bytes(&bytes) {
            Ok(tag) => tag,
            Err(error) => {
                self.status = format!("Not a valid MCC tag file: {error}");
                return;
            }
        };
        if tag.classic_engine().is_some() || tag.endian != Endian::Le {
            self.status = "Only little-endian MCC tags can be imported".to_owned();
            return;
        }
        let group_tag = tag.header.group_tag;
        let group_name = self
            .source()
            .and_then(|s| s.names.name_for(group_tag))
            .map(str::to_owned)
            .or_else(|| group_tag_to_extension(group_tag).map(str::to_owned))
            .unwrap_or_else(|| format_group_tag(group_tag));
        let extension = group_tag_to_extension(group_tag)
            .unwrap_or(group_name.as_str())
            .to_owned();
        let comparison = self.compare_import_against_json(group_tag, &tag);
        let name = picked
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported")
            .to_owned();
        self.import_tag_dialog = Some(ImportTagDialog {
            kit: self.active_kit_id(),
            source_path: picked,
            folder_rel: folder_rel.unwrap_or_default(),
            name,
            group_tag,
            group_name,
            extension,
            tag: Some(tag),
            comparison,
            import_anyway: false,
            error: None,
        });
    }

    /// Compare an imported tag's embedded layout against our shipped JSON
    /// definition for its group. `None` if we ship no schema for the group.
    fn compare_import_against_json(
        &self,
        group_tag: u32,
        imported: &TagFile,
    ) -> Option<blam_tags::LayoutComparison> {
        let game = self
            .source()
            .and_then(|s| s.game.clone())
            .unwrap_or_else(|| "haloce_evolved".to_owned());
        let group = load_new_tag_groups(&game)
            .ok()?
            .into_iter()
            .find(|g| g.group_tag == group_tag)?;
        let expected = TagFile::new(&group.schema_path).ok()?;
        Some(blam_tags::compare_root_layout(&expected, imported))
    }

    /// Apply the pending import: validate the schema gate, resolve the target
    /// path against existing tags, and either overwrite an existing tag's
    /// document (dirty, with a discard prompt if it has unsaved edits) or add a
    /// brand-new container tag.
    pub(super) fn confirm_import_tag(&mut self) {
        // The import is resolved and registered against the active kit's
        // source, so return to the workspace the dialog was opened for.
        let Some(kit) = self.import_tag_dialog.as_ref().map(|dialog| dialog.kit) else {
            return;
        };
        if !self.focus_navigation_kit(kit) {
            self.import_tag_dialog = None;
            self.status = "The workspace this import came from is closed".to_owned();
            return;
        }
        let Some(dialog) = self.import_tag_dialog.as_mut() else {
            return;
        };
        // Schema gate (policy calibrated by the CE drift sweep): a group /
        // version / root-size mismatch is a hard block; benign field-metadata
        // drift only warns and needs the "Import anyway" override.
        if let Some(cmp) = &dialog.comparison {
            if !cmp.group_match || !cmp.version_match || !cmp.root_size_match {
                dialog.error = Some(
                    "Schema is incompatible (group, version, or size differs) — this tag \
                     doesn't match the base game's definition."
                        .to_owned(),
                );
                return;
            }
            if cmp.severity != blam_tags::LayoutSeverity::Match && !dialog.import_anyway {
                dialog.error = Some(
                    "Schema differs in field metadata. Tick \"Import anyway\" to proceed."
                        .to_owned(),
                );
                return;
            }
        }
        let folder = normalize_container_tag_rel(&dialog.folder_rel);
        let leaf = normalize_container_tag_rel(&dialog.name);
        if leaf.is_empty() {
            dialog.error = Some("Enter a tag name".to_owned());
            return;
        }
        let logical = if folder.is_empty() {
            leaf
        } else {
            format!("{folder}/{leaf}")
        };
        let group_tag = dialog.group_tag;
        let group_name = dialog.group_name.clone();
        let extension = dialog.extension.clone();
        let Some(tag) = dialog.tag.take() else {
            dialog.error = Some("No tag to import".to_owned());
            return;
        };

        // Does a base-game tag already exist at this path+group?
        let existing = self.source().and_then(|s| match &s.source {
            TagSource::IoStoreContainerSet { index, .. } => index
                .lookup(group_tag, &logical)
                .map(|(c, r)| (c, r.to_owned())),
            _ => None,
        });
        if let Some((container, rel_path)) = existing {
            let key = self.source().and_then(|s| {
                s.entries
                    .iter()
                    .find(|e| {
                        matches!(&e.location, TagEntryLocation::Container { container: c, rel_path: rp }
                            if *c == container && rp == &rel_path)
                    })
                    .map(|e| e.key.clone())
            });
            let Some(key) = key else {
                self.import_tag_dialog = None;
                self.status = "Could not resolve the existing tag to overwrite".to_owned();
                return;
            };
            // Already open with unsaved edits → confirm discard first.
            if self.kits[self.active]
                .parsed_tags
                .get(&key)
                .map(|d| d.dirty.is_set())
                .unwrap_or(false)
            {
                self.import_discard_confirm = Some(PendingImport {
                    kit: self.active_kit_id(),
                    tag,
                    target_key: key,
                });
                self.import_tag_dialog = None;
                return;
            }
            self.apply_import_over_existing(&key, tag);
            self.import_tag_dialog = None;
        } else {
            match self.add_new_container_tag(&logical, group_tag, &group_name, &extension, tag) {
                Ok(()) => {
                    self.import_tag_dialog = None;
                    self.status = format!("Imported {logical}.{extension} (unsaved)");
                }
                Err(error) => {
                    if let Some(dialog) = self.import_tag_dialog.as_mut() {
                        dialog.error = Some(error);
                    }
                }
            }
        }
    }

    /// Replace an existing container tag's document with imported bytes, marked
    /// dirty (no pak write). Opens/selects the tab.
    fn apply_import_over_existing(&mut self, key: &str, tag: TagFile) {
        self.kits[self.active].open_tag_pane(key);
        self.kits[self.active].selected_key = Some(key.to_owned());
        self.kits[self.active]
            .parsed_tags
            .insert(key.to_owned(), TagDocument::modified(tag));
        let label = self.tag_path_label(key);
        self.status = format!("Imported over {label} (unsaved)");
    }

    /// If an import at `folder_rel`/`name` (group `group_tag`) would land on an
    /// existing base-game tag, return that tag's logical path; else `None` (a new
    /// tag). Used by the Import dialog's overwrite-vs-new banner.
    pub(super) fn import_overwrite_target(
        &self,
        folder_rel: &str,
        name: &str,
        group_tag: u32,
    ) -> Option<String> {
        let folder = normalize_container_tag_rel(folder_rel);
        let leaf = normalize_container_tag_rel(name);
        if leaf.is_empty() {
            return None;
        }
        let logical = if folder.is_empty() {
            leaf
        } else {
            format!("{folder}/{leaf}")
        };
        match &self.source()?.source {
            TagSource::IoStoreContainerSet { index, .. } => {
                index.lookup(group_tag, &logical).map(|_| logical)
            }
            _ => None,
        }
    }

    /// Resolve the pending "discard unsaved edits?" import confirmation.
    pub(super) fn apply_import_discard(&mut self) {
        let Some(pending) = self.import_discard_confirm.take() else {
            return;
        };
        if !self.focus_navigation_kit(pending.kit) {
            self.status = "The workspace this import came from is closed".to_owned();
            return;
        }
        self.apply_import_over_existing(&pending.target_key, pending.tag);
    }

    /// Find an existing container tag of `group_tag` and return its owning
    /// container index plus its `.uasset` container path — the package template
    /// for a new tag of the same group.
    pub(super) fn find_container_template(&self, group_tag: u32) -> Option<(usize, String)> {
        self.find_container_template_in(self.active, group_tag)
    }

    /// A specific kit's template. Project recovery names its kit: the container
    /// a stashed new tag is modelled on has to come from the source that tag
    /// belongs to, not from whichever kit happens to be focused.
    pub(super) fn find_container_template_in(
        &self,
        kit: usize,
        group_tag: u32,
    ) -> Option<(usize, String)> {
        let source = self.kits.get(kit)?.source.as_ref()?;
        source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .find_map(|entry| match &entry.location {
                TagEntryLocation::Container {
                    container,
                    rel_path,
                } if entry.group_tag == group_tag => {
                    let uasset = rel_path
                        .strip_suffix(".ubulk")
                        .map(|s| format!("{s}.uasset"))?;
                    Some((*container, uasset))
                }
                _ => None,
            })
    }

    /// Register an in-memory (unsaved) container tag: insert it into the browser
    /// entries, rebuild the folder + group trees so it shows up, open it in a
    /// **dirty** tab, and select it. Used by New Tag and Import for CE.
    pub(super) fn register_in_memory_tag(&mut self, entry: TagEntry, tag: TagFile) {
        let key = entry.key.clone();
        self.stash_in_memory_tag(entry, tag);
        self.kits[self.active].open_tag_pane(&key);
        self.kits[self.active].selected_key = Some(key);
    }

    /// The same registration without opening or selecting the tag.
    ///
    /// Recovering a stashed new tag at startup has to put it back in the browser
    /// without deciding what the user is looking at: adopting two of them would
    /// otherwise open two tabs and steal the selection on every launch.
    pub(super) fn stash_in_memory_tag(&mut self, entry: TagEntry, tag: TagFile) {
        let key = entry.key.clone();
        if let Some(source) = self.source_mut() {
            source.entries.retain(|existing| existing.key != key);
            source.entries.push(entry.clone());
            // Container sources keep their full set in `entries` (all_entries is
            // empty), so rebuild both trees from it.
            let tree = crate::source::build_tree(&source.entries);
            let group_tree = crate::source::build_group_tree(&source.entries);
            source.tree = tree;
            source.group_tree = group_tree;
        }
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        // Index what the new tag points at. Nothing else can: the reverse-
        // dependency builder reads tags from their source, and this one has no
        // source — so without this the tag is invisible to reference queries in
        // both directions, and to the tag tree's children view.
        let dependencies = {
            let mut refs = Vec::new();
            collect_tag_dependency_refs(tag.root(), &mut refs);
            refs
        };
        if let Some(index) = self
            .source_mut()
            .and_then(|source| source.reverse_dependencies.as_mut())
        {
            index.set_tag_dependencies(key.clone(), dependencies);
        }
        self.kits[self.active]
            .parsed_tags
            .insert(key, TagDocument::modified(tag));
    }

    pub(super) fn loaded_tags_root(&self) -> Option<PathBuf> {
        self.loaded_tags_root_for(self.active)
    }

    /// A specific kit's loose tags root. Background work has to name its kit:
    /// the one it started in may no longer be the focused one when it lands.
    pub(super) fn loaded_tags_root_for(&self, kit: usize) -> Option<PathBuf> {
        let TagSource::LooseFolder { root, .. } = &self.kits.get(kit)?.source.as_ref()?.source
        else {
            return None;
        };
        Some(root.clone())
    }

    fn favorite_kit_index(&self, root: &Path) -> Option<usize> {
        self.editing_kit_favorites
            .iter()
            .position(|kit| same_recent_path(&kit.tags_root, root))
    }

    /// Rebuild `kit`'s resolved favorite entries from the saved paths for its
    /// tags root. Kit-scoped because a finished background refactor refreshes
    /// the workspace it belonged to, which need not be the focused one.
    fn refresh_favorite_entries_for(&mut self, kit: usize) {
        self.kits[kit].active_favorite_entries.clear();
        let Some(root) = self.loaded_tags_root_for(kit) else {
            return;
        };
        let Some(index) = self.favorite_kit_index(&root) else {
            return;
        };
        let names = self.kits[kit]
            .source
            .as_ref()
            .map(|source| source.names.clone())
            .unwrap_or_else(|| self.kits[kit].names.clone());
        let saved_paths = self.editing_kit_favorites[index].tags.clone();
        let mut missing = Vec::new();
        for relative_path in saved_paths {
            let path = root.join(&relative_path);
            if !path.is_file() {
                missing.push(relative_path);
                continue;
            }
            if let Ok(Some(entry)) = loose_file_entry(&root, &path, &names) {
                self.kits[kit].active_favorite_entries.push(entry);
            }
        }
        if !missing.is_empty() {
            self.editing_kit_favorites[index].tags.retain(|path| {
                !missing
                    .iter()
                    .any(|missing| same_recent_path(missing, path))
            });
            if self.editing_kit_favorites[index].tags.is_empty() {
                self.editing_kit_favorites.remove(index);
            }
        }
    }

    fn toggle_favorite(&mut self, key: &str) {
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Favorites are only available for editing-kit tag folders".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer available".to_owned();
            return;
        };
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            self.status = "Only loose tags can be favorited".to_owned();
            return;
        };
        let Some(relative_path) = path
            .strip_prefix(&root)
            .ok()
            .map(Path::to_path_buf)
            .and_then(clean_favorite_relative_path)
        else {
            self.status = "Could not resolve tag relative to the loaded tags folder".to_owned();
            return;
        };
        let index = self.favorite_kit_index(&root).unwrap_or_else(|| {
            self.editing_kit_favorites.push(EditingKitFavorites {
                tags_root: clean_recent_path(root.clone()),
                tags: Vec::new(),
            });
            self.editing_kit_favorites.len() - 1
        });
        let kit = &mut self.editing_kit_favorites[index];
        if let Some(position) = kit
            .tags
            .iter()
            .position(|current| same_recent_path(current, &relative_path))
        {
            kit.tags.remove(position);
            self.kits[self.active]
                .active_favorite_entries
                .retain(|favorite| favorite.key != entry.key);
            if kit.tags.is_empty() {
                self.editing_kit_favorites.remove(index);
            }
            self.status = format!("Removed {} from Favorites", entry.display_path);
        } else {
            kit.tags.push(relative_path);
            self.kits[self.active]
                .active_favorite_entries
                .push(entry.clone());
            self.status = format!("Added {} to Favorites", entry.display_path);
        }
    }

    /// Rewrite `kit`'s favorites after a move or rename changed its tag paths.
    ///
    /// Takes the kit rather than reading the active one: this runs from a
    /// finished background refactor, which may well land while the user is in
    /// another workspace — and then it resolved the wrong root and remapped the
    /// wrong workspace's favorites with this one's rename map.
    fn remap_favorites_for_kit(&mut self, kit: usize, old_to_new_keys: &HashMap<String, String>) {
        let Some(root) = self.loaded_tags_root_for(kit) else {
            return;
        };
        let Some(index) = self.favorite_kit_index(&root) else {
            return;
        };
        remap_favorite_paths(
            &root,
            &mut self.editing_kit_favorites[index].tags,
            old_to_new_keys,
        );
        let mut unique: Vec<PathBuf> = Vec::new();
        self.editing_kit_favorites[index].tags.retain(|path| {
            if unique
                .iter()
                .any(|existing| same_recent_path(existing, path))
            {
                false
            } else {
                unique.push(path.clone());
                true
            }
        });
        self.refresh_favorite_entries_for(kit);
    }

    pub(super) fn open_dropped_files(&mut self, paths: Vec<PathBuf>, ctx: egui::Context) {
        if paths.is_empty() {
            return;
        }

        let count = paths.len();
        for path in paths {
            match self.open_dropped_file(path, ctx.clone()) {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        }

        self.status = if count == 1 {
            "Dropped file is not a supported tag".to_owned()
        } else {
            "No supported tag files were dropped".to_owned()
        };
    }

    fn open_dropped_file(&mut self, path: PathBuf, ctx: egui::Context) -> Result<bool, String> {
        if !path.is_file() {
            return Ok(false);
        }

        let Some(source) = self.source() else {
            return Err("Load an editing-kit tags folder before dropping tag files".to_owned());
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return Err("Drop-to-open requires a loaded loose tags folder".to_owned());
        };

        let root = fs::canonicalize(root)
            .map_err(|error| format!("Could not resolve loaded tags folder: {error}"))?;
        let path = fs::canonicalize(&path)
            .map_err(|error| format!("Could not resolve dropped file: {error}"))?;
        if !path.starts_with(&root) {
            return Err(format!(
                "Dropped tag must be inside the loaded tags folder: {}",
                root.display()
            ));
        }

        if let Some(key) = self.key_for_loose_path(&path) {
            self.select_entry(key, ctx);
            return Ok(true);
        }

        let Some(entry) = loose_file_entry(&root, &path, &source.names)
            .map_err(|error| format!("Could not inspect dropped tag: {error:#}"))?
        else {
            return Ok(false);
        };

        let key = entry.key.clone();
        if let Some(source) = self.source_mut() {
            source.entries.retain(|existing| existing.key != key);
            source.entries.push(entry.clone());

            if !source.all_entries.is_empty() {
                source.all_entries.retain(|existing| existing.key != key);
                source.all_entries.push(entry.clone());
                source
                    .all_entries
                    .sort_by(|a, b| a.display_path.cmp(&b.display_path));
                source.group_tree = crate::source::build_group_tree(&source.all_entries);
                if let (Some(game), TagSource::LooseFolder { root, .. }) =
                    (source.game.as_deref(), &source.source)
                {
                    let _ = crate::source::save_entry_index(game, root, &source.all_entries);
                }
            }
        }
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        self.select_entry(key, ctx);
        Ok(true)
    }

    fn key_for_loose_path(&self, path: &Path) -> Option<String> {
        let source = self.source()?;
        source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .find_map(|entry| {
                let TagEntryLocation::LooseFile(existing) = &entry.location else {
                    return None;
                };
                if existing == path || fs::canonicalize(existing).ok().as_deref() == Some(path) {
                    Some(entry.key.clone())
                } else {
                    None
                }
            })
    }

    pub(super) fn register_created_tag(&mut self, entry: TagEntry, tag: TagFile) {
        let key = entry.key.clone();
        if let Some(source) = self.source_mut() {
            source.entries.retain(|existing| existing.key != key);
            source.entries.push(entry.clone());
            source.all_entries.retain(|existing| existing.key != key);
            source.all_entries.push(entry.clone());
            if let TagSource::LooseFolder { root, .. } = &source.source {
                if let Ok(tree) = crate::source::build_folder_directory_tree(root) {
                    source.tree = tree;
                }
                source.group_tree = crate::source::build_group_tree(&source.all_entries);
                if let Some(game) = source.game.as_deref() {
                    let _ = crate::source::save_entry_index(game, root, &source.all_entries);
                }
            }
        }
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        self.kits[self.active]
            .parsed_tags
            .insert(key.clone(), TagDocument::clean(tag));
        self.kits[self.active].open_tag_pane(&key);
        self.kits[self.active].selected_key = Some(key.clone());
    }

    fn register_saved_copy_if_in_loaded_folder(&mut self, path: &Path) -> Result<bool, String> {
        let Some(source) = self.source_mut() else {
            return Ok(false);
        };
        let registered = register_saved_copy_in_loaded_source(source, path)?;
        if registered {
            self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        }
        Ok(registered)
    }

    /// Trigger a background full recursive scan of a LooseFolder source so
    /// that Groups mode and search work without needing to expand every tree
    /// node first. No-op if already scanning or source is not a LooseFolder.
    pub(super) fn begin_scan_all_entries(&mut self, ctx: egui::Context) {
        self.begin_scan_all_entries_with_label(ctx, "Indexing tags...");
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(super) fn begin_scan_all_entries_with_label(
        &mut self,
        ctx: egui::Context,
        label: impl Into<String>,
    ) {
        if self.kits[self.active].scanning_entries {
            return;
        }
        let Some(source) = self.source() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return; // monolithic/single-file already have all entries
        };
        let root = root.clone();
        let names = source.names.clone();
        let tx = self.tx.clone();
        self.refreshing_entry_index = false;
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        self.kits[self.active].field_index.invalidate();
        let stamp = self.kit_stamp();
        let label = label.into();
        self.kits[self.active].scanning_entries = true;
        self.show_entry_index_wait_notice = true;
        self.entry_index_progress = Some(EntryIndexProgressState {
            label: label.clone(),
            processed: 0,
            total: 0,
            matched: 0,
        });
        self.status = label;
        thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let result = scan_folder_subtree_entries_with_progress(
                &root,
                std::path::Path::new(""),
                &names,
                move |progress| {
                    let _ = progress_tx.send(WorkerMessage::EntryIndexScanProgress {
                        stamp,
                        processed: progress.processed,
                        total: progress.total,
                        matched: progress.matched,
                    });
                    progress_ctx.request_repaint();
                },
            )
            .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::AllEntriesScanned { stamp, result });
            ctx.request_repaint();
        });
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(super) fn maybe_refresh_entry_index(&mut self, ctx: egui::Context) {
        if self.kits[self.active].scanning_entries
            || self.refreshing_entry_index
            || self.building_reverse_dependencies
        {
            return;
        }
        let now = ctx.input(|input| input.time);
        if now < self.next_entry_index_refresh_at {
            return;
        }
        let should_refresh = self.source().is_some_and(|source| {
            source.game.is_some()
                && !source.all_entries.is_empty()
                && matches!(source.source, TagSource::LooseFolder { .. })
        });
        if should_refresh {
            self.begin_refresh_entry_index(ctx);
        } else {
            self.schedule_next_entry_index_refresh(&ctx);
        }
    }

    pub(super) fn begin_refresh_entry_index(&mut self, ctx: egui::Context) {
        if self.kits[self.active].scanning_entries || self.refreshing_entry_index {
            return;
        }
        let Some(source) = self.source() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return;
        };
        let Some(game) = source.game.clone() else {
            return;
        };
        let root = root.clone();
        let names = source.names.clone();
        let tx = self.tx.clone();
        let stamp = self.kit_stamp();
        self.refreshing_entry_index = true;
        thread::spawn(move || {
            let result =
                crate::source::refresh_entry_index(&game, &root, &names).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::EntryIndexRefreshed { stamp, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn refresh_tag_browser(&mut self, ctx: egui::Context) {
        let reset_result = self.source_mut().and_then(|source| {
            let TagSource::LooseFolder { root, .. } = &source.source else {
                return None;
            };
            Some(reset_lazy_folder_browser(
                root,
                &mut source.tree,
                &mut source.entries,
            ))
        });
        match reset_result {
            Some(Ok(())) => {
                self.kits[self.active].generation =
                    self.kits[self.active].generation.wrapping_add(1);
                self.status = "Tag browser refreshed; checking index...".to_owned();
                self.begin_refresh_entry_index(ctx);
            }
            Some(Err(error)) => self.status = format!("Tag browser refresh failed: {error}"),
            None => self.status = "No loose tag folder is loaded".to_owned(),
        }
    }

    fn schedule_next_entry_index_refresh(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        self.next_entry_index_refresh_at = now + ENTRY_INDEX_REFRESH_INTERVAL_SECS;
    }

    fn apply_entry_index_refresh(
        &mut self,
        kit_index: usize,
        refresh: EntryIndexRefresh,
        ctx: egui::Context,
    ) {
        let kit = &mut self.kits[kit_index];
        let Some(source) = kit.source.as_mut() else {
            return;
        };
        let n = refresh.entries.len();
        source.group_tree = crate::source::build_group_tree(&refresh.entries);
        source.all_entries = refresh.entries;
        let browser_refresh_error = if let TagSource::LooseFolder { root, .. } = &source.source {
            reset_lazy_folder_browser(root, &mut source.tree, &mut source.entries).err()
        } else {
            None
        };
        source.reverse_dependencies = None;
        kit.field_index.invalidate();
        kit.generation = kit.generation.wrapping_add(1);
        self.status = browser_refresh_error.map_or_else(
            || {
                format!(
                    "Index updated: {n} tags ({} added, {} changed, {} removed)",
                    refresh.added, refresh.updated, refresh.removed
                )
            },
            |error| format!("Index updated, but browser refresh failed: {error}"),
        );

        if let (Some(game), TagSource::LooseFolder { root, .. }) =
            (source.game.clone(), &source.source)
        {
            let root = root.clone();
            let entries = source.all_entries.clone();
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            let stamp = KitStamp {
                kit: kit.id,
                generation: kit.generation,
            };
            let path = crate::source::index_db_path();
            thread::spawn(move || {
                let result = crate::source::save_entry_index(&game, &root, &entries)
                    .map_err(|error| error.to_string());
                let _ = tx.send(WorkerMessage::EntryIndexSaved {
                    stamp,
                    path,
                    result,
                });
                ctx.request_repaint();
            });
        }
    }

    /// Starts the non-blocking release lookup and returns its result through `WorkerMessage`.
    ///
    /// A `silent` check announces neither its start nor an uneventful result —
    /// that is the automatic startup check, which must not spend the status
    /// line on "up to date" or on a failure the user never asked about.
    pub(super) fn begin_check_for_updates(&mut self, ctx: egui::Context, silent: bool) {
        if !silent {
            self.status = "Checking for updates...".to_owned();
        }
        let channel = self.update_channel;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = fetch_latest_release(channel);
            let _ = tx.send(WorkerMessage::UpdateCheckFinished { silent, result });
            ctx.request_repaint();
        });
    }

    /// Whether the automatic startup check should run.
    pub(super) fn should_check_updates_on_startup(&self) -> bool {
        self.check_updates_on_startup
    }

    pub(super) fn begin_terminal_command(&mut self, ctx: egui::Context) {
        let command = self.terminal.input.trim().to_owned();
        if command.is_empty() {
            return;
        }
        self.submit_terminal_command(command, ctx);
    }

    pub(super) fn submit_terminal_command(&mut self, command: String, ctx: egui::Context) {
        if self.terminal.history.last() != Some(&command) {
            self.terminal.history.push(command.clone());
        }
        self.terminal.history_cursor = None;
        self.terminal.input.clear();
        self.terminal.refocus_input = true;
        self.spawn_terminal_command(command, ctx);
    }

    pub(super) fn recall_terminal_history(&mut self, delta: i32) {
        let len = self.terminal.history.len();
        if len == 0 {
            return;
        }

        let next = match self.terminal.history_cursor {
            Some(index) => index as i32 + delta,
            None if delta < 0 => len as i32 - 1,
            None => return,
        };

        if next < 0 {
            self.terminal.history_cursor = Some(0);
            self.terminal.input = self.terminal.history[0].clone();
        } else if next >= len as i32 {
            self.terminal.history_cursor = None;
            self.terminal.input.clear();
        } else {
            let next = next as usize;
            self.terminal.history_cursor = Some(next);
            self.terminal.input = self.terminal.history[next].clone();
        }
    }

    /// Run `command` in the editing-kit root, streaming output to the terminal
    /// panel. Shared by the terminal input and the geometry Import button.
    /// Starts the configured command without blocking frame rendering.
    /// Output and completion return through ordered worker messages for the active run id.
    pub(super) fn spawn_terminal_command(&mut self, command: String, ctx: egui::Context) {
        if self.terminal.running {
            self.status = "A command is already running".to_owned();
            return;
        }
        let Some(work_dir) = self.kits[self.active].terminal_work_dir.clone() else {
            self.status = "Run requires a loaded editing-kit folder".to_owned();
            return;
        };
        self.kits[self.active].terminal_open = true;
        self.terminal
            .lines
            .push(TerminalLineEntry::new(format!("> {command}")));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
        self.terminal.refocus_input = true;
        self.terminal.running = true;
        let run_id = self.terminal.next_run_id;
        self.terminal.next_run_id = self.terminal.next_run_id.wrapping_add(1).max(1);
        self.terminal.running_id = Some(run_id);
        self.terminal.running_command = Some(command.clone());
        let mut log_file = match create_terminal_log_file(run_id, &command) {
            Ok((path, file)) => {
                self.terminal.last_log_path = Some(path);
                Some(file)
            }
            Err(error) => {
                self.status = format!("Terminal full log unavailable: {error}");
                self.terminal.last_log_path = None;
                None
            }
        };
        let tx = self.tx.clone();
        let child_slot: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
        let stop_requested = Arc::new(AtomicBool::new(false));
        self.terminal.process = Some(TerminalProcess {
            child: Arc::clone(&child_slot),
            stop_requested: Arc::clone(&stop_requested),
        });
        thread::spawn(move || {
            let mut log_error_reported = false;
            #[cfg(target_os = "windows")]
            let mut cmd = {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                let mut c = std::process::Command::new("cmd");
                c.creation_flags(CREATE_NO_WINDOW);
                c.args(["/C", &format!("{command} 2>&1")]);
                c
            };
            #[cfg(not(target_os = "windows"))]
            let mut cmd = {
                #[cfg(unix)]
                use std::os::unix::process::CommandExt;
                let mut c = std::process::Command::new("sh");
                c.args(["-c", &command]);
                #[cfg(unix)]
                c.process_group(0);
                c
            };
            cmd.current_dir(&work_dir)
                .stdout(std::process::Stdio::piped())
                .stdin(std::process::Stdio::null());
            match cmd.spawn() {
                Err(e) => {
                    send_terminal_line(
                        &tx,
                        &ctx,
                        &mut log_file,
                        &mut log_error_reported,
                        format!("[error] {e}"),
                    );
                    let _ = tx.send(WorkerMessage::TerminalDone { run_id });
                    ctx.request_repaint();
                }
                Ok(child) => {
                    let stdout = match child_slot.lock() {
                        Ok(mut slot) => {
                            *slot = Some(child);
                            slot.as_mut().and_then(|child| child.stdout.take())
                        }
                        Err(_) => {
                            send_terminal_line(
                                &tx,
                                &ctx,
                                &mut log_file,
                                &mut log_error_reported,
                                "[error] terminal process lock was poisoned".to_owned(),
                            );
                            let _ = tx.send(WorkerMessage::TerminalDone { run_id });
                            ctx.request_repaint();
                            return;
                        }
                    };
                    if let Some(stdout) = stdout {
                        let _ = stream_terminal_output(
                            stdout,
                            &tx,
                            &ctx,
                            &mut log_file,
                            &mut log_error_reported,
                        );
                    }
                    let exit = match child_slot.lock() {
                        Ok(mut slot) => {
                            if let Some(mut child) = slot.take() {
                                child.wait().ok()
                            } else {
                                None
                            }
                        }
                        Err(_) => {
                            send_terminal_line(
                                &tx,
                                &ctx,
                                &mut log_file,
                                &mut log_error_reported,
                                "[error] terminal process lock was poisoned".to_owned(),
                            );
                            None
                        }
                    };
                    if let Some(code) = exit.and_then(|status| status.code())
                        && !stop_requested.load(Ordering::SeqCst)
                    {
                        send_terminal_line(
                            &tx,
                            &ctx,
                            &mut log_file,
                            &mut log_error_reported,
                            format!("[exit {code}]"),
                        );
                    }
                    let _ = tx.send(WorkerMessage::TerminalDone { run_id });
                    ctx.request_repaint();
                }
            }
        });
    }

    pub(super) fn stop_terminal_command(&mut self) {
        if !self.terminal.running {
            self.status = "No terminal command is running".to_owned();
            return;
        }
        let Some(process) = self.terminal.process.as_ref() else {
            self.status = "No tracked terminal process to stop".to_owned();
            return;
        };

        process.stop_requested.store(true, Ordering::SeqCst);
        let command = self
            .terminal
            .running_command
            .clone()
            .unwrap_or_else(|| "command".to_owned());
        match stop_terminal_process(process) {
            Ok(TerminalStopResult::Stopped) => {
                let line = format!("[stopped] {command} stopped by user");
                let mut log_status = None;
                if let Some(path) = self.terminal.last_log_path.as_ref()
                    && let Err(error) = append_terminal_log_path(path, &line)
                {
                    log_status = Some(error);
                }
                self.terminal.lines.push(TerminalLineEntry::new(line));
                trim_terminal_lines(&mut self.terminal.lines);
                self.finish_stopped_terminal_command();
                self.status = log_status.unwrap_or_else(|| "Terminal command stopped".to_owned());
            }
            Ok(TerminalStopResult::AlreadyExited) => {
                self.finish_stopped_terminal_command();
                self.status = "Terminal command had already exited".to_owned();
            }
            Err(error) => {
                let line = format!("[error] could not stop terminal command: {error}");
                let mut log_status = None;
                if let Some(path) = self.terminal.last_log_path.as_ref()
                    && let Err(log_error) = append_terminal_log_path(path, &line)
                {
                    log_status = Some(log_error);
                }
                self.terminal.lines.push(TerminalLineEntry::new(line));
                trim_terminal_lines(&mut self.terminal.lines);
                self.terminal.scroll_to_bottom = true;
                self.status = log_status
                    .unwrap_or_else(|| format!("Could not stop terminal command: {error}"));
            }
        }
    }

    fn finish_stopped_terminal_command(&mut self) {
        self.terminal.running = false;
        self.terminal.running_id = None;
        self.terminal.running_command = None;
        self.terminal.process = None;
        self.terminal.scroll_to_bottom = true;
        self.terminal.refocus_input = true;
    }

    /// Throw away a tag's unsaved edits and put it back the way its source has
    /// it: drop the parsed document and everything derived from it, forget any
    /// stashed Campaign Evolved overlay, then reload the tag if it is open.
    ///
    /// Forgetting the overlay is the load-bearing half for a container source.
    /// The project autosaves every dirty tag within a second of the edit, so
    /// clearing the dirty flag alone leaves the edited bytes stashed and
    /// reopening the tag restores them — the edit would be unremovable.
    pub(super) fn discard_tag_changes(&mut self, kit: usize, key: &str, ctx: &egui::Context) {
        // Reloading below goes through the active-kit path, and discarding is a
        // user action on this kit either way.
        self.active = kit;
        let was_dirty = self.kits[kit]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set());
        let had_overlay = self.forget_campaign_overlay(kit, key);
        if !was_dirty && !had_overlay {
            self.status = "That tag has no unsaved changes".to_owned();
            return;
        }
        let label = self.tag_path_label(key);
        let kit_state = &mut self.kits[kit];
        kit_state.parsed_tags.remove(key);
        kit_state.loading_tags.remove(key);
        kit_state.bitmap_previews.remove(key);
        kit_state.model_previews.remove(key);
        kit_state.field_search.remove(key);
        kit_state.field_search_applied.remove(key);
        kit_state.edit_buffers.forget_tag(key);
        // Persist the removal. The document is gone by now, so the capture
        // below cannot put the overlay straight back.
        if had_overlay {
            let now = ctx.input(|input| input.time);
            if let Err(error) = self.checkpoint_campaign_project(kit, now) {
                self.status = format!("Could not update the Campaign Evolved project: {error}");
                return;
            }
        }
        // A brand-new tag has no source to reload from — the document that was
        // just dropped WAS the tag. Take its browser entry with it instead of
        // leaving a row that errors on every reopen.
        if self.forget_new_container_entry(kit, key) {
            self.status = format!("Discarded the unsaved new tag {label}");
            return;
        }
        // Still open: reload it as the source has it, rather than leaving an
        // empty pane behind.
        if self.kits[kit].open_tabs.iter().any(|open| open == key) {
            self.select_entry(key.to_owned(), ctx.clone());
        }
        self.status = format!("Discarded unsaved changes to {label}");
    }

    /// Drop a brand-new (never-saved) container tag's browser entry, closing its
    /// pane and dropping everything derived from it. No-op — returning `false` —
    /// for any other kind of entry.
    ///
    /// Load-bearing for every path that discards a new tag's document: the
    /// document is the tag's ONLY copy (there is no `.ubulk` behind it), so an
    /// entry that outlives it is a row whose every reopen fails in `read_entry`
    /// with "unsaved new tag is no longer loaded".
    pub(super) fn forget_new_container_entry(&mut self, kit: usize, key: &str) -> bool {
        if !matches!(
            self.entry_for_key_in(kit, key).map(|entry| &entry.location),
            Some(TagEntryLocation::NewContainer { .. })
        ) {
            return false;
        }
        self.kits[kit].close_tag_pane(key);
        let kit_state = &mut self.kits[kit];
        kit_state.parsed_tags.remove(key);
        kit_state.loading_tags.remove(key);
        kit_state.bitmap_previews.remove(key);
        kit_state.model_previews.remove(key);
        kit_state.field_search.remove(key);
        kit_state.field_search_applied.remove(key);
        kit_state.edit_buffers.forget_tag(key);
        if kit_state.selected_key.as_deref() == Some(key) {
            kit_state.selected_key = None;
        }
        if let Some(source) = kit_state.source.as_mut() {
            source.entries.retain(|entry| entry.key != key);
            source.all_entries.retain(|entry| entry.key != key);
            source.tree = crate::source::build_tree(&source.entries);
            source.group_tree = crate::source::build_group_tree(&source.entries);
            if let Some(index) = source.reverse_dependencies.as_mut() {
                index.clear_tag(key);
            }
        }
        kit_state.generation = kit_state.generation.wrapping_add(1);
        true
    }

    /// Whether `key` has anything to discard — unsaved edits, or bytes stashed
    /// in this kit's project from an earlier session.
    pub(super) fn tag_has_discardable_changes(&self, kit: usize, key: &str) -> bool {
        self.kits[kit]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set())
            || self.tag_has_stashed_overlay(kit, key)
    }

    pub(super) fn select_entry(&mut self, key: String, ctx: egui::Context) {
        self.kits[self.active].open_tag_pane(&key);
        self.kits[self.active].selected_key = Some(key.clone());
        // A tag the project has an overlay for opens from the project, not from
        // disk — otherwise reopening it would silently discard its edits.
        if !self.load_campaign_overlay_for_key(self.active, &key) {
            self.ensure_tag_loading(key, ctx);
        }
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn ensure_tag_loading(&mut self, key: String, ctx: egui::Context) {
        if self.kits[self.active].parsed_tags.contains_key(&key)
            || self.kits[self.active].loading_tags.contains(&key)
        {
            return;
        }
        let Some(source) = self.source() else {
            return;
        };
        // Check both the lazily-loaded entries and the full scan set (all_entries).
        // Flat search results reference all_entries, which may not overlap with entries.
        let Some(entry) = source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .chain(self.kits[self.active].active_favorite_entries.iter())
            .find(|e| e.key == key)
            .cloned()
        else {
            return;
        };
        // A new tag reaching here has lost its document and its project overlay
        // (`select_entry` tries the overlay first), so there is nothing left to
        // read — `read_entry` would only fail on the worker thread. Retire the
        // entry here instead of leaving a row that fails forever.
        if matches!(entry.location, TagEntryLocation::NewContainer { .. }) {
            let kit = self.active;
            self.forget_new_container_entry(kit, &key);
            self.status = format!("The unsaved new tag {} was discarded", entry.display_path);
            return;
        }
        let source_kind = source.source.clone();
        let tx = self.tx.clone();
        let kit = self.active_kit_id();
        self.kits[self.active].loading_tags.insert(key.clone());
        self.status = format!("Loading {}", entry.display_path);
        thread::spawn(move || {
            let result = read_entry(&source_kind, &entry).map_err(|error| format!("{error:#}"));
            let _ = tx.send(WorkerMessage::TagLoaded { kit, key, result });
            ctx.request_repaint();
        });
    }

    /// Kept for save/export paths that address "the current tag".
    #[allow(dead_code)]
    pub(super) fn selected_entry(&self) -> Option<&TagEntry> {
        let key = self.kits[self.active].selected_key.as_ref()?;
        self.entry_for_key(key)
    }

    pub(super) fn entry_for_key(&self, key: &str) -> Option<&TagEntry> {
        self.entry_for_key_in(self.active, key)
    }

    /// Resolve a tag key against a specific kit. Anything that runs for a kit
    /// other than the focused one has to use this: a key only means something
    /// inside its own source, so resolving it against the active kit silently
    /// finds nothing and the caller skips the tag.
    pub(super) fn entry_for_key_in(&self, kit: usize, key: &str) -> Option<&TagEntry> {
        self.kits.get(kit)?.entry_for_key(key)
    }

    pub(super) fn close_tab(&mut self, key: &str) {
        // `close_tag_pane` re-derives the open set and moves the selection off
        // a removed tag, so there is nothing to fix up afterwards.
        self.kits[self.active].close_tag_pane(key);
        self.unload_tag(key);
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn request_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        if self.save_changes_prompt.visible {
            return;
        }
        // The save prompt and every save path below it address documents by
        // tag key against the active kit. Point `active` at the kit the prompt
        // will be about first, so all of that — including the project check
        // just below — resolves against the right kit.
        match &action {
            PendingCloseAction::CloseKit(id) => {
                if let Some(index) = self.kit_index(*id) {
                    self.active = index;
                }
            }
            PendingCloseAction::CloseApp => {
                if let Some(index) = self.first_dirty_kit() {
                    self.active = index;
                }
            }
            _ => {}
        }
        // Upstream skipped the unsaved-changes prompt entirely for a container
        // source, checkpointing the project instead on the grounds that the
        // project retains the edits. It does — but silently, and there was no
        // way to say no: overlays were only ever inserted, so an edit could not
        // be taken back once stashed. The prompt is raised for these sources
        // too, and offers stashing as a third, named choice.
        let can_stash = self.current_source_is_campaign_project_capable(self.active);
        let dirty_tags = self.dirty_tags_for_close_action(&action);
        if dirty_tags.is_empty() {
            self.execute_close_action(action, ctx);
            return;
        }
        // What discarding would cost, resolved here rather than described in the
        // abstract: these edits were stashed into the workspace's project within
        // a second of being typed, so declining to save deletes them from a file
        // that outlives the session.
        let stashed = dirty_tags
            .iter()
            .filter(|entry| self.tag_has_stashed_overlay(self.active, &entry.tag_id))
            .count();
        let stash_file = self.kits[self.active]
            .campaign_project
            .as_ref()
            .map(|project| project.recovery_path.clone());
        self.save_changes_prompt = SaveChangesPrompt {
            visible: true,
            can_stash,
            dirty_tags,
            pending_action: action,
            error: None,
            allow_app_close_once: self.save_changes_prompt.allow_app_close_once,
            stash_file,
            stashed,
            confirm_discard: false,
        };
    }

    /// Native app close is a two-step flow in eframe 0.29: when the OS close
    /// request arrives, Baboon sends `CancelClose` to veto it, shows the shared
    /// save prompt, then re-issues `ViewportCommand::Close` only after the user
    /// chooses Save or Don't Save. `allow_app_close_once` lets that confirmed
    /// second close request pass without opening the prompt again.
    pub(super) fn handle_app_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        if self.save_changes_prompt.allow_app_close_once {
            self.save_changes_prompt.allow_app_close_once = false;
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if self.save_changes_prompt.visible {
            return;
        }
        self.defer_file_action(DeferredFileAction::Close(PendingCloseAction::CloseApp), ctx);
    }

    fn dirty_tags_for_close_action(&self, action: &PendingCloseAction) -> Vec<DirtyTagEntry> {
        self.close_action_tag_keys(action)
            .into_iter()
            .filter_map(|key| {
                let doc = self.kits[self.active].parsed_tags.get(&key)?;
                if !doc.dirty.is_set() {
                    return None;
                }
                // Edits to a tag that has no writer (a monolithic build, a
                // big-endian tag) are session-scratch by construction. Listing
                // them here would offer a Save that always fails, and — for
                // CloseApp, which re-checks for dirty work after the prompt —
                // a close that never terminates.
                if !document_edits_are_saveable(&self.kits[self.active], &key, doc) {
                    return None;
                }
                Some(DirtyTagEntry {
                    path: self.tag_path_label(&key),
                    tag_id: key,
                    checked: true,
                })
            })
            .collect()
    }

    fn close_action_tag_keys(&self, action: &PendingCloseAction) -> Vec<String> {
        match action {
            PendingCloseAction::CloseApp | PendingCloseAction::CloseAllTabs => {
                ordered_unique_keys(self.kits[self.active].open_tabs.iter())
            }
            PendingCloseAction::CloseTab(key) => vec![key.clone()],
            PendingCloseAction::CloseAllButThis(kept_key) => ordered_unique_keys(
                self.kits[self.active]
                    .open_tabs
                    .iter()
                    .filter(|key| *key != kept_key),
            ),
            // `request_close_action` has already made this kit active, so the
            // active-kit lookups above address the right documents.
            PendingCloseAction::CloseKit(_) => {
                ordered_unique_keys(self.kits[self.active].open_tabs.iter())
            }
        }
    }

    pub(super) fn tag_path_label(&self, key: &str) -> String {
        let Some(entry) = self.entry_for_key(key) else {
            return key.to_owned();
        };
        match &entry.location {
            TagEntryLocation::LooseFile(path) => path.display().to_string(),
            TagEntryLocation::Monolithic { .. }
            | TagEntryLocation::Container { .. }
            | TagEntryLocation::NewContainer { .. } => entry.display_path.clone(),
        }
    }

    /// Whether the loaded document for `key` still has unsaved edits. Save
    /// paths that report through `status` (container writes) use this to tell
    /// success from failure.
    pub(super) fn tag_is_dirty(&self, key: &str) -> bool {
        self.kits[self.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set())
    }

    fn execute_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        match action {
            PendingCloseAction::CloseApp => {
                // The prompt covers one kit at a time. If another kit still
                // holds unsaved work, ask about it before exiting rather than
                // dropping it silently — which is what a single-kit-only
                // check used to do. "Don't save" clears the flags it listed,
                // so this converges instead of looping.
                if self.any_kit_dirty() {
                    self.request_close_action(PendingCloseAction::CloseApp, ctx);
                    return;
                }
                if let Some(session) = self.current_session_state() {
                    let _ = save_last_session(&session);
                } else {
                    clear_last_session();
                }
                self.save_changes_prompt.allow_app_close_once = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            PendingCloseAction::CloseTab(key) => self.close_tab(&key),
            PendingCloseAction::CloseAllTabs => self.close_all_tabs(),
            PendingCloseAction::CloseAllButThis(key) => self.close_all_tabs_but(&key),
            PendingCloseAction::CloseKit(id) => {
                self.remove_kit(id);
                self.color_popup = None;
                self.function_popup = None;
                self.status = "Closed kit".to_owned();
            }
        }
    }

    /// Snapshot every kit's source and open tags for the restore prompt.
    pub(super) fn current_session_state(&self) -> Option<LastSessionState> {
        let kits = (0..self.kits.len())
            .filter_map(|index| self.session_kit_state(index))
            .collect::<Vec<_>>();
        (!kits.is_empty()).then_some(LastSessionState { kits })
    }

    fn session_kit_state(&self, kit_index: usize) -> Option<LastSessionKit> {
        let kit = &self.kits[kit_index];
        let source = kit.source.as_ref()?;
        let (source_kind, source_path) = match &source.source {
            TagSource::SingleFile { path } => (LastSessionSourceKind::SingleFile, path.clone()),
            TagSource::LooseFolder { root, .. } => {
                (LastSessionSourceKind::LooseFolder, root.clone())
            }
            TagSource::MonolithicCache { root, .. } => {
                (LastSessionSourceKind::MonolithicCache, root.clone())
            }
            TagSource::IoStoreContainerSet { root, .. } => {
                (LastSessionSourceKind::IoStoreContainerSet, root.clone())
            }
        };
        // Record the folder the user actually chose, not the directory the
        // source ended up reading from. They differ for exactly the sources
        // whose root is resolved inwards: a container set mounts from
        // `<install>/Meteorite/Content/Paks`, and a loose kit from
        // `<kit>/tags`. Storing the resolved one meant every session restore
        // reloaded that inner path and remembered *it* as a recent folder, so
        // "Paks" reappeared in the recents list after each restart however
        // often it was removed.
        let source_path = kit.requested_path.clone().unwrap_or(source_path);
        let mut tags = Vec::new();
        for key in ordered_unique_keys(kit.open_tabs.iter()) {
            let Some(entry) = source
                .entries
                .iter()
                .chain(source.all_entries.iter())
                .find(|entry| entry.key == key)
            else {
                continue;
            };
            let path = match &entry.location {
                TagEntryLocation::LooseFile(path) => Some(path.clone()),
                TagEntryLocation::Monolithic { .. }
                | TagEntryLocation::Container { .. }
                | TagEntryLocation::NewContainer { .. } => None,
            };
            tags.push(LastSessionTag {
                key: entry.key.clone(),
                label: format!(
                    "{} - {}",
                    entry.display_path,
                    group_label(&kit.names, entry.group_tag)
                ),
                group_tag: entry.group_tag,
                path,
            });
        }
        let chimp_packages = ordered_unique_keys(kit.chimp.open_packages.iter());
        let active_chimp_package = kit
            .chimp
            .selected_package
            .clone()
            .filter(|active| chimp_packages.contains(active));
        // A kit with no open tags is still worth saving if it carries a
        // project or open Chimp packages.
        if tags.is_empty() && chimp_packages.is_empty() && kit.campaign_project.is_none() {
            return None;
        }
        Some(LastSessionKit {
            source_kind,
            source_path,
            game: source.game.clone(),
            profile_id: kit.profile.as_ref().map(|profile| profile.id.clone()),
            // The `.baboon` this workspace has open, if any — not its recovery
            // file, which the next session finds from the source root anyway.
            project_path: kit
                .campaign_project
                .as_ref()
                .and_then(|project| project.project_path.clone()),
            has_project: kit.campaign_project.is_some(),
            browser_mode: Some(kit.browser_mode),
            browser_sort: Some(kit.browser_sort),
            tags,
            chimp_packages,
            active_chimp_package,
        })
    }

    /// Reopen each saved kit. Every kit gets its own load, and its tags are
    /// staged on the kit itself rather than in one shared slot, so the loads
    /// can finish in any order without stealing each other's tags.
    pub(super) fn begin_last_session_restore(&mut self, kits: Vec<RestoreKit>, ctx: egui::Context) {
        for RestoreKit {
            source_kind,
            source_path,
            profile_id,
            project_path,
            has_project,
            browser_mode,
            browser_sort,
            tags,
            chimp_packages,
            active_chimp_package,
        } in kits
        {
            // A workspace whose only content is its stash still has to be
            // reopened, so that its recovery file is picked back up.
            if tags.is_empty() && chimp_packages.is_empty() && !has_project {
                continue;
            }
            match source_kind {
                LastSessionSourceKind::SingleFile => {
                    self.begin_load_single_path(source_path, ctx.clone())
                }
                LastSessionSourceKind::LooseFolder => {
                    let started = if let Some(profile) = profile_id
                        .as_deref()
                        .and_then(|id| {
                            self.custom_editing_kit_profiles
                                .iter()
                                .find(|profile| profile.id == id)
                        })
                        .cloned()
                    {
                        self.load_custom_editing_kit_profile(profile, ctx.clone())
                    } else {
                        self.begin_load_folder_path(source_path, ctx.clone());
                        true
                    };
                    if !started {
                        continue;
                    }
                }
                LastSessionSourceKind::MonolithicCache => {
                    let blob_index = if source_path.is_dir() {
                        source_path.join("blob_index.dat")
                    } else {
                        source_path
                    };
                    self.begin_load_monolithic_path(blob_index, ctx.clone());
                }
                // Upstream added container sources to the session format, so a
                // Campaign Evolved install now comes back with the rest.
                LastSessionSourceKind::IoStoreContainerSet => {
                    self.begin_load_folder_path(install_root_for_paks(&source_path), ctx.clone())
                }
            }
            // The loaders route to a kit and leave it active, so this stages
            // the tags on the kit the load will land in.
            self.kits[self.active].pending_restore_tags = tags;
            self.kits[self.active].pending_restore_chimp_packages = chimp_packages;
            self.kits[self.active].pending_restore_active_chimp_package = active_chimp_package;
            // Its browser view is staged the same way: `install_loaded_source`
            // carries it across the load rather than resetting it, so each
            // workspace comes back in the view it was left in.
            if let Some(mode) = browser_mode {
                self.kits[self.active].browser_mode = mode;
            }
            if let Some(sort) = browser_sort {
                self.kits[self.active].browser_sort = sort;
            }
            // The project file it had open is queued the same way, and is
            // attached as this workspace's save target once the source has
            // mounted. The edits themselves come back from the recovery file.
            if let Some(project_path) = project_path {
                let restoring = self.active;
                self.queue_campaign_project_target(restoring, project_path);
            }
        }
    }

    /// Reopen the tags staged for the kit that just finished loading.
    fn finish_pending_session_restore(&mut self, ctx: egui::Context) {
        let restore = std::mem::take(&mut self.kits[self.active].pending_restore_tags);
        if restore.is_empty() {
            return;
        }
        let mut opened = 0usize;
        let mut missing = 0usize;
        for tag in restore {
            if self.ensure_restored_tag_entry(&tag) {
                self.select_entry(tag.key, ctx.clone());
                opened += 1;
            } else {
                missing += 1;
            }
        }
        if opened > 0 {
            self.status = if missing > 0 {
                format!("Restored {opened} window(s); skipped {missing} missing item(s)")
            } else {
                format!("Restored {opened} window(s)")
            };
        } else if missing > 0 {
            self.status = "No saved windows could be restored".to_owned();
        }
    }

    fn ensure_restored_tag_entry(&mut self, tag: &LastSessionTag) -> bool {
        if self.entry_for_key(&tag.key).is_some() {
            return true;
        }
        let Some(path) = tag.path.as_ref() else {
            return false;
        };
        if !path.is_file() {
            return false;
        }
        let Some(source) = self.source() else {
            return false;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return false;
        };
        let Ok(root) = fs::canonicalize(root) else {
            return false;
        };
        let Ok(path) = fs::canonicalize(path) else {
            return false;
        };
        if !path.starts_with(&root) {
            return false;
        }
        let Ok(entry) = loose_file_entry(&root, &path, &source.names) else {
            return false;
        };
        let Some(entry) = entry else {
            return false;
        };
        if let Some(source) = self.source_mut() {
            source.entries.retain(|existing| existing.key != tag.key);
            source.entries.push(entry.clone());
            if !source.all_entries.is_empty() {
                source
                    .all_entries
                    .retain(|existing| existing.key != tag.key);
                source.all_entries.push(entry);
                source
                    .all_entries
                    .sort_by(|a, b| a.display_path.cmp(&b.display_path));
                source.group_tree = crate::source::build_group_tree(&source.all_entries);
            }
        }
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        true
    }

    pub(super) fn close_all_tabs(&mut self) {
        let id = self.kits[self.active].id;
        self.kits[self.active].tag_tree = egui_tiles::Tree::empty(tag_tree_id(id));
        self.kits[self.active].open_tabs.clear();
        self.kits[self.active].parsed_tags.clear();
        self.kits[self.active].loading_tags.clear();
        self.kits[self.active].bitmap_previews.clear();
        self.kits[self.active].edit_buffers.clear();
        self.kits[self.active].selected_key = None;
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn close_all_tabs_but(&mut self, key: &str) {
        for open in self.kits[self.active].tabs_from_tree() {
            if open != key {
                self.kits[self.active].close_tag_pane(&open);
            }
        }
        self.kits[self.active]
            .parsed_tags
            .retain(|tab, _| tab == key);
        self.kits[self.active].loading_tags.retain(|tab| tab == key);
        self.kits[self.active]
            .bitmap_previews
            .retain(|tab, _| tab == key);
        let edit_prefix = format!("{key}|");
        self.kits[self.active]
            .edit_buffers
            .retain(|buffer_key, _| buffer_key.starts_with(&edit_prefix));
        self.kits[self.active].selected_key = Some(key.to_owned());
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn unload_tag(&mut self, key: &str) {
        self.kits[self.active].parsed_tags.remove(key);
        self.kits[self.active].loading_tags.remove(key);
        self.kits[self.active].bitmap_previews.remove(key);
        let edit_prefix = format!("{key}|");
        self.kits[self.active]
            .edit_buffers
            .retain(|buffer_key, _| !buffer_key.starts_with(&edit_prefix));
    }

    pub(super) fn handle_browser_action(&mut self, action: BrowserAction, ctx: egui::Context) {
        match action {
            BrowserAction::Select(key) => self.select_entry(key, ctx),
            BrowserAction::ToggleFavorite(key) => self.toggle_favorite(&key),
            BrowserAction::CopyTagName(key) => self.copy_tag_name(&key, &ctx),
            BrowserAction::DumpJson(key) => self.begin_export_json(key, ctx),
            BrowserAction::OpenInExplorer(key) => self.open_entry_in_explorer(&key),
            BrowserAction::DumpLoadedFolderJson(keys) => {
                self.begin_export_loaded_folder_json(keys, ctx)
            }
            BrowserAction::DumpLooseFolderJson { rel_path, label } => {
                self.begin_export_loose_folder_json(rel_path, label, ctx)
            }
            BrowserAction::MoveLooseFolder { rel_path, label } => {
                self.begin_refactor_loose_folder(rel_path, label, true)
            }
            BrowserAction::CopyLooseFolder { rel_path, label } => {
                self.begin_refactor_loose_folder(rel_path, label, false)
            }
            BrowserAction::ConvertLooseFolder { rel_path, label } => {
                self.open_folder_conversion_dialog(rel_path, label)
            }
            BrowserAction::ExtractRaw(key) => self.begin_extract_raw(key, ctx),
            BrowserAction::ExtractBitmap(key) => self.begin_extract_bitmap(key, ctx),
            BrowserAction::ExtractBitmapFolder(keys) => self.begin_extract_bitmap_folder(keys, ctx),
            BrowserAction::ExtractGeometry(key) => self.begin_extract_geometry(key, ctx),
            BrowserAction::ExtractImportInfo(key) => self.begin_extract_import_info(key, ctx),
            BrowserAction::ExtractAnimation(key) => self.begin_extract_animation(key, ctx),
            BrowserAction::ExtractMaterialShaderSources(key) => {
                self.begin_extract_material_shader_sources(key, ctx)
            }
            BrowserAction::ExtractMaterialShaderSourceFolder(keys) => {
                self.begin_extract_material_shader_source_folder(keys, ctx)
            }
            BrowserAction::ExtractHlslIncludeSource(key) => {
                self.begin_extract_hlsl_include_source(key, ctx)
            }
            BrowserAction::ExtractHlslIncludeFolder(keys) => {
                self.begin_extract_hlsl_include_folder(keys, ctx)
            }
            BrowserAction::ExtractScenarioScripts(key) => {
                self.begin_extract_scenario_scripts(key, ctx)
            }
            BrowserAction::ImportScenarioScripts(key) => self.import_scenario_scripts(&key),
            BrowserAction::RenameTag(key) => self.open_rename_tag(&key),
            BrowserAction::FindReferences(key) => self.show_references_for(&key),
            BrowserAction::ExploreReferences(key) => self.open_content_explorer(&key),
            BrowserAction::MoveTag(key) => self.begin_move_tag(&key),
            BrowserAction::ImportTagInFolder { folder_rel } => self.begin_import_tag(folder_rel),
            BrowserAction::NewTagInFolder { folder_rel } => {
                self.open_new_tag_dialog_in_folder(folder_rel)
            }
        }
    }

    pub(super) fn copy_tag_name(&mut self, key: &str, ctx: &egui::Context) {
        let Some(entry) = self.entry_for_key(key) else {
            self.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        let copied_path = crate::format::to_native_path_string(&entry.display_path);
        ctx.output_mut(|output| output.copied_text = copied_path.clone());
        self.status = format!("Copied {copied_path}");
    }

    pub(super) fn open_entry_in_explorer(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        let Some(source) = self.source().map(|source| &source.source) else {
            self.status = "No source loaded".to_owned();
            return;
        };
        let path = match (&entry.location, source) {
            (TagEntryLocation::LooseFile(path), _) => path.clone(),
            (_, TagSource::SingleFile { path }) => path.clone(),
            (TagEntryLocation::Monolithic { .. }, TagSource::MonolithicCache { root, .. }) => {
                root.join("blob_index.dat")
            }
            (TagEntryLocation::Monolithic { .. }, _) => {
                self.status = "Monolithic tag has no loose file to show".to_owned();
                return;
            }
            (TagEntryLocation::Container { .. }, _) => {
                self.status = "Container tag has no loose file to show".to_owned();
                return;
            }
            (TagEntryLocation::NewContainer { .. }, _) => {
                self.status = "New tag has not been saved yet".to_owned();
                return;
            }
        };
        #[cfg(windows)]
        {
            if !path.is_file() {
                self.status = format!(
                    "Could not open File Explorer: file no longer exists at {}",
                    path.display()
                );
                return;
            }
            match Command::new("explorer.exe")
                .args(explorer_select_args(&path))
                .spawn()
            {
                Ok(_) => self.status = format!("Opened {}", path.display()),
                Err(error) => self.status = format!("Could not open File Explorer: {error}"),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            self.status = "Open with File Explorer is only available on Windows".to_owned();
        }
    }

    pub(super) fn open_loaded_tags_folder(&mut self) {
        let Some(path) = self.loaded_tags_root() else {
            self.status = "Open Tags Folder requires a loaded editing-kit tags folder".to_owned();
            return;
        };
        self.open_folder_in_explorer(path, "tags");
    }

    pub(super) fn open_loaded_data_folder(&mut self) {
        let Some(path) = self.loaded_data_root() else {
            self.status = "Open Data Folder requires a loaded editing-kit tags folder".to_owned();
            return;
        };
        self.open_folder_in_explorer(path, "data");
    }

    fn loaded_data_root(&self) -> Option<PathBuf> {
        Some(self.editing_kit_root()?.join("data"))
    }

    pub(super) fn open_folder_in_explorer(&mut self, path: PathBuf, label: &str) {
        if !path.is_dir() {
            self.status = format!("{label} folder not found: {}", path.display());
            return;
        }

        #[cfg(windows)]
        {
            match Command::new("explorer").arg(&path).spawn() {
                Ok(_) => self.status = format!("Opened {} folder: {}", label, path.display()),
                Err(error) => self.status = format!("Could not open File Explorer: {error}"),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            self.status = "Open folder is only available on Windows".to_owned();
        }
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_export_json(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let default_name = format!("{}.json", tag_file_stem(&entry));
        let Some(output) = rfd::FileDialog::new()
            .set_title("Dump Tag JSON")
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        self.status = format!("Dumping JSON for {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = export_tag_json(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_export_loaded_folder_json(
        &mut self,
        keys: Vec<String>,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.source() else {
            return;
        };
        let entries = keys
            .iter()
            .filter_map(|key| source_data.entries.iter().find(|entry| entry.key == *key))
            .cloned()
            .collect::<Vec<_>>();
        if entries.is_empty() {
            self.status = "No loaded tags found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Dump Folder JSON")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Dumping {} loaded tag(s) to JSON", entries.len());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                export_tag_json_entries(&source, &entries, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_export_loose_folder_json(
        &mut self,
        rel_path: PathBuf,
        label: String,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.source() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source_data.source else {
            return;
        };
        let root = root.clone();
        let names = source_data.names.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Dump Folder JSON")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Dumping JSON for folder {label}");
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = export_loose_folder_json(&root, &rel_path, &names, &output)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts a filesystem refactoring transaction from a captured source snapshot.
    /// Progress and the final replacement tree are applied only through worker messages.
    pub(super) fn begin_refactor_loose_folder(
        &mut self,
        rel_path: PathBuf,
        label: String,
        move_folder: bool,
    ) {
        if self.folder_refactor.is_some() {
            self.status = "A folder move/copy is already running".to_owned();
            return;
        }
        if self.kits[self.active]
            .parsed_tags
            .values()
            .any(|doc| doc.dirty.is_set())
        {
            self.status = "Save or close dirty tags before moving/copying folders".to_owned();
            return;
        }
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Folder move/copy requires a loaded tags folder".to_owned();
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
        let names = self.names().clone();
        let existing_all_entries = self
            .source()
            .map(|source| source.all_entries.clone())
            .unwrap_or_default();
        let existing_reverse_dependencies = self
            .source()
            .and_then(|source| source.reverse_dependencies.clone());
        let game = self.source().and_then(|source| source.game.clone());
        // Routed back to the kit the refactor was started in, not
        // whichever one is focused when it lands.
        let stamp = self.kit_stamp();
        let tx = self.tx.clone();
        let job_label = if move_folder {
            format!("Moving {label}")
        } else {
            format!("Copying {label}")
        };
        self.folder_refactor = Some(FolderRefactorUiState {
            label: job_label.clone(),
            phase: "Preparing".to_owned(),
            progress: None,
        });
        self.status = format!("{job_label}: Preparing");
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_folder_refactor_job(
                    root,
                    rel_path,
                    destination_parent,
                    move_folder,
                    job_label,
                    names,
                    game,
                    existing_all_entries,
                    existing_reverse_dependencies,
                    &tx,
                )
            }))
            .unwrap_or_else(|_| Err("Folder move/copy worker crashed".to_owned()));
            let _ = tx.send(WorkerMessage::FolderRefactorFinished { stamp, result });
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_raw(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Raw Tag")
            .set_file_name(tag_file_name(&entry).as_str())
            .save_file()
        else {
            return;
        };
        self.status = format!("Extracting raw tag {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = extract_raw_tag(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_bitmap(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Bitmap Images")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting bitmap {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = extract_bitmap_images(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_bitmap_folder(&mut self, keys: Vec<String>, ctx: egui::Context) {
        let Some(source_data) = self.source() else {
            return;
        };
        let entries = keys
            .iter()
            .filter_map(|key| source_data.entries.iter().find(|entry| entry.key == *key))
            .cloned()
            .collect::<Vec<_>>();
        if entries.is_empty() {
            self.status = "No bitmap tags found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract All Bitmaps")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting {} bitmap tag(s)", entries.len());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_bitmap_entries(&source, &entries, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_geometry(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Geometry")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting geometry from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_geometry_for_entry(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_import_info(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Import Info")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting import info from {}", entry.display_path);
        let tx = self.tx.clone();
        let is_model = entry.group_tag == u32::from_be_bytes(*b"hlmt");
        thread::spawn(move || {
            let result = if is_model {
                extract_import_info_for_model_entry(&source, &entry, &output)
            } else {
                extract_import_info_for_entry(&source, &entry, &output)
            }
            .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_animation(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Animations")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting animations from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_animations_for_entry(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_material_shader_sources(
        &mut self,
        key: String,
        ctx: egui::Context,
    ) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Source Shaders")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting source shaders from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = extract_material_shader_sources(&source, &entry, &output)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_material_shader_source_folder(
        &mut self,
        keys: Vec<String>,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.source() else {
            return;
        };
        let entries = entries_for_keys(source_data, &keys);
        if entries.is_empty() {
            self.status = "No material shaders found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Material Shader Sources")
            .pick_folder()
        else {
            return;
        };
        self.status = format!(
            "Extracting source shaders from {} material shader(s)",
            entries.len()
        );
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = extract_material_shader_source_entries(&source, &entries, &output)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_scenario_scripts(&mut self, key: String, ctx: egui::Context) {
        if !self.active_game_is_campaign_evolved() {
            self.status = "Script extraction is only available for Campaign Evolved".to_owned();
            return;
        }
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Scripts")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting scripts from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_scenario_scripts(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Replace a scenario's `source files` block from a folder of `.hsc` files.
    ///
    /// Runs on the UI thread because it edits the loaded document: the tag is
    /// left **modified, not saved**, so the change goes through the same review
    /// and save path as any other edit. If the tag is not open yet it is read
    /// first — synchronously, since the result has to be mutated in the same
    /// step rather than handed to a worker.
    pub(super) fn import_scenario_scripts(&mut self, key: &str) {
        if !self.active_game_is_campaign_evolved() {
            self.status = "Script import is only available for Campaign Evolved".to_owned();
            return;
        }
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        if !is_scenario_group(entry.group_tag) {
            self.status = "Script import is only available for scenario tags".to_owned();
            return;
        }
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Import Scripts")
            .pick_folder()
        else {
            return;
        };

        if !self.kits[self.active].parsed_tags.contains_key(key) {
            let Some(source) = self.source().map(|source| source.source.clone()) else {
                self.status = "No tag source is loaded".to_owned();
                return;
            };
            self.status = format!("Loading {}", entry.display_path);
            match read_entry(&source, &entry) {
                Ok(tag) => {
                    self.kits[self.active]
                        .parsed_tags
                        .insert(key.to_owned(), TagDocument::clean(tag));
                }
                Err(error) => {
                    self.status = format!("Could not load {}: {error:#}", entry.display_path);
                    return;
                }
            }
        }

        let Some(document) = self.kits[self.active].parsed_tags.get_mut(key) else {
            self.status = "Load the tag before importing scripts".to_owned();
            return;
        };
        match replace_scenario_scripts(&mut document.tag, &folder) {
            Ok(message) => {
                document.dirty.touch();
                self.kits[self.active].open_tag_pane(key);
                self.kits[self.active].selected_key = Some(key.to_owned());
                self.status = format!("{message} (unsaved)");
            }
            // A failed read leaves the block untouched — `replace_scenario_scripts`
            // reads the whole folder before it clears anything.
            Err(error) => self.status = format!("Could not import scripts: {error:#}"),
        }
    }

    pub(super) fn active_game_is_campaign_evolved(&self) -> bool {
        self.source()
            .and_then(|source| source.game.as_deref())
            .is_some_and(|game| game == "haloce_evolved")
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_hlsl_include_source(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract HLSL Include")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting HLSL include from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_hlsl_include_source(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_extract_hlsl_include_folder(
        &mut self,
        keys: Vec<String>,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.source() else {
            return;
        };
        let entries = entries_for_keys(source_data, &keys);
        if entries.is_empty() {
            self.status = "No HLSL includes found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract HLSL Includes")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting {} HLSL include(s)", entries.len());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_hlsl_include_entries(&source, &entries, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn export_context(&self, key: &str) -> Option<(TagSource, TagEntry)> {
        let source = self.source()?.source.clone();
        let entry = self.entry_for_key(key)?.clone();
        Some((source, entry))
    }

    pub(super) fn save_current_tag(&mut self) {
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "No tag selected".to_owned();
            return;
        };
        // A brand-new (in-memory) container tag has no baseline to overwrite —
        // "Save" writes it as a new `_P` override container instead.
        if matches!(
            self.entry_for_key(&key).map(|entry| &entry.location),
            Some(TagEntryLocation::NewContainer { .. })
        ) {
            self.save_new_container_tag(&key);
            return;
        }
        // For a container tag, "Save" overwrites the tag inside the game's pak
        // in place (destructive). Confirm first unless the user opted out (loose
        // builds never prompt).
        if self.current_source_is_container() {
            if self.confirm_container_overwrite {
                self.overwrite_confirm = Some(OverwriteConfirm {
                    kit: self.active_kit_id(),
                    key,
                });
            } else {
                self.overwrite_current_tag_in_place(&key);
            }
            return;
        }
        match self.save_tag_by_key(&key) {
            Ok(path) => self.status = format!("Saved {}", path.display()),
            Err(error) => self.status = format!("Save failed: {error}"),
        }
    }

    pub(super) fn save_tag_by_key(&mut self, key: &str) -> Result<PathBuf, String> {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return Err("Selected tag is no longer in the source".to_owned());
        };
        let Some(doc) = self.kits[self.active].parsed_tags.get(key) else {
            return Err("Load the selected tag before saving".to_owned());
        };
        if let Some(reason) = unsaveable_reason(&entry, &doc.tag) {
            return Err(reason.to_owned());
        }
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            // Container tags are writable, just not through the loose-file
            // path — reaching here means a caller skipped the container
            // routing, so say that rather than blaming a monolithic cache.
            return Err(match &entry.location {
                TagEntryLocation::Container { .. } | TagEntryLocation::NewContainer { .. } => {
                    "Container tags cannot be saved as loose files".to_owned()
                }
                _ => "Monolithic cache tags are read-only".to_owned(),
            });
        };
        let output = path.clone();
        doc.tag
            .write_atomic(&output)
            .map_err(|error| error.to_string())?;
        if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(key) {
            doc.dirty.clear();
        }
        Ok(output)
    }

    pub(super) fn current_source_is_container(&self) -> bool {
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
    fn export_container_override(
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

        // Tag content: current edited bytes if the tag is loaded, else the
        // original `.ubulk`.
        let tag_bytes = if let Some(doc) = self.kits[self.active].parsed_tags.get(key) {
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
                blam_tags::iostore::writer::write_new_tag_container(
                    &template,
                    &tag_bytes,
                    &new_pkg,
                    if redirect {
                        Some(old_pkg.as_str())
                    } else {
                        None
                    },
                    &output,
                )
                .map_err(|e| format!("write container: {e}"))?;
                Ok(Some(output))
            }
        }
    }

    /// Overwrite the current container tag inside its own pak, in place.
    /// **Destructive** — modifies the shipped game files. Only reached after the
    /// user confirms the overwrite dialog.
    pub(super) fn overwrite_current_tag_in_place(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer in the source".to_owned();
            return;
        };
        let TagEntryLocation::Container {
            container,
            rel_path,
        } = &entry.location
        else {
            self.status = "Not a Campaign Evolved container tag".to_owned();
            return;
        };
        let container_idx = *container;
        let rel_path = rel_path.clone();
        let Some(doc) = self.kits[self.active].parsed_tags.get(key) else {
            self.status = "Load the tag before saving".to_owned();
            return;
        };
        let bytes = match doc.tag.write_to_bytes() {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Failed to serialize tag: {e}");
                return;
            }
        };
        let (root, utoc_path, archive) = {
            let Some(source) = self.source() else {
                self.status = "No source loaded".to_owned();
                return;
            };
            let TagSource::IoStoreContainerSet {
                root, containers, ..
            } = &source.source
            else {
                self.status = "Source is not a container".to_owned();
                return;
            };
            let Some(m) = containers.get(container_idx) else {
                self.status = "Container provenance is stale".to_owned();
                return;
            };
            (root.clone(), m.utoc_path.clone(), m.archive.clone())
        };
        // Resolve against the MOUNTED archive, not a fresh handle: an override
        // container (an exported mod the user then reloaded) ships no directory
        // index, and only the mounted handle has the rebuilt file list that can
        // name `rel_path`.
        if let Err(e) = blam_tags::iostore::writer::overwrite_tag_in_place_with(
            &archive, &utoc_path, &rel_path, &bytes,
        ) {
            // A mod exported by an older build carries the tag alone, so there
            // is no `.uasset` chunk to rewrite the declared length into and
            // nothing can be added to a container in place.
            let hint = if e.to_string().contains("no paired .uasset") {
                " — export this mod again instead of saving into it"
            } else {
                ""
            };
            self.status = format!("Overwrite failed: {e}{hint}");
            return;
        }
        drop(archive);
        // Hot-swap the pak's archive so subsequent reads see the new bytes.
        let containers = match self.source().map(|s| &s.source) {
            Some(TagSource::IoStoreContainerSet { containers, .. }) => containers.clone(),
            _ => Vec::new(),
        };
        let reload_error =
            match crate::source::reopen_container_archive(&root, &containers, container_idx) {
                Ok(a) => {
                    if let Some(source) = self.source_mut()
                        && let TagSource::IoStoreContainerSet { containers, .. } =
                            &mut source.source
                        && let Some(m) = containers.get_mut(container_idx)
                    {
                        m.archive = std::sync::Arc::new(a);
                    }
                    None
                }
                Err(e) => Some(e),
            };
        if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(key) {
            doc.dirty.clear();
        }
        self.status = match reload_error {
            Some(e) => format!(
                "Saved into {}, but reloading the pak failed: {e}",
                utoc_path.display()
            ),
            None => format!("Saved into {} (game files modified)", utoc_path.display()),
        };
    }

    /// Save a brand-new (in-memory) container tag as a new `_P` override
    /// container. A new tag has no baseline in the paks to overwrite, so this
    /// writes a standalone override package via `write_new_tag_container`, using
    /// a same-group tag's `.uasset` as the package template. The base game is
    /// untouched; the user copies the emitted `.utoc`/`.ucas`/`.pak` into `Paks/`.
    fn save_new_container_tag(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer in the source".to_owned();
            return;
        };
        let TagEntryLocation::NewContainer {
            template_container,
            template_rel,
            package,
            ..
        } = &entry.location
        else {
            self.status = "Not a new container tag".to_owned();
            return;
        };
        let Some(doc) = self.kits[self.active].parsed_tags.get(key) else {
            self.status = "Load the tag before saving".to_owned();
            return;
        };
        let bytes = match doc.tag.write_to_bytes() {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Failed to serialize tag: {e}");
                return;
            }
        };
        let template = {
            let Some(source) = self.source() else {
                self.status = "No source loaded".to_owned();
                return;
            };
            let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
                self.status = "Source is not a container".to_owned();
                return;
            };
            let Some(mounted) = containers.get(*template_container) else {
                self.status = "Template container is stale".to_owned();
                return;
            };
            match mounted.archive.read(template_rel) {
                Ok(b) => b,
                Err(e) => {
                    self.status = format!("Failed to read template .uasset: {e}");
                    return;
                }
            }
        };
        let leaf = package.rsplit('/').next().unwrap_or("tag");
        let Some(output) = pick_override_utoc(&format!("{leaf}_P.utoc")) else {
            return;
        };
        match blam_tags::iostore::writer::write_new_tag_container(
            &template, &bytes, package, None, &output,
        ) {
            Ok(()) => {
                if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(key) {
                    doc.dirty.clear();
                }
                let stem = output.file_stem().and_then(|s| s.to_str()).unwrap_or("mod");
                self.status = format!(
                    "Saved new tag → {stem}.utoc/.ucas/.pak — copy all three into \
                     Meteorite/Content/Paks/ (base game unchanged)"
                );
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    /// Bundle every modified Campaign Evolved project tag into one portable `_P`
    /// overlay and write its `.baboon` recovery project beside the triplet.
    /// Open the review of what Export Mod would write.
    ///
    /// Nothing is written and no destination is chosen here. The snapshot is
    /// captured now and kept, so what the user reviews and what is written
    /// cannot drift apart between the two steps.
    pub(super) fn export_mod(&mut self) {
        self.open_mod_review(false);
    }

    /// Review what this workspace is carrying, without exporting anything.
    pub(super) fn review_changes(&mut self) {
        self.open_mod_review(true);
    }

    fn open_mod_review(&mut self, review_only: bool) {
        let exporting = self.active;
        let snapshot = match self.capture_campaign_project(exporting, 0.0) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.status = "Export Mod is only for Campaign Evolved containers".to_owned();
                return;
            }
            Err(error) => {
                self.status = format!("Could not checkpoint project for export: {error}");
                return;
            }
        };
        let mut rows: Vec<ModExportRow> = snapshot
            .overlays
            .values()
            .map(|overlay| {
                // An overlay whose tag no longer resolves cannot be written.
                // That was previously counted into a status line and dropped;
                // it is a row here, so it is at least visible.
                let resolvable = self
                    .campaign_entry_for_identity(exporting, &overlay.identity)
                    .is_some();
                let kind = classify_overlay(
                    resolvable,
                    overlay.kind,
                    // Only asked where the answer can matter, so a review does
                    // not read shipped payloads it has no use for.
                    resolvable
                        && overlay.kind == CampaignProjectTagKind::Existing
                        && self.overlay_matches_shipped(exporting, overlay),
                );
                let overridden_by = self.mod_serving_tag(exporting, &overlay.identity);
                ModExportRow {
                    identity: overlay.identity.clone(),
                    display_path: overlay.logical_path.clone(),
                    group_tag: overlay.group_tag,
                    kind,
                    include: !matches!(
                        kind,
                        ModExportChange::Unresolved | ModExportChange::Unchanged
                    ),
                    bytes: overlay.bytes.len(),
                    reason: match kind {
                        ModExportChange::Unresolved => Some("not in this source".to_owned()),
                        ModExportChange::Unchanged => {
                            Some("identical to the game's copy".to_owned())
                        }
                        _ => None,
                    },
                    overridden_by,
                }
            })
            .collect();
        rows.sort_by(|a, b| a.display_path.cmp(&b.display_path));

        // The container source's root is already the game's `Paks` directory,
        // so a mod exported straight there needs no copying at all.
        let folder = self.kits[exporting]
            .source
            .as_ref()
            .map(|source| source.source.root_path().to_path_buf())
            .unwrap_or_default();
        self.mod_export = Some(ModExportDialog {
            kit: self.active_kit_id(),
            review_only,
            snapshot,
            rows,
            name: "mymod".to_owned(),
            folder,
            overwrite_acknowledged: false,
            expanded: HashSet::new(),
            diffs: HashMap::new(),
            controls_height: 0.0,
        });
    }

    /// Whether a stashed overlay is byte-for-byte what the game already ships.
    ///
    /// Answered on bytes rather than by diffing parsed tags: a diff can come
    /// back empty for two tags that are not identical (a field the differ does
    /// not reach), and "nothing to export" has to mean *nothing*, not "nothing
    /// I looked at".
    ///
    /// A tag only a mod provides has no shipped counterpart, so it is never
    /// unchanged -- there is nothing for it to be identical to.
    fn overlay_matches_shipped(&self, kit: usize, overlay: &CampaignProjectOverlay) -> bool {
        let Some(entry) = self.campaign_entry_for_identity(kit, &overlay.identity) else {
            return false;
        };
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return false;
        };
        matches!(
            crate::source::read_shipped_entry_bytes(&source.source, &entry),
            Ok(Some(bytes)) if bytes == *overlay.bytes
        )
    }

    /// Compute the field differences for one reviewed tag, against the tag as
    /// the game ships it.
    ///
    /// The baseline comes from the *shipped* containers, not from whatever the
    /// mount resolved the tag to -- which is the comparison the reviewer actually
    /// wants: what this mod changes about the game, not what changed since the
    /// last autosave, and not "nothing" because an earlier export of this very
    /// mod is installed under `Paks` and now serves the tag.
    pub(super) fn diff_reviewed_tag(&self, kit: usize, identity: &str) -> ModRowDiff {
        const LIMIT: usize = 5000;
        let failed = |error: String| ModRowDiff {
            rows: Vec::new(),
            base: None,
            edited: None,
            truncated: false,
            error: Some(error),
        };
        let Some(dialog) = self.mod_export.as_ref() else {
            return failed("The review is no longer open".to_owned());
        };
        let Some(overlay) = dialog.snapshot.overlays.get(identity) else {
            return failed("This tag is no longer in the export".to_owned());
        };
        let Some(entry) = self.campaign_entry_for_identity(kit, identity) else {
            return failed("This tag is no longer in the source".to_owned());
        };
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return failed("No source loaded".to_owned());
        };
        let edited_tag = match TagFile::read_from_bytes(&overlay.bytes) {
            Ok(edited) => edited,
            Err(error) => return failed(format!("Could not read the edited tag: {error}")),
        };
        // A tag this workspace created has no shipped counterpart to compare
        // against, so the whole tag is described instead.
        let describe_whole = |edited_tag: TagFile, names: &TagNameIndex| {
            let (rows, truncated) = describe_tag(&edited_tag, names, LIMIT);
            ModRowDiff {
                rows,
                base: None,
                edited: Some(edited_tag),
                truncated,
                error: None,
            }
        };
        if overlay.kind == CampaignProjectTagKind::New {
            return describe_whole(edited_tag, &self.kits[kit].names);
        }
        let base = match crate::source::read_shipped_entry(&source.source, &entry) {
            Ok(Some(base)) => base,
            // Only a mod carries this tag, so there is no shipped version to
            // difference against — describing it whole is the honest answer, and
            // it is what the reviewer needs to see either way.
            Ok(None) => return describe_whole(edited_tag, &self.kits[kit].names),
            Err(error) => return failed(format!("Could not read the shipped tag: {error}")),
        };
        let (rows, truncated) = diff_tags(&base, &edited_tag, &self.kits[kit].names, LIMIT);
        ModRowDiff {
            rows,
            base: Some(base),
            edited: Some(edited_tag),
            truncated,
            error: None,
        }
    }

    /// Dump everything the review is working from into `folder`, so a diff
    /// that looks wrong can be reproduced away from the UI.
    ///
    /// Writes the computed rows as JSON, and for every tag both sides as raw
    /// bytes: the tag as the game ships it and the tag as this workspace has
    /// it. Those two files are enough to re-run the comparison exactly.
    pub(super) fn save_review_diagnostic(&mut self, folder: PathBuf) -> Result<usize, String> {
        let Some(kit) = self
            .mod_export
            .as_ref()
            .map(|dialog| dialog.kit)
            .and_then(|kit| self.resolve_kit(kit))
        else {
            return Err("The review is no longer open".to_owned());
        };
        let identities: Vec<String> = self
            .mod_export
            .as_ref()
            .map(|dialog| dialog.rows.iter().map(|row| row.identity.clone()).collect())
            .unwrap_or_default();

        let mut tags = Vec::new();
        for identity in identities {
            // Computed on demand, so a diagnostic does not depend on which rows
            // the user happened to expand.
            if !self
                .mod_export
                .as_ref()
                .is_some_and(|dialog| dialog.diffs.contains_key(&identity))
            {
                let diff = self.diff_reviewed_tag(kit, &identity);
                if let Some(dialog) = self.mod_export.as_mut() {
                    dialog.diffs.insert(identity.clone(), diff);
                }
            }
            let Some(dialog) = self.mod_export.as_ref() else {
                break;
            };
            let Some(diff) = dialog.diffs.get(&identity) else {
                continue;
            };
            let Some(row) = dialog.rows.iter().find(|row| row.identity == identity) else {
                continue;
            };
            let stem: String = identity
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            for (suffix, tag) in [
                ("base", diff.base.as_ref()),
                ("edited", diff.edited.as_ref()),
            ] {
                let Some(tag) = tag else { continue };
                let bytes = tag
                    .write_to_bytes()
                    .map_err(|error| format!("Could not serialize {identity}: {error}"))?;
                std::fs::write(folder.join(format!("{stem}.{suffix}.tag")), bytes)
                    .map_err(|error| format!("Could not write {stem}.{suffix}.tag: {error}"))?;
            }
            tags.push(serde_json::json!({
                "identity": identity,
                "path": row.display_path,
                "kind": match row.kind {
                    ModExportChange::New => "new",
                    ModExportChange::Modified => "modified",
                    ModExportChange::Unresolved => "unresolved",
                    ModExportChange::Unchanged => "unchanged",
                },
                "bytes": row.bytes,
                "error": diff.error,
                "truncated": diff.truncated,
                "rows": diff
                    .rows
                    .iter()
                    .map(|row| serde_json::json!({
                        "path": row.path,
                        "base_path": row.base_path,
                        "before": row.a,
                        "after": row.b,
                    }))
                    .collect::<Vec<_>>(),
            }));
        }
        let count = tags.len();
        let document = serde_json::json!({ "tags": tags });
        let text = serde_json::to_string_pretty(&document)
            .map_err(|error| format!("Could not encode the diagnostic: {error}"))?;
        std::fs::write(folder.join("review-diagnostic.json"), text)
            .map_err(|error| format!("Could not write review-diagnostic.json: {error}"))?;
        Ok(count)
    }

    /// The mounted mod currently serving this tag, if the mount resolved it to
    /// one rather than to the game's own pack.
    pub(super) fn mod_serving_tag(&self, kit: usize, identity: &str) -> Option<String> {
        let source = self.kits.get(kit)?.source.as_ref()?;
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return None;
        };
        let entry = self.campaign_entry_for_identity(kit, identity)?;
        let TagEntryLocation::Container { container, .. } = &entry.location else {
            return None;
        };
        containers
            .get(*container)
            .filter(|container| container.is_mod)
            .map(|container| container.chunk_label.clone())
    }

    /// The container a tag would be written into, and whether it is a mod.
    pub(super) fn container_label_for_tag(&self, kit: usize, key: &str) -> Option<(String, bool)> {
        let source = self.kits.get(kit)?.source.as_ref()?;
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return None;
        };
        let TagEntryLocation::Container { container, .. } =
            &self.entry_for_key_in(kit, key)?.location
        else {
            return None;
        };
        containers
            .get(*container)
            .map(|container| (container.chunk_label.clone(), container.is_mod))
    }

    /// Every mod this workspace has mounted, by container label.
    pub(super) fn mounted_mod_labels(&self, kit: usize) -> Vec<String> {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return Vec::new();
        };
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return Vec::new();
        };
        containers
            .iter()
            .filter(|container| container.is_mod)
            .map(|container| container.chunk_label.clone())
            .collect()
    }

    /// The mounted containers an export to `output` would replace, by label.
    ///
    /// A mod installed under `Paks` is mounted like any other container, and
    /// mounting memory-maps its `.ucas`. Replacing that file means releasing the
    /// mapping first — Windows refuses to truncate a file with a mapped section
    /// open — so this is what the review dialog says out loud and what the export
    /// releases before it writes.
    pub(super) fn export_replaces_mounted(&self, kit: usize, output: &Path) -> Vec<String> {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return Vec::new();
        };
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return Vec::new();
        };
        crate::source::mounted_containers_at(&source.source, output)
            .into_iter()
            .filter_map(|index| containers.get(index))
            .map(|container| container.chunk_label.clone())
            .collect()
    }

    /// Release the `.ucas` mapping of every mounted container an export to
    /// `output` is about to replace, so the writer can truncate the file.
    ///
    /// `Err` when a mapping cannot be released because something else still holds
    /// the archive — better a named refusal than a write that fails at the OS with
    /// `ERROR_USER_MAPPED_FILE` and nothing to connect it to the mod being
    /// installed.
    pub(super) fn release_export_target_mappings(
        &mut self,
        kit: usize,
        output: &Path,
    ) -> Result<Vec<usize>, String> {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return Ok(Vec::new());
        };
        let targets = crate::source::mounted_containers_at(&source.source, output);
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let Some(source) = self.kits.get_mut(kit).and_then(|kit| kit.source.as_mut()) else {
            return Ok(Vec::new());
        };
        let TagSource::IoStoreContainerSet { containers, .. } = &mut source.source else {
            return Ok(Vec::new());
        };
        let mut released = Vec::new();
        for index in targets {
            let Some(mounted) = containers.get_mut(index) else {
                continue;
            };
            let label = mounted.chunk_label.clone();
            // Only the mount may hold this archive. A surviving clone — a preview
            // still reading, a worker mid-scan — keeps the mapping alive whatever
            // this does, and the write would fail anyway.
            let Some(archive) = std::sync::Arc::get_mut(&mut mounted.archive) else {
                return Err(format!(
                    "Export Mod failed: {label} is being read by this workspace right now, so it \
                     cannot be replaced. Try again in a moment, or export under a different name."
                ));
            };
            archive.release_partition();
            released.push(index);
        }
        Ok(released)
    }

    /// Put back what [`Self::release_export_target_mappings`] released, once the
    /// files it was protecting have been rewritten.
    ///
    /// The container on disk is a different file now, so its index is reopened
    /// rather than the old one remapped: an override container ships no directory
    /// index, and `reopen_container_archive` is what rebuilds its file list from
    /// the containers it overrides. Falls back to remapping the archive that is
    /// already there, which at least leaves the mount readable.
    pub(super) fn restore_released_mappings(
        &mut self,
        kit: usize,
        released: &[usize],
    ) -> Vec<String> {
        if released.is_empty() {
            return Vec::new();
        }
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return Vec::new();
        };
        let TagSource::IoStoreContainerSet {
            root, containers, ..
        } = &source.source
        else {
            return Vec::new();
        };
        let (root, containers) = (root.clone(), containers.clone());
        let mut failures = Vec::new();
        for &index in released {
            let reopened = crate::source::reopen_container_archive(&root, &containers, index);
            let Some(source) = self.kits.get_mut(kit).and_then(|kit| kit.source.as_mut()) else {
                continue;
            };
            let TagSource::IoStoreContainerSet { containers, .. } = &mut source.source else {
                continue;
            };
            let Some(mounted) = containers.get_mut(index) else {
                continue;
            };
            match reopened {
                Ok(archive) => mounted.archive = std::sync::Arc::new(archive),
                Err(error) => {
                    if let Some(archive) = std::sync::Arc::get_mut(&mut mounted.archive)
                        && archive.remap_partition().is_ok()
                    {
                        failures.push(format!("{}: {error}", mounted.chunk_label));
                        continue;
                    }
                    failures.push(format!("{}: {error}", mounted.chunk_label));
                }
            }
        }
        failures
    }

    /// Write the reviewed mod. `included` are the identities the user kept.
    pub(super) fn write_reviewed_mod(
        &mut self,
        snapshot: &CampaignProjectSnapshot,
        included: &HashSet<String>,
        output: PathBuf,
    ) {
        let exporting = self.active;
        let Some(source) = self.source() else {
            self.status = "No source loaded".to_owned();
            return;
        };
        let TagSource::IoStoreContainerSet {
            containers,
            shipped,
            ..
        } = &source.source
        else {
            self.status = "Export Mod is only for Campaign Evolved containers".to_owned();
            return;
        };
        let mut overrides: Vec<(
            std::sync::Arc<blam_tags::iostore::IoStoreArchive>,
            String,
            Vec<u8>,
        )> = Vec::new();
        let mut new_pkgs: Vec<(Vec<u8>, Vec<u8>, String)> = Vec::new();
        let mut skipped = 0usize;
        for overlay in snapshot.overlays.values() {
            if !included.contains(&overlay.identity) {
                continue;
            }
            let Some(entry) = self.campaign_entry_for_identity(exporting, &overlay.identity) else {
                skipped += 1;
                continue;
            };
            match &entry.location {
                TagEntryLocation::Container {
                    container,
                    rel_path,
                } => {
                    // The base an override is built against is the game's own
                    // pack, not whatever the mount resolved this tag to. With a
                    // mod installed under `Paks`, the latter is that mod — so the
                    // export read its chunk layout out of the very file it was
                    // about to replace.
                    let base = shipped.container_for(rel_path).unwrap_or(*container);
                    let Some(m) = containers.get(base) else {
                        skipped += 1;
                        continue;
                    };
                    overrides.push((
                        m.archive.clone(),
                        rel_path.clone(),
                        overlay.bytes.as_ref().clone(),
                    ));
                }
                TagEntryLocation::NewContainer {
                    template_container,
                    template_rel,
                    package,
                    ..
                } => {
                    let Some(m) = containers.get(*template_container) else {
                        skipped += 1;
                        continue;
                    };
                    let Ok(template) = m.archive.read(template_rel) else {
                        skipped += 1;
                        continue;
                    };
                    new_pkgs.push((template, overlay.bytes.as_ref().clone(), package.clone()));
                }
                _ => skipped += 1,
            }
        }
        let count = overrides.len() + new_pkgs.len();
        if count == 0 {
            self.status = "Nothing selected to export".to_owned();
            return;
        }
        // The writer truncates the three files it writes, and a container this
        // workspace has mounted has its `.ucas` mapped — which Windows will not
        // let anyone truncate. Releasing has to happen after the bases above are
        // collected (they are other containers) and before the writer runs.
        let released = match self.release_export_target_mappings(exporting, &output) {
            Ok(released) => released,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let override_refs: Vec<(&blam_tags::iostore::IoStoreArchive, &str, &[u8])> = overrides
            .iter()
            .map(|(a, p, b)| (a.as_ref(), p.as_str(), b.as_slice()))
            .collect();
        let new_refs: Vec<blam_tags::iostore::writer::NewPackage> = new_pkgs
            .iter()
            .map(|(template, bytes, package)| blam_tags::iostore::writer::NewPackage {
                template_uasset: template.as_slice(),
                tag_bytes: bytes.as_slice(),
                new_package_path: package.as_str(),
                redirect_from: None,
                // A new tag's wrapper is built from the template with the
                // donor's own bindings stripped; nothing here chooses one.
                asset_reference: None,
            },)
            .collect();
        let written =
            blam_tags::iostore::writer::write_mod_container_ex(&override_refs, &new_refs, &output);
        // Before anything reads through the mount again, and whether or not the
        // write worked: a released partition refuses every payload read.
        drop(override_refs);
        drop(overrides);
        let reopen_failures = self.restore_released_mappings(exporting, &released);
        match written {
            Ok(()) => {
                let stem = output
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("mod")
                    .to_owned();
                let sidecar = output.with_extension("baboon");
                // A sidecar written next to an exported mod may be replacing an
                // older one, and nothing here knows what is in it.
                if let Err(error) = save_campaign_project(&sidecar, snapshot, None) {
                    self.status =
                        format!("Exported {count} tag(s), but the .baboon sidecar failed: {error}");
                }
                // The sidecar travels with the mod; the workspace keeps its own
                // project, checkpointed here so it carries what the export did.
                let _ = self.checkpoint_campaign_project(exporting, 0.0);
                let directory = output.parent().map(Path::to_path_buf).unwrap_or_default();
                let in_place = self
                    .source()
                    .map(|source| source.source.root_path() == directory)
                    .unwrap_or(false);
                self.status = if !reopen_failures.is_empty() {
                    // The mod was written; what failed is picking it back up.
                    format!(
                        "Exported {count} tag(s) as {stem}, but remounting failed ({}) — reload \
                         the source",
                        reopen_failures.join("; ")
                    )
                } else if released.is_empty() {
                    format!("Exported {count} tag(s) as {stem}")
                } else {
                    // The container it replaced is mounted, so the browser is now
                    // showing the mod it just wrote. Anything that mod used to
                    // carry and no longer does still has an entry pointing at it.
                    format!(
                        "Exported {count} tag(s) as {stem}, replacing the mounted copy — reload \
                         the source if its tag list changed"
                    )
                };
                // Written straight into the game's own folder: there is nothing
                // to copy, so the instructions would only be noise.
                if !in_place {
                    self.exported_mod = Some(ExportedMod {
                        stem,
                        directory,
                        count,
                        skipped,
                    });
                }
            }
            Err(error) => self.status = format!("Export Mod failed: {error}"),
        }
    }

    pub(super) fn save_current_tag_as(&mut self) {
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "No tag selected".to_owned();
            return;
        };
        // For a container tag, "Save As" opens the rename dialog in duplicate
        // mode (new name, no reference redirect) and writes an override.
        if self.current_source_is_container() {
            self.open_container_duplicate(&key);
            return;
        }
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Selected tag is no longer in the source".to_owned();
            return;
        };
        let Some(doc) = self.kits[self.active].parsed_tags.get(&key) else {
            self.status = "Load the selected tag before saving".to_owned();
            return;
        };
        if let Some(reason) = unsaveable_reason(&entry, &doc.tag) {
            self.status = reason.to_owned();
            return;
        }

        let extension = save_as_extension(self, &entry);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save Current Tag As")
            .set_file_name(save_as_file_name(&entry, extension.as_deref()));
        if let Some(parent) = save_as_start_dir(&entry) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(extension) = extension.as_deref() {
            dialog = dialog.add_filter("Tag file", &[extension]);
        }
        let Some(mut output) = dialog.save_file() else {
            return;
        };
        if output.extension().is_none() {
            if let Some(extension) = extension.as_deref() {
                output.set_extension(extension);
            }
        }

        match doc.tag.write_atomic(&output) {
            Ok(()) => {
                self.status = match self.register_saved_copy_if_in_loaded_folder(&output) {
                    Ok(_) => format!("Saved copy to {}", output.display()),
                    Err(error) => format!(
                        "Saved copy to {}, but did not update browser: {error}",
                        output.display()
                    ),
                };
            }
            Err(error) => self.status = format!("Save As failed: {error}"),
        }
    }

    pub(super) fn fix_current_tag_dependencies(&mut self) {
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "No tag selected".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Selected tag is no longer in the source".to_owned();
            return;
        };
        let TagEntryLocation::LooseFile(_) = entry.location else {
            self.status = "Fix Tag Dependencies requires a loose-folder tag".to_owned();
            return;
        };
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Fix Tag Dependencies requires a loaded tags folder".to_owned();
            return;
        };

        let entries = match self.dependency_database_entries(&root) {
            Ok(entries) => entries,
            Err(error) => {
                self.status = format!("Could not build dependency database: {error}");
                return;
            }
        };
        let names = self.names().clone();
        let index = build_dependency_candidate_index(&entries, &names);
        let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&key) else {
            self.status = "Load the selected tag before fixing dependencies".to_owned();
            return;
        };
        if doc.tag.endian != Endian::Le {
            self.status = "Only little-endian loose tags can be edited".to_owned();
            return;
        }

        let report = fix_tag_dependencies_in_tag(&mut doc.tag, &root, &names, &index);
        if report.fixed > 0 {
            doc.dirty.touch();
        }
        let status = report.status();
        self.terminal
            .lines
            .extend(report.lines.into_iter().map(TerminalLineEntry::new));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
        self.status = status;
    }

    fn dependency_database_entries(&mut self, root: &Path) -> Result<Vec<TagEntry>, String> {
        let kit_index = self.active;
        if !matches!(
            self.kits[kit_index].source.as_ref().map(|s| &s.source),
            Some(TagSource::LooseFolder { .. })
        ) {
            return Err("load a loose editing-kit tags folder first".to_owned());
        }
        let entries = scan_folder_subtree_entries(root, Path::new(""), &self.kits[kit_index].names)
            .map_err(|error| error.to_string())?;
        let Some(source) = self.kits[kit_index].source.as_mut() else {
            return Err("no tag source is loaded".to_owned());
        };
        source.all_entries = entries;
        source.group_tree = crate::source::build_group_tree(&source.all_entries);
        if let Some(game) = source.game.as_deref() {
            let _ = crate::source::save_entry_index(game, root, &source.all_entries);
        }
        Ok(source.all_entries.clone())
    }

    /// Tags that reference `entry` (its "parents"), via the reverse-dependency
    /// index. `None` when no index is available (non-folder source or not yet
    /// scanned).
    /// Open the rename/move dialog for a tag, pre-listing the tags that
    /// reference it (which will be rewritten on apply).
    pub(super) fn open_rename_tag(&mut self, key: &str) {
        self.open_rename_or_duplicate(key, true);
    }

    /// Open the rename dialog in "duplicate" mode (Save As for a container tag —
    /// writes a new tag with no redirect). For a tag that has never been saved
    /// there is nothing to write yet, so the copy is made in memory and both
    /// tags stay unsaved until Save/Export Mod.
    pub(super) fn open_container_duplicate(&mut self, key: &str) {
        self.open_rename_or_duplicate(key, false);
    }

    fn open_rename_or_duplicate(&mut self, key: &str, redirect: bool) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return;
        };
        let is_new_container = matches!(entry.location, TagEntryLocation::NewContainer { .. });
        let is_container =
            is_new_container || matches!(entry.location, TagEntryLocation::Container { .. });
        if !is_container && !matches!(entry.location, TagEntryLocation::LooseFile(_)) {
            self.status = "Only loose-folder or container tags can be renamed".to_owned();
            return;
        }
        let display = entry.display_path.replace('\\', "/");
        let (stem, extension) = match display.rsplit_once('.') {
            Some((stem, ext)) => (stem.to_owned(), ext.to_owned()),
            None => (display.clone(), String::new()),
        };
        // A new tag edits its whole path (rename and move are the same in-memory
        // operation for it); everything else edits the leaf name only.
        let name = if is_new_container {
            stem.clone()
        } else {
            stem.rsplit(['/', '\\']).next().unwrap_or(&stem).to_owned()
        };
        let (referrers, referrers_unavailable) = match self.references_to_entry(&entry) {
            Some(list) => (
                list.iter()
                    .map(|e| e.display_path.replace('\\', "/"))
                    .collect(),
                false,
            ),
            None => (Vec::new(), true),
        };
        self.rename_tag = Some(RenameTagState {
            kit: self.active_kit_id(),
            key: entry.key.clone(),
            old_display: display,
            extension,
            new_path_input: name,
            referrers,
            referrers_unavailable,
            is_container,
            redirect,
            is_new_container,
        });
    }

    /// Apply the rename/move: move the file on disk and rewrite every
    /// referencing tag, in the background (reuses the folder-refactor pipeline).
    pub(super) fn begin_rename_tag(&mut self) {
        // Everything below resolves against the active kit's tags root or
        // container set, so return to the workspace the dialog was opened for.
        // A closed workspace drops the rename rather than moving a file in
        // whichever game is focused now.
        let Some(kit) = self.rename_tag.as_ref().map(|state| state.kit) else {
            return;
        };
        if !self.focus_navigation_kit(kit) {
            self.rename_tag = None;
            self.status = "The workspace this rename came from is closed".to_owned();
            return;
        }
        let Some((key, old_display, new_name_raw, is_container, redirect, is_new_container)) =
            self.rename_tag.as_ref().map(|s| {
                (
                    s.key.clone(),
                    s.old_display.clone(),
                    s.new_path_input.clone(),
                    s.is_container,
                    s.redirect,
                    s.is_new_container,
                )
            })
        else {
            return;
        };
        let new_name = new_name_raw.trim().to_owned();
        if new_name.is_empty() {
            self.status = "Enter a new tag name".to_owned();
            return;
        }
        // A new tag's whole path is editable, so a separator is the move half of
        // the operation rather than a mistake.
        if !is_new_container && new_name.contains(['/', '\\']) {
            self.status = "Enter a name only; use Move to choose a folder".to_owned();
            return;
        }
        if new_name.contains('.') {
            self.status = "Enter a name without an extension".to_owned();
            return;
        }
        let new_rel = if is_new_container {
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
            self.rename_tag = None;
            match self.apply_new_container_rename(&key, &new_rel, !redirect) {
                Ok(message) => self.status = message,
                Err(error) => self.status = error,
            }
            return;
        }

        // Container tags: write an override container (rename adds a redirect,
        // duplicate does not) instead of moving a loose file.
        if is_container {
            self.rename_tag = None;
            match self.export_container_override(&key, Some((new_rel, redirect))) {
                Ok(Some(path)) => {
                    let what = if redirect { "renamed tag" } else { "tag copy" };
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("mod");
                    self.status = format!(
                        "Exported {what} → {stem}.utoc/.ucas/.pak — copy all three into \
                         Meteorite/Content/Paks/ (base game unchanged)"
                    );
                }
                Ok(None) => {}
                Err(e) => self.status = format!("Export failed: {e}"),
            }
            return;
        }

        // Loose folder: move the file on disk + rewrite references.
        if self.folder_refactor.is_some() {
            self.status = "A move/rename is already running".to_owned();
            return;
        }
        if self.kits[self.active]
            .parsed_tags
            .values()
            .any(|doc| doc.dirty.is_set())
        {
            self.status = "Save or close dirty tags before renaming".to_owned();
            return;
        }
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Rename requires a loaded tags folder".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Tag no longer exists".to_owned();
            return;
        };
        self.rename_tag = None;
        self.start_tag_rename_job(root, entry, new_rel, "Renaming tag");
    }

    /// Starts a filesystem refactoring transaction from a captured source snapshot.
    /// Progress and the final replacement tree are applied only through worker messages.
    pub(super) fn begin_move_tag(&mut self, key: &str) {
        // A new tag has no file to move and no folder to browse to: its whole
        // container-relative path is editable in the rename dialog, so Move and
        // Rename are the same in-memory operation for it.
        if matches!(
            self.entry_for_key(key).map(|entry| &entry.location),
            Some(TagEntryLocation::NewContainer { .. })
        ) {
            self.open_rename_tag(key);
            return;
        }
        if self.folder_refactor.is_some() {
            self.status = "A move/rename is already running".to_owned();
            return;
        }
        if self.kits[self.active]
            .parsed_tags
            .values()
            .any(|doc| doc.dirty.is_set())
        {
            self.status = "Save or close dirty tags before moving".to_owned();
            return;
        }
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Move requires a loaded tags folder".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag no longer exists".to_owned();
            return;
        };
        if !matches!(entry.location, TagEntryLocation::LooseFile(_)) {
            self.status = "Only loose-folder tags can be moved".to_owned();
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
            self.status = "Choose a destination inside the loaded tags folder".to_owned();
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

    fn start_tag_rename_job(
        &mut self,
        root: PathBuf,
        entry: TagEntry,
        new_rel: String,
        job_label: &str,
    ) {
        let names = self.names().clone();
        let game = self.source().and_then(|source| source.game.clone());
        let all_entries = self
            .source()
            .map(|source| source.all_entries.clone())
            .unwrap_or_default();
        let reverse_dependencies = self
            .source()
            .and_then(|source| source.reverse_dependencies.clone());
        // Routed back to the kit the refactor was started in, not
        // whichever one is focused when it lands.
        let stamp = self.kit_stamp();
        let tx = self.tx.clone();
        let job_label = job_label.to_owned();
        self.folder_refactor = Some(FolderRefactorUiState {
            label: job_label.clone(),
            phase: "Preparing".to_owned(),
            progress: None,
        });
        self.status = format!("{job_label}: Preparing");
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_tag_rename_job(
                    root,
                    entry,
                    new_rel,
                    job_label,
                    names,
                    game,
                    all_entries,
                    reverse_dependencies,
                    &tx,
                )
            }))
            .unwrap_or_else(|_| Err("Tag move worker crashed".to_owned()));
            let _ = tx.send(WorkerMessage::FolderRefactorFinished { stamp, result });
        });
    }

    pub(super) fn references_to_entry(&self, entry: &TagEntry) -> Option<Vec<TagEntry>> {
        let source = self.source()?;
        let index = source.reverse_dependencies.as_ref()?;
        let rel = dependency_entry_reference_path(entry, self.names())?;
        let referrer_keys = index
            .dependents_for(entry.group_tag, &rel)
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut out: Vec<TagEntry> = source
            .full_entry_set()
            .iter()
            .filter(|entry| referrer_keys.contains(entry.key.as_str()))
            .cloned()
            .collect();
        out.sort_by(|a, b| natural_entry_order(a).cmp(&natural_entry_order(b)));
        Some(out)
    }

    /// All tags that nothing references (orphans / roots). `None` when no index
    /// is available.
    pub(super) fn unreferenced_entries(&self) -> Option<Vec<TagEntry>> {
        let source = self.source()?;
        let index = source.reverse_dependencies.as_ref()?;
        let mut out: Vec<TagEntry> = source
            .full_entry_set()
            .iter()
            .filter(|entry| {
                dependency_entry_reference_path(entry, self.names())
                    .map(|rel| index.dependents_for(entry.group_tag, &rel).is_empty())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| natural_entry_order(a).cmp(&natural_entry_order(b)));
        Some(out)
    }

    /// Resolve the dependencies a tag declares (children) into browseable
    /// entries, via a one-shot dependency-key → entry lookup over all entries.
    fn children_of_entry(&self, key: &str) -> (Vec<TagEntry>, bool) {
        let Some(source) = self.source() else {
            return (Vec::new(), true);
        };
        let Some(index) = source.reverse_dependencies.as_ref() else {
            return (Vec::new(), true);
        };
        let deps = index.dependencies_of(key);
        let mut by_key: HashMap<String, &TagEntry> = HashMap::new();
        for entry in source.full_entry_set() {
            if let Some(rel) = dependency_entry_reference_path(entry, self.names()) {
                by_key
                    .entry(crate::source::dependency_key(entry.group_tag, &rel))
                    .or_insert(entry);
            }
        }
        let mut children: Vec<TagEntry> = deps
            .iter()
            .filter_map(|dep| {
                by_key
                    .get(&crate::source::dependency_key(dep.group_tag, &dep.rel_path))
                    .map(|entry| (*entry).clone())
            })
            .collect();
        children.sort_by(|a, b| natural_entry_order(a).cmp(&natural_entry_order(b)));
        children.dedup_by(|a, b| a.key == b.key);
        (children, false)
    }

    /// Open the Content Explorer centered on `key`.
    pub(super) fn open_content_explorer(&mut self, key: &str) {
        let Some(focus) = self.entry_for_key(key).cloned() else {
            return;
        };
        let (parents, parents_unavailable) = match self.references_to_entry(&focus) {
            Some(parents) => (parents, false),
            None => (Vec::new(), true),
        };
        let (children, children_unavailable) = self.children_of_entry(key);
        self.content_explorer = Some(ContentExplorer {
            kit: self.active_kit_id(),
            focus,
            parents,
            children,
            filter: String::new(),
            index_unavailable: parents_unavailable && children_unavailable,
            back: Vec::new(),
            forward: Vec::new(),
        });
    }

    /// Re-center the open Content Explorer on `entry`, recording history.
    pub(super) fn content_explorer_navigate(&mut self, entry: TagEntry) {
        let key = entry.key.clone();
        let (parents, parents_unavailable) = match self.references_to_entry(&entry) {
            Some(parents) => (parents, false),
            None => (Vec::new(), true),
        };
        let (children, children_unavailable) = self.children_of_entry(&key);
        if let Some(explorer) = self.content_explorer.as_mut() {
            explorer.back.push(explorer.focus.clone());
            explorer.forward.clear();
            explorer.focus = entry;
            explorer.parents = parents;
            explorer.children = children;
            explorer.index_unavailable = parents_unavailable && children_unavailable;
        }
    }

    pub(super) fn content_explorer_back(&mut self) {
        let Some(prev) = self
            .content_explorer
            .as_mut()
            .and_then(|explorer| explorer.back.pop())
        else {
            return;
        };
        self.recenter_explorer(prev, true);
    }

    pub(super) fn content_explorer_forward(&mut self) {
        let Some(next) = self
            .content_explorer
            .as_mut()
            .and_then(|explorer| explorer.forward.pop())
        else {
            return;
        };
        self.recenter_explorer(next, false);
    }

    /// Re-center without clearing history; pushes the current focus onto the
    /// opposite stack (used by back/forward).
    fn recenter_explorer(&mut self, entry: TagEntry, going_back: bool) {
        let key = entry.key.clone();
        let (parents, parents_unavailable) = match self.references_to_entry(&entry) {
            Some(parents) => (parents, false),
            None => (Vec::new(), true),
        };
        let (children, children_unavailable) = self.children_of_entry(&key);
        if let Some(explorer) = self.content_explorer.as_mut() {
            let current = std::mem::replace(&mut explorer.focus, entry);
            if going_back {
                explorer.forward.push(current);
            } else {
                explorer.back.push(current);
            }
            explorer.parents = parents;
            explorer.children = children;
            explorer.index_unavailable = parents_unavailable && children_unavailable;
        }
    }

    pub(super) fn show_references_for(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return;
        };
        // Fresh query — drop any expander state from a previous references popup.
        self.ref_jump_expanded.clear();
        self.ref_jump_occurrences.clear();
        let title = format!("References to {}", entry.display_path.replace('\\', "/"));
        // The referenced tag's dependency path, so a clicked row can jump to the
        // exact field that points here.
        let ref_target =
            dependency_entry_reference_path(&entry, self.names()).map(|rel| (entry.group_tag, rel));
        match self.references_to_entry(&entry) {
            Some(entries) => {
                let note = entries
                    .is_empty()
                    .then(|| "No other tags reference this tag.".to_owned());
                self.query_results = Some(TagQueryResults {
                    kit: self.active_kit_id(),
                    title,
                    entries,
                    annotations: Vec::new(),
                    note,
                    ref_target,
                });
            }
            None => {
                self.query_results = Some(TagQueryResults {
                    kit: self.active_kit_id(),
                    title,
                    entries: Vec::new(),
                    annotations: Vec::new(),
                    note: Some(self.reference_index_unavailable_note()),
                    ref_target: None,
                });
            }
        }
    }

    /// Once-per-frame driver for reference-jumps. Expires a finished glow, and —
    /// when a pending jump's referrer tag has become the focused, parsed tab —
    /// walks it for the exact field referencing the target and navigates there.
    pub(super) fn apply_field_nav(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        if let Some(nav) = &self.field_nav {
            if now >= nav.glow_until {
                self.field_nav = None;
            } else {
                // Keep frames coming so the glow expires on time even when idle.
                ctx.request_repaint();
            }
        }
        // A glow belongs to the kit whose tag it is; drop it once that kit is
        // gone rather than glowing a field in another game.
        if let Some(nav) = &self.field_nav
            && self.kit_index(nav.kit).is_none()
        {
            self.field_nav = None;
        }
        if let Some(hit) = self.pending_find_jump.clone() {
            if self.kits[self.active].selected_key.as_deref() == Some(hit.tag_key.as_str())
                && self.kits[self.active]
                    .parsed_tags
                    .contains_key(&hit.tag_key)
            {
                self.activate_find_occurrence(ctx, hit);
            }
        }
        let Some(jump) = self.pending_ref_jump.clone() else {
            return;
        };
        // The jump belongs to the kit it was queued from; if that kit closed
        // while the referrer was loading, drop it.
        let Some(kit) = self.kit_index(jump.kit) else {
            self.pending_ref_jump = None;
            return;
        };
        // Wait until the referrer is the focused tab and finished loading.
        if self.kits[kit].selected_key.as_deref() != Some(jump.tag_key.as_str()) {
            return;
        }
        let Some(doc) = self.kits[kit].parsed_tags.get(&jump.tag_key) else {
            return; // still loading — retry next frame
        };
        let mut refs = Vec::new();
        collect_tag_references(doc.tag.root(), "", &mut refs);
        let target = normalize_ref(&jump.rel_path);
        let hit = refs.into_iter().find(|reference| {
            reference.group_tag == jump.group_tag && normalize_ref(&reference.rel_path) == target
        });
        self.pending_ref_jump = None;
        match hit {
            Some(reference) => self.navigate_to_field(ctx, &jump.tag_key, &reference.field_path),
            None => {
                self.status = format!(
                    "Could not locate the referencing field in {}",
                    jump.tag_key.replace('\\', "/")
                );
            }
        }
    }

    /// Drive the editor to reveal `field_path` in the tag `tag_key`: select the
    /// element index at every ancestor block, scroll the exact leaf into view,
    /// and glow it briefly. Element selection and scroll targets are written once
    /// via egui temp-data; the glow/force-open persist via `self.field_nav`.
    pub(super) fn navigate_to_field(
        &mut self,
        ctx: &egui::Context,
        tag_key: &str,
        field_path: &str,
    ) {
        // Select the referenced element at each ancestor block level. The block's
        // selection is keyed by view-scope; write both so it lands whether the tab
        // is docked or floating.
        for (block_path, index) in ancestor_block_indices(field_path) {
            for scope in ["docked", "floating"] {
                let id = egui::Id::new((
                    "field_edit",
                    scope,
                    tag_key,
                    ("block_sel", block_path.as_str()),
                ));
                ctx.data_mut(|data| data.insert_temp(id, index));
            }
        }
        // Scroll the exact leaf field into view next frame, plus the enclosing
        // block header as a fallback for non-value leaves.
        ctx.data_mut(|data| data.insert_temp(field_jump_target_id(), field_path.to_owned()));
        if let Some(block) = parent_block_path(field_path) {
            ctx.data_mut(|data| data.insert_temp(jump_target_id(), block));
        }
        self.field_nav = Some(FieldNav {
            kit: self.active_kit_id(),
            tag_key: tag_key.to_owned(),
            field_path: field_path.to_owned(),
            glow_until: ctx.input(|input| input.time) + 2.5,
        });
        ctx.request_repaint();
    }

    /// Populate `ref_jump_occurrences` for any expanded, uncached referrer row in
    /// the current "References to X" popup. Parsed referrers are walked in place;
    /// unparsed ones trigger a background load and stay uncached ("loading…").
    pub(super) fn refresh_ref_jump_occurrences(&mut self, ctx: &egui::Context) {
        let Some((group_tag, rel_path)) = self
            .query_results
            .as_ref()
            .and_then(|results| results.ref_target.clone())
        else {
            return;
        };
        // Snapshot (row, key) for expanded-but-uncached rows before borrowing
        // `parsed_tags` / triggering loads.
        let pending: Vec<(usize, String)> = self
            .query_results
            .as_ref()
            .map(|results| {
                self.ref_jump_expanded
                    .iter()
                    .filter(|index| !self.ref_jump_occurrences.contains_key(index))
                    .filter_map(|&index| {
                        results
                            .entries
                            .get(index)
                            .map(|entry| (index, entry.key.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let target = normalize_ref(&rel_path);
        for (index, key) in pending {
            match self.kits[self.active].parsed_tags.get(&key) {
                Some(doc) => {
                    let mut refs = Vec::new();
                    collect_tag_references(doc.tag.root(), "", &mut refs);
                    let occurrences = refs
                        .into_iter()
                        .filter(|reference| {
                            reference.group_tag == group_tag
                                && normalize_ref(&reference.rel_path) == target
                        })
                        .map(|reference| RefOccurrence {
                            label: occurrence_label(&reference.field_path),
                            field_path: reference.field_path,
                        })
                        .collect();
                    self.ref_jump_occurrences.insert(index, occurrences);
                }
                None => self.ensure_tag_loading(key.clone(), ctx.clone()),
            }
        }
    }

    /// Explain why a reference lookup found no index, tailored to whether one is
    /// currently building (auto after the full scan, or via Tools → Build
    /// Reference Index).
    fn reference_index_unavailable_note(&self) -> String {
        if self.building_reverse_dependencies || self.kits[self.active].scanning_entries {
            "Reference index is building — try again in a moment.".to_owned()
        } else {
            "Reference index unavailable — run Tools → Build Reference Index.".to_owned()
        }
    }

    pub(super) fn show_unreferenced_tags(&mut self) {
        match self.unreferenced_entries() {
            Some(entries) => {
                let note = entries
                    .is_empty()
                    .then(|| "Every tag is referenced by at least one other tag.".to_owned());
                self.query_results = Some(TagQueryResults {
                    kit: self.active_kit_id(),
                    title: format!("Unreferenced tags ({})", entries.len()),
                    entries,
                    annotations: Vec::new(),
                    note,
                    ref_target: None,
                });
            }
            None => {
                self.query_results = Some(TagQueryResults {
                    kit: self.active_kit_id(),
                    title: "Unreferenced tags".to_owned(),
                    entries: Vec::new(),
                    annotations: Vec::new(),
                    note: Some(self.reference_index_unavailable_note()),
                    ref_target: None,
                });
            }
        }
    }

    /// Scan every scenario (`scnr`) tag and list its map id (+ map name where
    /// present). Reads `map id` at the scenario root, which covers the modern
    /// engines (H2A/H3/ODST/Reach/H4); classic Halo 2 stores it elsewhere.
    pub(super) fn show_map_ids(&mut self) {
        let Some(source) = self.source() else {
            self.query_results = Some(TagQueryResults {
                kit: self.active_kit_id(),
                title: "Scenario map IDs".to_owned(),
                entries: Vec::new(),
                annotations: Vec::new(),
                note: Some("No source loaded.".to_owned()),
                ref_target: None,
            });
            return;
        };
        let mut entries = Vec::new();
        let mut annotations = Vec::new();
        for entry in &source.all_entries {
            if &entry.group_tag.to_be_bytes() != b"scnr" {
                continue;
            }
            let Ok(tag) = crate::source::read_entry(&source.source, entry) else {
                continue;
            };
            let root = tag.root();
            if let Some(id) = root.read_int_any("map id") {
                // `map name` carries a `#tooltip` suffix in Reach/H4, so resolve
                // it via the cleaned-name lookup rather than an exact match.
                let name = find_full_field_name(&root, "map name")
                    .and_then(|full| root.read_string_id(full))
                    .unwrap_or_default();
                annotations.push(if name.is_empty() {
                    format!("map id {id}")
                } else {
                    format!("map id {id}  ({name})")
                });
                entries.push(entry.clone());
            }
        }
        let note = entries.is_empty().then(|| {
            "No scenario map IDs found (scnr tags only; classic Halo 2 stores them elsewhere)."
                .to_owned()
        });
        self.query_results = Some(TagQueryResults {
            kit: self.active_kit_id(),
            title: format!("Scenario map IDs ({})", entries.len()),
            entries,
            annotations,
            note,
            ref_target: None,
        });
    }

    /// Scan every `snd!` tag once, reading its `sound class` + `compression`
    /// enum names. Shared by the class-listing and uncompressed-listing tools.
    /// Returns `(class, compression, entry)` triples, or `None` if no source.
    fn scan_sound_tags(&self) -> Option<Vec<(String, String, TagEntry)>> {
        let source = self.source()?;
        let mut rows = Vec::new();
        for entry in &source.all_entries {
            if &entry.group_tag.to_be_bytes() != b"snd!" {
                continue;
            }
            let Ok(tag) = crate::source::read_entry(&source.source, entry) else {
                continue;
            };
            let root = tag.root();
            let class = find_full_field_name(&root, "sound class")
                .and_then(|full| root.read_enum_name(full))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "(none)".to_owned());
            let compression = find_full_field_name(&root, "compression")
                .and_then(|full| root.read_enum_name(full))
                .unwrap_or_default();
            rows.push((class, compression, entry.clone()));
        }
        Some(rows)
    }

    /// List every `snd!` tag annotated with its sound class + compression, with a
    /// per-class count summary (mirrors `count-class-sounds` /
    /// `count-all-class-sounds`).
    pub(super) fn show_sounds_by_class(&mut self) {
        let title = "Sounds by class";
        let Some(mut rows) = self.scan_sound_tags() else {
            self.query_results = Some(TagQueryResults {
                kit: self.active_kit_id(),
                title: title.to_owned(),
                entries: Vec::new(),
                annotations: Vec::new(),
                note: Some("No source loaded.".to_owned()),
                ref_target: None,
            });
            return;
        };
        rows.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.2.display_path.cmp(&b.2.display_path))
        });
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for (class, _, _) in &rows {
            *counts.entry(class.as_str()).or_default() += 1;
        }
        let entries: Vec<TagEntry> = rows.iter().map(|(_, _, e)| e.clone()).collect();
        let annotations: Vec<String> = rows
            .iter()
            .map(|(class, comp, _)| {
                if comp.is_empty() {
                    format!("[{class}]")
                } else {
                    format!("[{class}] {comp}")
                }
            })
            .collect();
        let note = if entries.is_empty() {
            Some("No sound tags found.".to_owned())
        } else {
            let summary = counts
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!("{} class(es) \u{2014} {summary}", counts.len()))
        };
        self.query_results = Some(TagQueryResults {
            kit: self.active_kit_id(),
            title: format!("{title} ({})", entries.len()),
            entries,
            annotations,
            note,
            ref_target: None,
        });
    }

    /// List `snd!` tags stored uncompressed (compression name contains "none"),
    /// mirroring `dump-uncompressed-sounds`.
    pub(super) fn show_uncompressed_sounds(&mut self) {
        let title = "Uncompressed sounds";
        let Some(rows) = self.scan_sound_tags() else {
            self.query_results = Some(TagQueryResults {
                kit: self.active_kit_id(),
                title: title.to_owned(),
                entries: Vec::new(),
                annotations: Vec::new(),
                note: Some("No source loaded.".to_owned()),
                ref_target: None,
            });
            return;
        };
        let mut hits: Vec<(String, String, TagEntry)> = rows
            .into_iter()
            .filter(|(_, comp, _)| comp.to_ascii_lowercase().contains("none"))
            .collect();
        hits.sort_by(|a, b| a.2.display_path.cmp(&b.2.display_path));
        let entries: Vec<TagEntry> = hits.iter().map(|(_, _, e)| e.clone()).collect();
        let annotations: Vec<String> = hits
            .iter()
            .map(|(class, comp, _)| format!("{comp}  [{class}]"))
            .collect();
        let note = entries
            .is_empty()
            .then(|| "No uncompressed sound tags found.".to_owned());
        self.query_results = Some(TagQueryResults {
            kit: self.active_kit_id(),
            title: format!("{title} ({})", entries.len()),
            entries,
            annotations,
            note,
            ref_target: None,
        });
    }

    /// Locate a tag in the browser tree: switch to Folders mode, clear the
    /// filter, select it, and request a one-shot force-open + scroll.
    pub(super) fn reveal_in_browser(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return;
        };
        self.kits[self.active].filter.clear();
        self.kits[self.active].browser_mode = BrowserMode::Folders;
        self.kits[self.active].selected_key = Some(entry.key.clone());
        self.reveal_target = Some(RevealRequest {
            kit: self.active_kit_id(),
            key: entry.key.clone(),
            ancestors: browser::ancestor_labels(&entry.display_path),
        });
    }

    /// Run a field-value search for the current query. If the in-memory index is
    /// ready it answers instantly from cache; otherwise it kicks off a live
    /// background scan (correct, slower) and builds the index for next time.
    /// Starts source-scoped indexing or search work without blocking the UI thread.
    /// Generation-tagged completion is ignored if the active source changes first.
    pub(super) fn begin_field_value_search(&mut self, ctx: egui::Context) {
        let display = self.field_value_query.trim().to_owned();
        if display.is_empty() {
            return;
        }
        let query_lower = display.to_ascii_lowercase();
        let group_filter = self.field_value_group.trim().to_ascii_lowercase();
        let stamp = self.kit_stamp();

        // Fast path: answer from the cached index.
        if self.kits[self.active]
            .field_index
            .is_ready_for(stamp.generation)
        {
            // Over-fetch when group-filtering so the cap applies post-filter.
            let raw_cap = if group_filter.is_empty() { 1000 } else { 8000 };
            let hits = self.kits[self.active]
                .field_index
                .query(&query_lower, raw_cap);
            let mut entries = Vec::new();
            let mut annotations = Vec::new();
            for (key, snippet) in hits {
                if let Some(entry) = self.entry_for_key(&key).cloned() {
                    if !group_filter.is_empty()
                        && !self.group_label_matches(entry.group_tag, &group_filter)
                    {
                        continue;
                    }
                    entries.push(entry);
                    annotations.push(snippet);
                    if entries.len() >= 1000 {
                        break;
                    }
                }
            }
            let note = entries
                .is_empty()
                .then(|| format!("No tag field values contain \"{display}\"."));
            self.status = format!(
                "Field search for \"{display}\": {} match(es) (indexed)",
                entries.len()
            );
            self.query_results = Some(TagQueryResults {
                kit: self.active_kit_id(),
                title: format!("Field value '{display}' ({})", entries.len()),
                entries,
                annotations,
                note,
                ref_target: None,
            });
            return;
        }

        if self.source().is_none() {
            return;
        }
        let base_entries: Vec<TagEntry> = {
            let source = self.source().expect("checked");
            if source.all_entries.is_empty() {
                source.entries.clone()
            } else {
                source.all_entries.clone()
            }
        };
        let entries: Vec<TagEntry> = if group_filter.is_empty() {
            base_entries
        } else {
            base_entries
                .into_iter()
                .filter(|entry| self.group_label_matches(entry.group_tag, &group_filter))
                .collect()
        };
        let tag_source = self.source().expect("checked").source.clone();
        let tx = self.tx.clone();
        self.field_value_searching = true;
        self.status = format!("Searching field values for \"{display}\"…");
        let search_ctx = ctx.clone();
        thread::spawn(move || {
            let result = run_field_value_search(&tag_source, &entries, &query_lower);
            let _ = tx.send(WorkerMessage::FieldValueSearchFinished {
                stamp,
                query: display,
                result,
            });
            search_ctx.request_repaint();
        });
        // Build the index in the background so the next search is instant.
        self.begin_build_field_index(ctx);
    }

    /// Whether a group matches a (lowercased) group filter — by four-CC or by a
    /// substring of the group's name/extension (e.g. "weap" or "weapon").
    fn group_label_matches(&self, group_tag: u32, filter_lower: &str) -> bool {
        if format_group_tag(group_tag).to_ascii_lowercase() == filter_lower {
            return true;
        }
        self.names()
            .name_for(group_tag)
            .or_else(|| group_tag_to_extension(group_tag))
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(filter_lower)
    }

    /// Build the in-memory searchable-text index in the background (idempotent —
    /// skips if already ready for this generation or already building).
    /// Starts source-scoped indexing or search work without blocking the UI thread.
    /// Generation-tagged completion is ignored if the active source changes first.
    pub(super) fn begin_build_field_index(&mut self, ctx: egui::Context) {
        let stamp = self.kit_stamp();
        if self.kits[self.active]
            .field_index
            .is_ready_for(stamp.generation)
            || self.kits[self.active].field_index.is_building()
        {
            return;
        }
        let Some(source) = self.source() else {
            return;
        };
        let entries: Vec<TagEntry> = if source.all_entries.is_empty() {
            source.entries.clone()
        } else {
            source.all_entries.clone()
        };
        let tag_source = source.source.clone();
        let tx = self.tx.clone();
        self.kits[self.active].field_index.mark_building();
        thread::spawn(move || {
            let blobs = build_field_value_index(&tag_source, &entries);
            let _ = tx.send(WorkerMessage::FieldIndexBuilt { stamp, blobs });
            ctx.request_repaint();
        });
    }

    /// Build the reverse-dependency index in the background so the
    /// find-references / unreferenced / Content Explorer features work without
    /// first running a move/rename. Idempotent: skips while a build is running,
    /// and skips an already-present index unless `force` is set (Tools →
    /// Rebuild). Loose-folder sources only; the result is persisted to disk so
    /// future launches load it instantly.
    /// Starts source-scoped indexing or search work without blocking the UI thread.
    /// Generation-tagged completion is ignored if the active source changes first.
    pub(super) fn begin_build_reverse_dependencies(&mut self, ctx: egui::Context, force: bool) {
        self.begin_build_reverse_dependencies_inner(ctx, force, false);
    }

    fn begin_build_reverse_dependencies_for_entry_index(&mut self, ctx: egui::Context) {
        self.begin_build_reverse_dependencies_inner(ctx, false, true);
    }

    fn begin_build_reverse_dependencies_inner(
        &mut self,
        ctx: egui::Context,
        force: bool,
        paired_entry_index_build: bool,
    ) {
        if self.building_reverse_dependencies || self.kits[self.active].scanning_entries {
            return;
        }
        let Some(source) = self.source() else {
            return;
        };
        // Loose folders index automatically after their scan. Containers are
        // indexable too, but only on request (Tools → Build Reference Index):
        // container tags carry no dependency-list stream, so every tag has to be
        // parsed — for Campaign Evolved that is ~12k tags and several GB of
        // reads, too much to run behind every mount.
        let is_loose = matches!(source.source, TagSource::LooseFolder { .. });
        if !is_loose && !matches!(source.source, TagSource::IoStoreContainerSet { .. }) {
            return;
        }
        if source.reverse_dependencies.is_some() && !force {
            return;
        }
        // A container mount enumerates every tag up front. A loose folder must
        // use its completed scan only: an index built from the lazy browser
        // subset would be wrong (it would flag tags as unreferenced just because
        // their referrers weren't scanned).
        let entries = if is_loose {
            source.all_entries.clone()
        } else {
            source.full_entry_set().to_vec()
        };
        if entries.is_empty() {
            // The full entry set isn't ready yet, so kick the scan first.
            // `begin_scan_all_entries` is idempotent (guards on
            // `scanning_entries`); the update loop re-enters here and builds the
            // index once the scan lands. Containers have nothing to scan — an
            // empty mount simply has nothing to index.
            if is_loose {
                if !self.kits[self.active].scanning_entries {
                    self.status = "Indexing tags, then building reference index…".to_owned();
                }
                self.begin_scan_all_entries_with_label(
                    ctx,
                    "Indexing tags, then building reference index...",
                );
            }
            return;
        }
        let tag_source = source.source.clone();
        let stamp = self.kit_stamp();
        let tx = self.tx.clone();
        self.building_reverse_dependencies = true;
        self.building_reference_for_entry_index = paired_entry_index_build;
        self.reference_index_progress = Some(ReferenceIndexProgressState {
            label: "Building reference index...".to_owned(),
            processed: 0,
            total: entries.len(),
        });
        if paired_entry_index_build {
            self.show_entry_index_wait_notice = true;
        }
        self.status = "Building reference index…".to_owned();
        thread::spawn(move || {
            let total = entries.len();
            let _ = tx.send(WorkerMessage::ReferenceIndexProgress {
                stamp,
                processed: 0,
                total,
            });
            let worker_count = std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1)
                .clamp(1, total.max(1));
            let chunk_size = total.div_ceil(worker_count).max(1);
            let processed = std::sync::atomic::AtomicUsize::new(0);

            let mut index = ReverseDependencyIndex::default();
            std::thread::scope(|scope| {
                let mut handles = Vec::new();
                for chunk in entries.chunks(chunk_size) {
                    let tag_source = &tag_source;
                    let progress_tx = tx.clone();
                    let progress_ctx = ctx.clone();
                    let processed = &processed;
                    handles.push(scope.spawn(move || {
                        let mut chunk_results = Vec::new();
                        for entry in chunk {
                            if let Ok(deps) = read_entry_dependencies(tag_source, entry) {
                                chunk_results.push((entry.key.clone(), deps));
                            }
                            let processed_now =
                                processed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            if processed_now == total || processed_now % 32 == 0 {
                                let _ = progress_tx.send(WorkerMessage::ReferenceIndexProgress {
                                    stamp,
                                    processed: processed_now,
                                    total,
                                });
                                progress_ctx.request_repaint();
                            }
                        }
                        chunk_results
                    }));
                }

                for handle in handles {
                    if let Ok(chunk_results) = handle.join() {
                        for (key, deps) in chunk_results {
                            index.set_tag_dependencies(key, deps);
                        }
                    }
                }
            });
            let _ = tx.send(WorkerMessage::ReverseDependenciesBuilt { stamp, index });
            ctx.request_repaint();
        });
    }

    /// Apply pasted TSV (header row = field names, one data row per element) to
    /// the target block's EXISTING elements, cell-by-cell via `apply_field_edit`.
    /// Rows beyond the current element count are reported and ignored (no
    /// structural changes — fully covered by undo). Returns a status summary.
    pub(super) fn apply_tsv_paste(&mut self) {
        // The document is looked up in the active kit below, and two workspaces
        // of the same game share a key space, so a paste answered after a
        // switch could land in the wrong game's tag rather than simply missing.
        let Some(kit) = self.tsv_paste.as_ref().map(|paste| paste.kit) else {
            return;
        };
        if !self.focus_navigation_kit(kit) {
            self.set_tsv_paste_status("The workspace this paste came from is closed.");
            return;
        }
        let Some(paste) = self.tsv_paste.as_ref() else {
            return;
        };
        let tag_key = paste.tag_key.clone();
        let block_path = paste.block_path.clone();
        let text = paste.text.clone();

        let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&tag_key) else {
            self.set_tsv_paste_status("Tag is no longer open.");
            return;
        };
        let Some(block) = doc
            .tag
            .root()
            .field_path(&block_path)
            .and_then(|field| field.as_block())
        else {
            self.set_tsv_paste_status("Block no longer resolves in this tag.");
            return;
        };
        let element_count = block.len();
        let columns = block_leaf_columns(&block); // (clean, full) per leaf field

        let mut lines = text.lines();
        let Some(header_line) = lines.next() else {
            self.set_tsv_paste_status("Nothing to paste.");
            return;
        };
        // Map each pasted column index → the full field name to write.
        let header_to_full = map_tsv_header_to_fields(header_line, &columns);
        if header_to_full.iter().all(Option::is_none) {
            self.set_tsv_paste_status("No pasted column headers matched this block's fields.");
            return;
        }

        let mut edits = Vec::new();
        let mut data_rows = 0usize;
        let mut skipped_rows = 0usize;
        for (row_index, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            data_rows += 1;
            if row_index >= element_count {
                skipped_rows += 1;
                continue;
            }
            for (col_index, cell) in line.split('\t').enumerate() {
                if let Some(Some(full)) = header_to_full.get(col_index) {
                    edits.push(PendingFieldEdit {
                        path: format!("{block_path}[{row_index}]/{full}"),
                        input: cell.trim().to_owned(),
                    });
                }
            }
        }

        if edits.is_empty() {
            self.set_tsv_paste_status("No editable cells matched.");
            return;
        }
        let edit_count = edits.len();
        let applied_rows = data_rows.saturating_sub(skipped_rows);
        doc.journal.begin_edit(&doc.tag, "Paste TSV");
        let _ = apply_pending_edits(&mut doc.tag, edits, &mut doc.dirty);
        doc.journal.end_edit_window();
        let active = self.active;
        self.invalidate_tag_caches_in(active, &tag_key);

        let mut summary = format!("Pasted {edit_count} cell(s) across {applied_rows} row(s)");
        if skipped_rows > 0 {
            summary.push_str(&format!(
                " — {skipped_rows} extra row(s) ignored (block has {element_count} elements; add more first)"
            ));
        }
        self.status = summary.clone();
        self.set_tsv_paste_status(&summary);
    }

    fn set_tsv_paste_status(&mut self, message: &str) {
        if let Some(paste) = self.tsv_paste.as_mut() {
            paste.status = Some(message.to_owned());
        }
    }

    /// The documentation overlay (help/units + explanations) for a group,
    /// parsed once from its definition JSON and cached. `None` when the
    /// definitions can't be located (e.g. non-loose sources).
    /// Documentation overlay for `entry`'s group, resolved against `kit_index`
    /// rather than the active kit — in a split the two panes can be different
    /// games, whose definitions and group naming differ.
    pub(super) fn def_docs_for_entry(
        &mut self,
        kit_index: usize,
        entry: &TagEntry,
    ) -> Option<Rc<DefDocs>> {
        let source = self.kits[kit_index].source.as_ref()?;
        let root = match &source.source {
            TagSource::LooseFolder {
                definitions_root, ..
            } => definitions_root.clone(),
            _ => return None,
        };
        let game = source.game.clone()?;
        let group = self.kits[kit_index]
            .names
            .name_for(entry.group_tag)
            .or_else(|| group_tag_to_extension(entry.group_tag))?
            .to_owned();
        // Cache key is the group's own JSON path; the docs themselves merge the
        // whole `parent_tag` inheritance chain (object-family fields live in
        // parent files).
        let path = root.join(&game).join(format!("{group}.json"));
        if let Some(docs) = self.def_docs_cache.get(&path) {
            return Some(docs.clone());
        }
        let docs = Rc::new(build_def_docs(&root, &game, &group));
        self.def_docs_cache.insert(path, docs.clone());
        Some(docs)
    }

    pub(super) fn show_tags_with_keyword(&mut self, keyword: &str) {
        let keys = self.kits[self.active].keywords.tags_with(keyword);
        let entries: Vec<TagEntry> = keys
            .iter()
            .filter_map(|key| self.entry_for_key(key).cloned())
            .collect();
        let note = entries
            .is_empty()
            .then(|| "No tags with this keyword are in the current source.".to_owned());
        self.query_results = Some(TagQueryResults {
            kit: self.active_kit_id(),
            title: format!("Tags tagged '{keyword}' ({})", entries.len()),
            entries,
            annotations: Vec::new(),
            note,
            ref_target: None,
        });
    }

    /// Drop cached previews derived from a tag's contents so they rebuild from
    /// the (newly restored) tag bytes after an undo/redo.
    /// Drop derived previews for `key` in `kit`, after its document changed.
    pub(super) fn invalidate_tag_caches_in(&mut self, kit: usize, key: &str) {
        if let Some(preview) = self.kits[kit].model_previews.get_mut(key) {
            preview.loaded_key = None;
            preview.data = None;
        }
        if let Some(bitmap) = self.kits[kit].bitmap_previews.get_mut(key) {
            bitmap.decoded = None;
            bitmap.texture = None;
            bitmap.texture_dirty = true;
        }
        // rmdf/rmop caches are keyed by external render-method paths, not by this
        // tag's contents, and the shader grid rebuilds from the tag each frame —
        // so nothing to clear there.
    }

    pub(super) fn undo_current_tag(&mut self) {
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "Nothing to undo".to_owned();
            return;
        };
        let restored = self.kits[self.active]
            .parsed_tags
            .get_mut(&key)
            .and_then(|doc| doc.journal.undo(&doc.tag));
        self.restore_snapshot(&key, restored, "Undo");
    }

    pub(super) fn redo_current_tag(&mut self) {
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "Nothing to redo".to_owned();
            return;
        };
        let restored = self.kits[self.active]
            .parsed_tags
            .get_mut(&key)
            .and_then(|doc| doc.journal.redo(&doc.tag));
        self.restore_snapshot(&key, restored, "Redo");
    }

    /// Apply a snapshot returned by the journal: re-parse the bytes into the
    /// document and invalidate derived caches.
    fn restore_snapshot(&mut self, key: &str, restored: Option<(Vec<u8>, String)>, verb: &str) {
        // Classic (Halo CE / Halo 2) snapshots are serialized in classic format,
        // which `read_from_bytes` can't parse — re-parse with the JSON layout.
        let group_tag = self.kits[self.active]
            .parsed_tags
            .get(key)
            .map(|doc| doc.tag.group().tag);
        let game = self.source_game().map(str::to_owned);
        let definitions_root = self.source_definitions_root().map(Path::to_owned);
        match restored {
            Some((bytes, label)) => {
                match group_tag
                    .context("no open tag to restore")
                    .and_then(|group_tag| {
                        crate::source::read_tag_from_bytes(
                            &bytes,
                            game.as_deref(),
                            definitions_root.as_deref(),
                            group_tag,
                        )
                    }) {
                    Ok(tag) => {
                        if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(key) {
                            doc.tag = tag;
                            doc.dirty.touch();
                        }
                        let active = self.active;
                        self.invalidate_tag_caches_in(active, key);
                        self.status = format!("{verb}: {label}");
                    }
                    Err(error) => {
                        self.status = format!("{verb} failed: {error}");
                    }
                }
            }
            None => {
                self.status = format!("Nothing to {}", verb.to_ascii_lowercase());
            }
        }
    }

    pub(super) fn can_undo_current(&self) -> bool {
        self.kits[self.active]
            .selected_key
            .as_ref()
            .and_then(|key| self.kits[self.active].parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_undo())
    }

    pub(super) fn can_redo_current(&self) -> bool {
        self.kits[self.active]
            .selected_key
            .as_ref()
            .and_then(|key| self.kits[self.active].parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_redo())
    }

    pub(super) fn current_prefs(&self) -> GuiPrefs {
        GuiPrefs {
            // The focused workspace's view is what a new one is seeded with,
            // so a single-workspace session remembers its choice as before.
            browser_mode: self.kits[self.active].browser_mode,
            browser_sort: self.kits[self.active].browser_sort,
            nested_default: self.nested_default,
            show_browser_prefixes: self.show_browser_prefixes,
            folders_before_tags: self.folders_before_tags,
            double_click_to_open_tags: self.double_click_to_open_tags,
            session_restore: self.session_restore,
            update_channel: self.update_channel,
            check_updates_on_startup: self.check_updates_on_startup,
            show_block_sizes: self.show_block_sizes,
            scroll_to_cycle_dropdowns: self.scroll_to_cycle_dropdowns,
            confirm_container_overwrite: self.confirm_container_overwrite,
            confirm_runtime_poke: self.confirm_runtime_poke,
            enable_chimp: self.enable_chimp,
            chimp_output_dir: self.chimp_output_dir.clone(),
            chimp_usmap_path: self.chimp_usmap_path.clone(),
            expert_mode: self.expert_mode,
            dark_mode: self.dark_mode,
            ui_scale: self.ui_scale,
            model_preview_size: self.model_preview_size,
            blender_path: self.blender_path.clone(),
            editing_kit_paths: self.editing_kit_paths.clone(),
            ek_folder_aliases: self.ek_folder_aliases.clone(),
            custom_editing_kit_profiles: self.custom_editing_kit_profiles.clone(),
            tool_commands_window_pos: self.tool_commands_window_pos,
            tool_commands_window_size: Some(self.tool_commands_window_size),
            tool_commands_left_width: self.tool_commands_left_width,
            tool_commands_collapsed_categories: self.tool_commands_collapsed_categories.clone(),
            recent_folders: self.recent_folders.clone(),
            editing_kit_favorites: self.editing_kit_favorites.clone(),
            custom_color_swatches: self.custom_color_swatches.clone(),
            palette_last_dir: self.palette_last_dir.clone(),
        }
    }

    pub(super) fn editing_kit_root(&self) -> Option<PathBuf> {
        let TagSource::LooseFolder { root, .. } = &self.source()?.source else {
            return None;
        };
        if root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("tags"))
        {
            return root.parent().map(Path::to_path_buf);
        }
        Some(root.clone())
    }

    pub(super) fn kit_tool_path(&self, executable_name: &str) -> Option<PathBuf> {
        Some(self.editing_kit_root()?.join(executable_name))
    }

    pub(super) fn launch_sapien(&mut self) {
        self.launch_kit_tool("Sapien", "sapien.exe");
    }

    /// The tag_test executable name for the loaded game. Each editing kit ships
    /// its own renamed build (e.g. H3EK is `halo3_tag_test.exe`); fall back to
    /// the generic name when the game is unknown.
    pub(super) fn tag_test_executable(&self) -> &'static str {
        tag_test_executable_for_game(self.source().and_then(|s| s.game.as_deref()))
    }

    pub(super) fn launch_tag_test(&mut self) {
        self.launch_kit_tool_clearing_startup("tag_test", self.tag_test_executable(), "init.txt");
    }

    pub(super) fn can_launch_scenario_in_sapien(&self, kit: usize, entry: &TagEntry) -> bool {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return false;
        };
        let Ok(context) = scenario_launch_context(source, entry) else {
            return false;
        };
        sapien_supports_scenario_argument(&context.game)
            && context.kit_root.join("sapien.exe").is_file()
    }

    pub(super) fn launch_scenario_in_sapien(&mut self, key: &str) {
        let context = {
            let Some(source) = self.source() else {
                self.status = "Scenario launching requires a loaded editing kit".to_owned();
                return;
            };
            let Some(entry) = self.entry_for_key(key) else {
                self.status = "The scenario tag is no longer in the source".to_owned();
                return;
            };
            match scenario_launch_context(source, entry) {
                Ok(context) => context,
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        };
        if !sapien_supports_scenario_argument(&context.game) {
            self.status =
                "Opening a scenario directly in Sapien is not supported for this editing kit"
                    .to_owned();
            return;
        }
        let executable = context.kit_root.join("sapien.exe");
        if !executable.is_file() {
            self.status = format!("Sapien executable not found: {}", executable.display());
            return;
        }

        let dirty = self.kits[self.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set());
        if dirty {
            if let Err(error) = self.save_tag_by_key(key) {
                self.status = format!("Could not save scenario before launch: {error}");
                return;
            }
        }

        let mut process = Command::new(&executable);
        process
            .arg(&context.scenario_file)
            .current_dir(&context.kit_root);
        match process.spawn() {
            Ok(_) => {
                self.status = format!("Launched Sapien for {}", context.scenario_path);
            }
            Err(error) => {
                self.status = format!("Could not launch Sapien for this scenario: {error}");
            }
        }
    }

    pub(super) fn can_launch_scenario_in_tag_test(&self, kit: usize, entry: &TagEntry) -> bool {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return false;
        };
        let Ok(context) = scenario_launch_context(source, entry) else {
            return false;
        };
        let executable = tag_test_executable_for_game(Some(context.game.as_str()));
        context.kit_root.join(executable).is_file()
    }

    pub(super) fn launch_scenario_in_tag_test(&mut self, key: &str) {
        let context = {
            let Some(source) = self.source() else {
                self.status = "Scenario launching requires a loaded editing kit".to_owned();
                return;
            };
            let Some(entry) = self.entry_for_key(key) else {
                self.status = "The scenario tag is no longer in the source".to_owned();
                return;
            };
            match scenario_launch_context(source, entry) {
                Ok(context) => context,
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        };
        let executable_name = tag_test_executable_for_game(Some(context.game.as_str()));
        let executable = context.kit_root.join(executable_name);
        if !executable.is_file() {
            self.status = format!("tag_test executable not found: {}", executable.display());
            return;
        }

        let dirty = self.kits[self.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set());
        if dirty {
            if let Err(error) = self.save_tag_by_key(key) {
                self.status = format!("Could not save scenario before launch: {error}");
                return;
            }
        }

        let startup_file = context.kit_root.join("init.txt");
        let command = scenario_startup_command(&context.game, &context.scenario_path);
        if let Err(error) = update_scenario_startup_file(&startup_file, &command) {
            self.status = error;
            return;
        }
        let mut process = Command::new(&executable);
        process.current_dir(&context.kit_root);
        match process.spawn() {
            Ok(_) => {
                self.status = format!(
                    "Launched tag_test for {} using {}",
                    context.scenario_path,
                    startup_file.display()
                );
            }
            Err(error) => {
                self.status = format!(
                    "Wrote {}, but could not launch tag_test: {error}",
                    startup_file.display()
                );
            }
        }
    }

    pub(super) fn launch_blender(&mut self) {
        let Some(path) = self.blender_path.clone() else {
            self.settings_open = true;
            self.status = "Set the Blender path in File > Settings first".to_owned();
            return;
        };
        if !path.is_file() {
            self.status = format!("Blender executable not found: {}", path.display());
            self.settings_open = true;
            return;
        }
        self.spawn_tool("Blender", &path, path.parent().map(Path::to_path_buf));
    }

    pub(super) fn choose_blender_path(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Select Blender Executable");
        if let Some(path) = self.blender_path.as_ref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(path);
        }
        #[cfg(target_os = "windows")]
        {
            dialog = dialog.add_filter("Executable", &["exe"]);
        }
        if let Some(path) = dialog.pick_file() {
            self.blender_path = Some(path.clone());
            self.blender_path_input = path.display().to_string();
            self.status = format!("Blender path set to {}", path.display());
        }
    }

    pub(super) fn load_editing_kit_shortcut(
        &mut self,
        shortcut: EditingKitShortcut,
        ctx: egui::Context,
    ) {
        let Some(path) = self.editing_kit_paths.get(shortcut.game).cloned() else {
            self.prompt_for_editing_kit_path(
                shortcut,
                format!("Set the {} path in Settings first", shortcut.label),
            );
            return;
        };
        let status = self
            .editing_kit_validation
            .refresh_builtin(shortcut, Some(&path));
        let Some(layout) = status.layout().cloned() else {
            self.prompt_for_editing_kit_path(shortcut, status.message());
            return;
        };
        if shortcut.game == "haloce_evolved" {
            self.begin_load_folder_path(path, ctx);
        } else {
            self.begin_load_editing_kit_layout(
                layout,
                shortcut.game.to_owned(),
                game_display_name(shortcut.game).to_owned(),
                None,
                ctx,
            );
        }
    }

    pub(super) fn begin_command_line_launch(
        &mut self,
        launch: CommandLineLaunch,
        ctx: egui::Context,
    ) {
        let Some(shortcut) = EDITING_KIT_SHORTCUTS
            .iter()
            .copied()
            .find(|shortcut| shortcut.game == launch.game)
        else {
            self.status = format!(
                "Command line: {} is not a supported MCC editing kit",
                launch.kit_label
            );
            return;
        };
        let Some(path) = self.editing_kit_paths.get(shortcut.game).cloned() else {
            self.status = format!(
                "Command line: set the {} path in Settings before launching tags",
                launch.kit_label
            );
            return;
        };
        let status = self
            .editing_kit_validation
            .refresh_builtin(shortcut, Some(&path));
        let Some(layout) = status.layout().cloned() else {
            self.status = format!("Command line: {}", status.message());
            return;
        };
        self.kits[self.active].pending_launch_tags = Some(launch.tag_paths);
        self.begin_load_editing_kit_layout(
            layout,
            shortcut.game.to_owned(),
            game_display_name(shortcut.game).to_owned(),
            None,
            ctx,
        );
    }

    fn finish_pending_command_line_launch(&mut self, ctx: egui::Context) {
        let Some(requested) = self.kits[self.active].pending_launch_tags.take() else {
            return;
        };
        // Command-line startup deliberately remains popup-free. Indexing still
        // runs in the background and remains visible in the status bar.
        self.show_entry_index_wait_notice = false;
        let Some(source) = self.source() else {
            self.status = "Command line: the editing-kit source did not load".to_owned();
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            self.status = "Command line: the selected source is not a loose editing kit".to_owned();
            return;
        };
        let root = root.clone();
        let names = source.names.clone();
        let resolved = match resolve_launch_tag_entries(&root, &requested, &names) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.status = format!("Command line: {error}");
                return;
            }
        };
        let errors = resolved.errors;
        let entries = resolved.entries;
        if let Some(source) = self.source_mut() {
            for entry in &entries {
                if !source
                    .entries
                    .iter()
                    .any(|existing| existing.key == entry.key)
                {
                    source.entries.push(entry.clone());
                }
                if !source.all_entries.is_empty()
                    && !source
                        .all_entries
                        .iter()
                        .any(|existing| existing.key == entry.key)
                {
                    source.all_entries.push(entry.clone());
                }
            }
            if !source.all_entries.is_empty() {
                source
                    .all_entries
                    .sort_by(|a, b| a.display_path.cmp(&b.display_path));
                source.group_tree = crate::source::build_group_tree(&source.all_entries);
            }
        }
        for entry in &entries {
            self.select_entry(entry.key.clone(), ctx.clone());
        }
        self.status = match (entries.len(), errors.len()) {
            (opened, 0) => format!("Opened {opened} command-line tag(s)"),
            (opened, skipped) => format!(
                "Opened {opened} command-line tag(s); skipped {skipped}: {}",
                errors.join("; ")
            ),
        };
    }

    pub(super) fn load_custom_editing_kit_profile(
        &mut self,
        profile: CustomEditingKitProfile,
        ctx: egui::Context,
    ) -> bool {
        let layout = match self.editing_kit_validation.refresh_custom(&profile) {
            Ok(layout) => layout,
            Err(error) => {
                self.status = format!("{} is unavailable: {error}", profile.name);
                return false;
            }
        };
        self.begin_load_editing_kit_layout(
            layout,
            profile.game.clone(),
            profile.name.clone(),
            Some(EditingKitProfileIdentity {
                id: profile.id,
                name: profile.name,
            }),
            ctx,
        );
        true
    }

    fn begin_load_editing_kit_layout(
        &mut self,
        layout: EditingKitLayout,
        game: String,
        label: String,
        profile: Option<EditingKitProfileIdentity>,
        ctx: egui::Context,
    ) {
        if let Some(profile_identity) = profile.as_ref() {
            if let Some(index) = self.kits.iter().position(|kit| {
                kit.profile.as_ref().map(|open| open.id.as_str())
                    == Some(profile_identity.id.as_str())
            }) {
                self.active = index;
                self.status = format!("Switched to {}", label);
                return;
            }
            if let Some(index) = self.kits.iter().position(|kit| {
                kit.requested_path
                    .as_deref()
                    .is_some_and(|open| same_recent_path(open, &layout.root))
                    && kit
                        .source
                        .as_ref()
                        .and_then(|source| source.game.as_deref())
                        == Some(game.as_str())
            }) {
                self.active = index;
                self.kits[index].profile = Some(profile_identity.clone());
                self.status = format!("Switched to {}", label);
                return;
            }
            if !self.kits[self.active].is_empty_workspace() {
                self.add_kit();
            }
            self.kits[self.active].requested_path = Some(layout.root.clone());
        } else if self.open_kit_for(&layout.root) {
            self.status = format!("Switched to {}", label);
            return;
        }
        self.kits[self.active].profile = profile;
        let tx = self.tx.clone();
        let kit = self.active_kit_id();
        let names = self.default_names.clone();
        let definitions_root = locate_definitions_root();
        let tags_root = layout.tags;
        let recent_path = layout.root.clone();
        self.status = format!("Indexing {} as {game}", tags_root.display());
        thread::spawn(move || {
            let result = load_editing_kit_layout(tags_root, label, game, &names, &definitions_root)
                .map_err(|error| error.to_string());
            let _ = tx.send(WorkerMessage::SourceLoaded {
                kit,
                result,
                recent_path: Some(recent_path),
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn choose_editing_kit_path(&mut self, shortcut: EditingKitShortcut) {
        let title = if shortcut.game == "haloce_evolved" {
            "Select Campaign Evolved Install or Paks Folder".to_owned()
        } else {
            format!("Select {} Editing Kit Folder", shortcut.label)
        };
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(path) = self.editing_kit_paths.get(shortcut.game) {
            if path.is_dir() {
                dialog = dialog.set_directory(path);
            } else if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
                dialog = dialog.set_directory(parent);
            }
        }
        if let Some(path) = dialog.pick_folder() {
            self.editing_kit_paths
                .insert(shortcut.game.to_owned(), path.clone());
            self.editing_kit_path_inputs
                .insert(shortcut.game.to_owned(), path.display().to_string());
            if self.editing_kit_path_attention.as_deref() == Some(shortcut.game) {
                self.editing_kit_path_attention = None;
            }
            self.status = format!("{} path set to {}", shortcut.label, path.display());
            self.refresh_builtin_editing_kit_validation(shortcut);
        }
    }

    pub(super) fn auto_detect_editing_kit_paths(&mut self) {
        let detected = detect_editing_kit_paths();
        let added = apply_detected_editing_kit_paths(
            &mut self.editing_kit_paths,
            &mut self.editing_kit_path_inputs,
            &mut self.editing_kit_path_attention,
            &detected,
        );
        self.refresh_editing_kit_validation();
        self.status = if added == 0 {
            "No new editing kit paths detected".to_owned()
        } else {
            format!("Detected {added} editing kit path(s)")
        };
    }

    fn prompt_for_editing_kit_path(&mut self, shortcut: EditingKitShortcut, status: String) {
        self.settings_open = true;
        self.settings_tab = SettingsTab::EditingKits;
        self.editing_kit_path_attention = Some(shortcut.game.to_owned());
        self.editing_kit_path_inputs
            .entry(shortcut.game.to_owned())
            .or_default();
        self.status = status;
    }

    fn launch_kit_tool(&mut self, label: &str, executable_name: &str) {
        let Some(path) = self.kit_tool_path(executable_name) else {
            self.status = format!("{label} requires a loaded editing-kit folder");
            return;
        };
        if !path.is_file() {
            self.status = format!("{label} executable not found: {}", path.display());
            return;
        }
        self.spawn_tool(label, &path, self.editing_kit_root());
    }

    fn launch_kit_tool_clearing_startup(
        &mut self,
        label: &str,
        executable_name: &str,
        startup_file_name: &str,
    ) {
        let Some(path) = self.kit_tool_path(executable_name) else {
            self.status = format!("{label} requires a loaded editing-kit folder");
            return;
        };
        if !path.is_file() {
            self.status = format!("{label} executable not found: {}", path.display());
            return;
        }
        let Some(root) = self.editing_kit_root() else {
            self.status = format!("{label} requires a loaded editing-kit folder");
            return;
        };
        let startup_file = root.join(startup_file_name);
        if let Err(error) = clear_scenario_startup_commands(&startup_file) {
            self.status = error;
            return;
        }
        self.spawn_tool(label, &path, Some(root));
    }

    fn spawn_tool(&mut self, label: &str, path: &Path, work_dir: Option<PathBuf>) {
        let mut command = Command::new(path);
        if let Some(work_dir) = work_dir {
            command.current_dir(work_dir);
        }
        match command.spawn() {
            Ok(_) => self.status = format!("Launched {label}"),
            Err(error) => self.status = format!("Could not launch {label}: {error}"),
        }
    }

    /// Record the current terminal-open state against the loaded game so it
    /// is restored next time that editing kit is opened.
    pub(super) fn remember_terminal_open_for_game(&mut self) {
        let Some(game) = self.source().and_then(|s| s.game.clone()) else {
            return;
        };
        if self.kits[self.active].terminal_open {
            self.terminal_open_games.insert(game);
        } else {
            self.terminal_open_games.remove(&game);
        }
    }

    /// Resolve a pending "Open referenced tag" request against its active
    /// source. Loose folders resolve a file path; Campaign Evolved resolves the
    /// existing stable entry from its mounted container catalog.
    pub(super) fn process_pending_open(&mut self, ctx: &egui::Context) {
        let Some(req) = self.pending_open.take() else {
            return;
        };
        let container_key = self.source().and_then(|source| {
            matches!(&source.source, TagSource::IoStoreContainerSet { .. }).then(|| {
                container_entry_for_reference(
                    &source.entries,
                    req.group_tag,
                    &req.rel_path,
                    self.names(),
                )
                .map(|entry| entry.key.clone())
            })
        });
        if let Some(container_key) = container_key {
            let Some(key) = container_key else {
                self.status = format!(
                    "Referenced Campaign Evolved tag not found: {} (group {})",
                    req.rel_path.replace('\\', "/"),
                    blam_tags::format_group_tag(req.group_tag)
                );
                return;
            };
            self.select_entry(key.clone(), ctx.clone());
            if req.float {
                self.kits[self.active].open_tag_pane_beside(&key);
            }
            return;
        }

        let root = match self.source().map(|s| &s.source) {
            Some(TagSource::LooseFolder { root, .. }) => root.clone(),
            _ => {
                self.status = "Open requires a loose-folder source".to_owned();
                return;
            }
        };
        // Resolve the file extension from the definitions name index first
        // (covers every group, e.g. collision_model/physics_model), falling
        // back to the built-in table.
        let ext = self
            .names()
            .name_for(req.group_tag)
            .or_else(|| blam_tags::paths::group_tag_to_extension(req.group_tag))
            .unwrap_or("");
        // Normalize: tolerate forward slashes and a path that already carries
        // its extension (e.g. a shader bitmap ref), so we don't double-append.
        let mut rel = req.rel_path.replace('/', "\\");
        if !ext.is_empty() {
            if let Some(stripped) = rel
                .strip_suffix(&format!(".{ext}"))
                .or_else(|| rel.strip_suffix(&format!(".{}", ext.to_ascii_uppercase())))
            {
                rel = stripped.to_owned();
            }
        }
        let abs = blam_tags::paths::resolve_tag_path(&root, &rel, ext);
        if !abs.exists() {
            self.status = format!(
                "Referenced tag not found: {} (group {})",
                abs.display(),
                blam_tags::format_group_tag(req.group_tag)
            );
            return;
        }
        let key = format!("file:{}", abs.display());
        // Ensure an entry exists so ensure_tag_loading can resolve it.
        if self.entry_for_key(&key).is_none() {
            let group_name = self.names().name_for(req.group_tag).map(str::to_owned);
            let display_path = if ext.is_empty() {
                req.rel_path.replace('\\', "/")
            } else {
                format!("{}.{ext}", req.rel_path.replace('\\', "/"))
            };
            let entry = TagEntry {
                key: key.clone(),
                display_path,
                group_tag: req.group_tag,
                group_name,
                location: TagEntryLocation::LooseFile(abs),
            };
            if let Some(source) = self.source_mut() {
                source.entries.push(entry);
            }
        }
        self.select_entry(key.clone(), ctx.clone());
        // Alt-click asks for the tag beside the current one rather than as
        // another tab in the same group.
        if req.float {
            self.kits[self.active].open_tag_pane_beside(&key);
        }
    }

    /// Run a geometry Import request (`tool render/collision/physics/...`)
    /// streamed to the terminal panel.
    pub(super) fn process_pending_tool_import(&mut self, ctx: &egui::Context) {
        let Some(req) = self.pending_tool_import.take() else {
            return;
        };
        if self.editing_kit_root().is_none() {
            self.status = "Import requires a loaded editing-kit folder".to_owned();
            return;
        }
        let command = format!("tool {} \"{}\"", req.verb, req.source_dir);
        self.spawn_terminal_command(command, ctx.clone());
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_reimport_bitmap(&mut self, key: String, ctx: egui::Context) {
        if self.terminal.running {
            self.status = "A command is already running".to_owned();
            return;
        }
        let Some(source) = self.source().map(|source| source.source.clone()) else {
            self.status = "Reimport requires a loaded editing-kit folder".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Bitmap tag is no longer in the source".to_owned();
            return;
        };
        let Some(tags_root) = (match &source {
            TagSource::LooseFolder { root, .. } => Some(root.as_path()),
            _ => None,
        }) else {
            self.status = "Bitmap reimport requires a loose tags folder".to_owned();
            return;
        };
        let Some(work_dir) = tags_root.parent().map(Path::to_path_buf) else {
            self.status = "Could not resolve editing-kit root".to_owned();
            return;
        };
        let Some(data_path) = bitmap_reimport_data_path(&entry, Some(tags_root)) else {
            self.status = "Could not resolve bitmap data path".to_owned();
            return;
        };
        let command = format!("tool bitmaps \"{data_path}\"");
        self.kits[self.active].terminal_open = true;
        self.terminal
            .lines
            .push(TerminalLineEntry::new(format!("> {command}")));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
        self.terminal.refocus_input = true;
        self.terminal.running = true;
        self.status = format!("Reimporting bitmap {}", entry.display_path);
        let run_id = self.terminal.next_run_id;
        self.terminal.next_run_id = self.terminal.next_run_id.wrapping_add(1).max(1);
        let log_file = match create_terminal_log_file(run_id, &command) {
            Ok((path, file)) => {
                self.terminal.last_log_path = Some(path);
                Some(file)
            }
            Err(error) => {
                self.status = format!("Terminal full log unavailable: {error}");
                self.terminal.last_log_path = None;
                None
            }
        };

        let tx = self.tx.clone();
        let kit = self.active_kit_id();
        thread::spawn(move || {
            let result =
                run_terminal_command_for_reimport(&command, &work_dir, &tx, &ctx, log_file)
                    .and_then(|_| read_entry(&source, &entry).map_err(|error| error.to_string()));
            let _ = tx.send(WorkerMessage::BitmapReimportFinished { kit, key, result });
            ctx.request_repaint();
        });
    }

    /// Render the block delete/delete-all confirmation modal (if pending) and
    /// apply the op on confirm.
    pub(super) fn handle_block_confirm(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.block_confirm.as_ref() else {
            return;
        };
        // The op is applied to the active kit's document, and two workspaces of
        // the same game share a key space, so a confirmation answered after a
        // switch could delete from the wrong game's tag.
        let confirm_kit = confirm.kit;
        let message = confirm.message.clone();
        let confirm_label = confirm.confirm_label.clone();
        let mut do_apply = false;
        let mut do_cancel = false;
        egui::Window::new("Confirm")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(RichText::new(message).color(text_dark()));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(&confirm_label)
                                    .color(Color32::from_rgb(230, 230, 228)),
                            )
                            .fill(Color32::from_rgb(150, 48, 40))
                            .min_size(Vec2::new(80.0, 24.0)),
                        )
                        .clicked()
                    {
                        do_apply = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(Vec2::new(80.0, 24.0)))
                        .clicked()
                    {
                        do_cancel = true;
                    }
                });
            });
        if do_apply {
            let routed = confirm_kit.is_some_and(|kit| self.focus_navigation_kit(kit));
            if let Some(confirm) = self.block_confirm.take()
                && routed
            {
                if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&confirm.tag_key) {
                    let op = BlockOp {
                        path: confirm.path,
                        kind: confirm.kind,
                    };
                    doc.journal.begin_edit(&doc.tag, "Block edit");
                    if let Some(status) = apply_block_ops(&mut doc.tag, vec![op], &mut doc.dirty) {
                        self.status = status;
                    }
                    doc.journal.end_edit_window();
                }
            }
        } else if do_cancel {
            self.block_confirm = None;
        }
    }

    pub(super) fn handle_save_changes_prompt(&mut self, ctx: &egui::Context) {
        let action = render_save_changes_prompt(ctx, &mut self.save_changes_prompt);
        match action {
            SaveChangesPromptAction::None => {}
            SaveChangesPromptAction::Cancel => {
                self.save_changes_prompt.visible = false;
                self.save_changes_prompt.dirty_tags.clear();
                self.save_changes_prompt.error = None;
                self.save_changes_prompt.confirm_discard = false;
            }
            // Arming, not acting: the click that deletes is the next one.
            SaveChangesPromptAction::ConfirmDiscard => {
                self.save_changes_prompt.confirm_discard = true;
            }
            SaveChangesPromptAction::StashForMod => {
                let action = self.save_changes_prompt.pending_action.clone();
                let now = ctx.input(|input| input.time);
                match self.checkpoint_campaign_project(self.active, now) {
                    Ok(_) => {
                        // The project holds these bytes now, so they are no
                        // longer unsaved work: leaving them dirty would prompt
                        // again on the next close and, for a CloseApp walking
                        // several kits, would never terminate.
                        for entry in &self.save_changes_prompt.dirty_tags {
                            if let Some(document) =
                                self.kits[self.active].parsed_tags.get_mut(&entry.tag_id)
                            {
                                document.dirty.clear();
                            }
                        }
                        self.save_changes_prompt.visible = false;
                        self.save_changes_prompt.dirty_tags.clear();
                        self.save_changes_prompt.error = None;
                        self.save_changes_prompt.confirm_discard = false;
                        self.status = match self.kits[self.active]
                            .campaign_project
                            .as_ref()
                            .and_then(|project| project.project_path.clone())
                        {
                            // Named, because the file the user thinks of as their
                            // project is not the one this wrote.
                            Some(path) => format!(
                                "Stashed for Export Mod. {} is unchanged until you save it",
                                path.display()
                            ),
                            None => "Stashed for Export Mod".to_owned(),
                        };
                        self.execute_close_action(action, ctx);
                    }
                    Err(error) => {
                        self.save_changes_prompt.error =
                            Some(format!("Could not stash into the project: {error}"));
                    }
                }
            }
            SaveChangesPromptAction::DontSave => {
                let action = self.save_changes_prompt.pending_action.clone();
                // Discarding is explicit, so drop the dirty flags the prompt
                // listed. Without this, a CloseApp that spans several kits
                // would see the same unsaved work again and re-prompt forever.
                //
                // On a stashing workspace this also deletes the stashed copies —
                // which is why the button is named Discard there and takes a
                // second, confirming click. What it deletes from is the
                // workspace's own recovery file; a `.baboon` the user opened or
                // saved is never written by a close.
                let kit = self.active;
                let tag_ids: Vec<String> = self
                    .save_changes_prompt
                    .dirty_tags
                    .iter()
                    .map(|entry| entry.tag_id.clone())
                    .collect();
                for tag_id in &tag_ids {
                    if let Some(doc) = self.kits[kit].parsed_tags.get_mut(tag_id) {
                        doc.dirty.clear();
                    }
                    // And forget anything the project stashed for it. Autosave
                    // captures a dirty tag within a second of the edit, so
                    // without this "Don't Save" cleared a flag while the edited
                    // bytes stayed behind and came back on reopen.
                    self.forget_campaign_overlay(kit, tag_id);
                    self.kits[kit].edit_buffers.forget_tag(tag_id);
                    // Declining to save a brand-new tag discards the tag, not
                    // just its edits: nothing backs it but the document the
                    // close is about to drop. Its browser entry goes with it.
                    self.forget_new_container_entry(kit, tag_id);
                }
                let now = ctx.input(|input| input.time);
                if let Err(error) = self.checkpoint_campaign_project(kit, now) {
                    self.status = format!("Could not update the Campaign Evolved project: {error}");
                }
                self.save_changes_prompt.visible = false;
                self.save_changes_prompt.dirty_tags.clear();
                self.save_changes_prompt.error = None;
                self.save_changes_prompt.confirm_discard = false;
                self.execute_close_action(action, ctx);
            }
            SaveChangesPromptAction::Save(tag_ids) => {
                let mut saved = Vec::new();
                let mut errors = Vec::new();
                for tag_id in tag_ids {
                    // Container tags have no loose file to write. A brand-new
                    // one saves via a file dialog (new override container); an
                    // existing one is overwritten inside the game's pak, with
                    // this prompt's Save button standing in for the separate
                    // overwrite confirmation. Both report through `status`
                    // instead of returning a path, so success is read back off
                    // the document's dirty flag.
                    match self.entry_for_key(&tag_id).map(|entry| &entry.location) {
                        Some(TagEntryLocation::NewContainer { .. }) => {
                            self.save_new_container_tag(&tag_id);
                            if self.tag_is_dirty(&tag_id) {
                                let label = self.tag_path_label(&tag_id);
                                errors.push(format!("{label}: not saved"));
                            } else {
                                saved.push(tag_id.clone());
                            }
                            continue;
                        }
                        Some(TagEntryLocation::Container { .. }) => {
                            self.overwrite_current_tag_in_place(&tag_id);
                            if self.tag_is_dirty(&tag_id) {
                                // The overwrite failure reason is in `status`.
                                let label = self.tag_path_label(&tag_id);
                                let detail = self.status.clone();
                                errors.push(format!("{label}: {detail}"));
                            } else {
                                saved.push(tag_id.clone());
                            }
                            continue;
                        }
                        _ => {}
                    }
                    match self.save_tag_by_key(&tag_id) {
                        Ok(path) => saved.push(path.display().to_string()),
                        Err(error) => {
                            let label = self.tag_path_label(&tag_id);
                            errors.push(format!("{label}: {error}"));
                        }
                    }
                }
                if errors.is_empty() {
                    let action = self.save_changes_prompt.pending_action.clone();
                    self.save_changes_prompt.visible = false;
                    self.save_changes_prompt.dirty_tags.clear();
                    self.save_changes_prompt.error = None;
                    self.status = if saved.is_empty() {
                        "No files selected to save".to_owned()
                    } else {
                        format!("Saved {} file(s)", saved.len())
                    };
                    self.execute_close_action(action, ctx);
                } else {
                    let message = format!("Save failed: {}", errors.join("; "));
                    let pending_action = self.save_changes_prompt.pending_action.clone();
                    self.save_changes_prompt.dirty_tags =
                        self.dirty_tags_for_close_action(&pending_action);
                    // A failed save leaves the prompt up, and an armed discard
                    // has no business surviving into it.
                    self.save_changes_prompt.confirm_discard = false;
                    self.status = message.clone();
                    self.save_changes_prompt.error = Some(message);
                }
            }
        }
    }

    pub(super) fn handle_last_opened_windows_prompt(&mut self, ctx: &egui::Context) {
        let action = render_last_opened_windows_prompt(ctx, self.last_opened_windows.as_mut());
        match action {
            LastOpenedWindowsAction::None => {}
            LastOpenedWindowsAction::OpenSettings => {
                self.last_opened_windows = None;
                self.settings_open = true;
            }
            LastOpenedWindowsAction::Cancel { remember } => {
                if remember {
                    self.session_restore = SessionRestore::Never;
                }
                self.last_opened_windows = None;
            }
            LastOpenedWindowsAction::Restore { kits, remember } => {
                if remember {
                    self.session_restore = SessionRestore::Always;
                }
                self.last_opened_windows = None;
                self.begin_last_session_restore(kits, ctx.clone());
            }
        }
    }

    pub(super) fn persist_prefs_if_changed(&mut self) {
        let prefs = self.current_prefs();
        if prefs == self.saved_prefs && self.terminal_open_games == self.saved_terminal_open_games {
            return;
        }
        match save_gui_prefs(&prefs, &self.terminal_open_games, true) {
            Ok(()) => {
                self.saved_prefs = prefs;
                self.saved_terminal_open_games = self.terminal_open_games.clone();
            }
            Err(error) => self.status = error,
        }
    }
}

fn reset_lazy_folder_browser(
    root: &Path,
    tree: &mut TagTree,
    entries: &mut Vec<TagEntry>,
) -> Result<(), String> {
    *tree = crate::source::build_folder_directory_tree(root).map_err(|error| error.to_string())?;
    entries.clear();
    Ok(())
}

#[cfg(test)]
#[path = "tests/browser_refresh.rs"]
mod browser_refresh_tests;

#[cfg(test)]
#[path = "tests/save_changes_prompt.rs"]
mod save_changes_prompt_tests;

#[cfg(test)]
#[path = "tests/mod_overrides.rs"]
mod mod_override_tests;

enum SaveChangesPromptAction {
    None,
    Save(Vec<String>),
    /// Keep the edits in this workspace's Baboon project, ready for Export
    /// Mod, without writing anything into the game's own files.
    StashForMod,
    /// Arm the discard. Only raised on a stashing workspace, where discarding
    /// deletes stashed bytes rather than just dropping an in-memory edit.
    ConfirmDiscard,
    DontSave,
    Cancel,
}

struct DiscardButton {
    label: &'static str,
    width: f32,
    /// Whether this click only arms the discard. False means it deletes.
    arming: bool,
}

/// What the prompt's discard button says and does.
///
/// On a workspace that stashes, this button deletes bytes that persist across
/// sessions — an edit is stashed within a second of being typed, and exporting a
/// mod does not clear it — so it is named for what it does and takes a second,
/// confirming click. A loose kit has no stash to lose and keeps the one-click
/// "Don't Save" every editor has.
fn discard_button(can_stash: bool, confirmed: bool) -> DiscardButton {
    match (can_stash, confirmed) {
        (false, _) => DiscardButton {
            label: "Don't Save",
            width: 78.0,
            arming: false,
        },
        (true, false) => DiscardButton {
            label: "Discard...",
            width: 96.0,
            arming: true,
        },
        (true, true) => DiscardButton {
            label: "Delete Stashed Edits",
            width: 150.0,
            arming: false,
        },
    }
}

fn render_save_changes_prompt(
    ctx: &egui::Context,
    prompt: &mut SaveChangesPrompt,
) -> SaveChangesPromptAction {
    if !prompt.visible {
        return SaveChangesPromptAction::None;
    }

    let mut action = SaveChangesPromptAction::None;
    egui::Window::new("Baboon - Save Changes?")
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(520.0)
        .default_height(260.0)
        .show(ctx, |ui| {
            ui.label(
                RichText::new("The following files have been modified. Select the files to save.")
                    .color(text_dark()),
            );
            if prompt.can_stash {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Save overwrites the game's own pak files in place. Stash for Mod keeps \
                         the edits in this workspace's project instead, ready for Export Mod. \
                         Discard throws them away, including the copy the project is holding.",
                    )
                    .small()
                    .color(subtle_dark()),
                );
            }
            ui.add_space(8.0);
            if let Some(error) = prompt.error.as_deref() {
                ui.label(RichText::new(error).color(Color32::from_rgb(180, 48, 40)));
                ui.add_space(6.0);
            }
            // Spelling out what the destructive button costs, and where from.
            // Exporting a mod does not clear these edits — the mod is a copy —
            // so this is the prompt an exporter sees on every exit, and it used
            // to delete the stash on one unlabelled click.
            if prompt.confirm_discard {
                ui.label(
                    RichText::new(match (prompt.stashed, prompt.stash_file.as_deref()) {
                        (0, _) => "Discard these edits? They are not stashed anywhere, so they \
                                   cannot be recovered."
                            .to_owned(),
                        (count, Some(file)) => format!(
                            "Discard deletes the stashed copy of {count} tag(s) from {}. Other \
                             stashed tags in this workspace, and mods you have already exported, \
                             are not affected.",
                            file.display()
                        ),
                        (count, None) => format!(
                            "Discard deletes the stashed copy of {count} tag(s). Mods you have \
                             already exported are not affected."
                        ),
                    })
                    .color(Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(6.0);
            }
            ScrollArea::both().max_height(150.0).show(ui, |ui| {
                for dirty in &mut prompt.dirty_tags {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut dirty.checked, "");
                        ui.label(RichText::new(&dirty.path).color(text_dark()));
                    });
                }
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("Cancel").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = SaveChangesPromptAction::Cancel;
                }
                let DiscardButton {
                    label,
                    width,
                    arming,
                } = discard_button(prompt.can_stash, prompt.confirm_discard);
                if ui
                    .add(egui::Button::new(label).min_size(Vec2::new(width, 24.0)))
                    .clicked()
                {
                    action = if arming {
                        SaveChangesPromptAction::ConfirmDiscard
                    } else {
                        SaveChangesPromptAction::DontSave
                    };
                }
                if prompt.can_stash
                    && ui
                        .add(egui::Button::new("Stash for Mod").min_size(Vec2::new(110.0, 24.0)))
                        .on_hover_text(
                            "Keep these edits in this workspace's Baboon project, ready for \
                             Export Mod. The game's own files are left untouched.",
                        )
                        .clicked()
                {
                    action = SaveChangesPromptAction::StashForMod;
                }
                if ui
                    .add(egui::Button::new("Save").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    let tag_ids = prompt
                        .dirty_tags
                        .iter()
                        .filter(|dirty| dirty.checked)
                        .map(|dirty| dirty.tag_id.clone())
                        .collect();
                    action = SaveChangesPromptAction::Save(tag_ids);
                }
            });
        });
    action
}

enum LastOpenedWindowsAction {
    None,
    OpenSettings,
    Restore {
        /// Each kit to reopen, with the tags checked for it.
        kits: Vec<RestoreKit>,
        /// "Don't ask again" was ticked — remember this as `Always`.
        remember: bool,
    },
    Cancel {
        /// "Don't ask again" was ticked — remember this as `Never`.
        remember: bool,
    },
}

fn render_last_opened_windows_prompt(
    ctx: &egui::Context,
    prompt: Option<&mut LastOpenedWindowsPrompt>,
) -> LastOpenedWindowsAction {
    let Some(prompt) = prompt else {
        return LastOpenedWindowsAction::None;
    };
    if !prompt.visible {
        return LastOpenedWindowsAction::None;
    }

    let mut action = LastOpenedWindowsAction::None;
    egui::Window::new("Last Opened Windows")
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(520.0)
        .default_height(300.0)
        .show(ctx, |ui| {
            ui.label(
                RichText::new("These windows were opened the last time you used Baboon.")
                    .color(text_dark()),
            );
            ui.label(RichText::new("Which of these would you like to reopen?").color(text_dark()));
            ui.add_space(8.0);
            ScrollArea::both().max_height(260.0).show(ui, |ui| {
                for (index, kit) in prompt.kits.iter_mut().enumerate() {
                    if index > 0 {
                        ui.add_space(10.0);
                    }
                    let heading = match kit.game.as_deref() {
                        Some(game) => game_display_name(game).to_owned(),
                        None => kit.source_path.display().to_string(),
                    };
                    ui.label(RichText::new(heading).color(text_dark()).strong())
                        .on_hover_text(kit.source_path.display().to_string());
                    if !kit.source_available {
                        ui.label(
                            RichText::new(format!("Missing source: {}", kit.source_path.display()))
                                .color(Color32::from_rgb(180, 48, 40)),
                        );
                    }
                    // Why a workspace is listed with nothing under it: its
                    // session is its stash, which comes back from its own
                    // recovery file rather than from a list of tabs.
                    if kit.entries.is_empty() && kit.has_project {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("Unsaved changes stashed in this workspace")
                                    .color(subtle_dark())
                                    .small(),
                            );
                        });
                    }
                    if !kit.entries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Tags").color(subtle_dark()).strong());
                        });
                    }
                    for entry in &mut kit.entries {
                        ui.add_enabled_ui(entry.available, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.checkbox(&mut entry.checked, "");
                                let label = if entry.available {
                                    entry.tag.label.clone()
                                } else {
                                    format!("{} (missing)", entry.tag.label)
                                };
                                ui.label(RichText::new(label).color(text_dark()));
                            });
                        });
                    }
                    if !kit.chimp_entries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Chimp").color(subtle_dark()).strong());
                        });
                    }
                    for entry in &mut kit.chimp_entries {
                        ui.add_enabled_ui(entry.available, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.checkbox(&mut entry.checked, "");
                                let label = if entry.available {
                                    entry.package.clone()
                                } else {
                                    format!("{} (missing source)", entry.package)
                                };
                                ui.label(RichText::new(label).color(text_dark()));
                            });
                        });
                    }
                }
            });
            ui.add_space(6.0);
            ui.checkbox(&mut prompt.dont_ask_again, "Don't ask again")
                .on_hover_text(
                    "Remember this choice: OK always reopens the last session, \
                     Cancel never does. Change it later in File > Settings.",
                );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Options for this window available in")
                        .color(subtle_dark())
                        .small(),
                );
                if ui.link("File > Settings").clicked() {
                    action = LastOpenedWindowsAction::OpenSettings;
                }
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("Cancel").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = LastOpenedWindowsAction::Cancel {
                        remember: prompt.dont_ask_again,
                    };
                }
                if ui
                    .add(egui::Button::new("OK").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = LastOpenedWindowsAction::Restore {
                        kits: prompt.checked_kits(),
                        remember: prompt.dont_ask_again,
                    };
                }
            });
        });
    action
}

#[cfg(test)]
mod tests {
    use super::ensure_priority_suffix;
    use std::path::PathBuf;

    /// A mod without `_P` mounts at the same priority as the game's own
    /// containers and loses, so it builds correctly and does nothing. Renaming
    /// the default to something meaningful is exactly how it gets dropped --
    /// which is how one was reported.
    /// A session written before the chosen folder was recorded holds the
    /// resolved `Paks` directory. Restoring from it put that directory back
    /// into the recents list on every launch, which is how "Paks" kept
    /// reappearing however often it was removed.
    #[test]
    fn a_paks_directory_walks_back_up_to_the_opened_folder() {
        let root = std::env::temp_dir().join(format!("baboon-paks-{}", std::process::id()));
        let paks = root.join("Meteorite").join("Content").join("Paks");
        std::fs::create_dir_all(&paks).unwrap();
        // `find_paks_dir` needs a container present to recognise the folder.
        std::fs::write(paks.join("pakchunk0-WinGDK.utoc"), []).unwrap();

        assert_eq!(super::install_root_for_paks(&paks), root);
        // Already the opened folder: nothing to strip.
        assert_eq!(super::install_root_for_paks(&root), root);
        // The shorter layout the resolver also accepts.
        assert_eq!(
            super::install_root_for_paks(&root.join("Content").join("Paks")),
            root
        );
        // An unfamiliar layout is left exactly as it is rather than guessed at.
        let odd = root.join("somewhere").join("Paks");
        assert_eq!(super::install_root_for_paks(&odd), odd);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mod_always_gets_the_priority_suffix() {
        assert_eq!(
            ensure_priority_suffix(PathBuf::from("/mods/h2a_magnum.utoc")),
            PathBuf::from("/mods/h2a_magnum_P.utoc")
        );
        // Already correct, including the platform suffix the game itself uses.
        assert_eq!(
            ensure_priority_suffix(PathBuf::from("/mods/mymod-WinGDK_P.utoc")),
            PathBuf::from("/mods/mymod-WinGDK_P.utoc")
        );
        // The loader folds case before comparing, so a lowercase suffix
        // already has priority and must not collect a second one.
        assert_eq!(
            ensure_priority_suffix(PathBuf::from("/mods/thing_p.utoc")),
            PathBuf::from("/mods/thing_p.utoc")
        );
        // A version before the suffix raises priority further; it is still a
        // suffixed name and must be left alone.
        assert_eq!(
            ensure_priority_suffix(PathBuf::from("/mods/thing_2_P.utoc")),
            PathBuf::from("/mods/thing_2_P.utoc")
        );
    }

    use super::*;

    #[test]
    fn normalize_container_tag_rel_cleans_path() {
        // Lowercases, normalizes separators, trims slashes, drops a leaf extension.
        assert_eq!(
            normalize_container_tag_rel("Objects\\Characters/Foo/Bar"),
            "objects/characters/foo/bar"
        );
        assert_eq!(
            normalize_container_tag_rel("/objects//foo/bar.biped/"),
            "objects/foo/bar"
        );
        assert_eq!(normalize_container_tag_rel("  Foo.Weapon  "), "foo");
        assert_eq!(normalize_container_tag_rel(""), "");
        assert_eq!(normalize_container_tag_rel("///"), "");
    }

    #[test]
    fn explorer_select_arguments_keep_switch_separate_from_path_with_spaces() {
        let path = Path::new(r"C:\Program Files\H2EK\tags\objects\example.weapon");

        assert_eq!(
            explorer_select_args(path),
            [
                std::ffi::OsString::from("/select,"),
                path.as_os_str().to_owned(),
            ]
        );
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("baboon-{name}-{}-{nanos}", std::process::id()))
    }

    fn write_classic_ce_tag(path: &Path, group: &[u8; 4]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut bytes = [0u8; 64];
        bytes[36..40].copy_from_slice(group);
        bytes[60..64].copy_from_slice(b"blam");
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn detect_editing_kit_paths_finds_all_known_common_folder_names() {
        let common = unique_test_dir("ek-detect-all");
        for shortcut in EDITING_KIT_SHORTCUTS {
            if shortcut.game == "haloce_evolved" {
                continue;
            }
            std::fs::create_dir_all(common.join(shortcut.label).join("tags")).unwrap();
        }
        let campaign_evolved_paks = common
            .join("Halo Campaign Evolved")
            .join("Meteorite")
            .join("Content")
            .join("Paks");
        std::fs::create_dir_all(&campaign_evolved_paks).unwrap();
        std::fs::write(campaign_evolved_paks.join("pakchunk0-WinGDK.utoc"), []).unwrap();

        let detected = detect_editing_kit_paths_in_common_roots(vec![common.clone()]);

        for shortcut in EDITING_KIT_SHORTCUTS {
            let expected = if shortcut.game == "haloce_evolved" {
                common.join("Halo Campaign Evolved")
            } else {
                common.join(shortcut.label)
            };
            assert_eq!(detected.get(shortcut.game), Some(&expected));
        }
        let _ = std::fs::remove_dir_all(common);
    }

    #[test]
    fn detect_editing_kit_paths_ignores_campaign_evolved_without_containers() {
        let common = unique_test_dir("ek-detect-campaign-evolved-containers-required");
        std::fs::create_dir_all(common.join("Halo Campaign Evolved")).unwrap();

        let detected = detect_editing_kit_paths_in_common_roots(vec![common.clone()]);

        assert!(!detected.contains_key("haloce_evolved"));
        let _ = std::fs::remove_dir_all(common);
    }

    #[test]
    fn detect_editing_kit_paths_ignores_folder_without_tags_child() {
        let common = unique_test_dir("ek-detect-tags-required");
        std::fs::create_dir_all(common.join("H3EK")).unwrap();
        std::fs::create_dir_all(common.join("H4EK").join("tags")).unwrap();

        let detected = detect_editing_kit_paths_in_common_roots(vec![common.clone()]);

        assert!(!detected.contains_key("halo3_mcc"));
        assert_eq!(detected.get("halo4_mcc"), Some(&common.join("H4EK")));
        let _ = std::fs::remove_dir_all(common);
    }

    #[test]
    fn apply_detected_editing_kit_paths_fills_blanks_only() {
        let mut paths = HashMap::from([
            ("halo3_mcc".to_owned(), PathBuf::from("C:/Custom/H3EK")),
            (
                "haloce_evolved".to_owned(),
                PathBuf::from("D:/Custom/Halo Campaign Evolved"),
            ),
        ]);
        let mut inputs = HashMap::from([
            ("halo3_mcc".to_owned(), "C:/Custom/H3EK".to_owned()),
            (
                "haloce_evolved".to_owned(),
                "D:/Custom/Halo Campaign Evolved".to_owned(),
            ),
        ]);
        let mut attention = Some("halo4_mcc".to_owned());
        let detected = HashMap::from([
            ("halo3_mcc".to_owned(), PathBuf::from("C:/Steam/H3EK")),
            ("halo4_mcc".to_owned(), PathBuf::from("C:/Steam/H4EK")),
            (
                "haloce_evolved".to_owned(),
                PathBuf::from("C:/Steam/Halo Campaign Evolved"),
            ),
        ]);

        let added =
            apply_detected_editing_kit_paths(&mut paths, &mut inputs, &mut attention, &detected);

        assert_eq!(added, 1);
        assert_eq!(
            paths.get("halo3_mcc"),
            Some(&PathBuf::from("C:/Custom/H3EK"))
        );
        assert_eq!(
            paths.get("halo4_mcc"),
            Some(&PathBuf::from("C:/Steam/H4EK"))
        );
        assert_eq!(
            paths.get("haloce_evolved"),
            Some(&PathBuf::from("D:/Custom/Halo Campaign Evolved"))
        );
        assert_eq!(
            inputs.get("halo4_mcc").map(String::as_str),
            Some("C:/Steam/H4EK")
        );
        assert_eq!(attention, None);
    }

    #[test]
    fn save_as_registers_classic_ce_copy_in_loaded_folder() {
        let root = unique_test_dir("save-as-register-ce");
        let old_path = root.join("objects").join("old").join("old.gbxmodel");
        write_classic_ce_tag(&old_path, b"mod2");
        std::fs::create_dir_all(root.join("objects")).unwrap();

        let names = TagNameIndex::default();
        let old_entry = loose_file_entry(&root, &old_path, &names)
            .unwrap()
            .expect("old CE tag should probe");
        let entries = vec![old_entry.clone()];
        let mut source = LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: root.clone(),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names,
            game: None,
            entries: entries.clone(),
            tree: crate::source::build_folder_directory_tree(&root).unwrap(),
            group_tree: crate::source::build_group_tree(&entries),
            all_entries: entries,
            reverse_dependencies: None,
            initial_tag: None,
        };

        let saved_path = root.join("saved").join("cyborg.gbxmodel");
        write_classic_ce_tag(&saved_path, b"mod2");

        let registered = register_saved_copy_in_loaded_source(&mut source, &saved_path).unwrap();

        let _ = std::fs::remove_dir_all(root);
        assert!(registered);
        assert!(
            source
                .tree
                .children
                .iter()
                .any(|node| node.label == "saved")
        );
        assert!(source.entries.iter().any(|entry| {
            entry.display_path == "saved/cyborg.gbxmodel"
                && entry.group_tag == u32::from_be_bytes(*b"mod2")
        }));
        assert!(source.all_entries.iter().any(|entry| {
            entry.display_path == "saved/cyborg.gbxmodel"
                && entry.group_tag == u32::from_be_bytes(*b"mod2")
        }));
        assert!(source.group_tree.children.iter().any(|node| {
            node.entries
                .iter()
                .any(|&index| source.all_entries[index].display_path == "saved/cyborg.gbxmodel")
        }));
    }

    #[test]
    fn moved_tags_remap_favorite_relative_paths() {
        let root = PathBuf::from("C:/Games/H2EK/tags");
        let old_relative = PathBuf::from("objects/old/brute.model");
        let new_relative = PathBuf::from("objects/characters/brute/brute.model");
        let mut favorites = vec![old_relative.clone(), PathBuf::from("sound/brute.sound")];
        let mut remap = HashMap::new();
        remap.insert(
            format!("file:{}", root.join(&old_relative).display()),
            format!("file:{}", root.join(&new_relative).display()),
        );

        remap_favorite_paths(&root, &mut favorites, &remap);

        assert_eq!(favorites[0], new_relative);
        assert_eq!(favorites[1], PathBuf::from("sound/brute.sound"));
    }

    #[test]
    fn parse_steam_library_paths_reads_libraryfolders_vdf_paths() {
        let text = r#"
            "libraryfolders"
            {
                "0"
                {
                    "path"      "C:\\Program Files (x86)\\Steam"
                }
                "1"
                {
                    "path"      "D:\\SteamLibrary"
                }
            }
        "#;

        let paths = parse_steam_library_paths(text);

        assert!(paths.contains(&PathBuf::from(r"C:\Program Files (x86)\Steam")));
        assert!(paths.contains(&PathBuf::from(r"D:\SteamLibrary")));
    }

    #[test]
    fn ancestor_block_indices_splits_indexed_path() {
        // Nested blocks: each pair's path is the drawn `path_prefix` (parent
        // indices kept, own index dropped).
        assert_eq!(
            ancestor_block_indices("custom references[3]/sounds[1]/melee sound"),
            vec![
                ("custom references".to_owned(), 3),
                ("custom references[3]/sounds".to_owned(), 1),
            ],
        );
        // A plain struct segment between blocks carries no selection.
        assert_eq!(
            ancestor_block_indices("weapon[2]/melee/damage sound"),
            vec![("weapon".to_owned(), 2)],
        );
        // A top-level (unindexed) reference field has no ancestor blocks.
        assert_eq!(
            ancestor_block_indices("havok cleanup resources"),
            Vec::<(String, usize)>::new(),
        );
        assert_eq!(
            ancestor_block_indices("custom references#5[3]/sounds#2[1]/melee sound#4"),
            vec![
                ("custom references#5".to_owned(), 3),
                ("custom references#5[3]/sounds#2".to_owned(), 1),
            ],
        );
        // Foundation renders inherited wrappers without ordinals, so selector
        // IDs beneath Unit/Object must preserve those plain wrapper segments.
        assert_eq!(
            ancestor_block_indices("unit/object/functions#25[2]/import name#3"),
            vec![("unit/object/functions#25".to_owned(), 2)],
        );
        // Reference-jump paths may retain schema ordinals on inherited wrappers;
        // normalize those to the same selector ID as canonical Find paths.
        assert_eq!(
            ancestor_block_indices("unit#0/object#0/functions#25[2]/import name#3"),
            vec![("unit/object/functions#25".to_owned(), 2)],
        );
    }

    #[test]
    fn occurrence_label_keeps_indices_and_cleans_names() {
        assert_eq!(
            occurrence_label("custom references[3]/melee sound"),
            "custom references[3] › melee sound",
        );
        assert_eq!(
            occurrence_label("havok cleanup resources"),
            "havok cleanup resources"
        );
        assert_eq!(
            occurrence_label("custom references#5[3]/melee sound#4"),
            "custom references[3] › melee sound",
        );
    }

    #[test]
    fn normalize_ref_matches_dependency_key_form() {
        assert_eq!(
            normalize_ref("Sound/Materials/Hard/Human_Weap_Melee"),
            normalize_ref("sound\\materials\\hard\\human_weap_melee"),
        );
    }

    fn collect_terminal_output(input: &[u8]) -> Vec<(&'static str, String)> {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = egui::Context::default();
        let mut log_file = None;
        let mut log_error_reported = false;
        let result = stream_terminal_output(
            std::io::Cursor::new(input),
            &tx,
            &ctx,
            &mut log_file,
            &mut log_error_reported,
        );
        assert!(result.is_ok());
        drop(tx);

        rx.try_iter()
            .filter_map(|message| match message {
                WorkerMessage::TerminalLine(line) => Some(("line", line)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn terminal_output_appends_carriage_return_progress() {
        let output = collect_terminal_output(
            b"building bsp3d children... 10%\rbuilding bsp3d children... 70%\rbuilding bsp3d children... 100%\nnext\n",
        );

        assert_eq!(
            output,
            vec![
                ("line", "building bsp3d children... 10%".to_owned()),
                ("line", "building bsp3d children... 70%".to_owned()),
                ("line", "building bsp3d children... 100%".to_owned()),
                ("line", "next".to_owned()),
            ]
        );
    }

    #[test]
    fn terminal_output_treats_crlf_as_newline() {
        let output = collect_terminal_output(b"done\r\nnext");

        assert_eq!(
            output,
            vec![("line", "done".to_owned()), ("line", "next".to_owned()),]
        );
    }

    #[test]
    fn terminal_output_full_log_keeps_carriage_return_progress() {
        let path = std::env::temp_dir().join(format!(
            "baboon-terminal-test-{}.log",
            terminal_log_timestamp()
        ));
        let file = std::fs::File::create(&path);
        assert!(file.is_ok());

        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = egui::Context::default();
        let mut log_file = file.ok();
        let mut log_error_reported = false;
        let result = stream_terminal_output(
            std::io::Cursor::new(b"building... 10%\rbuilding... 60%\rbuilding... 100%\n"),
            &tx,
            &ctx,
            &mut log_file,
            &mut log_error_reported,
        );
        assert!(result.is_ok());
        drop(log_file);
        drop(tx);

        let output: Vec<_> = rx
            .try_iter()
            .filter_map(|message| match message {
                WorkerMessage::TerminalLine(line) => Some(("line", line)),
                _ => None,
            })
            .collect();
        assert_eq!(
            output,
            vec![
                ("line", "building... 10%".to_owned()),
                ("line", "building... 60%".to_owned()),
                ("line", "building... 100%".to_owned()),
            ]
        );

        let text = std::fs::read_to_string(&path);
        assert!(text.is_ok());
        if let Ok(text) = text {
            assert!(text.contains("building... 10%\n"));
            assert!(text.contains("building... 60%\n"));
            assert!(text.contains("building... 100%\n"));
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn terminal_output_handles_heavy_carriage_return_progress() {
        let mut input = Vec::new();
        for index in 0..25_000 {
            input.extend_from_slice(format!("building bsp3d children... {index}%\r").as_bytes());
        }
        input.extend_from_slice(b"done\n");

        let output = collect_terminal_output(&input);

        assert_eq!(output.len(), 25_001);
        assert_eq!(
            output.first(),
            Some(&("line", "building bsp3d children... 0%".to_owned()))
        );
        assert_eq!(output.last(), Some(&("line", "done".to_owned())));
    }

    #[test]
    fn terminal_line_severity_classifies_tool_markers() {
        assert!(matches!(
            TerminalLineEntry::new("-ERROR- bad connection".to_owned()).severity,
            TerminalLineSeverity::Error
        ));
        assert!(matches!(
            TerminalLineEntry::new("WARNING overlapping surfaces".to_owned()).severity,
            TerminalLineSeverity::Warning
        ));
        assert!(matches!(
            TerminalLineEntry::new("[exit 0]".to_owned()).severity,
            TerminalLineSeverity::Success
        ));
        assert!(matches!(
            TerminalLineEntry::new("[exit 2]".to_owned()).severity,
            TerminalLineSeverity::Error
        ));
        assert!(matches!(
            TerminalLineEntry::new("=== summary".to_owned()).severity,
            TerminalLineSeverity::Summary
        ));
    }

    #[test]
    fn terminal_visible_lines_are_trimmed_to_limit() {
        let mut lines = Vec::new();
        for index in 0..(TERMINAL_VISIBLE_LINE_LIMIT + 10) {
            lines.push(TerminalLineEntry::new(format!("line {index}")));
        }

        trim_terminal_lines(&mut lines);

        assert_eq!(lines.len(), TERMINAL_VISIBLE_LINE_TRIM_TARGET);
        assert_eq!(
            lines.first().map(|line| line.text.as_str()),
            Some("line 2010")
        );
    }
}

type DependencyCandidateIndex = HashMap<(u32, String), Vec<String>>;

#[derive(Default)]
struct DependencyFixReport {
    scanned: usize,
    fixed: usize,
    already_ok: usize,
    unresolved: usize,
    ambiguous: usize,
    skipped: usize,
    lines: Vec<String>,
}

impl DependencyFixReport {
    fn status(&self) -> String {
        if self.fixed > 0 {
            format!(
                "Fixed {} dependenc{} ({} unresolved, {} ambiguous)",
                self.fixed,
                if self.fixed == 1 { "y" } else { "ies" },
                self.unresolved,
                self.ambiguous
            )
        } else if self.unresolved == 0 && self.ambiguous == 0 {
            format!(
                "No broken dependencies found across {} reference(s)",
                self.scanned
            )
        } else {
            format!(
                "No dependencies auto-fixed ({} unresolved, {} ambiguous)",
                self.unresolved, self.ambiguous
            )
        }
    }
}

#[derive(Clone, Debug)]
struct TagReferenceUse {
    field_path: String,
    group_tag: u32,
    rel_path: String,
}

fn fix_tag_dependencies_in_tag(
    tag: &mut TagFile,
    tags_root: &Path,
    names: &TagNameIndex,
    index: &DependencyCandidateIndex,
) -> DependencyFixReport {
    let mut refs = Vec::new();
    collect_tag_references(tag.root(), "", &mut refs);

    let mut report = DependencyFixReport {
        scanned: refs.len(),
        lines: vec![format!(
            "Fix Tag Dependencies: scanned {} reference(s)",
            refs.len()
        )],
        ..Default::default()
    };
    let mut fixes = Vec::new();
    for reference in refs {
        let Some(extension) = names
            .name_for(reference.group_tag)
            .or_else(|| group_tag_to_extension(reference.group_tag))
        else {
            report.skipped += 1;
            report.lines.push(format!(
                "Skipped {}: unknown group {}",
                reference.field_path,
                format_group_tag(reference.group_tag)
            ));
            continue;
        };
        if dependency_target_exists(tags_root, &reference.rel_path, extension) {
            report.already_ok += 1;
            continue;
        }

        let leaf = dependency_leaf_key(&reference.rel_path);
        let key = (reference.group_tag, leaf.clone());
        let candidates = index.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        match candidates {
            [candidate] if !candidate.eq_ignore_ascii_case(&reference.rel_path) => {
                fixes.push((reference.clone(), candidate.clone()));
            }
            [] => {
                report.unresolved += 1;
                report.lines.push(format!(
                    "Unresolved {}: {}",
                    reference.field_path,
                    format_reference_path(names, reference.group_tag, &reference.rel_path)
                ));
            }
            _ => {
                report.ambiguous += 1;
                report.lines.push(format!(
                    "Ambiguous {}: {} candidate(s) named {}.{}",
                    reference.field_path,
                    candidates.len(),
                    leaf,
                    extension
                ));
            }
        }
    }

    for (reference, fixed_path) in fixes {
        let mut root = tag.root_mut();
        let Some(mut field) = root.field_path_mut(&reference.field_path) else {
            report.unresolved += 1;
            report.lines.push(format!(
                "Skipped {}: field path no longer resolves",
                reference.field_path
            ));
            continue;
        };
        let result = field.set(TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((reference.group_tag, fixed_path.clone())),
        }));
        match result {
            Ok(()) => {
                report.fixed += 1;
                report.lines.push(format!(
                    "Fixed {}: {} -> {}",
                    reference.field_path,
                    format_reference_path(names, reference.group_tag, &reference.rel_path),
                    format_reference_path(names, reference.group_tag, &fixed_path)
                ));
            }
            Err(error) => {
                report.unresolved += 1;
                report.lines.push(format!(
                    "Skipped {}: could not write dependency ({error:?})",
                    reference.field_path
                ));
            }
        }
    }

    report.lines.push(report.status());
    report
}

fn collect_tag_references(
    tag_struct: TagStruct<'_>,
    path_prefix: &str,
    refs: &mut Vec<TagReferenceUse>,
) {
    for field in tag_struct.fields() {
        let field_path = append_field_path_for(path_prefix, &field);
        match field.value() {
            Some(TagFieldData::TagReference(reference)) => {
                let Some((group_tag, rel_path)) = reference.group_tag_and_name else {
                    continue;
                };
                let rel_path = sanitize_ref_path(&rel_path).replace('/', "\\");
                if rel_path.is_empty() || rel_path.eq_ignore_ascii_case("none") {
                    continue;
                }
                refs.push(TagReferenceUse {
                    field_path,
                    group_tag,
                    rel_path,
                });
                continue;
            }
            Some(_) => continue,
            None => {}
        }
        if let Some(nested) = field.as_struct() {
            collect_tag_references(nested, &field_path, refs);
        } else if let Some(block) = field.as_block() {
            for (index, element) in block.iter().enumerate() {
                let element_path = format!("{field_path}[{index}]");
                collect_tag_references(element, &element_path, refs);
            }
        } else if let Some(array) = field.as_array() {
            for (index, element) in array.iter().enumerate() {
                let element_path = format!("{field_path}[{index}]");
                collect_tag_references(element, &element_path, refs);
            }
        }
    }
}

/// Collect just the reference *targets* in a tag, without the field-path
/// bookkeeping [`collect_tag_references`] does for the reference-jump UI.
/// Indexing walks every element of every block across the whole tag set, where
/// building a path string per visited field dominates the cost — and the
/// dependency index discards those paths.
fn collect_tag_dependency_refs(tag_struct: TagStruct<'_>, refs: &mut Vec<DependencyRef>) {
    for field in tag_struct.fields() {
        match field.value() {
            Some(TagFieldData::TagReference(reference)) => {
                let Some((group_tag, rel_path)) = reference.group_tag_and_name else {
                    continue;
                };
                let rel_path = sanitize_ref_path(&rel_path).replace('/', "\\");
                if rel_path.is_empty() || rel_path.eq_ignore_ascii_case("none") {
                    continue;
                }
                refs.push(DependencyRef {
                    group_tag,
                    rel_path,
                });
                continue;
            }
            Some(_) => continue,
            None => {}
        }
        if let Some(nested) = field.as_struct() {
            collect_tag_dependency_refs(nested, refs);
        } else if let Some(block) = field.as_block() {
            for element in block.iter() {
                collect_tag_dependency_refs(element, refs);
            }
        } else if let Some(array) = field.as_array() {
            for element in array.iter() {
                collect_tag_dependency_refs(element, refs);
            }
        }
    }
}

fn build_dependency_candidate_index(
    entries: &[TagEntry],
    names: &TagNameIndex,
) -> DependencyCandidateIndex {
    let mut index: DependencyCandidateIndex = HashMap::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let Some(rel_path) = dependency_entry_reference_path(entry, names) else {
            continue;
        };
        if !seen.insert((entry.group_tag, rel_path.to_ascii_lowercase())) {
            continue;
        }
        let leaf = dependency_leaf_key(&rel_path);
        index
            .entry((entry.group_tag, leaf))
            .or_default()
            .push(rel_path);
    }
    for candidates in index.values_mut() {
        candidates.sort();
    }
    index
}

#[derive(Default)]
struct ReferenceRewriteResult {
    references_changed: usize,
    tags_changed: usize,
    changed_keys: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
/// Rename/move a SINGLE tag and rewrite every reference to it, mirroring
/// [`run_folder_refactor_job`] for one file with an explicit new relative path
/// (no extension). Reuses the same reference-rewrite + key-remap machinery and
/// returns a [`FolderRefactorFinished`] so the existing finish handler applies
/// the in-memory update.
#[allow(clippy::too_many_arguments)]
fn run_tag_rename_job(
    root: PathBuf,
    entry: TagEntry,
    new_rel: String,
    job_label: String,
    names: TagNameIndex,
    game: Option<String>,
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
        game.as_deref()
            .and_then(|game| crate::source::load_reverse_dependency_index(game, &root))
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
        .as_deref()
        .map(|game| {
            locate_definitions_root()
                .join(game)
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
        key: format!("file:{}", new_path.display()),
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
    let tree = crate::source::build_folder_directory_tree(&root).map_err(|e| e.to_string())?;
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
    let status =
        format!("{verb} tag, updated {references_changed} reference(s) in {tags_changed} tag(s)");
    let lines = vec![
        format!(
            "{verb}: {} -> {}",
            entry.display_path, new_entry.display_path
        ),
        format!("Updated {references_changed} reference(s) in {tags_changed} tag(s)"),
    ];
    Ok(FolderRefactorFinished {
        status,
        lines,
        tree,
        all_entries,
        reverse_dependencies,
        old_to_new_keys,
        moved: true,
    })
}

fn run_folder_refactor_job(
    root: PathBuf,
    rel_path: PathBuf,
    destination_parent: PathBuf,
    move_folder: bool,
    label: String,
    names: TagNameIndex,
    game: Option<String>,
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
    let destination = lexical_normalize_path(&destination_parent.join(folder_name));
    if destination == source {
        return Err("Source and destination are the same folder".to_owned());
    }
    if destination.starts_with(&source) {
        return Err("Cannot move/copy a folder into itself".to_owned());
    }
    if destination.exists() {
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
        game.as_deref()
            .and_then(|game| crate::source::load_reverse_dependency_index(game, &root))
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
        .as_deref()
        .map(|game| {
            locate_definitions_root()
                .join(game)
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

    send_folder_refactor_progress(tx, &label, "Refreshing browser", None);
    let tree = crate::source::build_folder_directory_tree(&root).map_err(|e| e.to_string())?;
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

    let action = if move_folder { "Moved" } else { "Copied" };
    let status = format!(
        "{action} {} tag(s), updated {} reference(s) in {} tag(s)",
        old_entries.len(),
        references_changed,
        tags_changed
    );
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

    Ok(FolderRefactorFinished {
        status,
        lines,
        tree,
        all_entries,
        reverse_dependencies,
        old_to_new_keys,
        moved: move_folder,
    })
}

fn send_folder_refactor_progress(
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

fn validate_relative_folder_path(path: &Path) -> Result<PathBuf, String> {
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

fn copy_folder_recursive_progress(
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

fn build_folder_reference_rewrites(
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

fn transform_folder_entries(
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
                key: format!("file:{}", new_path.display()),
                display_path,
                group_tag: entry.group_tag,
                group_name: entry.group_name.clone(),
                location: TagEntryLocation::LooseFile(new_path),
            })
        })
        .collect()
}

fn merge_refactored_entries(
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

fn affected_move_rewrite_entries(
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
    entries.sort_by(|a, b| natural_entry_order(a).cmp(&natural_entry_order(b)));
    entries
}

fn natural_entry_order(entry: &TagEntry) -> String {
    entry.display_path.to_ascii_lowercase().replace('\\', "/")
}

fn rewrite_references_in_entries(
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
        let bytes = fs::read(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
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
        let mut tag =
            read_entry(source, entry).map_err(|error| format!("Could not parse tag: {error}"))?;
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
        tag.write_atomic(&path)
            .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
        result.references_changed += changed;
        result.tags_changed += 1;
        result.changed_keys.push(entry.key.clone());
    }
    Ok(result)
}

fn build_reverse_dependency_index(
    root: &Path,
    source: &TagSource,
    entries: &[TagEntry],
    label: &str,
    tx: &Sender<WorkerMessage>,
) -> ReverseDependencyIndex {
    let mut index = ReverseDependencyIndex::default();
    let total = entries.len();
    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_index == 0 || (entry_index + 1) % 50 == 0 || entry_index + 1 == total {
            let progress = if total == 0 {
                None
            } else {
                Some((entry_index + 1) as f32 / total as f32)
            };
            send_folder_refactor_progress(
                tx,
                label,
                &format!("Building dependency index {}/{}", entry_index + 1, total),
                progress,
            );
        }
        let deps = match read_entry_dependencies(source, entry) {
            Ok(deps) => deps,
            Err(error) => {
                let _ = tx.send(WorkerMessage::TerminalLine(format!(
                    "Warning: skipped dependency index for {}: {error}",
                    entry.display_path
                )));
                continue;
            }
        };
        index.set_tag_dependencies(entry.key.clone(), deps);
    }
    let _ = tx.send(WorkerMessage::TerminalLine(format!(
        "Built dependency index for {} tag(s) under {}",
        index.len(),
        root.display()
    )));
    index
}

fn refresh_reverse_dependency_index_after_refactor(
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

fn read_entry_dependencies(
    source: &TagSource,
    entry: &TagEntry,
) -> Result<Vec<DependencyRef>, String> {
    match &entry.location {
        // A loose tag usually carries a `want` (dependency-list) stream, which
        // is far cheaper to read than the whole tag.
        TagEntryLocation::LooseFile(path) => {
            if let Some(refs) = TagFile::read_dependency_references(path)
                .map_err(|error| format!("Could not read dependency list: {error}"))?
            {
                return Ok(refs
                    .into_iter()
                    .map(|(group_tag, rel_path)| DependencyRef {
                        group_tag,
                        rel_path: sanitize_ref_path(&rel_path).replace('/', "\\"),
                    })
                    .collect());
            }
        }
        // Cache and container tags have no separate dependency-list stream to
        // shortcut through (verified: none of Campaign Evolved's 12,291
        // container tags has a `want` chunk), so they fall through to the parse
        // path below — which `read_entry` supports for both.
        TagEntryLocation::Monolithic { .. } | TagEntryLocation::Container { .. } => {}
        // A brand-new tag exists only as an in-memory document; it has no
        // payload to read here, and it is not yet referenced by anything.
        TagEntryLocation::NewContainer { .. } => return Ok(Vec::new()),
    }
    let tag = read_entry(source, entry).map_err(|error| format!("Could not parse tag: {error}"))?;
    let mut refs = Vec::new();
    collect_tag_dependency_refs(tag.root(), &mut refs);
    Ok(refs)
}

#[cfg(test)]
mod container_dependency_tests {
    use super::*;

    const CE_PAKS: &str = "/Users/camden/Halo/halo-campaign-evolved_pc/Meteorite/Content/Paks";

    fn find_entry<'a>(
        loaded: &'a crate::source::LoadedSourceData,
        group: &[u8; 4],
        path: &str,
    ) -> &'a TagEntry {
        let group_tag = u32::from_be_bytes(*group);
        loaded
            .entries
            .iter()
            .chain(loaded.all_entries.iter())
            .find(|entry| {
                entry.group_tag == group_tag
                    && entry.display_path.to_ascii_lowercase().replace('\\', "/") == path
            })
            .unwrap_or_else(|| panic!("no {path} entry in the mounted containers"))
    }

    /// Container tags carry no `want` stream, so their dependencies have to come
    /// out of the parsed tag. This walks the real path end to end: a Campaign
    /// Evolved biped must report outbound references, and the reverse index they
    /// feed must resolve back to the referenced tag's own entry — i.e. the
    /// reference strings inside a container tag normalize to the same key as the
    /// entry display paths built from the pak directory. Skips without the paks.
    #[test]
    fn campaign_evolved_container_tags_report_their_dependencies() {
        let paks = PathBuf::from(CE_PAKS);
        if !paks.exists() {
            eprintln!("skip: CE paks not found");
            return;
        }
        let definitions = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let loaded =
            crate::source::load_iostore_container_set(paks, &TagNameIndex::default(), &definitions)
                .expect("mount CE container set");
        let names = TagNameIndex::load_game(&definitions, "haloce_evolved")
            .expect("load Campaign Evolved tag names");

        // The index build and every lookup read the complete set through
        // `full_entry_set`; a container mount keeps it in `entries`.
        assert!(loaded.all_entries.is_empty());
        assert_eq!(loaded.full_entry_set().len(), loaded.entries.len());
        assert!(
            loaded.full_entry_set().len() > 10_000,
            "expected the full CE tag set, got {}",
            loaded.full_entry_set().len()
        );

        let biped = find_entry(&loaded, b"bipd", "objects/characters/elite/elite.biped").clone();
        let deps = read_entry_dependencies(&loaded.source, &biped).expect("read biped deps");
        assert!(
            !deps.is_empty(),
            "elite.biped reported no dependencies; the container parse path is not running"
        );

        // The model reference must land on the entry the browser shows.
        let model = find_entry(&loaded, b"hlmt", "objects/characters/elite/elite.model").clone();
        let model_ref =
            dependency_entry_reference_path(&model, &names).expect("model reference path");
        let mut index = ReverseDependencyIndex::default();
        index.set_tag_dependencies(biped.key.clone(), deps);
        assert!(
            index
                .dependents_for(model.group_tag, &model_ref)
                .contains(&biped.key),
            "elite.model has no recorded referrer; container reference paths do not \
             normalize to entry display paths"
        );
    }

    /// The whole-corpus build, for when the cost of indexing containers is in
    /// question — it parses every tag. Ignored by default (minutes in a debug
    /// build); run with:
    ///   cargo test --release -- --ignored campaign_evolved_full_reference_index
    #[test]
    #[ignore = "parses all ~12k Campaign Evolved tags"]
    fn campaign_evolved_full_reference_index_resolves_referrers() {
        let paks = PathBuf::from(CE_PAKS);
        if !paks.exists() {
            eprintln!("skip: CE paks not found");
            return;
        }
        let definitions = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let loaded =
            crate::source::load_iostore_container_set(paks, &TagNameIndex::default(), &definitions)
                .expect("mount CE container set");
        let names = TagNameIndex::load_game(&definitions, "haloce_evolved")
            .expect("load Campaign Evolved tag names");

        let started = std::time::Instant::now();
        let mut index = ReverseDependencyIndex::default();
        let mut failed = 0usize;
        for entry in loaded.full_entry_set() {
            match read_entry_dependencies(&loaded.source, entry) {
                Ok(deps) => index.set_tag_dependencies(entry.key.clone(), deps),
                Err(_) => failed += 1,
            }
        }
        eprintln!(
            "[perf] indexed {} tags in {:.1?} ({failed} unreadable)",
            loaded.full_entry_set().len(),
            started.elapsed()
        );
        assert_eq!(failed, 0, "some container tags could not be read");

        // A shared tag must come back with many referrers, and the elite biped
        // must be among the referrers of its own model.
        let model = find_entry(&loaded, b"hlmt", "objects/characters/elite/elite.model").clone();
        let model_ref =
            dependency_entry_reference_path(&model, &names).expect("model reference path");
        let referrers = index.dependents_for(model.group_tag, &model_ref);
        assert!(
            !referrers.is_empty(),
            "elite.model has no referrers in the full index"
        );
        let unreferenced = loaded
            .full_entry_set()
            .iter()
            .filter(|entry| {
                dependency_entry_reference_path(entry, &names)
                    .map(|rel| index.dependents_for(entry.group_tag, &rel).is_empty())
                    .unwrap_or(false)
            })
            .count();
        eprintln!(
            "[perf] {unreferenced} of {} tags are unreferenced",
            loaded.full_entry_set().len()
        );
        assert!(
            unreferenced < loaded.full_entry_set().len() / 2,
            "most tags came back unreferenced ({unreferenced}); reference paths are \
             probably not matching entry paths"
        );
    }
}

fn rewrite_reference_needles(rewrites: &HashMap<(u32, String), String>) -> Vec<Vec<u8>> {
    let mut seen = HashSet::new();
    rewrites
        .keys()
        .filter_map(|(_, old_ref)| {
            let lowered = old_ref.replace('/', "\\").to_ascii_lowercase().into_bytes();
            (!lowered.is_empty() && seen.insert(lowered.clone())).then_some(lowered)
        })
        .collect()
}

fn bytes_contain_any_ascii_case_insensitive(bytes: &[u8], needles: &[Vec<u8>]) -> bool {
    if needles.is_empty() || bytes.is_empty() {
        return false;
    }
    let lowered = bytes.to_ascii_lowercase();
    needles
        .iter()
        .any(|needle| contains_subslice(&lowered, needle.as_slice()))
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn rewrite_references_in_tag(
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

fn moved_key_map(
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
            map.insert(entry.key.clone(), format!("file:{}", new_path.display()));
        }
    }
    map
}

fn same_entry_key(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.eq_ignore_ascii_case(b)
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn remap_favorite_paths(
    root: &Path,
    relative_paths: &mut [PathBuf],
    old_to_new_keys: &HashMap<String, String>,
) {
    for relative_path in relative_paths {
        let old_key = format!("file:{}", root.join(&*relative_path).display());
        let Some(new_key) = old_to_new_keys
            .iter()
            .find_map(|(old, new)| same_entry_key(old, &old_key).then_some(new))
        else {
            continue;
        };
        let Some(new_path) = new_key.strip_prefix("file:").map(PathBuf::from) else {
            continue;
        };
        if let Some(new_relative) = new_path
            .strip_prefix(root)
            .ok()
            .map(Path::to_path_buf)
            .and_then(clean_favorite_relative_path)
        {
            *relative_path = new_relative;
        }
    }
}

fn reference_path_from_abs_file(
    tags_root: &Path,
    path: &Path,
    group_tag: u32,
    names: &TagNameIndex,
) -> Option<String> {
    let rel = path.strip_prefix(tags_root).ok()?;
    reference_path_from_rel_file(rel, group_tag, names)
}

fn reference_path_from_rel_file(
    rel_file: &Path,
    group_tag: u32,
    names: &TagNameIndex,
) -> Option<String> {
    reference_path_without_group_extension(&rel_file.to_string_lossy(), group_tag, names)
}

#[cfg(test)]
mod tsv_paste_tests {
    use super::*;

    #[test]
    fn header_maps_case_insensitively_reordered_and_ignores_unknown() {
        let columns = vec![
            ("material name".to_owned(), "material name^".to_owned()),
            ("sweetener mode".to_owned(), "sweetener mode".to_owned()),
        ];
        // Reordered, mixed case, plus an unknown column.
        let mapped = map_tsv_header_to_fields("Sweetener Mode\tbogus\tmaterial name", &columns);
        assert_eq!(
            mapped,
            vec![
                Some("sweetener mode".to_owned()),
                None,
                Some("material name^".to_owned()),
            ]
        );
    }
}

#[cfg(test)]
mod field_search_tests {
    use super::*;

    #[test]
    fn searchable_text_separator_only_appears_between_values() {
        let mut blob = String::new();

        append_searchable_text(&mut blob, "First");
        assert_eq!(blob, "first");

        append_searchable_text(&mut blob, "Second");
        assert_eq!(blob, "first · second");

        append_searchable_text(&mut blob, "Third");
        assert_eq!(blob, "first · second · third");
        assert!(!blob.starts_with(" · "));
        assert!(!blob.contains(" ·  · "));
    }

    #[test]
    fn searchable_text_extracts_text_kinds_only() {
        assert_eq!(
            field_searchable_text(Some(TagFieldData::String("Hello".to_owned()))).as_deref(),
            Some("Hello")
        );
        assert_eq!(
            field_searchable_text(Some(TagFieldData::CharEnum {
                value: 1,
                name: Some("alert".to_owned()),
            }))
            .as_deref(),
            Some("alert")
        );
        // Numbers / padding carry no searchable text.
        assert_eq!(
            field_searchable_text(Some(TagFieldData::LongInteger(42))),
            None
        );
        assert_eq!(field_searchable_text(None), None);
    }

    #[test]
    fn first_match_finds_a_string_id_value_and_path() {
        let mut tag = TagFile::new("definitions/halo2_mcc/model.json").unwrap();
        let mut dirty = Dirty::default();
        apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Create {
                name: "myhero".to_owned(),
                regions: Vec::new(),
            }],
            &mut dirty,
        );
        let hit = first_field_value_match(&tag.root(), "hero", "");
        let (path, value) = hit.expect("variant name should match 'hero'");
        assert!(value.to_ascii_lowercase().contains("hero"));
        assert!(path.to_ascii_lowercase().contains("variant"));
        assert!(
            first_field_value_match(&tag.root(), "zzz-not-present", "").is_none(),
            "absent text should not match"
        );
    }
}

#[cfg(test)]
mod dependency_tests {
    use super::*;

    fn entry(display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: format!("file:{display_path}"),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
        }
    }

    fn abs_entry(root: &Path, display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: format!("file:{}", root.join(display_path).display()),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: None,
            location: TagEntryLocation::LooseFile(root.join(display_path)),
        }
    }

    fn container_entry(key: &str, display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: key.to_owned(),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: None,
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: format!("Tags/{display_path}.ubulk"),
            },
        }
    }

    fn loose_source_with_counts(label: &str, entries: Vec<TagEntry>) -> LoadedSourceData {
        LoadedSourceData {
            label: label.to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("C:/kit/tags"),
                game: Some("halo3_mcc".to_owned()),
                definitions_root: PathBuf::from("C:/kit/definitions"),
            },
            names: TagNameIndex::default(),
            game: Some("halo3_mcc".to_owned()),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: entries,
            reverse_dependencies: None,
            initial_tag: None,
        }
    }

    #[test]
    fn loose_folder_status_does_not_report_zero_loaded_tags_before_scan() {
        let source = loose_source_with_counts("H3EK/tags (halo3_mcc)", Vec::new());

        assert_eq!(
            loaded_source_status(&source),
            "Browsing tags from H3EK/tags (halo3_mcc)"
        );
    }

    #[test]
    fn loose_folder_status_uses_recursive_index_when_available() {
        let shader = parse_group_tag("rmsh").unwrap();
        let source = loose_source_with_counts(
            "H3EK/tags (halo3_mcc)",
            vec![
                entry("objects/a.shader", shader),
                entry("objects/b.shader", shader),
            ],
        );

        assert_eq!(
            loaded_source_status(&source),
            "Found 2 tag(s) in H3EK/tags (halo3_mcc)"
        );
    }

    #[test]
    fn dependency_entry_reference_path_strips_only_group_extension() {
        let names = TagNameIndex::default();
        let bitmap = parse_group_tag("bitm").unwrap();
        let entry = entry("objects/weapons/decal_road_1.bitmap.bitmap", bitmap);

        assert_eq!(
            dependency_entry_reference_path(&entry, &names).unwrap(),
            "objects\\weapons\\decal_road_1.bitmap"
        );
    }

    #[test]
    fn container_reference_resolution_uses_group_and_normalized_path() {
        let names = TagNameIndex::default();
        let render_model = parse_group_tag("mode").unwrap();
        let weapon = parse_group_tag("weap").unwrap();
        let display_stem = "objects/shared/example";
        let entries = vec![
            entry(&format!("{display_stem}.render_model"), render_model),
            container_entry(
                "ublock:shared:model",
                &format!("{display_stem}.render_model"),
                render_model,
            ),
            container_entry(
                "ublock:shared:weapon",
                &format!("{display_stem}.weapon"),
                weapon,
            ),
        ];

        let model = container_entry_for_reference(
            &entries,
            render_model,
            "OBJECTS/SHARED/EXAMPLE.RENDER_MODEL",
            &names,
        )
        .expect("render-model reference should resolve");
        assert_eq!(model.key, "ublock:shared:model");

        let weapon_entry =
            container_entry_for_reference(&entries, weapon, r"objects\shared\example", &names)
                .expect("same path in another group should resolve independently");
        assert_eq!(weapon_entry.key, "ublock:shared:weapon");

        assert!(
            container_entry_for_reference(
                &entries,
                render_model,
                r"objects\shared\missing",
                &names
            )
            .is_none()
        );
    }

    /// A tag created this session is a reference target like any other: it is
    /// addressed by the same logical path, and "Open referenced tag" resolves
    /// through this lookup. Excluding it reported the tag as missing.
    #[test]
    fn container_reference_resolution_finds_an_unsaved_new_tag() {
        let names = TagNameIndex::default();
        let camera_track = parse_group_tag("trak").unwrap();
        let entries = vec![new_container_entry(
            "test/example.camera_track",
            camera_track,
            "camera_track",
        )];

        let found = container_entry_for_reference(
            &entries,
            camera_track,
            r"test\example.camera_track",
            &names,
        )
        .expect("a new tag should resolve as a reference target");
        assert_eq!(found.key, "newtag:/Game/Tags/test/example-camera_track");
    }

    /// The capability matrix for a brand-new container tag, in one place: what
    /// it can do, and the two things it deliberately cannot. Every one of these
    /// gates gets its answer from a `match` on `TagEntryLocation`, and each one
    /// that forgot the `NewContainer` arm broke the tag in a different way —
    /// the editability gate made every field and block button inert.
    #[test]
    fn a_new_container_tag_has_the_expected_capabilities() {
        let camera_track = parse_group_tag("trak").unwrap();
        let entry = new_container_entry("test/example.camera_track", camera_track, "camera_track");
        let tag = TagFile::new("definitions/haloce_evolved/camera_track.json").unwrap();

        assert!(
            crate::app::is_editable_tag(&entry, &tag),
            "fields and block controls must be live for a new tag"
        );
        assert!(
            crate::app::supports_rename_menu(&entry),
            "rename/move is the only way to correct a mistyped new-tag path"
        );
        // No `.ubulk` behind it, so there is nothing to pull out.
        assert!(
            !crate::app::is_embedded_tag_entry(&entry),
            "a new tag has no embedded payload to extract"
        );
    }

    fn new_container_entry(display_path: &str, group_tag: u32, group_name: &str) -> TagEntry {
        let logical = display_path
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(display_path);
        let package = new_container_package(logical, group_name);
        TagEntry {
            key: new_container_key(&package),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: Some(group_name.to_owned()),
            location: TagEntryLocation::NewContainer {
                template_container: 0,
                template_rel: "Tags/other-camera_track.uasset".to_owned(),
                package,
                group_tag,
            },
        }
    }

    /// Renaming a new tag must land on exactly the key and package that
    /// creating it at that path would have produced — the save and project-
    /// overlay paths identify the tag by them, so a second derivation that
    /// drifted would strand the renamed tag.
    #[test]
    fn renaming_a_new_tag_derives_the_same_identity_as_creating_it_there() {
        let created = new_container_package("objects/foo/bar", "camera_track");
        assert_eq!(created, "/Game/Tags/objects/foo/bar-camera_track");
        assert_eq!(
            new_container_key(&created),
            "newtag:/Game/Tags/objects/foo/bar-camera_track"
        );

        // The rename path normalizes its input first — backslashes, case, and
        // stray separators must not fork the identity.
        let renamed = new_container_package(
            &normalize_container_tag_rel("/Objects\\Foo//Bar/"),
            "camera_track",
        );
        assert_eq!(renamed, created);
    }

    #[test]
    fn dependency_candidate_index_matches_by_group_and_leaf_name() {
        let names = TagNameIndex::default();
        let bitmap = parse_group_tag("bitm").unwrap();
        let shader = parse_group_tag("rmsh").unwrap();
        let entries = vec![
            entry("objects/new/run.bitmap", bitmap),
            entry("objects/new/run.shader", shader),
        ];

        let index = build_dependency_candidate_index(&entries, &names);

        assert_eq!(
            index
                .get(&(bitmap, "run".to_owned()))
                .cloned()
                .unwrap_or_default(),
            vec!["objects\\new\\run".to_owned()]
        );
        assert_eq!(
            index
                .get(&(shader, "run".to_owned()))
                .cloned()
                .unwrap_or_default(),
            vec!["objects\\new\\run".to_owned()]
        );
    }

    #[test]
    fn folder_reference_rewrites_point_moved_tags_at_new_folder() {
        let names = TagNameIndex::default();
        let bitmap = parse_group_tag("bitm").unwrap();
        let root = Path::new("C:/kit/tags");
        let source = root.join("objects/old");
        let destination = root.join("objects/new/old");
        let entries = vec![abs_entry(
            root,
            "objects/old/decal_road_1.bitmap.bitmap",
            bitmap,
        )];

        let rewrites =
            build_folder_reference_rewrites(root, &source, &destination, &entries, &names);

        assert_eq!(
            rewrites
                .get(&(bitmap, "objects\\old\\decal_road_1.bitmap".to_owned()))
                .cloned(),
            Some("objects\\new\\old\\decal_road_1.bitmap".to_owned())
        );
    }

    #[test]
    fn rewrite_reference_prefilter_matches_ascii_case_insensitively() {
        let shader = parse_group_tag("rmsh").unwrap();
        let mut rewrites = HashMap::new();
        rewrites.insert(
            (shader, "objects\\characters\\bugger\\bugger".to_owned()),
            "zoeph_test\\bugger\\bugger".to_owned(),
        );
        let needles = rewrite_reference_needles(&rewrites);

        assert!(bytes_contain_any_ascii_case_insensitive(
            b"xx OBJECTS\\CHARACTERS\\BUGGER\\BUGGER yy",
            &needles
        ));
        assert!(!bytes_contain_any_ascii_case_insensitive(
            b"objects\\characters\\dervish\\dervish",
            &needles
        ));
    }

    #[test]
    fn affected_move_entries_include_moved_tags_and_external_dependents() {
        let shader = parse_group_tag("rmsh").unwrap();
        let model = parse_group_tag("hlmt").unwrap();
        let old_shader = entry("objects/characters/jackal/jackal.shader", shader);
        let new_shader = entry("zoeph_test/jackal/jackal.shader", shader);
        let outside_model = entry("objects/characters/shared/shared.model", model);
        let unrelated = entry("objects/characters/brute/brute.model", model);
        let all_entries = vec![old_shader.clone(), outside_model.clone(), unrelated];
        let old_entries = vec![old_shader.clone()];
        let new_entries = vec![new_shader.clone()];
        let mut rewrites = HashMap::new();
        rewrites.insert(
            (shader, "objects\\characters\\jackal\\jackal".to_owned()),
            "zoeph_test\\jackal\\jackal".to_owned(),
        );
        let mut reverse = ReverseDependencyIndex::default();
        reverse.set_tag_dependencies(
            outside_model.key.clone(),
            vec![DependencyRef {
                group_tag: shader,
                rel_path: "objects\\characters\\jackal\\jackal".to_owned(),
            }],
        );
        reverse.set_tag_dependencies(
            old_shader.key.clone(),
            vec![DependencyRef {
                group_tag: shader,
                rel_path: "objects\\characters\\jackal\\jackal".to_owned(),
            }],
        );

        let affected = affected_move_rewrite_entries(
            &all_entries,
            &old_entries,
            &new_entries,
            &rewrites,
            Some(&reverse),
        );
        let affected_keys = affected
            .into_iter()
            .map(|entry| entry.key)
            .collect::<HashSet<_>>();

        assert_eq!(affected_keys.len(), 2);
        assert!(affected_keys.contains(&new_shader.key));
        assert!(affected_keys.contains(&outside_model.key));
    }
}
