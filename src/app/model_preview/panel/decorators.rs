//! A decorator set's cards: what the set is, its types to show or hide, and
//! the patch its placements fill (`model_preview::decorators`).

use super::super::super::editor::fields::BoxPart;
use super::super::super::ui_kit::style::{subtle_dark, text_dark};
use super::super::decorators::{DecoratorSet, DetailFacing, Layout};
use super::super::{ModelPreviewData, ModelPreviewState, ModelRegionSelection};
use super::functions::setup_section;
use super::rows::{CardRows, name_cell, name_column_width, row_font};
use crate::app::ui_kit::checkbox::CheckboxUi as _;
use eframe::egui;
use eframe::egui::{RichText, Ui};

/// Past this many placements a patch takes a while to lay out, bake and draw.
const LARGE_PATCH: usize = 100_000;

/// The cards of an opened decorator set, in place of the Permutations card.
pub(in crate::app::model_preview) fn draw_decorator_cards(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let Some(set) = data.preview.decorators.as_deref() else {
        return;
    };
    setup_section(ui, "Decorator Set", false, |ui, part| {
        if part == BoxPart::Body {
            draw_set_rows(ui, set);
        }
    });
    let hidden = set.types.iter().enumerate().any(|(index, _)| {
        state
            .region_selections
            .get(&type_region(set, index))
            .is_some_and(|selection| !selection.enabled)
    });
    if setup_section(ui, "Types", hidden, |ui, part| {
        if part == BoxPart::Body {
            draw_type_rows(ui, set, state);
        }
    }) {
        for index in 0..set.types.len() {
            if let Some(selection) = state.region_selections.get_mut(&type_region(set, index)) {
                selection.enabled = true;
            }
        }
    }
    let layout = &mut state.decorator_layout;
    let default = Layout::default();
    let patch_changed =
        (layout.size, layout.count, layout.seed) != (default.size, default.count, default.seed);
    if setup_section(ui, "Layout", patch_changed, |ui, part| {
        if part == BoxPart::Body {
            draw_layout_rows(ui, set, layout);
        }
    }) {
        (layout.size, layout.count, layout.seed) = (default.size, default.count, default.seed);
    }
    if setup_section(ui, "Distance", layout.fade != default.fade, |ui, part| {
        if part == BoxPart::Body {
            draw_distance_rows(ui, set, layout);
        }
    }) {
        layout.fade = default.fade;
    }
    if setup_section(ui, "Lighting", layout.light != default.light, |ui, part| {
        if part == BoxPart::Body {
            draw_lighting_rows(ui, layout);
        }
    }) {
        layout.light = default.light;
    }
    if set.shader.is_some_and(|shader| shader.moves()) {
        if setup_section(ui, "Wind", layout.sway != default.sway, |ui, part| {
            if part == BoxPart::Body {
                draw_wind_rows(ui, set, layout);
            }
        }) {
            layout.sway = default.sway;
        }
    }
}

/// Whether the game's fade with distance draws, and over what distances.
fn draw_distance_rows(ui: &mut Ui, set: &DecoratorSet, layout: &mut Layout) {
    let mut rows = CardRows::default();
    rows.row(ui, |ui| {
        ui.baboon_checkbox(&mut layout.fade, "Fade with distance");
    });
    let note = if set.halo2.is_some() {
        "Each permutation shrinks away by its fade distance: close by 8 world units, medium by 16, far by 24.".to_owned()
    } else if let Some(collection) = &set.halo_ce {
        let ranges: Vec<String> = collection
            .types
            .iter()
            .map(|kind| format!("{} {}–{}", kind.name, kind.near_fade, kind.far_fade))
            .collect();
        format!(
            "Each type fades out between its near and far fade distance: {}.",
            ranges.join(", ")
        )
    } else if let Some([start, end]) = set.fade_range {
        format!("The set dissolves between its start and end fade, {start} and {end} world units.")
    } else {
        "This game cross-fades between its levels-of-detail models, which the preview doesn't draw yet.".to_owned()
    };
    ui.label(RichText::new(note).color(subtle_dark()).small());
}

/// How bright the light a level would bake into each decorator is.
fn draw_lighting_rows(ui: &mut Ui, layout: &mut Layout) {
    let width = name_column_width(ui, ["Light"], ui.available_width().max(1.0));
    let mut rows = CardRows::default();
    rows.row(ui, |ui| {
        name_cell(ui, "Light", width);
        ui.add(egui::Slider::new(&mut layout.light, 0.0..=2.0).fixed_decimals(2))
            .on_hover_text("The light a level's lightmap would bake into each decorator, as a share of the preview's stand-in for it");
    });
}

/// Whether the wind (or a wavy set's waves) moves the set, and which wind.
fn draw_wind_rows(ui: &mut Ui, set: &DecoratorSet, layout: &mut Layout) {
    let mut rows = CardRows::default();
    rows.row(ui, |ui| {
        ui.baboon_checkbox(&mut layout.sway, "Sway");
    });
    let note = if set.shader.is_some_and(|shader| shader.moves_in_the_wind()) {
        let wind = set
            .wind
            .as_ref()
            .map_or("none, as the globals name no default wind", |wind| {
                wind.path.as_str()
            });
        format!("The globals' default wind: {wind}")
    } else {
        "Its waves run on the preview's clock, by the set's own wavelength, speed and frequency."
            .to_owned()
    };
    ui.label(RichText::new(note).color(subtle_dark()).small());
}

fn type_region(set: &DecoratorSet, index: usize) -> String {
    super::super::decorators::type_name(index, &set.types[index].mesh_name)
}

/// What the set draws with, a row each: its shader, its sides and its
/// texture.
fn draw_set_rows(ui: &mut Ui, set: &DecoratorSet) {
    // Halo 2's permutations each name a shader of the set's, and nothing sways.
    if let Some(halo2) = &set.halo2 {
        let classes = halo2.classes.len().to_string();
        let mut rows = vec![("Classes", classes.as_str())];
        rows.extend(
            halo2
                .shaders
                .iter()
                .map(|shader| ("Shader", shader.as_str())),
        );
        draw_rows(ui, &rows);
        return;
    }
    if let Some(collection) = &set.halo_ce {
        let offset = format!("{} wu", collection.z_offset);
        let facing = match collection.facing {
            DetailFacing::ScreenFacing => "screen-facing",
            DetailFacing::ViewerFacing => "viewer-facing",
        };
        draw_rows(
            ui,
            &[
                ("Facing", facing),
                ("Sprite plate", collection.plate_path.as_str()),
                ("Z offset", offset.as_str()),
            ],
        );
        return;
    }
    let shader = set.shader.map_or("unknown", |shader| shader.name());
    let rows: Vec<(&str, &str)> = vec![
        ("Render shader", shader),
        ("Sides", if set.two_sided { "both" } else { "front" }),
        ("Texture", set.texture.as_str()),
    ];
    draw_rows(ui, &rows);
}

/// A row per `(name, value)`, the value cut short with its whole self on
/// hover.
fn draw_rows(ui: &mut Ui, rows: &[(&str, &str)]) {
    let width = name_column_width(
        ui,
        rows.iter().map(|(name, _)| *name),
        ui.available_width().max(1.0),
    );
    let mut card = CardRows::default();
    for &(name, value) in rows {
        card.row(ui, |ui| {
            name_cell(ui, name, width);
            ui.add(
                egui::Label::new(RichText::new(value).color(text_dark()).font(row_font()))
                    .truncate(),
            )
            .on_hover_text(value);
        });
    }
}

/// A row per type: whether it shows, its name, and how many the patch
/// stands.
fn draw_type_rows(ui: &mut Ui, set: &DecoratorSet, state: &mut ModelPreviewState) {
    let names: Vec<String> = (0..set.types.len())
        .map(|index| type_region(set, index))
        .collect();
    let width = name_column_width(
        ui,
        names.iter().map(String::as_str),
        ui.available_width().max(1.0),
    );
    let mut rows = CardRows::default();
    for (index, name) in names.iter().enumerate() {
        let placed = set.placed(index);
        let drawn = set.draws_type(index);
        let selection = state
            .region_selections
            .entry(name.clone())
            .or_insert_with(|| ModelRegionSelection {
                enabled: true,
                permutation: super::super::decorators::PERMUTATION.to_owned(),
            });
        rows.row(ui, |ui| {
            ui.add_enabled_ui(drawn, |ui| ui.baboon_checkbox(&mut selection.enabled, ""));
            name_cell(ui, name, width);
            let count = if drawn {
                format!("{placed} placed")
            } else {
                "no mesh".to_owned()
            };
            ui.label(RichText::new(count).color(subtle_dark()).font(row_font()));
        });
    }
}

/// The patch's edge, how many it tries to stand, and its seed.
fn draw_layout_rows(ui: &mut Ui, set: &DecoratorSet, layout: &mut Layout) {
    let names = ["Patch size", "Count", "Seed"];
    let width = name_column_width(ui, names, ui.available_width().max(1.0));
    let mut rows = CardRows::default();
    rows.row(ui, |ui| {
        name_cell(ui, names[0], width);
        ui.add(
            egui::DragValue::new(&mut layout.size)
                .range(0.25..=f32::MAX)
                .speed(0.1)
                .suffix(" wu"),
        )
        .on_hover_text("The patch's edge, in world units");
    });
    rows.row(ui, |ui| {
        name_cell(ui, names[1], width);
        ui.add(egui::DragValue::new(&mut layout.count).speed(5.0))
            .on_hover_text("How many decorators the patch tries to stand; fewer fit where types keep their distance");
        if set.placements.len() < layout.count && set.layout == *layout {
            ui.label(RichText::new(format!("{} fit", set.placements.len())).color(subtle_dark()).font(row_font()));
        }
    });
    rows.row(ui, |ui| {
        name_cell(ui, names[2], width);
        ui.add(egui::DragValue::new(&mut layout.seed))
            .on_hover_text("Another seed lays another patch");
    });
    if layout.count > LARGE_PATCH {
        ui.label(
            RichText::new("A patch this large is slow to lay out and draw.")
                .color(subtle_dark())
                .small(),
        );
    }
}
