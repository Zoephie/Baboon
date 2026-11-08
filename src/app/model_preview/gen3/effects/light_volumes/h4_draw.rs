//! Halo 4's packed light volume draw.

use super::*;

/// `light_volume_state_input_enum`'s order, the order of
/// `LightVolumeOverallState.inputs`; `allocate_random_seed` leaves nothing
/// reading `random`.
pub(super) const H4_STATE_ORDER: [H3VolumeInput; 10] = [
    H3VolumeInput::Percentile,
    H3VolumeInput::GameTime,
    H3VolumeInput::Age,
    H3VolumeInput::Random,
    H3VolumeInput::Seed(0),
    H3VolumeInput::Seed(1),
    H3VolumeInput::Lod,
    H3VolumeInput::EffectA,
    H3VolumeInput::EffectB,
    H3VolumeInput::Invalid,
];

/// `ProfileMemexportedState`, as `light_volume_main` writes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct H4ProfileState {
    pub(super) position: [f32; 3],
    pub(super) percentile: f32,
    /// Unclamped; the packing saturates it.
    pub(super) color: [f32; 4],
    pub(super) thickness: f32,
    pub(super) intensity: f32,
}

/// `WriteProfileMemexportedState`: `RawProfileState`'s eight words, the
/// thickness and intensity as halves, the colour as ARGB8, the pad 0.
pub(super) fn pack_profile_state(state: &H4ProfileState) -> [u32; 8] {
    let [x, y, z] = state.position.map(f32::to_bits);
    let misc = f32_to_f16(state.thickness) | (f32_to_f16(state.intensity) << 16);
    [x, y, z, state.percentile.to_bits(), misc, pack_argb8(state.color), 0, 0]
}

/// One Halo 4 volume's frame: `LightVolumeOverallState` and its profiles'
/// packed states, in row order.
#[derive(Debug, Clone)]
pub(crate) struct H4VolumeDraw {
    pub(crate) appearance: f32,
    pub(crate) brightness_ratio: f32,
    pub(crate) offset: f32,
    /// `numProfiles`: `length × density`, whole or not.
    pub(crate) profiles: f32,
    /// `profileDistance`.
    pub(crate) distance: f32,
    /// `profileLength`.
    pub(crate) side_length: f32,
    pub(crate) origin: [f32; 3],
    pub(crate) direction: [f32; 3],
    pub(crate) inputs: [f32; 10],
    pub(crate) states: Vec<[u32; 8]>,
}
