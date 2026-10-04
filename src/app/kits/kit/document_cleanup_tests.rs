use super::*;
use crate::app::kits::{KitMut, KitView};

/// Two sources loaded one after another into the same kit must not share a
/// generation, or a job stamped against the first resolves against the
/// second.
#[test]
fn a_second_source_in_a_kit_never_reuses_a_generation() {
    let mut app = crate::app::Baboon::for_test();
    let source = |label: &str| LoadedSourceData {
        label: label.to_owned(),
        source: TagSource::LooseFolder {
            root: PathBuf::from(format!("/{label}")),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game: None,
        entries: Vec::new(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    };
    let mut seen = Vec::new();
    let mut stamps = Vec::new();
    for label in ["first", "second", "third"] {
        app.install_loaded_source(source(label));
        // The load handler moves the generation on once more after
        // installing; mirror it so the test sees what jobs see.
        app.model.kits[app.model.active].generation = app.model.kits[app.model.active].generation.wrapping_add(1);
        seen.push(app.model.kits[app.model.active].generation);
        stamps.push(app.model.kit_stamp());
    }
    let mut unique = seen.clone();
    unique.dedup();
    assert_eq!(unique, seen, "generations {seen:?} repeat");
    assert!(app.model.resolve_stamp(stamps[0]).is_none(), "a stale stamp is refused");
    assert!(app.model.resolve_stamp(stamps[2]).is_some());
}

/// Closing tabs drops every cache kept for them, model previews included.
/// Three of the four close paths kept the model preview (its geometry and
/// textures) for the rest of the session.
#[test]
fn closing_tabs_drops_everything_kept_for_them() {
    let mut kit = Kit::empty(KitId(0), TagNameIndex::default());
    let mut view = KitView::for_test(&kit);
    for key in ["kept", "closed"] {
        view.caches.model_previews
            .insert(key.to_owned(), ModelPreviewState::default());
        view.caches.bitmap_previews
            .insert(key.to_owned(), BitmapPreviewState::default());
        kit.loading_tags.insert(key.to_owned());
        view.edit_buffers
            .insert_clean(format!("{key}|name"), "x".to_owned());
    }

    KitMut::new(&mut kit, &mut view).drop_documents_except(Some("kept"));
    assert_eq!(view.caches.model_previews.keys().collect::<Vec<_>>(), ["kept"]);
    assert_eq!(view.caches.bitmap_previews.keys().collect::<Vec<_>>(), ["kept"]);
    assert_eq!(kit.loading_tags.iter().collect::<Vec<_>>(), ["kept"]);

    KitMut::new(&mut kit, &mut view).drop_document("kept");
    assert!(view.caches.model_previews.is_empty());
    assert!(view.caches.bitmap_previews.is_empty());
    assert!(kit.loading_tags.is_empty());
}
