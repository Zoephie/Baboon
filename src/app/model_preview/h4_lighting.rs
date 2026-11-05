//! Halo 4's frame around an object with no level (`docs/h4-preview-research/
//! lighting.md`): the default lighting `c_lighting_interface::
//! setup_default_lighting(true)` leaves in the constants (X360 0x82A72368,
//! MCC `sub_1408C3E60`), exposure, and the final composite.

use super::gen3::shaders::H3Value;
use super::*;
use crate::app::export::geometry::load_referenced_tag_from_source;

/// `s_dual_vmf_basis::SetDefaultLighting` (X360 0x82A3CF50, MCC
/// `sub_1408E6B70`, the same immediates): two linear-SH lobes, the direct one
/// straight down from above and a dimmer one from below. `[1].w` is the
/// analytic scalar, `[3].w` the occlusion.
pub(super) const H4_DEFAULT_VMF: [[f32; 4]; 4] = [
    [0.0, 0.0, 1.0, 0.1],
    [3.0, 3.0, 3.0, 0.0],
    [0.0, 0.0, -0.8, 0.2],
    [1.0, 0.9, 0.9, 1.0],
];

/// `real_rgb_lightprobe::clear` then `SetLightmapSHConstants(probe, NULL, 0)`
/// (X360 0x82A7EEB0): a cleared quadratic probe, and `[7].w` = 1 because its
/// confidence is not.
fn default_sh() -> Vec<[f32; 4]> {
    let mut sh = vec![[0.0; 4]; 8];
    sh[7] = [0.0, 0.0, -1.0, 1.0];
    sh
}

/// MCC's scale on the render exposure (`xmmword_1424A1488` +0x2C):
/// `get_render_exposure` (X360 0x82B97C98) divides `2^stops` by it.
const EXPOSURE_SCALE: f32 = 1.493_801_6;

/// The camera fx the preview frames Halo 4 objects with, its own choice (user
/// decision, 2026-11-04). `globals\defaults\default.camera_fx_settings` is
/// like no level (bloom 0.3, no self-illumination bloom or exposure, no tone
/// curve), so the preview takes one real level's instead:
/// `environments\solo\m80_delta\m80_cam_fx_global`, the set Cortana's
/// mission and her m81, m82 and m91 cinematics share (13 kit tags), without
/// its colour grading so nothing is tinted. Its exposure auto-adjusts, and
/// against a dark picture climbs to its `maximum`, +1, which is also the
/// maximum of 70 of the kit's 110 auto-exposed camera fx.
pub(super) const H4_PREVIEW_STOPS: f32 = 1.0;

/// The preview camera fx's `self_illum_preferred` (+1.5) and
/// `self_illum_scale` "exposure change" (0): self-illumination, and the
/// hologram composite, hold +1.5 stops whatever the scene's exposure.
const SELF_ILLUM_PREFERRED: f32 = 1.5;
const SELF_ILLUM_CHANGE: f32 = 0.0;

/// The preview camera fx's `filmic_tone_curve` (enabled): shoulder strength,
/// linear strength, linear angle, toe strength, toe numerator, toe
/// denominator, linear white point. Every one of the kit's 125 enabled
/// curves is this one.
pub(super) const H4_FILMIC_CURVE: [f32; 7] = [0.2, 0.3, 0.2, 0.2, 0.01, 0.4, 4.0];

/// `SetFilmicToneCurveParameters` (X360 0x83AFD0A0): the five constants
/// `ApplyFilmicToneCurve` and `InverseFilmicToneCurve` read, each a splat.
/// A field within 1e-4 of zero makes the curve identity, whose constants
/// differ for the inverse.
pub(super) fn h4_filmic_params(curve: [f32; 7], inverse: bool) -> [f32; 5] {
    let [a, b, c, d, e, f, w] = curve;
    if curve.iter().any(|field| field.abs() < 1e-4) {
        return if inverse { [1.0, 0.0, 0.0, 1.0, 0.0] } else { [0.0, 1.0, 0.0, 0.0, 1.0] };
    }
    let white = 1.0 / ((w * (a * w + c * b) + d * e) / (w * (a * w + b) + d * f) - e / f);
    [white * (a * f - a * e), white * (b * c * f - b * e), a * f, b * f, d * f * f]
}

/// `ApplyFilmicToneCurve`: `x(x·p0 + p1) / (x(x·p2 + p3) + p4)`.
#[cfg(test)]
fn apply_filmic(params: [f32; 5], x: f32) -> f32 {
    x * (x * params[0] + params[1]) / (x * (x * params[2] + params[3]) + params[4])
}

/// `c_camera_fx_values::get_render_exposure`: `2^stops / 1.4938016`.
pub(super) fn h4_exposure(stops: f32) -> f32 {
    stops.exp2() / EXPOSURE_SCALE
}

/// `get_illum_render_scale` (X360 0x829A1D50): `2^(v87 - stops)`, with
/// `update` (0x83A963A0) leaving `v87 = (stops - preferred) · change +
/// preferred`. Self-illumination, and the hologram composite, follow the
/// scene's exposure only by `change`.
pub(super) fn h4_illum_scale(stops: f32) -> f32 {
    let illum_stops = (stops - SELF_ILLUM_PREFERRED) * SELF_ILLUM_CHANGE + SELF_ILLUM_PREFERRED;
    (illum_stops - stops).exp2()
}

/// `c_camera_fx_values::g_HDR_target_stops` (X360 0x86E3C54C): the HDR
/// target holds light three stops down.
pub(super) const H4_DARK_MULTIPLIER: f32 = 8.0;

/// Light Follows Camera, the preview's own: the default lobes turned as one,
/// by the shortest rotation that takes straight up to halfway between up and
/// `to_camera` (unit, from the object toward the eye). The bright lobe then
/// comes from above the viewer's shoulder and the dim one from opposite it;
/// their colours and the energy stay the game's. Halo 4's lobes are both
/// vertical, so the turn about the vertical that Halo 3 and Reach use would
/// change nothing.
pub(super) fn h4_turned_vmf(to_camera: [f32; 3]) -> [[f32; 4]; 4] {
    let up = [0.0, 0.0, 1.0];
    let half = [to_camera[0], to_camera[1], to_camera[2] + 1.0];
    let length = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
    // Looking straight up from below leaves nothing to lean toward.
    if length < 1e-4 {
        return H4_DEFAULT_VMF;
    }
    let target = half.map(|axis| axis / length);
    // Rodrigues' rotation from `up` to `target`.
    let cos = target[2];
    let axis = [up[1] * target[2] - up[2] * target[1], up[2] * target[0] - up[0] * target[2], 0.0];
    let turn = |v: [f32; 4]| -> [f32; 4] {
        let k = axis;
        let cross = [k[1] * v[2] - k[2] * v[1], k[2] * v[0] - k[0] * v[2], k[0] * v[1] - k[1] * v[0]];
        let dot = k[0] * v[0] + k[1] * v[1] + k[2] * v[2];
        let factor = if cos > -1.0 + 1e-6 { 1.0 / (1.0 + cos) } else { 0.0 };
        let rotated: [f32; 3] = std::array::from_fn(|i| v[i] * cos + cross[i] + k[i] * dot * factor);
        [rotated[0], rotated[1], rotated[2], v[3]]
    };
    let mut lobes = H4_DEFAULT_VMF;
    lobes[0] = turn(lobes[0]);
    lobes[2] = turn(lobes[2]);
    lobes
}

/// A Halo 4 engine constant's value for an object here, by name.
pub(super) fn h4_frame_value(name: &str) -> Option<H3Value> {
    let floats = |value: [f32; 4]| Some(H3Value::Floats(value));
    match name {
        "ps_lighting_constants" => Some(H3Value::FloatArray(H4_DEFAULT_VMF.to_vec())),
        "ps_model_sh_lighting" => Some(H3Value::FloatArray(default_sh())),
        // `setup_render_target_globals_with_exposure`: (e / 2^Y, 2^Y, e,
        // alpha), and the self-illumination scale (1 at defaults) times the
        // same. `.w` of the latter is the alpha the lerp keeps [inferred].
        // `restore_render_target_globals_with_exposure` (X360 0x82A23838):
        // the self-illumination one is `(illum, illum · e / 2^Y)`.
        "ps_view_exposure" | "vs_view_exposure" => {
            let exposure = h4_exposure(H4_PREVIEW_STOPS);
            floats([exposure / H4_DARK_MULTIPLIER, H4_DARK_MULTIPLIER, exposure, 1.0])
        }
        "ps_view_self_illum_exposure" | "vs_view_self_illum_exposure" => {
            let illum = h4_illum_scale(H4_PREVIEW_STOPS);
            floats([illum, illum * h4_exposure(H4_PREVIEW_STOPS) / H4_DARK_MULTIPLIER, 0.0, 1.0])
        }
        // Uncompressed: the preview's vertices are already in model units.
        "vs_mesh_position_compression_scale" => floats([1.0, 1.0, 1.0, 0.0]),
        "vs_mesh_position_compression_offset" => floats([0.0; 4]),
        "vs_mesh_uv_compression_scale_offset" => floats([1.0, 1.0, 0.0, 0.0]),
        // No level: no sun, no analytic light, no floating shadow, no
        // lightmap (written only by `setup_object_lighting_for_pass`, which
        // returns first). Their constants stay zero [inferred for MCC: O3].
        "ps_boolean_using_floating_sun"
        | "ps_boolean_using_analytic_light"
        | "ps_boolean_using_analytic_light_gobo"
        | "ps_boolean_using_static_lightmap_only" => Some(H3Value::Integer(0)),
        _ => None,
    }
}

/// `globals\defaults\default`'s colour grading texture: the camera fx
/// settings the game falls back on with no cluster to read others from
/// (`c_player_view::setup_camera_fx_parameters`, X360 0x82B704C8).
pub(super) fn read_h4_color_grading(source: &TagSource) -> Option<super::materials::H3Bitmap> {
    let settings = load_referenced_tag_from_source(source, "globals\\defaults\\default", "camera_fx_settings", b"cfxs").ok()?;
    let grading = settings.root().descend("color_grading")?;
    let path = grading.read_tag_ref_path("color grading texture").filter(|path| !path.is_empty())?;
    super::materials::read_h3_bitmap(source, &path, 0, crate::core::h2_pixel_shader::SamplerKind::Volume)
}

/// The light a white surface takes from the default lobes: `VMFDiffuse` for
/// a probe with a white shadow mask, before exposure.
#[cfg(test)]
fn default_irradiance(normal: [f32; 3]) -> [f32; 3] {
    let sh7 = default_sh()[7][3];
    let linear = |lobe: [f32; 4], color: [f32; 4]| {
        let dot = 0.325_735 * (lobe[0] * normal[0] + lobe[1] * normal[1] + lobe[2] * normal[2]) + 0.282_094_8 * lobe[3];
        let scale = (dot * sh7).clamp(0.0, 1.0);
        [color[0] * scale, color[1] * scale, color[2] * scale]
    };
    let [direct, indirect] = [linear(H4_DEFAULT_VMF[0], H4_DEFAULT_VMF[1]), linear(H4_DEFAULT_VMF[2], H4_DEFAULT_VMF[3])];
    std::array::from_fn(|channel| direct[channel] + indirect[channel])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
    }

    /// `lighting.md`'s figures: lit from straight above, a little from below.
    #[test]
    fn the_default_lobes_light_from_above() {
        assert!(close(default_irradiance([0.0, 0.0, 1.0]), [1.0618, 1.0618, 1.0618]), "{:?}", default_irradiance([0.0, 0.0, 1.0]));
        assert!(close(default_irradiance([0.0, 0.0, -1.0]), [0.3170, 0.2853, 0.2853]), "{:?}", default_irradiance([0.0, 0.0, -1.0]));
        assert!(close(default_irradiance([1.0, 0.0, 0.0]), [0.1410, 0.1354, 0.1354]), "{:?}", default_irradiance([1.0, 0.0, 0.0]));
        // Not Reach's: its default probe lights the side far more.
        assert!(default_irradiance([1.0, 0.0, 0.0])[0] < 0.2);
    }

    #[test]
    fn following_the_camera_leans_the_light_toward_it_and_keeps_its_strength() {
        let length = |v: [f32; 4]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        // A camera off to +x, level with the object.
        let turned = h4_turned_vmf([1.0, 0.0, 0.0]);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!((turned[0][0] - half).abs() < 1e-5 && (turned[0][2] - half).abs() < 1e-5, "{:?}", turned[0]);
        assert!((length(turned[0]) - 1.0).abs() < 1e-5);
        assert!((length(turned[2]) - 0.8).abs() < 1e-5, "the dim lobe keeps its size");
        assert!(turned[2][0] < 0.0 && turned[2][2] < 0.0, "and turns opposite: {:?}", turned[2]);
        assert_eq!(turned[1], H4_DEFAULT_VMF[1]);
        assert_eq!(turned[3], H4_DEFAULT_VMF[3]);
        // From straight above there is nothing to turn.
        let above = h4_turned_vmf([0.0, 0.0, 1.0]);
        assert!(above.iter().zip(H4_DEFAULT_VMF).all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6)));
        // Whatever faces the camera now takes more light than it did.
        let facing = |lobes: &[[f32; 4]; 4]| 0.325_735 * lobes[0][0] + 0.282_094_8 * lobes[0][3];
        assert!(facing(&turned) > facing(&H4_DEFAULT_VMF));
    }

    #[test]
    fn exposure_at_zero_stops_is_the_settings_scale() {
        assert!((h4_exposure(0.0) - 0.669_433).abs() < 1e-6);
        assert!((h4_exposure(1.0) - 2.0 * 0.669_433).abs() < 1e-5);
        let Some(H3Value::Floats(exposure)) = h4_frame_value("ps_view_exposure") else { panic!() };
        assert_eq!(exposure[1], 8.0);
        let expected = h4_exposure(H4_PREVIEW_STOPS);
        assert!((exposure[0] * exposure[1] - expected).abs() < 1e-6, "the dark target times its multiplier is the exposure");
    }

    /// With no exposure change, self-illumination holds its preferred stops
    /// whatever the scene's: `illum · e` is `2^1.5 / 1.4938` at any stops.
    #[test]
    fn self_illumination_holds_its_preferred_exposure() {
        let held = 1.5f32.exp2() * h4_exposure(0.0);
        for stops in [-2.0, 0.0, 1.0, 2.0] {
            let self_illum = h4_illum_scale(stops) * h4_exposure(stops);
            assert!((self_illum - held).abs() < 1e-5, "{stops}: {self_illum}");
        }
        assert!(h4_illum_scale(2.0) < h4_illum_scale(0.0), "it takes back what the scene's exposure adds");
    }

    /// The curve maps its white point to 1 and darkens the toe; with a field
    /// at zero it is identity, both ways.
    #[test]
    fn the_filmic_curve_reaches_one_at_its_white_point() {
        let params = h4_filmic_params(H4_FILMIC_CURVE, false);
        assert!((apply_filmic(params, 4.0) - 1.0).abs() < 1e-5);
        assert!(apply_filmic(params, 0.1) < 0.1, "the toe is darker");
        let identity = h4_filmic_params([0.0, 0.3, 0.2, 0.2, 0.01, 0.4, 4.0], false);
        assert_eq!(apply_filmic(identity, 0.37), 0.37);
        assert_eq!(h4_filmic_params([0.0; 7], true), [1.0, 0.0, 0.0, 1.0, 0.0]);
    }

    /// With `BLAM_TEST_H4EK`: the preview's camera fx is
    /// `m80_cam_fx_global`'s, as its doc says.
    #[test]
    fn the_preview_camera_fx_is_m80s() {
        let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
            return;
        };
        let path = std::path::Path::new(&root).join("environments/solo/m80_delta/m80_cam_fx_global.camera_fx_settings");
        let tag = blam_tags::TagFile::read(path).unwrap();
        let root = tag.root();
        let real = |name: &str| root.descend(name.rsplit_once('/').unwrap().0).and_then(|holder| holder.read_real(name.rsplit_once('/').unwrap().1)).unwrap();
        assert_eq!(real("exposure/maximum"), H4_PREVIEW_STOPS);
        assert_eq!(real("self_illum_preferred/preferred exposure"), SELF_ILLUM_PREFERRED);
        assert_eq!(real("self_illum_scale/exposure change"), SELF_ILLUM_CHANGE);
        let curve = ["shoulder strength", "linear strength", "linear angle", "toe strength", "toe numerator", "toe denominator", "linear white point"];
        assert_eq!(curve.map(|field| real(&format!("filmic_tone_curve/{field}"))), H4_FILMIC_CURVE);
    }
}

/// `final_composite` (`postprocess/final_composite_shared.fxh`), at the
/// preview's camera fx: the HDR target brought back up (`× (8, 8, 8, 32)`)
/// and the bloom added unscaled; hue, saturation and contrast are off (not
/// on PC); the filmic curve ([`h4_filmic_params`]); then `sqrt` and
/// `apply_color_grading` with one lookup, which is
/// what reaches the screen. The lookup is uploaded as its bytes stand, so
/// that its output is the displayed value [inferred: the back buffer's
/// format, O7, decides whether the game's view decodes it, and either way
/// the default lookup's output displays as sRGB(x²)]. Coverage is Reach's:
/// what the object drew covers what lies behind it.
pub(super) const H4_COMPOSITE_FRAGMENT: &str = "#version 410 core
uniform sampler2D accumulation;
uniform sampler2D bloom;
uniform sampler2D scene_depth;
uniform sampler3D grading;
uniform vec3 grading_scale;
uniform vec3 grading_offset;
uniform float dark_multiplier;
uniform float view_scale;
uniform float filmic[5];
out vec4 color;
void main() {
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    place = (place - 0.5) * view_scale + 0.5;
    vec4 accumulated = texture(accumulation, place);
    if (place.x < 0.0 || place.x > 1.0 || place.y < 0.0 || place.y > 1.0) {
        accumulated = vec4(0.0);
    }
    vec3 combined = max(accumulated.rgb, vec3(0.0)) * dark_multiplier + texture(bloom, place).rgb;
    combined = combined * (combined * filmic[0] + filmic[1]) / (combined * (combined * filmic[2] + filmic[3]) + filmic[4]);
    vec3 encoded = sqrt(max(combined, vec3(0.0)));
    // `apply_color_grading` reads level 0 (`sample3DLOD`). An implicit level
    // follows the lookup coordinate's change across each 2×2 block, which
    // jumps at a silhouette, and drew a grey rim from the lookup's small mips.
    color.rgb = textureLod(grading, encoded * grading_scale + grading_offset, 0.0).rgb;
    bool drawn = texture(scene_depth, place).r < 1.0;
    color.a = drawn ? 1.0 : accumulated.a;
}
";

/// `SetColorGradingParameters` (X360 0x829B78E0): a lookup of `size` texels
/// a side is read at `x (size - 1) / size + 0.5 / size`, its texel centres.
pub(super) fn h4_grading_scale_offset(size: [usize; 3]) -> ([f32; 3], [f32; 3]) {
    let scale = size.map(|side| (side.max(1) as f32 - 1.0) / side.max(1) as f32);
    let offset = size.map(|side| 0.5 / side.max(1) as f32);
    (scale, offset)
}
