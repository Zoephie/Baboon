use super::*;

fn dialog(report: Option<FolderConversionReport>) -> CacheImportDialog {
    CacheImportDialog {
        kit: KitId(0),
        prefix: r"objects\weapons\rifle".to_owned(),
        selected: 12,
        targets: vec![CacheImportTarget {
            kit: KitId(1),
            label: "HREK".to_owned(),
            game: GameId::HaloReach,
            tags_root: PathBuf::from("D:/HREK/tags"),
        }],
        target_index: 0,
        outside_tree: OutsideTree::default(),
        outside_picked: std::collections::BTreeMap::new(),
        single: None,
        destination: None,
        replace: ReplaceChoice::Always,
        conflicts: OutsideTree::default(),
        conflict_picked: std::collections::BTreeMap::new(),
        conflicts_stale: true,
        scanning: false,
        running: false,
        cancel: Arc::new(AtomicBool::new(false)),
        progress: None,
        report,
        error: None,
    }
}

fn render(dialog: &mut CacheImportDialog) {
    let ctx = egui::Context::default();
    let _ = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(700.0, 900.0),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                draw_cache_import_body(ui, &ctx, dialog);
            });
        },
    );
}

/// Every state the window can be in draws.
///
/// Worth a test on its own because none of what breaks here is visible to a
/// compile: an egui id used twice, a scroll area nested where it cannot be,
/// a borrow that only fails once the closure actually runs. The window has
/// five shapes — asking for a folder, asking for one tag, running, failed,
/// done — and the done one carries every list it can show at once,
/// including a reference tree several folders deep.
#[test]
fn the_cache_import_window_draws_in_every_state() {
    render(&mut dialog(None));

    let mut running = dialog(None);
    running.running = true;
    running.progress = Some(FolderConversionProgress {
        phase: "Converting tags".to_owned(),
        current: r"objects\weapons\rifle\assault_rifle".to_owned(),
        processed: 40,
        total: 210,
        converted: 38,
        failed: 2,
    });
    render(&mut running);

    let mut failed = dialog(None);
    failed.error = Some("the destination kit went away".to_owned());
    render(&mut failed);

    // One tag, at its own path and at one the user picked: the second draws
    // a warning the first does not.
    let mut single = dialog(None);
    single.single = Some(SingleTagImport {
        key: r"cache:bitm:objects\weapons\rifle\bitmaps\ar_diffuse".to_owned(),
        display_path: r"objects\weapons\rifle\bitmaps\ar_diffuse.bitmap".to_owned(),
        parent: r"objects\weapons\rifle\bitmaps".to_owned(),
    });
    render(&mut single);
    single.destination = Some(PathBuf::from("scratch/imported"));
    render(&mut single);

    // The folder case now offers the same choice, and keeps its
    // shape under the folder that was picked rather than flattening.
    let mut moved = dialog(None);
    moved.destination = Some(PathBuf::from("scratch"));
    render(&mut moved);

    // Picking what to replace asks for a scan before it can draw
    // anything, and draws the answer once it has one.
    moved.replace = ReplaceChoice::Chosen;
    render(&mut moved);
    moved.conflicts_stale = false;
    render(&mut moved);
    moved.conflicts = OutsideTree::build(&[OutsideReference {
        key: r"cache:bitm:objects\weapons\rifle\bitmaps\ar_diffuse".to_owned(),
        display_path: "scratch/bitmaps/ar_diffuse.bitmap".to_owned(),
    }]);
    render(&mut moved);

    let mut done = dialog(Some(FolderConversionReport {
        source_root: PathBuf::from(r"objects\weapons\rifle"),
        source_game: "haloreach_mcc".to_owned(),
        target_game: "haloreach_mcc".to_owned(),
        destination_root: PathBuf::from("D:/HREK/tags"),
        files: vec![FolderConversionFileResult {
            source: "objects/weapons/rifle/assault_rifle.weapon".to_owned(),
            output: Some(PathBuf::from(
                "D:/HREK/tags/objects/weapons/rifle/assault_rifle.weapon",
            )),
            status: FolderConversionFileStatus::GeneratedLayout,
            overwritten: true,
            detail: "Built from the target profile's own definitions".to_owned(),
        }],
        ignored_files: Vec::new(),
        held_back: vec![FolderConversionHeldBack {
            source: "objects/weapons/rifle/fp_assault_rifle.model_animation_graph".to_owned(),
            key: r"cache:jmad:objects\weapons\rifle\fp_assault_rifle".to_owned(),
            losses: vec!["the animation payload has no way across".to_owned()],
        }],
        outside_references: vec![
            OutsideReference {
                key: r"cache:bitm:fx\decals\_bitmaps\scorch".to_owned(),
                display_path: "fx/decals/_bitmaps/scorch.bitmap".to_owned(),
            },
            OutsideReference {
                key: r"cache:rmt2:shaders\shader_templates\_0_0".to_owned(),
                display_path: "shaders/shader_templates/_0_0.render_method_template".to_owned(),
            },
        ],
        unresolved_references: ["fx/decals/_bitmaps/gone.bitmap".to_owned()]
            .into_iter()
            .collect(),
        levels_without_lighting: [r"levels\multirchive8_boneyard_v2".to_owned()]
            .into_iter()
            .collect(),
        cancelled: true,
    }));
    // The tree the window draws is built when a run reports, so a fixture
    // that skips that step would exercise an empty one.
    if let Some(report) = done.report.as_ref() {
        done.outside_tree = OutsideTree::build(&report.outside_references);
        done.outside_picked = report
            .outside_references
            .iter()
            .map(|reference| (reference.key.clone(), true))
            .collect();
    }
    render(&mut done);
}
