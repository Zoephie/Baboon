use super::*;
use crate::app::browser::draw_entry;
use crate::app::editor::fields::extracted_tests::tests::with_test_edit_context;

/// Drag `entry` from a real browser row onto a real shader reference cell
/// of `kind`, and return the field edits the cell committed.
fn drop_onto(kind: ShaderRowEditKind, entry: &TagEntry, editable: bool) -> Vec<String> {
    let row_edit = ShaderRowEdit {
        path: "field".to_owned(),
        current: String::new(),
        kind,
    };
    let ctx = egui::Context::default();
    let row_rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let cell_rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut committed = Vec::new();
    let mut frame = |events: Vec<egui::Event>| {
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
                    let top = ui.cursor().min;
                    draw_entry(ui, entry, None, false, false, None, None, true);
                    row_rect.set(egui::Rect::from_min_size(
                        top,
                        Vec2::new(240.0, ui.spacing().interact_size.y),
                    ));
                    ui.add_space(120.0);
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(360.0, 22.0), Sense::hover());
                    // The cell's text box, left of Open and browse.
                    cell_rect.set(egui::Rect::from_min_size(rect.min, Vec2::new(200.0, 22.0)));
                    with_test_edit_context(|edit| {
                        edit.editable = editable;
                        draw_shader_editable_value(
                            ui,
                            rect,
                            "Reference",
                            &row_edit,
                            edit,
                            &mut None,
                        );
                        committed.extend(edit.pending.iter().map(|edit| edit.input.clone()));
                    });
                });
            },
        );
    };
    frame(Vec::new());
    let (start, end) = (row_rect.get().center(), cell_rect.get().center());
    frame(vec![egui::Event::PointerMoved(start)]);
    frame(vec![egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    }]);
    frame(vec![egui::Event::PointerMoved(
        start + Vec2::new(0.0, 40.0),
    )]);
    frame(vec![egui::Event::PointerMoved(end)]);
    frame(vec![egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    committed
}

fn entry(path: &str, group: &[u8; 4]) -> TagEntry {
    TagEntry {
        key: format!("file:{path}"),
        display_path: path.to_owned(),
        group_tag: u32::from_be_bytes(*group),
        group_name: None,
        location: TagEntryLocation::LooseFile(std::path::PathBuf::from(path)),
    }
}

/// Every kind of reference cell takes a dropped tag of its own group, in
/// its own committed form, through the one shared cell.
#[test]
fn each_reference_cell_takes_a_dropped_tag() {
    let bitmap = entry("objects/rifle/rifle.bitmap", b"bitm");
    assert_eq!(
        drop_onto(
            ShaderRowEditKind::BitmapRef {
                group_tag: u32::from_be_bytes(*b"bitm"),
                create: None,
            },
            &bitmap,
            true,
        ),
        ["objects/rifle/rifle.bitmap"],
    );
    let template = entry("shaders/opaque.shader_template", b"stem");
    assert_eq!(
        drop_onto(ShaderRowEditKind::ShaderTemplateRef, &template, true),
        ["stem:shaders\\opaque"],
    );
    let definition = entry("shaders/shader.render_method_definition", b"rmdf");
    let structural = || ShaderRowEditKind::StructuralRef {
        group_tag: u32::from_be_bytes(*b"rmdf"),
        extension: "render_method_definition",
    };
    let dropped = drop_onto(structural(), &definition, true);
    assert_eq!(dropped.len(), 1, "{dropped:?}");

    // A tag of another group is refused, and so is any drop on a
    // read-only tag — which only the bitmap cell used to refuse.
    assert!(drop_onto(structural(), &bitmap, true).is_empty());
    assert!(drop_onto(ShaderRowEditKind::ShaderTemplateRef, &template, false).is_empty());
}
