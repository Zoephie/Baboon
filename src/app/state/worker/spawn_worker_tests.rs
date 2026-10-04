use super::*;
use std::time::Duration;

/// Every export and extraction reports through `ExportFinished`, and the
/// status line says "Extracting ..." until it does. Each ran on a bare
/// thread, so one that panicked left that status up for good.
#[test]
fn an_export_that_panics_still_reports_and_replaces_its_status() {
    let mut app = crate::app::Baboon::for_test();
    let ctx = egui::Context::default();
    app.status = "Extracting bitmap objects/rock".to_owned();
    with_panicking_workers(|| {
        spawn_export(&app.tx, &ctx, || Ok("Extracted objects/rock".to_owned()))
    });
    assert!(apply_next_worker_message(&mut app), "the export answered");
    assert!(app.status.starts_with("The export failed"), "{}", app.status);

    // And one that runs reports its own result.
    spawn_export(&app.tx, &ctx, || Ok("Extracted objects/rock".to_owned()));
    assert!(apply_next_worker_message(&mut app));
    assert_eq!(app.status, "Extracted objects/rock");
}

/// The test helper applies results in the order they arrived. It used to
/// put the message it waited for back on the channel, behind anything
/// already queued, so two queued results were applied in reverse.
#[test]
fn the_test_helper_applies_results_in_arrival_order() {
    let mut app = crate::app::Baboon::for_test();
    app.tx.send(WorkerMessage::ExportFinished(Ok("first".to_owned()))).unwrap();
    app.tx.send(WorkerMessage::ExportFinished(Ok("second".to_owned()))).unwrap();
    assert!(apply_next_worker_message(&mut app));
    assert_eq!(app.status, "second", "the later result is applied last");
}

/// Every export reports through `spawn_export`, not a hand-rolled send
/// from a thread of its own, which is what let a panic skip the report.
#[test]
fn exports_report_only_through_spawn_export() {
    for (file, text) in [
        ("controller/mod.rs", include_root_str!("src/app/controller/mod.rs")),
        ("chimp/extract/mod.rs", include_root_str!("src/app/chimp/extract/mod.rs")),
    ] {
        assert!(
            !text.contains("send(WorkerMessage::ExportFinished("),
            "{file} sends ExportFinished itself"
        );
        assert!(text.contains("spawn_export("), "{file}: the scan is not looking");
    }
}

/// A worker that panics still answers, so whatever the UI marked as in
/// flight is settled. A plain `thread::spawn` sent nothing.
#[test]
fn a_panicking_worker_still_sends_its_message() {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = egui::Context::default();
    spawn_worker(
        &tx,
        &ctx,
        || panic!("decoder fell over"),
        |error| WorkerMessage::TagLoaded {
            kit: KitId(0),
            key: "k".to_owned(),
            result: Err(error),
        },
    );
    spawn_worker(
        &tx,
        &ctx,
        || WorkerMessage::TagLoaded {
            kit: KitId(0),
            key: "fine".to_owned(),
            result: Err("not a panic".to_owned()),
        },
        |_| unreachable!(),
    );

    let mut results = Vec::new();
    for _ in 0..2 {
        let Ok(WorkerMessage::TagLoaded { key, result, .. }) =
            rx.recv_timeout(Duration::from_secs(10))
        else {
            panic!("a worker did not answer");
        };
        results.push((key, result.unwrap_err()));
    }
    results.sort();
    assert_eq!(results[0], ("fine".to_owned(), "not a panic".to_owned()));
    assert_eq!(results[1].0, "k");
    assert!(
        results[1].1.contains("decoder fell over"),
        "{}",
        results[1].1
    );
}
