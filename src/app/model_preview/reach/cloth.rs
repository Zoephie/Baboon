//! Reach's cloth rules, which Halo 4 runs too.

use super::super::cloth::{ClothRules, ClothSheetBasis, ClothWind, MOST_VERTICES, REACH_TICK_RATE};

/// `cloth_setup_attachments` (the 360's 0x82C8E450) fetches up to
/// `NUMBEROF(marker)`, ten, of the attachment's markers.
const MOST_ATTACHMENTS: usize = 10;

/// The 360's 0x82C8EE80, MCC's 0x140D96750: at most 9 iterations, and the
/// tenth pass ends the loop (`if (pass++ == 9) i = N`).
const MOST_ITERATIONS: i32 = 9;

const MOST_PASSES: usize = 10;

pub(in crate::app::model_preview) const CLOTH: ClothRules = ClothRules {
    tick_rate: REACH_TICK_RATE,
    most_vertices: MOST_VERTICES,
    spans_texture_per_side: true,
    most_attachments: MOST_ATTACHMENTS,
    pins_secondary: false,
    clamps_properties: true,
    sheet_basis: ClothSheetBasis::RestNormal,
    region_keeps_extension: true,
    rests_at_first_pin: false,
    follows_center: true,
    most_iterations: MOST_ITERATIONS,
    most_passes: MOST_PASSES,
    settles_new_sheet: true,
    wind: ClothWind::PerCloth,
    relaxes_as_halo2: false,
    normals_carry_over: false,
    back_normal_sign: 1.0,
    derives_binormal: true,
};
