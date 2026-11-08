//! The main model program: its GLSL for each dialect, the vertex layout and the linker that binds its attributes.

use super::*;
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
    let vertex_source = format!(
        "{}{precision}\
             {attribute} vec3 a_position;\n\
             {attribute} vec3 a_normal;\n\
             {attribute} vec2 a_texcoord;\n\
             {attribute} vec3 a_tangent;\n\
             {attribute} vec3 a_binormal;\n\
             {attribute} vec4 a_node_indices;\n\
             {attribute} vec4 a_node_weights;\n\
             uniform vec3 u_center;\n\
             uniform float u_scale;\n\
             uniform vec2 u_angles;\n\
             uniform vec2 u_clip_scale;\n\
             uniform float u_depth_scale;\n\
             // Depth remap `z = t * x + y` across the near and far planes.\n\
             // See PREVIEW_RENDER_DISTANCE.\n\
             uniform vec2 u_depth_map;\n\
             // 1 / eye distance (scaled view units); 0 is orthographic.\n\
             uniform float u_perspective;\n\
             // Skinning: three vec4 rows of an affine matrix per bone,\n\
             // bind-relative (world * inverse_bind), so zero-weight vertices\n\
             // and unanimated draws stay exactly where the tag put them.\n\
             uniform float u_animated;\n\
             uniform vec4 u_bones[{bone_rows}];\n\
             {varying_out} vec2 v_uv;\n\
             {varying_out} vec3 v_normal;\n\
             {varying_out} vec3 v_tangent;\n\
             {varying_out} vec3 v_binormal;\n\
             vec3 rotate_view(vec3 value) {{\n\
                 float sy = sin(u_angles.x);\n\
                 float cy = cos(u_angles.x);\n\
                 float sp = sin(u_angles.y);\n\
                 float cp = cos(u_angles.y);\n\
                 vec3 yawed = vec3(value.x * cy - value.y * sy, value.x * sy + value.y * cy, value.z);\n\
                 return vec3(yawed.x, yawed.y * cp - yawed.z * sp, yawed.y * sp + yawed.z * cp);\n\
             }}\n\
             void skin_influence(float index_f, float weight, vec4 point_h,\n\
                                 inout vec3 skinned_pos, inout vec3 skinned_nrm,\n\
                                 inout vec3 skinned_tan, inout vec3 skinned_bin) {{\n\
                 if (weight <= 0.0) {{ return; }}\n\
                 int base = int(index_f + 0.5) * 3;\n\
                 vec4 r0 = u_bones[base];\n\
                 vec4 r1 = u_bones[base + 1];\n\
                 vec4 r2 = u_bones[base + 2];\n\
                 skinned_pos += weight * vec3(dot(r0, point_h), dot(r1, point_h), dot(r2, point_h));\n\
                 skinned_nrm += weight * vec3(dot(r0.xyz, a_normal), dot(r1.xyz, a_normal), dot(r2.xyz, a_normal));\n\
                 skinned_tan += weight * vec3(dot(r0.xyz, a_tangent), dot(r1.xyz, a_tangent), dot(r2.xyz, a_tangent));\n\
                 skinned_bin += weight * vec3(dot(r0.xyz, a_binormal), dot(r1.xyz, a_binormal), dot(r2.xyz, a_binormal));\n\
             }}\n\
             void skinned_pose(out vec3 position, out vec3 normal_in, out vec3 tangent_in, out vec3 binormal_in) {{\n\
                 position = a_position;\n\
                 normal_in = a_normal;\n\
                 tangent_in = a_tangent;\n\
                 binormal_in = a_binormal;\n\
                 float weight_total = a_node_weights.x + a_node_weights.y + a_node_weights.z + a_node_weights.w;\n\
                 if (u_animated > 0.5 && weight_total > 0.0001) {{\n\
                     vec4 point_h = vec4(a_position, 1.0);\n\
                     vec3 sp = vec3(0.0);\n\
                     vec3 sn = vec3(0.0);\n\
                     vec3 st = vec3(0.0);\n\
                     vec3 sb = vec3(0.0);\n\
                     skin_influence(a_node_indices.x, a_node_weights.x, point_h, sp, sn, st, sb);\n\
                     skin_influence(a_node_indices.y, a_node_weights.y, point_h, sp, sn, st, sb);\n\
                     skin_influence(a_node_indices.z, a_node_weights.z, point_h, sp, sn, st, sb);\n\
                     skin_influence(a_node_indices.w, a_node_weights.w, point_h, sp, sn, st, sb);\n\
                     position = sp / weight_total;\n\
                     normal_in = sn;\n\
                     tangent_in = st;\n\
                     binormal_in = sb;\n\
                 }}\n\
             }}\n\
             vec4 clip_position(vec3 position) {{\n\
                 vec3 view = rotate_view((position - u_center) * u_scale);\n\
                 // Perspective: the eye sits along -Y so w grows with depth;\n\
                 // at the focus plane (view.y = 0) w is exactly 1 and the\n\
                 // framing matches the orthographic view. See the derivation\n\
                 // on PERSPECTIVE_FOV_Y.\n\
                 float w = 1.0 + view.y * u_perspective;\n\
                 float z_clip = view.y * u_depth_scale * u_depth_map.x + u_depth_map.y;\n\
                 return vec4(view.x * u_clip_scale.x, view.z * u_clip_scale.y, z_clip, w);\n\
             }}\n\
             void main() {{\n\
                 vec3 position;\n\
                 vec3 normal_in;\n\
                 vec3 tangent_in;\n\
                 vec3 binormal_in;\n\
                 skinned_pose(position, normal_in, tangent_in, binormal_in);\n\
                 gl_Position = clip_position(position);\n\
                 vec3 source_normal = length(normal_in) > 0.0001 ? normalize(normal_in) : vec3(0.0, 0.0, 1.0);\n\
                 v_normal = rotate_view(source_normal);\n\
                 v_tangent = rotate_view(tangent_in);\n\
                 v_binormal = rotate_view(binormal_in);\n\
                 v_uv = a_texcoord;\n\
             }}\n",
        version_declaration
    );
    // The lighting rig is the one this preview has always used — a key, a
    // fill, a rim and an overhead term, all in view space so they follow the
    // camera. It moved from the vertex shader to here so a normal map has
    // something to perturb: per-vertex lighting would sample the map and
    // then throw the result away between vertices.
    let fragment_source = format!(
        "{}{precision}\
             {varying_in} vec2 v_uv;\n\
             {varying_in} vec3 v_normal;\n\
             {varying_in} vec3 v_tangent;\n\
             {varying_in} vec3 v_binormal;\n\
             uniform sampler2D u_tex_base;\n\
             uniform sampler2D u_tex_detail;\n\
             uniform sampler2D u_tex_bump;\n\
             uniform sampler2D u_tex_bump_detail;\n\
             uniform sampler2D u_tex_alpha;\n\
             uniform sampler2D u_tex_multipurpose;\n\
             // Which slots this material bound: 0-3 in `u_have_a`, 4-5 in\n\
             // `u_have_b.xy`. Each slot's UV multiplier is a per-axis pair:\n\
             // base and detail in `_a`, bump and bump detail in `_b`, alpha\n\
             // and multipurpose in `_c`.\n\
             uniform vec4 u_have_a;\n\
             uniform vec4 u_have_b;\n\
             uniform vec4 u_uv_scale_a;\n\
             uniform vec4 u_uv_scale_b;\n\
             uniform vec4 u_uv_scale_c;\n\
             // Detail combine: x the function (0 the Halo 3 linear-space\n\
             // biased multiply; 1-3 Halo CE's biased multiply, multiply and\n\
             // biased add, on stored values), y the CE detail mask (0 none),\n\
             // z the Xbox multipurpose channel order.\n\
             uniform vec4 u_detail;\n\
             uniform vec3 u_base_color;\n\
             uniform float u_unlit;\n\
             uniform float u_shaded;\n\
             {fragment_output}\n\
             vec3 to_linear(vec3 c) {{ return pow(c, vec3(2.2)); }}\n\
             vec3 to_srgb(vec3 c) {{ return pow(clamp(c, 0.0, 1.0), vec3(1.0 / 2.2)); }}\n\
             void main() {{\n\
                 vec3 albedo = u_base_color;\n\
                 bool shaded = u_shaded > 0.5;\n\
                 if (shaded) {{\n\
                     // Alpha test first: a discarded fragment costs nothing else.\n\
                     // This is the ONLY thing that discards. A base map's alpha\n\
                     // is not opacity in Halo — it carries a mask, most often\n\
                     // specular — so treating it as coverage punched holes\n\
                     // through every character whose diffuse had a dark mask.\n\
                     if (u_have_b.x > 0.5 && {sample}(u_tex_alpha, v_uv * u_uv_scale_c.xy).a < 0.5) discard;\n\
                     if (u_have_a.x > 0.5) {{\n\
                         albedo = to_linear({sample}(u_tex_base, v_uv * u_uv_scale_a.xy).rgb);\n\
                     }}\n\
                     // Halo detail maps are grey-centred and modulate: mid-grey\n\
                     // leaves the base untouched, darker and lighter push it.\n\
                     // They tile — commonly sixteen times — so the scale matters.\n\
                     if (u_have_a.y > 0.5) {{\n\
                         vec3 detail = {sample}(u_tex_detail, v_uv * u_uv_scale_a.zw).rgb;\n\
                         if (u_detail.x < 0.5) {{\n\
                             albedo = clamp(albedo * to_linear(detail) * 2.0, 0.0, 1.0);\n\
                         }} else {{\n\
                             // Halo CE's ShaderModel() (fx/model_common.h): the\n\
                             // mask lerps the detail toward its function's\n\
                             // neutral value, then combines with the base.\n\
                             float mask = 1.0;\n\
                             if (u_detail.y > 0.5 && u_have_b.y > 0.5) {{\n\
                                 vec4 mp = {sample}(u_tex_multipurpose, v_uv * u_uv_scale_c.zw);\n\
                                 if (u_detail.z > 0.5) mp = mp.agrb;\n\
                                 float k = u_detail.y;\n\
                                 float channel = k < 2.5 ? mp.b : (k < 4.5 ? mp.g : (k < 6.5 ? mp.a : mp.r));\n\
                                 mask = mod(k, 2.0) > 0.5 ? 1.0 - channel : channel;\n\
                             }}\n\
                             vec3 neutral = u_detail.x > 1.5 && u_detail.x < 2.5 ? vec3(1.0) : vec3(0.5);\n\
                             vec3 d = mix(neutral, detail, mask);\n\
                             vec3 base = to_srgb(albedo);\n\
                             vec3 combined = u_detail.x < 1.5 ? base * d * 2.0\n\
                                 : (u_detail.x < 2.5 ? base * d : base + 2.0 * d - 1.0);\n\
                             albedo = to_linear(clamp(combined, 0.0, 1.0));\n\
                         }}\n\
                     }}\n\
                 }}\n\
                 vec3 normal = normalize(v_normal);\n\
                 if (shaded && (u_have_a.z > 0.5 || u_have_a.w > 0.5)) {{\n\
                     // Tangent space, using the frame the tag authored rather\n\
                     // than a reconstructed one — mirrored UV islands need the\n\
                     // binormal's own sign to come out the right way round.\n\
                     vec3 t = normalize(v_tangent);\n\
                     vec3 b = normalize(v_binormal);\n\
                     if (length(v_tangent) > 0.0001 && length(v_binormal) > 0.0001) {{\n\
                         // The engine's own unpack, verbatim from the kit's\n\
                         // source (rasterizer/hlsl/bump_mapping.fx): X and Y\n\
                         // come off the texture through BUMP_CONVERT — byte\n\
                         // 128 is exactly flat — Z is ALWAYS reconstructed\n\
                         // rather than sampled (the compressed formats carry\n\
                         // no trustworthy blue), and the components go into\n\
                         // the authored tangent frame with no negation.\n\
                         vec3 tn = vec3(0.0, 0.0, 1.0);\n\
                         if (u_have_a.z > 0.5) {{\n\
                             vec2 sampled = {sample}(u_tex_bump, v_uv * u_uv_scale_b.xy).xy * (255.0 / 127.0) - (128.0 / 127.0);\n\
                             tn = vec3(sampled, sqrt(1.0 - min(dot(sampled, sampled), 1.0)));\n\
                         }}\n\
                         // A detail normal tiles far finer than the base one\n\
                         // and adds its tangent-plane components over it —\n\
                         // the engine's stock combine — staying stable when\n\
                         // either map is flat.\n\
                         if (u_have_a.w > 0.5) {{\n\
                             vec2 dn = {sample}(u_tex_bump_detail, v_uv * u_uv_scale_b.zw).xy * (255.0 / 127.0) - (128.0 / 127.0);\n\
                             tn = vec3(tn.xy + dn, tn.z);\n\
                         }}\n\
                         tn = normalize(tn);\n\
                         normal = normalize(t * tn.x + b * tn.y + normal * tn.z);\n\
                     }}\n\
                 }}\n\
                 // The rig this preview has always used — a key, a fill, a rim\n\
                 // and an overhead term, in view space so they follow the camera.\n\
                 // Neutral on purpose: the maps supply the colour, and anything\n\
                 // that tried to supply more needed a scene this preview has not\n\
                 // got.\n\
                 float key = max(dot(normal, normalize(vec3(-0.35, -0.55, 0.76))), 0.0);\n\
                 float fill = max(dot(normal, normalize(vec3(0.72, 0.22, 0.36))), 0.0);\n\
                 float rim = pow(clamp(1.0 - abs(normal.y), 0.0, 1.0), 2.0);\n\
                 float overhead = clamp(normal.z * 0.5 + 0.5, 0.0, 1.0);\n\
                 float shade = clamp(0.42 + key * 0.46 + fill * 0.16 + rim * 0.10 + overhead * 0.08, 0.32, 1.22);\n\
                 vec3 lit = albedo * shade + vec3(key * key * (22.0 / 255.0));\n\
                 vec3 flat_color = shaded ? to_srgb(albedo) : u_base_color;\n\
                 vec3 result = mix(to_srgb(lit), flat_color, u_unlit);\n\
                 {output_name} = vec4(result, 1.0);\n\
             }}\n",
        version_declaration
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
}
