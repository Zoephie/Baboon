//! Halo 2's cloth rules.

use super::super::cloth::{
    ClothRules, ClothSheetBasis, ClothWind, MOST_ATTACHMENTS, MOST_ITERATIONS, MOST_PASSES,
};

/// The most vertices a Halo 2 sheet keeps.
const MOST_VERTICES: usize = 128;

pub(in crate::app::model_preview) const CLOTH: ClothRules = ClothRules {
    tick_rate: 30.0,
    most_vertices: MOST_VERTICES,
    spans_texture_per_side: false,
    most_attachments: MOST_ATTACHMENTS,
    pins_secondary: true,
    clamps_properties: false,
    sheet_basis: ClothSheetBasis::TurnsOver,
    region_keeps_extension: false,
    rests_at_first_pin: true,
    follows_center: false,
    most_iterations: MOST_ITERATIONS,
    most_passes: MOST_PASSES,
    settles_new_sheet: false,
    wind: ClothWind::HeldFromUp,
    relaxes_by_pull: true,
    normals_carry_over: true,
    back_normal_sign: -1.0,
    derives_binormal: false,
};
