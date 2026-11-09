//! Each game's cloth rules: limits, tick rates, gravity and the sheet basis.

/// `k_maximum_vertices_per_cloth`.
const MOST_VERTICES: usize = 121;

/// The same, in Halo 2.
const H2_MOST_VERTICES: usize = 128;

/// `k_maximum_attachments_per_cloth`.
const MOST_ATTACHMENTS: usize = 6;

/// Reach's: `cloth_setup_attachments` (the 360's 0x82C8E450) fetches up to
/// `NUMBEROF(marker)`, ten, of the attachment's markers.
const REACH_MOST_ATTACHMENTS: usize = 10;

/// `k_maximum_collision_spheres_per_cloth`.
pub(super) const MOST_SPHERES: usize = 8;

/// `cloth_constrain_positions_relaxation`'s, past the tag's own limit because
/// a moving object adds to them; a preview's stands still.
pub(super) const MOST_ITERATIONS: i32 = 15;

/// Its passes, a sheet that keeps coming apart included.
const MOST_PASSES: usize = 16;

/// Reach's (the 360's 0x82C8EE80, MCC's 0x140D96750): at most 9 iterations,
/// and the tenth pass ends the loop (`if (pass++ == 9) i = N`).
const REACH_MOST_ITERATIONS: i32 = 9;

const REACH_MOST_PASSES: usize = 10;

/// How far Reach's `cloth_enforce_attachments` (the 360's 0x82C8F9B0) lets
/// the object's bounding sphere move before it carries the whole sheet along.
pub(super) const REACH_CENTER_JUMP: f32 = 0.4;

/// The relaxations Reach's `cloth_new_attachment` (the 360's 0x82C8E2F0)
/// settles a new sheet with.
pub(super) const REACH_CREATION_RELAXATIONS: usize = 5;

pub(super) const ERROR_MAX: f32 = 2.0;

/// How many links it keeps of those that touch a pin.
pub(super) const MOST_HELD_LINKS: usize = 1210;

/// The shortest a link is measured at, and how near a sphere's centre a point
/// has no way out but `global_left3d`.
pub(super) const NEARLY_ZERO: f32 = 0.0001;

/// Which game's cloth a sheet is: they share the sheet and its links, and
/// differ in their limits, wind, ticks and how they start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClothGame {
    Halo2,
    /// Halo 3 and ODST.
    Halo3,
    Reach,
}

/// Reach's game ticks a second, for its cloth and its antennas. \[inferred\]
/// The rate comes from `game_options +12` (the 360's
/// `game_time_initialize_for_new_map`, 0x82D7A998), which was not read for
/// MCC. 30 is the 360's, and the antennas' fixed step, 0.0333 a tick, is a
/// 30th of a second.
pub(crate) const REACH_TICK_RATE: f32 = 30.0;

/// Where the games' cloth differs, decision by decision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClothRules {
    /// The game's ticks a second: `cloth_update` runs each cloth at most once
    /// a tick (`last_update_tick`), whatever the frame's time. Halo 2 ticks at
    /// 30; Halo 3 and ODST at their native rate, the 60 of `get_vblank_rate`;
    /// Reach at [`REACH_TICK_RATE`].
    pub(crate) tick_rate: f32,
    /// The most vertices a sheet keeps (`cloth_generate_flag_grid`).
    pub(super) most_vertices: usize,
    /// Reach's `create_cloth_grid_data` spans the texture across each side by
    /// its own step, where the others span a square across the longer side.
    pub(super) spans_texture_per_side: bool,
    /// The most attachment markers a sheet pins to: Reach takes ten to the
    /// others' six.
    pub(super) most_attachments: usize,
    /// Whether the sheet's last vertex pins to the secondary marker. Reach's
    /// would pin it to a slot that is never filled.
    pub(super) pins_secondary: bool,
    /// Whether the tag's properties are held to `tool.exe`'s ranges. Halo 2's
    /// tool limits none of them.
    pub(super) clamps_properties: bool,
    /// How `cloth_render_opaque` frames each side.
    pub(super) sheet_basis: ClothSheetBasis,
    /// Whether the preview's region for a sheet keeps the tag's `.cloth`.
    pub(super) region_keeps_extension: bool,
    /// `cloth_reset_flag_to_default_positions`: Halo 2 and 3 move the sheet up
    /// to its first pin; Reach's leaves it at the world's origin.
    pub(super) rests_at_first_pin: bool,
    /// Reach's `cloth_enforce_attachments` first carries the whole sheet with
    /// the object's bounding sphere, where that has jumped.
    pub(super) follows_center: bool,
    /// `cloth_constrain_positions_relaxation`'s limits: the iterations a pass
    /// runs, and the passes before it gives up.
    pub(super) most_iterations: i32,
    pub(super) most_passes: usize,
    /// Reach's `cloth_new_attachment` pins a new sheet and relaxes it five
    /// times.
    pub(super) settles_new_sheet: bool,
    /// Whose wind blows the sheets.
    pub(super) wind: ClothWind,
    /// Whether the sheet relaxes as Halo 2's does.
    pub(super) relaxes_as_halo2: bool,
    /// Halo 2 adds to what the normal was, so each carries a little of the
    /// last tick, and an unset one points up; the others start from nothing
    /// and point along x.
    pub(super) normals_carry_over: bool,
    /// The sign of the back side's normal: Halo 2 turns it over, Halo 3 and
    /// Reach keep the one normal.
    pub(super) back_normal_sign: f32,
    /// Whether each vertex's binormal follows its normal, as Reach's does.
    pub(super) derives_binormal: bool,
}

/// How a cloth's two sides are framed in the model's vertices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClothSheetBasis {
    /// Halo 2's `cloth_render` turns the whole frame over with the side.
    TurnsOver,
    /// Halo 3's binormal turns over, as loaded.
    BinormalTurns,
    /// Both sides one frame, from the rest normal, `global_left3d`.
    RestNormal,
}

/// Whose wind blows a slot's cloth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClothWind {
    Halo2,
    /// Halo 3's wind clock counts the ticks the wind was authored at, 30 a second.
    Halo3,
    /// Reach's moves as each cloth's verlet asks, on its own clock.
    PerCloth,
}

impl ClothGame {
    /// The game's cloth rules.
    pub(crate) const fn rules(self) -> ClothRules {
        let reach = matches!(self, ClothGame::Reach);
        let halo2 = matches!(self, ClothGame::Halo2);
        ClothRules {
            tick_rate: match self {
                ClothGame::Halo2 => 30.0,
                ClothGame::Halo3 => 60.0,
                ClothGame::Reach => REACH_TICK_RATE,
            },
            most_vertices: if halo2 {
                H2_MOST_VERTICES
            } else {
                MOST_VERTICES
            },
            spans_texture_per_side: reach,
            most_attachments: if reach {
                REACH_MOST_ATTACHMENTS
            } else {
                MOST_ATTACHMENTS
            },
            pins_secondary: !reach,
            clamps_properties: !halo2,
            sheet_basis: match self {
                ClothGame::Halo2 => ClothSheetBasis::TurnsOver,
                ClothGame::Halo3 => ClothSheetBasis::BinormalTurns,
                ClothGame::Reach => ClothSheetBasis::RestNormal,
            },
            region_keeps_extension: reach,
            rests_at_first_pin: !reach,
            follows_center: reach,
            most_iterations: if reach {
                REACH_MOST_ITERATIONS
            } else {
                MOST_ITERATIONS
            },
            most_passes: if reach {
                REACH_MOST_PASSES
            } else {
                MOST_PASSES
            },
            settles_new_sheet: reach,
            wind: match self {
                ClothGame::Halo2 => ClothWind::Halo2,
                ClothGame::Halo3 => ClothWind::Halo3,
                ClothGame::Reach => ClothWind::PerCloth,
            },
            relaxes_as_halo2: halo2,
            normals_carry_over: halo2,
            back_normal_sign: if halo2 { -1.0 } else { 1.0 },
            derives_binormal: reach,
        }
    }
}

/// The game's ticks a second; see [`ClothRules::tick_rate`].
pub(super) fn tick_rate(game: ClothGame) -> f32 {
    game.rules().tick_rate
}

/// `cloth_update_positions_verlet`'s gravity for one tick at `rate`.
pub(super) fn gravity_per_tick(rate: f32) -> f32 {
    -9.78 * 0.328_084 / rate / 32.0
}
