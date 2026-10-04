use super::*;

/// A container delete holds the write lease while it runs and gives it back
/// when it finishes, failed or not, so the container can be written again.
#[test]
fn a_finished_container_delete_gives_its_lease_back() {
    let mut app = Baboon::for_test();
    let utoc = PathBuf::from("/game/Paks/pakchunk0-WinGDK.utoc");
    let lease = app
        .acquire_container_write_lease(&utoc, ContainerWriteMode::AppendInPlace)
        .ok()
        .expect("a free container leases");
    let lease_id = app.park_container_write_lease(lease);
    assert!(
        app.acquire_container_write_lease(&utoc, ContainerWriteMode::AppendInPlace)
            .is_err(),
        "leased while the delete runs"
    );

    let stamp = app.kit_stamp();
    app.handle_container_delete_finished(stamp, lease_id, Err("disk full".to_owned()));

    let again = app
        .acquire_container_write_lease(&utoc, ContainerWriteMode::AppendInPlace)
        .ok()
        .expect("free again once it finished");
    app.release_in_place_lease(again, ContainerWriteOutcome::Unchanged);
}
