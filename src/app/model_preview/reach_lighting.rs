//! Reach's objects lit with no level around them. Reach lights by a dual vMF
//! (two lobes, a lookup table across them) and one analytical light, where
//! Halo 3 lit by nine spherical harmonics, and its final composite has no tone
//! curve: the frame is encoded by a square root.

use super::h3_shaders::H3Value;
use super::*;

/// `c_geometry_sampler::m_default_lightprobe_sample` (MCC 0x14201E920, the
/// 360's 0x8527C000, the same floats): its dual vMF, as
/// `set_lightmap_vmf_constants` copies it into `p_lighting_constant_0..3`.
/// Each lobe is a direction with its analytical mask, then a colour with its
/// bandwidth, the lookup table's row.
const DEFAULT_DUAL_VMF: [[f32; 4]; 4] = [
    [-0.045971, -0.25616, 0.965541, 1.0],
    [1.754306, 1.543852, 2.033247, 0.294311],
    [-0.174406, -0.984354, 0.025111, 1.0],
    [0.289907, 0.122187, 0.0, 0.996935],
];

/// The same sample's analytical light (+76), directional: the light
/// `convert_light_for_object_lighting` hands on with no level.
const DEFAULT_LIGHT_DIRECTION: [f32; 3] = [-0.122788, -0.696364, 0.707107];
const DEFAULT_LIGHT_COLOR: [f32; 3] = [4.659558, 3.947189, 3.600812];

/// `c_camera_fx_values::g_HDR_target_stops`: the lit target holds light this
/// many stops down, and the final composite brings it back.
const HDR_TARGET_STOPS: f32 = 3.0;

/// `get_render_exposure` at the default `camera_fx_settings`' 0 stops and a
/// white point of 1 (the 360's 0x8272B0F8; MCC's not located).
const EXPOSURE: f32 = 1.0;

/// What `setup_object_lighting_for_pass` (the 360's 0x826DD650) gives an
/// object's static lighting pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ReachObjectLighting {
    pub(super) dual_vmf: [[f32; 4]; 4],
    pub(super) direction: [f32; 3],
    pub(super) intensity: [f32; 3],
}

fn sum(color: [f32; 3]) -> f32 {
    color[0] + color[1] + color[2]
}

fn normalized(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length > 0.0 { vector.map(|value| value / length) } else { vector }
}

/// `s_lightmap_sample::construct_analytical_light_from_dualvmf` (MCC
/// 0x140828CD0, the 360's 0x827402C8): the light moved toward the vMF's
/// lobes by how little of the light it carries, and that share.
fn analytical_light_from_dual_vmf(vmf: &[[f32; 4]; 4], direction: [f32; 3], color: [f32; 3]) -> ([f32; 3], [f32; 3], f32) {
    let mask = vmf[0][3];
    let [first, second] = [vmf[1], vmf[3]].map(|lobe| sum([lobe[0], lobe[1], lobe[2]]));
    let total = (first + second).max(0.00001);
    let (first, second) = (first / total, second / total);
    let carried = sum(color) * mask;
    let mut share = mask;
    if total + carried > 0.0001 && carried / (total + carried) <= mask {
        share = carried / (total + carried);
    }
    let lobes = normalized(std::array::from_fn(|axis| vmf[0][axis] * first + vmf[2][axis] * second));
    let toward = normalized(std::array::from_fn(|axis| direction[axis] * share + lobes[axis] * (1.0 - share)));
    let color = std::array::from_fn(|channel| {
        (vmf[1][channel] * first + vmf[3][channel] * second) * (1.0 - share) + color[channel] * share
    });
    (toward, color, share)
}

/// Turn a direction about the vertical.
fn turned(direction: [f32; 3], angle: f32) -> [f32; 3] {
    let (sin, cos) = angle.sin_cos();
    [cos * direction[0] - sin * direction[1], sin * direction[0] + cos * direction[1], direction[2]]
}

/// The default sample's lighting, its lobes and light turned about the
/// vertical by `angle`: the preview's own, for a light that follows the camera.
pub(super) fn reach_object_lighting(angle: f32) -> ReachObjectLighting {
    let mut sample = DEFAULT_DUAL_VMF;
    for lobe in [0, 2] {
        let [x, y, z] = turned([sample[lobe][0], sample[lobe][1], sample[lobe][2]], angle);
        sample[lobe][..3].copy_from_slice(&[x, y, z]);
    }
    let (direction, intensity, share) =
        analytical_light_from_dual_vmf(&sample, turned(DEFAULT_LIGHT_DIRECTION, angle), DEFAULT_LIGHT_COLOR);
    // The light now carries its share; the vMF's own mask is full, and the
    // second lobe's says how much the analytical light holds.
    sample[0][3] = 1.0;
    sample[2][3] = share;
    ReachObjectLighting { dual_vmf: sample, direction, intensity }
}

/// Which way, about the vertical, the default light comes from.
fn light_heading() -> f32 {
    DEFAULT_LIGHT_DIRECTION[1].atan2(DEFAULT_LIGHT_DIRECTION[0])
}

/// The lighting for this frame: the game's, or turned to come from over the
/// viewer's shoulder when the light follows the camera.
pub(super) fn reach_frame_lighting(frame: &ModelGpuFrame) -> ReachObjectLighting {
    if !frame.light_follows_camera {
        return reach_object_lighting(0.0);
    }
    let [x, y, _] = h2_camera_light(frame.camera.yaw, frame.camera.pitch);
    reach_object_lighting(y.atan2(x) - light_heading())
}

/// What Reach's rasterizer globals hold for an object here, by uniform name.
/// Anything not named falls to what Reach shares with Halo 3.
pub(super) fn reach_frame_value(frame: &ModelGpuFrame, name: &str) -> Option<H3Value> {
    let floats = |value: [f32; 4]| Some(H3Value::Floats(value));
    let lighting = || reach_frame_lighting(frame);
    if let Some(index) = name.strip_prefix("p_lighting_constant_").and_then(|index| index.parse::<usize>().ok()) {
        return lighting().dual_vmf.get(index).copied().map(H3Value::Floats);
    }
    let dark = 2f32.powf(HDR_TARGET_STOPS);
    match name {
        "k_ps_analytical_light_direction" | "v_analytical_light_direction" => {
            let [x, y, z] = lighting().direction;
            floats([x, y, z, 0.0])
        }
        // `set_lightmap_analytical_light_constants`: at full intensity, with
        // no rain to dim it.
        "k_ps_analytical_light_intensity" => {
            let [r, g, b] = lighting().intensity;
            floats([r, g, b, 0.0])
        }
        "k_ps_bool_is_uber_light_directional" => Some(H3Value::Integer(1)),
        // The default sample has no bounce.
        "k_ps_bounce_light_direction" | "k_ps_bounce_light_intensity" => floats([0.0; 4]),
        "shadow_mask_enabled" | "vs_boolean_enable_wet_effect" | "ps_boolean_enable_wet_effect" => {
            Some(H3Value::Integer(0))
        }
        // `setup_per_object_render_state` (MCC 0x140885320): no level offsets
        // the cube maps' levels, and the object's cube map sample, at its
        // defaults, weighs the first map whole.
        "dynamic_environment_blend" => floats([0.0, 0.0, 0.0, 1.0]),
        // `restore_render_target_globals_with_exposure`: exposure brought down
        // by the target's stops, the stops' factor, exposure, alpha.
        "g_exposure" => floats([EXPOSURE / dark, dark, EXPOSURE, 1.0]),
        // Self-illumination's scale at no stops, and it under the same.
        "g_alt_exposure" => floats([1.0, EXPOSURE / dark, EXPOSURE, 0.0]),
        "object_velocity" | "antialias_scalars" => floats([0.0; 4]),
        _ => None,
    }
}

/// `final_composite_base`'s factor on the lit target and on the bloom.
pub(super) const REACH_DARK_MULTIPLIER: f32 = 8.0;

/// `final_composite`, as MCC runs it at the default settings: the lit target
/// and bloom brought back up, `pow(x, 0.5)` for the default gamma
/// (`convert_output_gamma2`), the colour matrix, and nothing after. The tone
/// curve's call is commented out in the source.
pub(super) const REACH_COMPOSITE_FRAGMENT: &str = "#version 410 core
uniform sampler2D accumulation;
uniform sampler2D bloom;
uniform mat4 hue_saturation;
uniform float dark_multiplier;
uniform float view_scale;
out vec4 color;
void main() {
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    place = (place - 0.5) * view_scale + 0.5;
    vec4 accumulated = texture(accumulation, place);
    if (place.x < 0.0 || place.x > 1.0 || place.y < 0.0 || place.y > 1.0) {
        accumulated = vec4(0.0);
    }
    vec3 combined = (accumulated.rgb + texture(bloom, place).rgb) * dark_multiplier;
    combined = sqrt(max(combined, vec3(0.0)));
    color.rgb = clamp((vec4(combined, 1.0) * hue_saturation).rgb, 0.0, 1.0);
    color.a = accumulated.a;
}
";

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    #[test]
    fn the_default_light_takes_most_of_the_vmf_and_turns_toward_its_lobes() {
        let lighting = reach_object_lighting(0.0);
        assert!((lighting.dual_vmf[2][3] - 0.680047).abs() < 1e-5, "{}", lighting.dual_vmf[2][3]);
        assert_eq!(lighting.dual_vmf[0][3], 1.0);
        assert!(close(lighting.direction, [-0.104348, -0.590215, 0.800473]), "{:?}", lighting.direction);
        assert!(close(lighting.intensity, [3.696396, 3.145597, 3.052588]), "{:?}", lighting.intensity);
        assert!(!close(lighting.direction, DEFAULT_LIGHT_DIRECTION), "the lobes move it");
    }

    #[test]
    fn a_light_brighter_than_the_mask_allows_takes_only_the_mask() {
        let mut vmf = DEFAULT_DUAL_VMF;
        vmf[0][3] = 0.25;
        let (_, _, share) = analytical_light_from_dual_vmf(&vmf, DEFAULT_LIGHT_DIRECTION, DEFAULT_LIGHT_COLOR);
        assert_eq!(share, 0.25);
        let (direction, color, share) = analytical_light_from_dual_vmf(&DEFAULT_DUAL_VMF, [0.0, 0.0, 1.0], [0.0; 3]);
        assert_eq!(share, 0.0, "no light, none of the share");
        assert!(close(color, std::array::from_fn(|at| {
            let total = sum([1.754306, 1.543852, 2.033247]) + sum([0.289907, 0.122187, 0.0]);
            (DEFAULT_DUAL_VMF[1][at] * sum([1.754306, 1.543852, 2.033247]) + DEFAULT_DUAL_VMF[3][at] * sum([0.289907, 0.122187, 0.0])) / total
        })));
        assert!(direction[2] > 0.0);
    }

    #[test]
    fn turning_moves_the_lobes_and_the_light_together_and_keeps_the_sky() {
        let still = reach_object_lighting(0.0);
        let turned = reach_object_lighting(std::f32::consts::FRAC_PI_2);
        assert!((still.direction[2] - turned.direction[2]).abs() < 1e-5);
        assert!(close(turned.direction, [-still.direction[1], still.direction[0], still.direction[2]]), "{:?}", turned.direction);
        assert!(close(turned.intensity, still.intensity));
        assert_eq!(turned.dual_vmf[1], still.dual_vmf[1]);
    }

    #[test]
    fn the_lit_target_is_three_stops_down_and_comes_back_up_by_as_much() {
        let dark = 2f32.powf(HDR_TARGET_STOPS);
        assert_eq!(dark, REACH_DARK_MULTIPLIER);
        assert_eq!(EXPOSURE / dark * REACH_DARK_MULTIPLIER, EXPOSURE);
    }
}
