//! The later engines' exposure model (Reach's, which Halo 4's effects and
//! the cheap particles share): the default light probe sample as effects
//! light by it, and the render target's exposure, `g_exposure` and
//! `g_alt_exposure`.

/// `c_geometry_sampler::m_default_lightprobe_sample` (MCC 0x14201E920, the
/// 360's 0x8527C000, the same floats): its dual vMF, as
/// `set_lightmap_vmf_constants` copies it into `p_lighting_constant_0..3`.
/// Each lobe is a direction with its analytical mask, then a colour with its
/// bandwidth, the lookup table's row.
pub(in crate::app::model_preview) const DEFAULT_DUAL_VMF: [[f32; 4]; 4] = [
    [-0.045971, -0.25616, 0.965541, 1.0],
    [1.754306, 1.543852, 2.033247, 0.294311],
    [-0.174406, -0.984354, 0.025111, 1.0],
    [0.289907, 0.122187, 0.0, 0.996935],
];

/// The same sample's analytical light (+76), directional: the light
/// `convert_light_for_object_lighting` hands on with no level.
#[allow(
    clippy::approx_constant,
    reason = "a sampled direction, not 1/√2; the constant's bits differ"
)]
pub(in crate::app::model_preview) const DEFAULT_LIGHT_DIRECTION: [f32; 3] =
    [-0.122788, -0.696364, 0.707107];
pub(in crate::app::model_preview) const DEFAULT_LIGHT_COLOR: [f32; 3] =
    [4.659558, 3.947189, 3.600812];

/// `effect_lightprobe_setup_lighting_constants` (the 360's 0x82ED2E90, MCC
/// sub_1405CE3C0) of the default sample, which
/// `effect_lightprobe_get_geometry_sample` returns with no level: what a lit
/// particle's `v_lighting_constant_0..3` and `p_lighting_constant_0..3` hold.
/// Effects sample the probe, so the preview's turning of the object's light
/// does not reach them [inferred: with no level, the default sample unturned].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ProbeParticleLight {
    /// `get_bright_direction` (0x8273F890): the analytical light's direction.
    pub(crate) direction: [f32; 3],
    /// `get_bright_color` (0x8273F8C0): `(c1·c1.w + c3 + c2.w²·L) / π`.
    pub(crate) bright: [f32; 3],
    /// `get_dark_color` (0x8273FA00): `(c3 + c1·(1 − c1.w)) / π`.
    pub(crate) dark: [f32; 3],
    /// `convert_to_ambient(…, true)` (0x8273FAF8): `(c1 + c3 + c2.w·L) · 0.5 / π`.
    pub(crate) ambient: [f32; 3],
}

/// The default sample's [`ProbeParticleLight`], from its dual vMF's colours
/// (`c1`, `c3`), their masks and the analytical light's colour `L`.
pub(crate) fn probe_particle_light() -> ProbeParticleLight {
    use std::f32::consts::PI;
    let [_, first, second, third] = DEFAULT_DUAL_VMF;
    let light = DEFAULT_LIGHT_COLOR;
    ProbeParticleLight {
        direction: DEFAULT_LIGHT_DIRECTION,
        bright: std::array::from_fn(|at| {
            (first[at] * first[3] + third[at] + second[3] * second[3] * light[at]) / PI
        }),
        dark: std::array::from_fn(|at| (third[at] + first[at] * (1.0 - first[3])) / PI),
        ambient: std::array::from_fn(|at| {
            (first[at] + third[at] + second[3] * light[at]) * 0.5 / PI
        }),
    }
}

/// `effect_lightprobe_get_lightmap_tint_rgbe` (the 360's 0x82ED2F38, MCC
/// sub_1405CE1B0) of the default sample, as `tint from lightmap` comes out
/// in `default_vs`: the ambient over its exponent in 255ths of a stop (of at
/// most one), back up by it as `initial.rgb · exp2(initial.w)`. The colour
/// goes out as an `argb_color`, so it holds at most 1 \[inferred\].
pub(crate) fn probe_lightmap_tint() -> [f32; 3] {
    let ambient = probe_particle_light().ambient;
    let brightest = ambient[0].max(ambient[1]).max(ambient[2]).max(1e-4);
    let exponent = (brightest.log2().clamp(0.0, 1.0) * 255.0).floor() / 255.0;
    ambient.map(|channel| (channel * 2f32.powf(-exponent)).min(1.0) * 2f32.powf(exponent))
}

/// `c_camera_fx_values::g_HDR_target_stops`: the lit target holds light this
/// many stops down, and the final composite brings it back.
const HDR_TARGET_STOPS: f32 = 3.0;

/// What the stops bring light down by: `2^HDR_TARGET_STOPS`.
pub(in crate::app::model_preview) const HDR_DIVISOR: f32 = 8.0;

/// `get_render_exposure` at the default `camera_fx_settings`' 0 stops and a
/// white point of 1 (the 360's 0x8272B0F8; MCC's not located).
pub(in crate::app::model_preview) const DEFAULT_RENDER_EXPOSURE: f32 = 1.0;
use DEFAULT_RENDER_EXPOSURE as EXPOSURE;

/// `globals\defaults\default.camera_fx_settings`' `self illum preferred` and
/// `self illum scale`, both fixed.
const SELF_ILLUM_PREFERRED: f32 = 0.0;
const SELF_ILLUM_SCALE: f32 = 0.3;

/// `get_render_exposure` (the 360's 0x8272B0F8) with an area screen effect's
/// `stops` (boost less deboost), which `c_player_view::render` hands
/// `setup_camera_fx_parameters` before the scene draws: `2^stops` up on the
/// default's, with no adaptation.
fn render_exposure(stops: f32) -> f32 {
    EXPOSURE * 2f32.powf(stops)
}

/// `get_illum_render_scale` (the 360's 0x8272B198): self-illumination moves
/// by `self illum scale` of the stops, so it scales by
/// `2^(illum − stops)`. 1 at no stops.
fn illum_render_scale(stops: f32) -> f32 {
    let illum = (stops - SELF_ILLUM_PREFERRED) * SELF_ILLUM_SCALE + SELF_ILLUM_PREFERRED;
    2f32.powf(illum - stops)
}

/// `restore_render_target_globals_with_exposure` (the 360's 0x82666F08):
/// `g_exposure`, and the vertex shaders' `v_exposure` alike. Exposure brought
/// down by the target's stops, the stops' factor, exposure, alpha. `stops`
/// is what the area screen effects come to; 0 without one.
pub(crate) fn stops_exposure(stops: f32) -> [f32; 4] {
    let dark = 2f32.powf(HDR_TARGET_STOPS);
    let exposure = render_exposure(stops);
    [exposure / dark, dark, exposure, 1.0]
}

/// The same's `g_alt_exposure` and `v_alt_exposure`: self-illumination's
/// scale, and it under the same. That `.x` is the illum scale itself is
/// lighting.md's reading of 0x82666F08, not re-read with stops on.
pub(crate) fn stops_alt_exposure(stops: f32) -> [f32; 4] {
    let dark = 2f32.powf(HDR_TARGET_STOPS);
    let (illum, exposure) = (illum_render_scale(stops), render_exposure(stops));
    [illum, illum * exposure / dark, exposure, 0.0]
}

#[cfg(test)]
mod tests {
    use super::super::super::super::reach::composite::REACH_DARK_MULTIPLIER;
    use super::*;

    /// `get_render_exposure` and `get_illum_render_scale` at the screen
    /// effects' stops: everything lit by `2^s`, self-illumination by
    /// `2^(0.3s)`; at none, as before.
    #[test]
    fn reach_exposure_moves_by_the_stops_and_illumination_by_less() {
        assert_eq!(stops_exposure(0.0), [1.0 / 8.0, 8.0, 1.0, 1.0]);
        assert_eq!(stops_alt_exposure(0.0), [1.0, 1.0 / 8.0, 1.0, 0.0]);
        assert_eq!(stops_exposure(1.0), [2.0 / 8.0, 8.0, 2.0, 1.0]);
        let alt = stops_alt_exposure(1.0);
        assert!((alt[0] - 2f32.powf(-0.7)).abs() < 1e-6, "{alt:?}");
        assert!((alt[1] - 2f32.powf(0.3) / 8.0).abs() < 1e-6, "{alt:?}");
        assert_eq!(alt[2], 2.0);
        assert!(
            stops_alt_exposure(-2.0)[1] < stops_alt_exposure(0.0)[1],
            "a deboost darkens"
        );
    }
    #[test]
    fn the_lit_target_is_three_stops_down_and_comes_back_up_by_as_much() {
        let dark = 2f32.powf(HDR_TARGET_STOPS);
        assert_eq!(dark, REACH_DARK_MULTIPLIER);
        assert_eq!(EXPOSURE / dark * REACH_DARK_MULTIPLIER, EXPOSURE);
    }
}
