//! Depth-tested Glow rendering, markers, and preview geometry conversion.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::common::camera::*;
use super::gl::texture::*;
use super::gl::buffer::*;
use super::gl::target::H2Target;
use super::common::sprite_gl::*;
use super::common::view::ce_flare_view;
use super::gl::link::link;
use super::gl::program::Linked;
use super::gl::scope::PostScope;
use super::gl::target::{ColorTarget, Made, Sampling, Texels};
use super::*;
use super::ce::draw::*;
use super::particles::{H3ParticleGl, ReachCheapGl, sprite_maps, upload_cheap_render_texture};
use super::dx9::passes::{
    H2BlendEquation, H2BlendFactor, H2Compare, H2Cull, H2DefaultLight, H2DrawPass, H2Layer,
    H2_GLSL_VERSION, H2_LAYER_COUNT, H2_REFLECTION_SCALE,
    h2_normalization_face,
};
use super::h2::bloom::H2Bloom;
use super::dx9::passes::{H2Clock, h2_prt_lights};
use super::dx9::stage::H2VertexStage;
use crate::app::model_preview::common::image::H3Sampler;
use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};
use crate::core::h2_pixel_shader::SamplerKind;
use eframe::glow::{self, HasContext as _};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// GPU skinning bone budget: three vec4 uniform rows per bone. Halo node
/// indices are bytes, so 256 covers every skeleton any graph can address
/// (Reach's halsey alone has 163 nodes; the old budget of 96 silently
/// refused her animations). 768 vec4 rows sit well inside desktop uniform
/// limits — the practical floor is 1024 vec4.
pub(in crate::app) const MAX_PREVIEW_BONES: usize = 256;

thread_local! {
    static MODEL_GL_RENDERER: RefCell<Option<CachedModelGlRenderer>> = const { RefCell::new(None) };
}

struct CachedModelGlRenderer {
    context_id: usize,
    renderer: Result<ModelGlRenderer, String>,
    /// Geometry whose drawing panicked, with what the panic said: it is not
    /// drawn again, and the panel says why.
    failed: HashMap<u64, String>,
}

thread_local! {
    /// Models whose played Halo CE event effects still have something
    /// running, as the last frame drawn left them.
    pub(in crate::app::model_preview) static CE_EVENTS_RUNNING: RefCell<std::collections::HashSet<u64>> = RefCell::new(Default::default());
}

/// Whether a played event effect on `geometry_id` still needs frames.
pub(in crate::app::model_preview) fn ce_events_running(geometry_id: u64) -> bool {
    CE_EVENTS_RUNNING.with(|running| running.borrow().contains(&geometry_id))
}

/// Why drawing `geometry_id` stopped, if it panicked.
pub(super) fn model_gl_failure(geometry_id: u64) -> Option<String> {
    MODEL_GL_RENDERER.with(|cell| cell.borrow().as_ref()?.failed.get(&geometry_id).cloned())
}

pub(in crate::app::model_preview) fn paint_model_gl(
    info: egui::PaintCallbackInfo,
    painter: &eframe::egui_glow::Painter,
    frame: &ModelGpuFrame,
) {
    let gl = painter.gl();
    let viewport = info.viewport_in_pixels();
    let clip = info.clip_rect_in_pixels();
    let left = viewport.left_px.max(clip.left_px);
    let bottom = viewport.from_bottom_px.max(clip.from_bottom_px);
    let right = (viewport.left_px + viewport.width_px).min(clip.left_px + clip.width_px);
    let top =
        (viewport.from_bottom_px + viewport.height_px).min(clip.from_bottom_px + clip.height_px);
    if right <= left || top <= bottom {
        return;
    }

    unsafe {
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(left, bottom, right - left, top - bottom);
        gl.color_mask(true, true, true, true);
        gl.depth_mask(true);
        let [r, g, b] = frame.background;
        gl.clear_color(r, g, b, 1.0);
        gl.clear_depth_f32(1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
    }

    MODEL_GL_RENDERER.with(|cell| {
        let context_id = Arc::as_ptr(gl) as usize;
        let mut cached = cell.borrow_mut();
        if cached
            .as_ref()
            .is_none_or(|cached| cached.context_id != context_id)
        {
            let created = ModelGlRenderer::new(gl);
            if let Err(error) = &created {
                eprintln!("{error}");
            }
            *cached = Some(CachedModelGlRenderer {
                context_id,
                renderer: created,
                failed: HashMap::new(),
            });
        }
        let Some(CachedModelGlRenderer { renderer: Ok(renderer), failed, .. }) = cached.as_mut() else {
            return;
        };
        if failed.contains_key(&frame.geometry_id) {
            return;
        }
        // A panic part way through leaves the scene target bound; what the
        // frame drew into goes back, so the rest of the window still draws.
        let state = unsafe { PostScope::capture(gl) };
        let painted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { renderer.paint(gl, frame) }));
        if let Err(panic) = painted {
            unsafe {
                state.restore(gl);
                gl.use_program(None);
                gl.bind_vertex_array(None);
            }
            let message = crate::app::shell::worker::panic_detail(panic.as_ref()).to_owned();
            eprintln!("model preview: drawing crashed: {message}");
            failed.insert(frame.geometry_id, message);
        }
    });
}


/// Sampler uniform per slot, in `TextureSlot` order — the fragment shader binds
/// texture unit `i` to `SAMPLER_UNIFORMS[i]`.
const SAMPLER_UNIFORMS: [&str; SLOT_COUNT] = [
    "u_tex_base",
    "u_tex_detail",
    "u_tex_bump",
    "u_tex_bump_detail",
    "u_tex_alpha",
    "u_tex_multipurpose",
];

/// The vertex declarations and helpers every model stage shares: all of the
/// modern source before its `main`.
pub(super) fn model_vertex_prelude(version_declaration: &str) -> String {
    let (vertex, _) = model_shader_sources(version_declaration, true, "");
    vertex.split("void main()").next().unwrap_or_default().to_owned()
}

/// Build the two GLSL sources.
///
/// Split out of `RenderCore::new` so the shaders can be checked
/// without a GL context: a compile failure here disables the whole preview
/// and the only signal is a line on stderr, which no test would see.
fn model_shader_sources(
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

/// Ground-reference grid on the z = 0 plane (the world ground in Halo's
/// Z-up space), Blender-style: minor lines at a spacing picked from the
/// model's size, plus the world X and Y axes across the same extent. Returns
/// the line-list vertices and the index where the axis vertices begin —
/// minor lines first, then two vertices of the X axis, then two of the Y
/// axis — so the axes can draw in their own colors.
fn grid_line_vertices(preview: &RenderModelPreview) -> (Vec<RenderModelPreviewVertex>, usize) {
    let (bounds_center, radius) = preview_center_radius(preview);
    // One Halo world unit is ten feet; a biped is a third of one. Fine
    // spacing for hand-held things, world units for vehicles, and coarser
    // tiers so a BSP is not buried under thousands of lines.
    let spacing = if radius <= 1.5 {
        0.1
    } else if radius <= 15.0 {
        1.0
    } else if radius <= 150.0 {
        10.0
    } else {
        100.0
    };
    const HALF_CELLS: i32 = 16;
    let snap = |value: f32| (value / spacing).round() * spacing;
    let center = [snap(bounds_center[0]), snap(bounds_center[1])];
    let extent = HALF_CELLS as f32 * spacing;
    let (min_x, max_x) = (center[0] - extent, center[0] + extent);
    let (min_y, max_y) = (center[1] - extent, center[1] + extent);
    let line = |from: [f32; 2], to: [f32; 2]| {
        [from, to].map(|[x, y]| RenderModelPreviewVertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            ..Default::default()
        })
    };
    let mut vertices = Vec::with_capacity((HALF_CELLS as usize * 2 + 1) * 4 + 4);
    for step in -HALF_CELLS..=HALF_CELLS {
        let offset = step as f32 * spacing;
        vertices.extend(line(
            [center[0] + offset, min_y],
            [center[0] + offset, max_y],
        ));
        vertices.extend(line(
            [min_x, center[1] + offset],
            [max_x, center[1] + offset],
        ));
    }
    let axis_start = vertices.len();
    vertices.extend(line([min_x, 0.0], [max_x, 0.0]));
    vertices.extend(line([0.0, min_y], [0.0, max_y]));
    (vertices, axis_start)
}

/// Wire the full `RenderModelPreviewVertex` layout into the currently bound
/// vertex array. Shared by the model and grid buffers so no attribute is
/// ever left disabled and reading a stale constant (a disabled array reads
/// `(0, 0, 0, 1)` — a phantom skinning weight on bone 1).
unsafe fn bind_preview_vertex_layout(
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

const H2_SHADOW_MAP_SIZE: i32 = 2048;
/// Past the units a pass's own samplers take.
const H2_SHADOW_MAP_UNIT: u32 = 8;
const H3_NORMAL_TARGET_UNIT: u32 = 9;
const H2_GEL_MASK_UNIT: u32 = 10;
/// `c_rasterizer::render_globals.k_shadow_resolution`.
pub(super) const H3_SHADOW_MAP_SIZE: i32 = 512;
/// `set_shadow_generate_mode`'s bias, the second in depth units.
const H3_SHADOW_SLOPE_BIAS: f32 = 1.414;
const H3_SHADOW_DEPTH_BIAS: f32 = 0.003;

/// A depth texture and the framebuffer to fill it through.
unsafe fn h2_shadow_map(gl: &glow::Context) -> Option<(glow::NativeFramebuffer, glow::NativeTexture)> {
    unsafe {
        let draw = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        let read = gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
        let mut made = Made::new(gl);
        let size = [H2_SHADOW_MAP_SIZE; 2];
        let map = made.depth(size, Texels::DEPTH24, Sampling::NEAREST_CLAMPED, glow::DEPTH_ATTACHMENT);
        let complete = map.as_ref().is_some_and(|map| made.complete(&[map.framebuffer]));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
        let map = map.filter(|_| complete)?;
        made.keep();
        Some((map.framebuffer, map.texture))
    }
}

/// The view from the light, an orthographic box around the whole model.
fn h2_shadow_matrix(preview: &RenderModelPreview, toward: [f32; 3]) -> [f32; 16] {
    let center: [f32; 3] =
        std::array::from_fn(|axis| (preview.bounds_min[axis] + preview.bounds_max[axis]) * 0.5);
    let radius = (0..3)
        .map(|axis| (preview.bounds_max[axis] - preview.bounds_min[axis]).powi(2))
        .sum::<f32>()
        .sqrt()
        .max(1e-4)
        * 0.55;
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    };
    let unit = |v: [f32; 3]| {
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
        [v[0] / length, v[1] / length, v[2] / length]
    };
    let forward = toward.map(|value| -value);
    let right = unit(cross(forward, [0.0, 0.0, 1.0]));
    let up = cross(right, forward);
    let dot = |a: [f32; 3]| a[0] * center[0] + a[1] * center[1] + a[2] * center[2];
    let row = |axis: [f32; 3]| {
        [axis[0] / radius, axis[1] / radius, axis[2] / radius, -dot(axis) / radius]
    };
    let [x, y, z] = [row(right), row(up), row(forward)];
    [
        x[0], y[0], z[0], 0.0,
        x[1], y[1], z[1], 0.0,
        x[2], y[2], z[2], 0.0,
        x[3], y[3], z[3], 1.0,
    ]
}

/// Above the attributes the main program takes, which it numbers itself.
const PRT_CLUSTER_ATTRIBUTE: u32 = 14;
const PRT_WEIGHTS_ATTRIBUTE: u32 = 15;

/// How many models stay on the GPU at once: geometry, ground grid and
/// textures. Past this, the one drawn longest ago gives up its slot.
const MODEL_GPU_SLOTS: usize = 4;

/// Which slot each model draws from, least recently drawn out first.
///
/// A slot per model keeps two previews on screen from replacing each other's
/// upload every frame, which would re-send both models' vertices, indices
/// and textures to the GPU each time.
#[derive(Default)]
struct ModelSlotLru {
    /// `(geometry id, last drawn)`, one per slot in use.
    slots: Vec<(u64, u64)>,
    clock: u64,
}

impl ModelSlotLru {
    /// The slot for `geometry_id`, and whether it must be uploaded: a new
    /// slot, or one taken from the model drawn longest ago.
    fn acquire(&mut self, geometry_id: u64, capacity: usize) -> (usize, bool) {
        self.clock += 1;
        if let Some(index) = self.slots.iter().position(|(id, _)| *id == geometry_id) {
            self.slots[index].1 = self.clock;
            return (index, false);
        }
        if self.slots.len() < capacity.max(1) {
            self.slots.push((geometry_id, self.clock));
            return (self.slots.len() - 1, true);
        }
        let index = self
            .slots
            .iter()
            .enumerate()
            .min_by_key(|(_, (_, used))| *used)
            .map(|(index, _)| index)
            .unwrap_or(0);
        self.slots[index] = (geometry_id, self.clock);
        (index, true)
    }

    /// Forget a slot that could not be created.
    fn abandon(&mut self, index: usize) {
        if index + 1 == self.slots.len() {
            self.slots.pop();
        }
    }
}

/// A slot's simulations: what each carries from frame to frame, restarted
/// together when the slot takes another model.
#[derive(Default)]
pub(in crate::app::model_preview) struct SlotSimulations {
    /// Halo 2's and the gen3 games' particle systems.
    pub(in crate::app::model_preview) particles: particles::H3ParticleField,
    /// Reach's cheap particles.
    pub(in crate::app::model_preview) reach_cheap: particles::cheap::ReachCheapField,
    pub(in crate::app::model_preview) h3_strips: gen3::strips::H3StripField,
    /// When each effect began, for Halo 2's and the gen3 games' effect parts.
    pub(in crate::app::model_preview) effect_onsets: effects::EffectOnsets,
    /// Halo 2's and the gen3 games' cloth.
    pub(in crate::app::model_preview) cloth: cloth::H3ClothField,
    /// Every antenna's spring.
    pub(in crate::app::model_preview) antennas: antennas::spring::ReachAntennaField,
}

impl SlotSimulations {
    /// Each simulation by name, for the G-sim gate. Taken apart whole, so one
    /// added without a name here does not compile.
    #[cfg(test)]
    fn each(&self, mut visit: impl FnMut(&str, &dyn std::fmt::Debug)) {
        let Self {
            particles,
            reach_cheap,
            h3_strips,
            effect_onsets,
            cloth,
            antennas,
        } = self;
        visit("particles", particles);
        visit("reach_cheap", reach_cheap);
        visit("h3_strips", h3_strips);
        visit("effect_onsets", effect_onsets);
        visit("cloth", cloth);
        visit("antennas", antennas);
    }
}

/// One model's buffers, ground grid and textures on the GPU.
pub(super) struct ModelGpuSlot {
    /// What runs from frame to frame, restarted together.
    pub(in crate::app::model_preview) simulations: SlotSimulations,
    pub(in crate::app::model_preview) vertex_array: glow::NativeVertexArray,
    vertex_buffer: glow::NativeBuffer,
    /// Halo 2 per-vertex PRT, unused by every other model.
    prt_buffer: glow::NativeBuffer,
    index_buffer: glow::NativeBuffer,
    /// The ground grid's own vertex array and buffer, built per geometry
    /// (its spacing and extent follow the model's bounds).
    grid_vertex_array: glow::NativeVertexArray,
    grid_vertex_buffer: glow::NativeBuffer,
    grid_vertex_count: i32,
    /// Vertex index where the world-axis lines begin; see `grid_line_vertices`.
    grid_axis_start: i32,
    grid_built: bool,
    /// The `mesh_id` the buffers hold.
    mesh_id: Option<u64>,
    /// How many models the slot has taken, which each game's own state for
    /// the slot follows.
    uploads: u64,
    /// Per batch, where its transparent passes sort from.
    pub(in crate::app::model_preview) h2_sort_points: Vec<[f32; 3]>,
    /// GL textures per material, in `RenderModelPreview::materials` order.
    materials: Vec<MaterialGlTextures>,
    /// The `textures_id` the uploaded `materials` belong to, so a reloaded
    /// model, or another model given this slot, re-uploads rather than drawing
    /// the wrong textures.
    uploaded_textures: Option<u64>,
    /// Halo 3's and Halo 4's lens flares: per flare, then per image, of
    /// `flare_geometry`.
    pub(in crate::app::model_preview) flare_textures: Vec<Vec<Option<glow::NativeTexture>>>,
    pub(in crate::app::model_preview) h3_textures: gen3::pipeline::H3SlotTextures,
    flare_geometry: Option<u64>,
    pub(in crate::app::model_preview) antenna_textures: Vec<Option<glow::NativeTexture>>,
    pub(in crate::app::model_preview) volume_textures: Vec<Option<glow::NativeTexture>>,
    /// The cheap particles' library's render texture array.
    pub(in crate::app::model_preview) reach_cheap_texture: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) strip_textures: Vec<[Option<glow::NativeTexture>; 4]>,
    pub(in crate::app::model_preview) particle_textures: Vec<[Option<glow::NativeTexture>; 4]>,
}

impl ModelGpuSlot {
    unsafe fn new(gl: &glow::Context, program: glow::NativeProgram) -> Result<Self, String> {
        unsafe {
            let vertex_array = gl
                .create_vertex_array()
                .map_err(|error| error.to_string())?;
            let vertex_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            let prt_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            let index_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            gl.bind_vertex_array(Some(vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buffer));
            bind_preview_vertex_layout(gl, program)?;
            gl.bind_vertex_array(None);

            let grid_vertex_array = gl
                .create_vertex_array()
                .map_err(|error| error.to_string())?;
            let grid_vertex_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            gl.bind_vertex_array(Some(grid_vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(grid_vertex_buffer));
            bind_preview_vertex_layout(gl, program)?;
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            Ok(Self {
                mesh_id: None,
                uploads: 0,
                h2_sort_points: Vec::new(),
                vertex_array,
                vertex_buffer,
                prt_buffer,
                index_buffer,
                grid_vertex_array,
                grid_vertex_buffer,
                grid_vertex_count: 0,
                grid_axis_start: 0,
                grid_built: false,
                materials: Vec::new(),
                uploaded_textures: None,
                flare_textures: Vec::new(),
                h3_textures: Default::default(),
                flare_geometry: None,
                simulations: SlotSimulations::default(),
                antenna_textures: Vec::new(),
                volume_textures: Vec::new(),
                reach_cheap_texture: None,
                strip_textures: Vec::new(),
                particle_textures: Vec::new(),
            })
        }
    }

    /// Write `moved` (places, normals and, where given, binormals) over the
    /// uploaded vertices from `first` on, the rest of each vertex as loaded.
    /// A range the preview does not have (a simulation left from another
    /// model) writes nothing.
    pub(in crate::app::model_preview) unsafe fn rewrite_vertices(
        &self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        first: usize,
        moved: impl IntoIterator<Item = cloth::MovedClothVertex>,
    ) {
        let moved: Vec<_> = moved.into_iter().collect();
        let Some(loaded) = frame.preview.vertices.get(first..first + moved.len()) else {
            return;
        };
        let mut vertices = loaded.to_vec();
        for (vertex, (position, normal, binormal)) in vertices.iter_mut().zip(moved) {
            vertex.position = position;
            vertex.normal = normal;
            if let Some(binormal) = binormal {
                vertex.binormal = binormal;
            }
        }
        let offset = first * size_of::<RenderModelPreviewVertex>();
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertex_buffer));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, offset as i32, slice_bytes(&vertices));
        }
    }

    unsafe fn upload_geometry(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        self.mesh_id = Some(frame.mesh_id);
        self.h2_sort_points = h2_sort_points(&frame.preview);
        unsafe {
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertex_buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                slice_bytes(&frame.preview.vertices),
                glow::STATIC_DRAW,
            );
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.index_buffer));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                slice_bytes(&frame.preview.indices),
                glow::STATIC_DRAW,
            );
            match &frame.preview.h2.prt {
                Some(prt) => {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.prt_buffer));
                    gl.buffer_data_u8_slice(
                        glow::ARRAY_BUFFER,
                        slice_bytes(&prt.vertices),
                        glow::STATIC_DRAW,
                    );
                    let stride = std::mem::size_of::<[f32; 5]>() as i32;
                    gl.enable_vertex_attrib_array(PRT_CLUSTER_ATTRIBUTE);
                    gl.vertex_attrib_pointer_f32(
                        PRT_CLUSTER_ATTRIBUTE,
                        1,
                        glow::FLOAT,
                        false,
                        stride,
                        0,
                    );
                    gl.enable_vertex_attrib_array(PRT_WEIGHTS_ATTRIBUTE);
                    gl.vertex_attrib_pointer_f32(
                        PRT_WEIGHTS_ATTRIBUTE,
                        4,
                        glow::FLOAT,
                        false,
                        stride,
                        4,
                    );
                }
                None => {
                    gl.disable_vertex_attrib_array(PRT_CLUSTER_ATTRIBUTE);
                    gl.disable_vertex_attrib_array(PRT_WEIGHTS_ATTRIBUTE);
                }
            }
            match &frame.preview.gen3.prt {
                Some(prt) => {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.prt_buffer));
                    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&prt.vertices), glow::STATIC_DRAW);
                    let stride = std::mem::size_of::<[f32; gen3::prt::COEFFICIENTS]>() as i32;
                    for (index, attribute) in gen3::pipeline::PRT_ATTRIBUTES.into_iter().enumerate() {
                        gl.enable_vertex_attrib_array(attribute);
                        // The first holds a fourth for the linear shader, in the
                        // second's first place.
                        let size = if index == 0 { 4 } else { 3 };
                        gl.vertex_attrib_pointer_f32(attribute, size, glow::FLOAT, false, stride, index as i32 * 12);
                    }
                }
                None => {
                    for attribute in gen3::pipeline::PRT_ATTRIBUTES {
                        gl.disable_vertex_attrib_array(attribute);
                    }
                }
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertex_buffer));
        }
        self.grid_built = false;
    }

    /// Upload the maps of the sprites, flares, antennas and effects the
    /// slot's geometry draws, once per geometry.
    pub(in crate::app::model_preview) unsafe fn sync_sprite_textures(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if self.flare_geometry == Some(frame.geometry_id) {
            return;
        }
        unsafe {
            let drained = self
                .particle_textures
                .drain(..)
                .chain(self.strip_textures.drain(..));
            for texture in drained.flatten().flatten() {
                gl.delete_texture(texture);
            }
            let maps = |look: &particles::H3SpriteShader| sprite_maps(gl, look);
            self.particle_textures =
                frame.preview.particles.iter().map(|system| maps(&system.look.shader)).collect();
            self.strip_textures = frame.preview.gen3.strips.iter().map(|strip| maps(strip.shader())).collect();
            if let Some(texture) = self.reach_cheap_texture.take() {
                gl.delete_texture(texture);
            }
            let render = frame.preview.gen3.cheap_emitters.first().and_then(|at| at.library().render_texture());
            self.reach_cheap_texture = render.and_then(|render| upload_cheap_render_texture(gl, render));
            for texture in self.flare_textures.drain(..).flatten().flatten() {
                gl.delete_texture(texture);
            }
            self.flare_textures = frame
                .preview
                .gen3.flares
                .iter()
                .map(|at| {
                    let (image, texels, mips) = at.bitmap();
                    vec![crate::app::model_preview::gl::texture::upload_h2_image_linear(gl, image, *texels, mips, None)]
                })
                .chain(h4::flare_pass::h4_flare_images(gl, frame))
                .collect();
            for texture in self.antenna_textures.drain(..).flatten() {
                gl.delete_texture(texture);
            }
            self.antenna_textures = frame
                .preview
                .antennas
                .iter()
                .map(|antenna| match &antenna.h3_bitmap {
                    // `build_sprite`. Reach's `draw_textured_transparent_quad`
                    // sets address mode 1 and filter mode 2, taken to be
                    // clamped and trilinear. [inferred]
                    Some((image, texels, mips)) => {
                        let filter =
                            if antenna.is_reach() { BitmapFilterMode::Trilinear } else { BitmapFilterMode::Bilinear };
                        let sampler = H3Sampler { address: [BitmapAddressMode::Clamp; 2], filter };
                        crate::app::model_preview::gl::texture::upload_h2_image_linear(gl, image, *texels, mips, Some(sampler))
                    }
                    None => upload_h2_image(gl, &antenna.image),
                })
                .collect();
            for texture in self.volume_textures.drain(..).flatten() {
                gl.delete_texture(texture);
            }
            self.volume_textures = frame
                .preview
                .gen3.light_volumes
                .iter()
                .map(|at| {
                    let (image, texels, mips) = at.bitmap()?;
                    crate::app::model_preview::gl::texture::upload_h2_image_linear(gl, image, *texels, mips, at.sampler())
                })
                .collect();
            self.flare_geometry = Some(frame.geometry_id);
        }
    }

    /// Start every simulation over, for a model new to this slot.
    ///
    /// The fields restart on their own only when their count changes or time
    /// goes back, so a slot taken over by another model would step the last
    /// model's particles, flags and cloth against the new model's definitions.
    /// How many models the slot has taken.
    pub(in crate::app::model_preview) fn uploads(&self) -> u64 {
        self.uploads
    }

    fn restart_simulations(&mut self) {
        self.simulations = SlotSimulations::default();
    }

    /// Build the ground grid for this slot's model, once.
    unsafe fn sync_grid(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if self.grid_built {
            return;
        }
        let (vertices, axis_start) = grid_line_vertices(&frame.preview);
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.grid_vertex_buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                slice_bytes(&vertices),
                glow::STATIC_DRAW,
            );
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        self.grid_axis_start = axis_start as i32;
        self.grid_vertex_count = vertices.len() as i32;
        self.grid_built = true;
    }

    /// Bring the slot's textures in line with the frame's, uploading only
    /// when they changed or arrived.
    unsafe fn sync_textures(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        // Untextured modes keep what is uploaded: switching back must not
        // upload every map again in the paint callback. They draw without it
        // (see `bound_materials`).
        if frame.textures.is_none() {
            return;
        }
        let wanted = Some(frame.textures_id);
        if self.uploaded_textures == wanted {
            return;
        }
        unsafe { self.release_textures(gl) };
        self.uploaded_textures = wanted;
        let Some(textures) = frame.textures.as_ref() else {
            return;
        };
        self.materials = textures
            .iter()
            .map(|material| {
                let mut uploaded = MaterialGlTextures::default();
                for slot in ALL_SLOTS {
                    let index = slot as usize;
                    let Some(image) = material.get(slot) else {
                        continue;
                    };
                    uploaded.uv_scales[index] = image.scale;
                    uploaded.textures[index] = unsafe { upload_texture(gl, image) };
                }
                uploaded.detail = material.detail;
                let upload = |passes: &[H2DrawPass]| -> Vec<Vec<Option<glow::NativeTexture>>> {
                    passes
                        .iter()
                        .map(|pass| {
                            pass.textures
                                .iter()
                                .map(|texture| {
                                    let image = texture.image.as_ref()?;
                                    unsafe { upload_h2_image(gl, image) }
                                })
                                .collect()
                        })
                        .collect()
                };
                uploaded.h2_textures = std::array::from_fn(|layer| upload(&material.h2_layers[layer]));
                uploaded
            })
            .collect();
    }

    unsafe fn release_textures(&mut self, gl: &glow::Context) {
        for material in self.materials.drain(..) {
            let passes = material.h2_textures.into_iter().flatten().flatten().flatten();
            for texture in material.textures.into_iter().flatten().chain(passes) {
                unsafe { gl.delete_texture(texture) };
            }
        }
    }
}

/// The clear color of the preview, shared with the scene target.
/// The theme's panel color, so a preview sits in the UI around it.
pub(super) fn preview_background(visuals: &egui::Visuals) -> [f32; 3] {
    let [r, g, b, _] = visuals.panel_fill.to_array();
    [r, g, b].map(|channel| channel as f32 / 255.0)
}

fn background_is_dark(background: [f32; 3]) -> bool {
    background.iter().sum::<f32>() / 3.0 < 0.5
}

/// Minor grid lines: a step off the background, toward the contrast.
fn grid_line_color(background: [f32; 3]) -> [f32; 3] {
    const STEP: f32 = 0.11;
    let step = if background_is_dark(background) { STEP } else { -STEP };
    background.map(|channel| (channel + step).clamp(0.0, 1.0))
}

/// Link a pass with the main program's attribute locations, because the
/// model's vertex array is bound to those.
pub(super) unsafe fn link_h2_program(
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

pub(in crate::app::model_preview) fn gl_blend_factor(factor: H2BlendFactor) -> u32 {
    match factor {
        H2BlendFactor::Zero => glow::ZERO,
        H2BlendFactor::One => glow::ONE,
        H2BlendFactor::SrcColor => glow::SRC_COLOR,
        H2BlendFactor::OneMinusSrcColor => glow::ONE_MINUS_SRC_COLOR,
        H2BlendFactor::SrcAlpha => glow::SRC_ALPHA,
        H2BlendFactor::OneMinusSrcAlpha => glow::ONE_MINUS_SRC_ALPHA,
        H2BlendFactor::DstColor => glow::DST_COLOR,
        H2BlendFactor::OneMinusDstColor => glow::ONE_MINUS_DST_COLOR,
        H2BlendFactor::DstAlpha => glow::DST_ALPHA,
        H2BlendFactor::OneMinusDstAlpha => glow::ONE_MINUS_DST_ALPHA,
        H2BlendFactor::SrcAlphaSaturate => glow::SRC_ALPHA_SATURATE,
        H2BlendFactor::ConstantColor => glow::CONSTANT_COLOR,
        H2BlendFactor::OneMinusConstantColor => glow::ONE_MINUS_CONSTANT_COLOR,
        H2BlendFactor::ConstantAlpha => glow::CONSTANT_ALPHA,
        H2BlendFactor::OneMinusConstantAlpha => glow::ONE_MINUS_CONSTANT_ALPHA,
    }
}

pub(in crate::app::model_preview) fn gl_blend_equation(equation: H2BlendEquation) -> u32 {
    match equation {
        H2BlendEquation::Add => glow::FUNC_ADD,
        H2BlendEquation::Subtract => glow::FUNC_SUBTRACT,
        H2BlendEquation::ReverseSubtract => glow::FUNC_REVERSE_SUBTRACT,
        H2BlendEquation::Min => glow::MIN,
        H2BlendEquation::Max => glow::MAX,
    }
}

fn gl_compare(compare: H2Compare) -> u32 {
    match compare {
        H2Compare::Never => glow::NEVER,
        H2Compare::Less => glow::LESS,
        H2Compare::Equal => glow::EQUAL,
        H2Compare::LessOrEqual => glow::LEQUAL,
        H2Compare::Greater => glow::GREATER,
        H2Compare::NotEqual => glow::NOTEQUAL,
        H2Compare::GreaterOrEqual => glow::GEQUAL,
        H2Compare::Always => glow::ALWAYS,
    }
}

/// Where a batch sorts from. The game keys a part by the depth of its rigid
/// node, and a skinned part by node 0, so skinned parts keep their order among
/// themselves: they all share one point here, and the sort is stable.
fn h2_sort_points(preview: &RenderModelPreview) -> Vec<[f32; 3]> {
    let center = preview.vertices.iter().fold([0.0f32; 3], |sum, vertex| {
        std::array::from_fn(|axis| sum[axis] + vertex.position[axis])
    });
    let count = preview.vertices.len().max(1) as f32;
    let center = center.map(|sum| sum / count);
    preview
        .batches
        .iter()
        .map(|batch| {
            let start = batch.index_start as usize;
            let end = start + batch.index_count as usize;
            let indices = preview.indices.get(start..end).unwrap_or(&[]);
            let vertex = |index: &u32| preview.vertices.get(*index as usize);
            let rigid = indices.first().and_then(vertex).is_some_and(|first| {
                indices.iter().filter_map(vertex).all(|other| {
                    other.node_indices[0] == first.node_indices[0]
                        && other.node_weights[1..].iter().all(|weight| *weight <= 0.0)
                })
            });
            if !rigid || indices.is_empty() {
                return center;
            }
            let sum = indices.iter().filter_map(vertex).fold([0.0f32; 3], |sum, vertex| {
                std::array::from_fn(|axis| sum[axis] + vertex.position[axis])
            });
            sum.map(|sum| sum / indices.len() as f32)
        })
        .collect()
}

/// Visible batches farthest first; the eye looks along +Y in view space.
pub(in crate::app::model_preview) fn h2_draw_order(frame: &ModelGpuFrame, points: &[[f32; 3]]) -> Vec<usize> {
    let camera = &frame.camera;
    let depth = |index: usize| {
        let point = points.get(index).copied().unwrap_or(camera.center);
        let offset = std::array::from_fn(|axis| point[axis] - camera.center[axis]);
        rotate_view_vector(camera.yaw, camera.pitch, offset)[1]
    };
    let mut order = frame.visible_batches.clone();
    order.sort_by(|a, b| depth(*b).total_cmp(&depth(*a)));
    order
}

/// Whether a context can compile `H2_GLSL_VERSION`. This asks the context, not
/// egui's `ShaderVersion`: that only sorts contexts into the dialects egui
/// itself uses, and calls everything from GL 3.1 up `#version 140`.
fn h2_context_supported(major: u32, minor: u32, embedded: bool) -> bool {
    !embedded && (major, minor) >= (3, 3)
}

/// The preview's renderer: what every game draws with ([`RenderCore`]),
/// and each game's own drawing ([`GameRender`]), which it hands the frame to.
pub(super) struct ModelGlRenderer {
    core: RenderCore,
    games: GameRenders,
}

/// What a game's drawing is handed each frame.
pub(super) struct FrameCx<'a> {
    pub(super) gl: &'a glow::Context,
    pub(super) frame: &'a ModelGpuFrame,
    pub(super) core: &'a mut RenderCore,
    /// The frame's slot in `core`.
    pub(super) slot: usize,
}

/// A game's drawing: the renderer runs the frame and hands these steps to
/// the game it is from.
pub(super) trait GameRender {
    /// Before the scene target: what moves the geometry or makes lights.
    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>);
    /// The shading, when the mode draws any.
    unsafe fn shade(&mut self, cx: &mut FrameCx<'_>);
    /// After the wireframe: what draws over it.
    unsafe fn finish(&mut self, cx: &mut FrameCx<'_>);

    /// Each simulation the game keeps for `slot` while it holds its `uploads`th
    /// model, by name: what the G-sim gate reads.
    #[cfg(test)]
    fn each_simulation(&self, _slot: usize, _uploads: u64, _visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {}
}

/// Each game's drawing, by the game a preview is from.
#[derive(Default)]
struct GameRenders {
    plain: common::plain::PlainRender,
    dx9: dx9::render::Dx9Render,
    gen3: gen3::render::Gen3Render,
}

impl GameRenders {
    /// Each game's own simulations for `slot`.
    #[cfg(test)]
    fn each_simulation(&self, slot: usize, uploads: u64, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        let Self { plain, dx9, gen3 } = self;
        for game in [plain as &dyn GameRender, dx9, gen3] {
            game.each_simulation(slot, uploads, visit);
        }
    }

    /// The drawing for `frame`, by [`Route::of`] its preview.
    fn of(&mut self, frame: &ModelGpuFrame) -> &mut dyn GameRender {
        match Route::of(&frame.preview) {
            Route::Gen3 => &mut self.gen3,
            Route::Dx9 => &mut self.dx9,
            Route::Plain => &mut self.plain,
        }
    }
}

/// Which drawing a preview goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Gen3,
    Dx9,
    Plain,
}

impl Route {
    /// By the game the preview is from. One of no game this draws draws as
    /// Halo 3's when its tag is gen3 (from a source that names no game), and
    /// plainly when no tag gave it (Chimp's meshes).
    fn of(preview: &RenderModelPreview) -> Self {
        match preview.game {
            Some(GameId::Halo3 | GameId::Halo3Odst | GameId::HaloReach | GameId::Halo4) => Self::Gen3,
            Some(GameId::HaloCe | GameId::Halo2) => Self::Dx9,
            _ if preview.generation == Some(blam_tags::game::Game::Halo3) => Self::Gen3,
            _ => Self::Plain,
        }
    }
}

impl ModelGlRenderer {
    pub(super) fn new(gl: &glow::Context) -> Result<Self, String> {
        Ok(Self { core: RenderCore::new(gl)?, games: GameRenders::default() })
    }

    /// Draw `frame`: its slot, the game's preparation, the scene target, the
    /// shared program and grid, the game's shading, the wireframe, what the
    /// game draws over it, and the scene put back.
    pub(super) unsafe fn paint(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        // Set again if this frame steps the effects and something runs on.
        CE_EVENTS_RUNNING.with(|running| running.borrow_mut().remove(&frame.geometry_id));
        unsafe {
            let Some(slot) = self.core.acquire_frame_slot(gl, frame) else {
                return;
            };
            let game = self.games.of(frame);
            game.prepare(&mut FrameCx { gl, frame, core: &mut self.core, slot });
            let scene = self.core.begin_scene(gl, frame);
            // The game's passes need the scene target's stencil and copies;
            // without one, the frame draws as with them off.
            if scene.is_none() && frame.game_shaders && self.core.shaders_supported {
                let plain = ModelGpuFrame { game_shaders: false, ..frame.clone() };
                self.paint(gl, &plain);
                return;
            }
            self.core.begin_drawing(gl, frame, slot);
            let game = self.games.of(frame);
            if frame.render_mode.draws_shading() {
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
                gl.uniform_1_f32(Some(&self.core.unlit), 0.0);
                game.shade(&mut FrameCx { gl, frame, core: &mut self.core, slot });
            }
            if frame.render_mode.draws_wireframe() {
                self.core.draw_wireframe(gl, frame, slot);
            }
            game.finish(&mut FrameCx { gl, frame, core: &mut self.core, slot });
            self.core.end_frame(gl, scene);
        }
    }

    #[cfg(test)]
    pub(super) fn for_each_simulation(&self, geometry_id: u64, mut visit: impl FnMut(&str, &dyn std::fmt::Debug)) {
        self.core.for_each_simulation(geometry_id, &mut visit);
        if let Some(slot) = self.core.slot_of(geometry_id) {
            self.games.each_simulation(slot, self.core.slot_uploads(slot), &mut visit);
        }
    }
}

/// The GL state the preview draws with: the shared program, slots and
/// targets, and, until each game's drawing moves to its own folder, every
/// game's own state too.
pub(super) struct RenderCore {
    /// The normals the gen3 albedo pass laid down, which Halo 3's dx9
    /// shadow apply reads: the gen3 targets' own, kept as they are remade.
    pub(in crate::app::model_preview) h3_normal_target: Option<glow::NativeTexture>,
    /// What a gen3 array sampler reads with nothing of its own.
    pub(in crate::app::model_preview) black_array: Option<glow::NativeTexture>,
    /// As `h2_bloom`.
    pub(in crate::app::model_preview) sprite_gl: Option<Option<SpriteGl>>,
    /// The frame's CE point lights with those its effects have made, which
    /// only the drawing knows; none while effects make no lights.
    pub(in crate::app::model_preview) ce_points_now: Option<Vec<[[f32; 4]; ce::lights::CE_POINT_LIGHT_ROWS]>>,
    /// As `h2_bloom`.
    pub(in crate::app::model_preview) particle_gl: Option<Option<H3ParticleGl>>,
    /// As `h2_bloom`.
    pub(in crate::app::model_preview) reach_cheap_gl: Option<Option<ReachCheapGl>>,
    /// Particles step once a frame, however many calls draw them.
    pub(in crate::app::model_preview) sprites_advanced: bool,
    /// Halo 2's `target_particle_distortion`, copied from the frame.
    pub(in crate::app::model_preview) h2_distortion_copy: Option<ColorTarget>,
    /// Whether this context's GLSL can draw the games' shaders, which every
    /// pipeline's scene needs; see `H2_GLSL_VERSION`.
    shaders_supported: bool,
    /// Linked pass programs. `None` records a failure, so it is not retried
    /// every frame and the material keeps drawing as it did.
    h2_programs: HashMap<(H2VertexStage, String), Option<Rc<Linked>>>,
    /// Whether this context can draw the gen3 games' shaders: the translated
    /// shaders want GLSL 4.10.
    pub(in crate::app::model_preview) gen3_supported: bool,
    /// What a map nothing fills reads: an unmapped particle stage, a
    /// composite with no bloom built.
    pub(in crate::app::model_preview) black_flat: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) black_cube: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) black_volume: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) white_volume: Option<glow::NativeTexture>,
    h2_normalization_cube: Option<glow::NativeTexture>,
    h2_target: Option<H2Target>,
    pub(in crate::app::model_preview) scene: Option<H2Target>,
    h2_frame_copy: Option<H2Target>,
    /// The gel and `H2GelRows` of the frustum light drawing now.
    pub(in crate::app::model_preview) h2_gel: Option<(Option<glow::NativeTexture>, [[f32; 4]; 4])>,
    /// Made on first use; `Some(None)` once making it failed, so a driver
    /// that cannot is not asked again (and nothing leaks) every frame.
    h2_bloom: Option<Option<H2Bloom>>,
    /// As `h2_bloom`.
    h2_shadow_map: Option<Option<(glow::NativeFramebuffer, glow::NativeTexture)>>,
    pub(in crate::app::model_preview) program: glow::NativeProgram,
    pub(in crate::app::model_preview) slots: Vec<ModelGpuSlot>,
    lru: ModelSlotLru,
    center: glow::NativeUniformLocation,
    scale: glow::NativeUniformLocation,
    angles: glow::NativeUniformLocation,
    clip_scale: glow::NativeUniformLocation,
    depth_scale: glow::NativeUniformLocation,
    depth_map: glow::NativeUniformLocation,
    perspective: glow::NativeUniformLocation,
    /// Optional like the texture uniforms: a driver may strip the skinning
    /// path from a program whose draws never enable it.
    animated: Option<glow::NativeUniformLocation>,
    bones: Option<glow::NativeUniformLocation>,
    base_color: glow::NativeUniformLocation,
    unlit: glow::NativeUniformLocation,
    shaded: Option<glow::NativeUniformLocation>,
    have_a: Option<glow::NativeUniformLocation>,
    have_b: Option<glow::NativeUniformLocation>,
    uv_scale_a: Option<glow::NativeUniformLocation>,
    uv_scale_b: Option<glow::NativeUniformLocation>,
    uv_scale_c: Option<glow::NativeUniformLocation>,
    detail: Option<glow::NativeUniformLocation>,
    /// One sampler location per slot, in `TextureSlot` order.
    samplers: [Option<glow::NativeUniformLocation>; SLOT_COUNT],
    /// Bound to every slot a material lacks; see [`Self::bind_material`].
    pub(in crate::app::model_preview) placeholder: Option<glow::NativeTexture>,
}

/// One material's textures on the GPU, in `TextureSlot` order.
struct MaterialGlTextures {
    textures: [Option<glow::NativeTexture>; SLOT_COUNT],
    /// By `H2Layer` and pass, one texture per entry of its `textures`.
    h2_textures: [Vec<Vec<Option<glow::NativeTexture>>>; H2_LAYER_COUNT],
    uv_scales: [[f32; 2]; SLOT_COUNT],
    detail: DetailComposition,
}

impl Default for MaterialGlTextures {
    fn default() -> Self {
        Self {
            textures: Default::default(),
            h2_textures: Default::default(),
            uv_scales: [[1.0; 2]; SLOT_COUNT],
            detail: DetailComposition::default(),
        }
    }
}

impl RenderCore {
    /// Each simulation `geometry_id`'s slot keeps between frames, by name,
    /// for the refactor's G-sim gate.
    #[cfg(test)]
    pub(super) fn for_each_simulation(&self, geometry_id: u64, visit: impl FnMut(&str, &dyn std::fmt::Debug)) {
        if let Some(slot) = self.slot_of(geometry_id) {
            self.slots[slot].simulations.each(visit);
        }
    }

    /// How many models `slot` has taken: a game's own state for the slot
    /// starts afresh when this moves on.
    pub(in crate::app::model_preview) fn slot_uploads(&self, slot: usize) -> u64 {
        self.slots[slot].uploads
    }

    /// The slot holding `geometry_id`, if one does.
    #[cfg(test)]
    fn slot_of(&self, geometry_id: u64) -> Option<usize> {
        let index = self.lru.slots.iter().position(|(id, _)| *id == geometry_id)?;
        (index < self.slots.len()).then_some(index)
    }

    /// `flag_render_proper` builds its mesh from where the sheet is now.
    pub(in crate::app::model_preview) unsafe fn move_ce_flags(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        ce: &mut ce::slot::CeSlot,
    ) {
        let uploaded = &mut self.slots[slot];
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let lowest = frame.preview.vertices.iter().map(|vertex| vertex.position[2]);
        let ground = lowest.fold(0.0, f32::min);
        let written = ce.simulations.flags.advance(&frame.preview.ce.flags, bones, ground, frame.time);
        for (first, moved) in written {
            let moved = moved.into_iter().map(|(position, normal)| (position, normal, None));
            unsafe { uploaded.rewrite_vertices(gl, frame, first, moved) };
        }
    }

    fn new(gl: &glow::Context) -> Result<Self, String> {
        let shader_version = eframe::egui_glow::ShaderVersion::get(gl);
        let modern = shader_version.is_new_shader_interface();
        let precision = if shader_version.is_embedded() { "precision mediump float;\n" } else { "" };
        let (vertex_source, fragment_source) =
            model_shader_sources(shader_version.version_declaration(), modern, precision);

        unsafe {
            let stages = [(glow::VERTEX_SHADER, vertex_source.as_str()), (glow::FRAGMENT_SHADER, fragment_source.as_str())];
            let program = link(gl, &stages, &[]).map_err(|error| format!("model preview shader: {error}"))?;

            let uniform = |name| {
                gl.get_uniform_location(program, name)
                    .ok_or_else(|| format!("model preview shader has no {name} uniform"))
            };
            let version = gl.version();
            // Made in this order so every texture keeps the name it had.
            let black_array = upload_solid_volume(gl, glow::TEXTURE_2D_ARRAY, [0, 0, 0, 255]);
            let black_flat = upload_black_flat(gl);
            Ok(Self {
                gen3_supported: !version.is_embedded && (version.major, version.minor) >= (4, 1),
                black_flat,
                black_array,
                h3_normal_target: None,
                sprite_gl: None,
                shaders_supported: h2_context_supported(version.major, version.minor, version.is_embedded),
                h2_programs: HashMap::new(),
                black_cube: upload_black_cube(gl),
                particle_gl: None,
                reach_cheap_gl: None,
                ce_points_now: None,
                sprites_advanced: false,
                h2_distortion_copy: None,
                black_volume: upload_solid_volume(gl, glow::TEXTURE_3D, [0, 0, 0, 255]),
                white_volume: upload_solid_volume(gl, glow::TEXTURE_3D, [255; 4]),
                h2_normalization_cube: upload_normalization_cube(gl),
                h2_target: None,
                scene: None,
                h2_frame_copy: None,
                h2_gel: None,
                h2_bloom: None,
                h2_shadow_map: None,
                program,
                slots: Vec::new(),
                lru: ModelSlotLru::default(),
                center: uniform("u_center")?,
                scale: uniform("u_scale")?,
                angles: uniform("u_angles")?,
                clip_scale: uniform("u_clip_scale")?,
                depth_scale: uniform("u_depth_scale")?,
                depth_map: uniform("u_depth_map")?,
                perspective: uniform("u_perspective")?,
                animated: gl.get_uniform_location(program, "u_animated"),
                bones: gl.get_uniform_location(program, "u_bones"),
                base_color: uniform("u_base_color")?,
                unlit: uniform("u_unlit")?,
                // Optional: a driver is free to drop a uniform the program does
                // not end up reading, and the untextured path reads none of
                // these. Missing is not an error.
                shaded: gl.get_uniform_location(program, "u_shaded"),
                have_a: gl.get_uniform_location(program, "u_have_a"),
                have_b: gl.get_uniform_location(program, "u_have_b"),
                uv_scale_a: gl.get_uniform_location(program, "u_uv_scale_a"),
                uv_scale_b: gl.get_uniform_location(program, "u_uv_scale_b"),
                uv_scale_c: gl.get_uniform_location(program, "u_uv_scale_c"),
                detail: gl.get_uniform_location(program, "u_detail"),
                samplers: SAMPLER_UNIFORMS.map(|name| gl.get_uniform_location(program, name)),
                placeholder: upload_placeholder_texture(gl),
            })
        }
    }

    /// The frame's slot, its geometry and textures uploaded and its
    /// simulations restarted for a new model; none if no slot can be made.
    unsafe fn acquire_frame_slot(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) -> Option<usize> {
        unsafe {
            let (slot, upload) = self.lru.acquire(frame.geometry_id, MODEL_GPU_SLOTS);
            if slot == self.slots.len() {
                match ModelGpuSlot::new(gl, self.program) {
                    Ok(created) => self.slots.push(created),
                    Err(error) => {
                        eprintln!("model preview: {error}");
                        self.lru.abandon(slot);
                        return None;
                    }
                }
            }
            if upload {
                self.slots[slot].restart_simulations();
                self.slots[slot].uploads += 1;
            }
            // Overlays merging in keep the slot, and its running effects,
            // and change only the buffers.
            if upload || self.slots[slot].mesh_id != Some(frame.mesh_id) {
                self.slots[slot].upload_geometry(gl, frame);
            }
            self.slots[slot].sync_textures(gl, frame);
            self.ce_points_now = None;
            Some(slot)
        }
    }

    /// The shared program's state, camera and pose, then the ground grid.
    unsafe fn begin_drawing(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            // Sampler i reads texture unit i, fixed for the life of the program.
            for (unit, location) in self.samplers.iter().enumerate() {
                if let Some(location) = location {
                    gl.uniform_1_i32(Some(location), unit as i32);
                }
            }
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.front_face(glow::CCW);
            if frame.show_backfaces {
                gl.disable(glow::CULL_FACE);
            } else {
                gl.enable(glow::CULL_FACE);
                gl.cull_face(glow::BACK);
            }
            gl.uniform_3_f32(
                Some(&self.center),
                frame.camera.center[0],
                frame.camera.center[1],
                frame.camera.center[2],
            );
            gl.uniform_1_f32(Some(&self.scale), frame.camera.scale);
            gl.uniform_2_f32(Some(&self.angles), frame.camera.yaw, frame.camera.pitch);
            gl.uniform_2_f32(
                Some(&self.clip_scale),
                frame.camera.clip_scale[0],
                frame.camera.clip_scale[1],
            );
            gl.uniform_1_f32(Some(&self.depth_scale), frame.camera.depth_scale);
            gl.uniform_2_f32(
                Some(&self.depth_map),
                frame.camera.depth_map[0],
                frame.camera.depth_map[1],
            );
            gl.uniform_1_f32(Some(&self.perspective), frame.camera.perspective);
            match (frame.bones.as_ref(), &self.animated) {
                (Some(rows), Some(animated)) => {
                    gl.uniform_1_f32(Some(animated), 1.0);
                    if let Some(bones) = &self.bones {
                        gl.uniform_4_f32_slice(Some(bones), rows.as_flattened());
                    }
                }
                (None, Some(animated)) => gl.uniform_1_f32(Some(animated), 0.0),
                _ => {}
            }

            // The ground grid draws first, depth-tested and depth-written, so
            // the model correctly occludes the lines behind it and stands on
            // the ones in front.
            if frame.show_grid {
                self.slots[slot].sync_grid(gl, frame);
                let ModelGpuSlot {
                    grid_vertex_array,
                    grid_vertex_count,
                    grid_axis_start,
                    vertex_array,
                    ..
                } = self.slots[slot];
                if grid_vertex_count > 0 {
                    gl.bind_vertex_array(Some(grid_vertex_array));
                    if let Some(animated) = &self.animated {
                        gl.uniform_1_f32(Some(animated), 0.0);
                    }
                    gl.uniform_1_f32(Some(&self.unlit), 1.0);
                    self.bind_material(gl, None);
                    gl.line_width(1.0);
                    let flat = |gl: &glow::Context, location, [r, g, b]: [f32; 3]| {
                        gl.uniform_3_f32(Some(location), r, g, b);
                    };
                    // Minor lines a step off the clear color; the world X
                    // and Y axes in Blender's muted red and green.
                    flat(gl, &self.base_color, grid_line_color(frame.background));
                    gl.draw_arrays(glow::LINES, 0, grid_axis_start);
                    let axis_verts = grid_vertex_count - grid_axis_start;
                    if axis_verts >= 2 {
                        flat(gl, &self.base_color, [0.72, 0.42, 0.42]);
                        gl.draw_arrays(glow::LINES, grid_axis_start, 2);
                    }
                    if axis_verts >= 4 {
                        flat(gl, &self.base_color, [0.42, 0.62, 0.42]);
                        gl.draw_arrays(glow::LINES, grid_axis_start + 2, 2);
                    }
                    gl.bind_vertex_array(Some(vertex_array));
                    if frame.bones.is_some()
                        && let Some(animated) = &self.animated
                    {
                        gl.uniform_1_f32(Some(animated), 1.0);
                    }
                }
            }

        }
    }


    /// The slot's effect onsets: when each effect's scale came on.
    pub(in crate::app::model_preview) fn effect_onsets(&mut self, slot: usize) -> &mut effects::EffectOnsets {
        &mut self.slots[slot].simulations.effect_onsets
    }

    /// Bind the slot's model vertex array again.
    pub(in crate::app::model_preview) unsafe fn bind_slot_vertices(&self, gl: &glow::Context, slot: usize) {
        unsafe { gl.bind_vertex_array(Some(self.slots[slot].vertex_array)) }
    }

    /// The slot's own draw of the visible batches, in the shared program:
    /// what a game's passes do not replace.
    pub(super) unsafe fn draw_slot_batches(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        unsafe { self.draw_batches(gl, frame, false, Self::bound_materials(frame, &self.slots[slot].materials)) }
    }

    /// The wireframe over what shaded, in the shared program.
    unsafe fn draw_wireframe(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        unsafe {
            if frame.render_mode.draws_wireframe() {
                // Halo 3's passes end on their own programs; the camera and
                // material uniforms below are the preview's.
                gl.use_program(Some(self.program));
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);
                gl.line_width(1.0);
                gl.depth_func(glow::LEQUAL);
                gl.uniform_1_f32(Some(&self.unlit), 1.0);
                self.draw_batches(gl, frame, true, Self::bound_materials(frame, &self.slots[slot].materials));
            }
        }
    }

    /// Halo 2's bloom over `bloom_order`, after the wireframe and the sprites.
    pub(in crate::app::model_preview) unsafe fn draw_dx9_bloom(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize, bloom_order: Option<Vec<usize>>) {
        unsafe {
            if let Some(order) = bloom_order {
                gl.use_program(Some(self.program));
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                self.draw_h2_bloom(gl, frame, slot, &order);
            }

        }
    }

    /// The frame's end: fill polygons again, `scene` copied to where the
    /// frame belonged, and nothing left bound.
    unsafe fn end_frame(&mut self, gl: &glow::Context, scene: Option<PostScope>) {
        unsafe {
            // Polygon mode is not reset by egui_glow's generic callback-state
            // restoration and would otherwise turn subsequent UI meshes into wireframes.
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
            if let Some(scene) = scene {
                self.finish_scene(gl, scene);
            }
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.use_program(None);
        }
    }

    /// Whether a frame has flares or antennas to draw.
    pub(in crate::app::model_preview) fn draws_object_sprites(&self, frame: &ModelGpuFrame) -> bool {
        let preview = &frame.preview;
        frame.game_shaders
            && self.shaders_supported
            && !(preview.ce.lights.is_empty()
                && preview.ce.antennas.is_empty()
                && preview.ce.effects.is_empty()
                && preview.ce.contrails.is_empty()
                && preview.ce.light_volumes.is_empty()
                && preview.ce.lightning.is_empty()
                && preview.ce.glows.is_empty()
                && preview.ce.particle_systems.is_empty()
                && preview.antennas.is_empty()
                && preview.h2.flares.is_empty()
                && preview.gen3.flares.is_empty()
                && preview.h4.flares.is_empty()
                && preview.gen3.light_volumes.is_empty()
                && preview.h4.tracers.is_empty()
                && preview.h2.light_volumes.is_empty()
                && preview.h2.contrails.is_empty()
                && preview.h2.liquids.is_empty()
                && preview.particles.is_empty()
                && preview.gen3.cheap_emitters.is_empty()
                && preview.gen3.strips.is_empty()
                && preview.h3.leaves.is_empty())
    }

    /// Halo CE's transparents as `rasterizer_transparent_geometry_draw` draws
    /// them: the model's transparent parts and the effects' groups in one sort
    /// by `z_sort`, far to near, each part from its centroid on its node.
    pub(in crate::app::model_preview) unsafe fn draw_ce_transparents(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        ce: &mut ce::slot::CeSlot,
        effect_gl: &mut Option<Option<CeEffectGl>>,
    ) {
        enum Transparent {
            Part(usize),
            Group(CeGroup),
        }
        unsafe {
            let view = ce_flare_view(&frame.camera);
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            let mut groups = Vec::new();
            if self.draws_object_sprites(frame) && ce_has_groups(&frame.preview) {
                self.sprite_gl.get_or_insert_with(|| link_sprite_gl(gl, self.program));
                self.slots[slot].sync_sprite_textures(gl, frame);
                groups = ce_groups(gl, ce, frame, bones, &view);
            }
            let points = &self.slots[slot].h2_sort_points;
            let parts = frame.visible_batches.iter().map(|&index| {
                let place = frame.preview.batches.get(index).and_then(|batch| batch.sort_position);
                let point = match place {
                    Some((point, node)) => common::view::posed_on(node, point, [0.0; 3], bones).0,
                    None => points.get(index).copied().unwrap_or(frame.camera.center),
                };
                (ce_z_sort(&view, point, false), Transparent::Part(index))
            });
            let groups = groups.into_iter().map(|group| (group.z_sort, Transparent::Group(group)));
            let mut sorted: Vec<_> = parts.chain(groups).collect();
            sorted.sort_by(|(a, _), (b, _)| a.total_cmp(b));
            let vertex_array = self.slots[slot].vertex_array;
            for (_, transparent) in sorted {
                match transparent {
                    Transparent::Part(index) => {
                        // What the passes take as theirs: the model's vertices
                        // and the frame's culling.
                        gl.bind_vertex_array(Some(vertex_array));
                        if frame.show_backfaces {
                            gl.disable(glow::CULL_FACE);
                        } else {
                            gl.enable(glow::CULL_FACE);
                        }
                        self.draw_h2_passes(gl, frame, slot, &[index], H2Layer::Transparent, None);
                    }
                    Transparent::Group(group) => {
                        let Some(flares) = self.sprite_gl.as_ref().and_then(Option::as_ref) else {
                            continue;
                        };
                        draw_ce_group(gl, effect_gl, self.program, flares, ce, &frame.camera, group);
                    }
                }
            }
            // As the passes leave it.
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(vertex_array));
            gl.disable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.depth_mask(true);
            gl.color_mask(true, true, true, true);
            gl.active_texture(glow::TEXTURE0);
            if frame.show_backfaces {
                gl.disable(glow::CULL_FACE);
            } else {
                gl.enable(glow::CULL_FACE);
            }
        }
    }

    /// The slot's uploaded maps, or none in a mode that draws without them.
    fn bound_materials<'a>(frame: &ModelGpuFrame, uploaded: &'a [MaterialGlTextures]) -> &'a [MaterialGlTextures] {
        if frame.textures.is_some() { uploaded } else { &[] }
    }

    fn h2_passes(frame: &ModelGpuFrame, material: usize, layer: H2Layer) -> &[H2DrawPass] {
        frame
            .textures
            .as_ref()
            .and_then(|textures| textures.get(material))
            .map_or(&[], |material| material.h2_layers[layer as usize].as_slice())
    }

    /// Whether the model is camouflaged: camo needs the model's shader for it.
    pub(in crate::app::model_preview) fn h2_camo_on(&self, frame: &ModelGpuFrame) -> bool {
        frame.active_camo
            && (0..frame.preview.materials.len())
                .any(|material| self.h2_material_ready(frame, material, H2Layer::ActiveCamo))
    }

    /// Whether a material's passes stand in for the slots. Transparent ones
    /// always do. Opaque ones need game shaders on, and a template with a
    /// lightmap layer needs that layer to have prepared.
    fn h2_replaces_slots(&self, frame: &ModelGpuFrame, material: usize) -> bool {
        if frame.game_shaders && self.h3_material(frame, material).is_some() {
            return true;
        }
        if self.h2_material_ready(frame, material, H2Layer::Transparent) {
            return true;
        }
        if self.h2_camo_on(frame) {
            return self.h2_material_ready(frame, material, H2Layer::ActiveCamo);
        }
        if !frame.game_shaders {
            return false;
        }
        let lightmap = frame.h2_lightmap_layer();
        self.h2_material_ready(frame, material, lightmap)
            || self.h2_material_ready(frame, material, H2Layer::Prt)
            || (Self::h2_passes(frame, material, lightmap).is_empty()
                && (self.h2_material_ready(frame, material, H2Layer::Overlay)
                    || (frame
                        .textures
                        .as_ref()
                        .and_then(|textures| textures.get(material))
                        .is_none_or(|material| material.get(TextureSlot::Base).is_none())
                        && self.h2_material_ready(frame, material, H2Layer::SelfIllumination))))
    }

    /// Whether a material draws through its passes rather than the slots: it
    /// has some, and every one linked.
    fn h2_material_ready(&self, frame: &ModelGpuFrame, material: usize, layer: H2Layer) -> bool {
        let passes = Self::h2_passes(frame, material, layer);
        self.shaders_supported
            && !passes.is_empty()
            && (layer != H2Layer::Prt || (frame.prt && frame.preview.h2.prt.is_some()))
            && passes.iter().all(|pass| {
                self.h2_programs
                    .get(&(pass.stage, pass.fragment.clone()))
                    .is_some_and(Option::is_some)
            })
    }

    /// Link the dx9 pass programs the frame's materials name: Halo 2's and
    /// Halo CE's passes, and the shadow passes Halo 3 and ODST borrow.
    pub(super) unsafe fn prepare_h2_programs(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if !self.shaders_supported {
            return;
        }
        let Some(textures) = frame.textures.as_ref() else {
            return;
        };
        let passes = textures.iter().flat_map(|material| material.h2_layers.iter().flatten());
        for pass in passes {
            let key = (pass.stage, pass.fragment.clone());
            if self.h2_programs.contains_key(&key) {
                continue;
            }
            let vertex = pass.stage.source(H2_GLSL_VERSION);
            let linked = unsafe { link_h2_program(gl, self.program, &vertex, &pass.fragment) };
            let program = match linked {
                Ok(program) => Some(Rc::new(Linked::new(program))),
                Err(error) => {
                    eprintln!("model preview: {error}");
                    None
                }
            };
            self.h2_programs.insert(key, program);
        }
    }

    /// With game shaders on, send the whole frame to the scene target. What it
    /// replaces is handed back for `finish_scene`.
    unsafe fn begin_scene(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
    ) -> Option<PostScope> {
        if !frame.game_shaders || !self.shaders_supported {
            return None;
        }
        unsafe {
            let state = PostScope::capture(gl);
            let size = state.size();
            if self.scene.as_ref().is_none_or(|scene| scene.size != size) {
                if let Some(old) = self.scene.take() {
                    old.delete(gl);
                }
                self.scene = H2Target::new(gl, size[0], size[1]);
            }
            let Some(scene) = &self.scene else {
                state.restore(gl);
                return None;
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene.framebuffer));
            gl.viewport(0, 0, size[0], size[1]);
            gl.disable(glow::SCISSOR_TEST);
            let [r, g, b] = frame.background;
            gl.clear_color(r, g, b, 1.0);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            Some(state)
        }
    }

    /// Copy the scene target to where the frame belonged, and put everything
    /// back.
    unsafe fn finish_scene(&self, gl: &glow::Context, state: PostScope) {
        let Some(scene) = &self.scene else {
            return;
        };
        unsafe {
            let [x, y, width, height] = state.viewport;
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(scene.framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, state.framebuffer);
            if state.scissored {
                gl.enable(glow::SCISSOR_TEST);
            }
            gl.scissor(state.scissor[0], state.scissor[1], state.scissor[2], state.scissor[3]);
            gl.blit_framebuffer(
                0,
                0,
                scene.size[0],
                scene.size[1],
                x,
                y,
                x + width,
                y + height,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            state.restore(gl);
        }
    }

    /// Draw every texaccum pass into the target, sized to the live viewport,
    /// and put the framebuffer, viewport and scissor back exactly as found.
    pub(in crate::app::model_preview) unsafe fn fill_h2_texaccum(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) {
        let needed = order.iter().any(|&index| {
            frame.preview.batches.get(index).is_some_and(|batch| {
                let material = batch.material_index as usize;
                self.h2_material_ready(frame, material, H2Layer::Texaccum)
                    || self.h2_material_ready(frame, material, H2Layer::LightAlbedo)
            })
        });
        if !needed {
            return;
        }
        unsafe {
            let state = PostScope::capture(gl);
            let size = state.size();
            if self.h2_target.as_ref().is_none_or(|target| target.size != size) {
                if let Some(old) = self.h2_target.take() {
                    old.delete(gl);
                }
                self.h2_target = H2Target::new(gl, size[0], size[1]);
            }
            if let Some(target) = &self.h2_target {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.viewport(0, 0, size[0], size[1]);
                gl.disable(glow::SCISSOR_TEST);
                // The game draws texaccum against the scene's depth: a shader
                // may lay its depth down here alone, with its later passes
                // testing for equal.
                let scene_depth = self.scene.as_ref().map(|scene| scene.depth);
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_STENCIL_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(scene_depth.unwrap_or(target.depth)),
                );
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear_depth_f32(1.0);
                gl.clear(if scene_depth.is_some() {
                    glow::COLOR_BUFFER_BIT
                } else {
                    glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT
                });
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::Texaccum, None);
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::LightAlbedo, None);
            }
            state.restore(gl);
        }
    }

    /// Mark in the stencil what the key light cannot reach: the shadow map
    /// from the light's side, then the model in view against it.
    pub(in crate::app::model_preview) unsafe fn draw_h2_shadow(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) -> bool {
        unsafe {
            let Some((framebuffer, _)) = *self.h2_shadow_map.get_or_insert_with(|| h2_shadow_map(gl)) else {
                return false;
            };
            let h3 = self.h3_draws(frame);
            let size = if h3 { H3_SHADOW_MAP_SIZE } else { H2_SHADOW_MAP_SIZE };
            let state = PostScope::capture(gl);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.disable(glow::SCISSOR_TEST);
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.viewport(0, 0, size, size);
            if h3 {
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(H3_SHADOW_SLOPE_BIAS, H3_SHADOW_DEPTH_BIAS * (1 << 24) as f32);
            }
            self.draw_h2_passes(gl, frame, slot, order, H2Layer::ShadowCaster, None);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            state.restore(gl);
            if h3 {
                return true;
            }
            gl.clear_stencil(0);
            gl.clear(glow::STENCIL_BUFFER_BIT);
            gl.enable(glow::STENCIL_TEST);
            gl.stencil_func(glow::ALWAYS, 1, 0xFF);
            gl.stencil_op(glow::KEEP, glow::KEEP, glow::REPLACE);
            self.draw_h2_passes(gl, frame, slot, order, H2Layer::ShadowMask, None);
            gl.disable(glow::STENCIL_TEST);
            true
        }
    }

    /// Write the bloom layer to the frame's alpha, and add the glow from it.
    unsafe fn draw_h2_bloom(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) {
        let Some((framebuffer, color, size)) =
            self.scene.as_ref().map(|scene| (scene.framebuffer, scene.color, scene.size))
        else {
            return;
        };
        unsafe {
            gl.color_mask(false, false, false, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.color_mask(true, true, true, true);
            self.draw_h2_passes(gl, frame, slot, order, H2Layer::Bloom, None);
            // `sub_6C18F0`: the hologram shader draws while its control is up.
            let hologram = frame.preview.h2.hologram_control.as_deref().is_some_and(|name| {
                frame.function_values.as_ref().and_then(|values| values.get(name)).is_some_and(|value| *value > 0.0)
            });
            if hologram {
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::HologramBloom, None);
            }
            if let Some(bloom) = self.h2_bloom.get_or_insert_with(|| H2Bloom::new(gl, H2_GLSL_VERSION)) {
                bloom.apply(gl, framebuffer, color, size, frame.show_backfaces);
            }
        }
    }

    /// Keep what lies behind the model, for what shows through it: as
    /// `sub_6BD480` does, before anything of the model has drawn.
    pub(in crate::app::model_preview) unsafe fn copy_h2_frame(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, order: &[usize]) {
        let needed = order.iter().any(|&index| {
            frame.preview.batches.get(index).is_some_and(|batch| {
                let material = batch.material_index as usize;
                H2Layer::ALL.iter().any(|layer| {
                    Self::h2_passes(frame, material, *layer)
                        .iter()
                        .any(|pass| pass.textures.iter().any(|texture| texture.frame_copy))
                })
            })
        });
        let Some(scene) = self.scene.as_ref().filter(|_| needed) else {
            return;
        };
        let (framebuffer, [width, height]) = (scene.framebuffer, scene.size);
        unsafe {
            if self.h2_frame_copy.as_ref().is_none_or(|copy| copy.size != [width, height]) {
                if let Some(old) = self.h2_frame_copy.take() {
                    old.delete(gl);
                }
                self.h2_frame_copy = H2Target::new(gl, width, height);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            }
            if let Some(copy) = &self.h2_frame_copy {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(copy.framebuffer));
                gl.blit_framebuffer(
                    0,
                    0,
                    width,
                    height,
                    0,
                    0,
                    width,
                    height,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            }
        }
    }

    /// Draw Halo 2 materials through their passes, in `order`. Leaves the GL
    /// state the wireframe pass and the UI expect.
    pub(in crate::app::model_preview) unsafe fn draw_h2_passes(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
        layer: H2Layer,
        light: Option<&H2DefaultLight>,
    ) {
        let eye = h2_eye_position(&frame.camera);
        let mut drew = false;
        let camouflaged = self.h2_camo_on(frame);
        for &batch_index in order {
            let Some(batch) = frame.preview.batches.get(batch_index) else {
                continue;
            };
            let material = batch.material_index as usize;
            if !self.h2_material_ready(frame, material, layer)
                || (layer != H2Layer::Transparent && !self.h2_replaces_slots(frame, material))
            {
                continue;
            }
            let Some((byte_offset, index_count)) = model_draw_range(
                batch.index_start,
                batch.index_count,
                frame.preview.indices.len(),
            ) else {
                continue;
            };
            for (pass_index, pass) in Self::h2_passes(frame, material, layer).iter().enumerate() {
                if camouflaged && pass.hidden_by_camo {
                    continue;
                }
                if pass.numeric.as_ref().is_some_and(|numeric| !numeric.shows(&frame.outputs_of(material))) {
                    continue;
                }
                let key = (pass.stage, pass.fragment.clone());
                let Some(Some(program)) = self.h2_programs.get(&key).cloned() else {
                    continue;
                };
                drew = true;
                unsafe {
                    gl.use_program(Some(program.program));
                    self.bind_h2_uniforms(gl, frame, material, &program, pass, eye, light);
                    let textures = self.slots[slot]
                        .materials
                        .get(material)
                        .and_then(|material| material.h2_textures[layer as usize].get(pass_index));
                    for (index, texture) in pass.textures.iter().enumerate() {
                        let uploaded = textures.and_then(|list| list.get(index)).copied().flatten();
                        gl.active_texture(glow::TEXTURE0 + texture.slot);
                        match texture.kind {
                            SamplerKind::Texture2D => {
                                let target = self.h2_target.as_ref().map(|target| target.color);
                                let gel = self.h2_gel.and_then(|(texture, _)| texture);
                                let bound = if texture.gel {
                                    gel.or(self.placeholder)
                                } else if texture.texaccum {
                                    target.or(self.placeholder)
                                } else if texture.frame_copy {
                                    let copy = self.h2_frame_copy.as_ref();
                                    copy.map(|copy| copy.color).or(self.placeholder)
                                } else {
                                    uploaded.or(self.placeholder)
                                };
                                gl.bind_texture(glow::TEXTURE_2D, bound);
                            }
                            SamplerKind::Cube => gl.bind_texture(
                                glow::TEXTURE_CUBE_MAP,
                                if texture.normalization {
                                    self.h2_normalization_cube
                                } else {
                                    uploaded.or(self.black_cube)
                                },
                            ),
                            SamplerKind::Volume => {
                                let fallback = if texture.falloff {
                                    self.white_volume
                                } else {
                                    self.black_volume
                                };
                                gl.bind_texture(glow::TEXTURE_3D, uploaded.or(fallback));
                            }
                        }
                        let location = program.location(gl, &format!("s{}", texture.slot));
                        if location.is_some() {
                            gl.uniform_1_i32(location.as_ref(), texture.slot as i32);
                        }
                    }
                    match pass.blend {
                        Some(blend) => {
                            gl.enable(glow::BLEND);
                            gl.blend_equation(gl_blend_equation(blend.equation));
                            gl.blend_func(
                                gl_blend_factor(blend.source),
                                gl_blend_factor(blend.destination),
                            );
                            let [a, r, g, b] = blend.constant.map(|value| value as f32 / 255.0);
                            gl.blend_color(r, g, b, a);
                        }
                        None => gl.disable(glow::BLEND),
                    }
                    match pass.depth.compare {
                        Some(compare) => {
                            gl.enable(glow::DEPTH_TEST);
                            gl.depth_func(gl_compare(compare));
                        }
                        None => gl.disable(glow::DEPTH_TEST),
                    }
                    gl.depth_mask(pass.depth.write);
                    if pass.depth_bias != 0.0 {
                        gl.enable(glow::POLYGON_OFFSET_FILL);
                        gl.polygon_offset(0.0, pass.depth_bias * DEPTH_UNITS);
                    } else {
                        gl.disable(glow::POLYGON_OFFSET_FILL);
                    }
                    let [r, g, b, a] = pass.channels;
                    gl.color_mask(r, g, b, a);
                    match pass.cull {
                        H2Cull::Frame => {}
                        H2Cull::Back | H2Cull::Front => {
                            let front = pass.cull == H2Cull::Front;
                            gl.enable(glow::CULL_FACE);
                            gl.cull_face(if front { glow::FRONT } else { glow::BACK });
                        }
                        H2Cull::None => gl.disable(glow::CULL_FACE),
                    }
                    gl.draw_elements(
                        glow::TRIANGLES,
                        index_count,
                        glow::UNSIGNED_INT,
                        byte_offset,
                    );
                    if pass.cull != H2Cull::Frame {
                        gl.cull_face(glow::BACK);
                        if frame.show_backfaces {
                            gl.disable(glow::CULL_FACE);
                        } else {
                            gl.enable(glow::CULL_FACE);
                        }
                    }
                }
            }
        }
        if drew {
            unsafe {
                gl.use_program(Some(self.program));
                gl.disable(glow::BLEND);
                gl.disable(glow::POLYGON_OFFSET_FILL);
                gl.blend_equation(glow::FUNC_ADD);
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LESS);
                gl.depth_mask(true);
                gl.color_mask(true, true, true, true);
                gl.active_texture(glow::TEXTURE0);
            }
        }
    }

    unsafe fn bind_h2_uniforms(
        &self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        material: usize,
        program: &Linked,
        pass: &H2DrawPass,
        eye: [f32; 3],
        light: Option<&H2DefaultLight>,
    ) {
        unsafe {
            if matches!(pass.stage, H2VertexStage::CeModel | H2VertexStage::CeGlassLit) {
                let object = frame.preview.materials.get(material).map_or(0, |material| material.object);
                let location = program.location(gl, "u_ce_point");
                let points = self.ce_points_now.as_ref().unwrap_or(&frame.ce_points);
                let rows = points.get(usize::from(object)).copied().unwrap_or_default();
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            if matches!(pass.stage, H2VertexStage::ShadowCaster | H2VertexStage::ShadowMask) {
                let location = program.location(gl, "u_shadow_matrix");
                let (toward, shadow_alpha) = if self.h3_draws(frame) {
                    gen3::pipeline::h3_shadow_light(frame)
                } else if frame.default_lights {
                    (h2_prt_lights()[0].direction, 1.0)
                } else {
                    (frame.h2_object_light(), 1.0)
                };
                let matrix = h2_shadow_matrix(&frame.preview, toward);
                gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix);
                // A caster draws into the map: bound then, it would be read
                // and written at once.
                if pass.stage == H2VertexStage::ShadowMask
                    && let Some(Some((_, map))) = self.h2_shadow_map
                {
                    gl.active_texture(glow::TEXTURE0 + H2_SHADOW_MAP_UNIT);
                    gl.bind_texture(glow::TEXTURE_2D, Some(map));
                    let location = program.location(gl, "u_shadow_map");
                    gl.uniform_1_i32(location.as_ref(), H2_SHADOW_MAP_UNIT as i32);
                }
                if let Some(normal) = self.h3_normal_target {
                    gl.active_texture(glow::TEXTURE0 + H3_NORMAL_TARGET_UNIT);
                    gl.bind_texture(glow::TEXTURE_2D, Some(normal));
                    let location = program.location(gl, "u_normal_target");
                    gl.uniform_1_i32(location.as_ref(), H3_NORMAL_TARGET_UNIT as i32);
                    let location = program.location(gl, "u_shadow_toward");
                    gl.uniform_3_f32_slice(location.as_ref(), &toward);
                    let location = program.location(gl, "u_shadow_size");
                    gl.uniform_1_f32(location.as_ref(), H3_SHADOW_MAP_SIZE as f32);
                    let location = program.location(gl, "u_shadow_alpha");
                    gl.uniform_1_f32(location.as_ref(), shadow_alpha);
                }
            }
            for (name, axis) in [
                ("u_view_right", [1.0, 0.0, 0.0]),
                ("u_view_up", [0.0, 0.0, 1.0]),
                ("u_view_back", [0.0, -1.0, 0.0]),
            ] {
                let location = program.location(gl, name);
                if location.is_some() {
                    let vector = unrotate_view_vector(frame.camera.yaw, frame.camera.pitch, axis);
                    gl.uniform_3_f32_slice(location.as_ref(), &vector);
                }
            }
            if let Some(prt) = &frame.preview.h2.prt {
                let location = program.location(gl, "u_prt");
                if location.is_some() {
                    let clusters = match &frame.h2_prt_clusters {
                        Some([lit, shadowed]) => {
                            if light.is_some() { shadowed } else { lit }
                        }
                        None => &prt.clusters,
                    };
                    gl.uniform_4_f32_slice(location.as_ref(), clusters.as_flattened());
                    let direction = if frame.h2_prt_clusters.is_some() {
                        frame.h2_object_light()
                    } else {
                        prt.light
                    };
                    let location = program.location(gl, "u_prt_light");
                    gl.uniform_3_f32_slice(location.as_ref(), &direction);
                }
            }
            let camera = &frame.camera;
            let set_1 = |gl: &glow::Context, program: &Linked, name: &str, value: f32| {
                let location = program.location(gl, name);
                gl.uniform_1_f32(location.as_ref(), value);
            };
            set_1(gl, program, "u_scale", camera.scale);
            set_1(gl, program, "u_depth_scale", camera.depth_scale);
            set_1(gl, program, "u_perspective", camera.perspective);
            set_1(gl, program, "u_reflection_scale", H2_REFLECTION_SCALE);
            set_1(gl, program, "u_animated", if frame.bones.is_some() { 1.0 } else { 0.0 });
            let location = program.location(gl, "u_light_direct");
            let follows = matches!(pass.stage, H2VertexStage::CeModel | H2VertexStage::CeGlassLit)
                && frame.light_follows_camera;
            let direct = if !follows && (frame.default_lights || !pass.stage.is_lightmap()) {
                pass.light_direct
            } else {
                frame.h2_object_light().map(|value| -value)
            };
            gl.uniform_3_f32_slice(location.as_ref(), &direct);
            let location = program.location(gl, "u_light_indirect");
            gl.uniform_3_f32_slice(location.as_ref(), &pass.light_indirect);
            let location = program.location(gl, "u_center");
            gl.uniform_3_f32_slice(location.as_ref(), &camera.center);
            let location = program.location(gl, "u_eye");
            gl.uniform_3_f32_slice(location.as_ref(), &eye);
            if let Some(copy) = &self.h2_frame_copy {
                let location = program.location(gl, "u_view_size");
                gl.uniform_2_f32(location.as_ref(), copy.size[0] as f32, copy.size[1] as f32);
            }
            let location = program.location(gl, "u_angles");
            gl.uniform_2_f32(location.as_ref(), camera.yaw, camera.pitch);
            let location = program.location(gl, "u_clip_scale");
            gl.uniform_2_f32_slice(location.as_ref(), &camera.clip_scale);
            let location = program.location(gl, "u_depth_map");
            gl.uniform_2_f32_slice(location.as_ref(), &camera.depth_map);
            if let Some(rows) = frame.bones.as_ref() {
                let location = program.location(gl, "u_bones");
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            let location = program.location(gl, "u_texture_rows");
            let (outputs, change_colors) = (frame.outputs_of(material), frame.change_colors_of(material));
            let clock = H2Clock { time: frame.time, values: frame.function_values_of(material) };
            let rows = pass.texture_rows_at(clock, &outputs);
            gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            let (tint_facing, tint_glancing) = pass.tints_at(clock);
            let location = program.location(gl, "u_tint_facing");
            gl.uniform_4_f32_slice(location.as_ref(), &tint_facing);
            let location = program.location(gl, "u_tint_glancing");
            gl.uniform_4_f32_slice(location.as_ref(), &tint_glancing);
            for (register, value) in &pass.constants {
                let mut value = *value;
                pass.animate_constant(*register, &mut value, clock);
                if let Some(animation) = &pass.ce_animation {
                    animation.constant(
                        *register,
                        &mut value,
                        frame.time,
                        &change_colors,
                        &outputs,
                    );
                    if animation.eye_register == Some(*register) {
                        let [x, y, z] =
                            unrotate_view_vector(frame.camera.yaw, frame.camera.pitch, [0.0, -1.0, 0.0]);
                        value = [x, y, z, 1.0];
                    }
                }
                for (changed, component, channel) in &pass.change_colors {
                    if changed == register {
                        value[*component] = change_colors[*channel][*component];
                    }
                }
                for (changed, component) in &pass.camo_amounts {
                    if changed == register {
                        value[*component] = if frame.active_camo { 1.0 } else { 0.0 };
                    }
                }
                if let Some(light) = light {
                    for (changed, component) in &pass.light_colors {
                        if changed == register {
                            value[*component] = light.color[*component];
                        }
                    }
                }
                let location = program.location(gl, &format!("c{register}"));
                let location = location.or_else(|| program.location(gl, &format!("ce_registers[{register}]")));
                gl.uniform_4_f32_slice(location.as_ref(), &value);
            }
            if let Some(light) = light {
                let location = program.location(gl, "u_light_position");
                gl.uniform_3_f32_slice(location.as_ref(), &light.position);
                let location = program.location(gl, "u_light_falloff");
                gl.uniform_1_f32(location.as_ref(), 0.5 / light.radius);
            }
            if let Some((_, rows)) = &self.h2_gel {
                let location = program.location(gl, "u_gel_rows");
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            if let (H2VertexStage::LightGel, Some(location), Some((gel, _))) =
                (pass.stage, program.location(gl, "u_gel_map"), &self.h2_gel)
            {
                gl.active_texture(glow::TEXTURE0 + H2_GEL_MASK_UNIT);
                gl.bind_texture(glow::TEXTURE_2D, gel.or(self.placeholder));
                gl.uniform_1_i32(Some(&location), H2_GEL_MASK_UNIT as i32);
            }
            let location = program.location(gl, "zwriteout");
            gl.uniform_4_f32(location.as_ref(), 1.0, 1.0, 1.0, 1.0);
        }
    }

    /// Bind one material's textures, and tell the shader which slots are live.
    ///
    /// A slot with no texture is flagged absent, which makes the shader skip
    /// it, and gets the 1x1 placeholder bound rather than nothing. Leaving
    /// the slot on texture 0 is legal GL, but macOS's driver checks every
    /// active sampler at each draw, whether or not the shader branch reads it,
    /// and logs "unit 0 GLD_TEXTURE_INDEX_2D is unloadable" for any batch
    /// without a base map. That includes every batch while the textures are
    /// still loading.
    unsafe fn bind_material(&self, gl: &glow::Context, material: Option<&MaterialGlTextures>) {
        let mut have = [0.0f32; SLOT_COUNT];
        let mut scales = [[1.0f32; 2]; SLOT_COUNT];
        for slot in ALL_SLOTS {
            let index = slot as usize;
            let texture = material.and_then(|material| material.textures[index]);
            have[index] = if texture.is_some() { 1.0 } else { 0.0 };
            if let Some(material) = material {
                scales[index] = material.uv_scales[index];
            }
            unsafe {
                gl.active_texture(glow::TEXTURE0 + index as u32);
                gl.bind_texture(glow::TEXTURE_2D, texture.or(self.placeholder));
            }
        }
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            if let Some(location) = &self.shaded {
                gl.uniform_1_f32(Some(location), if material.is_some() { 1.0 } else { 0.0 });
            }
            if let Some(location) = &self.have_a {
                gl.uniform_4_f32(Some(location), have[0], have[1], have[2], have[3]);
            }
            if let Some(location) = &self.have_b {
                gl.uniform_4_f32(Some(location), have[4], have[5], 0.0, 0.0);
            }
            for (location, [first, second]) in [
                (&self.uv_scale_a, [0, 1]),
                (&self.uv_scale_b, [2, 3]),
                (&self.uv_scale_c, [4, 5]),
            ] {
                if let Some(location) = location {
                    let ([a, b], [c, d]) = (scales[first], scales[second]);
                    gl.uniform_4_f32(Some(location), a, b, c, d);
                }
            }
            if let Some(location) = &self.detail {
                let detail = material.map(|material| material.detail).unwrap_or_default();
                let function = match detail.function {
                    DetailFunction::LinearBiasedMultiply => 0.0,
                    DetailFunction::BiasedMultiply => 1.0,
                    DetailFunction::Multiply => 2.0,
                    DetailFunction::BiasedAdd => 3.0,
                };
                let xbox = if detail.xbox_channel_order { 1.0 } else { 0.0 };
                gl.uniform_4_f32(Some(location), function, detail.mask as f32, xbox, 0.0);
            }
        }
    }

    unsafe fn draw_batches(
        &self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        wire: bool,
        materials: &[MaterialGlTextures],
    ) {
        for &batch_index in &frame.visible_batches {
            let Some(batch) = frame.preview.batches.get(batch_index) else {
                continue;
            };
            let Some((byte_offset, index_count)) = model_draw_range(
                batch.index_start,
                batch.index_count,
                frame.preview.indices.len(),
            ) else {
                continue;
            };
            let color = if wire && background_is_dark(frame.background) {
                Color32::from_rgb(190, 205, 215)
            } else if wire {
                Color32::from_rgb(38, 55, 65)
            } else {
                batch
                    .flat_color
                    .map(|[r, g, b]| Color32::from_rgb(r, g, b))
                    .unwrap_or_else(|| material_color(batch.material_index))
            };
            // Passes draw after every opaque batch; see `draw_h2_passes`.
            let index = batch.material_index as usize;
            if !wire && self.h2_replaces_slots(frame, index) {
                continue;
            }
            // The wireframe pass draws flat on purpose, so it never binds a
            // texture — and neither does a batch whose material did not resolve.
            let material = (!wire)
                .then(|| materials.get(batch.material_index as usize))
                .flatten();
            unsafe {
                self.bind_material(gl, material);
                gl.uniform_3_f32(
                    Some(&self.base_color),
                    color.r() as f32 / 255.0,
                    color.g() as f32 / 255.0,
                    color.b() as f32 / 255.0,
                );
                gl.draw_elements(
                    glow::TRIANGLES,
                    index_count,
                    glow::UNSIGNED_INT,
                    byte_offset,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::harness::kit::kit_root;
    use crate::core::game::GameId;
    /// Each game's preview goes to its family's drawing; one of no game goes
    /// to gen3 if its tag is gen3, and is drawn plainly if no tag gave it.
    #[test]
    fn a_preview_routes_by_its_game_then_its_generation() {
        use blam_tags::game::Game;
        let route = |game: Option<GameId>, generation: Option<Game>| {
            Route::of(&RenderModelPreview { game, generation, ..Default::default() })
        };
        for game in [GameId::Halo3, GameId::Halo3Odst, GameId::HaloReach, GameId::Halo4] {
            assert_eq!(route(Some(game), Some(Game::Halo3)), Route::Gen3, "{game:?}");
        }
        assert_eq!(route(Some(GameId::HaloCe), Some(Game::Halo1)), Route::Dx9);
        assert_eq!(route(Some(GameId::Halo2), Some(Game::Halo2)), Route::Dx9);
        assert_eq!(route(None, Some(Game::Halo3)), Route::Gen3, "a gen3 tag of no game");
        assert_eq!(route(None, None), Route::Plain, "geometry no tag gave");
    }

    #[test]
    fn the_background_is_the_theme_s_and_grid_lines_stay_visible_on_it() {
        for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
            let background = preview_background(&visuals);
            assert_eq!(background_is_dark(background), visuals.dark_mode);
            let line = grid_line_color(background);
            let (line, background) = (line[0], background[0]);
            assert!((line - background).abs() > 0.05);
            assert_eq!(line > background, visuals.dark_mode);
        }
    }

    #[test]
    fn the_camera_light_keeps_its_place_as_the_view_orbits() {
        let toward_eye = |yaw, pitch| unrotate_view_vector(yaw, pitch, [0.0, -1.0, 0.0]);
        let share = |yaw, pitch| {
            let (light, eye): ([f32; 3], [f32; 3]) = (h2_camera_light(yaw, pitch), toward_eye(yaw, pitch));
            (0..3).map(|axis| light[axis] * eye[axis]).sum::<f32>()
        };
        for (yaw, pitch) in [(0.0, 0.0), (1.5, 0.4), (3.1, -0.6), (-2.0, 1.0)] {
            assert!((share(yaw, pitch) - share(0.0, 0.0)).abs() < 1e-5);
        }
        assert!(share(0.0, 0.0) > 0.5);
    }

    /// Two previews on screen draw alternately. Each model is uploaded once
    /// and then stays; with one slot, every draw replaced the other model.
    #[test]
    fn two_visible_previews_each_upload_once() {
        let mut lru = ModelSlotLru::default();
        let mut uploads = 0;
        let mut slots = std::collections::HashSet::new();
        for _ in 0..100 {
            for geometry in [10, 20] {
                let (slot, upload) = lru.acquire(geometry, MODEL_GPU_SLOTS);
                uploads += usize::from(upload);
                slots.insert((geometry, slot));
            }
        }
        assert_eq!(uploads, 2);
        assert_eq!(slots.len(), 2, "each model keeps its own slot");
    }

    /// Past capacity, the model drawn longest ago gives up its slot, not one
    /// still being drawn.
    #[test]
    fn the_least_recently_drawn_model_is_evicted() {
        let mut lru = ModelSlotLru::default();
        for geometry in 1..=4 {
            lru.acquire(geometry, 4);
        }
        // 1 is drawn again, so 2 is now the oldest.
        assert_eq!(lru.acquire(1, 4), (0, false));
        let (slot, upload) = lru.acquire(5, 4);
        assert!(upload);
        assert_eq!(slot, 1, "model 2's slot");
        assert_eq!(lru.acquire(1, 4), (0, false), "still resident");
        assert!(lru.acquire(2, 4).1, "evicted, so uploaded again");
    }

    #[test]
    fn dense_indices_use_full_u32_offsets_and_counts() {
        let start = 70_002_u32;
        let count = 120_003_u32;
        assert_eq!(
            model_draw_range(start, count, 250_000),
            Some(((start * 4) as i32, count as i32))
        );
    }

    /// With `BLAM_TEST_HREK`: the marine's radio antenna is drawn only while
    /// its `trooper_radio` backpack is.
    #[test]
    fn kit_reach_antenna_shows_only_with_its_permutation() {
        let Some(root) = kit_root(GameId::HaloReach) else { return };
        let path = "objects\\characters\\marine\\marine";
        let data = super::super::harness::kit::kit_preview(
            &root,
            crate::core::game::GameId::HaloReach,
            path,
            "biped",
            b"bipd",
        );
        let shown = |permutation: &str| {
            let mut state = ModelPreviewState::default();
            let selection = ModelRegionSelection { enabled: true, permutation: permutation.to_owned() };
            state.region_selections.insert("flair_backpack".to_owned(), selection);
            let shown = common::features::FeatureShown::of(&data.preview, &state.region_selections);
            shown.list(common::features::paths::ANTENNAS).to_vec()
        };
        assert_eq!(shown("trooper_radio"), [true]);
        assert_eq!(shown("trooper"), [false]);
    }

    /// The grid is a flat z = 0 line list with no skinning influences (a
    /// stray weight would let an animated draw warp it), snapped to world
    /// spacing, with the two axis lines appended last for their own colors.
    #[test]
    fn ground_grid_lies_flat_unskinned_and_ends_with_the_axes() {
        let preview = RenderModelPreview {
            bounds_min: [-0.3, -0.25, 0.0],
            bounds_max: [0.31, 0.25, 0.65],
            ..Default::default()
        };
        let (vertices, axis_start) = grid_line_vertices(&preview);
        assert_eq!(vertices.len(), axis_start + 4, "two axis lines at the end");
        assert_eq!(vertices.len() % 2, 0, "a line list pairs up");
        assert!(axis_start > 0, "no minor lines");
        for vertex in &vertices {
            assert_eq!(vertex.position[2], 0.0, "grid left the ground plane");
            assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
            assert_eq!(vertex.node_weights, [0.0; 4], "grid must not skin");
        }
        // Biped-sized bounds pick the 0.1-unit tier; every line then sits on
        // a multiple of it.
        for vertex in &vertices[..axis_start] {
            for value in [vertex.position[0], vertex.position[1]] {
                let snapped = (value / 0.1).round() * 0.1;
                assert!(
                    (value - snapped).abs() < 1e-4,
                    "line off the 0.1 spacing: {value}"
                );
            }
        }
        // The axis lines really are the world axes.
        assert_eq!(vertices[axis_start].position[1], 0.0);
        assert_eq!(vertices[axis_start + 1].position[1], 0.0);
        assert_eq!(vertices[axis_start + 2].position[0], 0.0);
        assert_eq!(vertices[axis_start + 3].position[0], 0.0);
    }

    /// The depth window must cover an animation's travel: the reach bound is
    /// at least the largest per-frame chain length, and an empty pose adds
    /// nothing.
    #[test]
    fn pose_reach_covers_the_travelling_frame() {
        let frame = |x: f32| {
            vec![
                PreviewNodeTransform {
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    translation: [x, 0.0, 0.9],
                    scale: 1.0,
                },
                PreviewNodeTransform {
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    translation: [0.0, 0.0, 0.25],
                    scale: 1.0,
                },
            ]
        };
        let pose = PreviewAnimationPose::new(0, vec![frame(0.0), frame(3.0)]);
        let reach = pose.reach;
        let travelled = (3.0f32 * 3.0 + 0.9 * 0.9).sqrt() + 0.25;
        assert!(
            reach >= travelled - 1e-4,
            "reach {reach} misses the travelled frame {travelled}"
        );
        let empty = PreviewAnimationPose::new(0, Vec::new());
        assert_eq!(empty.reach, 0.0);
    }

    /// The extended render distance must clip exactly at its declared bounds
    /// in both projections, wherever the dolly has put the eye: near plane
    /// just in front of the eye, far plane at t = K, and post-divide depth
    /// monotone between them so the depth test orders correctly.
    #[test]
    fn depth_remap_clips_at_the_near_and_far_planes_for_any_eye() {
        let k = PREVIEW_RENDER_DISTANCE;

        // Orthographic: w = 1, so z = t·x + y must span NDC over t ∈ [-K, K].
        let [x, y] = depth_map_coefficients(None);
        assert!(((-k) * x + y - -1.0).abs() < 1e-5);
        assert!((k * x + y - 1.0).abs() < 1e-5);

        // Perspective, from the default framing's eye down to a dolly deep
        // inside a level.
        for eye in [1.9, 0.5, 0.02, 0.0005] {
            let [x, y] = depth_map_coefficients(Some(eye));
            let z = |t: f32| t * x + y;
            let w = |t: f32| 1.0 + t / eye;
            let near = -eye * (1.0 - PERSPECTIVE_NEAR_FRACTION);
            let tolerance = 1e-4 * w(k);
            assert!(
                (z(near) + w(near)).abs() < tolerance,
                "eye {eye}: near plane moved"
            );
            assert!(
                (z(k) - w(k)).abs() < tolerance,
                "eye {eye}: far plane short of K"
            );
            let samples = (0..=64)
                .map(|step| near + (k - near) * step as f32 / 64.0)
                .map(|t| z(t) / w(t))
                .collect::<Vec<_>>();
            // Strictly increasing wherever f32 can tell the samples apart;
            // at the deepest dolly the far samples round together, so ties
            // are allowed there but never a reversal.
            assert!(
                samples.windows(2).all(|pair| if eye >= 0.02 {
                    pair[1] > pair[0]
                } else {
                    pair[1] >= pair[0]
                }),
                "eye {eye}: post-divide depth not monotone"
            );
        }
    }

    #[test]
    fn a_models_size_does_not_change_how_far_the_view_pulls_back() {
        let widest = |half: f32| {
            let preview = RenderModelPreview {
                bounds_min: [-half; 3],
                bounds_max: [half; 3],
                ..Default::default()
            };
            let (_, radius) = preview_center_radius(&preview);
            (radius * 1.1 / min_preview_scale(&preview), radius * 1.1 / 0.02)
        };
        let (tiny, tiny_at_a_fixed_scale) = widest(0.01);
        let (level, level_at_a_fixed_scale) = widest(500.0);
        assert!((tiny - level).abs() < 1.0, "{tiny} {level}");
        assert!(tiny_at_a_fixed_scale * 1000.0 < level_at_a_fixed_scale);
    }

    fn camera(scale: f32, perspective: bool) -> PreviewCamera {
        PreviewCamera {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0)),
            center: [0.0; 3],
            radius: 1.0,
            depth_radius: 1.0,
            // Unrotated: view space is world space, the eye looks along +Y.
            yaw: 0.0,
            pitch: 0.0,
            scale,
            perspective,
        }
    }

    /// How far from the viewport center a point lands, in pixels.
    fn screen_offset(camera: &PreviewCamera, point: [f32; 3]) -> f32 {
        (camera.project(point).pos - camera.rect.center()).length()
    }

    /// On the orbit point's plane, perspective frames exactly like the
    /// orthographic view at every zoom, so toggling never jumps and the
    /// pan / zoom-to-cursor math holds in both.
    #[test]
    fn perspective_matches_orthographic_at_the_focus_plane() {
        for scale in [0.05, 1.0, 20.0, 400.0] {
            let point = [0.3 / scale, 0.0, 0.2 / scale];
            let flat = camera(scale, false).project(point).pos;
            let deep = camera(scale, true).project(point).pos;
            assert!(
                (flat - deep).length() < 1e-3,
                "scale {scale}: {flat:?} vs {deep:?}"
            );
        }
    }

    /// The reported defect: zooming in only magnified the picture, so the
    /// perspective stayed the same strength and up close looked flat. With
    /// the eye dollying in, a point the same world distance nearer the
    /// camera than the orbit point grows ever larger relative to one on the
    /// focus plane.
    #[test]
    fn zooming_in_dollies_the_eye_and_deepens_the_perspective() {
        let convergence = |scale: f32| {
            let camera = camera(scale, true);
            let offset = 0.1 / scale;
            let nearer = screen_offset(&camera, [offset, -0.2, 0.0]);
            let on_plane = screen_offset(&camera, [offset, 0.0, 0.0]);
            nearer / on_plane
        };
        let (wide, close) = (convergence(1.0), convergence(8.0));
        assert!(wide > 1.05, "no perspective at the default zoom: {wide}");
        assert!(
            close > wide * 1.5,
            "zoom did not deepen the perspective: {wide} → {close}"
        );

        // The lens itself does not change: at every zoom the eye sits where
        // a 60° field of view frames the focus plane, `1.1 · radius /
        // tan(30°)` out in scaled units — `1 / scale` of that in world units.
        for scale in [1.0, 8.0, 100.0] {
            let eye_world = camera(scale, true).eye_distance() / scale;
            let expected = 1.1 / (30.0f32.to_radians().tan() * scale);
            assert!((eye_world - expected).abs() < 1e-5 * expected.max(1.0));
        }
    }

    /// Overlays behind the eye are dropped rather than drawn stretched across
    /// the viewport; the orthographic view has no eye and keeps everything.
    #[test]
    fn overlay_points_behind_the_eye_are_not_drawn() {
        let scale = 10.0;
        let perspective = camera(scale, true);
        let eye_world = perspective.eye_distance() / scale;
        let in_front = [0.0, -eye_world * 0.5, 0.0];
        let behind = [0.0, -eye_world * 1.5, 0.0];
        assert!(perspective.project(in_front).in_front);
        assert!(!perspective.project(behind).in_front);
        assert!(camera(scale, false).project(behind).in_front);
    }

    #[test]
    fn draw_range_clamps_to_complete_valid_triangles() {
        assert_eq!(model_draw_range(6, 10, 14), Some((24, 6)));
        assert_eq!(model_draw_range(20, 3, 14), None);
    }

    /// The attribute offsets in `RenderCore::new` are hand-written against
    /// this layout, and nothing else checks them — a reordered or resized field
    /// would silently feed the shader the wrong bytes and show as a model that
    /// renders but looks wrong.
    /// A malformed shader disables the whole preview and reports it only on
    /// stderr, so these check the structure no GPU is here to check.
    /// The pan and zoom-to-cursor math turns screen deltas back into world
    /// moves through `unrotate_view_vector`; if it drifts from the forward
    /// rotation, panning smears diagonally and zoom-to-cursor orbits away
    /// from the pointer instead of onto it.
    #[test]
    fn unrotate_inverts_rotate_for_any_view_angles() {
        for (yaw, pitch) in [(0.0, 0.0), (-0.45, 0.25), (1.2, -1.4), (3.0, 0.9)] {
            for vector in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, -2.0, 5.0]] {
                let there = rotate_view_vector(yaw, pitch, vector);
                let back = unrotate_view_vector(yaw, pitch, there);
                for axis in 0..3 {
                    assert!(
                        (back[axis] - vector[axis]).abs() < 1e-5,
                        "round trip failed at yaw {yaw} pitch {pitch}: {vector:?} -> {back:?}"
                    );
                }
            }
        }
    }

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

pub(in crate::app) fn material_color(index: u16) -> Color32 {
    const COLORS: &[(u8, u8, u8)] = &[
        (132, 168, 188),
        (176, 166, 128),
        (142, 182, 150),
        (180, 136, 134),
        (150, 145, 190),
        (186, 154, 104),
        (126, 174, 176),
    ];
    let (r, g, b) = COLORS[index as usize % COLORS.len()];
    Color32::from_rgb(r, g, b)
}

/// Halo 2's, 3's and Reach's antennas, whose maps follow Halo CE's in the
/// slot's, each with whether it is Reach's; Reach's far ends are at `tips`.
/// Those on a permutation not shown are left out.
pub(in crate::app::model_preview) fn h2_antennas(
    frame: &ModelGpuFrame,
    textures: &[Option<glow::NativeTexture>],
    bones: Option<&[[f32; 4]]>,
    view: &common::view::ViewFrame,
    tips: &[Option<[f32; 3]>],
) -> Vec<(Vec<crate::app::model_preview::common::sprites::AntennaQuad>, glow::NativeTexture, bool)> {
    let preview = &frame.preview;
    preview
        .antennas
        .iter()
        .zip(textures)
        .enumerate()
        .filter(|(index, _)| frame.shown.is(common::features::paths::ANTENNAS, *index))
        .filter_map(|(index, (antenna, texture))| {
            let tip = tips.get(index).copied().flatten();
            Some((antenna.quads(bones, view, tip), (*texture)?, antenna.is_reach()))
        })
        .collect()
}

/// The cube `GLOBAL vector-normalization` names. Bilinear, as a direction
/// lookup wants, and small: only its directions matter.
unsafe fn upload_normalization_cube(gl: &glow::Context) -> Option<glow::NativeTexture> {
    const SIZE: usize = 64;
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
        for face in 0..6 {
            gl.tex_image_2d(
                glow::TEXTURE_CUBE_MAP_POSITIVE_X + face as u32,
                0,
                glow::RGBA8 as i32,
                SIZE as i32,
                SIZE as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&h2_normalization_face(face, SIZE))),
            );
        }
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_R, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, name, value as i32);
        }
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
        Some(texture)
    }
}

