//! One grid, two libraries: each lists only its own kind of tag, searches
//! within it, and queues thumbnail jobs only for what it shows.

use super::*;

fn entry(display_path: &str, group: &[u8; 4]) -> TagEntry {
    TagEntry {
        key: format!("file:{display_path}"),
        display_path: display_path.to_owned(),
        group_tag: u32::from_be_bytes(*group),
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
    }
}

/// A kit holding bitmaps, render models, a gbxmodel, and tags of neither kind.
fn app_with_mixed_kit() -> Baboon {
    let mut entries = vec![
        entry("objects/warthog/warthog.render_model", b"mode"),
        entry("objects/ghost/ghost.gbxmodel", b"mod2"),
        entry("objects/warthog/warthog.model", b"hlmt"),
        entry("objects/warthog/warthog.vehicle", b"vehi"),
    ];
    for index in 0..40 {
        entries.push(entry(&format!("textures/grass_{index:02}.bitmap"), b"bitm"));
    }
    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "mixed kit".to_owned(),
        source: TagSource::LooseFolder {
            root: PathBuf::from("<no kit root>"),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game: None,
        entries: entries.clone(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: entries,
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: true,
    });
    app
}

fn listed<S: ThumbnailSource>(app: &Baboon) -> Vec<String> {
    let library = S::library(&app.kits[0]);
    library
        .matches
        .iter()
        .map(|&index| library.entries[index].display_path.clone())
        .collect()
}

#[test]
fn each_library_lists_only_its_own_kind_of_tag() {
    let mut app = app_with_mixed_kit();
    let ctx = egui::Context::default();
    app.refresh_thumbnail_library::<Models>(0, &ctx);
    app.refresh_thumbnail_library::<Bitmaps>(0, &ctx);

    assert_eq!(
        listed::<Models>(&app),
        [
            "objects/warthog/warthog.render_model",
            "objects/ghost/ghost.gbxmodel"
        ],
        "render geometry only: not the .model wrapper, not the vehicle"
    );
    let bitmaps = listed::<Bitmaps>(&app);
    assert_eq!(bitmaps.len(), 40);
    assert!(bitmaps.iter().all(|path| path.ends_with(".bitmap")));
}

#[test]
fn a_search_narrows_only_its_own_library() {
    let mut app = app_with_mixed_kit();
    let ctx = egui::Context::default();
    app.kits[0].model_browser.filter = "ghost".to_owned();
    app.refresh_thumbnail_library::<Models>(0, &ctx);
    app.refresh_thumbnail_library::<Bitmaps>(0, &ctx);

    assert_eq!(listed::<Models>(&app), ["objects/ghost/ghost.gbxmodel"]);
    assert_eq!(
        listed::<Bitmaps>(&app).len(),
        40,
        "the other library's search is its own"
    );
}

/// Drawing a library queues thumbnail jobs for the cells it shows, bounded,
/// and only for its own tags.
fn draws_and_queues_its_own<S: ThumbnailSource>(expected_suffix: &str) {
    let mut app = app_with_mixed_kit();
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            app.draw_thumbnail_library::<S>(ui, ctx, 0);
        });
    });
    let pending = &S::library(&app.kits[0]).pending;
    assert!(!pending.is_empty(), "no thumbnail was asked for");
    assert!(pending.len() <= MAX_DECODES_IN_FLIGHT);
    assert!(
        pending.iter().all(|key| key.ends_with(expected_suffix)),
        "queued another library's tag: {pending:?}"
    );
}

#[test]
fn the_bitmap_library_queues_bitmaps() {
    draws_and_queues_its_own::<Bitmaps>(".bitmap");
}

#[test]
fn the_model_library_queues_models() {
    draws_and_queues_its_own::<Models>("model");
}

/// How long egui may wait before the next frame, once the bitmap library has
/// settled with every bitmap's thumbnail either pending or failed.
fn repaint_delay_with_thumbnails(failed: bool) -> std::time::Duration {
    let mut app = app_with_mixed_kit();
    if failed {
        let mut thumbnails = Bitmaps::library(&app.kits[0]).thumbnails.lock().unwrap();
        for index in 0..40 {
            thumbnails.insert(format!("file:textures/grass_{index:02}.bitmap"), None);
        }
    }
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };
    // egui repaints its first frames by itself, and the scrollbar fades in
    // over time that only passes if the input says so. Once both settle, the
    // frame says what the library itself asked for.
    let mut delay = std::time::Duration::ZERO;
    for frame in 0..3 {
        let input = egui::RawInput {
            time: Some(frame as f64),
            ..input.clone()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                app.draw_thumbnail_library::<Bitmaps>(ui, ctx, 0);
            });
        });
        delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    }
    delay
}

/// A failed thumbnail is finished, not loading: a screen of them must not
/// spin, or keep the whole window repainting for as long as they are shown.
#[test]
fn failed_thumbnails_do_not_keep_the_window_repainting() {
    let pending = repaint_delay_with_thumbnails(false);
    assert!(
        pending <= std::time::Duration::from_millis(16),
        "pending thumbnails should animate: {pending:?}"
    );
    let failed = repaint_delay_with_thumbnails(true);
    assert!(
        failed > std::time::Duration::from_secs(1),
        "failed thumbnails still repaint every {failed:?}"
    );
}
