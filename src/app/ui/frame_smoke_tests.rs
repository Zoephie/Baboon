//! Whole-frame smoke test: every window, dialog, prompt and pane the app can
//! show, opened one at a time over a populated app and drawn through
//! [`Baboon::run_frame`] — `eframe::App::logic` then `App::ui` — for several
//! frames.
//!
//! Each case is a `base` state (a kit of some kind, or nothing) and an `open`
//! step that puts one window or pane on top of it. The case runs [`FRAMES`]
//! frames and requires every one of its `expect` strings in the last frame's
//! painted text; a panic anywhere in the frame fails it too. A window paints
//! nothing on its first frame, and one that never drew would leave its title
//! and labels unpainted, so a window that silently stayed shut fails rather
//! than passing on an empty screen.
//!
//! The expectations are themselves checked: each case also runs its `base`
//! alone, and at least one `expect` string must be missing there. An
//! expectation the base already paints (a menu label, a browser row) would
//! pass whether or not the window drew, so it is rejected.
//!
//! The cases are split across [`SHARDS`] tests, which run in parallel; each
//! runs all of its cases and reports every failure together. Run them with
//! `cargo test frame_smoke`. `BABOON_SMOKE_ONLY=name,name` runs only cases
//! whose name contains one of them; `BABOON_SMOKE_DUMP=1` (with
//! `--nocapture`) prints every case's painted text.
//!
//! All data is synthetic: tags are built from this repository's
//! `definitions/` schemas, and the loose kits are temporary folders of those
//! tags. Nothing is read from a real kit or game.
//!
//! Adding a window is one row in [`cases`]. [`every_window_has_a_smoke_case`]
//! is what notices a window without one: it reads the `Baboon` struct out of
//! `src/app/mod.rs` and every `egui::Window::new` site out of `src/app/`, and
//! requires each field shaped like window state and each file that opens a
//! window to be named by some case, or listed in [`NOT_WINDOWS`] with the
//! reason. A new `Option<…Dialog>` field, or a new file with a window in it,
//! fails that test until it has a row here.

use super::perf_baseline_tests::{Harness, fixture};
use super::*;
use crate::core::source::{LoadedSourceData, TagEntry, TagEntryLocation, TagSource};

/// Frames each case runs after its setup. A window's first frame only
/// measures it; the second is the first that paints; the rest let anything
/// it asks for on its first frames (a second layout pass, a fast worker)
/// land.
const FRAMES: usize = 6;

type Step = fn(&mut Harness);

/// One smoke case: a base state, one thing opened over it, and what must be
/// on screen.
struct Case {
    name: &'static str,
    /// `Baboon` fields this case opens: the registry
    /// [`every_window_has_a_smoke_case`] checks the struct against.
    fields: &'static [&'static str],
    /// Files under `src/app/` whose `egui::Window::new` this case draws.
    sources: &'static [&'static str],
    base: Step,
    open: Step,
    /// Every one must be painted by the last frame, and at least one must
    /// not be painted by `base` alone.
    expect: &'static [&'static str],
}

const fn case(
    name: &'static str,
    fields: &'static [&'static str],
    sources: &'static [&'static str],
    base: Step,
    open: Step,
    expect: &'static [&'static str],
) -> Case {
    Case {
        name,
        fields,
        sources,
        base,
        open,
        expect,
    }
}

// ---------------------------------------------------------------------------
// Bases
// ---------------------------------------------------------------------------

fn welcome(_: &mut Harness) {}

thread_local! {
    /// Folders the current case's bases made, removed after its frames.
    static TEMP_DIRS: std::cell::RefCell<Vec<PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = crate::test_kits::unique_temp_dir(name);
    TEMP_DIRS.with(|dirs| dirs.borrow_mut().push(dir.clone()));
    dir
}

/// The perf fixture's 1,000-tag in-memory Halo 3 kit.
fn memory_kit(h: &mut Harness) {
    fixture::install_kit(&mut h.app, fixture::synthetic_entries(4, 5, 50));
}

/// A loose Halo 3 editing kit in a fresh temporary folder, holding a few
/// synthetic tags written from the definitions.
fn loose_kit(h: &mut Harness) {
    let root = temp_dir("frame-smoke-kit").join("tags");
    for (rel, group) in [
        ("objects/weapons/rifle/rifle.biped", "biped"),
        ("objects/weapons/rifle/rifle.scenery", "scenery"),
        ("levels/smoke/smoke.scenario", "scenario"),
    ] {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        fixture::new_tag(group).write(&path).unwrap();
    }
    let source = crate::core::source::load_editing_kit_layout(
        root,
        "Smoke Kit".to_owned(),
        GameId::from_id(fixture::GAME).unwrap(),
        &h.app.default_names,
        &locate_definitions_root(),
    )
    .expect("the synthetic loose kit loads");
    h.app.install_loaded_source(source);
}

/// The tags root of the [`loose_kit`] base.
fn loose_root(h: &Harness) -> PathBuf {
    h.app.loaded_tags_root().expect("a loose kit is loaded")
}

const CE_TAG: &str = "objects/weapons/rifle/rifle.weapon";

/// A Campaign Evolved container workspace with no containers on disk: its
/// entries are in memory, which is all the dialogs over it read.
fn container_kit(h: &mut Harness) {
    let entries: Vec<TagEntry> = [CE_TAG, "objects/weapons/rifle/rifle.model"]
        .into_iter()
        .map(|path| TagEntry {
            key: format!("ce:{path}"),
            display_path: path.to_owned(),
            group_tag: if path.ends_with(".weapon") {
                u32::from_be_bytes(*b"weap")
            } else {
                u32::from_be_bytes(*b"hlmt")
            },
            group_name: path.rsplit_once('.').map(|(_, ext)| ext.to_owned()),
            location: TagEntryLocation::LooseFile(path.into()),
        })
        .collect();
    let root = temp_dir("frame-smoke-paks");
    h.app.install_loaded_source(LoadedSourceData {
        label: "Campaign Evolved".to_owned(),
        source: TagSource::IoStoreContainerSet {
            root,
            containers: Vec::new(),
            index: Default::default(),
            packages: Default::default(),
            shipped: Default::default(),
        },
        names: h.app.default_names.clone(),
        game: Some(GameId::CampaignEvolved),
        tree: crate::core::source::build_tree(&entries),
        group_tree: crate::core::source::build_group_tree(&entries),
        all_entries: entries.clone(),
        entries,
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: true,
        chosen_kit_layout: None,
    });
}

fn ce_key() -> String {
    format!("ce:{CE_TAG}")
}

fn active_id(h: &Harness) -> KitId {
    h.app.active_kit_id()
}

fn biped_key() -> String {
    fixture::entry_key("folder_00/sub_00/tag_000.biped")
}

// --- tags for the pane cases: each base lists the tag, each open opens it ---

const SCENARIO: &str = "levels/smoke/smoke.scenario";

fn scenario_tag() -> TagFile {
    fixture::large_scenario(&[2]).0
}

/// An in-memory kit of `game` whose browser lists `path`.
fn pane_kit(h: &mut Harness, game: &str, path: &str, tag: &TagFile) {
    let mut entries = fixture::synthetic_entries(2, 2, 10);
    entries.push(fixture::document_entry(path, tag));
    fixture::install_kit_for_game(&mut h.app, entries, game);
}

fn scenario_kit(h: &mut Harness) {
    pane_kit(h, fixture::GAME, SCENARIO, &scenario_tag());
}

fn open_scenario(h: &mut Harness) -> String {
    fixture::open_document(&mut h.app, SCENARIO, scenario_tag())
}

fn shader_kit(h: &mut Harness) {
    let shader = fixture::synthetic_shader(3);
    pane_kit(h, fixture::GAME, "shaders/smoke.shader", &shader);
    fixture::install_render_method(&mut h.app, &shader, 3, 2, 4);
}

fn material_tag() -> TagFile {
    fixture::new_tag_for("halo4_mcc", "material")
}

fn sound_tag() -> TagFile {
    fixture::synthetic_ce_sound(2, 1.0)
}

fn render_model_tag() -> TagFile {
    fixture::new_tag("render_model")
}

/// A Halo 3 light, whose function fields sit near the top of the tag.
fn light_tag() -> TagFile {
    fixture::new_tag("light")
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

fn cases() -> Vec<Case> {
    let mut cases = vec![
        // --- the shell ---
        case(
            "kit_browser",
            &[],
            &[],
            welcome,
            memory_kit,
            &["folder_00", "folder_03", "search tags"],
        ),
        case(
            "terminal",
            &["terminal"],
            &[],
            memory_kit,
            |h| fixture::open_terminal(&mut h.app, (0..50).map(fixture::terminal_line)),
            &["Open full log", ": tool.exe: importing"],
        ),
        // --- tag panes, one per editor kind ---
        case(
            "pane_generic_fields",
            &[],
            &[],
            scenario_kit,
            |h| {
                open_scenario(h);
            },
            &["smoke.scenario", "Skies"],
        ),
        case(
            "pane_shader",
            &[],
            &[],
            shader_kit,
            |h| {
                fixture::open_document(
                    &mut h.app,
                    "shaders/smoke.shader",
                    fixture::synthetic_shader(3),
                );
            },
            &["smoke.shader", "PERF_CATEGORY_0"],
        ),
        case(
            "pane_material",
            &[],
            &[],
            |h| pane_kit(h, "halo4_mcc", "shaders/smoke.material", &material_tag()),
            |h| {
                fixture::open_document(&mut h.app, "shaders/smoke.material", material_tag());
            },
            &["smoke.material"],
        ),
        case(
            "pane_sound",
            &["audio"],
            &[],
            |h| pane_kit(h, "haloce_mcc", "sound/smoke.sound", &sound_tag()),
            |h| {
                fixture::open_document(&mut h.app, "sound/smoke.sound", sound_tag());
            },
            &["smoke.sound", "2 permutations"],
        ),
        case(
            "pane_bitmap",
            &[],
            &[],
            |h| {
                let bitmap = fixture::new_tag("bitmap");
                pane_kit(h, fixture::GAME, "bitmaps/smoke.bitmap", &bitmap);
            },
            |h| {
                let bitmap = fixture::new_tag("bitmap");
                fixture::open_document(&mut h.app, "bitmaps/smoke.bitmap", bitmap);
            },
            &["smoke.bitmap"],
        ),
        case(
            "pane_model_preview",
            &[],
            &[],
            |h| {
                let model = render_model_tag();
                pane_kit(h, fixture::GAME, "objects/smoke.render_model", &model);
            },
            |h| {
                let model = render_model_tag();
                let key = fixture::open_document(&mut h.app, "objects/smoke.render_model", model);
                let kit = &mut h.app.kits[h.app.active];
                kit.model_previews.entry(key).or_default().active_tab =
                    ModelTagPanelTab::ModelPreview;
            },
            &["smoke.render_model", "Model Preview"],
        ),
        case(
            "pane_function_rows",
            &[],
            &[],
            |h| pane_kit(h, fixture::GAME, "objects/smoke.light", &light_tag()),
            |h| {
                let key = fixture::open_document(&mut h.app, "objects/smoke.light", light_tag());
                fixture::expand_all(&mut h.app, &key);
            },
            &["smoke.light", "Function type:"],
        ),
        // --- synthetic panes ---
        case(
            "pane_bitmap_library",
            &[],
            &[],
            memory_kit,
            |h| h.app.open_bitmap_library(),
            &["Bitmap Library", "No bitmap tags in this workspace."],
        ),
        case(
            "pane_model_library",
            &[],
            &[],
            memory_kit,
            |h| h.app.open_model_library(),
            &["Model Library", "No render model tags in this workspace."],
        ),
        case(
            "pane_git_review",
            &[],
            &[],
            loose_kit,
            |h| h.app.kits[h.app.active].open_tag_pane(GIT_REVIEW_KEY),
            &["Git Review", "No Git repository found"],
        ),
        case(
            "pane_blam",
            &[],
            &[],
            loose_kit,
            |h| h.app.kits[h.app.active].open_tag_pane(BLAM_KEY),
            &["Blam!", "No import has run yet."],
        ),
        case(
            "pane_folder",
            &[],
            &[],
            loose_kit,
            |h| {
                let ctx = h.ctx.clone();
                h.app.handle_browser_action(
                    BrowserAction::OpenFolderBrowser {
                        rel_path: PathBuf::from("objects/weapons/rifle"),
                        label: "rifle".to_owned(),
                        open_in_new_tab: false,
                    },
                    ctx,
                );
            },
            &["rifle.biped", "rifle.scenery"],
        ),
        // --- the three tile trees, split ---
        case(
            "tiles_tag_tree_split",
            &[],
            &[],
            |h| {
                let mut entries = fixture::synthetic_entries(2, 2, 10);
                entries.push(fixture::document_entry(
                    "shaders/left.shader",
                    &fixture::synthetic_shader(1),
                ));
                entries.push(fixture::document_entry("levels/right.scenario", &scenario_tag()));
                fixture::install_kit(&mut h.app, entries);
            },
            |h| {
                fixture::open_document(
                    &mut h.app,
                    "shaders/left.shader",
                    fixture::synthetic_shader(1),
                );
                let kit = &mut h.app.kits[h.app.active];
                let key = fixture::entry_key("levels/right.scenario");
                kit.parsed_tags.insert(key.clone(), TagDocument::clean(scenario_tag()));
                kit.open_tag_pane_beside(&key);
            },
            &["left.shader", "right.scenario"],
        ),
        case(
            "tiles_kit_tree_split",
            &[],
            &[],
            |h| {
                fixture::install_kit_for_game(
                    &mut h.app,
                    fixture::synthetic_entries(2, 2, 10),
                    fixture::GAME,
                );
            },
            |h| {
                h.app.add_kit();
                let entries = fixture::synthetic_entries(1, 1, 5)
                    .into_iter()
                    .map(|mut entry| {
                        entry.display_path = entry.display_path.replace("folder_", "second_");
                        entry.key = fixture::entry_key(&entry.display_path);
                        entry
                    })
                    .collect();
                fixture::install_kit_for_game(&mut h.app, entries, "haloreach_mcc");
                let ids: Vec<KitId> = h.app.kits.iter().map(|kit| kit.id).collect();
                let mut tree = egui_tiles::Tree::empty("smoke_kit_tree");
                let panes = ids.iter().map(|id| tree.tiles.insert_pane(*id)).collect();
                tree.root = Some(tree.tiles.insert_horizontal_tile(panes));
                h.app.kit_tree = tree;
            },
            &["folder_00", "second_00"],
        ),
        case(
            "tiles_chimp_surface",
            &[],
            &[],
            |h| {
                h.app.prefs.enable_chimp = true;
                container_kit(h);
            },
            |h| h.app.kits[h.app.active].surface = KitSurface::Chimp,
            &["The Unreal package index has not been started."],
        ),
        // --- windows over the shell ---
        case(
            "first_run_storage",
            &["first_run_wizard"],
            &["ui/first_run/mod.rs"],
            welcome,
            |h| h.app.first_run_wizard = Some(FirstRunWizardState::new(None)),
            &["Welcome to Baboon", "Installed mode (recommended)"],
        ),
        case(
            "first_run_interface",
            &["first_run_wizard"],
            &["ui/first_run/mod.rs"],
            welcome,
            |h| {
                let mut wizard = FirstRunWizardState::new(None);
                wizard.page = FirstRunPage::Interface;
                h.app.first_run_wizard = Some(wizard);
            },
            &["Welcome to Baboon", "Updates and interface"],
        ),
        case(
            "first_run_editing_kits",
            &["first_run_wizard"],
            &["ui/first_run/mod.rs"],
            welcome,
            |h| {
                let mut wizard = FirstRunWizardState::new(None);
                wizard.page = FirstRunPage::EditingKits;
                // Detection searches the machine for installed kits; the
                // case is about the page, not about what is installed here.
                wizard.editing_kit_detection_ran = true;
                h.app.first_run_wizard = Some(wizard);
            },
            &["Welcome to Baboon", "Detected paths fill only empty entries."],
        ),
        case(
            "help_about",
            &["about_open", "help_panel_tab"],
            &["help/window/mod.rs"],
            welcome,
            |h| {
                h.app.about_open = true;
                h.app.help_panel_tab = HelpPanelTab::About;
            },
            &["Baboon Help", "blam-tags created by"],
        ),
        case(
            "help_doc",
            &["about_open", "help_docs"],
            &["help/window/mod.rs"],
            welcome,
            |h| {
                h.app.about_open = true;
                h.app.help_panel_tab = HelpPanelTab::Doc;
            },
            &["Baboon Help", "Supported games"],
        ),
        case(
            "help_tutorials",
            &["about_open", "tutorials"],
            &["help/window/mod.rs"],
            welcome,
            |h| {
                h.app.about_open = true;
                h.app.help_panel_tab = HelpPanelTab::Tutorials;
            },
            &["Baboon Help", "Watch on YouTube"],
        ),
        case(
            "help_script_doc",
            &["about_open", "script_docs"],
            &["help/window/mod.rs"],
            welcome,
            |h| {
                h.app.about_open = true;
                h.app.help_panel_tab = HelpPanelTab::ScriptDoc;
            },
            &["Baboon Help", "Network safe"],
        ),
        case(
            "help_tag_compat",
            &["about_open", "tag_compat"],
            &["help/window/mod.rs"],
            welcome,
            |h| {
                h.app.about_open = true;
                h.app.help_panel_tab = HelpPanelTab::TagCompat;
            },
            &["Baboon Help", "Only what is lost"],
        ),
        case(
            "help_map_names",
            &["about_open"],
            &["help/window/mod.rs"],
            welcome,
            |h| {
                h.app.about_open = true;
                h.app.help_panel_tab = HelpPanelTab::MapNames;
            },
            &["Baboon Help", "The Pillar of Autumn"],
        ),
        case(
            "settings_startup",
            &["settings_open", "settings_tab"],
            &["ui/settings/mod.rs"],
            welcome,
            |h| {
                h.app.settings_open = true;
                h.app.settings_tab = SettingsTab::Startup;
            },
            &["Settings", "When reopening Baboon with a previous session:"],
        ),
        case(
            "settings_browser",
            &["settings_open"],
            &["ui/settings/mod.rs"],
            welcome,
            |h| {
                h.app.settings_open = true;
                h.app.settings_tab = SettingsTab::Browser;
            },
            &["Settings", "Double-click to open tags"],
        ),
        case(
            "settings_editing_kits",
            &["settings_open"],
            &["ui/settings/mod.rs"],
            welcome,
            |h| {
                h.app.settings_open = true;
                h.app.settings_tab = SettingsTab::EditingKits;
            },
            &["Settings", "Auto Detect"],
        ),
        case(
            "settings_appearance",
            &["settings_open"],
            &["ui/settings/mod.rs"],
            welcome,
            |h| {
                h.app.settings_open = true;
                h.app.settings_tab = SettingsTab::Appearance;
            },
            &["Settings", "Angles in degrees"],
        ),
        case(
            "settings_tools",
            &["settings_open"],
            &["ui/settings/mod.rs"],
            welcome,
            |h| {
                h.app.settings_open = true;
                h.app.settings_tab = SettingsTab::Tools;
            },
            &["Settings", "Chimp — Unreal mappings"],
        ),
        case(
            "settings_custom_kit_draft",
            &["custom_editing_kit_draft"],
            &["ui/settings/mod.rs"],
            welcome,
            |h| {
                h.app.settings_open = true;
                h.app.settings_tab = SettingsTab::EditingKits;
                h.app.custom_editing_kit_draft = Some(CustomEditingKitDraft::new());
            },
            &["Editing Kit Root Folder"],
        ),
        case(
            "settings_custom_kit_removal",
            &["custom_editing_kit_removal"],
            &["ui/settings/mod.rs"],
            welcome,
            |h| {
                h.app.settings_open = true;
                h.app.settings_tab = SettingsTab::EditingKits;
                h.app.custom_editing_kit_removal = Some(CustomEditingKitRemoval {
                    id: "smoke".to_owned(),
                    name: "Smoke Kit".to_owned(),
                });
            },
            &["Remove Editing Kit?"],
        ),
        case(
            "find",
            &["find"],
            &["search/find_window/mod.rs"],
            |h| {
                scenario_kit(h);
                open_scenario(h);
            },
            |h| {
                h.app.find.open = true;
                h.app.find.query = "sky".to_owned();
            },
            &["Find", "Filter Results"],
        ),
        case(
            "tool_commands",
            &["tool_commands"],
            &["ui/tool_commands.rs"],
            loose_kit,
            |h| h.app.tool_commands.open = true,
            &["Tool Commands"],
        ),
        case(
            "tag_compare",
            &["tag_diff"],
            &["compare/tag_compare/mod.rs"],
            |h| {
                scenario_kit(h);
                open_scenario(h);
            },
            |h| {
                h.app.tag_diff = Some(TagDiffState {
                    kit: active_id(h),
                    a_key: fixture::entry_key(SCENARIO),
                    source: TagCompareSource::OpenTag,
                    b_kit: None,
                    b_key: None,
                    b_path: None,
                    comparison_kit_root: None,
                    git_history: Default::default(),
                    error: None,
                    filters: Default::default(),
                    swapped: false,
                    results: None,
                    git_pending: None,
                });
            },
            &["Compare"],
        ),
        case(
            "content_explorer",
            &["content_explorer"],
            &["search/result_windows/mod.rs"],
            memory_kit,
            |h| {
                let focus = h.app.kits[h.app.active]
                    .source
                    .as_ref()
                    .unwrap()
                    .entries[0]
                    .clone();
                h.app.content_explorer = Some(ContentExplorer {
                    kit: active_id(h),
                    focus,
                    parents: Vec::new(),
                    children: Vec::new(),
                    filter: String::new(),
                    index_unavailable: true,
                    back: Vec::new(),
                    forward: Vec::new(),
                });
            },
            &["Content Explorer"],
        ),
        case(
            "query_results",
            &["query_results"],
            &["search/result_windows/mod.rs"],
            memory_kit,
            |h| {
                let entries = h.app.kits[h.app.active].source.as_ref().unwrap().entries[..3]
                    .to_vec();
                h.app.query_results = Some(TagQueryResults {
                    kit: active_id(h),
                    title: "Smoke Query Results".to_owned(),
                    entries,
                    annotations: Vec::new(),
                    note: None,
                    ref_target: None,
                });
            },
            &["Smoke Query Results", "tag_000"],
        ),
        case(
            "field_value_search",
            &["field_value_search_open"],
            &["search/result_windows/mod.rs"],
            loose_kit,
            |h| h.app.field_value_search_open = true,
            &["Search Field Values"],
        ),
        case(
            "tag_reference_picker",
            &["tag_reference_picker", "tag_reference_picker_kit"],
            &["search/result_windows/mod.rs"],
            container_kit,
            |h| {
                h.app.tag_reference_picker = Some(TagReferencePickerState {
                    tag_key: ce_key(),
                    field_path: "model".to_owned(),
                    allowed_groups: vec![u32::from_be_bytes(*b"hlmt")],
                    current_group: None,
                    search: String::new(),
                });
                h.app.tag_reference_picker_kit = Some(active_id(h));
            },
            &["Select Tag Reference"],
        ),
        case(
            "colour_popup",
            &["color_popup", "color_popup_kit"],
            &["material/color_picker/mod.rs"],
            memory_kit,
            |h| {
                h.app.color_popup =
                    Some(MaterialColorPopup::new("Smoke Tint", 1.0, 0.5, 0.25, 1.0));
                h.app.color_popup_kit = Some(active_id(h));
            },
            &["Color Picker"],
        ),
        case(
            "function_popup",
            &["function_popup", "function_popup_kit"],
            &["function_editor/mod.rs"],
            memory_kit,
            |h| {
                let bytes = decode_hex(&constant_function_hex(0.5)).unwrap();
                let view = FunctionView::from_function(TagFunction::parse(&bytes).unwrap());
                h.app.function_popup = Some(FunctionPopup::new(
                    biped_key(),
                    "Smoke Function".to_owned(),
                    view,
                    true,
                ));
                h.app.function_popup_kit = Some(active_id(h));
            },
            &["Smoke Function"],
        ),
        case(
            "save_changes_prompt",
            &["save_changes_prompt"],
            &["controller/mod.rs"],
            memory_kit,
            |h| {
                h.app.save_changes_prompt = SaveChangesPrompt {
                    visible: true,
                    dirty_tags: vec![DirtyTagEntry {
                        path: "objects/smoke.biped".to_owned(),
                        tag_id: biped_key(),
                        checked: true,
                    }],
                    ..Default::default()
                };
            },
            &["Baboon - Save Changes?", "objects/smoke.biped"],
        ),
        case(
            "last_opened_windows",
            &["last_opened_windows"],
            &["controller/mod.rs"],
            welcome,
            |h| {
                h.app.last_opened_windows = Some(LastOpenedWindowsPrompt {
                    visible: true,
                    kits: vec![LastOpenedWindowsKit {
                        source_kind: LastSessionSourceKind::LooseFolder,
                        source_path: PathBuf::from("/no/such/smoke/tags"),
                        game: Some(fixture::GAME.to_owned()),
                        profile_id: None,
                        profile_name: None,
                        profile_root: None,
                        source_available: false,
                        project_path: None,
                        has_project: false,
                        browser_mode: None,
                        browser_sort: None,
                        entries: vec![LastOpenedWindowEntry {
                            tag: LastSessionTag {
                                key: "file:/no/such/smoke.biped".to_owned(),
                                label: "smoke.biped".to_owned(),
                                group_tag: u32::from_be_bytes(*b"bipd"),
                                path: None,
                            },
                            checked: false,
                            available: false,
                        }],
                        folder_entries: Vec::new(),
                        chimp_entries: Vec::new(),
                        bitmap_library_open: false,
                        model_library_open: false,
                        active_chimp_package: None,
                        was_active: true,
                    }],
                    dont_ask_again: false,
                });
            },
            &["Last Opened Windows", "smoke.biped"],
        ),
        case(
            "block_confirm",
            &["block_confirm"],
            &["controller/mod.rs"],
            |h| {
                scenario_kit(h);
                open_scenario(h);
            },
            |h| {
                h.app.block_confirm = Some(BlockConfirm {
                    kit: Some(active_id(h)),
                    tag_key: fixture::entry_key(SCENARIO),
                    path: "skies".to_owned(),
                    kind: BlockOpKind::DeleteAll,
                    message: "Delete every smoke element?".to_owned(),
                    confirm_label: "Delete".to_owned(),
                });
            },
            &["Confirm", "Delete every smoke element?"],
        ),
        case(
            "entry_index_wait_notice",
            &["show_entry_index_wait_notice"],
            &["ui/shell/mod.rs"],
            memory_kit,
            |h| {
                h.app.show_entry_index_wait_notice = true;
                h.app.kits[h.app.active].scanning_entries = true;
            },
            &["Indexing"],
        ),
        case(
            "folder_refactor_lock",
            &["folder_refactor"],
            &[],
            memory_kit,
            |h| {
                h.app.folder_refactor = Some(FolderRefactorUiState {
                    label: "Renaming smoke to fog".to_owned(),
                    phase: "Moving files".to_owned(),
                    progress: Some(0.5),
                });
            },
            &["Renaming smoke to fog", "Baboon is locked until references are updated."],
        ),
        // --- ui/dialogs/ ---
        case(
            "new_tag",
            &["new_tag_open", "new_tag_dialog"],
            &["tag_ops/new_tag_window.rs"],
            loose_kit,
            |h| h.app.open_new_tag_dialog(),
            &["New Tag"],
        ),
        case(
            "delete_confirm_loose",
            &["delete_confirm"],
            &["tag_ops/delete_confirm.rs"],
            loose_kit,
            |h| {
                let path = loose_root(h).join("objects/weapons/rifle/rifle.biped");
                h.app.delete_confirm = Some(DeleteConfirm {
                    kit: active_id(h),
                    key: file_entry_key(&path),
                    display_path: "objects/weapons/rifle/rifle.biped".to_owned(),
                    kind: DeleteKind::Loose,
                    referrers: vec!["levels/smoke/smoke.scenario".to_owned()],
                    referrers_unavailable: false,
                    has_unsaved_edits: true,
                });
            },
            &["Delete tag?", "levels/smoke/smoke.scenario"],
        ),
        case(
            "delete_confirm_container",
            &["delete_confirm"],
            &["tag_ops/delete_confirm.rs"],
            container_kit,
            |h| {
                h.app.delete_confirm = Some(DeleteConfirm {
                    kit: active_id(h),
                    key: ce_key(),
                    display_path: CE_TAG.to_owned(),
                    kind: DeleteKind::Container {
                        target_label: "pakchunk0-smoke".to_owned(),
                    },
                    referrers: Vec::new(),
                    referrers_unavailable: true,
                    has_unsaved_edits: false,
                });
            },
            &["Delete from Campaign Evolved container?"],
        ),
        case(
            "rename_tag",
            &["rename_tag"],
            &["tag_ops/rename_tag_window.rs"],
            loose_kit,
            |h| {
                let path = loose_root(h).join("objects/weapons/rifle/rifle.biped");
                h.app.rename_tag = Some(RenameTagState {
                    kit: active_id(h),
                    key: file_entry_key(&path),
                    old_display: "objects/weapons/rifle/rifle.biped".to_owned(),
                    extension: "biped".to_owned(),
                    operation: TagNameOperation::Rename,
                    new_path_input: "objects/weapons/rifle/smoke_rifle".to_owned(),
                    fixed_parent: String::new(),
                    focus_input: true,
                    referrers: Vec::new(),
                    referrers_unavailable: true,
                    is_container: false,
                    is_new_container: false,
                    whole_path_editable: true,
                    in_place_pak: None,
                });
            },
            &["Rename / Move Tag"],
        ),
        case(
            "duplicate_tag",
            &["rename_tag"],
            &["tag_ops/rename_tag_window.rs"],
            container_kit,
            |h| {
                h.app.rename_tag = Some(RenameTagState {
                    kit: active_id(h),
                    key: ce_key(),
                    old_display: CE_TAG.to_owned(),
                    extension: "weapon".to_owned(),
                    operation: TagNameOperation::Duplicate,
                    new_path_input: "rifle_copy".to_owned(),
                    fixed_parent: "objects/weapons/rifle".to_owned(),
                    focus_input: true,
                    referrers: Vec::new(),
                    referrers_unavailable: true,
                    is_container: true,
                    is_new_container: false,
                    whole_path_editable: false,
                    in_place_pak: None,
                });
            },
            &["Duplicate Tag"],
        ),
        case(
            "import_tag",
            &["import_tag_dialog"],
            &["import/import_tag_dialog.rs"],
            container_kit,
            |h| {
                h.app.import_tag_dialog = Some(ImportTagDialog {
                    kit: active_id(h),
                    source_path: PathBuf::from("/no/such/smoke.weapon"),
                    folder_rel: "objects/weapons".to_owned(),
                    name: "smoke".to_owned(),
                    group_tag: u32::from_be_bytes(*b"weap"),
                    group_name: "weapon".to_owned(),
                    extension: "weapon".to_owned(),
                    tag: Some(fixture::new_tag_for("haloce_evolved", "weapon")),
                    mode: ImportMode::Native {
                        comparison: None,
                        import_anyway: false,
                    },
                    profile_verdicts: Vec::new(),
                    error: None,
                });
            },
            &["Import Tag"],
        ),
        case(
            "import_discard_confirm",
            &["import_discard_confirm"],
            &["import/import_tag_dialog.rs"],
            container_kit,
            |h| {
                h.app.import_discard_confirm = Some(PendingImport {
                    kit: active_id(h),
                    tag: fixture::new_tag_for("haloce_evolved", "weapon"),
                    target_key: ce_key(),
                });
            },
            &["Discard unsaved changes?"],
        ),
        case(
            "tag_import",
            &["tag_import_dialog"],
            &["import/tags_window.rs"],
            loose_kit,
            |h| h.app.open_tag_import_dialog(Some("objects".to_owned())),
            &["Import Tags"],
        ),
        case(
            "cache_import",
            &["cache_import_dialog"],
            &["import/cache_window/mod.rs"],
            loose_kit,
            |h| {
                let target = CacheImportTarget {
                    kit: active_id(h),
                    label: "Smoke Kit".to_owned(),
                    game: GameId::from_id(fixture::GAME).unwrap(),
                    tags_root: loose_root(h),
                };
                h.app.cache_import_dialog = Some(CacheImportDialog {
                    kit: active_id(h),
                    prefix: "objects/weapons".to_owned(),
                    selected: 2,
                    targets: vec![target],
                    target_index: 0,
                    outside_tree: Default::default(),
                    outside_picked: Default::default(),
                    single: None,
                    destination: None,
                    replace: ReplaceChoice::Always,
                    conflicts: Default::default(),
                    conflict_picked: Default::default(),
                    conflicts_stale: false,
                    scanning: false,
                    running: false,
                    cancel: Default::default(),
                    progress: None,
                    report: None,
                    error: None,
                });
            },
            &["Import Cache Folder"],
        ),
        case(
            "overwrite_confirm",
            &["overwrite_confirm"],
            &["mods/overwrite_confirm.rs"],
            container_kit,
            |h| {
                h.app.overwrite_confirm = Some(OverwriteConfirm {
                    kit: active_id(h),
                    key: ce_key(),
                });
            },
            &["Overwrite game files?"],
        ),
        case(
            "clear_stash_confirm",
            &["clear_stash_confirm"],
            &["mods/clear_stash_confirm.rs"],
            container_kit,
            |h| {
                h.app.clear_stash_confirm = Some(ClearStashConfirm {
                    kit: active_id(h),
                    stashed: vec![CE_TAG.to_owned()],
                    unsaved: 1,
                });
            },
            &["Clear unsaved modifications?"],
        ),
        case(
            "container_dump_confirm",
            &["container_dump_confirm"],
            &["export/container_dump_confirm.rs"],
            container_kit,
            |h| {
                h.app.container_dump_confirm = Some(ContainerDumpConfirm {
                    kit: active_id(h),
                    output: PathBuf::from("/no/such/smoke-out"),
                    total: 2,
                    scope: ContainerDumpScope::AllShipped,
                });
            },
            &["Extract every shipped tag?"],
        ),
        case(
            "container_duplicate_confirm",
            &["container_duplicate_confirm"],
            &["tag_ops/container_duplicate_confirm.rs"],
            container_kit,
            |h| {
                h.app.container_duplicate_confirm = Some(ContainerDuplicateConfirm {
                    kit: active_id(h),
                    key: ce_key(),
                    destination_leaf: "rifle_copy".to_owned(),
                });
            },
            &["Duplicate in Campaign Evolved container?"],
        ),
        case(
            "container_folder",
            &["container_folder_dialog"],
            &["tag_ops/container_folder_window.rs"],
            container_kit,
            |h| {
                h.app.container_folder_dialog = Some(ContainerFolderDialog {
                    kit: active_id(h),
                    parent_rel: Some("objects".to_owned()),
                    renaming: None,
                    name_input: "smoke".to_owned(),
                    focus_input: true,
                    error: None,
                });
            },
            &["New Folder"],
        ),
        case(
            "loose_folder_rename",
            &["loose_folder_rename"],
            &["tag_ops/loose_folder_rename_window.rs"],
            loose_kit,
            |h| {
                h.app.loose_folder_rename = Some(LooseFolderRenameState {
                    kit: active_id(h),
                    rel_path: PathBuf::from("objects/weapons/rifle"),
                    parent_display: "objects/weapons".to_owned(),
                    old_name: "rifle".to_owned(),
                    name_input: "smoke_rifle".to_owned(),
                    focus_input: true,
                    error: None,
                    tag_count: 2,
                    outside_referrers: None,
                });
            },
            &["Rename Folder"],
        ),
        case(
            "exported_mod",
            &["exported_mod"],
            &["mods/exported_mod_window.rs"],
            container_kit,
            |h| {
                h.app.exported_mod = Some(ExportedMod {
                    stem: "Smoke_P".to_owned(),
                    directory: PathBuf::from("/no/such/Paks/~mods"),
                    count: 2,
                    skipped: 0,
                });
            },
            &["Mod exported"],
        ),
        case(
            "mod_export",
            &["mod_export"],
            &["mods/mod_export_window/mod.rs"],
            container_kit,
            |h| {
                h.app.mod_export = Some(ModExportDialog {
                    kit: active_id(h),
                    review_only: false,
                    snapshot: CampaignProjectSnapshot {
                        game: "haloce_evolved".to_owned(),
                        source_path: PathBuf::new(),
                        selected_identity: None,
                        tabs: Vec::new(),
                        overlays: HashMap::new(),
                        history: Default::default(),
                        folders: Default::default(),
                    },
                    rows: vec![ModExportRow {
                        identity: CE_TAG.to_owned(),
                        display_path: CE_TAG.to_owned(),
                        group_tag: u32::from_be_bytes(*b"weap"),
                        kind: ModExportChange::Modified,
                        include: true,
                        bytes: 1024,
                        reason: None,
                        overridden_by: None,
                    }],
                    name: "Smoke".to_owned(),
                    folder: PathBuf::from("/no/such/Paks/~mods"),
                    overwrite_acknowledged: false,
                    expanded: HashSet::new(),
                    diffs: HashMap::new(),
                    controls_height: 0.0,
                });
            },
            &["Export Mod", "Smoke_P.utoc"],
        ),
        case(
            "keyword_chooser",
            &["keyword_chooser_open"],
            &["ui/dialogs/keyword_chooser.rs"],
            memory_kit,
            |h| h.app.keyword_chooser_open = true,
            &["Keywords"],
        ),
        case(
            "tsv_paste",
            &["tsv_paste"],
            &["ui/dialogs/tsv_paste.rs"],
            |h| {
                scenario_kit(h);
                open_scenario(h);
            },
            |h| {
                h.app.tsv_paste = Some(TsvPasteState {
                    kit: active_id(h),
                    tag_key: fixture::entry_key(SCENARIO),
                    block_path: "skies".to_owned(),
                    block_label: "skies".to_owned(),
                    element_count: 2,
                    text: String::new(),
                    status: None,
                });
            },
            &["Paste TSV → skies"],
        ),
        case(
            "operation_notice",
            &["operation_notice"],
            &["ui/dialogs/operation_notice.rs"],
            welcome,
            |h| {
                h.app.operation_notice = Some(OperationNotice {
                    title: "Smoke notice".to_owned(),
                    message: "The smoke operation finished.".to_owned(),
                    failed: false,
                });
            },
            &["Smoke notice", "The smoke operation finished."],
        ),
        case(
            "extract_target",
            &["extract_target"],
            &["export/extract_target_window/mod.rs"],
            memory_kit,
            |h| {
                h.app.extract_target = Some(ExtractTargetPrompt {
                    key: biped_key(),
                    display_path: "objects/smoke.render_model".to_owned(),
                    kind: ExtractKind::Geometry,
                    source: blam_tags::game::Game::Halo3,
                    target: blam_tags::game::Game::Halo3,
                });
            },
            &["Extract Geometry"],
        ),
        case(
            "chimp_discard",
            &["chimp_discard_prompt"],
            &["chimp/save/mod.rs"],
            container_kit,
            |h| {
                h.app.chimp_discard_prompt = Some(ChimpDiscardPrompt {
                    kit: active_id(h),
                    packages: vec!["/Game/Smoke/SM_Smoke".to_owned()],
                    pending_action: None,
                    error: None,
                });
            },
            &["Discard Chimp changes?"],
        ),
        case(
            "chimp_save",
            &[],
            &["chimp/save/mod.rs"],
            container_kit,
            |h| {
                let kit = h.app.active;
                h.app.open_chimp_save_dialog_for_test(kit);
            },
            &["Save Chimp changes"],
        ),
        case(
            "chimp_mesh_texture_prompt",
            &["chimp_mesh_texture_prompt"],
            &["ui/dialogs/chimp_prompts.rs"],
            container_kit,
            |h| {
                h.app.chimp_mesh_texture_prompt = Some(ChimpMeshTexturePrompt::for_test(
                    active_id(h),
                    "/Game/Smoke/SM_Smoke",
                ));
            },
            &["Export textures with this mesh?"],
        ),
        case(
            "chimp_texture_export_prompt",
            &["chimp_texture_export_prompt"],
            &["ui/dialogs/chimp_prompts.rs"],
            container_kit,
            |h| {
                h.app.chimp_texture_export_prompt = Some(ChimpTextureExportPrompt::for_test(
                    active_id(h),
                    "/Game/Smoke/T_Smoke",
                ));
            },
            &["Extract texture"],
        ),
        case(
            "chimp_level_export_prompt",
            &["chimp_level_export_prompt"],
            &["ui/dialogs/chimp_prompts.rs"],
            container_kit,
            |h| {
                h.app.chimp_level_export_prompt = Some(ChimpLevelExportPrompt::for_test(
                    active_id(h),
                    "/Game/Smoke/L_Smoke",
                ));
            },
            &["Export level"],
        ),
        case(
            "poke_scanning",
            &["poke_dialog"],
            &["runtime_poke/mod.rs"],
            container_kit,
            |h| {
                h.app.poke_dialog = Some(PokeDialog {
                    kit: active_id(h),
                    key: ce_key(),
                    state: PokeDialogState::Scanning,
                });
            },
            &["Poke Current Tag"],
        ),
        case(
            "poke_error",
            &["poke_dialog"],
            &["runtime_poke/mod.rs"],
            container_kit,
            |h| {
                h.app.poke_dialog = Some(PokeDialog {
                    kit: active_id(h),
                    key: ce_key(),
                    state: PokeDialogState::Error("The smoke process is not running".to_owned()),
                });
            },
            &["Poke Current Tag", "The smoke process is not running"],
        ),
    ];
    cases.sort_by_key(|case| case.name);
    cases
}

/// `Baboon` fields shaped like window state (an `Option`, a flag, or a
/// `…Dialog`/`…Prompt`/`…State` type) that are not a window of their own,
/// and why. A field of that shape named neither here nor by a case fails
/// [`every_window_has_a_smoke_case`].
const NOT_WINDOWS: &[(&str, &str)] = &[
    ("window_state", "native window geometry tracker"),
    ("native_clock", "the clock of the latest input"),
    ("native_template_cache", "import cache"),
    ("available_update", "data shown in Settings and the status bar"),
    ("last_update_check", "data shown in Settings"),
    ("container_dump_job", "a running job; its progress is in the status bar"),
    ("chimp_level_job", "a running job; its progress is in the status bar"),
    ("last_mod_export_name", "remembered text"),
    ("chimp_writes", "running saves"),
    ("game_banner_textures", "texture cache keyed by game"),
    ("last_poke", "undo record"),
    ("poke_direct_running", "running flag"),
    ("poke_undo_running", "running flag"),
    ("editing_kit_path_attention", "highlights a row of the Settings window"),
    ("deferred_file_action", "a queued action"),
    ("restored_active_kit", "session restore bookkeeping"),
    ("reveal_target", "a one-shot browser request"),
    ("field_value_searching", "running flag of the field value search"),
    ("kit_tool_drag", "drag-and-drop tracker"),
    ("ce_usmap", "parsed mappings cache"),
    ("pending_sound_extract", "a queued request"),
    ("pending_ce_sound_ref", "a queued request"),
    ("pending_open", "a queued request"),
    ("pending_tool_import", "a queued request"),
    ("blender_icon", "texture"),
    ("sapien_icon", "texture"),
    ("tag_test_icon", "texture"),
    ("block_clipboard", "clipboard contents"),
    ("pending_ref_jump", "a queued navigation"),
    ("pending_find_jump", "a queued navigation"),
    ("field_nav", "navigation highlight"),
];

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Run `steps` on a fresh app, then [`FRAMES`] frames; what the last painted.
fn painted_after(steps: &[Step]) -> (Vec<String>, String) {
    let mut h = Harness::new();
    for step in steps {
        step(&mut h);
    }
    for _ in 0..FRAMES {
        h.frame(Vec::new());
    }
    for dir in TEMP_DIRS.with(|dirs| std::mem::take(&mut *dirs.borrow_mut())) {
        let _ = std::fs::remove_dir_all(dir);
    }
    (h.painted, h.app.status)
}

fn missing(painted: &[String], expect: &[&'static str]) -> Vec<&'static str> {
    expect
        .iter()
        .copied()
        .filter(|needle| !painted.iter().any(|text| text.contains(needle)))
        .collect()
}

fn run_case(case: &Case) -> Result<(), String> {
    let (painted, status) = painted_after(&[case.base, case.open]);
    if std::env::var_os("BABOON_SMOKE_DUMP").is_some() {
        eprintln!("[smoke] {}: status `{status}`; painted {painted:?}", case.name);
    }
    let absent = missing(&painted, case.expect);
    if !absent.is_empty() {
        let mut shown = painted;
        shown.truncate(80);
        return Err(format!(
            "not painted: {absent:?}; status `{status}`; painted: {shown:?}"
        ));
    }
    let (control, _) = painted_after(&[case.base]);
    if missing(&control, case.expect).is_empty() {
        return Err(format!(
            "the base alone already paints every expectation {:?}, so they cannot show \
             the window drew",
            case.expect
        ));
    }
    Ok(())
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "panicked".to_owned())
}

/// Run every `SHARDS`th case from `shard`, each on a fresh app, and report
/// every failure at once.
fn run_shard(shard: usize) {
    let only: Vec<String> = std::env::var("BABOON_SMOKE_ONLY")
        .map(|value| value.split(',').map(str::trim).map(str::to_owned).collect())
        .unwrap_or_default();
    let mut failures = Vec::new();
    let mut ran = 0;
    for (index, case) in cases().iter().enumerate() {
        if index % SHARDS != shard
            || (!only.is_empty() && !only.iter().any(|part| case.name.contains(part.as_str())))
        {
            continue;
        }
        ran += 1;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_case(case)))
            .unwrap_or_else(|panic| Err(format!("panicked: {}", panic_message(panic))));
        if let Err(problem) = result {
            failures.push(format!("{}: {problem}", case.name));
        }
    }
    assert!(ran > 0 || !only.is_empty(), "shard {shard} has no cases");
    assert!(
        failures.is_empty(),
        "{} of {ran} case(s) failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// The cases are split across this many tests, which the harness runs in
/// parallel: run one after another they take most of a minute.
const SHARDS: usize = 8;

macro_rules! smoke_shards {
    ($($name:ident = $shard:literal),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run_shard($shard);
            }
        )*
        #[test]
        fn every_case_is_in_a_shard() {
            assert_eq!([$($shard),*], std::array::from_fn::<usize, SHARDS, _>(|i| i));
        }
    };
}

smoke_shards!(
    windows_draw_shard_0 = 0,
    windows_draw_shard_1 = 1,
    windows_draw_shard_2 = 2,
    windows_draw_shard_3 = 3,
    windows_draw_shard_4 = 4,
    windows_draw_shard_5 = 5,
    windows_draw_shard_6 = 6,
    windows_draw_shard_7 = 7,
);

// ---------------------------------------------------------------------------
// Registry: a window without a case is noticed
// ---------------------------------------------------------------------------

/// The `Baboon` struct's fields and their types, read from `src/app/mod.rs`.
fn baboon_fields() -> Vec<(String, String)> {
    let source = include_root_str!("src/app/mod.rs");
    let body = source
        .split_once("pub struct Baboon {")
        .expect("src/app/mod.rs declares `pub struct Baboon`")
        .1;
    let body = &body[..body.find("\n}\n").expect("the struct ends")];
    body.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .filter_map(|line| {
            let (name, ty) = line.split_once(':')?;
            let name = name.trim();
            (!name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
                .then(|| (name.to_owned(), ty.trim().trim_end_matches(',').to_owned()))
        })
        .collect()
}

/// Whether a field has the shape of window state: optional, a flag, or a
/// type named like a dialog's.
fn looks_like_a_window(ty: &str) -> bool {
    let leaf = ty.rsplit("::").next().unwrap_or(ty);
    ty.contains("Option<")
        || ty == "bool"
        || ["Dialog", "Prompt", "State", "Confirm", "Popup"]
            .iter()
            .any(|suffix| leaf.ends_with(suffix))
}

/// `text` without its top-level `#[cfg(test)] mod … { … }` blocks.
fn product_code(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("#[cfg(test)]\nmod ") {
        out.push_str(&rest[..start]);
        let module = &rest[start..];
        let header_end = module.find('\n').unwrap() + 1;
        let line_end = header_end + module[header_end..].find('\n').unwrap_or(0);
        rest = if module[header_end..line_end].trim_end().ends_with('{') {
            // Through the module's closing brace, at the start of a line.
            match module.find("\n}\n") {
                Some(end) => &module[end + 3..],
                None => "",
            }
        } else {
            &module[line_end..]
        };
    }
    out.push_str(rest);
    out
}

/// Files under `src/app/` (relative, `/`-separated) whose product code
/// calls `egui::Window::new`.
fn window_sources() -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "tests") {
                    continue;
                }
                walk(&path, root, out);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == "tests.rs" || name.ends_with("_tests.rs"))
            {
                let text = std::fs::read_to_string(&path).unwrap();
                if product_code(&text).contains("Window::new(") {
                    let rel = path.strip_prefix(root).unwrap();
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

/// Everything the registry is missing or names wrongly, given the struct's
/// fields and the files that open windows.
fn registry_problems(
    cases: &[Case],
    fields: &[(String, String)],
    sources: &[String],
) -> Vec<String> {
    let named_fields: HashSet<&str> = cases.iter().flat_map(|c| c.fields.iter().copied()).collect();
    let named_sources: HashSet<&str> =
        cases.iter().flat_map(|c| c.sources.iter().copied()).collect();
    let excused: HashSet<&str> = NOT_WINDOWS.iter().map(|(name, _)| *name).collect();
    let mut problems = Vec::new();
    for (name, ty) in fields {
        if looks_like_a_window(ty)
            && !named_fields.contains(name.as_str())
            && !excused.contains(name.as_str())
        {
            problems.push(format!(
                "`Baboon::{name}: {ty}` has no smoke case: add a row to `cases()` that opens \
                 it, or list it in NOT_WINDOWS with the reason"
            ));
        }
    }
    let field_names: HashSet<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
    for name in named_fields.iter().chain(excused.iter()) {
        if !field_names.contains(name) {
            problems.push(format!("`{name}` is named by the registry but is not a Baboon field"));
        }
    }
    for source in sources {
        if !named_sources.contains(source.as_str()) {
            problems.push(format!(
                "src/app/{source} opens an egui::Window that no smoke case draws"
            ));
        }
    }
    for source in &named_sources {
        if !sources.iter().any(|s| s == source) {
            problems.push(format!("a case names src/app/{source}, which opens no window"));
        }
    }
    problems
}

#[test]
fn every_window_has_a_smoke_case() {
    let fields = baboon_fields();
    assert!(
        fields.len() > 50 && fields.iter().any(|(name, _)| name == "delete_confirm"),
        "the field scan found {} fields; it no longer reads the struct",
        fields.len()
    );
    let sources = window_sources();
    assert!(
        sources.iter().any(|s| s == "tag_ops/delete_confirm.rs")
            && sources.iter().any(|s| s == "ui/settings/mod.rs"),
        "the source scan found {sources:?}; it no longer finds windows"
    );
    let problems = registry_problems(&cases(), &fields, &sources);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The registry check can fail: a new dialog field, and a new file with a
/// window in it, are each reported when nothing names them.
#[test]
fn the_registry_check_notices_a_window_without_a_case() {
    let mut fields = baboon_fields();
    let mut sources = window_sources();
    assert!(registry_problems(&cases(), &fields, &sources).is_empty());
    fields.push(("smoke_dialog".to_owned(), "Option<SmokeDialog>".to_owned()));
    sources.push("ui/dialogs/smoke.rs".to_owned());
    let problems = registry_problems(&cases(), &fields, &sources);
    assert!(problems.iter().any(|p| p.contains("Baboon::smoke_dialog")), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("ui/dialogs/smoke.rs")), "{problems:?}");
    assert_eq!(problems.len(), 2, "{problems:?}");
}
