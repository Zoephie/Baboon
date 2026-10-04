use super::*;

thread_local! {
    pub(super) static TREES_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn row(path: &str, a: &str, b: &str) -> TagFieldDiff {
    TagFieldDiff {
        path: path.to_owned(),
        base_path: None,
        a: a.to_owned(),
        b: b.to_owned(),
    }
}

fn rows() -> Vec<TagFieldDiff> {
    vec![
        row("flags", "0", "1"),
        row("zone sets[2]", "old", "new"),
        row("zone sets[2]/name", "a", "b"),
        row("zone sets[2]/bsp flags", "0", "4"),
        row("zone set pvs[3]", "pvs", ""),
        row("zone set pvs[3]/bsp mask", "1", ""),
    ]
}

/// Each section's filter is built once its rows are complete, and is the
/// filter those rows describe.
#[test]
fn each_section_keeps_the_filter_its_rows_describe() {
    let sections = Baboon::build_diff_sections(&rows());
    assert!(sections.len() >= 3);
    for section in &sections {
        assert_eq!(
            section.filter.visible_paths,
            Baboon::diff_field_filter(&section.rows).visible_paths,
            "{}",
            section.element
        );
    }
}

/// The review draws a tag's diff every frame; its tree is built on the
/// first and reused after.
#[test]
fn a_reviewed_diff_is_arranged_once() {
    let diff = ModRowDiff {
        rows: rows(),
        base: None,
        edited: None,
        truncated: false,
        error: None,
        view: Default::default(),
    };
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::app::foundation_fonts());
    TREES_BUILT.with(|built| built.set(0));
    for _ in 0..3 {
        let _ = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                Baboon::draw_mod_export_diff(
                    ui,
                    &diff,
                    &TagNameIndex::default(),
                    u32::from_be_bytes(*b"scnr"),
                    None,
                    None,
                    false,
                    "review",
                );
            });
        });
    }
    assert_eq!(TREES_BUILT.with(std::cell::Cell::get), 1);
}
