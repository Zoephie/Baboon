use super::*;

fn payload(file: Option<&str>) -> DraggedTagRef {
    DraggedTagRef {
        group_tag: u32::from_be_bytes(*b"weap"),
        input: r"weap:objects\weapons\rifle\rifle".to_owned(),
        rel_path: "objects/weapons/rifle/rifle".to_owned(),
        file_path: file.map(PathBuf::from),
    }
}

const IN_KIT: &str = "/kits/h3ek/tags/objects/weapons/rifle/rifle.weapon";

fn plan(
    tool: KitTool,
    accepts_files: bool,
    tool_accepts_files: bool,
    file: Option<&str>,
) -> Result<KitToolDropPlan, String> {
    let mut app = Baboon::for_test();
    let target = KitToolDropTarget::for_tests(
        tool,
        Path::new("/kits/h3ek"),
        accepts_files,
        tool_accepts_files,
        0,
    );
    app.plan_kit_tool_drop(&target, &payload(file), &egui::Context::default())
}

/// The cursor a drag over a kit tool asks for is the one shown, over
/// egui's grabbing hand for a drag. Without a request the hand shows,
/// which is what the request has to win against.
#[test]
fn a_kit_tool_drop_cursor_outlasts_the_drag_hand() {
    let ctx = egui::Context::default();
    Baboon::configure_context(&ctx);
    let cursor_after = |requested: Option<egui::CursorIcon>| {
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::DragAndDrop::set_payload(ui.ctx(), "rifle.weapon");
            if let Some(cursor) = requested {
                let id = egui::Id::new(KIT_TOOL_DROP_CURSOR);
                ui.ctx().data_mut(|data| data.insert_temp(id, cursor));
            }
        });
        output.platform_output.cursor_icon
    };
    assert_eq!(cursor_after(None), egui::CursorIcon::Grabbing);
    assert_eq!(cursor_after(Some(egui::CursorIcon::Copy)), egui::CursorIcon::Copy);
    assert_eq!(
        cursor_after(Some(egui::CursorIcon::NotAllowed)),
        egui::CursorIcon::NotAllowed
    );
}

/// Every objection, in the order a drop meets them, and the plans that
/// pass. No Windows needed: the target is what the window lookup reports.
#[test]
fn a_drop_on_a_kit_tool_is_planned_or_refused_with_the_reason() {
    let refused = |result: Result<KitToolDropPlan, String>| result.err().unwrap();
    assert!(
        refused(plan(KitTool::Sapien, true, true, None))
            .starts_with("Only a tag on disk can be dropped into Sapien")
    );
    assert!(
        refused(plan(KitTool::Sapien, false, true, Some(IN_KIT)))
            .contains("drop rifle.weapon on Sapien's main window instead")
    );
    assert!(
        refused(plan(KitTool::Sapien, false, false, Some(IN_KIT)))
            .contains("Halo CE's and Halo 2's do not")
    );
    assert!(
        refused(plan(KitTool::Guerilla, false, false, Some(IN_KIT)))
            .contains("open rifle.weapon from Guerilla's File menu instead")
    );
    assert!(
        refused(plan(
            KitTool::Sapien,
            true,
            true,
            Some("/kits/hrek/tags/objects/weapons/rifle/rifle.weapon")
        ))
        .starts_with("rifle.weapon is not inside Sapien's editing kit")
    );

    // A kit Baboon has not loaded gets no palette gate: Sapien decides.
    let sapien = plan(KitTool::Sapien, true, true, Some(IN_KIT)).expect("planned");
    assert_eq!(sapien.file, PathBuf::from(IN_KIT));
    assert_eq!(sapien.palette, None);
    assert_eq!(
        hover_message(KitTool::Sapien, &sapien),
        "Release to hand rifle.weapon to Sapien"
    );
    let guerilla = plan(KitTool::Guerilla, true, true, Some(IN_KIT)).expect("planned");
    assert_eq!(guerilla.palette, None);
    assert_eq!(
        hover_message(KitTool::Guerilla, &guerilla),
        "Release to open rifle.weapon in Guerilla"
    );
    let with_palette = KitToolDropPlan {
        file: PathBuf::from(IN_KIT),
        palette: Some("weapons palette".to_owned()),
    };
    assert_eq!(
        hover_message(KitTool::Sapien, &with_palette),
        "Release to add rifle.weapon to Sapien's weapons palette"
    );
    assert_eq!(
        hover_message(KitTool::Guerilla, &with_palette),
        "Release to open rifle.weapon in Guerilla",
        "Guerilla has no palettes"
    );
}
