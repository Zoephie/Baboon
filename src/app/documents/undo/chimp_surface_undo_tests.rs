//! Undo and redo on the Chimp surface.
//!
//! Ctrl+Z, Ctrl+Y and the Edit menu act on the selected tag. On the Chimp
//! surface that tag is hidden, so they used to change it without the user
//! seeing anything happen. Chimp has no undo yet, so there they do nothing.

use super::*;

const KEY: &str = "ublock:pakchunk0:objects/vehicles/warthog";

fn app_with_undoable_tag() -> Baboon {
    let definition = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("definitions")
        .join("haloce_evolved")
        .join("cinematic_scene.json");
    let tag = TagFile::new(definition).expect("build a tag from the CE schema");
    let mut document = TagDocument::modified(tag);
    document.journal.begin_edit(&document.tag, "Edit");
    document.journal.end_edit_window();
    let mut app = Baboon::for_test();
    app.model.prefs.enable_chimp = true;
    app.model.kits[0].parsed_tags.insert(KEY.to_owned(), document);
    app.model.kits[0].selected_key = Some(KEY.to_owned());
    app
}

fn can_undo_tag(app: &Baboon) -> bool {
    app.model.kits[0].parsed_tags[KEY].journal.can_undo()
}

#[test]
fn undo_on_the_chimp_surface_leaves_the_hidden_tag_alone() {
    let mut app = app_with_undoable_tag();
    app.model.kits[0].surface = KitSurface::Chimp;

    assert!(!app.can_undo_current(), "the Edit menu's Undo is disabled");
    app.undo_current_tag();
    assert!(can_undo_tag(&app), "the hidden tag's history is untouched");
    app.redo_current_tag();
    assert!(!app.model.kits[0].parsed_tags[KEY].journal.can_redo());

    // Back on the tag surface, the same undo acts on the tag.
    app.model.kits[0].surface = KitSurface::Tags;
    assert!(app.can_undo_current());
    app.undo_current_tag();
    assert!(!can_undo_tag(&app));
    assert!(app.model.kits[0].parsed_tags[KEY].journal.can_redo());
}
