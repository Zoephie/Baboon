//! Halo 4's light volumes on the GPU (`c_light_volume_gpu::render`, X360
//! 0x829E5DA0, MCC 0x1408ECFD0): a volume's packed profiles in a buffer
//! texture, one instanced four-vertex strip each, through its material's
//! `light_volume` pass with `SetMaterialState`'s overall state
//! (X360 0x82B96320) and a strip of rows in order.

use eframe::glow::{self, HasContext};
use super::super::super::gen3::effects::light_volumes::H4VolumeDraw;
use super::super::effects::gl::{H4EffectCx, material_functions};
use super::super::super::h4::materials::H4EffectMaterial;
use crate::core::hlsl::parse::Type;

/// `k_maxRowsPerStrip`.
const ROWS_PER_STRIP: usize = 8;

/// `vs_lightVolumeStrip.row`: a volume's profiles fill its own rows of the
/// buffer in order, so row `n` of its strip is buffer row `n`.
fn strip_rows() -> [f32; ROWS_PER_STRIP] {
    std::array::from_fn(|row| row as f32)
}

/// `vs_lightVolumeOverallState`, member by member, in `SetMaterialState`'s
/// order.
fn overall_members(draw: &H4VolumeDraw) -> Vec<(String, Vec<f32>)> {
    let scalar =
        |name: &str, value: f32| (format!("vs_lightVolumeOverallState.{name}"), vec![value]);
    let mut members = vec![
        scalar("appearanceFlags", draw.appearance),
        scalar("brightnessRatio", draw.brightness_ratio),
        scalar("offset", draw.offset),
        scalar("numProfiles", draw.profiles),
        scalar("profileDistance", draw.distance),
        scalar("profileLength", draw.side_length),
        (
            "vs_lightVolumeOverallState.origin".to_owned(),
            draw.origin.to_vec(),
        ),
        (
            "vs_lightVolumeOverallState.direction".to_owned(),
            draw.direction.to_vec(),
        ),
    ];
    members.extend(
        draw.inputs
            .iter()
            .enumerate()
            .map(|(index, value)| scalar(&format!("inputs[{index}].value"), *value)),
    );
    members
}

/// Draw one volume, through `material`, into the bound light target.
/// `depth` is a copy of the scene's depth for the fade.
pub(in crate::app::model_preview) unsafe fn draw_h4_light_volume(
    cx: &mut H4EffectCx<'_>,
    material: &H4EffectMaterial,
    draw: &H4VolumeDraw,
) {
    let H4EffectCx { gl, ref mut state, frame, depth, size, placeholder, object } = *cx;
    let pass = &*material.pass;
    // `profile_layout` holds a volume to a strip's 128.
    let count = draw.states.len();
    if count == 0 {
        return;
    }
    unsafe {
        if state.begin(gl, material).is_none() {
            return;
        }
        let buffer = "vs_light_volume_state_buffer";
        if state
            .bind_textures(gl, material, buffer, &draw.states, None, depth, placeholder)
            .is_none()
        {
            return;
        }
        let functions = material_functions(frame, material, object);
        for (name, ty) in &pass.uniforms {
            match name.as_str() {
                "vs_lightVolumeOverallState" => {
                    for (member, value) in overall_members(draw) {
                        state.set_floats(gl, pass, &member, &value);
                    }
                }
                "vs_lightVolumeStrip" => {
                    let location = state.location(gl, pass, "vs_lightVolumeStrip.row[0]");
                    gl.uniform_1_f32_slice(location.as_ref(), &strip_rows());
                }
                "light_volume_index_range" => {
                    let location = state.location(gl, pass, name);
                    gl.uniform_2_u32(location.as_ref(), 0, count as u32);
                }
                _ => {
                    if state.location(gl, pass, name).is_none() && !matches!(ty, Type::Named(_)) {
                        continue;
                    }
                    state.set_shared_uniform(gl, frame, material, &functions, name, ty, size);
                }
            }
        }
        gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, count as i32);
        state.end(gl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `k_profilesPerRow`.
    const PROFILES_PER_ROW: usize = 16;

    fn draw() -> H4VolumeDraw {
        H4VolumeDraw {
            appearance: 0.0,
            brightness_ratio: 0.5,
            offset: 0.25,
            profiles: 3.5,
            distance: 0.2,
            side_length: 1.5,
            origin: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, 1.0],
            inputs: std::array::from_fn(|index| index as f32),
            states: Vec::new(),
        }
    }

    /// The members go by the names the translated struct gives them, and
    /// each input to its own slot.
    #[test]
    fn the_overall_state_goes_member_by_member() {
        let members = overall_members(&draw());
        let value = |name: &str| {
            members
                .iter()
                .find(|(member, _)| member == &format!("vs_lightVolumeOverallState.{name}"))
                .map(|(_, value)| value.clone())
        };
        assert_eq!(value("numProfiles"), Some(vec![3.5]));
        assert_eq!(value("profileLength"), Some(vec![1.5]));
        assert_eq!(value("origin"), Some(vec![1.0, 2.0, 3.0]));
        assert_eq!(value("inputs[6].value"), Some(vec![6.0]));
        assert_ne!(value("inputs[6].value"), value("inputs[7].value"));
        assert_eq!(members.len(), 8 + 10);
    }

    /// Row `n` of the strip is buffer row `n`, so profile `p` reads state `p`
    /// (`ProfileIndexToBufferIndex`).
    #[test]
    fn the_strip_puts_each_profile_at_its_own_state() {
        let rows = strip_rows();
        let buffer_index = |profile: usize| {
            rows[profile / PROFILES_PER_ROW] as usize * PROFILES_PER_ROW
                + profile % PROFILES_PER_ROW
        };
        for profile in [0, 15, 16, 17, 127] {
            assert_eq!(buffer_index(profile), profile);
        }
        assert_ne!(rows[1], rows[0]);
    }
}
