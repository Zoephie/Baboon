use super::*;

fn draw_pane<'a>(app: &'a mut Baboon, package: &'a str) -> impl FnMut(&mut egui::Ui) + 'a {
    move |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            app.draw_chimp_document_pane(ui, 0, package, "test");
        });
    }
}

fn draw_tiles(app: &mut Baboon) -> impl FnMut(&mut egui::Ui) + '_ {
    move |ui| {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| {
            app.draw_chimp_tiles(ui, &ctx, 0);
        });
    }
}

/// A clean package's pane: what it is, where it lives, its views, and a
/// save button that does nothing until something changes.
#[test]
fn a_clean_pane_describes_the_package_and_offers_its_views() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[THING]);
    let bytes = app.model.kits[0].chimp.documents[THING].original.len();
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut draw_pane(&mut app, THING));
    let utoc = install.root.join("Paks").join("pakchunk0-Windows.utoc");
    for text in [
        THING.to_owned(),
        format!("1 exports • 2 imports • {bytes} bytes • {}", utoc.display()),
        "Document".to_owned(),
        "Properties".to_owned(),
        "Header".to_owned(),
        "Metadata".to_owned(),
        "Decoded Unreal package document".to_owned(),
        "Copy JSON".to_owned(),
    ] {
        assert!(frames.shows(&text), "{text}");
    }
    assert!(!frames.shows("Texture") && !frames.shows("Mesh"));
    frames.click("Save Chimp changes…", &mut draw_pane(&mut app, THING));
    assert!(!app.has_chimp_save_dialog(), "disabled while clean");
    assert_eq!(app.model.kits[0].chimp.documents[THING].edits, 0);
}

/// An edit made in the pane marks the document modified, counts it,
/// stales the derived views, and schedules a checkpoint a second out —
/// pushed back by the next edit.
#[test]
fn an_edit_in_the_pane_marks_counts_and_schedules_a_checkpoint() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[THING]);
    let mut frames = Frames::new();
    frames.click_exact("Properties", 0, &mut draw_pane(&mut app, THING));
    assert_eq!(
        app.chimp_pane(0, THING).view,
        ChimpDocumentView::Properties
    );
    assert!(frames.shows("●  Thing"));
    assert!(!app.model.kits[0].chimp.documents[THING].dirty);

    // The editor sits in a panel nested inside the pane's own.
    frames.value_x = VALUE_X - 8.0;
    let before = frames.time();
    frames.enter_value_of("Count", "42", &mut draw_pane(&mut app, THING));
    let document = &app.model.kits[0].chimp.documents[THING];
    assert!(matches!(first_value(document, "Count"), PropValue::Int(42)));
    assert!(document.dirty);
    assert_eq!(document.edits, 1);
    let pane = app.chimp_pane(0, THING);
    assert!(pane.document_text_dirty && pane.metadata_text_dirty);
    assert!(pane.header_usage.is_none());
    let first = document.checkpoint_due.expect("a checkpoint is scheduled");
    assert!(
        first > before + CHIMP_CHECKPOINT_DELAY && first <= frames.time() + CHIMP_CHECKPOINT_DELAY,
        "{first} is a second after the edit"
    );

    frames.enter_value_of("Count", "43", &mut draw_pane(&mut app, THING));
    let document = &app.model.kits[0].chimp.documents[THING];
    assert_eq!(document.edits, 2);
    assert!(document.checkpoint_due.unwrap() > first, "pushed back");

    frames.click("Save Chimp changes…", &mut draw_pane(&mut app, THING));
    assert!(app.has_chimp_save_dialog());
}

/// The Header view through the pane: a rename counts as an edit, and the
/// referrer button starts the sweep on a worker.
#[test]
fn the_header_view_edits_and_scans_through_the_pane() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[THING]);
    let mut frames = Frames::new();
    frames.click_exact("Header", 0, &mut draw_pane(&mut app, THING));
    assert_eq!(
        app.chimp_pane(0, THING).view,
        ChimpDocumentView::Header
    );
    frames.click_exact("Rocket", 0, &mut draw_pane(&mut app, THING));
    frames.replace_text("Comet", &mut draw_pane(&mut app, THING));
    frames.key(
        egui::Key::Enter,
        egui::Modifiers::NONE,
        &mut draw_pane(&mut app, THING),
    );
    let document = &app.model.kits[0].chimp.documents[THING];
    assert_eq!(document.header.name_map.names()[2], "Comet");
    assert!(document.dirty);
    assert_eq!(document.edits, 1);

    frames.click("Referenced by", &mut draw_pane(&mut app, THING));
    frames.click(
        "Find packages that import this",
        &mut draw_pane(&mut app, THING),
    );
    assert!(matches!(
        app.chimp_pane(0, THING).referrers,
        ChimpReferrerState::Scanning
    ));
    apply_until(&mut app, |app| {
        matches!(
            app.chimp_pane(0, THING).referrers,
            ChimpReferrerState::Done(_)
        )
    });
    frames.frame(Vec::new(), &mut draw_pane(&mut app, THING));
    assert!(frames.shows("No hard import, of 1 packages read"));
}

/// The Metadata view renders the header dump; an orphaned document says
/// it cannot be written back; an unloaded package says so.
#[test]
fn the_metadata_view_and_the_orphaned_and_unloaded_states() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[THING]);
    let mut frames = Frames::new();
    frames.click_exact("Metadata", 0, &mut draw_pane(&mut app, THING));
    assert_eq!(
        app.chimp_pane(0, THING).view,
        ChimpDocumentView::Metadata
    );
    assert!(frames.shows("Decoded package metadata"));
    assert!(frames.shows("Copy metadata JSON"));

    app.model.kits[0]
        .chimp
        .documents
        .get_mut(THING)
        .unwrap()
        .orphaned = true;
    frames.frame(Vec::new(), &mut draw_pane(&mut app, THING));
    assert!(frames.shows("No mounted container provides this package any more."));
    assert!(frames.shows("1 exports • 2 imports •"));
    assert!(
        frames
            .labels
            .iter()
            .any(|(label, _)| label.ends_with("• (no container)"))
    );

    frames.frame(Vec::new(), &mut draw_pane(&mut app, "/Game/Test/Missing"));
    assert!(frames.shows("This package is no longer loaded."));
}

/// The tab strip marks a modified package, and its menu closes what it
/// can: a modified package stays and the status says why.
#[test]
fn the_tab_menu_closes_clean_packages_and_keeps_modified_ones() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[THING, OTHER]);
    app.model.kits[0].chimp.documents.get_mut(THING).unwrap().dirty = true;
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut draw_tiles(&mut app));
    assert!(frames.shows("• Thing"));
    assert!(frames.shows("Other"));

    frames.right_click_exact("• Thing", &mut draw_tiles(&mut app));
    frames.click_exact("Close", 0, &mut draw_tiles(&mut app));
    assert!(app.model.kits[0].chimp.documents.contains_key(THING));
    assert_eq!(
        app.model.status,
        "Save or discard modified Chimp packages before closing them."
    );

    frames.right_click_exact("• Thing", &mut draw_tiles(&mut app));
    frames.click("Close all but this", &mut draw_tiles(&mut app));
    assert!(!app.model.kits[0].chimp.documents.contains_key(OTHER));
    assert_eq!(app.model.kits[0].chimp.open_packages, [THING]);

    app.model.kits[0].chimp.documents.get_mut(THING).unwrap().dirty = false;
    frames.right_click_exact("Thing", &mut draw_tiles(&mut app));
    frames.click_exact("Close all", 0, &mut draw_tiles(&mut app));
    assert!(app.model.kits[0].chimp.documents.is_empty());
    assert!(app.model.kits[0].chimp.open_packages.is_empty());
    frames.frame(Vec::new(), &mut draw_tiles(&mut app));
    assert!(frames.shows("Select a package to inspect it."));
}

/// Frame time of the JSON pane on a 100,000-line document. Run with
/// `--release --ignored --nocapture`. The single-label pane took 18 ms a
/// frame; the row-virtualized one takes about 60 µs.
#[test]
#[ignore]
fn bench_json_viewer_frame() {
    let mut text = String::from("{\n");
    let mut lines = ChimpJsonLines::default();
    for i in 0..100_000 {
        text.push_str(&format!("  \"Key{i}\": \"/Game/Some/Path/Asset_{i}\",\n"));
    }
    text.push('}');
    eprintln!("text {} bytes", text.len());
    let ctx = egui::Context::default();
    let mut frame = |ctx: &egui::Context| {
        let started = std::time::Instant::now();
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1000.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    draw_chimp_json_document(ui, "bench", "t", "c", &text, &mut lines);
                });
            },
        );
        started.elapsed()
    };
    for i in 0..6 {
        eprintln!("frame {i}: {:?}", frame(&ctx));
    }
}

/// An orphaned document's stored container index addresses a list that a
/// remount may have shrunk: the header line indexed with it directly, a
/// panic on every frame the pane was drawn.
#[test]
fn an_orphaned_documents_header_names_no_container() {
    let container = |index: usize| blam_tags::iostore::world::WorldContainer {
        index,
        path: PathBuf::from(format!("pakchunk{index}.utoc")),
        read_order: index as u32,
        recovered_directory_index: false,
        package_count: 1,
    };
    let mut document = rename_fixture();
    document.provider.container = 3;
    document.orphaned = true;
    assert_eq!(
        chimp_document_container_label(&document, &[container(0)]),
        "(no container)"
    );
    document.provider.container = 0;
    assert_eq!(
        chimp_document_container_label(&document, &[container(0)]),
        "(no container)",
        "an orphan's old index names some other container now"
    );
    document.orphaned = false;
    assert_eq!(
        chimp_document_container_label(&document, &[container(0)]),
        "pakchunk0.utoc"
    );
}

#[test]
fn json_documents_have_line_numbers_and_semantic_colours() {
    let text = "{\n  \"Name\": \"Probe\",\n  \"ObjectPath\": \"/Game/Probe.0\",\n  \"Count\": 3,\n  \"Enabled\": true,\n  \"Missing\": null\n}";
    let job = chimp_json_layout_job(text, egui::FontId::monospace(12.0), true);
    let lines = split_layout_job_lines(&job);
    assert_eq!(lines.len(), 7);
    for (line, expected) in lines.iter().zip(text.lines()) {
        assert_eq!(line.text, expected);
    }
    // Split, every piece keeps the colour it had in the whole document.
    let pieces = |jobs: &[&egui::text::LayoutJob]| {
        jobs.iter()
            .flat_map(|job| {
                job.sections.iter().flat_map(|section| {
                    job.text[section.byte_range.start.0..section.byte_range.end.0]
                        .split('\n')
                        .filter(|piece| !piece.is_empty())
                        .map(|piece| (piece.to_owned(), section.format.color))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(pieces(&lines.iter().collect::<Vec<_>>()), pieces(&[&job]));
    assert_eq!(job.text, text);
    let mut colors = Vec::new();
    for section in &job.sections {
        if !colors.contains(&section.format.color) {
            colors.push(section.format.color);
        }
    }
    assert!(
        colors.len() >= 7,
        "keys, strings, paths, numbers, literals, punctuation, and whitespace need distinct colours"
    );
}

#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn real_package_rebuilds_into_a_readable_overlay() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let mut browser = ChimpView::default();
    browser.refresh_filter(&world);
    assert_eq!(browser.filtered_packages.len(), world.packages().len());
    assert_eq!(browser.filtered_files.len(), world.pak_files().len());
    assert_eq!(
        browser.content_tree.package_count,
        browser.filtered_packages.len()
    );
    assert_eq!(
        browser.content_tree.file_count,
        browser.filtered_files.len()
    );
    let container = world
        .containers()
        .iter()
        .find(|container| container.package_count > 0)
        .expect("an IoStore container with packages");
    browser.selected_archive = Some(ChimpArchive::IoStore(container.index));
    browser.reset_filter();
    browser.refresh_filter(&world);
    assert!(!browser.filtered_packages.is_empty());
    assert!(browser.filtered_packages.iter().all(|&index| {
        world.packages()[index]
            .providers
            .iter()
            .any(|provider| provider.container == container.index)
    }));
    let pak = world
        .pak_containers()
        .iter()
        .find(|container| container.file_count > 0)
        .expect("a legacy pak with files");
    browser.selected_archive = Some(ChimpArchive::Pak(pak.index));
    browser.reset_filter();
    browser.refresh_filter(&world);
    assert!(browser.filtered_packages.is_empty());
    assert!(!browser.filtered_files.is_empty());
    assert!(browser.filtered_files.iter().all(|&index| {
        world.pak_files()[index]
            .providers
            .iter()
            .any(|provider| provider.container == pak.index)
    }));
    assert!(
        !world.pak_files().is_empty(),
        "the real mount should index legacy .pak files too"
    );
    let package = world
        .packages()
        .iter()
        .find(|package| package.name.starts_with("/Game/"))
        .expect("a /Game package")
        .name
        .clone();
    let (document, mut pane) = load_chimp_document_with_pane(&world, &package).unwrap();
    assert_eq!(pane.view, ChimpDocumentView::Document);
    assert!(!pane.document_text_dirty);
    assert!(
        !document.exports.is_empty(),
        "the real package should expose at least one readable export"
    );
    let readable: Value =
        serde_json::from_str(&pane.document_text).expect("readable document is valid JSON");
    assert_eq!(readable["Package"], package);
    assert_eq!(
        readable["Exports"].as_array().map(Vec::len),
        Some(document.exports.len())
    );
    let export = readable["Exports"]
        .as_array()
        .and_then(|exports| exports.first())
        .expect("the JSON document should contain its first export");
    assert!(export.get("Type").is_some());
    assert!(export.get("Name").is_some());
    assert!(export.get("Properties").is_some());
    assert_eq!(
        pane
            .document_lines
            .lines(
                &pane.document_text,
                &egui::FontId::monospace(12.0),
                true
            )
            .len(),
        pane.document_text.lines().count()
    );
    let metadata: Value =
        serde_json::from_str(&pane.metadata_text).expect("metadata document is valid JSON");
    assert_eq!(metadata["Summary"]["Package"], package);
    assert_eq!(
        metadata["NameMap"].as_array().map(Vec::len),
        Some(document.header.name_map.copy_raw_names().len())
    );
    assert_eq!(
        metadata["ExportMap"].as_array().map(Vec::len),
        Some(document.header.export_map.len())
    );
    assert!(
        metadata["PhysicalProviders"]
            .as_array()
            .is_some_and(|providers| !providers.is_empty())
    );
    assert_eq!(
        pane
            .metadata_lines
            .lines(
                &pane.metadata_text,
                &egui::FontId::monospace(12.0),
                true
            )
            .len(),
        pane.metadata_text.lines().count()
    );
    let (bytes, store) = rebuild_chimp_document(&world, &document).unwrap();
    FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        Some(store.clone()),
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .expect("rebuilt package parses");

    let directory = std::env::temp_dir().join(format!("baboon-chimp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("ChimpTest_P.utoc");
    let override_ = PackageOverride {
        archive: &world.archives()[document.provider.container],
        uasset_path: &document.provider.entry_path,
        bytes: bytes.clone(),
        store,
    };
    write_package_mod_container(&[override_], &output).unwrap();
    let mut overlay = blam_tags::iostore::IoStoreArchive::open(&output).unwrap();
    let bases: Vec<&blam_tags::iostore::IoStoreArchive> = world.archives().iter().collect();
    overlay.recover_entries(&bases, Some("Meteorite/Content/"));
    assert_eq!(overlay.read(&document.provider.entry_path).unwrap(), bytes);
    std::fs::remove_dir_all(directory).unwrap();
}
