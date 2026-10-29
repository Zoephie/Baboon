//! The kit tools' string editors: a menu of the values a `string_id` field
//! expects, on a button beside its box, and the shared error style while its value
//! isn't one of them. Drawn as Foundation's `TagFieldElementWithMenuPanel`
//! draws them, with buttons beside the box and the type label after them. blam-tags'
//! `string_editor` builds the menus; this opens the other tags they read
//! (the globals tag, a referenced object, the scenario a resource tag belongs
//! to) and caches what it built.

use super::*;
#[path = "string_references.rs"]
mod string_references;
use crate::core::source::{LoadedSourceData, TagEntry, TagEntryLocation};
use blam_tags::string_editor::{Context as EditorContext, Menu, StringEditors};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use crate::app::editor::string_picker;

/// The tag a pane draws, as string editors need it.
#[derive(Clone, Copy)]
pub(in crate::app) struct EditedTag<'a> {
    pub(in crate::app) tag: &'a blam_tags::TagFile,
    pub(in crate::app) entry: &'a TagEntry,
    /// The document's identity and edit revision: a menu built from the tag's
    /// own blocks is stale once either moves.
    pub(in crate::app) stamp: (u64, u64),
    pub(in crate::app) source: &'a LoadedSourceData,
}

/// The game's string editors, read once per definitions folder and game.
/// `None` when the definitions can't be read.
pub(in crate::app) fn string_editors(definitions_root: Option<&Path>, game: Option<GameId>) -> Option<Arc<StringEditors>> {
    use std::sync::{Mutex, OnceLock};
    type Editors = Option<Arc<StringEditors>>;
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, GameId), Editors>>> = OnceLock::new();
    let game = game?;
    let bundled = crate::core::bundled::locate_definitions_root();
    let root = definitions_root.unwrap_or(&bundled);
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry((root.to_path_buf(), game))
        .or_insert_with(|| match StringEditors::load(root.join(game.as_str())) {
            Ok(editors) => Some(Arc::new(editors)),
            Err(error) => {
                eprintln!("string editors for {}: {error}", game.as_str());
                None
            }
        })
        .clone()
}

pub(in crate::app) fn is_hud_message_list(edit: &FieldEditContext<'_>) -> bool {
    string_references::is_hud_message_list(edit)
}

/// The string editor of the field at `path`, with the game's editors.
pub(in crate::app) fn field_string_editor(edit: &FieldEditContext<'_>, path: &str) -> Option<(Arc<StringEditors>, String)> {
    let root = edit.root?;
    let editors = string_editors(edit.definitions_root, edit.game)?;
    let editor = editors.editor_at(root, path)?.to_owned();
    Some((editors, editor))
}

/// The edited tag's path in the kit's tags folder: `\`-separated, no
/// extension, as tag references write paths.
fn kit_relative_path(source: &LoadedSourceData, entry: &TagEntry) -> Option<String> {
    let crate::core::source::TagSource::LooseFolder { root, .. } = &source.source else { return None };
    let TagEntryLocation::LooseFile(path) = &entry.location else { return None };
    let relative = path.strip_prefix(root).ok()?.with_extension("");
    Some(tag_path::relative_tag_path(&relative))
}

thread_local! {
    /// Tags a menu read, by file, with the modification time they were read
    /// at. Reread when the file changes.
    static TAGS: RefCell<HashMap<PathBuf, (Option<std::time::SystemTime>, Option<Arc<blam_tags::TagFile>>)>> = RefCell::new(HashMap::new());
    /// Built menus, by (tag key, field path), with what they were built
    /// from.
    static MENUS: RefCell<HashMap<(String, String), MenuEntry>> = RefCell::new(HashMap::new());
}

struct MenuEntry {
    editor: String,
    stamp: (u64, u64),
    /// Whether the kit's reference index was there: a resource tag's
    /// scenario is found through it once the index is built.
    indexed: bool,
    menu: Arc<Menu>,
}

/// The kit's tags, opened read-only by path for the menus.
struct KitTags<'a> {
    source: &'a LoadedSourceData,
}

impl KitTags<'_> {
    fn file(&self, group: u32, path: &str) -> Option<PathBuf> {
        let crate::core::source::TagSource::LooseFolder { root, .. } = &self.source.source else { return None };
        let extension = self.source.names.name_for(group)?;
        Some(tag_path::resolve_tag_path(root, path.trim_end_matches('\0'), extension))
    }

    fn read(&self, file: &Path, group: u32) -> Option<Arc<blam_tags::TagFile>> {
        let crate::core::source::TagSource::LooseFolder { definitions_root, game, .. } = &self.source.source else { return None };
        let modified = std::fs::metadata(file).and_then(|metadata| metadata.modified()).ok();
        TAGS.with(|tags| {
            let mut tags = tags.borrow_mut();
            if let Some((when, tag)) = tags.get(file) {
                if *when == modified {
                    return tag.clone();
                }
            }
            let tag = modified
                .and_then(|_| crate::core::source::read_tag_at_path(file, *game, Some(definitions_root), group).ok())
                .map(Arc::new);
            tags.insert(file.to_path_buf(), (modified, tag.clone()));
            tag
        })
    }
}

impl blam_tags::string_editor::TagSource for KitTags<'_> {
    fn tag(&self, group: u32, path: &str) -> Option<Arc<blam_tags::TagFile>> {
        let file = self.file(group, path)?;
        self.read(&file, group)
    }

    /// The scenario that references the tag, from the kit's reference
    /// index; failing that, the one scenario beside it (or beside its
    /// `resources` folder), as a level folder lays them out.
    fn owning_scenario(&self, group: u32, path: &str) -> Option<Arc<blam_tags::TagFile>> {
        let scenario = u32::from_be_bytes(*b"scnr");
        if let Some(index) = self.source.reverse_dependencies.as_ref() {
            for key in index.dependents_for(group, path) {
                let Some(entry) = self.source.entry_for_key(key) else { continue };
                if let (true, TagEntryLocation::LooseFile(file)) = (entry.group_tag == scenario, &entry.location) {
                    if let Some(tag) = self.read(file, scenario) {
                        return Some(tag);
                    }
                }
            }
        }
        let file = self.file(group, path)?;
        let mut folder = file.parent()?.to_path_buf();
        if folder.file_name().is_some_and(|name| name.eq_ignore_ascii_case("resources")) {
            folder = folder.parent()?.to_path_buf();
        }
        let extension = self.source.names.name_for(scenario)?;
        let scenarios: Vec<PathBuf> = std::fs::read_dir(&folder)
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case(extension)))
            .collect();
        let stem = file.file_stem()?.to_string_lossy().to_ascii_lowercase();
        let chosen = match scenarios.as_slice() {
            [only] => only.clone(),
            many => many.iter().find(|path| path.file_stem().is_some_and(|s| stem.starts_with(&s.to_string_lossy().to_ascii_lowercase())))?.clone(),
        };
        self.read(&chosen, scenario)
    }
}

/// The menu of `editor` for the field at `path`, built when the tag or the
/// field's editor has changed since, or when `refresh` asks (the button was
/// pressed: Foundation rebuilds the menu on every click).
fn menu_for(edit: &FieldEditContext<'_>, editors: &StringEditors, editor: &str, path: &str, refresh: bool) -> Arc<Menu> {
    let Some(tag) = edit.string_editor_tag else { return Arc::new(Menu::default()) };
    let key = (edit.tag_key.to_owned(), path.to_owned());
    let indexed = tag.source.reverse_dependencies.is_some();
    if !refresh {
        let cached = MENUS.with(|menus| {
            menus.borrow().get(&key).filter(|entry| entry.stamp == tag.stamp && entry.editor == editor && entry.indexed == indexed).map(|entry| entry.menu.clone())
        });
        if let Some(menu) = cached {
            return menu;
        }
    }
    let tag_path = kit_relative_path(tag.source, tag.entry).unwrap_or_default();
    let ctx = EditorContext { tag: tag.tag, tag_path: &tag_path, field_path: path };
    let menu = Arc::new(editors.menu(editor, &ctx, &KitTags { source: tag.source }));
    MENUS.with(|menus| {
        menus.borrow_mut().insert(key, MenuEntry { editor: editor.to_owned(), stamp: tag.stamp, indexed, menu: menu.clone() });
    });
    menu
}

/// A string editor's row: the value's box (styled as an error while the value isn't one
/// the menu offers), the units, and the editor's menu button. Picking a
/// menu entry writes it to the field.
pub(in crate::app) fn draw_string_editor_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
    editors: &StringEditors,
    editor: &str,
) {
    if editors.is_string_reference(editor) {
        string_references::draw_row(ui, meta, value, suffix, depth, path, edit, editors, editor);
        return;
    }
    let menu = menu_for(edit, editors, editor, path, false);
    let caption = string_picker::caption(editor, editors.caption(editor));
    let search_hint = match editor {
        "object_function" | "object_function_no_own" | "object_function_panel" => "Search Functions".to_owned(),
        _ if caption == "Select String" => "Search Strings".to_owned(),
        _ => format!("Search {caption}"),
    };
    let enabled = edit.can_edit(meta);
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("text", &buffer_key));
    let menu_id = edit.widget_id(("string editor", &buffer_key));
    let draft = edit.buffers.draft_mut(&buffer_key, value);
    let valid = menu.accepts(&draft.text);
    let mut picked = None;
    let mut opened = false;
    let mut popup_drawn = false;
    let filter_id = menu_id.with("filter");
    let was_open_id = menu_id.with("was open");
    let grouping_id = egui::Id::new("model variant picker prefix grouping");
    let can_group = matches!(editor, "default_model_variant" | "object_variant" | "cinematic_object_variant");
    let mut grouped = can_group && ui.data(|data| data.get_temp::<bool>(grouping_id)).unwrap_or(false);
    let mut filter = ui.data(|data| data.get_temp::<String>(filter_id)).unwrap_or_default();
    draw_foundation_labelled_cell_row(ui, meta, "", depth, |ui, available_width| {
        let width = foundation_value_width(&draft.text, available_width).clamp(150.0, 300.0);
        let response = foundation_string_id_value_cell(ui, &mut draft.text, width, id, enabled, !valid);
        draft.note_response(&response);
        if enabled && draft.should_commit(ui, &response) {
            edit.pending.extend(field_edit_ops(path, &draft.text).pending);
        }
        draft.keep_commit(|| single_field_commit(edit.tag_key, &buffer_key, path));
        ui.scope_builder(egui::UiBuilder::new().id(menu_id), |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                let image = string_picker::image(ui, editor);
                crate::app::ui_kit::button_icons::text_dropdown_button(ui, &caption, image, |ui| {
                    popup_drawn = true;
                    let just_opened = !ui.data(|data| data.get_temp::<bool>(was_open_id)).unwrap_or(false);
                    if just_opened {
                        filter.clear();
                        opened = true;
                    }
                    let mut items = if grouped {
                        std::borrow::Cow::Owned(string_picker::prefix_groups(&menu.items))
                    } else {
                        std::borrow::Cow::Borrowed(menu.items.as_slice())
                    };
                    let width = string_picker::menu_popup_width(ui, &items, &filter);
                    string_picker::search(ui, &mut filter, filter_id, just_opened, width, &search_hint);
                    if can_group && ui.checkbox(&mut grouped, "Group by Prefix").changed() {
                        items = if grouped {
                            std::borrow::Cow::Owned(string_picker::prefix_groups(&menu.items))
                        } else {
                            std::borrow::Cow::Borrowed(menu.items.as_slice())
                        };
                    }
                    ui.separator();
                    string_picker::scroll(ui, menu_id.with(("choices", grouped))).show(ui, |ui| {
                        if menu.items.is_empty() {
                            ui.add_enabled(false, egui::Label::new(menu.note.as_deref().unwrap_or("No values are available.")));
                        } else if !string_picker::menu_has_matches(&items, &filter) {
                            ui.label("No matching strings.");
                        } else {
                            string_picker::menu_items(ui, &items, &filter, menu_id.with(grouped), &mut picked);
                        }
                    });
                });
            });
        });
        if !suffix.is_empty() { ui.label(RichText::new(suffix).color(subtle_dark()).small()); }
    });
    if let Some(text) = picked {
        draft.text = text.clone();
        edit.pending.extend(field_edit_ops(path, &text).pending);
    }
    ui.data_mut(|data| {
        data.insert_temp(filter_id, filter);
        data.insert_temp(was_open_id, popup_drawn);
        if can_group { data.insert_temp(grouping_id, grouped); }
    });
    if opened {
        menu_for(edit, editors, editor, path, true);
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::string_editor::MenuItem;
    use crate::app::loose_fixture::LooseKit;
    use blam_tags::{StringIdData, TagFile};

    /// Append one element per name to `block`, setting `label` (and
    /// `parent`, when given) on each.
    fn add_names(tag: &mut TagFile, block: &str, label: &str, parent: Option<&str>, rows: &[(&str, &str)]) {
        let mut root = tag.root_mut();
        let mut field = root.field_path_mut(block).expect("the block");
        let mut block = field.as_block_mut().expect("a block");
        for (index, (name, parent_name)) in rows.iter().enumerate() {
            block.add_element();
            let mut element = block.element_mut(index).expect("the element");
            element.field_path_mut(label).expect("the label").set(TagFieldData::StringId(StringIdData { string: (*name).to_owned() })).unwrap();
            if let Some(parent) = parent {
                element.field_path_mut(parent).expect("the parent").set(TagFieldData::StringId(StringIdData { string: (*parent_name).to_owned() })).unwrap();
            }
        }
    }

    /// An H3 kit with a globals tag of two materials, `metal_thin` under
    /// `metal`, and a model whose material names `value`.
    fn material_kit(value: &str) -> (LooseKit, std::path::PathBuf) {
        let kit = LooseKit::new("string_editor_material", "halo3_mcc");
        kit.write_mcc("globals/globals", "globals", |tag| {
            add_names(tag, "materials", "name", Some("parent name"), &[("metal", ""), ("metal_thin", "metal")]);
        });
        let model = kit.write_mcc("objects/thing/thing", "model", |tag| {
            add_names(tag, "materials", "global material name", None, &[(value, "")]);
        });
        (kit, model)
    }

    /// What the row for the field at `path` of the tag at `rel` paints (its
    /// texts, and whether its box is pink), and the menu it built.
    fn draw_field(kit: &LooseKit, rel: &str, group: &str, path: &str, indexed: bool) -> (Vec<String>, bool, Arc<Menu>) {
        let game = GameId::from_id(kit.game);
        let file = kit.root.join(rel);
        let tag = crate::core::source::read_tag_at_path(&file, game, Some(&locate_definitions_root()), crate::app::loose_fixture::group_tag(kit.game, group)).unwrap();
        let mut source = kit.source();
        if indexed {
            source.reverse_dependencies = Some(kit.index());
        }
        let key = kit.key(rel);
        let entry = source.entry_for_key(&key).expect("the tag's entry").clone();
        let definitions_root = locate_definitions_root();
        let mut menu = None;
        let output = crate::app::run_ui_test(&egui::Context::default(), egui::RawInput::default(), |ui| {
            let mut sinks = EditSinks::default();
            let mut edit = FieldEditContext::read_only(&mut sinks, "test", &key);
            edit.root = Some(tag.root());
            edit.game = game;
            edit.definitions_root = Some(&definitions_root);
            edit.editable = true;
            edit.string_editor_tag = Some(EditedTag { tag: &tag, entry: &entry, stamp: (1, 1), source: &source });
            let (editors, editor) = field_string_editor(&edit, path).expect("the field has an editor");
            let value = match tag.root().field_path(path).and_then(|field| field.value()) {
                Some(TagFieldData::StringId(id)) | Some(TagFieldData::OldStringId(id)) => id.string,
                _ => String::new(),
            };
            let meta = field_display_meta(path.rsplit('/').next().unwrap());
            draw_string_editor_row(ui, &meta, &value, "string id", 0, path, &mut edit, &editors, &editor);
            menu = Some(menu_for(&edit, &editors, &editor, path, false));
        });
        // Type labels follow picker controls, like tag-reference rows.
        let rendered: Vec<_> = output.shapes.iter().filter_map(|clipped| {
            if let egui::Shape::Text(text) = &clipped.shape { Some((text.galley.text(), text.pos.x, text.galley.size().x)) } else { None }
        }).collect();
        let type_x = rendered.iter().find(|(text, _, _)| *text == "string id").unwrap().1;
        if let Some((_, x, width)) = rendered.iter().find(|(text, _, _)| *text == "Materials") {
            assert!(*x + *width <= type_x, "picker must precede type label");
        }
        let mut texts = Vec::new();
        let mut pink = false;
        for clipped in &output.shapes {
            match &clipped.shape {
                egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                egui::Shape::Rect(rect) if rect.fill == invalid_value_fill() => pink = true,
                _ => {}
            }
        }
        (texts, pink, menu.unwrap())
    }

    fn draw(kit: &LooseKit, _model: &std::path::Path) -> (Vec<String>, bool, Option<Arc<Menu>>) {
        let (texts, pink, menu) = draw_field(kit, "objects/thing/thing.model", "model", "materials[0]/global material name", false);
        (texts, pink, Some(menu))
    }

    fn set_text(tag: &mut TagFile, path: &str, text: &str) {
        tag.root_mut().field_path_mut(path).unwrap_or_else(|| panic!("no {path}")).set(TagFieldData::StringId(StringIdData { string: text.to_owned() })).unwrap();
    }

    fn set_ref(tag: &mut TagFile, path: &str, group: u32, target: &str) {
        tag.root_mut()
            .field_path_mut(path)
            .unwrap_or_else(|| panic!("no {path}"))
            .set(TagFieldData::TagReference(blam_tags::TagReferenceData { group_tag_and_name: Some((group, target.to_owned())) }))
            .unwrap();
    }

    fn add(tag: &mut TagFile, block: &str) {
        tag.root_mut().field_path_mut(block).unwrap_or_else(|| panic!("no {block}")).as_block_mut().expect("a block").add_element();
    }

    /// A Reach kit with a location names tag (`area_a`, `area_b`), a
    /// scenario at `scenario` naming it, and a scenery resource in
    /// `levels\test\resources` whose multiplayer object's location is
    /// `value`. With `linked`, the scenario references the resource.
    fn location_kit(scenario: &str, value: &str, linked: bool) -> LooseKit {
        let kit = LooseKit::new("string_editor_locations", "haloreach_mcc");
        let locs = crate::app::loose_fixture::group_tag("haloreach_mcc", "location_name_globals_definition");
        let scenery = crate::app::loose_fixture::group_tag("haloreach_mcc", "scenario_scenery_resource");
        kit.write_mcc("levels/test/test_locations", "location_name_globals_definition", |tag| {
            add_names(tag, "location names", "name", None, &[("area_a", ""), ("area_b", "")]);
        });
        kit.write_mcc(scenario, "scenario", |tag| {
            set_ref(tag, "location name globals", locs, "levels\\test\\test_locations");
            if linked {
                add(tag, "scenario resources");
                add(tag, "scenario resources[0]/new split resources");
                set_ref(tag, "scenario resources[0]/new split resources[0]/scenery resource", scenery, "levels\\test\\resources\\test");
            }
        });
        kit.write_mcc("levels/test/resources/test", "scenario_scenery_resource", |tag| {
            add(tag, "scenerys");
            set_text(tag, "scenerys[0]/multiplayer data/location name", value);
        });
        kit
    }

    const RESOURCE_FIELD: &str = "scenerys[0]/multiplayer data/location name";

    /// A resource tag's location names are its scenario's: here the one in
    /// the level folder above `resources`, found with no reference index.
    #[test]
    fn a_resource_tag_lists_its_level_folders_scenarios_locations() {
        let kit = location_kit("levels/test/test", "area_b", false);
        let (_, pink, menu) = draw_field(&kit, "levels/test/resources/test.scenario_scenery_resource", "scenario_scenery_resource", RESOURCE_FIELD, false);
        assert_eq!(menu.picks(), vec!["area_a", "area_b"]);
        assert!(!pink, "area_b is a location");
        let kit = location_kit("levels/test/test", "nowhere", false);
        assert!(draw_field(&kit, "levels/test/resources/test.scenario_scenery_resource", "scenario_scenery_resource", RESOURCE_FIELD, false).1, "nowhere isn't one");
    }

    /// Through the reference index, the scenario that references the
    /// resource is found wherever it is; without the index, a scenario
    /// outside the level folder is not, and the menu is empty.
    #[test]
    fn a_resource_tag_finds_its_scenario_through_the_reference_index() {
        let kit = location_kit("levels/elsewhere/main", "area_a", true);
        let rel = "levels/test/resources/test.scenario_scenery_resource";
        let (_, _, menu) = draw_field(&kit, rel, "scenario_scenery_resource", RESOURCE_FIELD, true);
        assert_eq!(menu.picks(), vec!["area_a", "area_b"]);
        let (_, _, menu) = draw_field(&kit, rel, "scenario_scenery_resource", RESOURCE_FIELD, false);
        assert!(menu.picks().is_empty(), "no index and no scenario in the level folder: {:?}", menu.picks());
    }

    /// The material editor lists globals' materials as a tree, with a parent
    /// first in its own submenu, and its button uses the shared descriptive caption.
    #[test]
    fn material_menu_comes_from_globals() {
        let (kit, model) = material_kit("metal_thin");
        let (texts, pink, menu) = draw(&kit, &model);
        assert!(texts.iter().any(|text| text == "Materials"), "{texts:?}");
        assert!(!pink, "metal_thin is a material");
        let menu = menu.unwrap();
        assert_eq!(menu.items, vec![MenuItem::submenu("metal", vec![MenuItem::item("metal"), MenuItem::item("metal_thin")])]);
    }

    #[test]
    fn string_picker_material_search_expands_hierarchy_and_selects_parent_values() {
        use std::cell::Cell;
        let long_name = "metal_thin_reinforced";
        let (kit, model) = material_kit(long_name);
        kit.write_mcc("globals/globals", "globals", |tag| {
            add_names(tag, "materials", "name", Some("parent name"), &[("metal", ""), (long_name, "metal")]);
        });
        let defs = locate_definitions_root();
        let tag = crate::core::source::read_tag_at_path(
            &model, Some(GameId::Halo3), Some(&defs), u32::from_be_bytes(*b"hlmt"),
        ).unwrap();
        let source = kit.source();
        let key = kit.key("objects/thing/thing.model");
        let entry = source.entry_for_key(&key).unwrap();
        let ctx = egui::Context::default();
        Baboon::configure_context(&ctx);
        let mut sinks = EditSinks::default();
        let mut time = 0.0;
        let filter_id = Cell::new(None);
        let path = "materials[0]/global material name";
        let mut frame = |events| {
            time += 0.2;
            let mut pending = Vec::new();
            let output = crate::app::run_ui_test(&ctx, crate::app::loose_fixture::screen(events, time), |ui| {
                let mut edit = FieldEditContext::read_only(&mut sinks, "material-search", &key);
                edit.root = Some(tag.root());
                edit.game = Some(GameId::Halo3);
                edit.definitions_root = Some(&defs);
                edit.editable = true;
                edit.string_editor_tag = Some(EditedTag { tag: &tag, entry, stamp: (1, 0), source: &source });
                filter_id.set(Some(edit.widget_id(("string editor", format!("{key}|{path}"))).with("filter")));
                let field = tag.root().field_path(path).unwrap();
                draw_foundation_value_row(ui, field, &field_display_meta("global material name"),
                    "string id", &field.value().unwrap(), &source.names, 0, path, &mut edit, None, 300.0);
                pending = std::mem::take(edit.pending);
            });
            (output, pending)
        };
        let rect = |output: &egui::FullOutput, text: &str| {
            output.shapes.iter().rev().find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
                _ => None,
            })
        };
        let click = |pos| vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
        ];
        let (output, _) = frame(vec![]);
        frame(click(rect(&output, "Materials").unwrap().center()));
        frame(vec![]);
        let (output, _) = frame(vec![]);
        assert!(rect(&output, "Search Materials").is_some());
        assert!(rect(&output, "Search String IDs or Text").is_none());
        assert!(rect(&output, "metal (2)").is_some());
        assert!(rect(&output, "metal").is_none(), "browse leaves stay inside their submenu");
        let id = filter_id.get().unwrap();
        let search = ctx.read_response(id).unwrap().rect;
        assert!((search.height() - BUTTON_HEIGHT).abs() < 1.0);
        assert!((search.width() - string_picker::POPUP_MIN_WIDTH).abs() < 1.0);
        let popup_size = || ctx.memory(|memory| memory.areas().visible_layer_ids().into_iter()
            .filter(|layer| layer.order == egui::Order::Foreground)
            .find_map(|layer| memory.area_rect(layer.id).map(|rect| rect.size())).unwrap());
        let original_size = popup_size();
        ctx.memory_mut(|memory| memory.request_focus(id));
        frame(vec![egui::Event::Text("METAL_THIN".into())]);
        let (output, _) = frame(vec![]);
        let narrow_size = popup_size();
        assert!((narrow_size.x - original_size.x).abs() < 2.0);
        assert!(narrow_size.y > original_size.y,
            "searching exposes the matching group and its entry inline");
        assert!(rect(&output, "metal (1)").is_some());
        assert!(rect(&output, long_name).is_some());
        assert!(rect(&output, "metal").is_none());
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0), egui::text::CCursor::new(10),
        )));
        state.store(&ctx, id);
        frame(vec![egui::Event::Text("metal".into())]);
        let (output, _) = frame(vec![]);
        assert!(popup_size().y > narrow_size.y,
            "broadening a search grows to fit the additional matching entry");
        assert!(rect(&output, "metal (2)").is_some());
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0), egui::text::CCursor::new(5),
        )));
        state.store(&ctx, id);
        frame(vec![egui::Event::Key { key: egui::Key::Backspace, physical_key: None,
            pressed: true, repeat: false, modifiers: Default::default() }]);
        let (output, _) = frame(vec![]);
        assert!((popup_size() - original_size).length() < 2.0,
            "clearing search restores the original browsing size");
        frame(click(rect(&output, "metal (2)").unwrap().center()));
        frame(vec![]);
        let (output, _) = frame(vec![]);
        let widths = ctx.memory(|memory| memory.areas().visible_layer_ids().into_iter()
            .filter(|layer| layer.order == egui::Order::Foreground)
            .filter_map(|layer| memory.area_rect(layer.id).map(|rect| rect.width())).collect::<Vec<_>>());
        assert!(widths.len() >= 2 && widths.iter().any(|width| *width < 180.0),
            "short material names should produce a compact submenu: {widths:?}");
        for label in ["metal", long_name] {
            let painted = output.shapes.iter().rev().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some((text, shape.clip_rect)),
                _ => None,
            }).unwrap();
            let text_rect = egui::Rect::from_min_size(painted.0.pos, painted.0.galley.size());
            assert!(!painted.0.galley.elided && painted.1.contains_rect(text_rect),
                "submenu must fit {label}: elided={}, text={text_rect:?}, clip={:?}, widths={widths:?}",
                painted.0.galley.elided, painted.1);
        }
        let parent_rect = rect(&output, "metal").unwrap();
        let (_, pending) = frame(click(parent_rect.center()));
        assert!(pending.iter().any(|edit| edit.path == path && edit.input == "metal"),
            "a hierarchical parent remains selectable as its own leaf");
    }

    /// The object field is inherited through different depths (unit/object,
    /// item/object, and object). Every model-based game's definitions should
    /// supply the same picker, using the referenced model rather than self.
    #[test]
    fn default_model_variant_lookup_follows_inherited_model_references() {
        struct ModelSource(Arc<TagFile>);
        impl blam_tags::string_editor::TagSource for ModelSource {
            fn tag(&self, group: u32, path: &str) -> Option<Arc<TagFile>> {
                (group == self.0.group().tag && path == "objects\\test\\model")
                    .then(|| self.0.clone())
            }
        }
        let definitions = locate_definitions_root();
        for game in [GameId::Halo2, GameId::Halo2Amp, GameId::Halo3,
            GameId::Halo3Odst, GameId::HaloReach, GameId::Halo4, GameId::CampaignEvolved]
        {
            let root = definitions.join(game.as_str());
            let editors = StringEditors::load(&root).unwrap();
            let mut model = TagFile::new(root.join("model.json")).unwrap();
            add_names(&mut model, "variants", "name", None, &[("default", ""), ("armored", "")]);
            let source = ModelSource(Arc::new(model));
            for (group, prefix) in [("biped", "unit/object/"), ("vehicle", "unit/object/"),
                ("weapon", "item/object/"), ("scenery", "object/")]
            {
                let mut tag = TagFile::new(root.join(format!("{group}.json"))).unwrap();
                let path = format!("{prefix}default model variant");
                let editor = editors.editor_at(tag.root(), &path).unwrap_or_else(|| panic!("{game:?} {group}"));
                assert_eq!(editor, "default_model_variant");
                let menu = editors.menu(editor, &EditorContext { tag: &tag, tag_path: "objects\\test\\object", field_path: &path }, &source);
                assert!(menu.items.is_empty(), "an empty reference must not guess a model");
                assert!(menu.accepts("unresolved"));
                set_ref(&mut tag, &format!("{prefix}model"), source.0.group().tag, "objects\\test\\model");
                let menu = editors.menu(editor, &EditorContext { tag: &tag, tag_path: "objects\\test\\object", field_path: &path }, &source);
                assert_eq!(menu.picks(), ["default", "armored"], "{game:?} {group}");
                assert!(menu.accepts("ARMORED"));
                assert!(menu.accepts(""), "NONE remains valid");
                assert!(!menu.accepts("missing_variant"));
                set_ref(&mut tag, &format!("{prefix}model"), source.0.group().tag, "objects\\missing");
                let menu = editors.menu(editor, &EditorContext { tag: &tag, tag_path: "objects\\test\\object", field_path: &path }, &source);
                assert!(menu.items.is_empty());
                assert!(menu.accepts("unresolved"), "missing models cannot validate a name");
            }
        }
    }

    fn variant_kit(value: &str) -> LooseKit {
        let kit = LooseKit::new("default_model_variant", "halo3_mcc");
        kit.write_mcc("objects/test/model", "model", |tag| {
            add_names(tag, "variants", "name", None, &[("default", ""), ("armored", "")]);
        });
        kit.write_mcc("objects/test/unit", "biped", |tag| {
            set_ref(tag, "unit/object/model", u32::from_be_bytes(*b"hlmt"), "objects\\test\\model");
            set_text(tag, "unit/object/default model variant", value);
        });
        kit
    }

    #[test]
    fn default_model_variant_uses_shared_validation_style() {
        for (value, invalid) in [("ARMORED", false), ("", false), ("missing_variant", true)] {
            let kit = variant_kit(value);
            let (texts, pink, menu) = draw_field(&kit, "objects/test/unit.biped", "biped",
                "unit/object/default model variant", false);
            assert!(texts.iter().any(|text| text == "Model Variants"));
            assert_eq!(menu.picks(), ["default", "armored"]);
            assert_eq!(pink, invalid, "{value}");
        }
    }

    #[test]
    fn default_model_variant_shared_picker_searches_and_commits_selection() {
        let kit = variant_kit("");
        kit.write_mcc("objects/test/model", "model", |tag| {
            add_names(tag, "variants", "name", None, &[("default", ""), ("armored", ""),
                ("minor_scl", ""), ("minor_dog", ""), ("major_scl", ""), ("major_dog", "")]);
        });
        let definitions = locate_definitions_root();
        let tag = crate::core::source::read_tag_at_path(&kit.path("objects/test/unit.biped"),
            Some(GameId::Halo3), Some(&definitions), u32::from_be_bytes(*b"bipd")).unwrap();
        let source = kit.source();
        let key = kit.key("objects/test/unit.biped");
        let entry = source.entry_for_key(&key).unwrap();
        let ctx = egui::Context::default();
        Baboon::configure_context(&ctx);
        let mut sinks = EditSinks::default();
        let mut time = 0.0;
        let filter_id = std::cell::Cell::new(None);
        let path = "unit/object/default model variant";
        let mut frame = |events| {
            time += 0.2;
            let mut pending = Vec::new();
            let output = crate::app::run_ui_test(&ctx, crate::app::loose_fixture::screen(events, time), |ui| {
                let mut edit = FieldEditContext::read_only(&mut sinks, "variant-search", &key);
                edit.root = Some(tag.root());
                edit.game = Some(GameId::Halo3);
                edit.definitions_root = Some(&definitions);
                edit.editable = true;
                edit.string_editor_tag = Some(EditedTag { tag: &tag, entry, stamp: (1, 0), source: &source });
                filter_id.set(Some(edit.widget_id(("string editor", format!("{key}|{path}"))).with("filter")));
                let field = tag.root().field_path(path).unwrap();
                draw_foundation_value_row(ui, field, &field_display_meta("default model variant"),
                    "string id", &field.value().unwrap(), &source.names, 0, path, &mut edit, None, 300.0);
                pending = std::mem::take(edit.pending);
            });
            (output, pending)
        };
        let rect = |output: &egui::FullOutput, text: &str| {
            output.shapes.iter().rev().find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
                _ => None,
            })
        };
        let click = |pos| vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
        ];
        let (output, _) = frame(vec![]);
        frame(click(rect(&output, "Model Variants").unwrap().center()));
        frame(vec![]);
        let (output, _) = frame(vec![]);
        assert!(rect(&output, "Search Model Variants").is_some());
        assert!(rect(&output, "default").is_some());
        assert!(rect(&output, "armored").is_some());
        assert!(rect(&output, "minor_scl").is_some(), "grouping starts off");
        assert!(rect(&output, "minor (2)").is_none());
        let (_, pending) = frame(click(rect(&output, "Group by Prefix").unwrap().center()));
        assert!(pending.is_empty(), "grouping must not edit the tag");
        let (output, _) = frame(vec![]);
        assert!(rect(&output, "minor (2)").is_some());
        assert!(rect(&output, "major (2)").is_some());
        assert!(rect(&output, "minor_scl").is_none(), "browse children stay in their submenu");
        assert!(rect(&output, "armored").is_some(), "singletons remain selectable");
        frame(click(rect(&output, "Group by Prefix").unwrap().center()));
        let (output, _) = frame(vec![]);
        assert!(rect(&output, "minor_scl").is_some(), "grouping can be disabled again");
        assert!(rect(&output, "minor (2)").is_none());
        frame(click(rect(&output, "Group by Prefix").unwrap().center()));
        frame(vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None,
            pressed: true, repeat: false, modifiers: Default::default() }]);
        frame(vec![]);
        let (output, _) = frame(vec![]);
        frame(click(rect(&output, "Model Variants").unwrap().center()));
        frame(vec![]);
        let (output, _) = frame(vec![]);
        assert!(rect(&output, "minor (2)").is_some(), "grouping survives closing and reopening");
        ctx.memory_mut(|memory| memory.request_focus(filter_id.get().unwrap()));
        frame(vec![egui::Event::Text("DOG".into())]);
        let (output, _) = frame(vec![]);
        assert!(rect(&output, "default").is_none());
        assert!(rect(&output, "minor_scl").is_none());
        assert!(rect(&output, "minor (1)").is_some(), "search retains matching group headings");
        assert!(rect(&output, "major (1)").is_some());
        let (_, pending) = frame(click(rect(&output, "minor_dog").unwrap().center()));
        assert!(pending.iter().any(|edit| edit.path == path && edit.input == "minor_dog"),
            "grouped selection writes the complete variant ID");
    }

    /// A name globals doesn't have turns the box pink; case doesn't count.
    #[test]
    fn a_name_the_menu_lacks_is_pink() {
        let (kit, model) = material_kit("stone");
        assert!(draw(&kit, &model).1, "stone isn't a material");
        let (kit, model) = material_kit("METAL");
        assert!(!draw(&kit, &model).1, "case is ignored");
    }
}

/// Reuse the string-menu tag cache for HUD usage classification.
pub(in crate::app) fn hud_reference_tag(source: &LoadedSourceData, group: u32, path: &str) -> Option<Arc<blam_tags::TagFile>> {
    blam_tags::string_editor::TagSource::tag(&KitTags { source }, group, path)
}
