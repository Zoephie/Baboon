//! Halo 4's object self shadow with no level (`render_third_person_shadows`,
//! X360 0x83A98D18, MCC 0x140896E40): who casts, the camera the shadow is
//! drawn from, and the constants of the variance map and its apply. See
//! `docs/h4-preview-research/shadow-camo-shields.md` §1.

use blam_tags::TagFile;

use super::super::common::fields::{has_flag, object_block};
use crate::app::model_preview::common::math::{cross, dot, unit_or_unchanged};

/// `set_depth_biases(DynamicLightShadowBias, DynamicLightShadowSlopeScaledBias)`
/// (X .data 0x86E98CA4/A8): the generate's biases, whatever the model sets.
pub(crate) const DEPTH_BIAS: f32 = 1e-4;
pub(crate) const SLOPE_BIAS: f32 = 1.414;
/// `submit_visibility_and_render`'s largest map.
const MOST_PIXELS: i32 = 512;
/// `ps_shadow_parameters[9]`: `VSMEpsilon`, `VSMDPow`, `VSMAttenuationPow`
/// (X 0x86E3EAD4, M 0x142498294).
pub(crate) const VSM: [f32; 3] = [0.002, 1.51, 4.0];
/// `FADE_RESCALE`, `FADE_REOFFSET` (c182): the falloff over the box's lower
/// three quarters [inferred: `2·this[+120]/this[+56]` is 2 for a box whose
/// depth runs 0 to 1].
pub(crate) const FADE: [f32; 2] = [2.0, -0.5];
/// `ps_bsp_lightmap_scale_constants.y`, the floor under the shadowed direct
/// light: `setup_object_lighting_for_pass` writes it only with a level, and
/// 0.3 is its in-level default and the shader's own fallback \[inferred\].
pub(crate) const DIRECT_FLOOR: f32 = 0.3;

/// `RenderObjectGetShadowTypeFromObjectDefinition` (X360 0x83ABF928, MCC
/// 0x1408A64A0) for the third-person path: a lightmap shadow (types 2, 3, 4).
/// `hires` is type 4, which `CalculateShadowMode` gives its own map and the
/// apply without its normal test (explicit 87).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct H4ShadowCaster {
    pub(crate) hires: bool,
}

pub(crate) fn h4_shadow_caster(object: &TagFile) -> Option<H4ShadowCaster> {
    let holder = object_block(object, "lightmap shadow mode")?;
    if has_flag(holder, "flags", "does not cast shadow") {
        return None;
    }
    let mode = holder
        .read_enum_option("lightmap shadow mode")
        .unwrap_or_default();
    shadow_type(&mode, &object.group().tag.to_be_bytes())
        .filter(|kind| matches!(kind, 2..=4))
        .map(|kind| H4ShadowCaster { hires: kind == 4 })
}

/// The shadow type by `lightmap shadow mode` and object group.
fn shadow_type(mode: &str, group: &[u8; 4]) -> Option<u8> {
    match mode {
        "default" => match group {
            b"bipd" | b"vehi" => Some(2),
            b"weap" | b"bloc" => Some(4),
            b"mach" => Some(1),
            _ => None,
        },
        "always" | "blur" => (!matches!(group, b"eqip" | b"proj" | b"ssce")).then_some(2),
        _ => None,
    }
}

type Vector = [f32; 3];

/// `object_shadow_visible`'s camera (X360 0x83A98470) over one object's
/// bounding sphere, and the map it draws into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H4ShadowCamera {
    /// From the light toward the object.
    pub(crate) direction: Vector,
    /// The map's side, in texels.
    pub(crate) pixels: i32,
    /// `View_Projection` for the map, rowed for HLSL: x and y across it,
    /// depth along the light into OpenGL's [-1, 1].
    pub(crate) view_projection: [f32; 16],
}

impl H4ShadowCamera {
    /// With no level the default lighting sample's shadow direction is
    /// straight down (`calculate_shadow_direction`).
    pub(crate) const DOWN: Vector = [0.0, 0.0, -1.0];

    /// `direction` from the light; the sphere at `center` of `radius`. The box
    /// is the sphere's width across, and reaches from the light plane at the
    /// sphere's near side 1.2 diameters along the light [inferred: the depth
    /// formula's operands are partly lost in the decompile].
    pub(crate) fn new(direction: Vector, center: Vector, radius: f32) -> Self {
        let d = unit_or_unchanged(direction);
        let radius = radius.max(2e-4);
        // `up` across the light, off `global_left`, or `global_backward`
        // when the light runs near it.
        let side = if dot(d, [0.0, 1.0, 0.0]).abs() >= 0.9 {
            [-1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let up = unit_or_unchanged(cross(d, side));
        let left = cross(up, d);
        let plane: Vector = std::array::from_fn(|axis| center[axis] - radius * d[axis]);
        let depth = 1.2 * 2.0 * radius;
        // `CalculateShadowMode` at a resolution of one or more: mode 8,
        // `res = 0.5`, so `n = ceil((0.5·512 + 1) / 8)·8` [inferred: an object
        // the preview frames takes the whole surface].
        let pixels = ((((0.5 * MOST_PIXELS as f32) as i32 + 1) as f32 / 8.0).ceil() as i32 * 8)
            .clamp(16, MOST_PIXELS);
        let rows = [
            (left.map(|value| value / radius), -dot(plane, left) / radius),
            (up.map(|value| value / radius), -dot(plane, up) / radius),
            (
                d.map(|value| 2.0 * value / depth),
                -2.0 * dot(plane, d) / depth - 1.0,
            ),
        ];
        let mut view_projection = [0.0; 16];
        for (column, (axis, offset)) in rows.iter().enumerate() {
            for row in 0..3 {
                view_projection[row * 4 + column] = axis[row];
            }
            view_projection[12 + column] = *offset;
        }
        view_projection[15] = 1.0;
        Self {
            direction: d,
            pixels,
            view_projection,
        }
    }

    /// `ps_gaussian_parameters` for mode 8 at full resolution: the
    /// horizontal blur's weights, and the vertical one's offsets in texels.
    pub(crate) fn gaussian(&self) -> [[f32; 4]; 2] {
        let n = self.pixels as f32;
        [
            [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 0.0],
            [-1.5 / n, -0.5 / n, 0.5 / n, 1.5 / n],
        ]
    }

    /// `ps_shadow_direction`: the light's way back, in the space the normal
    /// buffer holds (`mul(normal, ps_worldspace_normal_axis)`).
    pub(crate) fn direction_in(&self, axis: &[f32; 9]) -> Vector {
        let back = self.direction.map(|value| -value);
        std::array::from_fn(|column| (0..3).map(|row| back[row] * axis[row * 3 + column]).sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::harness::kit::kit_source;
    use crate::core::game::GameId;

    #[test]
    fn bipeds_vehicles_weapons_and_crates_cast_by_default() {
        assert_eq!(shadow_type("default", b"bipd"), Some(2));
        assert_eq!(
            shadow_type("default", b"weap"),
            Some(4),
            "weapons and crates take the high resolution map"
        );
        assert_eq!(
            shadow_type("default", b"mach"),
            Some(1),
            "a machine's shadow is dynamic, not this one"
        );
        assert_eq!(shadow_type("default", b"scen"), None);
        assert_eq!(shadow_type("always", b"scen"), Some(2));
        assert_eq!(shadow_type("always", b"eqip"), None);
        assert_eq!(shadow_type("never", b"bipd"), None);
    }

    /// The sphere sits in the box: its top on the light plane at clip depth
    /// -1, its bottom inside, its middle at the map's centre.
    #[test]
    fn the_box_holds_the_sphere_from_the_light_plane_down() {
        let center = [1.0, 2.0, 3.0];
        let camera = H4ShadowCamera::new(H4ShadowCamera::DOWN, center, 0.5);
        let clip = |point: Vector| {
            let m = &camera.view_projection;
            std::array::from_fn::<f32, 3, _>(|column| {
                (0..3)
                    .map(|row| point[row] * m[row * 4 + column])
                    .sum::<f32>()
                    + m[12 + column]
            })
        };
        let top = clip([1.0, 2.0, 3.5]);
        let bottom = clip([1.0, 2.0, 2.5]);
        assert!((top[2] + 1.0).abs() < 1e-5, "{top:?}");
        assert!(bottom[2] > -1.0 && bottom[2] < 1.0, "{bottom:?}");
        let middle = clip(center);
        assert!(
            middle[0].abs() < 1e-5 && middle[1].abs() < 1e-5,
            "{middle:?}"
        );
        let side = clip([1.5, 2.0, 3.0]);
        assert!(
            (side[0].abs().max(side[1].abs()) - 1.0).abs() < 1e-5,
            "the sphere's edge at the map's: {side:?}"
        );
        assert_eq!(camera.pixels, 264);
    }

    #[test]
    fn the_shadow_direction_turns_into_the_normal_buffers_space() {
        let camera = H4ShadowCamera::new(H4ShadowCamera::DOWN, [0.0; 3], 1.0);
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        assert_eq!(camera.direction_in(&identity), [-0.0, -0.0, 1.0]);
        // A camera whose rows swap y and z.
        let swapped = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        assert_eq!(camera.direction_in(&swapped), [-0.0, 1.0, -0.0]);
    }

    /// The Chief casts as a biped; the assault rifle, a weapon left at
    /// `default`, casts on the high resolution map.
    #[test]
    fn the_kits_chief_and_rifle_cast() {
        use crate::app::export::geometry::load_referenced_tag_from_source;
        let Some(source) = kit_source(GameId::Halo4) else {
            return;
        };
        let load = |path: &str, extension: &str, group: &[u8; 4]| {
            load_referenced_tag_from_source(&source, path, extension, group).unwrap()
        };
        let chief = load(
            "objects\\characters\\storm_masterchief\\storm_masterchief",
            "biped",
            b"bipd",
        );
        assert_eq!(
            h4_shadow_caster(&chief),
            Some(H4ShadowCaster { hires: false })
        );
        let rifle = load(
            "objects\\weapons\\rifle\\storm_assault_rifle\\storm_assault_rifle",
            "weapon",
            b"weap",
        );
        assert_eq!(
            h4_shadow_caster(&rifle),
            Some(H4ShadowCaster { hires: true })
        );
    }
}
