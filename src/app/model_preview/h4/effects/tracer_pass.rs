//! Halo 4's tracers on the GPU (`TracerGpu::Render`, X360 0x83B3E870, MCC
//! 0x1408E9A10): a tracer's packed profiles in a buffer texture, each strip
//! of its shape an instance of a `2N`-vertex triangle strip, through its
//! material's `tracer` pass with `SetMaterialState`'s overall state (X360
//! 0x829E14B0) and a strip of rows in order. See
//! `docs/h4-preview-research/tracers.md` §4.

use eframe::glow::{self, HasContext};
use super::super::effects::gl::{H4EffectCx, material_functions};
use super::super::tracers::H4TracerDraw;
use super::super::super::h4::materials::H4EffectMaterial;
use crate::core::hlsl::parse::Type;

/// `k_maxRowsPerInstance`.
const ROWS_PER_STRIP: usize = 8;

/// `vs_tracerStrip.row`: the profiles fill the buffer in strip order, so
/// row `n` of the strip is buffer row `n`.
fn strip_rows() -> [f32; ROWS_PER_STRIP] {
    std::array::from_fn(|row| row as f32)
}

/// `vs_tracerOverallState`, member by member, in `SetMaterialState`'s order.
fn overall_members(draw: &H4TracerDraw) -> Vec<(String, Vec<f32>)> {
    let member =
        |name: &str, value: &[f32]| (format!("vs_tracerOverallState.{name}"), value.to_vec());
    let mut members = vec![
        member("profileShape", &[draw.shape]),
        member("ngonSides", &[draw.ngon_sides]),
        member("appearanceFlags", &[draw.appearance]),
        member("numProfiles", &[draw.states.len() as f32]),
        member("percentileStep", &[draw.percentile_step]),
        member("cappedLength", &[draw.capped_length]),
        member("offset", &[draw.offset]),
        member(
            "localSpaceOffset_gameTime",
            &[0.0, 0.0, 0.0, draw.game_time],
        ),
        member("uvTilingRate", &draw.uv_tiling),
        member("uvScrollRate", &draw.uv_scroll),
        member("uvOffset", &draw.uv_offset),
        member("origin", &draw.origin),
        member("direction", &draw.direction),
        member(
            "fade._originRange_originCutoff_edgeRange_edgeCutoff",
            &draw.fade,
        ),
    ];
    // `inputs[18]`, the invalid state, is past the 32 registers.
    members.extend(
        draw.inputs
            .iter()
            .take(18)
            .enumerate()
            .map(|(index, value)| member(&format!("inputs[{index}].value"), &[*value])),
    );
    members
}

/// Draw one tracer, through `material`, into the bound light target.
pub(in crate::app::model_preview) unsafe fn draw_h4_tracer(
    cx: &mut H4EffectCx<'_>,
    material: &H4EffectMaterial,
    draw: &H4TracerDraw,
) {
    let H4EffectCx { gl, ref mut state, frame, depth, size, placeholder, object } = *cx;
    let pass = &*material.pass;
    // `Render` skips one profile or fewer.
    let count = draw.states.len();
    if count <= 1 || draw.strips == 0 {
        return;
    }
    unsafe {
        if state.begin(gl, material).is_none() {
            return;
        }
        let buffer = "vs_tracer_profile_state_buffer";
        if state
            .bind_textures(gl, material, buffer, &draw.states, None, depth, placeholder)
            .is_none()
        {
            return;
        }
        let functions = material_functions(frame, material, object);
        for (name, ty) in &pass.uniforms {
            match name.as_str() {
                "vs_tracerOverallState" => {
                    for (member, value) in overall_members(draw) {
                        state.set_floats(gl, pass, &member, &value);
                    }
                }
                "vs_tracerStrip" => {
                    let location = state.location(gl, pass, "vs_tracerStrip.row[0]");
                    gl.uniform_1_f32_slice(location.as_ref(), &strip_rows());
                }
                _ => {
                    if state.location(gl, pass, name).is_none() && !matches!(ty, Type::Named(_)) {
                        continue;
                    }
                    state.set_shared_uniform(gl, frame, material, &functions, name, ty, size);
                }
            }
        }
        gl.draw_arrays_instanced(
            glow::TRIANGLE_STRIP,
            0,
            2 * count as i32,
            draw.strips as i32,
        );
        state.end(gl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw() -> H4TracerDraw {
        H4TracerDraw {
            shape: 1.0,
            ngon_sides: 0.0,
            appearance: 64.0,
            percentile_step: 0.25,
            capped_length: 4.0,
            offset: 0.0,
            game_time: 2.5,
            uv_tiling: [1.0, 1.0],
            uv_scroll: [-7.0, 0.0],
            uv_offset: [0.3, 0.0],
            origin: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, 1.0],
            fade: [0.2, 3.0, 0.0, 0.0],
            inputs: std::array::from_fn(|index| index as f32),
            strips: 2,
            states: vec![[0; 16]; 5],
        }
    }

    /// The members go by the names the translated struct gives them; the
    /// game time rides in `.w` of the local-space offset, and the invalid
    /// state is not sent.
    #[test]
    fn the_overall_state_goes_member_by_member() {
        let members = overall_members(&draw());
        let value = |name: &str| {
            members
                .iter()
                .find(|(member, _)| member == &format!("vs_tracerOverallState.{name}"))
                .map(|(_, value)| value.clone())
        };
        assert_eq!(value("numProfiles"), Some(vec![5.0]));
        assert_eq!(
            value("localSpaceOffset_gameTime"),
            Some(vec![0.0, 0.0, 0.0, 2.5])
        );
        assert_eq!(value("uvScrollRate"), Some(vec![-7.0, 0.0]));
        assert_eq!(value("inputs[17].value"), Some(vec![17.0]));
        assert_eq!(value("inputs[18].value"), None);
        assert_ne!(value("inputs[16].value"), value("inputs[17].value"));
    }

    #[test]
    fn the_strip_puts_each_profile_at_its_own_state() {
        let rows = strip_rows();
        for profile in [0usize, 15, 16, 127] {
            assert_eq!(rows[profile / 16] as usize * 16 + profile % 16, profile);
        }
    }
}
