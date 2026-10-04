use super::*;

/// A thumbnail that lands after a generation bump is dropped, but its key
/// must leave `pending`: it used to stay, and four such keys stopped every
/// further decode for the kit.
#[test]
fn a_stale_thumbnail_still_frees_its_decode_slot() {
    let mut app = Baboon::for_test();
    let stamp = app.kit_stamp();
    app.views[app.model.kits[0].id]
        .bitmap_browser
        .pending
        .insert("file:a.bitmap".to_owned());
    app.model.kits[0].generation = app.model.kits[0].generation.wrapping_add(1);

    app.handle_thumbnail_ready::<Bitmaps>(
        stamp,
        "file:a.bitmap".to_owned(),
        Err("stale".to_owned()),
        &egui::Context::default(),
    );

    assert!(app.views[app.model.kits[0].id].bitmap_browser.pending.is_empty());
    let cached = app.views[app.model.kits[0].id]
        .bitmap_browser
        .thumbnails
        .lock()
        .unwrap()
        .contains("file:a.bitmap");
    assert!(!cached, "the stale result itself is not kept");
}

/// A generation bump keeps the thumbnails that are still right: listed
/// and unmodified. Everything used to be thrown away and decoded again.
#[test]
fn a_generation_bump_keeps_thumbnails_that_are_still_right() {
    let root = std::env::temp_dir().join(format!(
        "baboon-thumb-revalidate-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let set_time = |path: &Path, seconds: u64| {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
            .unwrap();
    };
    let (same, changed, gone) = (
        root.join("same.bitmap"),
        root.join("changed.bitmap"),
        root.join("gone.bitmap"),
    );
    for path in [&same, &changed, &gone] {
        std::fs::write(path, b"bitmap").unwrap();
        set_time(path, 1_000_000);
    }
    let key = |path: &Path| file_entry_key(&path);
    let mut cache = ThumbnailCache::default();
    for listed in [
        key(&same),
        key(&changed),
        key(&gone),
        "ublock:0:pak.bitmap".to_owned(),
    ] {
        cache.insert(listed, None);
    }
    set_time(&changed, 2_000_000);

    let listed = [key(&same), key(&changed), "ublock:0:pak.bitmap".to_owned()];
    cache.revalidate(|candidate| listed.iter().any(|key| key == candidate));

    std::fs::remove_dir_all(&root).unwrap();
    assert!(cache.contains(&key(&same)), "unchanged: kept");
    assert!(
        !cache.contains(&key(&changed)),
        "modified since it was decoded: dropped"
    );
    assert!(!cache.contains(&key(&gone)), "no longer listed: dropped");
    assert!(
        cache.contains("ublock:0:pak.bitmap"),
        "a pak tag cannot change: kept"
    );
}

/// Same for a model preview's texture resolve: a stale result left
/// `textures_pending` set, and the preview showed "Loading shaders…" and
/// repainted every frame for good.
#[test]
fn a_stale_texture_resolve_clears_textures_pending() {
    let mut app = Baboon::for_test();
    let stamp = app.kit_stamp();
    let state = app.views[app.model.kits[0].id]
        .caches.model_previews
        .entry("file:a.model".to_owned())
        .or_default();
    state.textures_pending = true;
    app.model.kits[0].generation = app.model.kits[0].generation.wrapping_add(1);

    app.handle_model_textures_resolved(stamp, "file:a.model".to_owned(), 1, Vec::new());

    assert!(!app.views[app.model.kits[0].id].caches.model_previews["file:a.model"].textures_pending);
}
