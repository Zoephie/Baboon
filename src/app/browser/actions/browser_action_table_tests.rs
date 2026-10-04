//! Every `BrowserAction` dispatched through `handle_browser_action` on a
//! synthetic loose Halo 3 kit, one case per variant, each asserting the state
//! its arm leaves behind.
//!
//! Many arms end in a native file or folder dialog, which a test cannot
//! answer. For those the case pins everything before the dialog: the guard
//! that refuses, with its message, or -- where the guard returns silently --
//! that nothing at all changed. What runs after a dialog is covered by the
//! tests of the jobs themselves.
//!
//! `variant_index` matches every variant without a wildcard, so adding one
//! fails to compile here until it is given an index, and the table test then
//! fails until it is given a case.

use crate::app::loose_fixture::*;
use super::*;
use crate::app::shell::FolderRefactorUiState;
use crate::app::tag_ops::{DeleteConfirm, DeleteKind, NewTagDialog};
use crate::app::import::{CacheImportDialog, TagImportDialog};
use crate::app::search::QueryResultsWindow;
use std::collections::BTreeSet;
use std::time::Duration;

const MODEL: &str = "objects/props/crate.model";
const RENDER: &str = "objects/props/crate.render_model";
const BARREL: &str = "objects/props/barrel.model";
const FOLDER: &str = "objects/props";
const UNKNOWN: &str = "file:/no/such/tag.model";

/// How many variants `BrowserAction` has. Bump it with `variant_index`.
const VARIANTS: usize = 49;

fn variant_index(action: &BrowserAction) -> usize {
    use BrowserAction as A;
    match action {
        A::OpenFolderBrowser { .. } => 0,
        A::ToggleFolderFavorite(_) => 1,
        A::Select(_) => 2,
        A::ToggleFavorite(_) => 3,
        A::CopyTagName(_) => 4,
        A::CopyFolderPath(_) => 5,
        A::DumpJson(_) => 6,
        A::OpenInExplorer(_) => 7,
        A::DumpLoadedFolderJson(_) => 8,
        A::DumpLooseFolderJson { .. } => 9,
        A::RenameLooseFolder { .. } => 10,
        A::MoveLooseFolder { .. } => 11,
        A::CopyLooseFolder { .. } => 12,
        A::ImportTagsIntoLooseFolder { .. } => 13,
        A::OpenLooseFolderInExplorer { .. } => 14,
        A::ImportCacheFolderIntoKit { .. } => 15,
        A::ImportCacheTagIntoKit { .. } => 16,
        A::ExtractRaw(_) => 17,
        A::ExtractBitmap(_) => 18,
        A::ExtractBitmapFolder(_) => 19,
        A::ExtractBitmapSource(_) => 20,
        A::ExtractBitmapSourceFolder(_) => 21,
        A::ExtractSound { .. } => 22,
        A::LoadFolderExtractables { .. } => 23,
        A::ExtractGeometry(_) => 24,
        A::ExtractImportInfo(_) => 25,
        A::ExtractAnimation(_) => 26,
        A::ExtractMaterialShaderSources(_) => 27,
        A::ExtractMaterialShaderSourceFolder(_) => 28,
        A::ExtractHlslIncludeSource(_) => 29,
        A::ExtractHlslIncludeFolder(_) => 30,
        A::ReimportGeometry(_) => 31,
        A::ExtractContainerFolderTags { .. } => 32,
        A::ExtractScenarioScripts(_) => 33,
        A::ImportScenarioScripts(_) => 34,
        A::FindReferences(_) => 35,
        A::ExploreReferences(_) => 36,
        A::DumpReferences(_) => 37,
        A::LaunchScenarioInSapien(_) => 38,
        A::LaunchScenarioInTagTest(_) => 39,
        A::RenameTag(_) => 40,
        A::DuplicateTag(_) => 41,
        A::DeleteTag(_) => 42,
        A::MoveTag(_) => 43,
        A::ImportTagInFolder { .. } => 44,
        A::NewTagInFolder { .. } => 45,
        A::NewContainerFolder { .. } => 46,
        A::RenameContainerFolder { .. } => 47,
        A::DeleteContainerFolder { .. } => 48,
    }
}

/// An H3 kit: a model referencing its render model, and a second model.
fn fixture() -> LooseKit {
    let kit = LooseKit::new("browser-actions", "halo3_mcc");
    let mode = group_tag("halo3_mcc", "render_model");
    kit.write_mcc("objects/props/crate", "render_model", |_| {});
    kit.write_mcc("objects/props/crate", "model", |tag| {
        set_reference(tag, "render model", mode, "objects\\props\\crate");
    });
    kit.write_mcc("objects/props/barrel", "model", |_| {});
    kit
}

/// What one dispatch left behind beyond the app itself.
struct Outcome {
    copied_text: String,
    /// Whether a worker answered within a moment of the dispatch.
    worker_answered: bool,
}

type Setup = fn(&mut Baboon, &LooseKit);
type Check = fn(&Baboon, &LooseKit, &Outcome) -> Result<(), String>;

struct Case {
    action: fn(&LooseKit) -> BrowserAction,
    setup: Setup,
    check: Check,
}

fn no_setup(_: &mut Baboon, _: &LooseKit) {}

/// Leave the model open with an unsaved edit.
fn dirty_model(app: &mut Baboon, kit: &LooseKit) {
    let key = kit.open(app, MODEL);
    edit_field(app, &key, "disappear distance", "3");
}

fn ensure(condition: bool, what: impl Into<String>) -> Result<(), String> {
    condition.then_some(()).ok_or_else(|| what.into())
}

fn status_is(app: &Baboon, expected: &str) -> Result<(), String> {
    ensure(
        app.model.status == expected,
        format!("status {:?}, expected {expected:?}", app.model.status),
    )
}

/// Nothing visible happened: the status is untouched, no worker was
/// started, and no dialog or prompt opened. For arms whose guard returns
/// silently ahead of a native dialog.
fn nothing_happened(app: &Baboon, _: &LooseKit, outcome: &Outcome) -> Result<(), String> {
    status_is(app, "Ready")?;
    ensure(!outcome.worker_answered, "a worker was started")?;
    ensure(outcome.copied_text.is_empty(), "something was copied")?;
    ensure(
        app.dialogs.get::<RenameTagState>().is_none()
            && app.dialogs.get::<DeleteConfirm>().is_none()
            && app.dialogs.get::<ExtractTargetPrompt>().is_none()
            && app.dialogs.get::<QueryResultsWindow>().is_none()
            && app.dialogs.get::<ContentExplorer>().is_none()
            && app.tag_ops.folder_refactor.is_none()
            && app.dialogs.get::<LooseFolderRenameState>().is_none()
            && app.dialogs.get::<TagImportDialog>().is_none()
            && app.dialogs.get::<CacheImportDialog>().is_none()
            && app.dialogs.get::<ContainerFolderDialog>().is_none()
            && app.kit_tools.pending_tool_import.is_none()
            && app.export.pending_sound_extract.is_none(),
        "a dialog opened",
    )
}

fn cases() -> Vec<Case> {
    use BrowserAction as A;
    vec![
        // 0
        Case {
            action: |_| A::OpenFolderBrowser {
                rel_path: PathBuf::from(FOLDER),
                label: "props".to_owned(),
                open_in_new_tab: false,
            },
            setup: |app, kit| {
                kit.open(app, BARREL);
            },
            check: |app, kit, _| {
                let pane = folder_pane_key(Path::new(FOLDER));
                let state = app.views[app.model.kits[0].id].browser.folder_browsers.get(&pane).ok_or("no pane")?;
                ensure(state.label == "props", "label")?;
                ensure(state.rel_path == Path::new(FOLDER), "rel path")?;
                ensure(app.model.kits[0].open_tabs.contains(&pane), "not a tab")?;
                ensure(
                    app.model.kits[0].selected_key.as_deref() == Some(kit.key(BARREL).as_str()),
                    "a folder pane must not take the tag selection",
                )
            },
        },
        // 1
        Case {
            action: |_| A::ToggleFolderFavorite(PathBuf::from(FOLDER)),
            setup: no_setup,
            check: |app, kit, _| {
                status_is(app, "Added objects/props to Favorites")?;
                let favorites = &app.model.prefs.editing_kit_favorites;
                ensure(favorites.len() == 1, "one kit's favorites")?;
                ensure(same_recent_path(&favorites[0].tags_root, &kit.root), "tags root")?;
                ensure(favorites[0].folders == vec![PathBuf::from(FOLDER)], "folders")?;
                ensure(
                    app.model.kits[0].active_favorite_folders == vec![PathBuf::from(FOLDER)],
                    "kit favorites",
                )
            },
        },
        // 2
        Case {
            action: |kit| A::Select(kit.key(MODEL)),
            setup: no_setup,
            check: |app, kit, outcome| {
                let key = kit.key(MODEL);
                ensure(app.model.kits[0].selected_key.as_deref() == Some(key.as_str()), "selected")?;
                ensure(app.model.kits[0].open_tabs.contains(&key), "opened as a tab")?;
                ensure(outcome.worker_answered, "a load was started")?;
                status_is(app, &format!("Loading {MODEL}"))
            },
        },
        // 3
        Case {
            action: |kit| A::ToggleFavorite(kit.key(MODEL)),
            setup: no_setup,
            check: |app, kit, _| {
                status_is(app, &format!("Added {MODEL} to Favorites"))?;
                ensure(
                    app.model.prefs.editing_kit_favorites[0].tags == vec![PathBuf::from(MODEL)],
                    "favorite tags",
                )?;
                ensure(
                    app.model.kits[0].active_favorite_entries.len() == 1
                        && app.model.kits[0].active_favorite_entries[0].key == kit.key(MODEL),
                    "kit favorite entries",
                )
            },
        },
        // 4
        Case {
            action: |kit| A::CopyTagName(kit.key(MODEL)),
            setup: no_setup,
            check: |app, _, outcome| {
                let native = crate::core::format::to_native_path_string(MODEL);
                ensure(outcome.copied_text == native, format!("copied {:?}", outcome.copied_text))?;
                status_is(app, &format!("Copied {native}"))
            },
        },
        // 5
        Case {
            action: |_| A::CopyFolderPath(PathBuf::from(FOLDER)),
            setup: no_setup,
            check: |app, _, outcome| {
                let native = crate::core::format::to_native_path_string(FOLDER);
                ensure(outcome.copied_text == native, format!("copied {:?}", outcome.copied_text))?;
                status_is(app, &format!("Copied {native}"))
            },
        },
        // 6: a known key reaches the save dialog.
        Case {
            action: |_| A::DumpJson(UNKNOWN.to_owned()),
            setup: no_setup,
            check: nothing_happened,
        },
        // 7: elsewhere than Windows, the arm says it cannot; on Windows a
        // known key would launch Explorer, so the unknown-key guard is used.
        Case {
            action: |kit| {
                if cfg!(windows) {
                    A::OpenInExplorer(UNKNOWN.to_owned())
                } else {
                    A::OpenInExplorer(kit.key(MODEL))
                }
            },
            setup: no_setup,
            check: |app, _, _| {
                if cfg!(windows) {
                    status_is(app, "Tag is no longer in the browser")
                } else {
                    status_is(app, "Open with File Explorer is only available on Windows")
                }
            },
        },
        // 8
        Case {
            action: |_| A::DumpLoadedFolderJson(vec![UNKNOWN.to_owned()]),
            setup: no_setup,
            check: |app, _, _| status_is(app, "No loaded tags found in folder"),
        },
        // 9: a loose kit reaches the folder dialog; without a source the arm
        // returns before it.
        Case {
            action: |_| A::DumpLooseFolderJson {
                rel_path: PathBuf::from(FOLDER),
                label: "props".to_owned(),
            },
            setup: |app, _| app.model.kits[0].source = None,
            check: nothing_happened,
        },
        // 10
        Case {
            action: |_| A::RenameLooseFolder {
                rel_path: PathBuf::from(FOLDER),
                label: "props".to_owned(),
            },
            setup: |app, kit| {
                app.model.kits[0].source.as_mut().unwrap().reverse_dependencies = Some(kit.index());
            },
            check: |app, _, _| {
                let state = app
                    .dialogs
                    .get::<LooseFolderRenameState>()
                    .ok_or("no dialog")?;
                ensure(state.kit == app.model.kits[0].id, "kit")?;
                ensure(state.rel_path == Path::new(FOLDER), "rel path")?;
                ensure(state.old_name == "props" && state.name_input == "props", "name")?;
                ensure(state.parent_display == "objects", "parent")?;
                ensure(state.tag_count == 3, format!("{} tags", state.tag_count))?;
                ensure(
                    state.outside_referrers.as_deref() == Some(&[][..]),
                    "the only referrer is inside the folder",
                )
            },
        },
        // 11
        Case {
            action: |_| A::MoveLooseFolder {
                rel_path: PathBuf::from(FOLDER),
                label: "props".to_owned(),
            },
            setup: dirty_model,
            check: |app, _, _| status_is(app, "Save or close dirty tags before moving/copying folders"),
        },
        // 12
        Case {
            action: |_| A::CopyLooseFolder {
                rel_path: PathBuf::from(FOLDER),
                label: "props".to_owned(),
            },
            setup: |app, _| {
                app.tag_ops.folder_refactor = Some(FolderRefactorUiState {
                    label: "Moving".to_owned(),
                    phase: "Preparing".to_owned(),
                    progress: None,
                })
            },
            check: |app, _, _| status_is(app, "A folder move/copy is already running"),
        },
        // 13
        Case {
            action: |_| A::ImportTagsIntoLooseFolder {
                rel_path: PathBuf::from(FOLDER),
            },
            setup: no_setup,
            check: |app, kit, _| {
                let dialog = app.dialogs.get::<TagImportDialog>().ok_or("no dialog")?;
                ensure(dialog.target_game == "halo3_mcc", "target game")?;
                ensure(dialog.target_tags_root == kit.root, "tags root")?;
                ensure(dialog.destination_rel == FOLDER, format!("{:?}", dialog.destination_rel))?;
                ensure(dialog.destination_base == FOLDER, "base")?;
                ensure(!dialog.source_game.is_empty(), "a source game is offered")
            },
        },
        // 14: an existing folder would be opened in the file manager.
        Case {
            action: |_| A::OpenLooseFolderInExplorer {
                rel_path: PathBuf::from("objects/missing"),
            },
            setup: no_setup,
            check: |app, kit, _| {
                status_is(
                    app,
                    &format!(
                        "Tag folder not found: {}",
                        loose_folder_explorer_path(&kit.root, Path::new("objects/missing"))
                            .display()
                    ),
                )
            },
        },
        // 15
        Case {
            action: |_| A::ImportCacheFolderIntoKit {
                prefix: "objects".to_owned(),
            },
            setup: no_setup,
            check: |app, _, _| status_is(app, "This is not a monolithic cache workspace"),
        },
        // 16
        Case {
            action: |kit| A::ImportCacheTagIntoKit { key: kit.key(MODEL) },
            setup: no_setup,
            check: |app, _, _| status_is(app, "This is not a monolithic cache workspace"),
        },
        // 17: a known key reaches the save dialog.
        Case {
            action: |_| A::ExtractRaw(UNKNOWN.to_owned()),
            setup: no_setup,
            check: nothing_happened,
        },
        // 18: a known key reaches the folder dialog.
        Case {
            action: |_| A::ExtractBitmap(UNKNOWN.to_owned()),
            setup: no_setup,
            check: nothing_happened,
        },
        // 19
        Case {
            action: |_| A::ExtractBitmapFolder(vec![UNKNOWN.to_owned()]),
            setup: no_setup,
            check: |app, _, _| status_is(app, "No bitmap tags found in folder"),
        },
        // 20
        Case {
            action: |_| A::ExtractBitmapSource(UNKNOWN.to_owned()),
            setup: no_setup,
            check: |app, _, _| status_is(app, "No bitmap tags found"),
        },
        // 21
        Case {
            action: |_| A::ExtractBitmapSourceFolder(Vec::new()),
            setup: no_setup,
            check: |app, _, _| status_is(app, "No bitmap tags found"),
        },
        // 22
        Case {
            action: |kit| A::ExtractSound {
                keys: vec![kit.key(MODEL)],
                all_languages: true,
            },
            setup: no_setup,
            check: |app, _, _| {
                status_is(app, "No loaded sound tags found")?;
                ensure(app.export.pending_sound_extract.is_none(), "an extraction was queued")
            },
        },
        // 23: the whole scan is in memory, so the folder loads at once.
        Case {
            action: |_| A::LoadFolderExtractables {
                rel_path: PathBuf::from(FOLDER),
                label: "props".to_owned(),
            },
            setup: |app, _| app.model.kits[0].source.as_mut().unwrap().entries.clear(),
            check: |app, _, outcome| {
                status_is(app, "Loaded the entire props folder for extraction")?;
                ensure(!outcome.worker_answered, "no scan was needed")?;
                let entries = &app.model.kits[0].source.as_ref().unwrap().entries;
                ensure(entries.len() == 3, format!("{} entries loaded", entries.len()))
            },
        },
        // 24
        Case {
            action: |kit| A::ExtractGeometry(kit.key(MODEL)),
            setup: no_setup,
            check: |app, kit, _| {
                let prompt = app
                    .dialogs
                    .get::<ExtractTargetPrompt>()
                    .ok_or("no prompt")?;
                ensure(prompt.key == kit.key(MODEL), "key")?;
                ensure(prompt.display_path == MODEL, "display path")?;
                ensure(matches!(prompt.kind, ExtractKind::Geometry), "kind")?;
                ensure(
                    prompt.source == blam_tags::game::Game::Halo3
                        && prompt.target == blam_tags::game::Game::Halo3,
                    "the kit's own game, preselected",
                )
            },
        },
        // 25: a known key reaches the folder dialog.
        Case {
            action: |_| A::ExtractImportInfo(UNKNOWN.to_owned()),
            setup: no_setup,
            check: nothing_happened,
        },
        // 26
        Case {
            action: |kit| A::ExtractAnimation(kit.key(MODEL)),
            setup: no_setup,
            check: |app, _, _| {
                let prompt = app
                    .dialogs
                    .get::<ExtractTargetPrompt>()
                    .ok_or("no prompt")?;
                ensure(matches!(prompt.kind, ExtractKind::Animation), "kind")
            },
        },
        // 27: a known key reaches the folder dialog.
        Case {
            action: |_| A::ExtractMaterialShaderSources(UNKNOWN.to_owned()),
            setup: no_setup,
            check: nothing_happened,
        },
        // 28
        Case {
            action: |_| A::ExtractMaterialShaderSourceFolder(vec![UNKNOWN.to_owned()]),
            setup: no_setup,
            check: |app, _, _| status_is(app, "No material shaders found in folder"),
        },
        // 29: a known key reaches the folder dialog.
        Case {
            action: |_| A::ExtractHlslIncludeSource(UNKNOWN.to_owned()),
            setup: no_setup,
            check: nothing_happened,
        },
        // 30
        Case {
            action: |_| A::ExtractHlslIncludeFolder(vec![UNKNOWN.to_owned()]),
            setup: no_setup,
            check: |app, _, _| status_is(app, "No HLSL includes found in folder"),
        },
        // 31
        Case {
            action: |kit| A::ReimportGeometry(kit.key(RENDER)),
            setup: no_setup,
            check: |app, _, _| {
                let request = app.kit_tools.pending_tool_import.as_ref().ok_or_else(|| {
                    format!("no tool import queued; status {:?}", app.model.status)
                })?;
                ensure(request.verb == "render", format!("verb {:?}", request.verb))?;
                ensure(
                    request.source_dir == "objects\\props\\crate"
                        || request.source_dir == "objects/props/crate",
                    format!("source dir {:?}", request.source_dir),
                )
            },
        },
        // 32
        Case {
            action: |kit| A::ExtractContainerFolderTags {
                label: "props".to_owned(),
                keys: vec![kit.key(MODEL)],
            },
            setup: no_setup,
            check: |app, _, _| status_is(app, "Extracting tags needs a Campaign Evolved container"),
        },
        // 33
        Case {
            action: |kit| A::ExtractScenarioScripts(kit.key(MODEL)),
            setup: no_setup,
            check: |app, _, _| {
                status_is(app, "Script extraction is only available for Campaign Evolved")
            },
        },
        // 34
        Case {
            action: |kit| A::ImportScenarioScripts(kit.key(MODEL)),
            setup: no_setup,
            check: |app, _, _| status_is(app, "Script import is only available for Campaign Evolved"),
        },
        // 35
        Case {
            action: |kit| A::FindReferences(kit.key(RENDER)),
            setup: |app, kit| {
                app.model.kits[0].source.as_mut().unwrap().reverse_dependencies = Some(kit.index());
            },
            check: |app, kit, _| {
                let results = &app.dialogs.get::<QueryResultsWindow>().ok_or("no results")?.results;
                ensure(results.title == format!("References to {RENDER}"), results.title.clone())?;
                ensure(
                    results.entries.iter().map(|entry| entry.key.clone()).collect::<Vec<_>>()
                        == vec![kit.key(MODEL)],
                    "the model references it",
                )?;
                ensure(results.note.is_none(), "no note")?;
                ensure(
                    results.ref_target
                        == Some((group_tag("halo3_mcc", "render_model"), "objects\\props\\crate".to_owned())),
                    format!("ref target {:?}", results.ref_target),
                )
            },
        },
        // 36: without an index, both directions are unavailable.
        Case {
            action: |kit| A::ExploreReferences(kit.key(MODEL)),
            setup: no_setup,
            check: |app, kit, _| {
                let explorer = app.dialogs.get::<ContentExplorer>().ok_or("no explorer")?;
                ensure(explorer.focus.key == kit.key(MODEL), "focus")?;
                ensure(explorer.parents.is_empty() && explorer.children.is_empty(), "empty")?;
                ensure(explorer.index_unavailable, "index unavailable")
            },
        },
        // 37: with an index, the report goes to a save dialog.
        Case {
            action: |kit| A::DumpReferences(kit.key(MODEL)),
            setup: no_setup,
            check: |app, _, _| {
                status_is(
                    app,
                    "Build the reference index first — Tools ▸ Build/Rebuild Reference Index",
                )
            },
        },
        // 38
        Case {
            action: |kit| A::LaunchScenarioInSapien(kit.key(MODEL)),
            setup: no_setup,
            check: |app, _, _| status_is(app, "Only scenario tags can be launched"),
        },
        // 39
        Case {
            action: |kit| A::LaunchScenarioInTagTest(kit.key(MODEL)),
            setup: no_setup,
            check: |app, _, _| status_is(app, "Only scenario tags can be launched"),
        },
        // 40
        Case {
            action: |kit| A::RenameTag(kit.key(RENDER)),
            setup: |app, kit| {
                app.model.kits[0].source.as_mut().unwrap().reverse_dependencies = Some(kit.index());
            },
            check: |app, kit, _| {
                let state = app.dialogs.get::<RenameTagState>().ok_or("no dialog")?;
                ensure(state.key == kit.key(RENDER), "key")?;
                ensure(state.operation == TagNameOperation::Rename, "operation")?;
                ensure(state.old_display == RENDER, "old display")?;
                ensure(state.extension == "render_model", "extension")?;
                ensure(state.fixed_parent == FOLDER, format!("parent {:?}", state.fixed_parent))?;
                ensure(state.new_path_input == "crate", format!("input {:?}", state.new_path_input))?;
                ensure(state.referrers == vec![MODEL.to_owned()], format!("{:?}", state.referrers))?;
                ensure(!state.referrers_unavailable, "index available")?;
                ensure(!state.is_container && !state.whole_path_editable, "loose")
            },
        },
        // 41
        Case {
            action: |kit| A::DuplicateTag(kit.key(MODEL)),
            setup: no_setup,
            check: |app, _, _| {
                let state = app.dialogs.get::<RenameTagState>().ok_or("no dialog")?;
                ensure(state.operation == TagNameOperation::Duplicate, "operation")?;
                ensure(state.focus_input, "the name field takes focus")?;
                ensure(state.referrers_unavailable, "no index")
            },
        },
        // 42
        Case {
            action: |kit| A::DeleteTag(kit.key(MODEL)),
            setup: dirty_model,
            check: |app, kit, _| {
                let confirm = app
                    .dialogs
                    .get::<DeleteConfirm>()
                    .ok_or("no confirmation")?;
                ensure(confirm.key == kit.key(MODEL), "key")?;
                ensure(confirm.display_path == MODEL, "display path")?;
                ensure(matches!(confirm.kind, DeleteKind::Loose), "loose")?;
                ensure(confirm.has_unsaved_edits, "unsaved edits flagged")?;
                ensure(confirm.referrers_unavailable && confirm.referrers.is_empty(), "no index")
            },
        },
        // 43
        Case {
            action: |kit| A::MoveTag(kit.key(BARREL)),
            setup: dirty_model,
            check: |app, _, _| status_is(app, "Save or close dirty tags before moving"),
        },
        // 44
        Case {
            action: |_| A::ImportTagInFolder {
                folder_rel: Some(FOLDER.to_owned()),
            },
            setup: no_setup,
            check: |app, _, _| status_is(app, "Import tag is only for Campaign Evolved containers"),
        },
        // 45
        Case {
            action: |_| A::NewTagInFolder {
                folder_rel: Some(format!("{FOLDER}/")),
            },
            setup: no_setup,
            check: |app, _, _| {
                let Some(dialog) = app.dialogs.get::<NewTagDialog>() else {
                    return ensure(false, "the dialog opened");
                };
                ensure(
                    dialog.rel_path == format!("{FOLDER}/"),
                    format!("path {:?}", dialog.rel_path),
                )
            },
        },
        // 46: offered only on container kits; the arm itself does not check.
        Case {
            action: |_| A::NewContainerFolder {
                parent_rel: Some("objects\\props".to_owned()),
            },
            setup: no_setup,
            check: |app, _, _| {
                let dialog = app
                    .dialogs
                    .get::<ContainerFolderDialog>()
                    .ok_or("no dialog")?;
                ensure(dialog.kit == app.model.kits[0].id, "kit")?;
                ensure(
                    dialog.parent_rel.as_deref() == Some(FOLDER),
                    format!("parent {:?}", dialog.parent_rel),
                )?;
                ensure(dialog.renaming.is_none() && dialog.name_input.is_empty(), "new")
            },
        },
        // 47
        Case {
            action: |_| A::RenameContainerFolder {
                rel: "objects/props".to_owned(),
            },
            setup: no_setup,
            check: |app, _, _| {
                let dialog = app
                    .dialogs
                    .get::<ContainerFolderDialog>()
                    .ok_or("no dialog")?;
                ensure(dialog.parent_rel.as_deref() == Some("objects"), "parent")?;
                ensure(dialog.renaming.as_deref() == Some(FOLDER), "renaming")?;
                ensure(dialog.name_input == "props", "prefilled leaf")
            },
        },
        // 48: on a loose kit there is no pending folder to remove, and the
        // arm reports a removal anyway (QUIRK; the menu never offers it here).
        Case {
            action: |_| A::DeleteContainerFolder {
                rel: "objects/props".to_owned(),
            },
            setup: no_setup,
            check: |app, kit, _| {
                status_is(app, "Removed folder objects/props")?;
                ensure(app.model.kits[0].pending_container_folders.is_empty(), "nothing pending")?;
                ensure(kit.root.join(FOLDER).is_dir(), "the folder on disk is untouched")
            },
        },
    ]
}

/// Run one case on a fresh app over the shared kit.
fn run(case: &Case, kit: &LooseKit) -> Result<(), String> {
    let mut app = Baboon::for_test();
    kit.install(&mut app);
    (case.setup)(&mut app, kit);
    // Whatever the setup left in flight is settled before the dispatch, so
    // a worker answering afterwards is the arm's own.
    drain_messages(&mut app, Duration::from_millis(50));
    app.model.status = "Ready".to_owned();
    let mut action = Some((case.action)(kit));
    let ctx = egui::Context::default();
    let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
        if let Some(action) = action.take() {
            app.handle_browser_action(action, ui.ctx().clone())
        }
    });
    let worker_answered = app.rx.recv_timeout(Duration::from_millis(150)).is_ok();
    let outcome = Outcome {
        copied_text: crate::app::copied_text(&output.platform_output),
        worker_answered,
    };
    (case.check)(&app, kit, &outcome)
}

#[test]
fn every_browser_action_variant_has_a_case() {
    let kit = fixture();
    let covered: BTreeSet<usize> = cases()
        .iter()
        .map(|case| variant_index(&(case.action)(&kit)))
        .collect();
    let missing: Vec<usize> = (0..VARIANTS).filter(|index| !covered.contains(index)).collect();
    assert!(missing.is_empty(), "variants without a case: {missing:?}");
    assert_eq!(cases().len(), VARIANTS, "one case per variant");
}

#[test]
fn every_browser_action_leaves_its_state() {
    let kit = fixture();
    let failures: Vec<String> = cases()
        .iter()
        .enumerate()
        .filter_map(|(index, case)| {
            let action = (case.action)(&kit);
            let variant = variant_index(&action);
            run(case, &kit)
                .err()
                .map(|error| format!("case {index} (variant {variant}): {error}"))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// A second open of the same folder reuses its pane; "open in new tab"
/// makes another beside it.
#[test]
fn opening_a_folder_twice_reuses_its_pane_unless_asked_for_a_new_tab() {
    let kit = fixture();
    let mut app = Baboon::for_test();
    kit.install(&mut app);
    let open = |app: &mut Baboon, new_tab: bool| {
        app.handle_browser_action(
            BrowserAction::OpenFolderBrowser {
                rel_path: PathBuf::from(FOLDER),
                label: "props".to_owned(),
                open_in_new_tab: new_tab,
            },
            ctx(),
        )
    };
    open(&mut app, false);
    open(&mut app, false);
    assert_eq!(app.views[app.model.kits[0].id].browser.folder_browsers.len(), 1);
    open(&mut app, true);
    let base = folder_pane_key(Path::new(FOLDER));
    let mut keys: Vec<_> = app.views[app.model.kits[0].id].browser.folder_browsers.keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, vec![base.clone(), format!("{base}#2")]);
}

/// Toggling a favorite twice takes it back out, and an emptied kit drops
/// its favorites entry altogether.
#[test]
fn a_favorite_toggled_twice_is_gone() {
    let kit = fixture();
    let mut app = Baboon::for_test();
    kit.install(&mut app);
    for _ in 0..2 {
        app.handle_browser_action(BrowserAction::ToggleFavorite(kit.key(MODEL)), ctx());
    }
    assert_eq!(app.model.status, format!("Removed {MODEL} from Favorites"));
    assert!(app.model.prefs.editing_kit_favorites.is_empty());
    assert!(app.model.kits[0].active_favorite_entries.is_empty());
}
