//! Halo 4's particles on the GPU (`c_particle_emitter_gpu::render`, X360
//! 0x82A88A30, MCC 0x1408E5560): each emitter's packed particles in a buffer
//! texture, one instanced four-vertex strip each, through the particle's
//! material with `SetMaterialRenderState`'s constants. See
//! `docs/h4-preview-research/particles.md`.

use super::h3_particles::H4EmitterDraw;
use super::h3_shaders::{H3Pass, H3Value};
use super::h4_effect_gl::{H4EffectGl, material_functions};
use super::h4_particles::{H4ParticleRender, custom_vert_registers};
use super::*;
use crate::core::hlsl::parse::Type;

/// `effect_lightprobe_setup_lighting_constants` with the default probe
/// (`particles.md` §4): bright direction, bright and dark colours, ambient.
const DEFAULT_LIGHTING: [[f32; 3]; 4] = [
    [0.0, 0.0, 1.0],
    [0.413_803, 0.381_972, 0.381_972],
    [1.177_747, 1.145_916, 1.145_916],
    [0.5, 0.48, 0.48],
];

/// The system postprocess's far fade when the tag has none:
/// range at its floor, the cutoff 10000.
const NO_FAR_FADE: [f32; 2] = [1e-4, 10_000.0];

/// Draw `draws` of one particle system, whose look is `render`, into the
/// bound light target. `depth` is a copy of the scene's depth for the fade.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_h4_particles(
    gl: &glow::Context,
    state: &mut H4EffectGl,
    frame: &ModelGpuFrame,
    render: &H4ParticleRender,
    draws: &[H4EmitterDraw],
    depth: Option<glow::NativeTexture>,
    size: [i32; 2],
    placeholder: Option<glow::NativeTexture>,
    object: u8,
) {
    let material = &render.material;
    let pass = &*material.pass;
    unsafe {
        if state.begin(gl, material).is_none() {
            return;
        }
        let floats = |value: [f32; 4]| Some(H3Value::Floats(value));
        let bools = render.pixel_bools();
        let lit = |index: usize| {
            let [r, g, b] = DEFAULT_LIGHTING[index];
            [r, g, b, 0.0]
        };
        let functions = material_functions(frame, material, object);
        for draw in draws {
            let buffer = "vs_particle_state_buffer";
            if state
                .bind_textures(gl, material, buffer, &draw.states, depth, placeholder)
                .is_none()
            {
                continue;
            }
            for (name, ty) in &pass.uniforms {
                let location = state.location(gl, pass, name);
                if location.is_none() && !matches!(ty, Type::Named(_)) {
                    continue;
                }
                match name.as_str() {
                    "vsRenderState" => {
                        set_render_state(gl, state, pass, render, draw, frame.time);
                        continue;
                    }
                    "particle_index_range" => {
                        gl.uniform_2_u32(location.as_ref(), 0, 0);
                        continue;
                    }
                    "vs_emitter_to_world_matrix" => {
                        let [forward, left, up, position] = draw.to_world;
                        let rows: [f32; 12] =
                            std::array::from_fn(|at| [forward, left, up, position][at % 4][at / 4]);
                        gl.uniform_matrix_3x4_f32_slice(location.as_ref(), false, &rows);
                        continue;
                    }
                    "vs_sprite_list_sprites" => {
                        let mut sprites = [[0.0f32; 4]; 15];
                        for (at, sprite) in render.sprites.iter().take(15).enumerate() {
                            sprites[at] = *sprite;
                        }
                        gl.uniform_4_f32_slice(location.as_ref(), sprites.as_flattened());
                        continue;
                    }
                    _ => {}
                }
                let value = match name.as_str() {
                    "vs_sprite_offset_and_scale" => floats(render.sprite),
                    "vs_cameraVelocity" => floats([0.0; 4]),
                    "vs_sprite_list_count_and_max" => floats([render.sprite_count, 0.0, 0.0, 0.0]),
                    "vsLightingBrightDirection" | "psLightingBrightDirection" => floats(lit(0)),
                    "vsLightingBrightColor" | "psLightingBrightColor" => floats(lit(1)),
                    "vsLightingDarkColor" | "psLightingDarkColor" => floats(lit(2)),
                    "vsLightingAmbient" | "psLightingAmbient" => floats(lit(3)),
                    "psDepthFadeEnabled" => {
                        Some(H3Value::Integer(i32::from(bools[0] && depth.is_some())))
                    }
                    "psSphereWarpEnabled" => Some(H3Value::Integer(i32::from(bools[1]))),
                    "psNewSchoolFrameIndex" => Some(H3Value::Integer(i32::from(bools[2]))),
                    "psBlackOrWhitePointEnabled" => {
                        Some(H3Value::Integer(i32::from(draw.black_or_white)))
                    }
                    "psTintFactor" => floats([render.tint_factor, 0.0, 0.0, 0.0]),
                    _ => None,
                };
                match value {
                    Some(value) => h3_draw::set_uniform_value(gl, location.as_ref(), ty, &value),
                    None => {
                        state.set_shared_uniform(gl, frame, material, &functions, name, ty, size);
                    }
                }
            }
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, draw.states.len() as i32);
        }
        state.end(gl);
    }
}

/// `SetMaterialRenderState` (X360 0x83AD5618): `vsRenderState`, member by
/// member.
unsafe fn set_render_state(
    gl: &glow::Context,
    state: &mut H4EffectGl,
    pass: &H3Pass,
    render: &H4ParticleRender,
    draw: &H4EmitterDraw,
    time: f32,
) {
    let [main, appearance, animation] = render.flags.map(|flags| flags as f32);
    let [near_range, near_cutoff] = draw.near;
    let [edge_range, edge_cutoff] = render.edge_fade;
    let members: Vec<(String, Vec<f32>)> = [
        (
            "flags_main_appearance_animation".to_owned(),
            vec![main, appearance, animation],
        ),
        // Third person, at the unzoomed field of view.
        ("firstPerson_fadeDepthMultiplier".to_owned(), vec![0.0, 1.0]),
        // `camera offset` is not in the kit's system layout.
        (
            "curvature_cameraOffset".to_owned(),
            vec![render.curvature, 0.0],
        ),
        ("uvScrollRate".to_owned(), vec![0.0, 0.0]),
        // Render seconds, coming round at four hours.
        ("gameTime".to_owned(), vec![time.rem_euclid(14_400.0)]),
        ("billboardType".to_owned(), vec![render.billboard as f32]),
        ("vertexCount".to_owned(), vec![4.0]),
        (
            "fade.nearRange_nearCutoff_edgeRange_edgeCutoff".to_owned(),
            vec![near_range.max(1e-4), near_cutoff, edge_range, edge_cutoff],
        ),
        ("fade.farRange_farCutoff".to_owned(), NO_FAR_FADE.to_vec()),
    ]
    .into_iter()
    .chain(
        custom_vert_registers(&render.custom_verts)
            .iter()
            .enumerate()
            .map(|(register, value)| {
                (
                    format!("customVerts[{}].customVertex{}", register / 2, register % 2),
                    value.to_vec(),
                )
            }),
    )
    .collect();
    unsafe {
        for (member, value) in members {
            let location = state.location(gl, pass, &format!("vsRenderState.{member}"));
            match value.len() {
                1 => gl.uniform_1_f32(location.as_ref(), value[0]),
                2 => gl.uniform_2_f32_slice(location.as_ref(), &value),
                3 => gl.uniform_3_f32_slice(location.as_ref(), &value),
                _ => gl.uniform_4_f32_slice(location.as_ref(), &value),
            }
        }
    }
}
