//! The exports each object type publishes to its functions.

use super::*;

/// One of Halo 2's function exporters: the names it claims, with what an idle
/// object, just placed, gives each. MCC `halo2_tag_test`; the object one is
/// `sub_86A360`, and every type asks it first.
type ExportTable = &'static [(&'static str, f32)];

/// Every object's exporter. `body_vitality`, `shield_vitality`,
/// `shield_depleted` and `variant` depend on the model; see
/// [`TypeExports::idle`]. `compass`, `random_constant`, the clock and the
/// team, medic and holiday names depend on a game the preview has not got,
/// and stay 0.
pub(super) const OBJECT_EXPORTS: ExportTable = &[
    ("one", 1.0),
    ("zero", 0.0),
    ("morse_code", 1.0),
    ("alive", 1.0),
    ("body_vitality", 1.0),
    ("shield_vitality", 1.0),
    ("current_body_damage", 0.0),
    ("current_shield_damage", 0.0),
    ("object_overshield_amount", 0.0),
    ("shield_depleted", 0.0),
    ("electrical_power", 1.0),
    ("variant", 0.0),
    ("compass", 0.0),
    ("random_constant", 0.0),
    ("time_hours", 0.0),
    ("time_minutes", 0.0),
    ("time_seconds", 0.0),
    ("holiday", 0.0),
    ("all_quiet", 1.0),
    ("flashlights_allowed", 0.0),
    ("medic", 0.0),
    ("team0", 0.0),
    ("team1", 0.0),
    ("team2", 0.0),
    ("team3", 0.0),
    ("team4", 0.0),
    ("team5", 0.0),
    ("team6", 0.0),
    ("team7", 0.0),
    ("scripted_object_function_a", 0.0),
    ("scripted_object_function_b", 0.0),
    ("scripted_object_function_c", 0.0),
    ("scripted_object_function_d", 0.0),
    ("emblem_active", 1.0),
    ("emblem_foreground_index", 0.0),
    ("emblem_background_index", 0.0),
    ("emblem_toggle_foreground", 1.0),
    ("emblem_foreground_map_scale_x", 0.0),
    ("emblem_background_map_scale_x", 0.0),
];

/// A unit's, biped's and vehicle's, `sub_882EB0`. `barrel_spin` and
/// `illumination` it passes to the weapon held, and claims only with one in
/// hand; the held weapon's own functions answer for it here.
const UNIT_EXPORTS: ExportTable = &[
    ("driver_seat_occupied", 0.0),
    ("gunner_seat_occupied", 0.0),
    ("driver_seat_power", 0.0),
    ("gunner_seat_power", 0.0),
    ("active_camouflage", 0.0),
    ("can_blink", 1.0),
    ("crouch", 0.0),
    ("mouth_aperture", 0.0),
    ("vertical_thrust", 0.0),
    ("integrated_light_power", 0.0),
    ("aiming_change", 0.0),
    ("horizontal_aiming_change", 0.0),
    ("boost", 0.0),
    ("unit_open", 1.0),
    ("unit_closed", 0.0),
];

/// A biped's, `sub_8AE0E0`. `stunned` it claims only while stunned, and
/// `flying_speed` only for a biped that flies.
const BIPED_EXPORTS: ExportTable = &[("flight", 0.0)];

/// A vehicle's, `sub_8D0B60`: all at rest.
const VEHICLE_EXPORTS: ExportTable = &[
    ("speed_absolute", 0.0),
    ("speed_forward", 0.0),
    ("speed_backward", 0.0),
    ("speed_slide_maximum", 0.0),
    ("slide_left", 0.0),
    ("slide_right", 0.0),
    ("slide_absolute", 0.0),
    ("slide_back", 0.0),
    ("turn_left", 0.0),
    ("turn_right", 0.0),
    ("turn_absolute", 0.0),
    ("velocity_forward", 0.0),
    ("velocity_left", 0.0),
    ("velocity_up", 0.0),
    ("velocity_air", 0.0),
    ("velocity_ground", 0.0),
    ("velocity_water", 0.0),
    ("wingtip_contrail", 0.0),
    ("wingtip_contrail_new", 0.0),
    ("front_left_tire_position", 0.0),
    ("front_left_tire_velocity", 0.0),
    ("front_right_tire_position", 0.0),
    ("front_right_tire_velocity", 0.0),
    ("back_left_tire_position", 0.0),
    ("back_left_tire_velocity", 0.0),
    ("back_right_tire_position", 0.0),
    ("back_right_tire_velocity", 0.0),
    ("left_tread_position", 0.0),
    ("left_tread_velocity", 0.0),
    ("right_tread_position", 0.0),
    ("right_tread_velocity", 0.0),
    ("tread_grind", 0.0),
    ("engine_rpm", 0.0),
    ("engine_power", 0.0),
    ("engine_hack", 0.0),
    ("jump", 0.0),
    ("emitting", 0.0),
    ("hover", 0.0),
    ("thrust", 0.0),
    ("mean_antigrav", 0.0),
];

/// A weapon's, `sub_8A0000`: new, full and not yet in a hand, so turned on.
/// The preview counts no rounds, so the ones and tens of the count stay 0.
const WEAPON_EXPORTS: ExportTable = &[
    ("age", 0.0),
    ("heat", 0.0),
    ("overheated", 0.0),
    ("barrel_spin", 0.0),
    ("power", 0.0),
    ("ready", 0.0),
    ("turned_on", 1.0),
    ("primary_ammunition", 1.0),
    ("secondary_ammunition", 1.0),
    ("primary_ammunition_ones", 0.0),
    ("primary_ammunition_tens", 0.0),
    ("primary_trigger_down", 0.0),
    ("secondary_trigger_down", 0.0),
    ("primary_firing", 0.0),
    ("secondary_firing", 0.0),
    ("primary_charged", 0.0),
    ("secondary_charged", 0.0),
    ("primary_rate_of_fire", 0.0),
    ("secondary_rate_of_fire", 0.0),
    ("primary_ejection_port", 0.0),
    ("secondary_ejection_port", 0.0),
    ("illumination", 0.0),
    ("warthog_chaingun_illumination", 0.0),
    ("active_camouflage", 0.0),
    ("bomb_arming_amount", 0.0),
    ("bomb_is_armed", 0.0),
    ("bomb_is_arming", 0.0),
    ("bomb_is_unarmed", 0.0),
];

/// A projectile's, `sub_8F72F0`, as it is made.
const PROJECTILE_EXPORTS: ExportTable = &[
    ("tracer", 1.0),
    ("projectile_attach", 0.0),
    ("acceleration_range", 1.0),
    ("range_remaining", 0.0),
    ("time_remaining", 0.0),
];

/// A device's (machine, control, light fixture), `sub_8A83F0`, as one placed
/// with neither `initially off` nor `initially open` would be.
pub(super) const DEVICE_EXPORTS: ExportTable = &[
    ("power", 1.0),
    ("position", 0.0),
    ("change_in_power", 0.0),
    ("change_in_position", 0.0),
    ("loop", 1.0),
    ("delay", 0.0),
    ("locked", 0.0),
];

/// What a Halo 2 object's type exports to its functions: its exporters, in
/// the order `sub_8D8EC0` asks them, and what of its model decides the
/// vitality and variant names.
#[derive(Debug, Clone, Copy)]
pub(super) struct TypeExports {
    pub(super) tables: &'static [ExportTable],
    pub(super) has_body: bool,
    pub(super) has_shield: bool,
    pub(super) variants: usize,
}

impl TypeExports {
    /// For a Halo 2 `object` previewing `model`; none for another game.
    pub(super) fn of(object: &TagFile, model: Option<&TagFile>) -> Option<Self> {
        if blam_tags::game::Game::of(object) != blam_tags::game::Game::Halo2 {
            return None;
        }
        let tables: &'static [ExportTable] = match &object.header.group_tag.to_be_bytes() {
            b"bipd" => &[OBJECT_EXPORTS, UNIT_EXPORTS, BIPED_EXPORTS],
            b"vehi" => &[OBJECT_EXPORTS, UNIT_EXPORTS, VEHICLE_EXPORTS],
            b"weap" => &[OBJECT_EXPORTS, WEAPON_EXPORTS],
            b"proj" => &[OBJECT_EXPORTS, PROJECTILE_EXPORTS],
            b"mach" | b"ctrl" | b"lifi" => &[OBJECT_EXPORTS, DEVICE_EXPORTS],
            _ => &[OBJECT_EXPORTS],
        };
        let damage =
            model.and_then(|model| elements(model.root(), "new damage info").first().copied());
        let vitality = |field: &str| {
            damage
                .and_then(|damage| damage.read_real(field))
                .unwrap_or(0.0)
        };
        Some(Self {
            tables,
            has_body: vitality("maximum vitality") > 0.0,
            has_shield: vitality("maximum shield vitality") > 0.0,
            variants: model.map_or(0, |model| elements(model.root(), "variants").len()),
        })
    }

    /// What an idle object of this type gives `name`, if an exporter claims it.
    /// The vitalities are full where the model has them; the variant is its
    /// first of `variants`, `(index + 1) / count`.
    pub(super) fn idle(&self, name: &str) -> Option<f32> {
        let model_says = match name {
            "body_vitality" => Some(f32::from(u8::from(self.has_body))),
            "shield_vitality" => Some(f32::from(u8::from(self.has_shield))),
            "shield_depleted" => Some(f32::from(u8::from(!self.has_shield))),
            "variant" => Some(if self.variants > 0 {
                1.0 / self.variants as f32
            } else {
                0.0
            }),
            _ => None,
        };
        let claimed = self
            .tables
            .iter()
            .flat_map(|table| table.iter())
            .find(|(export, _)| *export == name);
        claimed.map(|(_, idle)| model_says.unwrap_or(*idle))
    }
}
