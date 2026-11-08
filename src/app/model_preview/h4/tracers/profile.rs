//! A tracer's packed profile states and its draw.

use super::*;

/// `TracerProfileState`, as the card holds it between frames.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct ProfileState {
    pub(super) position: [f32; 3],
    /// 0 to 1 over its lifespan; the vertex shader drops it at 1.
    pub(super) age: f32,
    pub(super) velocity: [f32; 3],
    pub(super) initial_alpha: f32,
    pub(super) random: [f32; 4],
    pub(super) size: f32,
    pub(super) intensity: f32,
    pub(super) offset: [f32; 2],
    /// Turns.
    pub(super) rotation: f32,
    pub(super) black_point: f32,
    pub(super) palette: f32,
    pub(super) percentile: f32,
    /// How far along the tracer it is, in world units.
    pub(super) length: f32,
    pub(super) lifespan: f32,
    pub(super) color: [f32; 4],
    /// `a` is an exponent: the colour goes by `2^a`.
    pub(super) initial_color: [f32; 4],
}

/// `RawTracerProfileState`'s sixteen words (`tracer_parameters.fxh`).
pub(super) fn pack_profile(profile: &ProfileState) -> [u32; 16] {
    let [x, y, z] = profile.position.map(f32::to_bits);
    let [vx, vy, vz] = profile.velocity;
    let [r0, r1, r2, r3] = profile.random;
    [
        x,
        y,
        z,
        profile.age.to_bits(),
        pack_half2(vx, vy),
        pack_half2(vz, profile.initial_alpha),
        pack_ushort2n(r0, r1),
        pack_ushort2n(r2, r3),
        pack_half2(profile.size, profile.intensity),
        pack_half2(profile.offset[0], profile.offset[1]),
        pack_ushort2n(profile.rotation, profile.black_point),
        pack_ushort2n(profile.palette, profile.percentile),
        pack_half2(profile.length, profile.lifespan),
        pack_argb8(profile.color),
        pack_argb8(profile.initial_color),
        0,
    ]
}

impl TracerDefinition {
    /// What `tracer_stateless_setup.fx` and `tracer_update.fx` both set from
    /// the properties: rotation, colour, size, offset, intensity, black
    /// point and palette.
    pub(super) fn apply_properties(&self, state: &TracerState, profile: &mut ProfileState) {
        let [red, green, blue] = self.color.color(state);
        let [x, y, _] = self.profile_offset.vector(state);
        profile.rotation = self.rotation.scalar(state).rem_euclid(1.0);
        profile.color = [
            red,
            green,
            blue,
            self.alpha.scalar(state) * self.alpha2.scalar(state),
        ];
        profile.size = self.size.scalar(state);
        profile.offset = [x, y];
        profile.intensity = self.intensity.scalar(state);
        profile.black_point = self.black_point.scalar(state).rem_euclid(1.0);
        profile.palette = self.palette.scalar(state).rem_euclid(1.0);
    }
}

/// One tracer's frame: `vs_tracerOverallState` and its profiles' packed
/// states, in strip order.
#[derive(Debug, Clone)]
pub(crate) struct H4TracerDraw {
    pub(crate) shape: f32,
    pub(crate) ngon_sides: f32,
    pub(crate) appearance: f32,
    pub(crate) percentile_step: f32,
    pub(crate) capped_length: f32,
    pub(crate) offset: f32,
    /// `localSpaceOffset_gameTime`: no offset, and the seconds since the
    /// system began.
    pub(crate) game_time: f32,
    pub(crate) uv_tiling: [f32; 2],
    pub(crate) uv_scroll: [f32; 2],
    pub(crate) uv_offset: [f32; 2],
    pub(crate) origin: [f32; 3],
    pub(crate) direction: [f32; 3],
    pub(crate) fade: [f32; 4],
    pub(crate) inputs: [f32; STATE_COUNT],
    pub(crate) strips: u32,
    pub(crate) states: Vec<[u32; 16]>,
}

/// `1 / (length · density)`, held to 0..1, an infinite step 1.
pub(super) fn percentile_step(length: f32, density: f32) -> f32 {
    let step = (length * density).recip();
    if step.is_nan() || step <= 0.0 {
        0.0
    } else {
        step.min(1.0)
    }
}

/// `clamp(ceil(length · density), 2, 128)`.
pub(super) fn profile_count(length: f32, density: f32) -> usize {
    let wanted = (length * density).ceil();
    if wanted.is_nan() {
        2
    } else {
        (wanted.max(2.0) as usize).min(MOST_PROFILES)
    }
}
