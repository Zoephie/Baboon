use super::*;

/// A stashed new tag that can never be placed leaves the retry queue and
/// says why. It used to stay queued, and adoption runs every frame, so it
/// redid the entry scans and the parse every frame for the whole session.
#[test]
fn an_overlay_that_cannot_be_placed_is_not_retried_every_frame() {
    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::SingleFile {
            path: PathBuf::from("a.model"),
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
    });
    let mut project = ActiveCampaignProject::fresh(PathBuf::from("recovery.baboon"), 0.0);
    project.pending_new_overlays.push(CampaignProjectOverlay {
        identity: "tag:unknown".to_owned(),
        group_tag: u32::from_be_bytes(*b"zzzz"),
        logical_path: "objects/unknown".to_owned(),
        kind: CampaignProjectTagKind::New,
        package: None,
        bytes: Arc::new(Vec::new()),
        digest: [0; 32],
    });
    app.kits[0].campaign_project = Some(project);

    app.adopt_pending_new_overlays(0);

    let queue = &app.kits[0]
        .campaign_project
        .as_ref()
        .unwrap()
        .pending_new_overlays;
    assert!(queue.is_empty(), "dropped from the retry queue");
    assert!(app.status.contains("Could not restore 1"), "{}", app.status);
}
