use super::*;
use crate::app::mods::in_place::InPlaceOverwrite;
use crate::app::mods::in_place::InPlaceOverwriteJob;

fn job(app: &Baboon, dirty_revision: u64) -> InPlaceOverwriteJob {
    InPlaceOverwriteJob {
        stamp: app.kit_stamp(),
        key: "tag".to_owned(),
        dirty_revision,
        root: PathBuf::new(),
        containers: Vec::new(),
        container_idx: 0,
        utoc_path: std::env::temp_dir().join("baboon-in-place-test/pakchunk0-Windows.utoc"),
        rel_path: String::new(),
        bytes: Vec::new(),
    }
}

fn saved() -> InPlaceOverwrite {
    InPlaceOverwrite {
        write: Ok(()),
        reopened: None,
        touched: true,
    }
}

/// The write runs on a worker from bytes serialized before it started.
/// An edit made meanwhile is not in them, so the tag stays dirty.
#[test]
fn a_tag_edited_during_its_save_stays_dirty() {
    let mut app = Baboon::for_test();
    let tag = TagFile::new(crate::app::test_definition_path(
        "halo4_mcc/camera_track.json",
    ))
    .unwrap();
    app.model.kits[0]
        .parsed_tags
        .insert("tag".to_owned(), TagDocument::modified(tag));
    let at_save = app.model.kits[0].parsed_tags["tag"].dirty.revision();

    app.model.kits[0]
        .parsed_tags
        .get_mut("tag")
        .unwrap()
        .dirty
        .touch();
    app.finish_in_place_overwrite(job(&app, at_save), saved());
    assert!(
        app.model.kits[0].parsed_tags["tag"].dirty.is_set(),
        "edited mid-save"
    );

    let now = app.model.kits[0].parsed_tags["tag"].dirty.revision();
    app.finish_in_place_overwrite(job(&app, now), saved());
    assert!(
        !app.model.kits[0].parsed_tags["tag"].dirty.is_set(),
        "saved as it stands"
    );
}

/// The worker's lease is released whatever the write did, or the
/// container refuses every later write.
#[test]
fn a_failed_in_place_overwrite_releases_its_lease() {
    let mut app = Baboon::for_test();
    let job = job(&app, 0);
    let lease = app
        .acquire_container_write_lease(&job.utoc_path, ContainerWriteMode::AppendInPlace)
        .unwrap();
    let lease = app.park_container_write_lease(lease);
    let failed = InPlaceOverwrite {
        write: Err("no paired .uasset".to_owned()),
        reopened: None,
        touched: false,
    };
    let utoc = job.utoc_path.clone();
    app.handle_in_place_overwrite_finished(job, lease, failed);
    assert!(
        app.model.status.contains("export this mod again"),
        "{}",
        app.model.status
    );
    let again = app
        .acquire_container_write_lease(&utoc, ContainerWriteMode::AppendInPlace)
        .expect("the container is writable again");
    app.release_in_place_lease(again, ContainerWriteOutcome::Unchanged);
}
