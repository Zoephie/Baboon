//! The weapon pickers: what a unit holds, in either hand.

use super::super::super::ui_kit::style::{subtle_dark, text_dark};
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

/// A dropdown of weapons, returning what was picked, `None` being empty hands.
pub(in crate::app::model_preview) fn weapon_dropdown(
    ui: &mut Ui,
    label: &str,
    salt: (&str, &str),
    weapons: &[super::super::weapons::WeaponOption],
    held: Option<&super::super::weapons::WeaponOption>,
) -> Option<Option<String>> {
    let mut chosen = None;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(label);
        let popup_id = ui.make_persistent_id(salt);
        let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        let response = picker_button(
            ui,
            popup_id,
            held.map_or("None", |weapon| weapon.name.as_str()),
            weapons.len(),
            220.0,
            text_dark(),
            true,
        );
        let just_opened = response.clicked() && !open;
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
                        let is_held = held.is_some_and(|held| held.path == weapon.path);
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
    chosen
}

/// Note what should outlive the reload a weapon change makes.
pub(in crate::app::model_preview) fn carry_through_weapon_change(
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    state.remember_selection();
    state.weapon_carry = Some((data.geometry_id, state.region_selections.clone()));
}

/// A Halo 2 unit's two hands. The left only opens for what can pair, and
/// a primary that cannot pair empties it.
pub(in crate::app::model_preview) fn draw_unit_weapon_pickers(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let Some(weapons) = data
        .weapons
        .as_ref()
        .filter(|weapons| !weapons.equippable.is_empty())
    else {
        return;
    };
    let primaries = weapons.primaries();
    let held = primaries
        .iter()
        .find(|weapon| Some(&weapon.path) == state.weapon.as_ref());
    let salt = ("primary_weapon_picker", data.render_model_path.as_str());
    if let Some(choice) = weapon_dropdown(ui, "Primary", salt, &primaries, held)
        .filter(|choice| *choice != state.weapon)
    {
        carry_through_weapon_change(data, state);
        let pairs = weapons.secondaries(choice.as_deref());
        if !pairs
            .iter()
            .any(|weapon| Some(&weapon.path) == state.secondary_weapon.as_ref())
        {
            state.secondary_weapon = None;
        }
        state.weapon = choice;
    }
    let secondaries = weapons.secondaries(state.weapon.as_deref());
    if secondaries.is_empty() {
        return;
    }
    let held = secondaries
        .iter()
        .find(|weapon| Some(&weapon.path) == state.secondary_weapon.as_ref());
    let salt = ("secondary_weapon_picker", data.render_model_path.as_str());
    if let Some(choice) = weapon_dropdown(ui, "Secondary", salt, &secondaries, held)
        .filter(|choice| *choice != state.secondary_weapon)
    {
        carry_through_weapon_change(data, state);
        state.secondary_weapon = choice;
    }
}
