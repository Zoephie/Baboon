use super::*;

fn source(source: TagSource, entries: Vec<TagEntry>) -> LoadedSourceData {
    LoadedSourceData {
        label: "test".to_owned(),
        source,
        names: TagNameIndex::default(),
        game: None,
        entries,
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    }
}

fn sound() -> TagEntry {
    TagEntry {
        key: "file:/kit/tags/a.sound".to_owned(),
        display_path: "a.sound".to_owned(),
        group_tag: u32::from_be_bytes(*b"snd!"),
        group_name: Some("sound".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from("/kit/tags/a.sound")),
    }
}

/// A source that lists every tag up front keeps them in `entries`, with
/// `all_entries` empty. The whole-source listings read `all_entries`, so
/// on such a source they walked nothing and reported "none found".
#[test]
fn whole_source_listings_see_a_source_listed_up_front() {
    let mut app = Baboon::for_test();
    app.install_loaded_source(source(
        TagSource::SingleFile {
            path: PathBuf::from("/kit/tags/a.sound"),
        },
        vec![sound()],
    ));
    assert_eq!(app.model.listing_entries().map(<[TagEntry]>::len), Ok(1));
}

/// A listing reads its tags on a worker: the window says it is reading,
/// and the listing arrives with the worker's message.
#[test]
fn a_source_listing_is_read_off_the_ui_thread() {
    let mut app = Baboon::for_test();
    app.install_loaded_source(source(
        TagSource::SingleFile {
            path: PathBuf::from("/kit/tags/a.sound"),
        },
        vec![sound()],
    ));
    let ctx = egui::Context::default();

    app.show_sounds_by_class(&ctx);
    let waiting = app
        .dialogs
        .get::<QueryResultsWindow>()
        .and_then(|window| window.results.note.clone());
    let message = app
        .rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the listing worker answers");
    app.apply_worker_message(message, &ctx);
    app.process_worker_messages(&ctx);

    assert_eq!(waiting.as_deref(), Some("Reading 1 tag(s)…"));
    let results = app
        .dialogs
        .close::<QueryResultsWindow>()
        .expect("results")
        .results;
    assert_eq!(
        results.title, "Sounds by class (0)",
        "the one sound is unreadable"
    );
    assert_eq!(results.note.as_deref(), Some("No sound tags found."));
}

/// A loose folder mid-scan has only the folders browsed so far. Saying
/// "none found" from that is wrong; saying the index is not ready is not.
#[test]
fn whole_source_listings_wait_for_a_loose_folder_scan() {
    let mut app = Baboon::for_test();
    app.install_loaded_source(source(
        TagSource::LooseFolder {
            root: PathBuf::from("/kit/tags"),
            game: None,
            definitions_root: PathBuf::new(),
        },
        vec![sound()],
    ));
    let Err(error) = app.model.listing_entries() else {
        panic!("a loose folder mid-scan must not be listed");
    };
    assert!(error.contains("still being built"), "{error}");
}
