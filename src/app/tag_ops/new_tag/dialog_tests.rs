//! The New Tag dialog through its Create: the create takes the dialog back
//! from the host, and returns it — with the reason — only when no tag was
//! made.

use super::*;

/// A loose Halo 3 editing kit in a fresh temporary folder, holding nothing.
fn loose_app() -> (Baboon, PathBuf) {
    let root = std::env::temp_dir()
        .join(format!("baboon-new-tag-{}", uuid::Uuid::new_v4()))
        .join("tags");
    fs::create_dir_all(&root).unwrap();
    let mut app = Baboon::for_test();
    let source = crate::core::source::load_editing_kit_layout(
        root.clone(),
        "New Tag Kit".to_owned(),
        GameId::Halo3,
        &app.model.default_names,
        &crate::core::bundled::locate_definitions_root(),
    )
    .expect("an empty loose kit loads");
    app.install_loaded_source(source);
    (app, root)
}

/// With nowhere to write, Create leaves the dialog open and says why.
#[test]
fn a_refused_create_keeps_the_dialog_with_its_reason() {
    let mut app = Baboon::for_test();
    app.open_new_tag_dialog();
    app.create_new_tag();
    let dialog = app.dialogs.get::<NewTagDialog>().expect("still open");
    assert_eq!(
        dialog.error.as_deref(),
        Some("Load a loose editing-kit tags folder before creating a tag")
    );
}

/// A tag that can be made is written, and the dialog closes.
#[test]
fn a_created_tag_closes_the_dialog() {
    let (mut app, root) = loose_app();
    app.open_new_tag_dialog();
    let output = root.join("objects/new/new.scenery");
    {
        let dialog = app.dialogs.get_mut::<NewTagDialog>().expect("open");
        dialog.selected_group = dialog
            .groups
            .iter()
            .position(|group| group.name == "scenery")
            .expect("Halo 3 defines scenery");
        dialog.output_path = Some(output.clone());
    }
    app.create_new_tag();
    let created = output.exists();
    let _ = fs::remove_dir_all(root.parent().unwrap());
    assert!(created, "{}", app.model.status);
    assert!(app.dialogs.get::<NewTagDialog>().is_none());
    assert_eq!(app.model.status, format!("Created {}", output.display()));
}
