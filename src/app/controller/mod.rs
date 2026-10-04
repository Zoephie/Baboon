//! Application actions and asynchronous workflow coordination for [`Baboon`].
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;
use crate::app::tag_ops::ENTRY_INDEX_REFRESH_INTERVAL_SECS;
use crate::app::mods::in_place::ContainerSaveRoute;
use crate::app::mods::in_place::container_save_route;
use anyhow::Context as _;

mod updates;
use updates::*;
pub(in crate::app) mod terminal;
pub(super) use terminal::open_terminal_log;
#[cfg(test)]
use terminal::terminal_log_timestamp;
use terminal::{
    TerminalStopResult, append_terminal_log_path, create_terminal_log_file,
    run_terminal_command_for_reimport, send_terminal_line, stop_terminal_process,
    stream_terminal_output, terminal_shell_command, trim_terminal_lines,
};
mod tools;
mod kit_tool_options;
use kit_tool_options::*;
pub(super) use tools::add_standard_editing_kit_profiles;
use tools::*;
mod scenario_launch;
mod tool_drop;
pub(super) use tool_drop::KIT_TOOL_DROP_CURSOR;
// Re-exported: the browser's row menus gate on this, and its drawing functions
// reach it through egui memory rather than through `Baboon`.
use scenario_launch::*;
pub(super) use scenario_launch::{
    ScenarioLaunchAvailability, scenario_launch_availability, scenario_launch_availability_with,
};
pub(in crate::app) mod saving;
pub(super) use saving::available_definition_games;
use saving::{
    ordered_unique_keys, save_as_extension, save_as_file_name, save_as_start_dir,
};
mod documents;
pub(in crate::app) mod loading;
pub(super) use crate::core::created_tags::{CreatedTagLedger, CreatedTagRecord};
mod group_report;

#[cfg(any(windows, test))]
fn explorer_select_args(path: &Path) -> [std::ffi::OsString; 2] {
    [
        std::ffi::OsString::from("/select,"),
        path.as_os_str().to_owned(),
    ]
}

fn loose_folder_explorer_path(tags_root: &Path, requested: &Path) -> PathBuf {
    if requested.is_absolute() || looks_like_absolute_windows_path(requested) {
        requested.to_path_buf()
    } else {
        tags_root.join(requested)
    }
}

/// `Path::is_absolute` follows the host platform, but favorite-folder actions
/// can carry an Explorer path while this pure helper is exercised by Unix CI.
/// Recognize the Windows forms explicitly so an already-rooted favorite is
/// never appended to whichever kit happens to be active.
fn looks_like_absolute_windows_path(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    let bytes = text.as_bytes();
    let drive_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let network_or_device_absolute =
        bytes.len() >= 2 && matches!(bytes[0], b'\\' | b'/') && matches!(bytes[1], b'\\' | b'/');
    drive_absolute || network_or_device_absolute
}

fn extraction_scope_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_ascii_lowercase()
}

fn scope_contains(parent: &str, child: &str) -> bool {
    parent.is_empty()
        || child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn mark_tree_loaded(tree: &mut TagTree) {
    fn mark(node: &mut TagTreeNode) {
        node.children_loaded = true;
        node.entries_loaded = true;
        for child in &mut node.children {
            mark(child);
        }
    }
    for node in &mut tree.children {
        mark(node);
    }
}

fn find_tree_node_mut<'a>(
    nodes: &'a mut [TagTreeNode],
    wanted: &str,
) -> Option<&'a mut TagTreeNode> {
    for node in nodes {
        if extraction_scope_key(&node.rel_path) == wanted {
            return Some(node);
        }
        if let Some(found) = find_tree_node_mut(&mut node.children, wanted) {
            return Some(found);
        }
    }
    None
}

/// Replace the overlap between a browser tree rooted at `tree_root` and one
/// recursively loaded extraction scope. Entry indices continue to address the
/// shared lazy `entries` vector.
fn replace_loaded_tree_scope(
    tree: &mut TagTree,
    tree_root: &Path,
    loaded_scope: &Path,
    entries: &[TagEntry],
) {
    let root_key = extraction_scope_key(tree_root);
    let scope_key = extraction_scope_key(loaded_scope);
    if scope_contains(&scope_key, &root_key) {
        *tree = crate::core::source::build_tree_beneath(entries, tree_root);
        mark_tree_loaded(tree);
    } else if scope_contains(&root_key, &scope_key) {
        let mut replacement = crate::core::source::build_tree_beneath(entries, loaded_scope);
        mark_tree_loaded(&mut replacement);
        if let Some(node) = find_tree_node_mut(&mut tree.children, &scope_key) {
            node.children = replacement.children;
            node.entries = replacement.entries;
            node.children_loaded = true;
            node.entries_loaded = true;
        }
    }
}

#[cfg(test)]
mod folder_extractable_tree_tests;

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
    ) -> Option<std::sync::Arc<crate::core::source::ce_audio::CeSoundBinding>> {
        use crate::core::source::ce_audio;

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
    ) -> Option<std::sync::Arc<crate::core::source::ce_audio::CeSoundBinding>> {
        use crate::core::source::ce_audio;

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
    ) -> Option<std::sync::Arc<crate::core::source::ce_audio::CeSoundBinding>> {
        use crate::core::source::ce_audio;

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

        // The pak set is needed for events whose media is cooked inside a
        // SoundBank. A decode worker holds the lock only while it reads a
        // file's bytes.
        let store = self.audio.ce_media.clone();
        let mut store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let binding = std::sync::Arc::new(ce_audio::resolve_sound_binding(
            containers,
            packages,
            &usmap,
            package,
            Some((root.as_path(), &mut store)),
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
        let Some((kit_id, tab_key, request)) = self.pending_ce_sound_ref.take() else {
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
        let media: Vec<crate::core::source::ce_audio::CeSoundMedia> = binding
            .media_for_language(&language)
            .into_iter()
            .cloned()
            .collect();
        if !request.extract {
            // Play the first permutation, matching the loose-folder player.
            let Some(first) = media.into_iter().next() else {
                return;
            };
            self.audio.pending.push_back(crate::app::audio::SoundRequest {
                owner: Some(crate::app::audio::SoundOwner {
                    kit: kit_id,
                    key: tab_key,
                }),
                clip: request.clip.clone(),
                preview: request.preview,
                action: crate::app::audio::SoundAction::PlayCeMedia {
                    paks_root,
                    label: format!("{} \u{00B7} {}", request.label, first.display_name()),
                    media: Box::new(first),
                },
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
            .map(|m| crate::app::export::sound_extract::ExtractItem {
                out_path: base.join(format!(
                    "{}.wav",
                    crate::app::export::sound_extract::sanitize_component(&m.display_name())
                )),
                source: crate::app::export::sound_extract::ExtractSource::CeMedia {
                    paks_root: paks_root.clone(),
                    media: Box::new(m),
                },
            })
            .collect();
        self.pending_sound_extract = Some(crate::app::export::sound_extract::ExtractRequest {
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
            self.apply_worker_message(message, ctx);
        }
    }

    /// Apply one worker result. Handlers drop a result whose source is stale
    /// themselves; what they return about it is not used here.
    pub(super) fn apply_worker_message(&mut self, message: WorkerMessage, ctx: &egui::Context) {
        {
            let _stale = match message {
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
                WorkerMessage::ScenarioPalettesRead { game, palettes } => {
                    let table = palettes.map_or(PaletteTable::Unreadable, PaletteTable::Ready);
                    self.kit_tool_drag.palettes.insert(game, table);
                    // A drag hovering Sapien with the mouse held still gets
                    // no event of its own to redraw the palette name with.
                    ctx.request_repaint();
                    false
                }
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
                WorkerMessage::SourceListingReady { stamp, results } => {
                    self.handle_source_listing_ready(stamp, results)
                }
                WorkerMessage::ReverseDependenciesBuilt {
                    stamp,
                    index,
                    missing,
                } => self.handle_reverse_dependencies_built(stamp, index, missing),
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
                WorkerMessage::ChimpReferrersScanned {
                    stamp,
                    package,
                    scan,
                } => self.handle_chimp_referrers_scanned(stamp, package, scan),
                WorkerMessage::ChimpModBuilt {
                    kit,
                    output,
                    temporary,
                    written,
                    result,
                } => self.handle_chimp_mod_built(kit, output, temporary, written, result, ctx),
                WorkerMessage::ChimpSourcesOverwritten {
                    kit,
                    leases,
                    containers,
                    touched,
                    written,
                    result,
                } => self.handle_chimp_sources_overwritten(
                    kit, leases, containers, touched, written, result, ctx,
                ),
                WorkerMessage::TagCompareGit { request, update } => {
                    self.handle_tag_compare_git(request, update)
                }
                WorkerMessage::GitReviewUpdated { kit, request, view } => {
                    self.handle_git_review_updated(kit, request, view)
                }
                WorkerMessage::ChimpPackageLoaded {
                    stamp,
                    package,
                    result,
                } => self.handle_chimp_package_loaded(stamp, package, result),
                WorkerMessage::TagLoaded { kit, key, result } => {
                    self.handle_tag_loaded(kit, key, result)
                }
                WorkerMessage::RefJumpOccurrences {
                    kit,
                    index,
                    key,
                    target,
                    result,
                } => self.handle_ref_jump_occurrences(kit, index, key, target, result),
                WorkerMessage::BitmapReimportFinished { kit, key, result } => {
                    self.handle_bitmap_reimport_finished(kit, key, result)
                }
                WorkerMessage::BlamImportProgress {
                    stamp,
                    kind,
                    message,
                } => self.handle_blam_import_progress(stamp, kind, message),
                WorkerMessage::BlamImportFinished {
                    stamp,
                    outcomes,
                    created,
                } => self.handle_blam_import_finished(stamp, outcomes, created),
                WorkerMessage::ContainerDuplicateFinished {
                    stamp,
                    lease,
                    result,
                } => self.handle_container_duplicate_finished(stamp, lease, result, ctx),
                WorkerMessage::ContainerRenameFinished {
                    stamp,
                    lease,
                    result,
                } => self.handle_container_rename_finished(stamp, lease, result, ctx),
                WorkerMessage::InPlaceOverwriteFinished {
                    job,
                    lease,
                    written,
                } => self.handle_in_place_overwrite_finished(*job, lease, written),
                WorkerMessage::ContainerDeleteFinished {
                    stamp,
                    lease,
                    result,
                } => self.handle_container_delete_finished(stamp, lease, result),
                WorkerMessage::ChimpLevelProgress {
                    kit,
                    phase,
                    done,
                    total,
                } => self.handle_chimp_level_progress(kit, phase, done, total),
                WorkerMessage::ContainerDumpProgress { stamp, done, total } => {
                    self.handle_container_dump_progress(stamp, done, total)
                }
                WorkerMessage::ContainerDumpFinished { stamp, result } => {
                    self.handle_container_dump_finished(stamp, result)
                }
                WorkerMessage::ExportFinished(result) => self.handle_export_finished(result),
                WorkerMessage::ChimpLevelExportFinished { job, result } => {
                    self.handle_chimp_level_export_finished(job, result)
                }
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
                WorkerMessage::PokeUndoFinished { result, unapplied } => {
                    self.handle_poke_undo(result, unapplied);
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
                    self.handle_folder_conversion_finished(report, ctx)
                }
                WorkerMessage::CacheImportProgress(progress) => {
                    self.handle_cache_import_progress(progress)
                }
                WorkerMessage::CacheImportFinished { stamp, result } => {
                    self.handle_cache_import_finished(stamp, result, ctx)
                }
                WorkerMessage::CacheImportConflicts { stamp, conflicts } => {
                    self.handle_cache_import_conflicts(stamp, conflicts)
                }
                WorkerMessage::ImportSourceResolved { input, result } => {
                    self.handle_import_source_resolved(input, result)
                }
                WorkerMessage::ImportAnalysisFinished { result, templates } => {
                    self.handle_import_analysis_finished(result, templates, ctx)
                }
                WorkerMessage::ModelTexturesResolved {
                    stamp,
                    key,
                    textures_id,
                    textures,
                } => self.handle_model_textures_resolved(stamp, key, textures_id, textures),
                WorkerMessage::BitmapThumbnailDecoded { stamp, key, result } => {
                    self.handle_thumbnail_ready::<Bitmaps>(stamp, key, result, ctx)
                }
                WorkerMessage::ModelThumbnailRendered { stamp, key, result } => {
                    self.handle_thumbnail_ready::<Models>(stamp, key, result, ctx)
                }
                WorkerMessage::ModelPreviewLoaded {
                    stamp,
                    key,
                    request_id,
                    result,
                } => self.handle_model_preview_loaded(stamp, key, request_id, result),
                WorkerMessage::ModelOverlaysBuilt {
                    stamp,
                    key,
                    geometry_id,
                    collision,
                    physics,
                } => self.handle_model_overlays_built(stamp, key, geometry_id, collision, physics),
                WorkerMessage::ModelAnimationsListed { stamp, key, result } => {
                    self.handle_model_animations_listed(stamp, key, result)
                }
                WorkerMessage::ModelAnimationDecoded {
                    stamp,
                    key,
                    animation_index,
                    result,
                } => self.handle_model_animation_decoded(stamp, key, animation_index, result),
                WorkerMessage::AllEntriesScanned { stamp, result } => {
                    self.handle_all_entries_scanned(stamp, result, ctx)
                }
                WorkerMessage::FolderExtractablesLoaded {
                    stamp,
                    rel_path,
                    label,
                    result,
                } => self.handle_folder_extractables_loaded(stamp, rel_path, label, result),
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
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_single_file(path, &names).map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: None,
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
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
        if let Some(paks) = crate::core::source::find_paks_dir(&path) {
            // Remember the folder the user picked, not the container directory
            // found inside it — the same way a loose kit remembers its root
            // rather than the `tags/` subfolder it actually scans.
            self.begin_load_iostore_container_set_path(paks, path, ctx);
            return;
        }
        // A kit's own tags folder opens that kit, with the data folder and
        // tool options it names, rather than as a bare folder whose data
        // folder would be guessed.
        if let Some(profile) = self.profile_using_chosen_tags_folder(&path) {
            self.load_custom_editing_kit_profile(profile, ctx);
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
        let ek_folder_aliases = self.prefs.ek_folder_aliases.clone();
        let folder_info = match resolve_folder_root(&path, &ek_folder_aliases) {
            Ok(info) => info,
            Err(error) => {
                self.release_source_load(kit);
                self.status = error.to_string();
                return;
            }
        };
        self.status = match folder_info.game {
            Some(game) => format!("Indexing {} as {game}", folder_info.scan_root.display()),
            None => format!("Indexing {}", folder_info.scan_root.display()),
        };
        let recent_path = clean_recent_path(path.clone());
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_folder(path, &names, &definitions_root, &ek_folder_aliases)
                    .map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
    }

    /// The profile that chose `path` as its tags folder, if any. Read from the
    /// validated layouts already cached, so asking costs no disk access.
    fn profile_using_chosen_tags_folder(&self, path: &Path) -> Option<CustomEditingKitProfile> {
        let path = canonical_or_clean(path);
        self.prefs
            .custom_editing_kit_profiles
            .iter()
            .filter(|profile| profile.has_chosen_folders())
            .find(|profile| {
                self.editing_kit_validation
                    .custom(&profile.id)
                    .is_ok_and(|layout| same_recent_path(&layout.tags, &path))
            })
            .cloned()
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
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_monolithic_blob_index(path, &names).map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
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
            self.prefs
                .custom_editing_kit_profiles
                .iter()
                .filter(|profile| profile.is_campaign_evolved())
                .find_map(|profile| crate::core::source::find_paks_dir(&profile.root))
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
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_iostore_container(path, pak_root, &names, &definitions_root)
                    .map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
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
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_iostore_container_set(paks_dir, &names, &definitions_root)
                    .map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
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
        self.prefs
            .recent_folders
            .retain(|existing| !same_recent_path(existing, &path));
        self.prefs.recent_folders.insert(0, path);
        self.prefs.recent_folders.truncate(MAX_RECENT_FOLDERS);
    }

    pub(super) fn remove_recent_folder(&mut self, path: &Path) {
        self.prefs
            .recent_folders
            .retain(|existing| !same_recent_path(existing, path));
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
        self.prefs
            .editing_kit_favorites
            .iter()
            .position(|kit| same_recent_path(&kit.tags_root, root))
    }

    /// Rebuild `kit`'s resolved favorite entries from the saved paths for its
    /// tags root. Kit-scoped because a finished background refactor refreshes
    /// the workspace it belonged to, which need not be the focused one.
    pub(in crate::app) fn refresh_favorite_entries_for(&mut self, kit: usize) {
        self.kits[kit].active_favorite_entries.clear();
        self.kits[kit].active_favorite_folders.clear();
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
        let saved_paths = self.prefs.editing_kit_favorites[index].tags.clone();
        let saved_folders = self.prefs.editing_kit_favorites[index].folders.clone();
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
        let mut missing_folders = Vec::new();
        for relative_path in saved_folders {
            if root.join(&relative_path).is_dir() {
                self.kits[kit].active_favorite_folders.push(relative_path);
            } else {
                missing_folders.push(relative_path);
            }
        }
        if !missing.is_empty() || !missing_folders.is_empty() {
            let favorites = &mut self.prefs.editing_kit_favorites[index];
            favorites.tags.retain(|path| {
                !missing
                    .iter()
                    .any(|missing| same_recent_path(missing, path))
            });
            favorites.folders.retain(|path| {
                !missing_folders
                    .iter()
                    .any(|missing| same_recent_path(missing, path))
            });
            if favorites.tags.is_empty() && favorites.folders.is_empty() {
                self.prefs.editing_kit_favorites.remove(index);
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
            self.prefs.editing_kit_favorites.push(EditingKitFavorites {
                tags_root: clean_recent_path(root.clone()),
                tags: Vec::new(),
                folders: Vec::new(),
            });
            self.prefs.editing_kit_favorites.len() - 1
        });
        let kit = &mut self.prefs.editing_kit_favorites[index];
        if let Some(position) = kit
            .tags
            .iter()
            .position(|current| same_recent_path(current, &relative_path))
        {
            kit.tags.remove(position);
            self.kits[self.active]
                .active_favorite_entries
                .retain(|favorite| favorite.key != entry.key);
            if kit.tags.is_empty() && kit.folders.is_empty() {
                self.prefs.editing_kit_favorites.remove(index);
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

    fn toggle_folder_favorite(&mut self, rel_path: &Path) {
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Only loose editing-kit folders can be favorited".to_owned();
            return;
        };
        let Some(relative_path) = clean_favorite_relative_path(rel_path.to_path_buf()) else {
            self.status = "Could not resolve the folder inside the loaded tags folder".to_owned();
            return;
        };
        if !root.join(&relative_path).is_dir() {
            self.status = format!("Folder no longer exists: {}", relative_path.display());
            return;
        }
        let index = self.favorite_kit_index(&root).unwrap_or_else(|| {
            self.prefs.editing_kit_favorites.push(EditingKitFavorites {
                tags_root: clean_recent_path(root.clone()),
                tags: Vec::new(),
                folders: Vec::new(),
            });
            self.prefs.editing_kit_favorites.len() - 1
        });
        let favorites = &mut self.prefs.editing_kit_favorites[index];
        if let Some(position) = favorites
            .folders
            .iter()
            .position(|current| same_recent_path(current, &relative_path))
        {
            favorites.folders.remove(position);
            self.kits[self.active]
                .active_favorite_folders
                .retain(|current| !same_recent_path(current, &relative_path));
            if favorites.tags.is_empty() && favorites.folders.is_empty() {
                self.prefs.editing_kit_favorites.remove(index);
            }
            self.status = format!("Removed {} from Favorites", relative_path.display());
        } else {
            favorites.folders.push(relative_path.clone());
            self.kits[self.active]
                .active_favorite_folders
                .push(relative_path.clone());
            self.status = format!("Added {} to Favorites", relative_path.display());
        }
    }

    /// Rewrite `kit`'s favorites after a move or rename changed its tag paths.
    ///
    /// Takes the kit rather than reading the active one: this runs from a
    /// finished background refactor, which may well land while the user is in
    /// another workspace — and then it resolved the wrong root and remapped the
    /// wrong workspace's favorites with this one's rename map.
    pub(in crate::app) fn remap_favorites_for_kit(
        &mut self,
        kit: usize,
        old_to_new_keys: &HashMap<String, String>,
        moved_folder: Option<(&Path, &Path)>,
    ) {
        let Some(root) = self.loaded_tags_root_for(kit) else {
            return;
        };
        let Some(index) = self.favorite_kit_index(&root) else {
            return;
        };
        remap_favorite_paths(
            &root,
            &mut self.prefs.editing_kit_favorites[index].tags,
            old_to_new_keys,
        );
        if let Some((from, to)) = moved_folder {
            remap_favorite_folders(&mut self.prefs.editing_kit_favorites[index].folders, from, to);
            let mut unique_folders: Vec<PathBuf> = Vec::new();
            self.prefs.editing_kit_favorites[index].folders.retain(|path| {
                if unique_folders
                    .iter()
                    .any(|existing| same_recent_path(existing, path))
                {
                    false
                } else {
                    unique_folders.push(path.clone());
                    true
                }
            });
        }
        let mut unique: Vec<PathBuf> = Vec::new();
        self.prefs.editing_kit_favorites[index].tags.retain(|path| {
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
        let folder_seeds = self.kits[self.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            source.upsert_entry(entry, &folder_seeds);
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
        self.begin_scan_all_entries_in(self.active, ctx, label);
    }

    /// Scan `kit_index`'s folder, which need not be the focused kit: the Model
    /// and Bitmap Libraries ask for their own kit's scan. They used to call the
    /// active-kit version, which scanned whichever kit had focus and left
    /// theirs waiting for a scan it had recorded as requested.
    pub(super) fn begin_scan_all_entries_in(
        &mut self,
        kit_index: usize,
        ctx: egui::Context,
        label: impl Into<String>,
    ) {
        let kit = &self.kits[kit_index];
        if kit.scanning_entries {
            return;
        }
        let Some(source) = kit.source.as_ref() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return; // monolithic/single-file already have all entries
        };
        let root = root.clone();
        let names = source.names.clone();
        let tx = self.tx.clone();
        let kit = &mut self.kits[kit_index];
        kit.index_jobs.refreshing = false;
        kit.generation = kit.generation.wrapping_add(1);
        kit.field_index.invalidate();
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        let label = label.into();
        kit.scanning_entries = true;
        kit.index_jobs.entry_progress = Some(EntryIndexProgressState {
            label: label.clone(),
            processed: 0,
            total: 0,
            matched: 0,
        });
        self.show_entry_index_wait_notice = true;
        self.status = label;
        let progress_ctx = ctx.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result = scan_folder_subtree_entries_with_progress(
                    &root,
                    std::path::Path::new(""),
                    &names,
                    move |progress| {
                        let _ = tx.send(WorkerMessage::EntryIndexScanProgress {
                            stamp,
                            processed: progress.processed,
                            total: progress.total,
                            matched: progress.matched,
                        });
                        progress_ctx.request_repaint();
                    },
                )
                .map_err(|e| e.to_string());
                WorkerMessage::AllEntriesScanned { stamp, result }
            },
            move |error| WorkerMessage::AllEntriesScanned {
                stamp,
                result: Err(error),
            },
        );
    }

    /// Recursively materialize one loose browser folder for the bulk extract
    /// menu. A completed global index can satisfy this immediately; otherwise
    /// the same progress-reporting scanner used by indexing runs on a worker.
    pub(super) fn begin_load_folder_extractables(
        &mut self,
        rel_path: PathBuf,
        label: String,
        ctx: egui::Context,
    ) {
        let kit_index = self.active;
        if self.kits[kit_index].scanning_entries {
            self.status = "A folder scan is already running".to_owned();
            return;
        }
        let Some(source) = self.kits[kit_index].source.as_ref() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return;
        };

        if source.complete_scan {
            let entries = source
                .all_entries
                .iter()
                .filter(|entry| crate::core::source::entry_is_beneath_folder(entry, &rel_path))
                .cloned()
                .collect();
            self.install_folder_extractables(kit_index, &rel_path, entries);
            self.status = format!("Loaded the entire {label} folder for extraction");
            return;
        }

        let root = root.clone();
        let names = source.names.clone();
        let tx = self.tx.clone();
        let kit = &mut self.kits[kit_index];
        kit.generation = kit.generation.wrapping_add(1);
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        let progress_label = format!("Loading the entire {label} folder for extraction...");
        kit.scanning_entries = true;
        kit.index_jobs.entry_progress = Some(EntryIndexProgressState {
            label: progress_label.clone(),
            processed: 0,
            total: 0,
            matched: 0,
        });
        self.status = progress_label;
        let progress_ctx = ctx.clone();
        let worker_path = rel_path.clone();
        let worker_label = label.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result = scan_folder_subtree_entries_with_progress(
                    &root,
                    &worker_path,
                    &names,
                    move |progress| {
                        let _ = tx.send(WorkerMessage::EntryIndexScanProgress {
                            stamp,
                            processed: progress.processed,
                            total: progress.total,
                            matched: progress.matched,
                        });
                        progress_ctx.request_repaint();
                    },
                )
                .map_err(|error| error.to_string());
                WorkerMessage::FolderExtractablesLoaded {
                    stamp,
                    rel_path: worker_path,
                    label: worker_label,
                    result,
                }
            },
            move |error| WorkerMessage::FolderExtractablesLoaded {
                stamp,
                rel_path,
                label,
                result: Err(error),
            },
        );
    }

    /// Merge a recursively-scanned scope into the lazy entry list and replace
    /// that scope in every browser tree whose root overlaps it.
    pub(super) fn install_folder_extractables(
        &mut self,
        kit_index: usize,
        rel_path: &Path,
        scanned: Vec<TagEntry>,
    ) {
        let kit = &mut self.kits[kit_index];
        let Some(source) = kit.source.as_mut() else {
            return;
        };
        let mut known: HashSet<String> = source
            .entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect();
        source.entries.extend(
            scanned
                .into_iter()
                .filter(|entry| known.insert(entry.key.clone())),
        );
        replace_loaded_tree_scope(&mut source.tree, Path::new(""), rel_path, &source.entries);
        source.group_tree = crate::core::source::build_group_tree(&source.entries);
        let new_generation = kit.generation.wrapping_add(1);
        for pane in kit.folder_browsers.values_mut() {
            replace_loaded_tree_scope(&mut pane.tree, &pane.rel_path, rel_path, &source.entries);
            // Keep the materialized tree installed above. Marking it stale
            // caused the next frame to replace it with a direct-only lazy tree.
            pane.cached_generation = new_generation;
            pane.group_tree_for = None;
            pane.filter_cache = FilterCache::default();
        }
        kit.generation = new_generation;
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(super) fn maybe_refresh_entry_index(&mut self, ctx: egui::Context) {
        if self.kits[self.active].scanning_entries
            || self.kits[self.active].index_jobs.refreshing
            || self.kits[self.active].index_jobs.building_references
        {
            return;
        }
        let now = ctx.input(|input| input.time);
        if now < self.kits[self.active].index_jobs.next_refresh_at {
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
            self.schedule_next_entry_index_refresh(self.active, &ctx);
        }
    }

    pub(super) fn begin_refresh_entry_index(&mut self, ctx: egui::Context) {
        if self.kits[self.active].scanning_entries || self.kits[self.active].index_jobs.refreshing {
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
        let tag_source = source.source.clone();
        let stamp = self.kit_stamp();
        self.kits[self.active].index_jobs.refreshing = true;
        spawn_worker(
            &self.tx,
            &ctx,
            move || WorkerMessage::EntryIndexRefreshed {
                stamp,
                result: crate::core::source::refresh_entry_index(game.as_str(), &root, &names)
                    .map(|refresh| persist_entry_index_changes(game.as_str(), &root, &tag_source, refresh))
                    .map_err(|e| e.to_string()),
            },
            move |error| WorkerMessage::EntryIndexRefreshed {
                stamp,
                result: Err(error),
            },
        );
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

    pub(super) fn schedule_next_entry_index_refresh(&mut self, kit: usize, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        self.kits[kit].index_jobs.next_refresh_at = now + ENTRY_INDEX_REFRESH_INTERVAL_SECS;
    }

    pub(in crate::app) fn apply_entry_index_refresh(
        &mut self,
        kit_index: usize,
        refresh: EntryIndexRefresh,
        _ctx: egui::Context,
    ) {
        let EntryIndexRefresh {
            entries,
            added,
            updated,
            removed,
            removed_keys,
            touched_dependencies,
            errors,
            ..
        } = refresh;
        let n = entries.len();
        let browser_refresh_error = self.install_complete_entry_set(kit_index, entries);
        // Patched, not dropped: the worker read the changed tags' references
        // and already wrote them (and the index rows) to disk. Dropping it, as
        // this used to, left "References to" unavailable until a manual
        // rebuild after any change the refresh noticed, including the user's
        // own saves.
        for key in &removed_keys {
            self.kits[kit_index].set_tag_references(key, None);
        }
        for (key, deps) in touched_dependencies {
            self.kits[kit_index].set_tag_references(&key, Some(deps));
        }
        self.status = browser_refresh_error.map_or_else(
            || {
                format!(
                    "Index updated: {n} tags ({added} added, {updated} changed, {removed} removed)"
                )
            },
            |error| format!("Index updated, but browser refresh failed: {error}"),
        );
        if let Some(first) = errors.first() {
            self.status = format!(
                "{}; {} tag(s) could not be indexed, first {first}",
                self.status,
                errors.len()
            );
        }
    }

    /// Adopt a complete entry set for a kit: the full list and its group tree,
    /// a reset lazy browser, and a new generation so panes and caches rebuild.
    /// Shared by the full scan and the periodic refresh, which used to do this
    /// separately and had drifted (only one of them moved the generation).
    /// Returns the browser reset's error, if it failed.
    pub(super) fn install_complete_entry_set(
        &mut self,
        kit_index: usize,
        entries: Vec<TagEntry>,
    ) -> Option<String> {
        let kit = &mut self.kits[kit_index];
        let source = kit.source.as_mut()?;
        source.group_tree = crate::core::source::build_group_tree(&entries);
        source.all_entries = entries;
        source.complete_scan = true;
        let error = if let TagSource::LooseFolder { root, .. } = &source.source {
            reset_lazy_folder_browser(root, &mut source.tree, &mut source.entries).err()
        } else {
            None
        };
        kit.field_index.invalidate();
        kit.generation = kit.generation.wrapping_add(1);
        error
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
        let channel = self.prefs.update_channel;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = fetch_latest_release(channel);
            let _ = tx.send(WorkerMessage::UpdateCheckFinished { silent, result });
            ctx.request_repaint();
        });
    }

    /// Whether the automatic startup check should run.
    pub(super) fn should_check_updates_on_startup(&self) -> bool {
        self.prefs.check_updates_on_startup
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
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        if self.terminal.running {
            self.status = "A command is already running".to_owned();
            return;
        }
        let Some(work_dir) = self.kits[self.active].terminal_work_dir.clone() else {
            self.status = "Run requires a loaded editing-kit folder".to_owned();
            return;
        };
        // Rewritten before it is echoed, so the terminal shows what really ran.
        let command = with_tool_folder_options(&command, &self.active_kit_tool_folder_options());
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
            let mut cmd = terminal_shell_command(&command, &work_dir);
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
        kit_state.find_filter_applied.remove(key);
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
        let folder_seeds = self.kits[kit].folder_seeds();
        let kit_state = &mut self.kits[kit];
        kit_state.parsed_tags.remove(key);
        kit_state.loading_tags.remove(key);
        kit_state.bitmap_previews.remove(key);
        kit_state.model_previews.remove(key);
        kit_state.find_filter_applied.remove(key);
        kit_state.edit_buffers.forget_tag(key);
        if kit_state.selected_key.as_deref() == Some(key) {
            kit_state.selected_key = None;
        }
        if let Some(source) = kit_state.source.as_mut() {
            source.remove_entry(key, &folder_seeds);
        }
        kit_state.set_tag_references(key, None);
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
            .entry_for_key(&key)
            .or_else(|| {
                self.kits[self.active]
                    .active_favorite_entries
                    .iter()
                    .find(|e| e.key == key)
            })
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
        let kit = self.active_kit_id();
        self.kits[self.active].loading_tags.insert(key.clone());
        self.status = format!("Loading {}", entry.display_path);
        let panic_key = key.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result = read_entry(&source_kind, &entry).map_err(|error| format!("{error:#}"));
                WorkerMessage::TagLoaded { kit, key, result }
            },
            move |error| WorkerMessage::TagLoaded {
                kit,
                key: panic_key,
                result: Err(error),
            },
        );
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
        // Chimp's recovery checkpoints wait for edits to pause; one still
        // waiting when the app or a workspace closes would be lost.
        self.flush_all_chimp_checkpoints();
        if self.save_changes_prompt.visible
            || self.chimp_discard_prompt.is_some()
            || self.has_chimp_save_dialog()
        {
            return;
        }
        // A Chimp save is writing containers on a worker. The close waits
        // for it and runs from its completion, like a close the save dialog
        // was opened for.
        let writing = match &action {
            PendingCloseAction::CloseApp => self.chimp_writes.keys().next().copied(),
            PendingCloseAction::CloseKit(id) => self.chimp_writes.contains_key(id).then_some(*id),
            _ => None,
        };
        if let Some(kit) = writing {
            self.chimp_writes.insert(kit, Some(action));
            self.status = "Closing once the Chimp save finishes…".to_owned();
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
        if !dirty_tags.is_empty() {
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
            return;
        }

        let chimp_packages = self.dirty_chimp_for_close_action(&action);
        if !chimp_packages.is_empty() {
            self.open_chimp_discard_prompt(self.active, chimp_packages, Some(action), None);
            return;
        }

        self.execute_close_action(action, ctx);
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
        // Quitting would kill the worker partway through rewriting references,
        // leaving some tags pointing at a path that no longer exists.
        if self.folder_refactor.is_some() {
            self.status = "Wait for the folder move/rename to finish before closing".to_owned();
            return;
        }
        if self.save_changes_prompt.visible
            || self.chimp_discard_prompt.is_some()
            || self.has_chimp_save_dialog()
        {
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

    fn dirty_chimp_for_close_action(&self, action: &PendingCloseAction) -> Vec<String> {
        if close_action_includes_chimp(action) {
            self.chimp_dirty_packages(self.active)
        } else {
            Vec::new()
        }
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
                // `request_close_action` is the close coordinator and only
                // calls this once every dirty workspace has been resolved. Do
                // not call it recursively here: a dirty Chimp document used
                // to be counted by this check but omitted from the tag prompt,
                // creating an infinite CloseApp -> request_close_action loop.
                if self.any_kit_dirty() {
                    self.status = "Could not close while unsaved workspace data remains".to_owned();
                    return;
                }
                if let Some(session) = self.current_session_state() {
                    if let Err(error) = save_last_session(&session) {
                        self.status = error;
                        return;
                    }
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

    /// Snapshot every kit's source and open tag/folder panes for the restore prompt.
    pub(super) fn current_session_state(&self) -> Option<LastSessionState> {
        let kits = (0..self.kits.len())
            .filter_map(|index| self.session_kit_state(index))
            .collect::<Vec<_>>();
        (!kits.is_empty()).then_some(LastSessionState { kits })
    }

    fn session_kit_state(&self, kit_index: usize) -> Option<LastSessionKit> {
        let kit = &self.kits[kit_index];
        let was_active = kit_index == self.active;
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
            let Some(entry) = source.entry_for_key(&key) else {
                continue;
            };
            let path = match &entry.location {
                TagEntryLocation::LooseFile(path) => {
                    Some(fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
                }
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
        let folders = ordered_unique_keys(kit.open_tabs.iter())
            .into_iter()
            .filter_map(|key| kit.folder_browsers.get(&key))
            .map(|folder| LastSessionFolder {
                rel_path: folder.rel_path.clone(),
                label: folder.label.clone(),
            })
            .collect();
        let chimp_packages = ordered_unique_keys(kit.chimp.open_packages.iter());
        let active_chimp_package = kit
            .chimp
            .selected_package
            .clone()
            .filter(|active| chimp_packages.contains(active));
        // The source itself is part of the workspace session, even when the
        // user has no tag, Chimp package, or project open in it. Otherwise a
        // second loaded editing kit disappears from the next-session prompt
        // simply because its tags were not selected yet.
        Some(LastSessionKit {
            source_kind,
            source_path,
            game: source.game.map(|game| game.as_str().to_owned()),
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
            folders,
            chimp_packages,
            active_chimp_package,
            // Read off the open tabs rather than the tag list: the libraries'
            // pane keys resolve to no entry, so the loop above skipped them.
            bitmap_library_open: kit.open_tabs.iter().any(|key| key == BITMAP_LIBRARY_KEY),
            model_library_open: kit.open_tabs.iter().any(|key| key == MODEL_LIBRARY_KEY),
            was_active,
        })
    }

    /// Reopen each saved kit. Every kit gets its own load, and its panes are
    /// staged on the kit itself rather than in one shared slot, so the loads
    /// can finish in any order without stealing each other's restore state.
    pub(super) fn begin_last_session_restore(&mut self, kits: Vec<RestoreKit>, ctx: egui::Context) {
        for RestoreKit {
            source_kind,
            source_path,
            profile_id,
            project_path,
            browser_mode,
            browser_sort,
            tags,
            folders,
            chimp_packages,
            active_chimp_package,
            bitmap_library_open,
            model_library_open,
            was_active,
        } in kits
        {
            match source_kind {
                LastSessionSourceKind::SingleFile => {
                    self.begin_load_single_path(source_path, ctx.clone())
                }
                LastSessionSourceKind::LooseFolder => {
                    let started = if let Some(profile) = profile_id
                        .as_deref()
                        .and_then(|id| {
                            self.prefs
                                .custom_editing_kit_profiles
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
            //
            // Each load also finishes by making its own kit active, so the
            // focused workspace would otherwise be whichever one happened to
            // load last. Remember the kit the session named and every kit still
            // to land, so the focus can be set once they all have.
            let restoring = self.kits[self.active].id;
            self.restoring_kits.insert(restoring);
            if was_active {
                self.restored_active_kit = Some(restoring);
            }
            self.kits[self.active].pending_restore_tags = tags;
            self.kits[self.active].pending_restore_folders = folders;
            self.kits[self.active].pending_restore_chimp_packages = chimp_packages;
            self.kits[self.active].pending_restore_bitmap_library = bitmap_library_open;
            self.kits[self.active].pending_restore_model_library = model_library_open;
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

    /// Record the session as the event loop tears down.
    ///
    /// Baboon's whole shutdown chain hangs off a window close request:
    /// `handle_app_close_request` only acts on `close_requested()`, and it is
    /// what eventually reaches [`Self::execute_close_action`] and saves the
    /// session. macOS never sends one for Cmd+Q — AppKit posts
    /// `applicationWillTerminate:`, which closes each window directly rather
    /// than asking it to close, so no `CloseRequested` is ever emitted and none
    /// of that runs. The session file was then left holding whatever last wrote
    /// it, which for a Campaign Evolved workspace is its project autosave: quit
    /// with a Halo 3 kit open and the next launch restored Campaign Evolved,
    /// because that was the last session anything had recorded.
    ///
    /// This runs on every shutdown, including the ordinary one that already
    /// saved a moment earlier — the write is the same document either way. It
    /// cannot prompt: the loop is already exiting and `LoopExiting` cannot be
    /// vetoed, so unsaved tag edits still go unremarked on a Cmd+Q.
    pub(super) fn persist_session_on_exit(&mut self) {
        match self.current_session_state() {
            Some(session) => {
                let _ = save_last_session(&session);
            }
            None => clear_last_session(),
        }
    }

    /// Mark one restored kit's load as settled, whatever became of it, and once
    /// none are left hand the focus to the kit the session named.
    ///
    /// Every completed load makes its own kit active, so during a restore the
    /// focused workspace is otherwise decided by which source finishes first —
    /// a loose folder against a container set is not a race with a stable
    /// winner. The saved kit is only honoured while it is still open and it
    /// still loaded; a kit the user unchecked in the restore prompt, or whose
    /// source has since moved, leaves the focus wherever the loads put it.
    pub(super) fn settle_restored_kit(&mut self, kit: KitId) {
        let Some(active) =
            focus_after_restore(&mut self.restoring_kits, &mut self.restored_active_kit, kit)
        else {
            return;
        };
        if let Some(index) = self.kit_index(active) {
            self.active = index;
        }
    }

    /// Reopen the panes staged for the kit that just finished loading.
    fn finish_pending_session_restore(&mut self, ctx: egui::Context) {
        // Ahead of the early return below: a workspace whose only open tab was
        // the Bitmap Library has no tags staged, and would otherwise come back
        // without it.
        if std::mem::take(&mut self.kits[self.active].pending_restore_bitmap_library) {
            self.open_bitmap_library();
        }
        if std::mem::take(&mut self.kits[self.active].pending_restore_model_library) {
            self.open_model_library();
        }
        let restore_folders = std::mem::take(&mut self.kits[self.active].pending_restore_folders);
        for folder in &restore_folders {
            self.handle_browser_action(
                BrowserAction::OpenFolderBrowser {
                    rel_path: folder.rel_path.clone(),
                    label: folder.label.clone(),
                    open_in_new_tab: true,
                },
                ctx.clone(),
            );
        }
        let restore = std::mem::take(&mut self.kits[self.active].pending_restore_tags);
        if restore.is_empty() && restore_folders.is_empty() {
            return;
        }
        let mut opened = restore_folders.len();
        let mut missing = 0usize;
        for tag in restore {
            if let Some(current_key) = self.restored_tag_entry_key(&tag) {
                self.select_entry(current_key, ctx.clone());
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

    /// Resolve a saved pane to the key used by the freshly mounted source.
    ///
    /// Loose-file keys include a displayed filesystem path. Windows accepts
    /// both separators, and older sessions could therefore persist a mixed
    /// `file:C:\.../objects\...` spelling that no longer compared equal to the
    /// newly scanned entry. Rediscovering the file was not enough: restore then
    /// opened the stale saved key and reported that the tag had disappeared.
    /// Return the source's current key so existing sessions recover in place.
    fn restored_tag_entry_key(&mut self, tag: &LastSessionTag) -> Option<String> {
        if let Some(entry) = self.entry_for_key(&tag.key) {
            return Some(entry.key.clone());
        }
        let path = tag.path.as_ref()?;
        if !path.is_file() {
            return None;
        }
        let source = self.source()?;
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return None;
        };
        let root = fs::canonicalize(root).ok()?;
        let path = fs::canonicalize(path).ok()?;
        if !path.starts_with(&root) {
            return None;
        }
        if let Some(current_key) = loose_entry_key_for_canonical_path(
            source.entries.iter().chain(source.all_entries.iter()),
            &path,
        ) {
            return Some(current_key);
        }
        let entry = loose_file_entry(&root, &path, &source.names).ok()??;
        let current_key = entry.key.clone();
        let folder_seeds = self.kits[self.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            if tag.key != current_key {
                source.remove_entry(&tag.key, &folder_seeds);
            }
            source.upsert_entry(entry, &folder_seeds);
        }
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        Some(current_key)
    }

    pub(super) fn close_all_tabs(&mut self) {
        let id = self.kits[self.active].id;
        self.kits[self.active].tag_tree = egui_tiles::Tree::empty(tag_tree_id(id));
        self.kits[self.active].open_tabs.clear();
        self.kits[self.active].drop_documents_except(None);
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
        self.kits[self.active].drop_documents_except(Some(key));
        self.kits[self.active].selected_key = (!is_folder_pane_key(key)).then(|| key.to_owned());
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn unload_tag(&mut self, key: &str) {
        self.kits[self.active].drop_document(key);
    }

    pub(super) fn handle_browser_action(&mut self, action: BrowserAction, ctx: egui::Context) {
        match action {
            BrowserAction::OpenFolderBrowser {
                rel_path,
                label,
                open_in_new_tab,
            } => {
                let matching_key = if open_in_new_tab {
                    None
                } else {
                    let normalized = rel_path.to_string_lossy().replace('\\', "/");
                    let base = folder_pane_key(&rel_path);
                    let panes = &self.kits[self.active].folder_browsers;
                    panes
                        .get(&base)
                        .filter(|pane| {
                            pane.rel_path
                                .to_string_lossy()
                                .replace('\\', "/")
                                .eq_ignore_ascii_case(&normalized)
                        })
                        .map(|_| base)
                        .or_else(|| {
                            let mut matches = panes
                                .iter()
                                .filter(|(_, pane)| {
                                    pane.rel_path
                                        .to_string_lossy()
                                        .replace('\\', "/")
                                        .eq_ignore_ascii_case(&normalized)
                                })
                                .map(|(key, _)| key.clone())
                                .collect::<Vec<_>>();
                            matches.sort();
                            matches.into_iter().next()
                        })
                };
                let key = matching_key.unwrap_or_else(|| {
                    let base = folder_pane_key(&rel_path);
                    if !self.kits[self.active].folder_browsers.contains_key(&base) {
                        return base;
                    }
                    (2..)
                        .map(|suffix| format!("{base}#{suffix}"))
                        .find(|candidate| {
                            !self.kits[self.active]
                                .folder_browsers
                                .contains_key(candidate)
                        })
                        .expect("folder pane suffix space is unbounded")
                });
                self.kits[self.active]
                    .folder_browsers
                    .entry(key.clone())
                    .or_insert_with(|| FolderBrowserState {
                        rel_path,
                        label,
                        filter: String::new(),
                        focus_search: false,
                        mode: BrowserMode::Folders,
                        sort: self.prefs.browser_sort,
                        cached_generation: u64::MAX,
                        cached_source_len: usize::MAX,
                        tree: TagTree::default(),
                        group_tree: TagTree::default(),
                        group_tree_for: None,
                        filter_cache: FilterCache::default(),
                    });
                let selected = self.kits[self.active].selected_key.clone();
                self.kits[self.active].open_tag_pane(&key);
                self.kits[self.active].selected_key = selected;
            }
            BrowserAction::ToggleFolderFavorite(rel_path) => self.toggle_folder_favorite(&rel_path),
            BrowserAction::Select(key) => self.select_entry(key, ctx),
            BrowserAction::ToggleFavorite(key) => self.toggle_favorite(&key),
            BrowserAction::CopyTagName(key) => self.copy_tag_name(&key, &ctx),
            BrowserAction::CopyFolderPath(path) => self.copy_folder_path(&path, &ctx),
            BrowserAction::DumpJson(key) => self.begin_export_json(key, ctx),
            BrowserAction::OpenInExplorer(key) => self.open_entry_in_explorer(&key),
            BrowserAction::DumpLoadedFolderJson(keys) => {
                self.begin_export_loaded_folder_json(keys, ctx)
            }
            BrowserAction::DumpLooseFolderJson { rel_path, label } => {
                self.begin_export_loose_folder_json(rel_path, label, ctx)
            }
            BrowserAction::RenameLooseFolder { rel_path, label } => {
                self.open_loose_folder_rename(rel_path, label)
            }
            BrowserAction::MoveLooseFolder { rel_path, label } => {
                self.begin_refactor_loose_folder(rel_path, label, true)
            }
            BrowserAction::CopyLooseFolder { rel_path, label } => {
                self.begin_refactor_loose_folder(rel_path, label, false)
            }
            BrowserAction::ImportTagsIntoLooseFolder { rel_path } => {
                self.open_tag_import_dialog(Some(rel_path.to_string_lossy().into_owned()))
            }
            BrowserAction::OpenLooseFolderInExplorer { rel_path } => {
                self.open_loose_folder_in_explorer(&rel_path)
            }
            BrowserAction::ImportCacheFolderIntoKit { prefix } => {
                self.open_cache_import_dialog(prefix)
            }
            BrowserAction::ImportCacheTagIntoKit { key } => {
                self.open_cache_import_dialog_for_tag(key)
            }
            BrowserAction::ExtractRaw(key) => self.begin_extract_raw(key, ctx),
            BrowserAction::ExtractBitmap(key) => self.begin_extract_bitmap(key, ctx),
            BrowserAction::ExtractBitmapFolder(keys) => self.begin_extract_bitmap_folder(keys, ctx),
            BrowserAction::ExtractBitmapSource(key) => {
                self.begin_extract_bitmap_sources(vec![key], false, ctx)
            }
            BrowserAction::ExtractBitmapSourceFolder(keys) => {
                self.begin_extract_bitmap_sources(keys, true, ctx)
            }
            BrowserAction::ExtractSound {
                keys,
                all_languages,
            } => self.begin_extract_sounds(keys, all_languages),
            BrowserAction::LoadFolderExtractables { rel_path, label } => {
                self.begin_load_folder_extractables(rel_path, label, ctx)
            }
            BrowserAction::ExtractGeometry(key) => {
                self.prompt_extract_target(key, ExtractKind::Geometry)
            }
            BrowserAction::ExtractImportInfo(key) => self.begin_extract_import_info(key, ctx),
            BrowserAction::ExtractAnimation(key) => {
                self.prompt_extract_target(key, ExtractKind::Animation)
            }
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
            BrowserAction::ReimportGeometry(key) => self.begin_reimport_geometry(&key),
            BrowserAction::ExtractContainerFolderTags { label, keys } => {
                self.begin_extract_container_folder_tags(label, keys)
            }
            BrowserAction::ExtractScenarioScripts(key) => {
                self.begin_extract_scenario_scripts(key, ctx)
            }
            BrowserAction::ImportScenarioScripts(key) => self.import_scenario_scripts(&key),
            BrowserAction::RenameTag(key) => self.open_rename_tag(&key),
            BrowserAction::DuplicateTag(key) => self.open_duplicate_tag(&key),
            BrowserAction::DeleteTag(key) => self.open_delete_tag(&key),
            BrowserAction::FindReferences(key) => self.show_references_for(&key),
            BrowserAction::ExploreReferences(key) => self.open_content_explorer(&key),
            BrowserAction::DumpReferences(key) => self.begin_dump_tag_references(&key, ctx),
            BrowserAction::LaunchScenarioInSapien(key) => self.launch_scenario_in_sapien(&key),
            BrowserAction::LaunchScenarioInTagTest(key) => self.launch_scenario_in_tag_test(&key),
            BrowserAction::MoveTag(key) => self.begin_move_tag(&key),
            BrowserAction::ImportTagInFolder { folder_rel } => self.begin_import_tag(folder_rel),
            BrowserAction::NewTagInFolder { folder_rel } => {
                self.open_new_tag_dialog_in_folder(folder_rel)
            }
            BrowserAction::NewContainerFolder { parent_rel } => {
                self.open_new_container_folder(parent_rel)
            }
            BrowserAction::RenameContainerFolder { rel } => self.open_rename_container_folder(rel),
            BrowserAction::DeleteContainerFolder { rel } => self.delete_container_folder(rel),
        }
    }

    pub(super) fn copy_tag_name(&mut self, key: &str, ctx: &egui::Context) {
        let Some(entry) = self.entry_for_key(key) else {
            self.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        let copied_path = crate::core::format::to_native_path_string(&entry.display_path);
        ctx.copy_text(copied_path.clone());
        self.status = format!("Copied {copied_path}");
    }

    pub(super) fn copy_folder_path(&mut self, path: &Path, ctx: &egui::Context) {
        let copied_path = crate::core::format::to_native_path_string(&path.to_string_lossy());
        ctx.copy_text(copied_path.clone());
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
        Some(self.kit_layout_for(self.active)?.data)
    }

    /// Show a browser folder in File Explorer.
    ///
    /// `rel_path` is the browser's own path for the node, which for a loose kit
    /// is the directory's path under the tags root — so the only work is joining
    /// the two. A kit that is not a loose folder has no directory to open, and
    /// says so rather than opening the wrong thing.
    pub(super) fn open_loose_folder_in_explorer(&mut self, rel_path: &Path) {
        let Some(root) = self.loaded_tags_root() else {
            self.status = "This workspace has no tags folder on disk".to_owned();
            return;
        };
        let path = loose_folder_explorer_path(&root, rel_path);
        self.open_folder_in_explorer(path, "Tag");
    }

    /// Show `path` in the system's file manager: File Explorer, Finder, or
    /// whatever `xdg-open` picks.
    ///
    /// Only Windows used to do anything here; everywhere else the user was
    /// told the action was Windows-only, though the git review panel already
    /// opened folders on macOS and Linux its own way.
    pub(super) fn open_folder_in_explorer(&mut self, path: PathBuf, label: &str) {
        self.open_folder_with(path, label, |mut command| command.spawn().map(drop));
    }

    /// [`Self::open_folder_in_explorer`] with the launch passed in, so the
    /// command can be checked without opening a window.
    fn open_folder_with(
        &mut self,
        path: PathBuf,
        label: &str,
        spawn: impl FnOnce(Command) -> std::io::Result<()>,
    ) {
        if !path.is_dir() {
            self.status = format!("{label} folder not found: {}", path.display());
            return;
        }
        self.status = match spawn(folder_opener(&path)) {
            Ok(()) => format!("Opened {} folder: {}", label, path.display()),
            Err(error) => format!("Could not open the {label} folder: {error}"),
        };
    }

    /// Start resolving a loaded model's materials to textures, if it needs it.
    ///
    /// Idempotent and cheap to call every frame: it only spawns when a model is
    /// loaded, a textured mode is on, its textures are still absent, and no job for it
    /// is already running.
    pub(in crate::app) fn maybe_request_model_textures(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) {
        let Some(state) = self.kits[kit_index].model_previews.get(key) else {
            return;
        };
        if !state.render_mode.uses_textures() || state.textures_pending {
            return;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return;
        };
        if data.textures.is_some() || data.preview.materials.is_empty() {
            return;
        }
        let Some(source) = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.clone())
        else {
            return;
        };
        let materials = data.preview.materials.clone();
        let textures_id = data.textures_id;
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        if let Some(state) = self.kits[kit_index].model_previews.get_mut(key) {
            state.textures_pending = true;
        }

        let (key, panic_key) = (key.to_owned(), key.to_owned());
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::ModelTexturesResolved {
                stamp,
                key,
                textures_id,
                textures: resolve_model_textures(&source, &materials),
            },
            // No textures: the preview draws untextured, and stops waiting.
            move |_| WorkerMessage::ModelTexturesResolved {
                stamp,
                key: panic_key,
                textures_id,
                textures: Vec::new(),
            },
        );
    }

    pub(in crate::app) fn handle_model_textures_resolved(
        &mut self,
        stamp: KitStamp,
        key: String,
        textures_id: u64,
        textures: Vec<MaterialTextures>,
    ) -> bool {
        let Some(kit_index) = self.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.resolve_stamp(stamp).is_none();
        let Some(state) = self.kits[kit_index].model_previews.get_mut(&key) else {
            return true;
        };
        // The in-flight marker is cleared before the staleness check: a result
        // dropped for a generation bump used to leave it set, and the preview
        // then waited for it for good ("Loading shaders…", repainting every
        // frame).
        state.textures_pending = false;
        if stale {
            return true;
        }
        let Some(Ok(data)) = state.data.as_mut() else {
            return true;
        };
        // The model was reloaded while this ran, and these textures are
        // indexed against the materials of the old one.
        if data.textures_id != textures_id {
            return true;
        }
        data.textures = Some(std::sync::Arc::new(textures));
        false
    }

    pub(super) fn active_game_is_campaign_evolved(&self) -> bool {
        self.source_game().is_some_and(GameId::is_campaign_evolved)
    }

    pub(super) fn save_current_tag(&mut self, ctx: &egui::Context) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
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
        // in place, which is destructive and is not how anyone should be
        // shipping a change — so it is an expert-mode route now. Everyone else
        // gets the export, which is the supported one.
        if self.current_source_is_container() {
            match container_save_route(
                self.prefs.expert_mode,
                self.prefs.confirm_container_overwrite,
            ) {
                ContainerSaveRoute::ExportReview => {
                    self.status = "Your change is kept in this workspace — export it as a mod to \
                                   put it in the game"
                        .to_owned();
                    self.export_mod();
                }
                ContainerSaveRoute::ConfirmOverwriteInPlace => {
                    self.overwrite_confirm = Some(OverwriteConfirm {
                        kit: self.active_kit_id(),
                        key,
                    });
                }
                ContainerSaveRoute::OverwriteInPlace => {
                    self.begin_overwrite_current_tag_in_place(&key, ctx)
                }
            }
            return;
        }
        match self.save_tag_by_key(&key) {
            Ok(path) => self.status = format!("Saved {}", path.display()),
            Err(error) => self.status = format!("Save failed: {error}"),
        }
    }

    pub(super) fn save_tag_by_key(&mut self, key: &str) -> Result<PathBuf, String> {
        if self.refuse_read_only_edit(self.active) {
            return Err(self.status.clone());
        }
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
        // What the tag now points at, from the document just written.
        let dependencies = {
            let mut refs = Vec::new();
            collect_tag_dependency_refs(doc.tag.root(), &mut refs);
            refs
        };
        if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(key) {
            doc.dirty.clear();
        }
        // The save also writes the index row, so the periodic refresh will
        // not see this file change; the shader grid has to hear it here.
        if is_render_method_layout_group(entry.group_tag) {
            self.kits[self.active].forget_render_methods();
        }
        self.record_saved_tag_in_indexes(&entry, dependencies);
        Ok(output)
    }

    /// Bring the on-disk indexes and the reference index up to date with a tag
    /// the user just saved.
    ///
    /// A plain Save touched neither. The next periodic refresh then saw the
    /// file's new modified time as a change, and (before refreshes were
    /// patched in) dropped the whole reference index for it. Writing the row
    /// here means the refresh sees nothing to do, and the references are the
    /// ones the saved document holds.
    ///
    /// The row and the references are written in one transaction, so a
    /// failure leaves both as they were and the refresh picks the change up;
    /// the failure goes to the terminal, with the other index warnings.
    fn record_saved_tag_in_indexes(&mut self, entry: &TagEntry, dependencies: Vec<DependencyRef>) {
        let Some(source) = self.source_mut() else {
            return;
        };
        if let (TagSource::LooseFolder { root, .. }, Some(game)) =
            (&source.source, source.game.map(GameId::as_str))
            && !source.all_entries.is_empty()
            && let Err(error) = crate::core::source::upsert_entry_with_dependencies(
                game,
                root,
                entry,
                Some(&dependencies),
            )
        {
            let _ = self.tx.send(WorkerMessage::TerminalLine(format!(
                "Warning: could not record {} in the tag index: {error:#}",
                entry.display_path
            )));
        }
        self.kits[self.active].set_tag_references(&entry.key, Some(dependencies));
    }

    pub(super) fn save_current_tag_as(&mut self) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
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
        if self.refuse_read_only_edit(self.active) {
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
        let applied_rows = data_rows.saturating_sub(skipped_rows);
        let active = self.active;
        let ops = DeferredOps {
            pending: edits,
            ..DeferredOps::default()
        };
        let Some(applied) = self.apply_doc_ops(active, &tag_key, "Paste TSV", ops, UndoStep::Own)
        else {
            return;
        };
        self.invalidate_tag_caches_in(active, &tag_key);

        let summary =
            tsv_paste_summary(&applied.outcomes, applied_rows, skipped_rows, element_count);
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
        let path = root.join(game.as_str()).join(format!("{group}.json"));
        if let Some(docs) = self.def_docs_cache.get(&path) {
            return Some(docs.clone());
        }
        let docs = Rc::new(build_def_docs(&root, game, &group));
        self.def_docs_cache.insert(path, docs.clone());
        Some(docs)
    }

    /// Drop cached previews derived from a tag's contents so they rebuild from
    /// the (newly restored) tag bytes after an undo/redo.
    /// Drop derived previews for `key` in `kit`, after its document changed.
    pub(super) fn invalidate_tag_caches_in(&mut self, kit: usize, key: &str) {
        if let Some(preview) = self.kits[kit].model_previews.get_mut(key) {
            preview.invalidate_load();
        }
        if let Some(bitmap) = self.kits[kit].bitmap_previews.get_mut(key) {
            bitmap.decoded = None;
            bitmap.decoding = None;
            bitmap.texture = None;
            bitmap.texture_dirty = true;
        }
        // rmdf/rmop caches are keyed by external render-method paths, not by this
        // tag's contents, and the shader grid's model is keyed by the document's
        // dirty revision, which the change has already moved — so nothing to
        // clear there.
    }

    /// Whether the active kit is showing its Chimp surface rather than tags.
    ///
    /// Undo and redo act on the selected tag, which is hidden there, and Chimp
    /// has no undo of its own yet; so on that surface they do nothing rather
    /// than silently changing a tag the user cannot see.
    fn chimp_surface_is_active(&self) -> bool {
        self.prefs.enable_chimp && self.kits[self.active].surface == KitSurface::Chimp
    }

    pub(super) fn undo_current_tag(&mut self) {
        if self.chimp_surface_is_active() {
            self.status = "Chimp has no undo yet; undo applies to tags.".to_owned();
            return;
        }
        if self.refuse_read_only_edit(self.active) {
            return;
        }
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
        if self.chimp_surface_is_active() {
            self.status = "Chimp has no redo yet; redo applies to tags.".to_owned();
            return;
        }
        if self.refuse_read_only_edit(self.active) {
            return;
        }
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
    fn restore_snapshot(
        &mut self,
        key: &str,
        restored: Option<(Arc<Vec<u8>>, String)>,
        verb: &str,
    ) {
        // Classic (Halo CE / Halo 2) snapshots are serialized in classic format,
        // which `read_from_bytes` can't parse — re-parse with the JSON layout.
        let group_tag = self.kits[self.active]
            .parsed_tags
            .get(key)
            .map(|doc| doc.tag.group().tag);
        let game = self.source_game();
        let definitions_root = self.source_definitions_root().map(Path::to_owned);
        match restored {
            Some((bytes, label)) => {
                match group_tag
                    .context("no open tag to restore")
                    .and_then(|group_tag| {
                        crate::core::source::read_tag_from_bytes(
                            &bytes,
                            game,
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
        if self.chimp_surface_is_active() || self.editing_kit_is_read_only(self.active) {
            return false;
        }
        self.kits[self.active]
            .selected_key
            .as_ref()
            .and_then(|key| self.kits[self.active].parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_undo())
    }

    pub(super) fn can_redo_current(&self) -> bool {
        if self.chimp_surface_is_active() || self.editing_kit_is_read_only(self.active) {
            return false;
        }
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
            ..self.prefs.clone()
        }
    }

    pub(super) fn editing_kit_root(&self) -> Option<PathBuf> {
        self.editing_kit_root_for(self.active)
    }

    pub(in crate::app) fn editing_kit_is_read_only(&self, kit_index: usize) -> bool {
        let Some(kit) = self.kits.get(kit_index) else {
            return false;
        };
        // The tags folder rather than the kit root: every folder above the
        // root is above the tags folder too, so this matches at least what the
        // root did and never makes a read-only kit writable.
        let root = self
            .kit_layout_for(kit_index)
            .map(|layout| layout.tags)
            .or_else(|| match &kit.source.as_ref()?.source {
                TagSource::SingleFile { path } => Some(path.clone()),
                _ => None,
            });
        self.prefs
            .custom_editing_kit_profiles
            .iter()
            .any(|profile| profile.is_read_only_for(kit.profile.as_ref(), root.as_deref()))
    }

    pub(in crate::app) fn refuse_read_only_edit(&mut self, kit_index: usize) -> bool {
        if self.editing_kit_is_read_only(kit_index) {
            self.status =
                "This editing kit is read-only. Change its Editing Kit settings to enable editing."
                    .to_owned();
            true
        } else {
            false
        }
    }

    pub(super) fn editing_kit_root_for(&self, kit_index: usize) -> Option<PathBuf> {
        Some(self.kit_layout_for(kit_index)?.root)
    }

    /// The loaded kit's root, tags and data folders. See [`KitLayout`].
    pub(super) fn kit_layout_for(&self, kit_index: usize) -> Option<KitLayout> {
        self.kits.get(kit_index)?.source.as_ref()?.kit_layout()
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
        tag_test_executable_for_game(self.source().and_then(|s| s.game))
    }

    pub(super) fn launch_tag_test(&mut self) {
        self.launch_kit_tool_clearing_startup("tag_test", self.tag_test_executable(), "init.txt");
    }

    /// Whether this workspace's editing kit has a Sapien that can open a
    /// scenario at all — the question of whether to *offer* the button, as
    /// opposed to whether it can be pressed right now.
    ///
    /// Answered from the kit's game alone, deliberately. Whether a particular
    /// scenario resolves to a launchable path, and whether `sapien.exe` is
    /// where it should be, are reasons to grey the button out; a kit whose
    /// Sapien has no way to be given a scenario is a reason for there to be no
    /// button.
    pub(super) fn kit_offers_scenario_sapien(&self, kit: usize) -> bool {
        self.kits
            .get(kit)
            .and_then(|kit| kit.source.as_ref())
            .and_then(|source| source.game)
            .is_some_and(GameFacts::sapien_takes_scenario_argument)
    }

    pub(super) fn can_launch_scenario_in_sapien(&self, kit: usize, entry: &TagEntry) -> bool {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return false;
        };
        let Ok(context) = scenario_launch_context(source, entry) else {
            return false;
        };
        context.game.sapien_takes_scenario_argument()
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
        if !context.game.sapien_takes_scenario_argument() {
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
        for (option, folder) in &context.tool_options {
            process.arg(option).arg(folder);
        }
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
        let executable = tag_test_executable_for_game(Some(context.game));
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
        let executable_name = tag_test_executable_for_game(Some(context.game));
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
        let command = scenario_startup_command(context.game, &context.scenario_path);
        if let Err(error) = update_scenario_startup_file(&startup_file, &command) {
            self.status = error;
            return;
        }
        let mut process = Command::new(&executable);
        for (option, folder) in &context.tool_options {
            process.arg(option).arg(folder);
        }
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
        let Some(path) = self.prefs.blender_path.clone() else {
            self.settings_open = true;
            self.status = "Set the Blender path in File > Settings first".to_owned();
            return;
        };
        if !path.is_file() {
            self.status = format!("Blender executable not found: {}", path.display());
            self.settings_open = true;
            return;
        }
        self.spawn_tool("Blender", &path, path.parent().map(Path::to_path_buf), &[]);
    }

    pub(super) fn choose_blender_path(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Select Blender Executable");
        if let Some(path) = self
            .prefs
            .blender_path
            .as_ref()
            .and_then(|path| path.parent())
        {
            dialog = dialog.set_directory(path);
        }
        #[cfg(target_os = "windows")]
        {
            dialog = dialog.add_filter("Executable", &["exe"]);
        }
        if let Some(path) = dialog.pick_file() {
            self.prefs.blender_path = Some(path.clone());
            self.blender_path_input = path.display().to_string();
            self.status = format!("Blender path set to {}", path.display());
        }
    }

    pub(super) fn load_editing_kit_shortcut(
        &mut self,
        shortcut: EditingKitShortcut,
        ctx: egui::Context,
    ) {
        let Some(path) = self.prefs.editing_kit_paths.get(shortcut.game.as_str()).cloned() else {
            if let Some(profile) = self
                .prefs
                .custom_editing_kit_profiles
                .iter()
                .find(|profile| profile.game == shortcut.game.as_str())
                .cloned()
            {
                self.load_custom_editing_kit_profile(profile, ctx);
                return;
            }
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
        if shortcut.game.is_campaign_evolved() {
            self.begin_load_folder_path(path, ctx);
        } else {
            self.begin_load_editing_kit_layout(
                layout,
                shortcut.game.as_str().to_owned(),
                shortcut.game.display_name().to_owned(),
                None,
                false,
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
        // Several kits of one game can share a root, each with its own tags
        // folder; the one holding the first tag named is the one meant.
        let profiles = &self.prefs.custom_editing_kit_profiles;
        let first_absolute = launch.tag_paths.iter().find(|path| path.is_absolute());
        let profile = first_absolute
            .and_then(|tag| {
                let tag = canonical_or_clean(tag);
                profiles.iter().find(|profile| {
                    profile.game == shortcut.game.as_str() && tag.starts_with(profile_tags_folder(profile))
                })
            })
            .or_else(|| {
                profiles
                    .iter()
                    .find(|profile| profile.game == shortcut.game.as_str())
            })
            .cloned();
        if let Some(profile) = profile
            .as_ref()
            .filter(|profile| profile.has_chosen_folders())
            .cloned()
        {
            self.kits[self.active].pending_launch_tags = Some(launch.tag_paths);
            if !self.load_custom_editing_kit_profile(profile, ctx) {
                self.kits[self.active].pending_launch_tags = None;
                self.status = format!("Command line: {}", self.status);
            }
            return;
        }
        let Some(path) = profile
            .map(|profile| profile.root)
            .or_else(|| self.prefs.editing_kit_paths.get(shortcut.game.as_str()).cloned())
        else {
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
            shortcut.game.as_str().to_owned(),
            shortcut.game.display_name().to_owned(),
            None,
            false,
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
        let folder_seeds = self.kits[self.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            for entry in &entries {
                if source.entry_for_key(&entry.key).is_none() {
                    source.upsert_entry(entry.clone(), &folder_seeds);
                }
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
        if profile.is_campaign_evolved() {
            self.begin_load_folder_path(profile.root.clone(), ctx);
            self.kits[self.active].profile = Some(EditingKitProfileIdentity {
                id: profile.id,
                name: profile.name,
            });
            return true;
        }
        let chosen_folders = profile.has_chosen_folders();
        self.begin_load_editing_kit_layout(
            layout,
            profile.game.clone(),
            profile.name.clone(),
            Some(EditingKitProfileIdentity {
                id: profile.id,
                name: profile.name,
            }),
            chosen_folders,
            ctx,
        );
        true
    }

    /// Load a kit from its validated layout. `chosen_folders` is a profile
    /// that names its own tags or data folder: the loaded source then carries
    /// exactly this layout, rather than working out its root and data folder
    /// from its tags folder, and the kit is identified by its tags folder,
    /// since its root may be shared with other kits.
    fn begin_load_editing_kit_layout(
        &mut self,
        layout: EditingKitLayout,
        game: String,
        label: String,
        profile: Option<EditingKitProfileIdentity>,
        chosen_folders: bool,
        ctx: egui::Context,
    ) {
        // Profiles keep the game id they were saved with; one this build does
        // not know has no definitions to load against.
        let Some(game) = GameId::from_id(&game) else {
            self.status = format!("{label} is for a game this version of Baboon does not know ({game})");
            return;
        };
        let chosen_layout = chosen_folders.then(|| KitLayout {
            root: layout.root.clone(),
            tags: layout.tags.clone(),
            data: layout
                .data
                .clone()
                .unwrap_or_else(|| layout.root.join("data")),
        });
        // What the kit is remembered and matched by: its root, unless other
        // kits may share that root, when it is its tags folder.
        let identity_path = if chosen_folders {
            layout.tags.clone()
        } else {
            layout.root.clone()
        };
        if let Some(profile_identity) = profile.as_ref() {
            if let Some(index) = self.kits.iter().position(|kit| {
                kit.profile.as_ref().map(|open| open.id.as_str())
                    == Some(profile_identity.id.as_str())
            }) {
                self.active = index;
                self.status = format!("Switched to {}", label);
                return;
            }
            // A kit already open on this profile's tags folder (opened as a
            // folder) becomes this profile's. Matched on the tags folder, not
            // the root: kits sharing a root are different kits.
            if !chosen_folders
                && let Some(index) = self.kits.iter().position(|kit| {
                    kit.requested_path
                        .as_deref()
                        .is_some_and(|open| same_recent_path(open, &layout.root))
                        && kit
                            .source
                            .as_ref()
                            .and_then(LoadedSourceData::kit_layout)
                            .is_some_and(|open| same_recent_path(&open.tags, &layout.tags))
                        && kit
                            .source
                            .as_ref()
                            .and_then(|source| source.game)
                            == Some(game)
                })
            {
                self.active = index;
                self.kits[index].profile = Some(profile_identity.clone());
                self.status = format!("Switched to {}", label);
                return;
            }
            if !self.kits[self.active].can_accept_source_load() {
                self.add_kit();
            }
            self.kits[self.active].requested_path = Some(identity_path.clone());
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
        let recent_path = identity_path;
        self.status = format!("Indexing {} as {game}", tags_root.display());
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_editing_kit_layout(tags_root, label, game, &names, &definitions_root)
                    .map(|mut source| {
                        source.chosen_kit_layout = chosen_layout;
                        source
                    })
                    .map_err(|error| error.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
    }

    pub(super) fn choose_editing_kit_path(&mut self, shortcut: EditingKitShortcut) {
        let title = if shortcut.game.is_campaign_evolved() {
            "Select Campaign Evolved Install or Paks Folder".to_owned()
        } else {
            format!("Select {} Editing Kit Folder", shortcut.label)
        };
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(path) = self.prefs.editing_kit_paths.get(shortcut.game.as_str()) {
            if path.is_dir() {
                dialog = dialog.set_directory(path);
            } else if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
                dialog = dialog.set_directory(parent);
            }
        }
        if let Some(path) = dialog.pick_folder() {
            self.prefs
                .editing_kit_paths
                .insert(shortcut.game.as_str().to_owned(), path.clone());
            self.editing_kit_path_inputs
                .insert(shortcut.game.as_str().to_owned(), path.display().to_string());
            if self.editing_kit_path_attention.as_deref() == Some(shortcut.game.as_str()) {
                self.editing_kit_path_attention = None;
            }
            self.status = format!("{} path set to {}", shortcut.label, path.display());
            self.refresh_builtin_editing_kit_validation(shortcut);
        }
    }

    pub(super) fn auto_detect_editing_kit_paths(&mut self) {
        let detected = detect_editing_kit_paths();
        let previous = self.prefs.custom_editing_kit_profiles.clone();
        let added = add_standard_editing_kit_profiles(
            &mut self.prefs.custom_editing_kit_profiles,
            &detected,
        );
        if added > 0 {
            let prefs = self.current_prefs();
            if let Err(error) = save_gui_prefs(
                &prefs,
                &self.terminal_open_games,
                self.first_run_wizard.is_none(),
            ) {
                self.prefs.custom_editing_kit_profiles = previous;
                self.status = error;
                return;
            }
            self.saved_prefs = prefs;
            self.saved_terminal_open_games = self.terminal_open_games.clone();
        }
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
        self.editing_kit_path_attention = Some(shortcut.game.as_str().to_owned());
        self.editing_kit_path_inputs
            .entry(shortcut.game.as_str().to_owned())
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
        let options = self.active_kit_tool_folder_options();
        self.spawn_tool(label, &path, self.editing_kit_root(), &options);
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
        let options = self.active_kit_tool_folder_options();
        self.spawn_tool(label, &path, Some(root), &options);
    }

    fn spawn_tool(
        &mut self,
        label: &str,
        path: &Path,
        work_dir: Option<PathBuf>,
        options: &[(&'static str, PathBuf)],
    ) {
        let mut command = Command::new(path);
        for (option, folder) in options {
            command.arg(option).arg(folder);
        }
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
            self.terminal_open_games.insert(game.as_str().to_owned());
        } else {
            self.terminal_open_games.remove(game.as_str());
        }
    }

    /// Run a geometry Import request (`tool render/collision/physics/...`)
    /// streamed to the terminal panel.
    pub(super) fn process_pending_tool_import(&mut self, ctx: &egui::Context) {
        if self.editing_kit_is_read_only(self.active) {
            self.pending_tool_import = None;
            self.refuse_read_only_edit(self.active);
            return;
        }
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

    /// Queue the same editing-kit geometry import that a compatible tag
    /// reference offers, deriving the tool source folder from the clicked tag.
    pub(super) fn begin_reimport_geometry(&mut self, key: &str) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "The tag is no longer in the browser".to_owned();
            return;
        };
        if !matches!(entry.location, TagEntryLocation::LooseFile(_)) {
            self.status = "Reimport requires a loose editing-kit tag".to_owned();
            return;
        }
        let Some(verb) = geometry_import_verb(self.names(), entry.group_tag) else {
            self.status = "This tag type does not support reimport".to_owned();
            return;
        };
        self.pending_tool_import = Some(ToolImportRequest {
            verb,
            source_dir: model_source_dir(&entry_rel_path(&entry)),
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn begin_reimport_bitmap(&mut self, key: String, ctx: egui::Context) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
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
        let Some(work_dir) = self.kit_layout_for(self.active).map(|layout| layout.root) else {
            self.status = "Could not resolve editing-kit root".to_owned();
            return;
        };
        let Some(data_path) = bitmap_reimport_data_path(&entry, Some(tags_root)) else {
            self.status = "Could not resolve bitmap data path".to_owned();
            return;
        };
        let command = with_tool_folder_options(
            &format!("tool bitmaps \"{data_path}\""),
            &self.active_kit_tool_folder_options(),
        );
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
            if routed && self.refuse_read_only_edit(self.active) {
                self.block_confirm = None;
                return;
            }
            if let Some(confirm) = self.block_confirm.take()
                && routed
            {
                let deletes_model_variant = confirm.path == "variants"
                    && matches!(confirm.kind, BlockOpKind::Delete(_))
                    && self.kits[self.active]
                        .parsed_tags
                        .get(&confirm.tag_key)
                        .is_some_and(|doc| doc.tag.header.group_tag.to_be_bytes() == *b"hlmt");
                let ops = DeferredOps {
                    block_ops: vec![BlockOp {
                        path: confirm.path,
                        kind: confirm.kind,
                    }],
                    ..DeferredOps::default()
                };
                let active = self.active;
                let applied =
                    self.apply_doc_ops(active, &confirm.tag_key, "Block edit", ops, UndoStep::Own);
                let refresh_model_preview = deletes_model_variant
                    && applied.is_some_and(|applied| applied.status.is_some());
                if refresh_model_preview
                    && let Some(preview) = self.kits[self.active]
                        .model_previews
                        .get_mut(&confirm.tag_key)
                {
                    preview.selected_variant = None;
                    preview.invalidate_load();
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
                        self.request_close_action(action, ctx);
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
                self.request_close_action(action, ctx);
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
                    match close_prompt_save_route(
                        self.entry_for_key(&tag_id).map(|entry| &entry.location),
                    ) {
                        ClosePromptSave::NewContainer => {
                            self.save_new_container_tag(&tag_id);
                            if self.tag_is_dirty(&tag_id) {
                                let label = self.tag_path_label(&tag_id);
                                errors.push(format!("{label}: not saved"));
                            } else {
                                saved.push(tag_id.clone());
                            }
                            continue;
                        }
                        ClosePromptSave::ContainerInPlace => {
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
                        ClosePromptSave::File => {}
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
                    self.request_close_action(action, ctx);
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
                    self.prefs.session_restore = SessionRestore::Never;
                }
                self.last_opened_windows = None;
            }
            LastOpenedWindowsAction::Restore { kits, remember } => {
                if remember {
                    self.prefs.session_restore = SessionRestore::Always;
                }
                self.last_opened_windows = None;
                self.begin_last_session_restore(kits, ctx.clone());
            }
        }
    }

    pub(super) fn persist_prefs_if_changed(&mut self) {
        let _ = self.try_persist_prefs();
    }

    /// Write prefs if they changed; whether a write failed.
    fn try_persist_prefs(&mut self) -> bool {
        let prefs = self.current_prefs();
        if prefs == self.saved_prefs && self.terminal_open_games == self.saved_terminal_open_games {
            return false;
        }
        match save_gui_prefs(&prefs, &self.terminal_open_games, true) {
            Ok(()) => {
                self.saved_prefs = prefs;
                self.saved_terminal_open_games = self.terminal_open_games.clone();
                false
            }
            Err(error) => {
                self.status = error;
                true
            }
        }
    }

    /// The per-frame prefs check, at most once a second.
    ///
    /// It ran every frame: a full GuiPrefs rebuilt (recents, favorites, kit
    /// profiles and swatches cloned) just to compare, and while a window, the
    /// UI-scale slider or a splitter was being dragged the value changed every
    /// frame, so prefs.json was rewritten at the frame rate. A failed write was
    /// retried, and reported, every frame too. Explicit calls (settings, runtime
    /// poke) still write at once, and exit flushes whatever is pending.
    pub(super) fn persist_prefs_throttled(&mut self, now: f64) {
        const CHECK_INTERVAL: f64 = 1.0;
        const RETRY_AFTER_FAILURE: f64 = 10.0;
        if now < self.prefs_next_check_at {
            return;
        }
        let failed = self.try_persist_prefs();
        self.prefs_next_check_at = now
            + if failed {
                RETRY_AFTER_FAILURE
            } else {
                CHECK_INTERVAL
            };
    }
}

fn close_action_includes_chimp(action: &PendingCloseAction) -> bool {
    matches!(
        action,
        PendingCloseAction::CloseApp | PendingCloseAction::CloseKit(_)
    )
}

/// The command that opens `folder` in the platform's file manager.
fn folder_opener(folder: &Path) -> Command {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(windows, target_os = "macos")))]
    let program = "xdg-open";
    let mut command = Command::new(program);
    command.arg(folder);
    command
}

fn reset_lazy_folder_browser(
    root: &Path,
    tree: &mut TagTree,
    entries: &mut Vec<TagEntry>,
) -> Result<(), String> {
    *tree = crate::core::source::build_folder_directory_tree(root).map_err(|error| error.to_string())?;
    entries.clear();
    Ok(())
}

#[cfg(test)]
mod browser_refresh_tests;

#[cfg(test)]
mod save_changes_prompt_tests;


#[cfg(test)]
mod campaign_import_gate_tests;

#[cfg(test)]
mod chimp_surface_undo_tests;

#[cfg(test)]
mod worker_panic_tests;

#[cfg(test)]
pub(in crate::app) mod loose_fixture;

#[cfg(test)]
mod save_close_session_tests;

#[cfg(test)]
mod browser_action_table_tests;


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
        .default_width(window_width(ctx, 520.0))
        .default_height(window_height(ctx, 260.0, true))
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

fn last_opened_workspace_heading(
    profile: Option<(&str, &Path)>,
    game: Option<&str>,
    source_path: &Path,
    project_path: Option<&Path>,
) -> (String, Option<String>) {
    if let Some((name, root)) = profile {
        return (name.to_owned(), Some(root.display().to_string()));
    }
    if let Some(project_path) = project_path {
        let name = project_path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .or_else(|| project_path.file_name().and_then(|name| name.to_str()))
            .map(str::to_owned)
            .unwrap_or_else(|| project_path.display().to_string());
        return (name, Some(project_path.display().to_string()));
    }

    let heading = match game {
        Some(game) => game_display_name(game).to_owned(),
        None => source_path.display().to_string(),
    };
    (heading, None)
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
        .default_width(window_width(ctx, 520.0))
        .default_height(window_height(ctx, 300.0, true))
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
                    let displayed_source_path =
                        kit.profile_root.as_deref().unwrap_or(&kit.source_path);
                    let (heading, project_path) = last_opened_workspace_heading(
                        kit.profile_name.as_deref().zip(kit.profile_root.as_deref()),
                        kit.game.as_deref(),
                        &kit.source_path,
                        kit.project_path.as_deref(),
                    );
                    let heading_response =
                        ui.label(RichText::new(heading).color(text_dark()).strong());
                    if let Some(project_path) = project_path {
                        heading_response.on_hover_text(&project_path);
                        ui.label(RichText::new(project_path).color(subtle_dark()).small());
                    } else {
                        heading_response.on_hover_text(kit.source_path.display().to_string());
                    }
                    if !kit.source_available {
                        ui.label(
                            RichText::new(format!(
                                "Missing source: {}",
                                displayed_source_path.display()
                            ))
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
                    if !kit.folder_entries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Folders").color(subtle_dark()).strong());
                        });
                    }
                    for entry in &mut kit.folder_entries {
                        ui.add_enabled_ui(entry.available, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.checkbox(&mut entry.checked, "");
                                let label = if entry.available {
                                    entry.folder.rel_path.display().to_string()
                                } else {
                                    format!("{} (missing source)", entry.folder.rel_path.display())
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

fn loose_entry_key_for_canonical_path<'a>(
    mut entries: impl Iterator<Item = &'a TagEntry>,
    canonical_path: &Path,
) -> Option<String> {
    entries.find_map(|entry| match &entry.location {
        TagEntryLocation::LooseFile(entry_path)
            if fs::canonicalize(entry_path)
                .ok()
                .is_some_and(|path| path == canonical_path) =>
        {
            Some(entry.key.clone())
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests;

/// What a TSV paste did, counted from the per-cell outcomes. It used to count
/// the cells it tried, so a paste whose cells all failed to parse still said
/// every one of them was pasted.
fn tsv_paste_summary(
    outcomes: &[FieldEditOutcome],
    applied_rows: usize,
    skipped_rows: usize,
    element_count: usize,
) -> String {
    let failed: Vec<&FieldEditOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.result.is_err())
        .collect();
    let pasted = outcomes.len() - failed.len();
    let mut summary = if failed.is_empty() {
        format!("Pasted {pasted} cell(s) across {applied_rows} row(s)")
    } else {
        format!(
            "Pasted {pasted} of {} cell(s) across {applied_rows} row(s)",
            outcomes.len()
        )
    };
    if let Some(first) = failed.first() {
        let error = first
            .result
            .as_ref()
            .err()
            .map(String::as_str)
            .unwrap_or("");
        summary.push_str(&format!(
            " — {} failed; first: {} = \"{}\": {error}",
            failed.len(),
            first.path,
            first.input
        ));
    }
    if skipped_rows > 0 {
        summary.push_str(&format!(
            " — {skipped_rows} extra row(s) ignored (block has {element_count} elements; add more first)"
        ));
    }
    summary
}

#[cfg(test)]
mod listing_entries_tests;

#[cfg(test)]
mod tsv_paste_summary_tests;

/// Write what a refresh found into the on-disk indexes, row by row, and read
/// the references of the tags that changed. Runs on the refresh worker.
///
/// A refresh used to hand the whole entry list back to the UI, which dropped
/// the reference index and spawned a rewrite of every index row, a stat per
/// tag although the refresh had just taken them all. Only the tags that
/// changed are touched now.
fn persist_entry_index_changes(
    game: &str,
    root: &Path,
    tag_source: &TagSource,
    mut refresh: EntryIndexRefresh,
) -> EntryIndexRefresh {
    for key in &refresh.removed_keys {
        if let Err(error) = crate::core::source::delete_entry_with_dependencies(game, root, key) {
            refresh.errors.push(format!("{key}: {error:#}"));
        }
    }
    // Each tag's row (its fingerprint) and its references go in one
    // transaction. Written apart with the errors dropped, a failure between
    // them left a current fingerprint over stale references, and no later
    // refresh would look at that tag again.
    for entry in &refresh.touched {
        let references = read_entry_dependencies(tag_source, entry);
        let written = crate::core::source::upsert_entry_with_dependencies(
            game,
            root,
            entry,
            references.as_deref().ok(),
        );
        if let Err(error) = written {
            refresh
                .errors
                .push(format!("{}: {error:#}", entry.display_path));
        }
        match references {
            Ok(references) => refresh
                .touched_dependencies
                .push((entry.key.clone(), references)),
            Err(error) => refresh.errors.push(format!(
                "{}: could not read its references: {error}",
                entry.display_path
            )),
        }
    }
    refresh
}

#[cfg(test)]
mod container_dependency_tests;

/// How the close prompt's Save writes one tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClosePromptSave {
    /// A brand-new container tag: a new override container, via a dialog.
    NewContainer,
    /// A mounted container tag: overwritten inside its own pak.
    ContainerInPlace,
    /// Everything else is a file on disk, or has none and is refused there.
    File,
}

/// A container tag has no file to write, so sending one down the file path
/// would fail, or worse, write the payload somewhere it does not belong.
fn close_prompt_save_route(location: Option<&TagEntryLocation>) -> ClosePromptSave {
    match location {
        Some(TagEntryLocation::NewContainer { .. }) => ClosePromptSave::NewContainer,
        Some(TagEntryLocation::Container { .. }) => ClosePromptSave::ContainerInPlace,
        Some(TagEntryLocation::LooseFile(_) | TagEntryLocation::Monolithic { .. }) | None => {
            ClosePromptSave::File
        }
    }
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

/// Favorite folders at or under `from` follow it to `to` (both relative to the
/// tags root). Folders are compared component by component ignoring case, the
/// way tag paths are, and keep the case of the part below the moved folder.
fn remap_favorite_folders(folders: &mut [PathBuf], from: &Path, to: &Path) {
    let from: Vec<String> = from
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    for folder in folders {
        let parts: Vec<_> = folder.components().collect();
        if parts.len() < from.len()
            || !parts
                .iter()
                .zip(&from)
                .all(|(part, wanted)| part.as_os_str().to_string_lossy().to_ascii_lowercase() == *wanted)
        {
            continue;
        }
        let mut moved = to.to_path_buf();
        for part in &parts[from.len()..] {
            moved.push(part);
        }
        *folder = moved;
    }
}

#[cfg(test)]
mod favorite_folder_tests;

fn remap_favorite_paths(
    root: &Path,
    relative_paths: &mut [PathBuf],
    old_to_new_keys: &HashMap<String, String>,
) {
    for relative_path in relative_paths {
        let old_key = file_entry_key(&root.join(&*relative_path));
        let Some(new_key) = old_to_new_keys
            .iter()
            .find_map(|(old, new)| same_entry_key(old, &old_key).then_some(new))
        else {
            continue;
        };
        let Some(new_path) = file_key_path(new_key).map(Path::to_path_buf) else {
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

#[cfg(test)]
mod tsv_paste_tests;

/// Retire one restored kit's load and report the kit that should take the focus
/// — `None` while any restore is still outstanding, or when the session named
/// no kit and there is nothing to honour.
///
/// Split out from [`Baboon::settle_restored_kit`] because it is the whole
/// decision: the app half only turns the answer into an index.
fn focus_after_restore(
    restoring: &mut HashSet<KitId>,
    restored_active: &mut Option<KitId>,
    settled: KitId,
) -> Option<KitId> {
    // A load that was not part of the restore settles nothing, and neither does
    // one that still leaves others in flight.
    if !restoring.remove(&settled) || !restoring.is_empty() {
        return None;
    }
    restored_active.take()
}

#[cfg(test)]
mod restore_focus_tests;

#[cfg(test)]
mod field_search_tests;

#[cfg(test)]
mod saved_tag_index_tests;

#[cfg(test)]
mod folder_opener_tests;

#[cfg(test)]
mod prefs_throttle_tests;

