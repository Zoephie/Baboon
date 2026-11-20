//! The weapon pickers: what a unit holds, in either hand.

use super::super::super::ui_kit::style::{subtle_dark, text_dark};
use super::rows::{CardRows, name_cell, name_column_width};
use super::variants::{draw_region_rows, weapon_region};
use super::super::{
    ModelPreviewData, ModelPreviewState, picker_button, picker_popup_width, picker_results,
    picker_search_field,
};
use eframe::egui;
use eframe::egui::{RichText, Ui};

pub(in crate::app::model_preview) const WEAPON_PICKER_HEIGHT: f32 = 300.0;

/// Whether the weapon list overflows its popup: `None`, a heading per class,
/// and a row per weapon.
pub(in crate::app::model_preview) fn weapon_picker_scrolls(
    weapons: &[super::super::weapons::WeaponOption],
    row_height: f32,
) -> bool {
    let classes = weapons.chunk_by(|a, b| a.class == b.class).count();
    (1 + classes + weapons.len()) as f32 * row_height > WEAPON_PICKER_HEIGHT
}

/// A weapon's row survives a search by its own name or its class's.
pub(in crate::app::model_preview) fn weapon_matches(
    weapon: &super::super::weapons::WeaponOption,
    query: &str,
) -> bool {
    weapon.name.to_lowercase().contains(query) || weapon.class.to_lowercase().contains(query)
}

/// What a weapon dropdown did this frame.
pub(in crate::app::model_preview) struct WeaponPick {
    /// What was picked, `None` being empty hands.
    pub(in crate::app::model_preview) chosen: Option<Option<String>>,
    /// Whether it opened, which is when its weapons are first listed.
    pub(in crate::app::model_preview) opened: bool,
}

/// A row of `label` and a dropdown of `weapons`, or of none yet, while
/// `None`: they are listed once it first opens. `held` is the held weapon's
/// path, shown by its tag's name until the list has its own.
#[allow(clippy::too_many_arguments)]
pub(in crate::app::model_preview) fn weapon_dropdown(
    ui: &mut Ui,
    rows: &mut CardRows,
    label: &str,
    label_width: f32,
    salt: (&str, &str),
    weapons: Option<&[super::super::weapons::WeaponOption]>,
    held: Option<&str>,
) -> WeaponPick {
    let mut chosen = None;
    let mut opened = false;
    let held_name = held.map_or("None", |path| {
        weapons
            .into_iter()
            .flatten()
            .find(|weapon| weapon.path == path)
            .map_or(crate::core::tag_path::name(path), |weapon| weapon.name.as_str())
    });
    rows.row(ui, |ui| {
        name_cell(ui, label, label_width);
        let popup_id = ui.make_persistent_id(salt);
        let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        let width = ui.available_width().max(60.0);
        let response = picker_button(
            ui,
            popup_id,
            held_name,
            weapons.map(<[_]>::len),
            width,
            text_dark(),
            true,
        );
        let just_opened = response.clicked() && !open;
        opened = just_opened;
        if response.clicked() {
            egui::Popup::toggle_id(ui.ctx(), popup_id);
        }
        // Open while its id is open in memory, as the button above toggles it;
        // left alone, a popup built from a response is always open.
        egui::Popup::from_response(&response)
            .id(popup_id)
            .open_memory(None)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                picker_popup_width(ui, &response);
                let Some(weapons) = weapons else {
                    ui.label(RichText::new("Listing weapons…").color(subtle_dark()));
                    return;
                };
                let row_height = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
                let mut query = String::new();
                let mut changed = false;
                if weapon_picker_scrolls(weapons, row_height) {
                    let filter_id = popup_id.with("entry_search");
                    let mut filter = ui
                        .data(|data| data.get_temp::<String>(filter_id))
                        .unwrap_or_default();
                    changed = picker_search_field(ui, &mut filter, "search weapons…", just_opened);
                    ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
                    ui.separator();
                    query = filter.trim().to_lowercase();
                }
                picker_results(ui, WEAPON_PICKER_HEIGHT, changed, |ui| {
                    let mut shown = 0;
                    if query.is_empty() || "none".contains(&query) {
                        shown += 1;
                        if ui.selectable_label(held.is_none(), "None").clicked() {
                            chosen = Some(None);
                            egui::Popup::close_id(ui.ctx(), popup_id);
                        }
                    }
                    let mut class = None;
                    for weapon in weapons {
                        if !weapon_matches(weapon, &query) {
                            continue;
                        }
                        shown += 1;
                        if class != Some(&weapon.class) {
                            class = Some(&weapon.class);
                            ui.label(RichText::new(&weapon.class).color(subtle_dark()));
                        }
                        let is_held = held == Some(weapon.path.as_str());
                        if ui.selectable_label(is_held, &weapon.name).clicked() {
                            chosen = Some(Some(weapon.path.clone()));
                            egui::Popup::close_id(ui.ctx(), popup_id);
                        }
                    }
                    if shown == 0 {
                        ui.label(RichText::new("No weapons match.").color(subtle_dark()));
                    }
                });
            });
    });
    WeaponPick { chosen, opened }
}

/// Note what should outlive the reload a weapon change or the Game Shaders
/// toggle makes.
pub(in crate::app::model_preview) fn carry_through_reload(
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    state.remember_selection();
    state.reload_carry = Some((data.geometry_id, state.region_selections.clone()));
}

/// A Halo 2 unit's two hands. The left only opens for what can pair, and
/// a primary that cannot pair empties it.
pub(in crate::app::model_preview) fn draw_unit_weapon_pickers(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let Some(weapons) = data.weapons.as_ref().filter(|weapons| weapons.can_hold()) else {
        return;
    };
    let primaries = weapons.primaries();
    let mut rows = CardRows::default();
    let labels = ["Primary", "Secondary"];
    let label_width = name_column_width(ui, labels, ui.available_width());
    let salt = ("primary_weapon_picker", data.render_model_path.as_str());
    let held = state.weapon.clone();
    let pick = weapon_dropdown(
        ui,
        &mut rows,
        labels[0],
        label_width,
        salt,
        primaries.as_deref(),
        held.as_deref(),
    );
    if pick.opened && primaries.is_none() {
        state.list_weapons = true;
    }
    if let Some(choice) = pick.chosen.filter(|choice| *choice != state.weapon) {
        carry_through_reload(data, state);
        let pairs = weapons.secondaries(choice.as_deref());
        if !pairs
            .iter()
            .any(|weapon| Some(&weapon.path) == state.secondary_weapon.as_ref())
        {
            state.secondary_weapon = None;
        }
        state.weapon = choice;
    }
    let primary = state.loaded_weapon.clone();
    draw_held_regions(ui, data, state, &mut rows, primary.as_deref());
    let secondaries = weapons.secondaries(state.weapon.as_deref());
    if secondaries.is_empty() {
        return;
    }
    let salt = ("secondary_weapon_picker", data.render_model_path.as_str());
    let held = state.secondary_weapon.clone();
    if let Some(choice) = weapon_dropdown(
        ui,
        &mut rows,
        labels[1],
        label_width,
        salt,
        Some(&secondaries),
        held.as_deref(),
    )
    .chosen
    .filter(|choice| *choice != state.secondary_weapon)
    {
        carry_through_reload(data, state);
        state.secondary_weapon = choice;
    }
    let secondary = state.loaded_secondary_weapon.clone();
    draw_held_regions(ui, data, state, &mut rows, secondary.as_deref());
}

/// The regions of the weapon at `path` the preview holds, as rows under its
/// dropdown, by their own names.
pub(in crate::app::model_preview) fn draw_held_regions(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    rows: &mut CardRows,
    path: Option<&str>,
) {
    let Some(path) = path else {
        return;
    };
    let weapon = crate::core::tag_path::name(path);
    let regions: Vec<(usize, &str)> = data
        .preview
        .regions
        .iter()
        .enumerate()
        .filter_map(|(index, region)| Some((index, weapon_region(&region.name, weapon)?)))
        .collect();
    draw_region_rows(ui, data, state, rows, &regions);
}
