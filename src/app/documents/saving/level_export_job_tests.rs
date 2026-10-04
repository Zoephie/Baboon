use super::*;

/// Many exports share `ExportFinished`; one finishing while a level export
/// runs used to end the level's job, hiding its progress and releasing
/// the container-write guard while its worker was still reading.
#[test]
fn only_the_level_exports_own_completion_ends_its_job() {
    let mut app = Baboon::for_test();
    let kit = app.model.kits[0].id;
    app.chimp.chimp_level_job = Some(ChimpLevelJob::for_test(7, kit));

    app.handle_export_finished(Ok("Extracted a texture".to_owned()));
    assert!(app.chimp.chimp_level_job.is_some(), "another export does not end it");
    assert_eq!(app.model.status, "Extracted a texture");

    app.handle_chimp_level_export_finished(6, Ok("an earlier level".to_owned()));
    assert!(app.chimp.chimp_level_job.is_some(), "nor does an earlier level job");

    app.handle_chimp_level_export_finished(7, Ok("Exported level".to_owned()));
    assert!(app.chimp.chimp_level_job.is_none());
    assert_eq!(app.model.status, "Exported level");
}
