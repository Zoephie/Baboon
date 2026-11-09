//! Reach's objects lit with no level around them. Reach lights by a dual vMF
//! (two lobes, a lookup table across them) and one analytical light, where
//! Halo 3 lit by nine spherical harmonics, and its final composite has no tone
//! curve: the frame is encoded by a square root.

use super::super::common::camera::camera_light;
use super::super::frame::ModelGpuFrame;
use super::super::gen3::pipeline::{
    DEFAULT_DUAL_VMF, DEFAULT_LIGHT_COLOR, DEFAULT_LIGHT_DIRECTION, reach_alt_exposure,
    reach_exposure,
};
use super::super::gen3::shaders::Gen3Value;
use crate::app::model_preview::common::math::unit_or_unchanged;

/// What `setup_object_lighting_for_pass` (the 360's 0x826DD650) gives an
/// object's static lighting pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) struct ReachObjectLighting {
    pub(in crate::app::model_preview) dual_vmf: [[f32; 4]; 4],
    pub(in crate::app::model_preview) direction: [f32; 3],
    pub(in crate::app::model_preview) intensity: [f32; 3],
}

fn sum(color: [f32; 3]) -> f32 {
    color[0] + color[1] + color[2]
}

/// `s_lightmap_sample::construct_analytical_light_from_dualvmf` (MCC
/// 0x140828CD0, the 360's 0x827402C8): the light moved toward the vMF's
/// lobes by how little of the light it carries, and that share.
fn analytical_light_from_dual_vmf(
    vmf: &[[f32; 4]; 4],
    direction: [f32; 3],
    color: [f32; 3],
) -> ([f32; 3], [f32; 3], f32) {
    let mask = vmf[0][3];
    let [first, second] = [vmf[1], vmf[3]].map(|lobe| sum([lobe[0], lobe[1], lobe[2]]));
    let total = (first + second).max(0.00001);
    let (first, second) = (first / total, second / total);
    let carried = sum(color) * mask;
    let mut share = mask;
    if total + carried > 0.0001 && carried / (total + carried) <= mask {
        share = carried / (total + carried);
    }
    let lobes = unit_or_unchanged(std::array::from_fn(|axis| {
        vmf[0][axis] * first + vmf[2][axis] * second
    }));
    let toward = unit_or_unchanged(std::array::from_fn(|axis| {
        direction[axis] * share + lobes[axis] * (1.0 - share)
    }));
    let color = std::array::from_fn(|channel| {
        (vmf[1][channel] * first + vmf[3][channel] * second) * (1.0 - share)
            + color[channel] * share
    });
    (toward, color, share)
}

/// Turn a direction about the vertical.
fn turned(direction: [f32; 3], angle: f32) -> [f32; 3] {
    let (sin, cos) = angle.sin_cos();
    [
        cos * direction[0] - sin * direction[1],
        sin * direction[0] + cos * direction[1],
        direction[2],
    ]
}

/// The default sample's lighting, its lobes and light turned about the
/// vertical by `angle`: the preview's own, for a light that follows the camera.
pub(in crate::app::model_preview) fn reach_object_lighting(angle: f32) -> ReachObjectLighting {
    let mut sample = DEFAULT_DUAL_VMF;
    for lobe in [0, 2] {
        let [x, y, z] = turned([sample[lobe][0], sample[lobe][1], sample[lobe][2]], angle);
        sample[lobe][..3].copy_from_slice(&[x, y, z]);
    }
    let (direction, intensity, share) = analytical_light_from_dual_vmf(
        &sample,
        turned(DEFAULT_LIGHT_DIRECTION, angle),
        DEFAULT_LIGHT_COLOR,
    );
    // The light now carries its share; the vMF's own mask is full, and the
    // second lobe's says how much the analytical light holds.
    sample[0][3] = 1.0;
    sample[2][3] = share;
    ReachObjectLighting {
        dual_vmf: sample,
        direction,
        intensity,
    }
}

/// `s_lightmap_sample::calculate_shadow_direction` of the default sample,
/// turned about the vertical by `angle`: away from the brightness-weighted
/// sum of the light and both lobes, taken before the light is folded in.
fn reach_shadow_direction_at(angle: f32) -> [f32; 3] {
    let brightness = |color: [f32; 3]| 0.299 * color[0] + 0.587 * color[1] + 0.114 * color[2];
    let lobe = |row: usize| {
        turned(
            [
                DEFAULT_DUAL_VMF[row][0],
                DEFAULT_DUAL_VMF[row][1],
                DEFAULT_DUAL_VMF[row][2],
            ],
            angle,
        )
    };
    let color = |row: usize| {
        [
            DEFAULT_DUAL_VMF[row][0],
            DEFAULT_DUAL_VMF[row][1],
            DEFAULT_DUAL_VMF[row][2],
        ]
    };
    let light = turned(DEFAULT_LIGHT_DIRECTION, angle);
    let (first, second) = (lobe(0), lobe(2));
    let weights = [
        DEFAULT_DUAL_VMF[0][3] * brightness(DEFAULT_LIGHT_COLOR),
        brightness(color(1)),
        brightness(color(3)),
    ];
    let sum: [f32; 3] = std::array::from_fn(|axis| {
        light[axis] * weights[0] + first[axis] * weights[1] + second[axis] * weights[2]
    });
    unit_or_unchanged(sum).map(|value| -value)
}

/// The frame's shadow direction, from the light toward the object.
pub(in crate::app::model_preview) fn reach_shadow_direction(frame: &ModelGpuFrame) -> [f32; 3] {
    if !frame.light_follows_camera {
        return reach_shadow_direction_at(0.0);
    }
    let [x, y, _] = camera_light(frame.camera.yaw, frame.camera.pitch);
    reach_shadow_direction_at(y.atan2(x) - light_heading())
}

/// Which way, about the vertical, the default light comes from.
fn light_heading() -> f32 {
    DEFAULT_LIGHT_DIRECTION[1].atan2(DEFAULT_LIGHT_DIRECTION[0])
}

/// The lighting for this frame: the game's, or turned to come from over the
/// viewer's shoulder when the light follows the camera.
pub(in crate::app::model_preview) fn reach_frame_lighting(
    frame: &ModelGpuFrame,
) -> ReachObjectLighting {
    if !frame.light_follows_camera {
        return reach_object_lighting(0.0);
    }
    let [x, y, _] = camera_light(frame.camera.yaw, frame.camera.pitch);
    reach_object_lighting(y.atan2(x) - light_heading())
}

/// Which of the object's dual vMF a uniform reads. `set_lightmap_vmf_constants`
/// (X360 0x826DD8D0) writes the same four to the pixel stage's c1 and the
/// vertex stage's c240, which foliage lights its vertices by.
fn reach_vmf_index(name: &str) -> Option<usize> {
    let index = name
        .strip_prefix("p_lighting_constant_")
        .or_else(|| name.strip_prefix("v_lighting_constant_"))?;
    index.parse().ok()
}

/// Reach's [`FrameValues`](super::super::gen3::pipeline::FrameValues):
/// its own, then Halo 3's, but for the lighting constants past its four
/// vMF, which are not an object's.
pub(in crate::app::model_preview) fn reach_frame_values(
    frame: &ModelGpuFrame,
    name: &str,
    size: [i32; 2],
    stops: f32,
) -> Option<Gen3Value> {
    if let Some(value) = reach_frame_value(frame, name, stops) {
        return Some(value);
    }
    if name.starts_with("p_lighting_constant_") || name.starts_with("v_lighting_constant_") {
        return None;
    }
    super::super::gen3::pipeline::frame_value(frame, name, size)
}

/// What Reach's rasterizer globals hold for an object here, by uniform name,
/// at the area screen effects' exposure `stops`. Anything not named falls to
/// what Reach shares with Halo 3.
pub(in crate::app::model_preview) fn reach_frame_value(
    frame: &ModelGpuFrame,
    name: &str,
    stops: f32,
) -> Option<Gen3Value> {
    let floats = |value: [f32; 4]| Some(Gen3Value::Floats(value));
    let lighting = || reach_frame_lighting(frame);
    if let Some(index) = reach_vmf_index(name) {
        return lighting()
            .dual_vmf
            .get(index)
            .copied()
            .map(Gen3Value::Floats);
    }
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
        "k_ps_bool_is_uber_light_directional" => Some(Gen3Value::Integer(1)),
        // The default sample has no bounce.
        "k_ps_bounce_light_direction" | "k_ps_bounce_light_intensity" => floats([0.0; 4]),
        "shadow_mask_enabled" | "vs_boolean_enable_wet_effect" | "ps_boolean_enable_wet_effect" => {
            Some(Gen3Value::Integer(0))
        }
        // `setup_per_object_render_state` (MCC 0x140885320): no level offsets
        // the cube maps' levels, and the object's cube map sample, at its
        // defaults, weighs the first map whole.
        "dynamic_environment_blend" => floats([0.0, 0.0, 0.0, 1.0]),
        "g_exposure" => floats(reach_exposure(stops)),
        "g_alt_exposure" => floats(reach_alt_exposure(stops)),
        "object_velocity" | "antialias_scalars" => floats([0.0; 4]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::gen3::pipeline::{reach_lightmap_tint, reach_particle_light};
    use super::*;

    /// Foliage reads the vMF in its vertex stage; both stages name the same four.
    #[test]
    fn both_stages_read_the_objects_vmf() {
        assert_eq!(reach_vmf_index("p_lighting_constant_2"), Some(2));
        assert_eq!(reach_vmf_index("v_lighting_constant_3"), Some(3));
        assert_eq!(reach_vmf_index("v_analytical_light_direction"), None);
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    #[test]
    fn the_default_light_takes_most_of_the_vmf_and_turns_toward_its_lobes() {
        let lighting = reach_object_lighting(0.0);
        assert!(
            (lighting.dual_vmf[2][3] - 0.680047).abs() < 1e-5,
            "{}",
            lighting.dual_vmf[2][3]
        );
        assert_eq!(lighting.dual_vmf[0][3], 1.0);
        assert!(
            close(lighting.direction, [-0.104348, -0.590215, 0.800473]),
            "{:?}",
            lighting.direction
        );
        assert!(
            close(lighting.intensity, [3.696396, 3.145597, 3.052588]),
            "{:?}",
            lighting.intensity
        );
        assert!(
            !close(lighting.direction, DEFAULT_LIGHT_DIRECTION),
            "the lobes move it"
        );
    }

    #[test]
    fn the_default_shadow_falls_away_from_the_samples_light() {
        let direction = reach_shadow_direction_at(0.0);
        assert!(
            close(direction, [0.106611, 0.603164, -0.790460]),
            "{direction:?}"
        );
        assert!(
            !close(reach_shadow_direction_at(1.0), direction),
            "it turns with the light"
        );
    }

    #[test]
    fn a_light_brighter_than_the_mask_allows_takes_only_the_mask() {
        let mut vmf = DEFAULT_DUAL_VMF;
        vmf[0][3] = 0.25;
        let (_, _, share) =
            analytical_light_from_dual_vmf(&vmf, DEFAULT_LIGHT_DIRECTION, DEFAULT_LIGHT_COLOR);
        assert_eq!(share, 0.25);
        let (direction, color, share) =
            analytical_light_from_dual_vmf(&DEFAULT_DUAL_VMF, [0.0, 0.0, 1.0], [0.0; 3]);
        assert_eq!(share, 0.0, "no light, none of the share");
        assert!(close(
            color,
            std::array::from_fn(|at| {
                let total = sum([1.754306, 1.543852, 2.033247]) + sum([0.289907, 0.122187, 0.0]);
                (DEFAULT_DUAL_VMF[1][at] * sum([1.754306, 1.543852, 2.033247])
                    + DEFAULT_DUAL_VMF[3][at] * sum([0.289907, 0.122187, 0.0]))
                    / total
            })
        ));
        assert!(direction[2] > 0.0);
    }

    #[test]
    fn turning_moves_the_lobes_and_the_light_together_and_keeps_the_sky() {
        let still = reach_object_lighting(0.0);
        let turned = reach_object_lighting(std::f32::consts::FRAC_PI_2);
        assert!((still.direction[2] - turned.direction[2]).abs() < 1e-5);
        assert!(
            close(
                turned.direction,
                [-still.direction[1], still.direction[0], still.direction[2]]
            ),
            "{:?}",
            turned.direction
        );
        assert!(close(turned.intensity, still.intensity));
        assert_eq!(turned.dual_vmf[1], still.dual_vmf[1]);
    }

    /// The numbers the default sample works out to by hand.
    #[test]
    fn a_lit_particle_takes_the_default_samples_bright_dark_and_ambient() {
        let light = reach_particle_light();
        assert!(
            close(light.direction, DEFAULT_LIGHT_DIRECTION),
            "{:?}",
            light.direction
        );
        assert!(
            close(light.bright, [1.739811, 1.439954, 1.336653]),
            "{:?}",
            light.bright
        );
        assert!(
            close(light.dark, [0.486346, 0.385685, 0.456724]),
            "{:?}",
            light.dark
        );
        assert!(
            close(light.ambient, [1.066938, 0.893373, 0.896688]),
            "{:?}",
            light.ambient
        );
        let objects = reach_object_lighting(0.0).direction;
        assert!(
            !close(light.direction, objects),
            "the light as sampled, not as objects get it"
        );
    }

    /// e = 23/255: red, over one, holds at one under it, and the rest come back
    /// as they were.
    #[test]
    fn a_lightmap_tint_keeps_its_exponent_in_255ths_and_its_colour_under_one() {
        let tint = reach_lightmap_tint();
        assert!(close(tint, [1.064515, 0.893373, 0.896688]), "{tint:?}");
        assert!(
            tint[0] < reach_particle_light().ambient[0],
            "clamped before the exponent comes back"
        );
    }
}
