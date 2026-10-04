use super::*;

/// Expanding a folder lazily loads its tags and, when handed the Groups
/// view's tree, rebuilds it from the lazily loaded entries. Handed the tree
/// built from the full index, that cut Groups down to the folders the user
/// had expanded (10 groups of halo2_mcc's 120). The browser now hands it
/// over only when there is no full index; handed nothing, it is left alone.
#[test]
fn expanding_a_lazy_folder_leaves_a_full_index_group_tree_alone() {
    let root = std::env::temp_dir().join(format!(
        "baboon-lazy-groups-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("empty_folder")).unwrap();
    let full_index = vec![TagEntry {
        key: "objects/characters/masterchief/masterchief.biped".to_owned(),
        display_path: "objects/characters/masterchief/masterchief.biped".to_owned(),
        group_tag: u32::from_be_bytes(*b"bipd"),
        group_name: None,
        location: TagEntryLocation::LooseFile(
            root.join("objects/characters/masterchief/masterchief.biped"),
        ),
    }];
    let ancestors = vec!["empty_folder".to_owned()];

    let expand = |hand_over: bool| -> usize {
        let mut tree = crate::core::source::build_folder_directory_tree(&root).unwrap();
        let mut entries = Vec::new();
        let mut group_tree = crate::core::source::build_group_tree(&full_index);
        assert_eq!(group_tree.children.len(), 1);
        let ctx = egui::Context::default();
        let mut requests = Vec::new();
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                draw_tree_lazy(
                    ui,
                    &tree,
                    &entries,
                    None,
                    "",
                    false,
                    false,
                    &mut requests,
                    // Reveal opens the folder, which loads it.
                    Some(Reveal {
                        key: "unused",
                        remaining: &ancestors,
                    }),
                    BrowserSort::default(),
                    true,
                    None,
                );
            });
        });
        load_lazy_folders(
            &mut tree,
            &mut entries,
            hand_over.then_some(&mut group_tree),
            &root,
            &TagNameIndex::default(),
            &requests,
        );
        assert!(
            tree.children.iter().any(|node| node.entries_loaded),
            "the folder was expanded and loaded"
        );
        group_tree.children.len()
    };

    assert_eq!(
        expand(true),
        0,
        "handed over, it is rebuilt from the lazy entries (the bug's mechanism)"
    );
    assert_eq!(
        expand(false),
        1,
        "not handed over, the full-index tree keeps its groups"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn hover_paths_use_the_host_separator() {
    let separator = std::path::MAIN_SEPARATOR;
    let expected = ["objects", "characters", "brute.model"].join(&separator.to_string());
    assert_eq!(
        native_display_path("objects/characters\\brute.model"),
        expected
    );
}

fn entry(location: TagEntryLocation) -> TagEntry {
    TagEntry {
        key: "test".to_owned(),
        display_path: "objects/example.model".to_owned(),
        group_tag: 0,
        group_name: None,
        location,
    }
}

#[test]
fn loaded_extractables_include_direct_files_and_descendants_only() {
    let loose_entry = |path: &str, group_tag: u32| TagEntry {
        key: path.to_owned(),
        display_path: path.to_owned(),
        group_tag,
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from(path)),
    };
    let entries = vec![
        loose_entry("sound/direct.sound", u32::from_be_bytes(*b"snd!")),
        loose_entry(
            "sound/characters/nested.sound",
            u32::from_be_bytes(*b"snd!"),
        ),
        loose_entry("sound/direct.bitmap", u32::from_be_bytes(*b"bitm")),
        loose_entry("other/unrelated.sound", u32::from_be_bytes(*b"snd!")),
    ];

    let (_, bitmaps, sounds, shaders, includes) =
        collect_loaded_extractable_keys(&entries, Path::new("sound"), false);

    assert_eq!(bitmaps, vec!["sound/direct.bitmap"]);
    assert_eq!(
        sounds,
        vec!["sound/direct.sound", "sound/characters/nested.sound"]
    );
    assert!(shaders.is_empty());
    assert!(includes.is_empty());
}

/// Only a hovered bitmap row asks for its preview thumbnail. Every bitmap
/// row laid out used to, so expanding a folder read and decoded all of its
/// bitmaps in the background, and repainted until they were done.
#[test]
fn only_a_hovered_bitmap_row_requests_its_thumbnail() {
    let entries: Vec<TagEntry> = (0..20)
        .map(|index| TagEntry {
            key: format!("file:bitmaps/b{index:02}.bitmap"),
            display_path: format!("b{index:02}.bitmap"),
            group_tag: u32::from_be_bytes(*b"bitm"),
            group_name: Some("bitmap".to_owned()),
            location: TagEntryLocation::LooseFile(PathBuf::from(format!("b{index:02}.bitmap"))),
        })
        .collect();
    let tree = crate::core::source::build_tree(&entries);
    let ctx = egui::Context::default();
    let requests_with_pointer_at = |pointer: Option<egui::Pos2>| {
        let mut requested = 0;
        // Twice: hover is decided from the previous frame's layout.
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(300.0, 900.0),
                )),
                events: pointer.map(egui::Event::PointerMoved).into_iter().collect(),
                ..Default::default()
            };
            let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let requests = crate::app::browser::begin_bitmap_hovers(
                        ui,
                        std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
                    );
                    draw_tree(
                        ui,
                        &tree,
                        &entries,
                        None,
                        "",
                        false,
                        false,
                        false,
                        false,
                        None,
                        BrowserSort::Natural,
                        false,
                        None,
                        false,
                    );
                    requested = requests.lock().unwrap().len();
                });
            });
        }
        requested
    };

    assert_eq!(requests_with_pointer_at(Some(egui::pos2(290.0, 890.0))), 0);
    assert_eq!(requests_with_pointer_at(Some(egui::pos2(60.0, 20.0))), 1);
}

/// A folder stays open when "Show prefixes" changes its label. Its open
/// state was keyed on the label, which gains a `[folder]` prefix.
#[test]
fn a_folder_stays_open_when_prefixes_are_shown() {
    let ctx = egui::Context::default();
    let body_drawn = |label: &str, open_first: bool| {
        let mut drawn = false;
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                if open_first {
                    let id = ui.make_persistent_id("objects");
                    let mut state =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            id,
                            false,
                        );
                    state.set_open(true);
                    state.store(ui.ctx());
                }
                show_folder_tree_header(
                    ui,
                    "objects",
                    label,
                    text_dark(),
                    false,
                    false,
                    |_| {
                        drawn = true;
                    },
                );
            });
        });
        drawn
    };

    assert!(body_drawn("objects", true), "opened");
    assert!(
        body_drawn("[folder] objects", false),
        "still open with the prefix shown"
    );
}

/// Draw one browser tree and report how much vertical space it left unused.
fn unused_height_after_tree(is_container: bool, groups_mode: bool) -> f32 {
    let entries = vec![entry(TagEntryLocation::Container {
        container: 0,
        rel_path: "Tags/objects/example-hlmt.ubulk".to_owned(),
    })];
    let tree = crate::core::source::build_tree(&entries);
    let ctx = egui::Context::default();
    let mut left = 0.0;
    let _ = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(300.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                draw_tree(
                    ui,
                    &tree,
                    &entries,
                    None,
                    "",
                    false,
                    false,
                    false,
                    groups_mode,
                    None,
                    BrowserSort::Natural,
                    false,
                    None,
                    is_container,
                );
                left = ui.available_size_before_wrap().y;
            });
        },
    );
    left
}

/// The container root has no tree node — a real node's `rel_path` is never
/// empty — so the only way to author into it is a gesture on the browser's
/// empty space. If that space stops being claimed, `folder_rel: None`
/// becomes unreachable again and root-level New Tag / New Folder silently
/// disappear, with nothing failing to say so.
#[test]
fn a_container_browser_claims_its_empty_space_so_the_root_is_reachable() {
    assert!(
        unused_height_after_tree(true, false) < 1.0,
        "a container tree must leave no unclaimed space below it"
    );
    // A loose folder has no container root to author into, and Groups mode
    // node paths are group labels rather than folders.
    assert!(unused_height_after_tree(false, false) > 100.0);
    assert!(unused_height_after_tree(true, true) > 100.0);
}

/// A tree the folder pane has already filtered is drawn with an empty
/// query, so the query no longer opens its folders; `expand_folders`
/// does. Measured by height: an open folder lays out its tags.
#[test]
fn a_prefiltered_tree_opens_its_folders_when_asked() {
    let entries: Vec<TagEntry> = (0..6)
        .map(|index| TagEntry {
            key: format!("file:objects/weapons/rifle_{index}.weapon"),
            display_path: format!("objects/weapons/rifle_{index}.weapon"),
            group_tag: u32::from_be_bytes(*b"weap"),
            group_name: Some("weapon".to_owned()),
            location: TagEntryLocation::LooseFile(PathBuf::from(format!(
                "objects/weapons/rifle_{index}.weapon"
            ))),
        })
        .collect();
    let tree = crate::core::source::build_tree(&entries);
    let height = |expand_folders: bool| {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        let _ = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let top = ui.cursor().top();
                draw_tree(
                    ui,
                    &tree,
                    &entries,
                    None,
                    "",
                    expand_folders,
                    false,
                    false,
                    false,
                    None,
                    BrowserSort::Natural,
                    false,
                    None,
                    false,
                );
                height = ui.cursor().top() - top;
            });
        });
        height
    };
    let (closed, open) = (height(false), height(true));
    assert!(
        open > closed + 5.0 * 16.0,
        "expanded {open} vs collapsed {closed}: the folders did not open"
    );
}

/// The path popup waits for the pointer to rest, as egui's own tooltips
/// do, then follows the pointer from row to row without waiting again,
/// and a click hides it until the pointer moves.
#[test]
fn the_path_popup_waits_for_the_pointer_to_rest() {
    let ctx = egui::Context::default();
    let rows = std::cell::Cell::new([egui::Rect::NOTHING; 2]);
    let mut time = 0.0;
    let mut frame = |dt: f64, events: Vec<egui::Event>| -> Vec<String> {
        time += dt;
        let output = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(600.0, 400.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut rects = [egui::Rect::NOTHING; 2];
                    for (index, rect) in rects.iter_mut().enumerate() {
                        let (row, response) = ui.allocate_exact_size(
                            Vec2::new(240.0, 20.0),
                            Sense::click_and_drag(),
                        );
                        *rect = row;
                        hover_tooltip_beside_pointer(ui, &response, &format!("path/{index}"));
                    }
                    rows.set(rects);
                });
            },
        );
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text().starts_with("path/") => {
                    Some(text.galley.text().to_owned())
                }
                _ => None,
            })
            .collect()
    };
    frame(0.0, Vec::new());
    let [first, second] = rows.get().map(|row| row.center());
    let delay = f64::from(egui::Style::default().interaction.tooltip_delay);
    // egui counts the pointer as moving only once it has a few positions
    // behind it, as a real mouse does: slide in from the empty space to
    // the right of the rows rather than jumping onto one.
    for x in [500.0, 430.0, 360.0, 290.0] {
        frame(
            0.01,
            vec![egui::Event::PointerMoved(egui::pos2(x, first.y))],
        );
    }
    assert!(
        frame(0.01, vec![egui::Event::PointerMoved(first)]).is_empty(),
        "the popup showed the moment the pointer arrived"
    );
    assert!(
        frame(delay * 0.5, Vec::new()).is_empty(),
        "the popup showed before the pointer had rested"
    );
    assert_eq!(frame(delay * 0.6, Vec::new()), ["path/0"]);
    assert_eq!(
        frame(0.01, vec![egui::Event::PointerMoved(second)]),
        ["path/1"],
        "moving to the next row while a popup is up should not wait again"
    );
    let button = |pressed| egui::Event::PointerButton {
        pos: second,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(0.01, vec![button(true)]);
    assert!(
        frame(0.01, vec![button(false)]).is_empty(),
        "a click should hide the popup"
    );
    assert!(
        frame(delay * 2.0, Vec::new()).is_empty(),
        "resting after a click should not bring the popup back"
    );
}

/// Control for the regression test below: the same pointer script against
/// a bare egui drag source, no Baboon code. Proves the synthetic events
/// can start a drag at all, so the test below indicts `draw_entry` rather
/// than the script.
#[test]
fn control_a_bare_drag_source_sets_its_payload() {
    let ctx = egui::Context::default();
    let mut source_rect = egui::Rect::NOTHING;
    let frame = |events: Vec<egui::Event>, source_rect: &mut egui::Rect| {
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(600.0, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(240.0, 20.0), Sense::click_and_drag());
                    *source_rect = rect;
                    // The non-blocking tooltip the real rows use; the raw
                    // `on_hover_text` here is what broke them (an egui
                    // 0.29 tooltip is interactable and owns the pointer's
                    // hit-test once shown, so the press never reaches the
                    // row and no drag starts).
                    hover_tooltip_beside_pointer(ui, &response, "objects/example.bitmap");
                    response.dnd_set_drag_payload(DraggedTagRef {
                        group_tag: 0,
                        input: String::new(),
                        rel_path: "control".to_owned(),
                        file_path: None,
                    });
                });
            },
        );
    };
    frame(Vec::new(), &mut source_rect);
    let start = source_rect.center();
    frame(vec![egui::Event::PointerMoved(start)], &mut source_rect);
    frame(
        vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        &mut source_rect,
    );
    frame(
        vec![egui::Event::PointerMoved(start + Vec2::new(0.0, 40.0))],
        &mut source_rect,
    );
    frame(
        vec![egui::Event::PointerMoved(start + Vec2::new(0.0, 80.0))],
        &mut source_rect,
    );
    assert!(
        egui::DragAndDrop::has_any_payload(&ctx),
        "even a bare egui drag source set no payload — the event script is wrong",
    );
}

/// Regression (user report: dragging textures into shader image fields
/// stopped working): a browser row dragged onto a shader-style drop cell
/// must deliver its `DraggedTagRef`. Drives the real drag source —
/// `draw_entry` — with real pointer events, into the same
/// TextEdit-then-hover-interact structure the shader bitmap cell uses.
#[test]
fn a_dragged_row_delivers_its_payload_to_a_reference_cell() {
    let bitm = TagEntry {
        key: "objects/example.bitmap".to_owned(),
        display_path: "objects/example.bitmap".to_owned(),
        group_tag: u32::from_be_bytes(*b"bitm"),
        group_name: Some("bitmap".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from(
            "C:/kit/tags/objects/example.bitmap",
        )),
    };
    let ctx = egui::Context::default();
    let mut row_rect = egui::Rect::NOTHING;
    let mut target_rect = egui::Rect::NOTHING;
    let mut hover_seen = false;
    let mut dropped: Option<String> = None;

    let frame = |events: Vec<egui::Event>,
                 row_rect: &mut egui::Rect,
                 target_rect: &mut egui::Rect,
                 hover_seen: &mut bool,
                 dropped: &mut Option<String>| {
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(600.0, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let row_top = ui.cursor().min;
                    draw_entry(ui, &bitm, None, false, false, None, None, true);
                    *row_rect = egui::Rect::from_min_size(
                        row_top,
                        Vec2::new(240.0, ui.spacing().interact_size.y),
                    );
                    ui.add_space(120.0);
                    // The shader bitmap cell's structure: a TextEdit in the
                    // cell rect, then a hover interact over the same rect
                    // asking for the payload.
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(220.0, 22.0), Sense::hover());
                    *target_rect = rect;
                    let mut text = String::new();
                    ui.put(
                        rect,
                        egui::TextEdit::singleline(&mut text)
                            .hint_text(placeholder_text("(no reference)")),
                    );
                    let drop =
                        ui.interact(rect, ui.make_persistent_id("test_drop"), Sense::hover());
                    if drop.dnd_hover_payload::<DraggedTagRef>().is_some() {
                        *hover_seen = true;
                    }
                    if let Some(payload) = drop.dnd_release_payload::<DraggedTagRef>() {
                        *dropped = Some(payload.rel_path.clone());
                    }
                });
            },
        );
    };

    // Lay out once to learn the rects, then press on the row, drag past
    // egui's drag threshold, cross onto the cell, and release there.
    frame(
        Vec::new(),
        &mut row_rect,
        &mut target_rect,
        &mut hover_seen,
        &mut dropped,
    );
    let start = row_rect.center();
    let end = target_rect.center();
    frame(
        vec![egui::Event::PointerMoved(start)],
        &mut row_rect,
        &mut target_rect,
        &mut hover_seen,
        &mut dropped,
    );
    frame(
        vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        &mut row_rect,
        &mut target_rect,
        &mut hover_seen,
        &mut dropped,
    );
    frame(
        vec![egui::Event::PointerMoved(start + Vec2::new(0.0, 40.0))],
        &mut row_rect,
        &mut target_rect,
        &mut hover_seen,
        &mut dropped,
    );
    frame(
        vec![egui::Event::PointerMoved(end)],
        &mut row_rect,
        &mut target_rect,
        &mut hover_seen,
        &mut dropped,
    );
    assert!(
        egui::DragAndDrop::has_any_payload(&ctx),
        "the row never set a drag payload — the drag itself is not starting",
    );
    frame(
        vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        &mut row_rect,
        &mut target_rect,
        &mut hover_seen,
        &mut dropped,
    );

    assert!(
        hover_seen,
        "the drop cell never saw the drag payload while hovered",
    );
    assert_eq!(
        dropped.as_deref(),
        Some(entry_rel_path(&bitm).as_str()),
        "releasing over the cell must deliver the dragged bitmap's path",
    );
}

/// The value a drop writes must be one the edit applier accepts. The
/// shader bitmap cell appends `.bitmap` to the payload's rel_path, and
/// structural cells use the payload's `GROUP:path` input form — the bare
/// rel_path is refused ("expected <path>.<group> or GROUP:<path>"), which
/// is exactly the error users hit dropping a bitmap onto base_map.
#[test]
fn drop_payload_forms_survive_the_reference_parser() {
    use crate::app::editor::parse_tag_reference;
    let bitm = TagEntry {
        key: "objects/example.bitmap".to_owned(),
        display_path: "objects/example.bitmap".to_owned(),
        group_tag: u32::from_be_bytes(*b"bitm"),
        group_name: Some("bitmap".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from(
            "C:/kit/tags/objects/example.bitmap",
        )),
    };
    let expected = Some((u32::from_be_bytes(*b"bitm"), "objects\\example".to_owned()));

    let bare = entry_rel_path(&bitm);
    assert!(
        parse_tag_reference(&bare).is_err(),
        "a bare rel_path is not a valid reference, so no drop site may push it",
    );
    let suffixed = format!("{bare}.bitmap");
    assert_eq!(
        parse_tag_reference(&suffixed)
            .expect("suffixed form parses")
            .group_tag_and_name,
        expected,
        "the shader bitmap cell's dropped value",
    );
    assert_eq!(
        parse_tag_reference(&entry_reference_input(&bitm))
            .expect("GROUP:path form parses")
            .group_tag_and_name,
        expected,
        "the structural cell's dropped value",
    );
}

#[test]
fn duplicate_menu_is_limited_to_writable_on_disk_entries() {
    assert!(supports_duplicate_menu(&entry(
        TagEntryLocation::LooseFile(PathBuf::from("objects/example.model"))
    )));
    assert!(supports_duplicate_menu(&entry(
        TagEntryLocation::Container {
            container: 0,
            rel_path: "Tags/objects/example-hlmt.ubulk".to_owned(),
        }
    )));
    assert!(!supports_duplicate_menu(&entry(
        TagEntryLocation::Monolithic {
            name: "objects/example".to_owned(),
            group_tag: 0,
        }
    )));
    assert!(!supports_duplicate_menu(&entry(
        TagEntryLocation::NewContainer {
            template: NewContainerTemplate::Donor {
                container: 0,
                rel_path: "Tags/template-hlmt.uasset".to_owned(),
            },
            package: "/Game/Tags/example-hlmt".to_owned(),
            group_tag: 0,
        }
    )));
}

#[test]
fn delete_menu_is_offered_for_loose_tags_and_recorded_container_copies_only() {
    let recorded =
        HashSet::from(["ublock:pakchunk240-WinGDK:Tags/copy-biped.ubulk".to_owned()]);
    let copy = TagEntry {
        key: "ublock:pakchunk240-WinGDK:Tags/copy-biped.ubulk".to_owned(),
        ..entry(TagEntryLocation::Container {
            container: 0,
            rel_path: "Tags/copy-biped.ubulk".to_owned(),
        })
    };
    let shipped = entry(TagEntryLocation::Container {
        container: 0,
        rel_path: "Tags/shipped-biped.ubulk".to_owned(),
    });

    assert!(supports_delete_menu(
        &entry(TagEntryLocation::LooseFile(PathBuf::from(
            "objects/example.model"
        ))),
        Some(&recorded)
    ));
    assert!(supports_delete_menu(&copy, Some(&recorded)));
    // A tag the game shipped is byte-for-byte as legitimate as a copy, so
    // the ledger is the only thing that may enable this.
    assert!(!supports_delete_menu(&shipped, Some(&recorded)));
    assert!(!supports_delete_menu(&copy, None));
    assert!(!supports_delete_menu(
        &entry(TagEntryLocation::Monolithic {
            name: "objects/example".to_owned(),
            group_tag: 0,
        }),
        Some(&recorded)
    ));
    assert!(!supports_delete_menu(
        &entry(TagEntryLocation::NewContainer {
            template: NewContainerTemplate::Donor {
                container: 0,
                rel_path: "Tags/template-hlmt.uasset".to_owned(),
            },
            package: "/Game/Tags/example-hlmt".to_owned(),
            group_tag: 0,
        }),
        Some(&recorded)
    ));
}

#[test]
fn delete_context_button_uses_the_garbage_icon() {
    assert_eq!(context_menu_icon("Delete"), Some(ButtonIcon::Garbage));
}

#[test]
fn duplicate_context_button_uses_duplicate_asset_icon() {
    assert_eq!(context_menu_icon("Duplicate"), Some(ButtonIcon::Duplicate));
}

#[test]
fn reimport_context_button_uses_import_icon() {
    assert_eq!(context_menu_icon("Reimport"), Some(ButtonIcon::Import));
}

#[test]
fn folder_context_commands_have_matching_icons() {
    assert_eq!(context_menu_icon("Move to..."), Some(ButtonIcon::Move));
    assert_eq!(context_menu_icon("Copy to..."), Some(ButtonIcon::Copy));
    assert_eq!(
        context_menu_icon("Copy Folder Path"),
        Some(ButtonIcon::CopyPath)
    );
    assert_eq!(
        context_menu_icon("Import tags here..."),
        Some(ButtonIcon::Import)
    );
    assert_eq!(
        context_menu_icon("New folder here..."),
        Some(ButtonIcon::FolderClosed)
    );
    assert_eq!(
        context_menu_icon("Dump folder to JSON... (12)"),
        Some(ButtonIcon::Json)
    );
    assert_eq!(
        context_menu_icon("Extract loaded HLSL includes... (3)"),
        Some(ButtonIcon::Export)
    );
}

#[test]
fn reimport_menu_matches_the_reference_field_tag_types() {
    let loose = TagEntryLocation::LooseFile(PathBuf::from(
        "objects/characters/example/example.render_model",
    ));
    for group_name in [
        "render_model",
        "collision_model",
        "physics_model",
        "model_animation_graph",
    ] {
        let candidate = TagEntry {
            group_name: Some(group_name.to_owned()),
            ..entry(loose.clone())
        };
        assert!(supports_tag_reimport(&candidate), "{group_name}");
    }

    let bitmap = TagEntry {
        group_name: Some("bitmap".to_owned()),
        ..entry(loose)
    };
    assert!(!supports_tag_reimport(&bitmap));
}

/// The count that decides whether a folder offers the cache import at all.
///
/// Recursive, and monolithic-only: a workspace can hold a mix after a
/// browser rebuild, and offering "import 40 tags" on a folder holding 3
/// cache tags and 37 loose ones would promise a run that converts 3.
#[test]
fn a_folder_counts_only_the_cache_tags_beneath_it() {
    let entries = vec![
        entry(TagEntryLocation::Monolithic {
            name: r"objects\weapons\rifle\assault_rifle".to_owned(),
            group_tag: u32::from_be_bytes(*b"weap"),
        }),
        entry(TagEntryLocation::Monolithic {
            name: r"objects\weapons\rifle\scope\scope".to_owned(),
            group_tag: u32::from_be_bytes(*b"weap"),
        }),
        entry(TagEntryLocation::LooseFile(PathBuf::from(
            "D:/HREK/tags/objects/weapons/rifle/assault_rifle.weapon",
        ))),
    ];
    let node = |rel: &str, entries: Vec<usize>| crate::core::source::TagTreeNode {
        label: rel.rsplit('/').next().unwrap_or(rel).to_owned(),
        rel_path: PathBuf::from(rel),
        children: Vec::new(),
        children_loaded: true,
        entries,
        entries_loaded: true,
        pending: false,
    };
    let mut tree = node("objects/weapons/rifle", vec![0, 2]);
    assert_eq!(
        count_cache_tags(&tree, &entries),
        1,
        "the loose tag was counted"
    );
    tree.children
        .push(node("objects/weapons/rifle/scope", vec![1]));
    assert_eq!(
        count_cache_tags(&tree, &entries),
        2,
        "a subfolder's tags were not counted"
    );
}

#[test]
fn sound_tags_are_extractable_and_collected_recursively() {
    let sound = |key: &str| TagEntry {
        key: key.to_owned(),
        display_path: format!("{key}.sound"),
        group_tag: u32::from_be_bytes(*b"snd!"),
        group_name: Some("sound".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from(format!(
            "C:/kit/tags/{key}.sound"
        ))),
    };
    let entries = vec![
        sound("sound/a"),
        entry(TagEntryLocation::LooseFile(PathBuf::from(
            "C:/kit/tags/objects/not_sound.model",
        ))),
        sound("sound/sub/b"),
    ];
    let node = |rel: &str, indices: Vec<usize>| crate::core::source::TagTreeNode {
        label: rel.rsplit('/').next().unwrap_or(rel).to_owned(),
        rel_path: PathBuf::from(rel),
        children: Vec::new(),
        children_loaded: true,
        entries: indices,
        entries_loaded: true,
        pending: false,
    };
    let mut root = node("sound", vec![0, 1]);
    root.children.push(node("sound/sub", vec![2]));

    assert!(supports_tag_extract_menu(u32::from_be_bytes(*b"snd!")));
    assert_eq!(
        collect_sound_keys(&root, &entries),
        vec!["sound/a".to_owned(), "sound/sub/b".to_owned()]
    );
}

#[test]
fn sound_menu_distinguishes_dialogue_from_shared_sfx_paths() {
    assert!(sound_key_may_have_languages(
        r"file:C:\kit\tags\sound\dialog\combat\brute.sound"
    ));
    assert!(!sound_key_may_have_languages(
        r"file:C:\kit\tags\sound\visual_fx\explosion.sound"
    ));
}
