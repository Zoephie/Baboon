//! Static map-name catalogs and their presentation UI.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

mod data;
use data::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MapNamesGameTab {
    HaloCe,
    Halo2,
    Halo2Anniversary,
    Halo3,
    Halo3Odst,
    HaloReach,
    Halo4,
    Stubbs,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MapKind {
    Campaign,
    Multiplayer,
    Survival,
}

struct MapEntry {
    internal_name: &'static str,
    name: &'static str,
    map_id: &'static str,
    kind: MapKind,
}

pub(super) fn draw_map_names_tab(ui: &mut Ui, active_tab: &mut MapNamesGameTab) {
    ui.horizontal_wrapped(|ui| {
        for (tab, label) in MAP_TABS {
            ui.selectable_value(active_tab, *tab, *label);
        }
    });
    ui.add_space(8.0);

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (kind, title) in map_sections(*active_tab) {
                let entries = map_entries(*active_tab)
                    .iter()
                    .filter(|entry| entry.kind == *kind)
                    .collect::<Vec<_>>();
                if entries.is_empty() {
                    continue;
                }
                ui.label(
                    RichText::new(format!("{title};"))
                        .color(subtle_dark())
                        .font(FontId::proportional(14.0))
                        .strong(),
                );
                ui.add_space(4.0);
                egui::Grid::new(("map_names_grid", title))
                    .num_columns(3)
                    .spacing(Vec2::new(28.0, 4.0))
                    .striped(false)
                    .show(ui, |ui| {
                        for entry in entries {
                            map_cell(ui, entry.map_id, 76.0);
                            map_cell(ui, entry.internal_name, 170.0);
                            map_cell(ui, entry.name, 260.0);
                            ui.end_row();
                        }
                    });
                ui.add_space(16.0);
            }
        });
}

fn map_cell(ui: &mut Ui, text: &str, width: f32) {
    ui.add_sized(
        Vec2::new(width, 18.0),
        egui::Label::new(RichText::new(text).color(foundation_blue())),
    );
}
