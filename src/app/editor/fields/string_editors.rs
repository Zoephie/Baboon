//! The kit tools' string editors: a menu of the values a `string_id` field
//! expects, on a button beside its box, and the box pink while its value
//! isn't one of them. Drawn as Foundation's `TagFieldElementWithMenuPanel`
//! draws them: the box, the units, then the button. blam-tags'
//! `string_editor` builds the menus; this opens the other tags they read
//! (the globals tag, a referenced object, the scenario a resource tag belongs
//! to) and caches what it built.

use super::*;
use crate::core::source::{LoadedSourceData, TagEntry, TagEntryLocation};
use blam_tags::string_editor::{Context as EditorContext, Menu, MenuItem, StringEditors};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Foundation's fill for a value the editor doesn't offer (`#FFC0CB`).
pub(in crate::app) const INVALID_VALUE_FILL: Color32 = Color32::from_rgb(0xFF, 0xC0, 0xCB);

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
    let (root, game) = (definitions_root?, game?);
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
    Some(relative.to_string_lossy().replace('/', "\\"))
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
        let relative = path.trim_end_matches('\0').replace('\\', "/");
        Some(root.join(format!("{relative}.{extension}")))
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

/// A string editor's row: the value's box (pink while the value isn't one
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
    let menu = menu_for(edit, editors, editor, path, false);
    let caption = editors.caption(editor).to_owned();
    let enabled = edit.can_edit(meta);
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("text", &buffer_key));
    let menu_id = edit.widget_id(("string editor", &buffer_key));
    let draft = edit.buffers.draft_mut(&buffer_key, value);
    let valid = menu.accepts(&draft.text);
    let mut picked = None;
    let mut opened = false;
    draw_foundation_labelled_cell_row(ui, meta, "", depth, |ui, available_width| {
        let width = foundation_value_width(&draft.text, available_width).clamp(150.0, 300.0);
        let response = foundation_value_cell_filled(ui, &mut draft.text, width, id, enabled, (!valid).then_some(INVALID_VALUE_FILL));
        draft.note_response(&response);
        if enabled && draft.should_commit(ui, &response) {
            edit.pending.extend(field_edit_ops(path, &draft.text).pending);
        }
        draft.keep_commit(|| single_field_commit(edit.tag_key, &buffer_key, path));
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        ui.push_id(menu_id, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                let (response, _) = egui::containers::menu::MenuButton::new(caption.as_str())
                    .config(crate::app::ui_kit::button_icons::menu_config())
                    .ui(ui, |ui| {
                        // Foundation's "No object functions available.": a
                        // disabled entry in place of an empty menu.
                        if let (true, Some(note)) = (menu.items.is_empty(), &menu.note) {
                            ui.add_enabled(false, egui::Button::new(note.as_str()));
                        }
                        menu_items(ui, &menu.items, &mut picked)
                    });
                opened = response.clicked();
            });
        });
    });
    if let Some(text) = picked {
        draft.text = text.clone();
        edit.pending.extend(field_edit_ops(path, &text).pending);
    }
    if opened {
        menu_for(edit, editors, editor, path, true);
    }
}

/// One level of a menu: items to pick, submenus to open. A long level
/// scrolls, as Foundation's virtualized menus do.
fn menu_items(ui: &mut Ui, items: &[MenuItem], picked: &mut Option<String>) {
    let max_height = (ui.ctx().content_rect().height() * 0.7).max(200.0);
    egui::ScrollArea::vertical().max_height(max_height).auto_shrink([true, true]).show(ui, |ui| {
        for item in items {
            if item.is_submenu() {
                ui.menu_button(item.text.as_str(), |ui| menu_items(ui, &item.children, picked));
            } else if ui.button(item.text.as_str()).clicked() {
                *picked = Some(item.text.clone());
                crate::app::ui_kit::button_icons::close_menu(ui);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
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
            draw_string_editor_row(ui, &meta, &value, "", 0, path, &mut edit, &editors, &editor);
            menu = Some(menu_for(&edit, &editors, &editor, path, false));
        });
        let mut texts = Vec::new();
        let mut pink = false;
        for clipped in &output.shapes {
            match &clipped.shape {
                egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                egui::Shape::Rect(rect) if rect.fill == INVALID_VALUE_FILL => pink = true,
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
    /// first in its own submenu, and its button says what Foundation's says.
    #[test]
    fn material_menu_comes_from_globals() {
        let (kit, model) = material_kit("metal_thin");
        let (texts, pink, menu) = draw(&kit, &model);
        assert!(texts.iter().any(|text| text == "Choose Material"), "{texts:?}");
        assert!(!pink, "metal_thin is a material");
        let menu = menu.unwrap();
        assert_eq!(menu.items, vec![MenuItem::submenu("metal", vec![MenuItem::item("metal"), MenuItem::item("metal_thin")])]);
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
