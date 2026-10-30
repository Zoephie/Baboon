//! Coverage for fields verified against the installed H2/H3/ODST/Reach tags.
use super::*;
use blam_tags::string_editor::{StringReferenceValidation, TagSource};
use blam_tags::{TagFile, TagFieldData, TagReferenceData};

struct Tags(HashMap<(u32, String), Arc<TagFile>>);
impl Tags {
    fn insert(&mut self, path: &str, tag: TagFile) {
        self.0.insert((tag.group().tag, path.into()), Arc::new(tag));
    }
}
impl TagSource for Tags {
    fn tag(&self, group: u32, path: &str) -> Option<Arc<TagFile>> {
        self.0.get(&(group, path.into())).cloned()
    }
}
fn new_tag(game: GameId, group: &str) -> TagFile {
    TagFile::new(crate::core::bundled::locate_definitions_root().join(game.as_str()).join(format!("{group}.json"))).unwrap()
}
fn add(tag: &mut TagFile, path: &str) {
    tag.root_mut().field_path_mut(path).unwrap().as_block_mut().unwrap().add_element();
}
fn reference(tag: &mut TagFile, path: &str, target: &TagFile, name: &str) {
    tag.root_mut().field_path_mut(path).unwrap().set(TagFieldData::TagReference(TagReferenceData {
        group_tag_and_name: Some((target.group().tag, name.into())),
    })).unwrap();
}
fn strings(game: GameId, id: &str) -> TagFile {
    use blam_tags::string_list::{self, StringListEdit};
    let mut tag = new_tag(game, "multilingual_unicode_string_list");
    let list = string_list::read_string_list(&tag).unwrap();
    let mut row = list.new_entry();
    row.id = Some(id.into());
    row.texts[0] = Some("HUD lookup coverage".into());
    string_list::apply_string_list_edits(&mut tag, Some(game), &[StringListEdit::Insert { at: 0, entries: vec![row] }]).unwrap();
    tag
}
fn fixture(game: GameId) -> (StringEditors, Tags, TagFile, String) {
    let editors = StringEditors::load(crate::core::bundled::locate_definitions_root().join(game.as_str())).unwrap();
    let mut tags = Tags(HashMap::new());
    let messages = strings(game, "hud_only");
    let mut globals = new_tag(game, "globals");
    add(&mut globals, "interface tags");
    let h2 = game == GameId::Halo2;
    let mut hud = new_tag(game, if h2 { "hud_globals" } else { "chud_globals_definition" });
    let training = if h2 {
        reference(&mut globals, "interface tags[0]/hud globals", &hud, "ui\\custom_hud");
        reference(&mut hud, "new globals/hud text", &messages, "ui\\custom_messages");
        "new globals/player training data"
    } else {
        reference(&mut globals, "interface tags[0]/chud globals", &hud, "ui\\custom_hud");
        add(&mut hud, "skins");
        reference(&mut hud, "skins[0]/chud text", &messages, "ui\\custom_messages");
        "player training data"
    };
    add(&mut hud, training);
    tags.insert("globals\\globals", globals);
    tags.insert("ui\\custom_hud", hud);
    tags.insert("ui\\custom_messages", messages);
    let mut training_tag = new_tag(game, if h2 { "hud_globals" } else { "chud_globals_definition" });
    add(&mut training_tag, training);
    (editors, tags, training_tag, format!("{training}[0]"))
}
fn check(editors: &StringEditors, tags: &Tags, tag: &TagFile, path: &str) {
    assert_eq!(editors.editor_at(tag.root(), path), Some("hud_message"), "{path}");
    let list = editors.string_references("hud_message", &EditorContext {
        tag, tag_path: "objects\\test", field_path: path,
    }, tags).unwrap();
    assert_eq!(list.unavailable, None);
    assert_eq!(list.source.as_ref().unwrap().path, "ui\\custom_messages");
    assert_eq!(list.entries[0].preview.as_deref(), Some("HUD lookup coverage"));
    assert_eq!(list.validate("hud_only"), StringReferenceValidation::Valid);
    assert_eq!(list.validate("page_only"), StringReferenceValidation::Missing);
    assert_eq!(list.validate(""), StringReferenceValidation::Valid);
}

#[test]
fn hud_lookup_coverage_uses_the_referenced_hud_list_for_verified_fields() {
    for game in [GameId::Halo2, GameId::Halo3, GameId::Halo3Odst, GameId::HaloReach] {
        let (editors, tags, training, path) = fixture(game);
        check(&editors, &tags, &training, &format!("{path}/display string"));
        if game == GameId::Halo2 {
            for field in ["display string2", "display string3"] {
                check(&editors, &tags, &training, &format!("{path}/{field}"));
            }
        }
        for (group, field) in [("vehicle", "flip message"), ("device_control", "action string")] {
            check(&editors, &tags, &new_tag(game, group), field);
        }
        if game != GameId::Halo2 {
            check(&editors, &tags, &new_tag(game, "device_terminal"), "action string");
        }
        if game == GameId::HaloReach {
            check(&editors, &tags, &new_tag(game, "biped"), "assassination chud text");
        }
    }
}

#[test]
fn hud_lookup_coverage_does_not_treat_terminal_page_text_as_a_hud_message() {
    let game = GameId::HaloReach;
    let (editors, mut tags, _, _) = fixture(game);
    let mut terminal = new_tag(game, "device_terminal");
    let pages = strings(game, "page_only");
    reference(&mut terminal, "strings", &pages, "ui\\terminal_pages");
    tags.insert("ui\\terminal_pages", pages);
    add(&mut terminal, "pages");
    check(&editors, &tags, &terminal, "action string");
    for field in ["name", "pages[0]/text"] {
        assert_ne!(editors.editor_at(terminal.root(), field), Some("hud_message"));
    }
}
