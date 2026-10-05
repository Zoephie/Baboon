use std::path::{Path, PathBuf};

use super::*;

#[test]
fn restored_loose_tag_uses_the_current_sources_key() {
    let root = std::env::temp_dir().join(format!("baboon-session-key-{}", std::process::id()));
    let path = root.join("objects").join("characters").join("brute.model");
    std::fs::create_dir_all(path.parent().expect("tag has parent")).expect("create tag path");
    std::fs::write(&path, b"tag").expect("create tag");
    let canonical = std::fs::canonicalize(&path).expect("canonical tag path");
    let entry = crate::core::source::TagEntry {
        key: file_entry_key(&canonical),
        display_path: "objects/characters/brute.model".to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: crate::core::source::TagEntryLocation::LooseFile(canonical.clone()),
    };

    assert_eq!(
        super::loose_entry_key_for_canonical_path(std::iter::once(&entry), &canonical),
        Some(entry.key.clone())
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn last_opened_workspace_heading_prefers_the_named_project() {
    let source = PathBuf::from("Games").join("Halo Infinite");
    let project = PathBuf::from("Baboon Projects").join("Campaign Overhaul.baboon");

    assert_eq!(
        super::last_opened_workspace_heading(
            None,
            Some("halo_infinite"),
            &source,
            Some(&project)
        ),
        (
            "Campaign Overhaul".to_owned(),
            Some(project.display().to_string())
        )
    );
}

#[test]
fn last_opened_workspace_heading_keeps_the_source_fallback() {
    let source = Path::new(r"C:\Editing Kits\Custom Kit");

    assert_eq!(
        super::last_opened_workspace_heading(None, None, source, None),
        (source.display().to_string(), None)
    );
}

#[test]
fn last_opened_workspace_heading_prefers_the_custom_editing_kit_profile() {
    let source = Path::new(r"C:\Editing Kits\H2EK");

    assert_eq!(
        super::last_opened_workspace_heading(
            Some(("Halo 2 Rebalance", source)),
            Some("halo2_mcc"),
            source,
            None
        ),
        (
            "Halo 2 Rebalance".to_owned(),
            Some(source.display().to_string())
        )
    );
}

fn restore_test_prompt(folder_count: usize) -> LastOpenedWindowsPrompt {
    LastOpenedWindowsPrompt::from_session(
        LastSessionState {
            kits: vec![LastSessionKit {
                source_kind: LastSessionSourceKind::LooseFolder,
                source_path: std::env::temp_dir(),
                game: None,
                profile_id: None,
                project_path: None,
                has_project: false,
                browser_mode: None,
                browser_sort: None,
                tags: Vec::new(),
                folders: (0..folder_count)
                    .map(|index| LastSessionFolder {
                        rel_path: PathBuf::from(format!(
                            "objects/characters/brute/folder{index}"
                        )),
                        label: format!("folder{index}"),
                    })
                    .collect(),
                chimp_packages: Vec::new(),
                active_chimp_package: None,
                bitmap_library_open: false,
                model_library_open: false,
                was_active: false,
            }],
        },
        &[],
    )
    .unwrap()
}

#[test]
fn restore_dialog_hugs_contents_and_stays_stable_during_width_resizing() {
    let ctx = egui::Context::default();
    ctx.set_fonts(foundation_fonts());
    ctx.set_global_style(foundation_style());
    let mut prompt = restore_test_prompt(20);
    let frame = |prompt: &mut LastOpenedWindowsPrompt, events| {
        let _ = crate::app::run_ui_test(&ctx, 
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1200.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                super::render_last_opened_windows_prompt(ui.ctx(), Some(prompt));
            },
        );
        ctx.memory(|memory| {
            memory
                .area_rect(egui::Id::new("last_opened_windows"))
                .unwrap()
        })
    };
    for _ in 0..5 {
        frame(&mut prompt, Vec::new());
    }
    let initial = frame(&mut prompt, Vec::new());
    assert!(
        initial.height() < 700.0,
        "long lists must be capped and scroll"
    );
    let pointer = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    for direction in [1.0, -1.0] {
        let before = frame(&mut prompt, Vec::new());
        let edge = egui::pos2(before.right() - 1.0, before.center().y);
        frame(
            &mut prompt,
            vec![egui::Event::PointerMoved(edge), pointer(edge, true)],
        );
        for step in 1..=40 {
            let pos = edge + egui::vec2(direction * step as f32 * 2.0, 0.0);
            let during = frame(&mut prompt, vec![egui::Event::PointerMoved(pos)]);
            assert!(
                (during.height() - initial.height()).abs() < 1.0,
                "width drag changed height from {} to {}",
                initial.height(),
                during.height()
            );
        }
        let pos = edge + egui::vec2(direction * 80.0, 0.0);
        frame(&mut prompt, vec![pointer(pos, false)]);
        let after = frame(&mut prompt, Vec::new());
        assert!(
            (after.width() - before.width()).abs() > 20.0,
            "the test must actually change the width"
        );
        for _ in 0..10 {
            assert!((frame(&mut prompt, Vec::new()).height() - initial.height()).abs() < 1.0);
        }
    }
    prompt.kits[0].folder_entries.truncate(2);
    for _ in 0..5 {
        frame(&mut prompt, Vec::new());
    }
    let short = frame(&mut prompt, Vec::new());
    assert!(
        short.height() < 270.0,
        "short lists must hug all rows: {short:?}"
    );
    assert!(initial.height() - short.height() > 300.0);
}

#[test]
fn restore_footer_fill_reaches_window_edges_without_content_clipping() {
    let ctx = egui::Context::default();
    ctx.set_fonts(foundation_fonts());
    ctx.set_global_style(foundation_style());
    ctx.set_visuals(foundation_visuals());
    let mut prompt = restore_test_prompt(2);
    let mut output = None;
    for _ in 0..6 {
        output = Some(crate::app::run_ui_test(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO, Vec2::new(1200.0, 800.0),
            )),
            ..Default::default()
        }, |ui| {
            super::render_last_opened_windows_prompt(ui.ctx(), Some(&mut prompt));
        }));
    }
    let window = ctx.memory(|memory| {
        memory.area_rect(egui::Id::new("last_opened_windows")).unwrap()
    });
    let output = output.unwrap();
    let (clip, footer) = output.shapes.iter().find_map(|clipped| {
        match &clipped.shape {
            egui::Shape::Rect(rect) if rect.corner_radius.nw == 0
                && rect.corner_radius.ne == 0 && rect.corner_radius.sw > 0
                && rect.corner_radius.se > 0 => Some((clipped.clip_rect, rect)),
            _ => None,
        }
    }).expect("footer background with rounded bottom corners");
    assert!((footer.rect.left() - window.left()).abs() < 1.0);
    assert!((footer.rect.right() - window.right()).abs() < 1.0);
    assert!((footer.rect.bottom() - window.bottom()).abs() < 1.0);
    assert!(clip.contains_rect(footer.rect), "the content clip must not inset the footer fill");
}

#[test]
fn restore_workspace_heading_centers_both_lines_with_padding() {
    let ctx = egui::Context::default();
    ctx.set_fonts(foundation_fonts());
    ctx.set_global_style(foundation_style());
    let mut prompt = restore_test_prompt(0);
    let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.scope(|ui| {
                let (row, text) = super::restore_workspace_row(ui, &mut prompt.kits[0]);
                assert!((row.center().y - text.center().y).abs() < 0.01);
                assert!(text.top() - row.top() >= 6.0);
                assert!(row.bottom() - text.bottom() >= 6.0);
                assert!(
                    ui.min_rect().bottom() <= row.bottom() + 1.0,
                    "heading widgets must not extend the allocated row"
                );
            });
        });
    });
}
