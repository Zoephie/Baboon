//! The main model program: its GLSL for each dialect, the vertex layout and the linker that binds its attributes.

use super::*;
use super::super::gl::glsl::fill;
use super::super::gl::link::link;
use eframe::glow;

/// GPU skinning bone budget: three vec4 uniform rows per bone. Halo node
/// indices are bytes, so 256 covers every skeleton any graph can address
/// (Reach's halsey alone has 163 nodes; the old budget of 96 silently
/// refused her animations). 768 vec4 rows sit well inside desktop uniform
/// limits — the practical floor is 1024 vec4.
pub(in crate::app) const MAX_PREVIEW_BONES: usize = 256;

/// Sampler uniform per slot, in `TextureSlot` order — the fragment shader binds
/// texture unit `i` to `SAMPLER_UNIFORMS[i]`.
pub(super) const SAMPLER_UNIFORMS: [&str; SLOT_COUNT] = [
    "u_tex_base",
    "u_tex_detail",
    "u_tex_bump",
    "u_tex_bump_detail",
    "u_tex_alpha",
    "u_tex_multipurpose",
];

/// The vertex declarations and helpers every model stage shares: all of the
/// modern source before its `main`.
pub(in crate::app::model_preview) fn model_vertex_prelude(version_declaration: &str) -> String {
    let (vertex, _) = model_shader_sources(version_declaration, true, "");
    vertex.split("void main()").next().unwrap_or_default().to_owned()
}

/// Build the two GLSL sources.
///
/// Split out of `RenderCore::new` so the shaders can be checked
/// without a GL context: a compile failure here disables the whole preview
/// and the only signal is a line on stderr, which no test would see.
pub(super) fn model_shader_sources(
    version_declaration: &str,
    modern: bool,
    precision: &str,
) -> (String, String) {
    let (attribute, varying_out, varying_in, fragment_output, output_name, sample) = if modern {
        (
            "in",
            "out",
            "in",
            "out vec4 out_color;",
            "out_color",
            "texture",
        )
    } else {
        (
            "attribute",
            "varying",
            "varying",
            "",
            "gl_FragColor",
            "texture2D",
        )
    };
    // Position and normal are required; the rest are read only on the
    // shaded path and a driver may drop them from an unused program.
    let bone_rows = MAX_PREVIEW_BONES * 3;
    let vertex_source = fill(
        include_str!("shaders/model.vert"),
        &[
            ("version", &version_declaration),
            ("precision", &precision),
            ("attribute", &attribute),
            ("bone_rows", &bone_rows),
            ("varying_out", &varying_out),
        ],
    );
    // The lighting rig is the one this preview has always used — a key, a
    // fill, a rim and an overhead term, all in view space so they follow the
    // camera. It moved from the vertex shader to here so a normal map has
    // something to perturb: per-vertex lighting would sample the map and
    // then throw the result away between vertices.
    let fragment_source = fill(
        include_str!("shaders/model.frag"),
        &[
            ("version", &version_declaration),
            ("precision", &precision),
            ("varying_in", &varying_in),
            ("fragment_output", &fragment_output),
            ("sample", &sample),
            ("output_name", &output_name),
        ],
    );

    (vertex_source, fragment_source)
}

/// Wire the full `RenderModelPreviewVertex` layout into the currently bound
/// vertex array. Shared by the model and grid buffers so no attribute is
/// ever left disabled and reading a stale constant (a disabled array reads
/// `(0, 0, 0, 1)` — a phantom skinning weight on bone 1).
pub(super) unsafe fn bind_preview_vertex_layout(
    gl: &glow::Context,
    program: glow::NativeProgram,
) -> Result<(), String> {
    let stride = std::mem::size_of::<RenderModelPreviewVertex>() as i32;
    unsafe {
        let position = gl
            .get_attrib_location(program, "a_position")
            .ok_or_else(|| "model preview shader has no position attribute".to_owned())?;
        let normal = gl
            .get_attrib_location(program, "a_normal")
            .ok_or_else(|| "model preview shader has no normal attribute".to_owned())?;
        gl.enable_vertex_attrib_array(position);
        gl.vertex_attrib_pointer_f32(position, 3, glow::FLOAT, false, stride, 0);
        gl.enable_vertex_attrib_array(normal);
        gl.vertex_attrib_pointer_f32(normal, 3, glow::FLOAT, false, stride, 12);
        // Optional, unlike the two above: the untextured paths (particle
        // models, Chimp's UE meshes) leave these at zero, and a driver is
        // free to optimise an unread attribute out of the program entirely.
        // A missing location is therefore not an error — offsets are pinned
        // by `gpu_vertex_layout_matches_the_hand_written_attribute_offsets`.
        for (name, size, offset) in [
            ("a_texcoord", 2, 24),
            ("a_tangent", 3, 32),
            ("a_binormal", 3, 44),
            // Skinning influences ride as floats: GLSL 100 has no integer
            // vertex attributes, and the shader casts the indices.
            ("a_node_indices", 4, 56),
            ("a_node_weights", 4, 72),
        ] {
            if let Some(location) = gl.get_attrib_location(program, name) {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(location, size, glow::FLOAT, false, stride, offset);
            }
        }
    }
    Ok(())
}

/// Above the attributes the main program takes, which it numbers itself.
pub(super) const PRT_CLUSTER_ATTRIBUTE: u32 = 14;

pub(super) const PRT_WEIGHTS_ATTRIBUTE: u32 = 15;

/// Link a pass with the main program's attribute locations, because the
/// model's vertex array is bound to those.
pub(in crate::app::model_preview) unsafe fn link_h2_program(
    gl: &glow::Context,
    main: glow::NativeProgram,
    vertex_source: &str,
    fragment_source: &str,
) -> Result<glow::NativeProgram, String> {
    unsafe {
        let mut attributes = Vec::new();
        for name in [
            "a_position",
            "a_normal",
            "a_texcoord",
            "a_tangent",
            "a_binormal",
            "a_node_indices",
            "a_node_weights",
        ] {
            if let Some(location) = gl.get_attrib_location(main, name) {
                attributes.push((location, name));
            }
        }
        attributes.push((PRT_CLUSTER_ATTRIBUTE, "a_prt_cluster"));
        attributes.push((PRT_WEIGHTS_ATTRIBUTE, "a_prt_weights"));
        let stages = [(glow::VERTEX_SHADER, vertex_source), (glow::FRAGMENT_SHADER, fragment_source)];
        link(gl, &stages, &attributes).map_err(|error| format!("Halo 2 pass: {error}"))
    }
}

/// Whether a context can compile `H2_GLSL_VERSION`. This asks the context, not
/// egui's `ShaderVersion`: that only sorts contexts into the dialects egui
/// itself uses, and calls everything from GL 3.1 up `#version 140`.
pub(super) fn h2_context_supported(major: u32, minor: u32, embedded: bool) -> bool {
    !embedded && (major, minor) >= (3, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `both_shader_dialects_declare_what_the_renderer_binds` reads the GLSL
    /// as text; a compile error would still
    /// only surface at runtime, as a preview that draws nothing. With
    /// `glslangValidator` on PATH (`brew install glslang`), compile every
    /// dialect egui can hand the renderer; without it, skip by name.
    #[test]
    fn pass_support_reads_the_context_version() {
        assert!(h2_context_supported(4, 1, false), "the macOS core profile");
        assert!(h2_context_supported(3, 3, false));
        assert!(!h2_context_supported(3, 2, false));
        assert!(!h2_context_supported(3, 3, true), "OpenGL ES");
    }

    #[test]
    fn every_shader_dialect_compiles_under_glslang() {
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH (brew install glslang)");
            return;
        }
        let dir = crate::core::test_kits::unique_temp_dir("model_preview_glsl");
        for (index, (declaration, modern, precision)) in [
            ("#version 330\n", true, ""),
            ("#version 140\n", true, ""),
            ("#version 300 es\n", true, "precision mediump float;\n"),
            ("#version 120\n", false, ""),
            ("#version 100\n", false, "precision mediump float;\n"),
        ]
        .into_iter()
        .enumerate()
        {
            let (vertex, fragment) = model_shader_sources(declaration, modern, precision);
            for (stage, source) in [("vert", vertex), ("frag", fragment)] {
                let path = dir.join(format!("model_{index}.{stage}"));
                std::fs::write(&path, source).expect("write shader");
                let output = std::process::Command::new("glslangValidator")
                    .arg(&path)
                    .output()
                    .expect("run glslangValidator");
                assert!(
                    output.status.success(),
                    "{} {stage} does not compile:\n{}",
                    declaration.trim(),
                    String::from_utf8_lossy(&output.stdout)
                );
            }
        }
    }

    #[test]
    fn both_shader_dialects_declare_what_the_renderer_binds() {
        for (declaration, modern, precision) in [
            ("#version 330\n", true, ""),
            ("#version 100\n", false, "precision mediump float;\n"),
        ] {
            let (vertex, fragment) = model_shader_sources(declaration, modern, precision);

            // Every attribute the vertex array points at, and every uniform the
            // renderer looks up, has to actually be declared.
            for name in [
                "a_position",
                "a_normal",
                "a_texcoord",
                "a_tangent",
                "a_binormal",
                "a_node_indices",
                "a_node_weights",
            ] {
                assert!(vertex.contains(name), "{name} missing from vertex shader");
            }
            for name in ["u_animated", "u_bones"] {
                assert!(vertex.contains(name), "{name} missing from vertex shader");
            }
            for name in SAMPLER_UNIFORMS {
                assert!(
                    fragment.contains(name),
                    "{name} missing from fragment shader"
                );
            }
            for name in [
                "u_have_a",
                "u_have_b",
                "u_uv_scale_a",
                "u_uv_scale_b",
                "u_shaded",
            ] {
                assert!(
                    fragment.contains(name),
                    "{name} missing from fragment shader"
                );
            }
            assert!(fragment.contains("uniform vec4 u_have_b;"));
            assert!(fragment.contains("uniform vec4 u_uv_scale_b;"));
            // The environment term must stay behind the ambient-strength gate: a
            // shader that asks for none of it has to light exactly as it did
            // before the term existed.

            // A surface with no specular mask must reflect LESS, not more.
            // Having that backwards buried dervish's bare skin — which carries
            // no mask — under a flat wash of environment tint.

            // The detail normal adds to the base one rather than replacing it.
            assert!(
                fragment.contains("tn = vec3(tn.xy + dn, tn.z);"),
                "bump_detail_map should blend into the base normal"
            );
            assert!(
                fragment.contains("u_have_a.z > 0.5 || u_have_a.w > 0.5"),
                "a detail normal with no base bump map should still perturb"
            );
            // The unpack is the engine's own, verbatim from the kit's
            // rasterizer/hlsl/bump_mapping.fx: BUMP_CONVERT on X and Y (byte
            // 128 = exactly flat) for BOTH maps, Z always reconstructed, and
            // NO channel negation — the authored tangent frame carries the
            // convention. `*2-1` unpacks and green flips have both been tried
            // and both bend the lighting; the engine source is the authority.
            assert_eq!(
                fragment
                    .matches("* (255.0 / 127.0) - (128.0 / 127.0)")
                    .count(),
                2,
                "base and detail bumps must both unpack through BUMP_CONVERT"
            );
            assert!(
                fragment.contains("sqrt(1.0 - min(dot(sampled, sampled), 1.0))"),
                "Z must be reconstructed from X/Y, never sampled"
            );
            assert!(
                !fragment.contains("tn.y = -tn.y"),
                "no green flip: the engine feeds sampled Y straight into the tangent frame"
            );
            for name in [
                "u_center",
                "u_scale",
                "u_angles",
                "u_clip_scale",
                "u_depth_scale",
                "u_depth_map",
                "u_perspective",
            ] {
                assert!(vertex.contains(name), "{name} missing from vertex shader");
            }
            for name in ["u_base_color", "u_unlit"] {
                assert!(
                    fragment.contains(name),
                    "{name} missing from fragment shader"
                );
            }

            // The varyings must be declared on both sides or the link fails.
            for name in ["v_uv", "v_normal", "v_tangent", "v_binormal"] {
                assert!(
                    vertex.contains(name) && fragment.contains(name),
                    "{name} not on both sides"
                );
            }

            // Dialect: `texture` vs `texture2D`, and the output keyword pair.
            if modern {
                assert!(fragment.contains("out vec4 out_color;"));
                assert!(fragment.contains("texture(u_tex_base"));
                assert!(vertex.contains("in vec3 a_position"));
            } else {
                assert!(fragment.contains("gl_FragColor"));
                assert!(fragment.contains("texture2D(u_tex_base"));
                assert!(vertex.contains("attribute vec3 a_position"));
            }

            // Only the alpha-test map may discard.
            //
            // A base map's alpha channel carries a mask in Halo — usually
            // specular — not coverage. Discarding on it made dervish, and every
            // other character with a dark diffuse mask, render mostly
            // see-through, while masterchief happened to look fine because his
            // mask is bright. One `discard`, in the alpha-test branch.
            assert_eq!(
                fragment.matches("discard;").count(),
                1,
                "the fragment shader should discard only on the alpha-test map"
            );
            assert!(
                fragment.contains("u_tex_alpha, v_uv * u_uv_scale_c.xy).a < 0.5) discard"),
                "the one discard should be the alpha-test map's"
            );

            for (label, source) in [("vertex", &vertex), ("fragment", &fragment)] {
                assert_eq!(
                    source.matches('{').count(),
                    source.matches('}').count(),
                    "unbalanced braces in the {label} shader — a `format!` escape slipped"
                );
                assert!(
                    source.starts_with(declaration),
                    "the {label} shader must open with its version declaration"
                );
                assert!(
                    !source.contains("{{") && !source.contains("}}"),
                    "a `format!` brace escape survived into the {label} source"
                );
            }
        }
    }

    #[test]
    fn gpu_vertex_layout_matches_the_hand_written_attribute_offsets() {
        assert_eq!(std::mem::size_of::<RenderModelPreviewVertex>(), 88);
        let vertex = RenderModelPreviewVertex::default();
        let base = std::ptr::addr_of!(vertex) as usize;
        let offset = |field: usize| field - base;
        assert_eq!(offset(std::ptr::addr_of!(vertex.normal) as usize), 12);
        assert_eq!(offset(std::ptr::addr_of!(vertex.texcoord) as usize), 24);
        assert_eq!(offset(std::ptr::addr_of!(vertex.tangent) as usize), 32);
        assert_eq!(offset(std::ptr::addr_of!(vertex.binormal) as usize), 44);
        assert_eq!(offset(std::ptr::addr_of!(vertex.node_indices) as usize), 56);
        assert_eq!(offset(std::ptr::addr_of!(vertex.node_weights) as usize), 72);
    }

    /// Both dialects of the model shader, as every caller builds them,
    /// through the shader dump (`BABOON_SHADER_DUMP`): the shots draw only
    /// the modern one.
    #[test]
    fn both_model_shader_dialects_go_through_the_shader_dump() {
        use crate::app::model_preview::gl::glsl::record_source;
        for (declaration, modern, precision) in [
            ("#version 330\n", true, ""),
            ("#version 120\n", false, ""),
            ("#version 100\n", false, "precision mediump float;\n"),
        ] {
            let (vertex, fragment) = model_shader_sources(declaration, modern, precision);
            record_source(&vertex);
            record_source(&fragment);
        }
    }
}
