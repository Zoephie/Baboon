use super::*;

#[test]
fn chimp_workspace_toolbar_does_not_consume_the_editor_viewport() {
    let context = egui::Context::default();
    let mut toolbar_height = None;
    let _ = crate::app::run_ui_test(
        &context,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1_200.0, 800.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let toolbar = chimp_workspace_toolbar(ui, |ui| {
                    let _ = ui.button("Discard");
                });
                toolbar_height = Some(toolbar.response.rect.height());
            });
        },
    );

    let toolbar_height = toolbar_height.expect("the Chimp toolbar was rendered");
    assert!(
        toolbar_height <= 40.0,
        "the Chimp toolbar expanded to {toolbar_height}px"
    );
}

#[test]
fn a_level_offers_a_menu_wherever_it_is_browsed() {
    // The bug this exists for: the level entry was added to every menu
    // body, but two of the four sites still only attached a menu for
    // textures and meshes - so on those the entry sat inside a menu that
    // was never built, and right-clicking a level did nothing at all.
    // A level is a `World`, which is neither.
    let level = ChimpPackageActions::of("/Game/Levels/Halo1/Solo/C10/C10", Some("World"));
    assert!(level.level);
    assert!(level.any(), "a level must open a menu");
    assert!(!level.texture && !level.mesh);
}

#[test]
fn anything_the_menu_would_offer_opens_it() {
    // The invariant that keeps the guard and the contents from drifting:
    // `any()` is true exactly when at least one entry would be drawn.
    for (package, kind) in [
        ("/Game/Levels/Halo1/Solo/C10/C10", Some("World")),
        ("/Game/Meshes/SM_Rock", Some("StaticMesh")),
        ("/Game/Characters/SK_Elite", Some("SkeletalMesh")),
        ("/Game/Textures/T_Bark", Some("Texture2D")),
        (
            "/Game/Levels/Halo1/Solo/C10/_Generated_/043ATWPYEEJ",
            Some("World"),
        ),
        ("/Game/Blueprints/BP_Door", Some("Blueprint")),
        ("/Game/Misc/Thing", None),
    ] {
        let actions = ChimpPackageActions::of(package, kind);
        assert_eq!(
            actions.any(),
            actions.texture || actions.mesh || actions.level,
            "{package} would draw entries into a menu that does not open"
        );
    }
}

#[test]
fn a_package_with_nothing_to_offer_opens_no_menu() {
    assert!(!ChimpPackageActions::of("/Game/Blueprints/BP_Door", Some("Blueprint")).any());
    // A cell is a World too, and there are 2,334 of them: offering to
    // export each one as a level would be noise.
    assert!(
        !ChimpPackageActions::of(
            "/Game/Levels/Halo1/Solo/C10/_Generated_/043ATWPYEEJ",
            Some("World")
        )
        .any()
    );
}

#[test]
fn chimp_search_matching_is_case_insensitive_without_allocating_per_package() {
    assert!(contains_ignore_ascii_case(
        "SM_SpiritDropShip_Body",
        "spirit"
    ));
    assert!(contains_ignore_ascii_case("Texture2D", "texture2d"));
    assert!(!contains_ignore_ascii_case("StaticMesh", "skeletal"));
}

/// Draw kit 0's Chimp surface and apply what it sent, as a frame does.
fn draw_workspace(app: &mut Baboon) -> impl FnMut(&mut egui::Ui) + '_ {
    move |ui| {
        let ctx = ui.ctx().clone();
        let kit = app.model.kits[0].id;
        egui::CentralPanel::default().show(ui, |ui| {
            draw_chimp_workspace(
                ui,
                &cx!(app, &ctx),
                &mut app.chimp,
                &mut app.views[kit].chimp,
                0,
            );
        });
        app.apply_commands(&ctx);
    }
}

/// The folder tree nests packages by path; clicking one opens it beside
/// the tree.
#[test]
fn the_folder_tree_opens_a_package() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[]);
    let mut frames = Frames::new();
    frames.click("Game  ·  2", &mut draw_workspace(&mut app));
    frames.click("Test  ·  2", &mut draw_workspace(&mut app));
    assert!(frames.shows("Other"));
    frames.click_exact("Thing", 0, &mut draw_workspace(&mut app));
    assert_eq!(
        app.views[app.model.kits[0].id].chimp.folder_selection,
        ChimpFolderSelection::Package
    );
    assert_eq!(app.model.kits[0].chimp.selected_package.as_deref(), Some(THING));
    apply_until(&mut app, |app| app.model.kits[0].chimp.documents.contains_key(THING));
    frames.frame(Vec::new(), &mut draw_workspace(&mut app));
    assert!(frames.shows("1 exports • 2 imports •"), "the pane is drawn");
}

/// The flat package list opens what is clicked, and the search box
/// narrows it.
#[test]
fn the_package_list_opens_and_filters() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[]);
    let mut frames = Frames::new();
    frames.click_exact("Packages", 0, &mut draw_workspace(&mut app));
    assert_eq!(app.views[app.model.kits[0].id].chimp.browser, ChimpBrowser::Packages);
    assert!(frames.shows(THING) && frames.shows(OTHER));
    frames.click_exact(OTHER, 0, &mut draw_workspace(&mut app));
    apply_until(&mut app, |app| app.model.kits[0].chimp.documents.contains_key(OTHER));

    frames.click("Search package or container…", &mut draw_workspace(&mut app));
    frames.type_text("thing", &mut draw_workspace(&mut app));
    frames.frame(Vec::new(), &mut draw_workspace(&mut app));
    let chimp = &app.model.kits[0].chimp;
    let view = &app.views[app.model.kits[0].id].chimp;
    assert_eq!(view.filter, "thing");
    let ChimpMount::Ready(world) = &chimp.mount else {
        unreachable!()
    };
    assert_eq!(
        view
            .filtered_packages
            .iter()
            .map(|&index| world.packages()[index].name.as_str())
            .collect::<Vec<_>>(),
        [THING]
    );
}

/// The archive list names each container with its package count; picking
/// one scopes the folder tree to it until "Show all".
#[test]
fn the_archive_list_scopes_the_tree() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[]);
    let mut frames = Frames::new();
    frames.click_exact("Archives", 0, &mut draw_workspace(&mut app));
    for text in [
        "All mounted archives",
        "pakchunk0-Windows.utoc  ·  2 packages",
        "pakchunk0-Windows.pak  ·  0 files",
        "Unavailable or empty",
        "Select an archive to browse its folder hierarchy.",
    ] {
        assert!(frames.shows(text), "{text}");
    }
    frames.click("pakchunk0-Windows.utoc  ·  2 packages", &mut draw_workspace(&mut app));
    let chimp = &app.views[app.model.kits[0].id].chimp;
    assert_eq!(chimp.selected_archive, Some(ChimpArchive::IoStore(0)));
    assert_eq!(chimp.browser, ChimpBrowser::Folders);
    assert!(frames.shows("Game  ·  2"));
    frames.click("Show all", &mut draw_workspace(&mut app));
    assert_eq!(app.views[app.model.kits[0].id].chimp.selected_archive, None);

    frames.click_exact("Archives", 0, &mut draw_workspace(&mut app));
    frames.click("pakchunk0-Windows.pak  ·  0 files", &mut draw_workspace(&mut app));
    let chimp = &app.views[app.model.kits[0].id].chimp;
    assert_eq!(chimp.selected_archive, Some(ChimpArchive::Pak(0)));
    assert_eq!(chimp.folder_selection, ChimpFolderSelection::File);
    assert!(frames.shows("Select a file from a legacy .pak container."));
    assert!(!frames.shows("Game  ·  2"), "the pak holds no packages");
}

/// Packages group by their indexed type; a group opens onto its packages.
#[test]
fn the_group_list_opens_a_package_by_type() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[]);
    // Indexed by mount order, which sorts `Other` first.
    app.views[app.model.kits[0].id].chimp.package_types = vec![Some("Texture2D".to_owned()), None];
    let mut frames = Frames::new();
    frames.click_exact("Groups", 0, &mut draw_workspace(&mut app));
    assert!(frames.shows("Texture2D  ·  1"));
    assert!(frames.shows("Unknown  ·  1"));
    frames.click("Texture2D  ·  1", &mut draw_workspace(&mut app));
    frames.click_exact("Other", 0, &mut draw_workspace(&mut app));
    apply_until(&mut app, |app| app.model.kits[0].chimp.documents.contains_key(OTHER));
}

/// Before the mount, the workspace offers to start it, waits while it
/// runs, and browses once it lands; a failed mount offers a retry.
#[test]
fn the_mount_status_starts_waits_and_retries() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[]);
    app.model.prefs.enable_chimp = true;
    app.model.kits[0].chimp.mount = ChimpMount::Idle;
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut draw_workspace(&mut app));
    assert!(frames.shows("The Unreal package index has not been started."));
    frames.click("Start Chimp", &mut draw_workspace(&mut app));
    assert!(matches!(app.model.kits[0].chimp.mount, ChimpMount::Loading));
    frames.frame(Vec::new(), &mut draw_workspace(&mut app));
    assert!(frames.shows("Please wait — Chimp is starting up…"));
    apply_until(&mut app, |app| {
        matches!(app.model.kits[0].chimp.mount, ChimpMount::Ready(_))
            && !app.views[app.model.kits[0].id].chimp.type_indexing
    });
    frames.frame(Vec::new(), &mut draw_workspace(&mut app));
    assert!(frames.shows("Game  ·  2"));

    app.model.kits[0].chimp.mount = ChimpMount::Failed("no containers".to_owned());
    frames.frame(Vec::new(), &mut draw_workspace(&mut app));
    assert!(frames.shows("no containers"));
    frames.click("Retry", &mut draw_workspace(&mut app));
    assert!(matches!(app.model.kits[0].chimp.mount, ChimpMount::Loading));
    apply_until(&mut app, |app| {
        matches!(app.model.kits[0].chimp.mount, ChimpMount::Ready(_))
            && !app.views[app.model.kits[0].id].chimp.type_indexing
    });
}

/// The toolbar's discard button opens the prompt for every modified
/// package, and does nothing while there are none.
#[test]
fn the_toolbar_discard_prompts_for_modified_packages() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[THING]);
    // The toolbar is right-aligned on the first row.
    let button = egui::pos2(VALUE_X - 4.0, 8.0 + 12.0);
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut draw_workspace(&mut app));
    frames.click_at(button, &mut draw_workspace(&mut app));
    assert!(app.chimp.chimp_discard_prompt.is_none(), "disabled while clean");

    app.model.kits[0].chimp.documents.get_mut(THING).unwrap().dirty = true;
    frames.click_at(button, &mut draw_workspace(&mut app));
    let prompt = app.chimp.chimp_discard_prompt.as_ref().expect("the prompt opened");
    assert_eq!(prompt.packages, [THING]);
    assert!(prompt.pending_action.is_none());
}

#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn real_file_types_filter_and_texture_preview() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let index = index_chimp_package_types(&world);
    assert_eq!(index.package_types.len(), world.packages().len());
    for expected in ["Blueprint", "SkeletalMesh", "StaticMesh", "Texture2D"] {
        assert!(
            index.type_counts.contains_key(expected),
            "real package index should contain {expected}"
        );
    }

    let mut browser = ChimpView {
        package_types: index.package_types,
        filter: "Texture2D".to_owned(),
        ..Default::default()
    };
    browser.refresh_filter(&world);
    assert!(!browser.filtered_packages.is_empty());
    let textures = &browser.filtered_groups["Texture2D"];
    assert!(!textures.is_empty());
    assert!(
        textures
            .iter()
            .all(|index| { browser.package_types[*index].as_deref() == Some("Texture2D") })
    );

    let target = std::env::var("CE_TEXTURE_PACKAGE").ok();
    let package_index = match target.as_deref() {
        Some(target) => {
            let target = target.to_ascii_lowercase();
            textures
                .iter()
                .copied()
                .find(|index| {
                    world.packages()[*index]
                        .name
                        .to_ascii_lowercase()
                        .contains(&target)
                })
                .unwrap_or_else(|| panic!("target Texture2D package {target:?} was not found"))
        }
        None => textures[0],
    };
    let package = world.packages()[package_index].name.clone();
    let (_, pane) = load_chimp_document_with_pane(&world, &package).unwrap();
    assert_eq!(pane.view, ChimpDocumentView::Texture);
    let decoded = pane
        .texture_previews
        .iter()
        .find_map(|preview| {
            preview
                .preview
                .decoded
                .as_ref()
                .and_then(|decoded| decoded.as_ref().ok())
        })
        .unwrap_or_else(|| panic!("{package} should decode at least one Texture2D preview"));
    assert_eq!(
        decoded.rgba.len(),
        decoded.width as usize * decoded.height as usize * 4
    );
    if package
        .to_ascii_lowercase()
        .ends_with("/t_odst_williams_default_d")
    {
        assert_eq!((decoded.width, decoded.height), (4096, 1024));
        assert!(
            decoded
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2]),
            "target virtual texture should contain decoded colour data"
        );
    }
    // A UDIM set writes one numbered file per block rather than the single
    // path chosen, so check the directory rather than that exact name.
    let directory =
        std::env::temp_dir().join(format!("baboon-chimp-texture-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    write_chimp_texture(
        &world,
        &package,
        &directory.join("texture.tif"),
        ChimpTextureExport {
            format: ChimpTextureFormat::Tiff,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let written: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(!written.is_empty(), "Texture2D extraction wrote nothing");
    for path in &written {
        let bytes = std::fs::read(path).unwrap();
        assert!(
            bytes.starts_with(b"II*") || bytes.starts_with(b"MM\0*"),
            "{} should be a TIFF file",
            path.display()
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
