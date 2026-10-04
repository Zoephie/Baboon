//! Import Tags: pull loose tags from another game's editing kit into this one.
//!
//! What a tag *becomes* in another game lives in `blam_tags::convert`, and the
//! folder worker itself is `conversion::run_folder_conversion_job`. This module owns
//! the editor's side: resolving what the user typed or picked, deciding which game
//! it came from, and landing the result in the active kit.

use super::*;
use blam_tags::classic::{ClassicEngine, ClassicHeader};

/// What a source path turned out to be, measured on a worker.
///
/// Deliberately a *finding* rather than a decision: the dialog shows every field
/// and lets the user overrule the detected profile. A wrong guess that announces
/// itself as a guess is recoverable; one presented as fact is not.
pub(in crate::app) struct ImportSourceFacts {
    pub(in crate::app) path: PathBuf,
    pub(in crate::app) is_folder: bool,
    /// Files that probe as tags — what the import will actually process.
    pub(in crate::app) tag_files: usize,
    /// Files that do not. Shown because "0 tags" and "0 files" are different
    /// mistakes: the first is the wrong folder, the second is a typo.
    pub(in crate::app) skipped_files: usize,
    /// The group of a single-file source, for naming the destination.
    pub(in crate::app) group_tag: Option<u32>,
    /// The profile this reads as, and the sentence explaining how that was
    /// decided. `None` when nothing identified it and the user has to say.
    pub(in crate::app) detected_game: Option<(String, String)>,
}

/// Size and modification time of the file a preview was built from.
///
/// Cheaper than re-hashing the tag and it answers the same question — has the
/// file moved on since the preview? A stat is enough because the gap being
/// guarded is one user's Analyze-then-Import, not a hostile writer.
pub(in crate::app) type SourceStamp = (u64, Option<std::time::SystemTime>);

/// A converted single tag on its way back to the UI thread.
///
/// `losses` is non-empty when the default policy refused it — the tag is here,
/// converted, and waiting on whether that loss is acceptable.
pub(in crate::app) struct ImportAnalysis {
    pub(in crate::app) draft: TagConversionDraft,
    pub(in crate::app) stamp: Option<SourceStamp>,
    pub(in crate::app) losses: Vec<String>,
    pub(in crate::app) refusal: Option<String>,
}

/// Built [`NativeTemplateIndex`]es, one per game, kept between imports.
///
/// Held on `Baboon` rather than in the dialog because it outlives any one
/// import. Keyed by game *and* the tags root each was built from, because a user
/// can point two workspaces at two installs of the same game — and because a
/// reconfigured kit must rebuild rather than silently reuse the old tree.
///
/// More than one entry because a routed conversion converts into each engine it
/// passes through, and each of those hops wants its own kit's templates.
#[derive(Default)]
pub(in crate::app) struct NativeTemplateCache {
    built: HashMap<String, (PathBuf, NativeTemplateIndex)>,
}

impl NativeTemplateCache {
    /// Build the index for `game` if it is missing or was built from a different
    /// kit. Walking a kit takes about a second, so this is the call that has to
    /// not happen twice.
    pub(in crate::app) fn ensure(&mut self, game: &str, tags_root: &Path, definitions_root: &Path) {
        if self
            .built
            .get(game)
            .is_some_and(|(root, _)| root == tags_root)
        {
            return;
        }
        let Ok(groups) = GameTagIndex::load(definitions_root, game) else {
            return;
        };
        self.built.insert(
            game.to_owned(),
            (
                tags_root.to_path_buf(),
                NativeTemplateIndex::build(tags_root, &groups),
            ),
        );
    }
}

impl TemplateSource for NativeTemplateCache {
    fn templates_for(&self, game: &str) -> Option<&NativeTemplateIndex> {
        self.built.get(game).map(|(_, index)| index)
    }
}

/// What happened when a tag was converted, from the point of view of someone who
/// has to decide what to do about it.
pub(in crate::app) enum ConversionOutcome {
    /// Converted with nothing the audit objects to. Write it.
    Clean(TagConversionDraft),
    /// Refused by default because it loses audited data, and this is the tag
    /// that accepting the loss produces. Nothing is written until somebody says
    /// so; `refusal` is the sentence explaining what the default objected to.
    Lossy {
        draft: Box<TagConversionDraft>,
        refusal: String,
    },
    /// Could not be converted at all, at any loss.
    Failed(String),
}

impl ConversionOutcome {
    #[cfg(test)]
    /// The audited fields this conversion gives up, empty unless it is lossy.
    pub(in crate::app) fn losses(&self) -> &[String] {
        match self {
            ConversionOutcome::Lossy { draft, .. } => &draft.report.fail_closed_losses,
            _ => &[],
        }
    }
}

/// Convert a tag and say which of the three things happened.
///
/// The default policy runs first, and it runs the whole route search — so a pair
/// that can reach the destination without losing anything still does, and only a
/// tag that nothing can carry cleanly pays for the second attempt.
///
/// Deliberately *not* trying to pick the least-lossy route once loss is on the
/// table. Under an accepting policy every route "succeeds", so choosing between
/// them would mean ranking by the number of audited fields lost — and audited
/// means audited *for that pair*. Halo 3 to ODST is not audited at all, so a
/// longer route can report fewer losses while having more places to lose
/// something quietly. Preferring the direct conversion keeps the hops, and the
/// opportunities, to a minimum.
pub(in crate::app) fn convert_tag_outcome(
    source: &TagFile,
    source_game: &str,
    target_game: &str,
    definitions_root: &Path,
    kit_roots: &HashMap<String, PathBuf>,
    cache: &mut NativeTemplateCache,
) -> ConversionOutcome {
    let refusal = match convert_tag_routed(
        source,
        source_game,
        target_game,
        definitions_root,
        kit_roots,
        cache,
        LossPolicy::FailClosed,
    ) {
        Ok(draft) => return ConversionOutcome::Clean(draft),
        Err(refusal) => refusal,
    };
    match convert_tag_routed(
        source,
        source_game,
        target_game,
        definitions_root,
        kit_roots,
        cache,
        LossPolicy::Accept,
    ) {
        // Accepting the loss got it through, so the refusal was about data going
        // missing rather than the tag being unconvertible.
        Ok(draft) if !draft.report.fail_closed_losses.is_empty() => ConversionOutcome::Lossy {
            draft: Box::new(draft),
            refusal,
        },
        // Succeeded with nothing recorded as lost, which means the first attempt
        // failed for some other reason that the second happened not to hit. Trust
        // the refusal rather than the surprise.
        Ok(_) | Err(_) => ConversionOutcome::Failed(refusal),
    }
}

/// Convert one tag, routing through intermediate engines when the direct pair
/// refuses to carry it.
///
/// The direct attempt runs first with only the destination's templates, so a
/// conversion that works pays nothing for the fallback existing. Only when it is
/// refused does this go looking: it works out which engines a route could pass
/// through, indexes whichever of those kits are configured, and hands the lot to
/// the engine to walk.
///
/// No intermediate file is ever written. The engine hands each hop the previous
/// one's serialized bytes, so the tag really does pass through Halo 3 on its way
/// from Halo 2 to Reach — it just never touches a kit on the way, which means
/// there is nothing to clean up and a failed hop leaves no debris behind.
/// `kit_roots` maps a game to its **tags** directory, already resolved. Resolving
/// here as well would be wrong rather than merely redundant: `import_tags_root`
/// appends `tags` to anything not already called that, so a second pass turns a
/// perfectly good root into a path that does not exist and the search silently
/// finds no templates at all.
pub(in crate::app) fn convert_tag_routed(
    source: &TagFile,
    source_game: &str,
    target_game: &str,
    definitions_root: &Path,
    kit_roots: &HashMap<String, PathBuf>,
    cache: &mut NativeTemplateCache,
    policy: LossPolicy,
) -> Result<TagConversionDraft, String> {
    if let Some(root) = kit_roots.get(target_game) {
        cache.ensure(target_game, root, definitions_root);
    }
    let direct = analyze_conversion_with_policy(
        source,
        source_game,
        target_game,
        definitions_root,
        cache.templates_for(target_game),
        policy,
    );
    let Err(direct_error) = direct else {
        return direct;
    };

    // Only the engines a route could actually pass through — which is a short
    // list, and never includes one outside the span between the two endpoints.
    let waypoints: HashSet<String> = conversion_routes(source_game, target_game)
        .into_iter()
        .flatten()
        .collect();
    if waypoints.len() <= 2 {
        // Nothing to route through; the direct refusal is the whole answer.
        return Err(direct_error);
    }
    for game in &waypoints {
        if let Some(root) = kit_roots.get(game) {
            cache.ensure(game, root, definitions_root);
        }
    }
    analyze_conversion_routed_with_policy(
        source,
        source_game,
        target_game,
        definitions_root,
        cache,
        policy,
    )
}

pub(in crate::app) struct TagImportDialog {
    /// The workspace this lands in. A modeless dialog outlives the frame that
    /// opened it, so the user can focus another game in between; resolving this
    /// first is what keeps the write on the kit the import was started from.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) target_game: String,
    pub(in crate::app) target_tags_root: PathBuf,
    /// What the user typed or picked. A path, not a mode — whether it names a
    /// file or a directory is what decides between one tag and a recursive run,
    /// so a pasted path is as good as the file picker rather than a lesser
    /// version of it.
    pub(in crate::app) source_input: String,
    /// The input `facts` were resolved from. A mismatch is how the dialog knows
    /// to say the path has not been checked yet.
    pub(in crate::app) resolved_input: String,
    pub(in crate::app) resolving: bool,
    pub(in crate::app) facts: Option<ImportSourceFacts>,
    pub(in crate::app) source_game: String,
    pub(in crate::app) source_game_note: String,
    /// Destination, relative to the target kit's tags root. For a single tag
    /// this includes the leaf name and excludes the extension, which belongs to
    /// the *target* group and so is not known until the conversion is analyzed.
    pub(in crate::app) destination_rel: String,
    /// The folder this dialog was raised on, if it was raised from one. Kept
    /// apart from `destination_rel` so resolving a source can append the
    /// source's own name to it without having to guess what the user typed.
    pub(in crate::app) destination_base: String,
    /// Set the moment the user edits the destination themselves, after which
    /// nothing re-seeds it. Picking a different source should still update a
    /// destination Baboon chose; it must never overwrite one the user chose.
    pub(in crate::app) destination_touched: bool,
    pub(in crate::app) analyzing: bool,
    /// Whether the analysis now running is the first half of an import, rather
    /// than a preview being refreshed on its own.
    pub(in crate::app) write_when_analyzed: bool,
    pub(in crate::app) draft: Option<TagConversionDraft>,
    /// Audited fields this tag gives up, when the default policy refused it and
    /// the user has not yet said whether to accept. Non-empty means the dialog is
    /// asking a question and nothing has been written.
    pub(in crate::app) pending_losses: Vec<String>,
    /// Why the default refused, kept beside the losses so the question can be
    /// asked in the converter's own words rather than a paraphrase.
    pub(in crate::app) pending_refusal: Option<String>,
    /// What the completed single-tag import wrote. Keeps the window up with its
    /// report afterwards, the way the folder import does, instead of closing on
    /// success and leaving the outcome only in the status bar.
    pub(in crate::app) written: Option<String>,
    pub(in crate::app) draft_stamp: Option<SourceStamp>,
    pub(in crate::app) running: bool,
    pub(in crate::app) progress: Option<FolderConversionProgress>,
    pub(in crate::app) report: Option<FolderConversionReport>,
    pub(in crate::app) error: Option<String>,
}

impl TagImportDialog {
    /// The absolute file a single-tag import would write, once the target
    /// extension is known. `None` until the conversion has been analyzed —
    /// the extension comes from the target group, not the source file.
    pub(in crate::app) fn single_output(&self) -> Option<PathBuf> {
        let draft = self.draft.as_ref()?;
        single_output_path(
            &self.target_tags_root,
            &self.destination_rel,
            &draft.target_extension,
        )
    }

    /// The root a folder import would write under.
    pub(in crate::app) fn folder_output_root(&self) -> Option<PathBuf> {
        let relative = normalize_import_rel(&self.destination_rel);
        if relative.is_empty() {
            return None;
        }
        Some(self.target_tags_root.join(relative))
    }

    pub(in crate::app) fn source_is_folder(&self) -> bool {
        self.facts.as_ref().is_some_and(|facts| facts.is_folder)
    }

    /// Whether the resolved facts still describe what is in the path box.
    pub(in crate::app) fn facts_are_current(&self) -> bool {
        self.facts.is_some() && self.resolved_input == normalize_import_input(&self.source_input)
    }

    /// Drop everything derived from the source, leaving the path and destination.
    /// Called whenever the source or the profile changes: a draft belongs to the
    /// exact pair it was made from.
    fn invalidate_analysis(&mut self) {
        self.draft = None;
        self.draft_stamp = None;
        self.pending_losses.clear();
        self.pending_refusal = None;
        self.report = None;
        self.error = None;
    }
}

/// Tidy a pasted path: Explorer's "Copy as path" wraps it in quotes, and a path
/// dragged from a terminal often arrives with trailing whitespace. Both are
/// paste accidents rather than user intent, and neither names a real file.
pub(in crate::app) fn normalize_import_input(input: &str) -> String {
    input.trim().trim_matches('"').trim().to_owned()
}

/// Tidy a typed destination into a relative path with forward slashes and no
/// leading, trailing or doubled separators.
pub(in crate::app) fn normalize_import_rel(input: &str) -> String {
    input
        .replace('\\', "/")
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// Where a source lands by default: under `base`, keeping its own name.
///
/// A folder keeps its whole name and a file only its stem, because the file's
/// extension names the *source* group and the imported tag's will name the
/// target's — `rifle.weapon` from Halo 3 is `rifle`, not `rifle.weapon`, before
/// Reach's extension goes back on.
pub(in crate::app) fn seeded_destination(base: &str, source: &Path, is_folder: bool) -> String {
    let leaf = if is_folder {
        source.file_name()
    } else {
        source.file_stem()
    }
    .and_then(|name| name.to_str())
    .unwrap_or_default();
    normalize_import_rel(&format!("{base}/{leaf}"))
}

/// Where a single imported tag is written.
///
/// The extension comes from the *target* group, never the source file: an H3
/// `shader` becomes an H4 `.material`, and a Reach `contrail` an H3
/// `.contrail_system`. `set_extension` rather than a push also means a user who
/// pasted `rifle.weapon` out of habit gets `rifle.<target>` instead of
/// `rifle.weapon.<target>`.
pub(in crate::app) fn single_output_path(
    tags_root: &Path,
    destination_rel: &str,
    target_extension: &str,
) -> Option<PathBuf> {
    let relative = normalize_import_rel(destination_rel);
    if relative.is_empty() {
        return None;
    }
    let mut output = tags_root.join(relative);
    output.set_extension(target_extension);
    Some(output)
}

/// The four paths the folder worker wants, from the two the user gave.
pub(in crate::app) struct FolderImportPlan {
    pub(in crate::app) source_root: PathBuf,
    pub(in crate::app) source_rel_path: PathBuf,
    pub(in crate::app) destination_parent: PathBuf,
    pub(in crate::app) destination_label: String,
}

/// Split the source folder and the typed destination into what the worker wants.
///
/// The worker reads relative to a root and writes relative to a parent, so both
/// paths are split at their last component. That split lives here rather than
/// inline because it is the part most likely to be silently wrong: swap a parent
/// for a leaf and it still typechecks, still writes files, and still reports
/// success — just one directory level away from where the user asked.
pub(in crate::app) fn plan_folder_import(
    source_folder: &Path,
    tags_root: &Path,
    destination_rel: &str,
) -> Result<FolderImportPlan, String> {
    let relative = normalize_import_rel(destination_rel);
    if relative.is_empty() {
        return Err("Enter a destination folder for the imported tags".to_owned());
    }
    let source_folder = normalize_conversion_path(source_folder);
    let output_root = normalize_conversion_path(&tags_root.join(&relative));
    // Either way round is the same accident: a run that reads what it is writing.
    if output_root.starts_with(&source_folder) || source_folder.starts_with(&output_root) {
        return Err("The destination must not overlap the folder being imported".to_owned());
    }
    // The source folder's own parent becomes the worker's root, so the relative
    // paths it builds keep the folder's name at the top — which is what makes
    // the destination mirror the source's shape rather than flattening it.
    let source_root = source_folder
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Choose a folder inside a kit, not a drive root".to_owned())?;
    let source_rel_path = source_folder
        .file_name()
        .map(PathBuf::from)
        .ok_or_else(|| "Choose a folder inside a kit, not a drive root".to_owned())?;
    let destination_parent = output_root
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Enter a destination folder for the imported tags".to_owned())?;
    let destination_label = output_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| "Enter a destination folder for the imported tags".to_owned())?;
    Ok(FolderImportPlan {
        source_root,
        source_rel_path,
        destination_parent,
        destination_label,
    })
}

/// The profiles that can be imported *from*, given where the tags are going.
pub(in crate::app) fn import_sources_for(target_game: &str) -> Vec<&'static str> {
    CONVERSION_PROFILES
        .iter()
        .copied()
        .filter(|source| conversion_pair_supported(source, target_game))
        .collect()
}

impl Baboon {
    /// Whether Import Tags can act on the active kit. Loose kits only: a
    /// monolithic cache is read-only, and a Campaign Evolved container has its
    /// own import path because a tag there is a package, not a file.
    pub(in crate::app) fn can_import_tags(&self) -> bool {
        if self.editing_kit_is_read_only(self.model.active) {
            return false;
        }
        self.source().is_some_and(|source| {
            matches!(source.source, TagSource::LooseFolder { .. }) && source.game.is_some()
        })
    }

    fn refuse_read_only_tag_import(&mut self) -> bool {
        let index = self
            .import.tag_import_dialog
            .as_ref()
            .and_then(|dialog| self.kit_index(dialog.kit));
        index.is_some_and(|index| self.refuse_read_only_edit(index))
    }

    /// Open Import Tags for the active kit. `destination_rel` pre-fills the
    /// destination from a right-clicked folder.
    pub(in crate::app) fn open_tag_import_dialog(&mut self, destination_rel: Option<String>) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        let Some(target_game) = self.source().and_then(|source| source.game).map(|game| game.as_str().to_owned()) else {
            self.model.status = "Import Tags needs a loaded editing kit with a detected game".to_owned();
            return;
        };
        let Some(target_tags_root) = self.loaded_tags_root() else {
            self.model.status = "Import Tags needs a loaded tags folder".to_owned();
            return;
        };
        let sources = import_sources_for(&target_game);
        if sources.is_empty() {
            self.model.status = format!("Nothing converts into {target_game}");
            return;
        }
        let base = destination_rel
            .map(|rel| normalize_import_rel(&rel))
            .unwrap_or_default();
        self.import.tag_import_dialog = Some(TagImportDialog {
            kit: self.active_kit_id(),
            target_game,
            target_tags_root,
            source_input: String::new(),
            resolved_input: String::new(),
            resolving: false,
            facts: None,
            source_game: sources[0].to_owned(),
            source_game_note: "Choose a source, and Baboon will work out which game it is from"
                .to_owned(),
            destination_rel: base.clone(),
            destination_base: base,
            destination_touched: false,
            analyzing: false,
            write_when_analyzed: false,
            draft: None,
            pending_losses: Vec::new(),
            pending_refusal: None,
            written: None,
            draft_stamp: None,
            running: false,
            progress: None,
            report: None,
            error: None,
        });
    }

    pub(in crate::app) fn choose_import_source_file(&mut self, ctx: &egui::Context) {
        let start = self.import_picker_directory();
        let mut picker = rfd::FileDialog::new().set_title("Choose a tag to import");
        if let Some(start) = start {
            picker = picker.set_directory(start);
        }
        let Some(picked) = picker.pick_file() else {
            return;
        };
        self.set_import_source(picked, ctx);
    }

    pub(in crate::app) fn choose_import_source_folder(&mut self, ctx: &egui::Context) {
        let start = self.import_picker_directory();
        let mut picker = rfd::FileDialog::new().set_title("Choose a folder of tags to import");
        if let Some(start) = start {
            picker = picker.set_directory(start);
        }
        let Some(picked) = picker.pick_folder() else {
            return;
        };
        self.set_import_source(picked, ctx);
    }

    /// Where a picker should open.
    ///
    /// Whatever is already in the box wins, then the kit for the profile the
    /// dialog is currently set to import from. Only then anything else — and
    /// that last case is sorted, because `editing_kit_paths` is a `HashMap` and
    /// a picker that opens somewhere different each time is worse than one that
    /// opens somewhere merely unhelpful.
    fn import_picker_directory(&self) -> Option<PathBuf> {
        let dialog = self.import.tag_import_dialog.as_ref()?;
        let current = normalize_import_input(&dialog.source_input);
        if !current.is_empty() {
            let path = PathBuf::from(&current);
            let start = if path.is_dir() {
                Some(path)
            } else {
                path.parent().map(Path::to_path_buf)
            };
            if let Some(start) = start.filter(|path| path.is_dir()) {
                return Some(start);
            }
        }
        if let Some(root) = self
            .model.prefs
            .editing_kit_paths
            .get(&dialog.source_game)
            .map(|root| import_tags_root(root))
            .filter(|root| root.is_dir())
        {
            return Some(root);
        }
        let mut others = self
            .model.prefs
            .editing_kit_paths
            .iter()
            .filter(|(game, _)| *game != &dialog.target_game)
            .map(|(game, root)| (game.clone(), import_tags_root(root)))
            .filter(|(_, root)| root.is_dir())
            .collect::<Vec<_>>();
        others.sort();
        others.into_iter().next().map(|(_, root)| root)
    }

    fn set_import_source(&mut self, path: PathBuf, ctx: &egui::Context) {
        if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
            dialog.source_input = path.display().to_string();
            dialog.facts = None;
            dialog.invalidate_analysis();
        }
        self.resolve_import_source(ctx);
    }

    /// Work out what the source path is, on a worker.
    ///
    /// Off the UI thread because the path may be a whole kit's `tags` tree —
    /// counting what is in there means opening every file's header, and 97,000
    /// of those is not something to do between frames.
    pub(in crate::app) fn resolve_import_source(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.import.tag_import_dialog.as_ref() else {
            return;
        };
        if dialog.resolving {
            return;
        }
        let input = normalize_import_input(&dialog.source_input);
        if input.is_empty() {
            if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                dialog.facts = None;
                dialog.resolved_input = String::new();
                dialog.invalidate_analysis();
            }
            return;
        }
        let kit_roots = self
            .model.prefs
            .editing_kit_paths
            .iter()
            .map(|(game, root)| (game.clone(), root.clone()))
            .collect::<Vec<_>>();
        let definitions_root = locate_definitions_root();
        let names = self
            .source()
            .map(|source| source.names.clone())
            .unwrap_or_else(|| TagNameIndex::load_from_definitions(&definitions_root));
        if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
            dialog.resolving = true;
            dialog.error = None;
        }
        // A walk that panicked used to send nothing, leaving the dialog
        // "Checking what is in there..." and repainting every frame for good.
        let panic_input = input.clone();
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::ImportSourceResolved {
                result: resolve_import_source_job(&input, &kit_roots, &names),
                input,
            },
            move |error| WorkerMessage::ImportSourceResolved {
                input: panic_input,
                result: Err(format!("Checking the import source failed: {error}")),
            },
        );
    }

    pub(in crate::app) fn handle_import_source_resolved(
        &mut self,
        input: String,
        result: Result<ImportSourceFacts, String>,
    ) -> bool {
        let Some(dialog) = self.import.tag_import_dialog.as_mut() else {
            return false;
        };
        dialog.resolving = false;
        // The user may have kept typing while the walk ran. Landing stale facts
        // on a path they have since changed is worse than showing none.
        if normalize_import_input(&dialog.source_input) != input {
            return true;
        }
        dialog.resolved_input = input;
        dialog.invalidate_analysis();
        match result {
            Ok(facts) => {
                if let Some((game, note)) = facts.detected_game.clone() {
                    dialog.source_game = game;
                    dialog.source_game_note = note;
                } else {
                    dialog.source_game_note =
                        "Could not tell which game this is from — choose the source profile"
                            .to_owned();
                }
                // Land the source under the folder this was raised on, keeping
                // its own name — the answer the user would have typed. Skipped
                // once they have edited the destination themselves.
                if !dialog.destination_touched {
                    dialog.destination_rel =
                        seeded_destination(&dialog.destination_base, &facts.path, facts.is_folder);
                }
                dialog.facts = Some(facts);
            }
            Err(error) => {
                dialog.facts = None;
                dialog.error = Some(error);
            }
        }
        true
    }

    /// Convert a single-tag source, and write it when the caller asked to.
    ///
    /// Analysis and the write are one action because they are one intention:
    /// nothing between them needs a decision from the user, and making them
    /// press a button to find out what a button would do is a step that only
    /// exists because the code has two phases. The report still appears — after,
    /// beside the result, which is where the folder import already puts it.
    pub(in crate::app) fn analyze_tag_import(&mut self, write_when_done: bool) {
        let Some(dialog) = self.import.tag_import_dialog.as_ref() else {
            return;
        };
        if dialog.analyzing || dialog.running {
            return;
        }
        let Some(facts) = dialog.facts.as_ref().filter(|facts| !facts.is_folder) else {
            return;
        };
        let Some(group_tag) = facts.group_tag else {
            return;
        };
        if dialog.source_game == dialog.target_game {
            if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                dialog.error = Some(format!(
                    "This is already a {} tag. Copy the file into the kit instead — there is                      nothing to convert.",
                    dialog.target_game
                ));
            }
            return;
        }
        let path = facts.path.clone();
        let source_game = dialog.source_game.clone();
        let target_game = dialog.target_game.clone();
        let target_tags_root = dialog.target_tags_root.clone();
        let definitions_root = locate_definitions_root();
        // Hand the cache to the worker and take it back with the result. A move
        // rather than a share: an index memoises through a `RefCell`, so it is
        // `Send` but not `Sync`, and only one analysis runs at a time.
        let mut cache = self.import.native_template_cache.take().unwrap_or_default();
        let mut kit_roots: HashMap<String, PathBuf> = self
            .model.prefs
            .editing_kit_paths
            .iter()
            .map(|(game, root)| (game.clone(), import_tags_root(root)))
            .collect();
        // The destination kit is the one that is definitely open, whether or not
        // it is the one configured in Settings.
        kit_roots.insert(target_game.clone(), target_tags_root.clone());
        if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
            dialog.analyzing = true;
            dialog.write_when_analyzed = write_when_done;
            dialog.invalidate_analysis();
        }
        self.spawn_job(move || {
            let stamp = source_stamp(&path);
            let mut analyze = || -> Result<_, String> {
                let tag = crate::core::source::read_tag_at_path(
                    &path,
                    GameId::from_id(&source_game),
                    Some(&definitions_root),
                    group_tag,
                )
                .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
                match convert_tag_outcome(
                    &tag,
                    &source_game,
                    &target_game,
                    &definitions_root,
                    &kit_roots,
                    &mut cache,
                ) {
                    ConversionOutcome::Clean(draft) => Ok((draft, Vec::new(), None)),
                    ConversionOutcome::Lossy { draft, refusal } => {
                        let losses = draft.report.fail_closed_losses.clone();
                        Ok((*draft, losses, Some(refusal)))
                    }
                    ConversionOutcome::Failed(error) => Err(error),
                }
            };
            let result = analyze();
            WorkerMessage::ImportAnalysisFinished {
                result: result.map(|(draft, losses, refusal)| ImportAnalysis {
                    draft,
                    stamp,
                    losses,
                    refusal,
                }),
                templates: cache,
            }
        // The template cache went down with the job; it is only a cache.
        }, |_| WorkerMessage::ImportAnalysisFinished {
            result: Err("The conversion crashed while analyzing this tag".to_owned()),
            templates: NativeTemplateCache::default(),
        });
    }

    pub(in crate::app) fn handle_import_analysis_finished(
        &mut self,
        result: Result<ImportAnalysis, String>,
        templates: NativeTemplateCache,
        ctx: &egui::Context,
    ) -> bool {
        self.import.native_template_cache = Some(templates);
        let Some(dialog) = self.import.tag_import_dialog.as_mut() else {
            return false;
        };
        dialog.analyzing = false;
        let write = std::mem::take(&mut dialog.write_when_analyzed);
        let analysis = match result {
            Ok(analysis) => analysis,
            Err(error) => {
                dialog.draft = None;
                dialog.draft_stamp = None;
                dialog.error = Some(error);
                return true;
            }
        };
        let lossy = !analysis.losses.is_empty();
        dialog.draft = Some(analysis.draft);
        dialog.draft_stamp = analysis.stamp;
        dialog.pending_losses = analysis.losses;
        dialog.pending_refusal = analysis.refusal;
        dialog.error = None;
        // A tag that gives up audited data is never written on the strength of
        // the Import click alone. The click asked for the tag; it did not answer
        // a question the user had not been shown yet.
        if write && !lossy {
            self.write_single_tag_import(ctx);
        }
        true
    }

    /// Write the tag the user was shown the cost of, having accepted it.
    pub(in crate::app) fn accept_import_losses(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
            dialog.pending_losses.clear();
            dialog.pending_refusal = None;
        }
        self.write_single_tag_import(ctx);
    }

    /// Run the import.
    ///
    /// Neither half writes from here: a single tag goes off to be converted and
    /// is written when that lands, and a folder is converted and written by its
    /// own worker. Both report back through the receive loop, which is where the
    /// browser refresh gets its context.
    pub(in crate::app) fn begin_tag_import(&mut self) {
        let Some(dialog) = self.import.tag_import_dialog.as_ref() else {
            return;
        };
        if dialog.running || dialog.analyzing || dialog.resolving {
            return;
        }
        if dialog.source_is_folder() {
            self.begin_folder_tag_import();
        } else {
            // One action: convert, then write what came out. The analysis runs
            // on a worker and the write follows in its completion handler.
            self.analyze_tag_import(true);
        }
    }

    fn write_single_tag_import(&mut self, ctx: &egui::Context) {
        if self.refuse_read_only_tag_import() {
            return;
        }
        let Some(dialog) = self.import.tag_import_dialog.as_ref() else {
            return;
        };
        let Some(facts) = dialog.facts.as_ref() else {
            return;
        };
        if dialog.draft.is_none() {
            if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                dialog.error = Some("Analyze the conversion first".to_owned());
            }
            return;
        }
        // The preview was built from bytes on disk; those bytes are not ours and
        // may have moved on. Refuse rather than write from a stale reading.
        if source_stamp(&facts.path) != dialog.draft_stamp {
            if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                dialog.invalidate_analysis();
                dialog.error = Some(
                    "The source file changed after it was analyzed. Analyze the conversion again."
                        .to_owned(),
                );
            }
            return;
        }
        let Some(output) = dialog.single_output() else {
            if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                dialog.error = Some("Enter a destination path for the imported tag".to_owned());
            }
            return;
        };
        let target_tags_root = dialog.target_tags_root.clone();
        if !normalize_conversion_path(&output)
            .starts_with(normalize_conversion_path(&target_tags_root))
        {
            if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                dialog.error = Some("The destination escapes this kit's tags folder".to_owned());
            }
            return;
        }
        let dependency_schema = locate_definitions_root()
            .join(&dialog.target_game)
            .join("tag_dependency_list.json");
        let result = (|| {
            let dialog = self
                .import.tag_import_dialog
                .as_mut()
                .expect("import dialog checked above");
            let draft = dialog.draft.as_mut().expect("draft checked above");
            let companion_outputs =
                prepare_companion_outputs(draft, &output, &target_tags_root, &dependency_schema)?;
            for path in companion_outputs.iter().chain(std::iter::once(&output)) {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("Could not create {}: {error}", parent.display())
                    })?;
                }
            }
            for (companion, path) in draft.companion_tags.iter().zip(&companion_outputs) {
                companion
                    .tag
                    .write_atomic(path)
                    .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
            }
            draft
                .tag
                .write_atomic(&output)
                .map_err(|error| format!("Could not save {}: {error}", output.display()))?;
            Ok::<usize, String>(companion_outputs.len())
        })();
        match result {
            Ok(companion_count) => {
                let summary = if companion_count == 0 {
                    format!("Imported {}", output.display())
                } else {
                    format!(
                        "Imported {} and {companion_count} companion tag(s)",
                        output.display()
                    )
                };
                self.model.status = summary.clone();
                let kit = self.import.tag_import_dialog.as_ref().map(|dialog| dialog.kit);
                if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                    dialog.written = Some(summary);
                    dialog.error = None;
                }
                if let Some(kit) = kit {
                    self.refresh_after_import(kit, ctx);
                }
            }
            Err(error) => {
                if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                    dialog.error = Some(error);
                }
            }
        }
    }

    fn begin_folder_tag_import(&mut self) {
        self.run_folder_tag_import(false, None);
    }

    /// Write the tags a previous run held back, now that their loss is accepted.
    ///
    /// Restricted to exactly those files rather than re-running the folder: the
    /// rest are already written, and redoing them would be minutes of work to
    /// reach the same bytes.
    pub(in crate::app) fn accept_held_back_imports(&mut self) {
        let held = self
            .import.tag_import_dialog
            .as_ref()
            .and_then(|dialog| dialog.report.as_ref())
            .map(|report| {
                report
                    .held_back
                    .iter()
                    .map(|entry| entry.key.clone())
                    .collect::<HashSet<String>>()
            })
            .unwrap_or_default();
        if held.is_empty() {
            return;
        }
        self.run_folder_tag_import(true, Some(held));
    }

    fn run_folder_tag_import(&mut self, accept_loss: bool, only: Option<HashSet<String>>) {
        if self.refuse_read_only_tag_import() {
            return;
        }
        let Some(dialog) = self.import.tag_import_dialog.as_ref() else {
            return;
        };
        let Some(facts) = dialog.facts.as_ref().filter(|facts| facts.is_folder) else {
            return;
        };
        if dialog.source_game == dialog.target_game {
            if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                dialog.error = Some(format!(
                    "These are already {} tags. Copy the folder into the kit instead — there is \
                     nothing to convert.",
                    dialog.target_game
                ));
            }
            return;
        }
        let plan = match plan_folder_import(
            &facts.path,
            &dialog.target_tags_root,
            &dialog.destination_rel,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
                    dialog.error = Some(error);
                }
                return;
            }
        };
        let definitions_root = locate_definitions_root();
        let source = TagSource::LooseFolder {
            root: plan.source_root,
            game: GameId::from_id(&dialog.source_game),
            definitions_root: definitions_root.clone(),
        };
        let names = self
            .source()
            .map(|source| source.names.clone())
            .unwrap_or_else(|| TagNameIndex::load_from_definitions(&definitions_root));
        let job = FolderConversionJob {
            source,
            names,
            scope: FolderConversionScope::LooseSubtree {
                source_rel_path: plan.source_rel_path,
                destination_label: plan.destination_label,
                destination_parent: plan.destination_parent,
            },
            source_game: dialog.source_game.clone(),
            target_game: dialog.target_game.clone(),
            target_tags_root: dialog.target_tags_root.clone(),
            kit_roots: self
                .model.prefs
                .editing_kit_paths
                .iter()
                .map(|(game, root)| (game.clone(), import_tags_root(root)))
                .collect(),
            accept_loss,
            replace: ReplacePolicy::Always,
            only,
            // Nothing offers to cancel a loose-folder import: it converts one
            // folder out of a kit already sitting on the disk it writes to. The
            // cache import is the long one, and that has a button.
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let tx = self.tx.clone();
        if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
            dialog.running = true;
            dialog.progress = Some(FolderConversionProgress {
                phase: "Preparing".to_owned(),
                current: String::new(),
                processed: 0,
                total: 0,
                converted: 0,
                failed: 0,
            });
            dialog.report = None;
            dialog.error = None;
        }
        self.model.status = "Importing tags".to_owned();
        self.spawn_job(
            move || WorkerMessage::FolderConversionFinished(run_folder_conversion_job(job, &tx)),
            |_| WorkerMessage::FolderConversionFinished(Err("The tag import worker crashed".to_owned())),
        );
    }

    pub(in crate::app) fn handle_folder_conversion_progress(
        &mut self,
        progress: FolderConversionProgress,
    ) -> bool {
        self.model.status = format!("Importing tags: {}", progress.phase);
        if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
            dialog.progress = Some(progress);
        }
        false
    }

    pub(in crate::app) fn handle_folder_conversion_finished(
        &mut self,
        result: Result<FolderConversionReport, String>,
        ctx: &egui::Context,
    ) -> bool {
        let mut imported = false;
        if let Some(dialog) = self.import.tag_import_dialog.as_mut() {
            dialog.running = false;
            dialog.progress = None;
            match result {
                Ok(report) => {
                    self.model.status = format!(
                        "Imported {} tag(s); {} failed",
                        report.converted_count(),
                        report.failed_count()
                    );
                    imported = report.converted_count() > 0;
                    dialog.report = Some(report);
                    dialog.error = None;
                }
                Err(error) => {
                    self.model.status = error.clone();
                    dialog.error = Some(error);
                }
            }
        }
        if imported {
            if let Some(kit) = self.import.tag_import_dialog.as_ref().map(|dialog| dialog.kit) {
                self.refresh_after_import(kit, ctx);
            }
        }
        true
    }

    /// Bring the imported tags into the browser.
    ///
    /// Returns to the kit the import was started from first: the tags landed in
    /// that kit's tree, and refreshing whichever workspace happens to be focused
    /// would rescan the wrong one and leave the right one stale.
    pub(in crate::app) fn refresh_after_import(&mut self, kit: KitId, ctx: &egui::Context) {
        if !self.focus_navigation_kit(kit) {
            return;
        }
        if self.can_import_tags() && !self.model.kits[self.model.active].scanning_entries {
            self.refresh_tag_browser(ctx.clone());
        }
    }
}

/// A kit path may be configured as the kit root or as its `tags` folder.
pub(in crate::app) fn import_tags_root(path: &Path) -> PathBuf {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("tags"))
    {
        path.to_path_buf()
    } else {
        path.join("tags")
    }
}

fn source_stamp(path: &Path) -> Option<SourceStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()))
}

/// Measure what a source path holds, and work out which game it came from.
fn resolve_import_source_job(
    input: &str,
    kit_roots: &[(String, PathBuf)],
    names: &TagNameIndex,
) -> Result<ImportSourceFacts, String> {
    let path = PathBuf::from(input);
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;

    if metadata.is_file() {
        let parent = path.parent().unwrap_or(Path::new(""));
        let entry = loose_file_entry(parent, &path, names)
            .map_err(|error| format!("Could not identify {}: {error}", path.display()))?
            .ok_or_else(|| {
                format!(
                    "{} is not a tag file — its header carries no tag signature",
                    path.display()
                )
            })?;
        let detected = detect_import_game(&path, kit_roots, Some(&path));
        return Ok(ImportSourceFacts {
            path,
            is_folder: false,
            tag_files: 1,
            skipped_files: 0,
            group_tag: Some(entry.group_tag),
            detected_game: detected,
        });
    }

    if !metadata.is_dir() {
        return Err(format!("{} is neither a file nor a folder", path.display()));
    }

    let mut tag_files = 0usize;
    let mut skipped_files = 0usize;
    let mut sample = None;
    for file in blam_tags::convert::walk_files(&path) {
        match loose_file_entry(&path, &file, names) {
            Ok(Some(_)) => {
                tag_files += 1;
                if sample.is_none() {
                    sample = Some(file);
                }
            }
            // An unreadable file counts as skipped rather than failing the whole
            // resolve: one locked file should not stop the user seeing that a
            // folder holds four thousand tags.
            Ok(None) | Err(_) => skipped_files += 1,
        }
    }
    let detected = detect_import_game(&path, kit_roots, sample.as_deref());
    Ok(ImportSourceFacts {
        path,
        is_folder: true,
        tag_files,
        skipped_files,
        group_tag: None,
        detected_game: detected,
    })
}

/// Decide which game a source was authored for, preferring the answer that
/// cannot be wrong.
///
/// Where the file *is* beats what it contains: a path inside a configured
/// editing kit names the game outright, whereas a layout comparison can only say
/// which profiles a tag is *compatible* with, and simple groups are compatible
/// with several. Content is the fallback for a tag sitting outside any kit.
fn detect_import_game(
    path: &Path,
    kit_roots: &[(String, PathBuf)],
    sample: Option<&Path>,
) -> Option<(String, String)> {
    let normalized = normalize_conversion_path(path);
    let mut best: Option<(usize, &str)> = None;
    for (game, root) in kit_roots {
        let root = normalize_conversion_path(root);
        if normalized.starts_with(&root) {
            let depth = root.components().count();
            if best.is_none_or(|(seen, _)| depth > seen) {
                best = Some((depth, game.as_str()));
            }
        }
    }
    if let Some((_, game)) = best {
        if CONVERSION_PROFILES.contains(&game) {
            return Some((
                game.to_owned(),
                format!("Inside the configured {game} editing kit"),
            ));
        }
    }

    let sample = sample?;
    let bytes = fs::read(sample).ok()?;

    // A classic container names its engine in the header, which is a fact rather
    // than an inference — and the only signal available, since a classic tag
    // carries no embedded layout to compare against anything.
    if let Some((_, engine)) = ClassicHeader::parse(&bytes) {
        let game = match engine {
            ClassicEngine::HaloCe => GameId::HaloCe,
            _ => GameId::Halo2,
        }
        .as_str();
        return Some((
            game.to_owned(),
            format!("The tag's classic header says {engine:?}"),
        ));
    }

    let imported = TagFile::read_from_bytes(&bytes).ok()?;
    let group_tag = imported.header.group_tag;
    let matches = CONVERSION_PROFILES
        .iter()
        .filter(|profile| {
            crate::app::import::single_tag::profile_fit(profile, group_tag, &imported)
                .is_some_and(|fit| fit.is_identical())
        })
        .copied()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => None,
        [only] => Some((
            (*only).to_owned(),
            format!("Its layout matches {only} and no other profile"),
        )),
        several => Some((
            several[0].to_owned(),
            format!(
                "Its layout fits {} equally well ({}) — confirm which one it is",
                several.len(),
                several.join(", ")
            ),
        )),
    }
}

#[cfg(test)]
mod import_timing_tests;

#[cfg(test)]
mod tests;
