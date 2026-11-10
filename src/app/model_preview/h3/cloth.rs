//! Halo 3's and ODST's cloth rules.

use super::super::cloth::{
    ClothRules, ClothSheetBasis, ClothWind, MOST_ATTACHMENTS, MOST_ITERATIONS, MOST_PASSES,
    MOST_VERTICES,
};

pub(in crate::app::model_preview) const CLOTH: ClothRules = ClothRules {
    tick_rate: 60.0,
    most_vertices: MOST_VERTICES,
    spans_texture_per_side: false,
    most_attachments: MOST_ATTACHMENTS,
    pins_secondary: true,
    clamps_properties: true,
    sheet_basis: ClothSheetBasis::BinormalTurns,
    region_keeps_extension: false,
    rests_at_first_pin: true,
    follows_center: false,
    most_iterations: MOST_ITERATIONS,
    most_passes: MOST_PASSES,
    settles_new_sheet: false,
    wind: ClothWind::OneShared,
    relaxes_by_pull: false,
    normals_carry_over: false,
    back_normal_sign: 1.0,
    derives_binormal: false,
};
