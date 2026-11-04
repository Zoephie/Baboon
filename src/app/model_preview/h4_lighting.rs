//! Halo 4's frame around an object with no level (`docs/h4-preview-research/
//! lighting.md`): the default lighting `c_lighting_interface::
//! setup_default_lighting(true)` leaves in the constants (X360 0x82A72368,
//! MCC `sub_1408C3E60`), exposure, and the final composite.

use super::h3_shaders::H3Value;
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

/// `c_camera_fx_values::get_render_exposure` at the default settings' 0
/// stops: 2^0, scaled by 1.0 / 1.4938016 (MCC `xmmword_1424A1488` +0x2C).
pub(super) const H4_EXPOSURE: f32 = 1.0 / 1.493_801_6;

/// `c_camera_fx_values::g_HDR_target_stops` (X360 0x86E3C54C): the HDR
/// target holds light three stops down.
pub(super) const H4_DARK_MULTIPLIER: f32 = 8.0;

/// A Halo 4 engine constant's value for an object here, by name.
pub(super) fn h4_frame_value(name: &str) -> Option<H3Value> {
    let floats = |value: [f32; 4]| Some(H3Value::Floats(value));
    match name {
        "ps_lighting_constants" => Some(H3Value::FloatArray(H4_DEFAULT_VMF.to_vec())),
        "ps_model_sh_lighting" => Some(H3Value::FloatArray(default_sh())),
        // `setup_render_target_globals_with_exposure`: (e / 2^Y, 2^Y, e,
        // alpha), and the self-illumination scale (1 at defaults) times the
        // same. `.w` of the latter is the alpha the lerp keeps [inferred].
        "ps_view_exposure" | "vs_view_exposure" => {
            floats([H4_EXPOSURE / H4_DARK_MULTIPLIER, H4_DARK_MULTIPLIER, H4_EXPOSURE, 1.0])
        }
        "ps_view_self_illum_exposure" | "vs_view_self_illum_exposure" => {
            floats([1.0, H4_EXPOSURE / H4_DARK_MULTIPLIER, 0.0, 1.0])
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
    fn exposure_at_zero_stops_is_the_settings_scale() {
        assert!((H4_EXPOSURE - 0.669_433).abs() < 1e-6);
        let Some(H3Value::Floats(exposure)) = h4_frame_value("ps_view_exposure") else { panic!() };
        assert_eq!(exposure[1], 8.0);
        assert!((exposure[0] * exposure[1] - H4_EXPOSURE).abs() < 1e-6, "the dark target times its multiplier is the exposure");
    }
}

/// `final_composite` (`postprocess/final_composite_shared.fxh`), at the
/// default camera fx: the HDR target brought back up (`× (8, 8, 8, 32)`) and
/// the bloom added unscaled; hue, saturation and contrast are off; the filmic
/// curve is identity (the default tag predates it and its upgrade leaves it
/// disabled); then `sqrt` and `apply_color_grading` with one lookup, which is
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
out vec4 color;
void main() {
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    place = (place - 0.5) * view_scale + 0.5;
    vec4 accumulated = texture(accumulation, place);
    if (place.x < 0.0 || place.x > 1.0 || place.y < 0.0 || place.y > 1.0) {
        accumulated = vec4(0.0);
    }
    vec3 combined = max(accumulated.rgb, vec3(0.0)) * dark_multiplier + texture(bloom, place).rgb;
    vec3 encoded = sqrt(max(combined, vec3(0.0)));
    color.rgb = texture(grading, encoded * grading_scale + grading_offset).rgb;
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
