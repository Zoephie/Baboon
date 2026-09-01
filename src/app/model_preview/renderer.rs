//! Depth-tested Glow rendering, markers, and preview geometry conversion.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;
use eframe::glow::{self, HasContext as _};
use std::cell::RefCell;
use std::sync::Arc;

pub(super) fn draw_model_viewport(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    desired_size: Vec2,
) {
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    // Pixels → world units at the current zoom, for the pan and zoom-to-cursor
    // math below. Uses the same bounds-derived fit as `PreviewCamera`.
    let (_, radius) = preview_center_radius(&data.preview);
    let fit = rect.width().min(rect.height()) / (radius * 2.2).max(0.001);
    let world_per_pixel = 1.0 / (fit * state.scale).max(0.0001);

    // Panning moves the world-space orbit point, not a screen offset: the
    // camera then orbits and zooms around whatever the user framed, which is
    // what makes a corner of a BSP inspectable. Screen-space panning kept the
    // orbit pivot at the model's center, so orbiting a panned view swung the
    // framed geometry away.
    let mut pan_world = |state: &mut ModelPreviewState, delta: Vec2| {
        let moved = unrotate_view_vector(
            state.yaw,
            state.pitch,
            [
                -delta.x * world_per_pixel,
                0.0,
                delta.y * world_per_pixel,
            ],
        );
        state.focus[0] += moved[0];
        state.focus[1] += moved[1];
        state.focus[2] += moved[2];
    };
    if response.dragged_by(egui::PointerButton::Middle) {
        pan_world(state, response.drag_delta());
    } else if response.dragged_by(egui::PointerButton::Primary) {
        let delta = response.drag_delta();
        if ui.input(|i| i.modifiers.shift) {
            pan_world(state, delta);
        } else {
            state.yaw += delta.x * 0.01;
            state.pitch = (state.pitch + delta.y * 0.01).clamp(-1.45, 1.45);
        }
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() > f32::EPSILON {
            let old_scale = state.scale.clamp(MIN_PREVIEW_SCALE, MAX_PREVIEW_SCALE);
            let new_scale =
                (old_scale * (scroll / 450.0).exp()).clamp(MIN_PREVIEW_SCALE, MAX_PREVIEW_SCALE);
            // Zoom toward the cursor: shift the orbit point so the geometry
            // under the pointer stays put. Without this, zooming into a BSP
            // always dives at its center and the doorway drifts offscreen.
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                let towards = pointer - rect.center();
                let factor = 1.0 / (fit * old_scale) - 1.0 / (fit * new_scale);
                let moved = unrotate_view_vector(
                    state.yaw,
                    state.pitch,
                    [towards.x * factor, 0.0, -towards.y * factor],
                );
                state.focus[0] += moved[0];
                state.focus[1] += moved[1];
                state.focus[2] += moved[2];
            }
            state.scale = new_scale;
        }
    }

    let camera = PreviewCamera::new(data, state, rect);
    let preview = Arc::clone(&data.preview);
    let visible_batches = preview
        .batches
        .iter()
        .enumerate()
        .filter_map(|(index, batch)| {
            // The overlay toggles are draw-time filters: the collision and
            // physics layers sit merged in the geometry from the moment the
            // worker delivers them, and showing or hiding them costs one
            // batch-list rebuild — not the tag re-reads that froze the frame
            // when the toggles re-merged the preview. Gated on
            // `overlays_loaded` so a standalone collision/physics tag — whose
            // MAIN content uses these region names — is never filtered.
            let region = batch.region_name.as_str();
            let is_overlay = region == COLLISION_REGION || region == PHYSICS_REGION;
            if state.overlays_loaded && is_overlay {
                if region == COLLISION_REGION && !state.show_collision {
                    return None;
                }
                if region == PHYSICS_REGION && !state.show_physics {
                    return None;
                }
            }
            // "Render off" leaves only the overlay layers on screen.
            if !state.show_render && !is_overlay {
                return None;
            }
            let selection = state.region_selections.get(&batch.region_name)?;
            (selection.enabled && selection.permutation == batch.permutation_name).then_some(index)
        })
        .collect::<Vec<_>>();
    let frame = ModelGpuFrame {
        preview,
        geometry_id: data.geometry_id,
        visible_batches,
        camera: camera.gpu_uniforms(),
        render_mode: state.render_mode,
        show_backfaces: state.show_backfaces,
        textures: state
            .shaded
            .then(|| data.textures.clone())
            .flatten(),
    };
    painter.add(egui::PaintCallback {
        rect,
        callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
            paint_model_gl(info, painter, &frame);
        })),
    });
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));

    if state.show_markers {
        let hover_pos = if response.hovered() {
            ui.input(|i| i.pointer.hover_pos())
        } else {
            None
        };
        let marker_filter = state.marker_filter.trim().to_ascii_lowercase();
        for marker in &data.preview.markers {
            // Name filter (case-insensitive substring; empty = show all).
            if !marker_filter.is_empty()
                && !marker.name.to_ascii_lowercase().contains(&marker_filter)
            {
                continue;
            }
            let projected = camera.project(marker.position);
            let axis_deltas = marker_axis_screen_deltas(&camera, marker.axes);
            draw_marker_axes(&painter, projected.pos, axis_deltas);
            if hover_pos.is_some_and(|pos| marker_axes_hovered(pos, projected.pos, axis_deltas)) {
                let text_pos = projected.pos + Vec2::new(7.0, -7.0);
                let label_rect = egui::Rect::from_min_size(
                    text_pos,
                    Vec2::new(marker.name.len() as f32 * 6.0 + 8.0, 16.0),
                );
                painter.rect_filled(
                    label_rect,
                    2.0,
                    Color32::from_rgba_unmultiplied(0, 0, 0, 180),
                );
                painter.text(
                    text_pos + Vec2::new(4.0, 1.0),
                    Align2::LEFT_TOP,
                    &marker.name,
                    FontId::proportional(10.0),
                    Color32::from_rgb(255, 230, 40),
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ModelGpuCamera {
    center: [f32; 3],
    scale: f32,
    yaw: f32,
    pitch: f32,
    clip_scale: [f32; 2],
    depth_scale: f32,
    /// 1.0 = perspective divide on, 0.0 = orthographic.
    perspective: f32,
}

/// Perspective geometry, shared between the shader and the CPU-side marker
/// projection. The eye sits at view-space `y = -d` with `d = 2 · depth_radius
/// · scale`; the depth bound is `r = 1.05 · depth_radius · scale`, so `r/d`
/// is this constant. From it: `w = 1 + view.y · depth_scale · (r/d)` (which
/// is 1 exactly at the focus plane, keeping the framing identical to the
/// orthographic view there and bounded ≥ 0.5 for all in-range geometry), and
/// `z_clip = view.y · depth_scale + (r/d)` maps the depth bound onto NDC
/// after the divide, monotonically because `d > r`.
const PERSPECTIVE_R_OVER_D: f32 = 1.05 / 2.0;

struct ModelGpuFrame {
    preview: Arc<RenderModelPreview>,
    geometry_id: u64,
    visible_batches: Vec<usize>,
    camera: ModelGpuCamera,
    render_mode: ModelRenderMode,
    show_backfaces: bool,
    /// `None` until the worker has resolved them, or when shading is off.
    textures: Option<Arc<Vec<MaterialTextures>>>,
}

thread_local! {
    static MODEL_GL_RENDERER: RefCell<Option<CachedModelGlRenderer>> = const { RefCell::new(None) };
}

struct CachedModelGlRenderer {
    context_id: usize,
    renderer: Result<ModelGlRenderer, String>,
}

fn paint_model_gl(
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
        gl.clear_color(228.0 / 255.0, 238.0 / 255.0, 244.0 / 255.0, 1.0);
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
            });
        }
        let Some(Ok(renderer)) = cached.as_mut().map(|cached| &mut cached.renderer) else {
            return;
        };
        unsafe { renderer.paint(gl, frame) };
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
];

/// Build the two GLSL sources.
///
/// Split out of `ModelGlRenderer::new` so the shaders can be checked
/// without a GL context: a compile failure here disables the whole preview
/// and the only signal is a line on stderr, which no test would see.
fn model_shader_sources(version_declaration: &str, modern: bool, precision: &str) -> (String, String) {
        let (attribute, varying_out, varying_in, fragment_output, output_name, sample) = if modern
        {
            ("in", "out", "in", "out vec4 out_color;", "out_color", "texture")
        } else {
            ("attribute", "varying", "varying", "", "gl_FragColor", "texture2D")
        };
        // Position and normal are required; the rest are read only on the
        // shaded path and a driver may drop them from an unused program.
        let persp_ratio = PERSPECTIVE_R_OVER_D;
        let vertex_source = format!(
            "{}{precision}\
             {attribute} vec3 a_position;\n\
             {attribute} vec3 a_normal;\n\
             {attribute} vec2 a_texcoord;\n\
             {attribute} vec3 a_tangent;\n\
             {attribute} vec3 a_binormal;\n\
             uniform vec3 u_center;\n\
             uniform float u_scale;\n\
             uniform vec2 u_angles;\n\
             uniform vec2 u_clip_scale;\n\
             uniform float u_depth_scale;\n\
             uniform float u_perspective;\n\
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
             void main() {{\n\
                 vec3 view = rotate_view((a_position - u_center) * u_scale);\n\
                 // Perspective: the eye sits along -Y so w grows with depth;\n\
                 // at the focus plane (view.y = 0) w is exactly 1 and the\n\
                 // framing matches the orthographic view. See the derivation\n\
                 // on PERSPECTIVE_R_OVER_D.\n\
                 float w = mix(1.0, 1.0 + view.y * u_depth_scale * {persp_ratio}, u_perspective);\n\
                 float z_clip = view.y * u_depth_scale + u_perspective * {persp_ratio};\n\
                 gl_Position = vec4(view.x * u_clip_scale.x, view.z * u_clip_scale.y, z_clip, w);\n\
                 vec3 source_normal = length(a_normal) > 0.0001 ? normalize(a_normal) : vec3(0.0, 0.0, 1.0);\n\
                 v_normal = rotate_view(source_normal);\n\
                 v_tangent = rotate_view(a_tangent);\n\
                 v_binormal = rotate_view(a_binormal);\n\
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
             // Which slots this material bound, and each one's UV multiplier.\n\
             // Slots 0-3 in the `_a` vectors, slot 4 in `_b.x`.\n\
             uniform vec4 u_have_a;\n\
             uniform vec4 u_have_b;\n\
             uniform vec4 u_uv_scale_a;\n\
             uniform vec4 u_uv_scale_b;\n\
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
                     if (u_have_b.x > 0.5 && {sample}(u_tex_alpha, v_uv * u_uv_scale_b.x).a < 0.5) discard;\n\
                     if (u_have_a.x > 0.5) {{\n\
                         albedo = to_linear({sample}(u_tex_base, v_uv * u_uv_scale_a.x).rgb);\n\
                     }}\n\
                     // Halo detail maps are grey-centred and modulate: mid-grey\n\
                     // leaves the base untouched, darker and lighter push it.\n\
                     // They tile — commonly sixteen times — so the scale matters.\n\
                     if (u_have_a.y > 0.5) {{\n\
                         vec3 detail = to_linear({sample}(u_tex_detail, v_uv * u_uv_scale_a.y).rgb);\n\
                         albedo = clamp(albedo * detail * 2.0, 0.0, 1.0);\n\
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
                             vec2 sampled = {sample}(u_tex_bump, v_uv * u_uv_scale_a.z).xy * (255.0 / 127.0) - (128.0 / 127.0);\n\
                             tn = vec3(sampled, sqrt(1.0 - min(dot(sampled, sampled), 1.0)));\n\
                         }}\n\
                         // A detail normal tiles far finer than the base one\n\
                         // and adds its tangent-plane components over it —\n\
                         // the engine's stock combine — staying stable when\n\
                         // either map is flat.\n\
                         if (u_have_a.w > 0.5) {{\n\
                             vec2 dn = {sample}(u_tex_bump_detail, v_uv * u_uv_scale_a.w).xy * (255.0 / 127.0) - (128.0 / 127.0);\n\
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

struct ModelGlRenderer {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    vertex_buffer: glow::NativeBuffer,
    index_buffer: glow::NativeBuffer,
    center: glow::NativeUniformLocation,
    scale: glow::NativeUniformLocation,
    angles: glow::NativeUniformLocation,
    clip_scale: glow::NativeUniformLocation,
    depth_scale: glow::NativeUniformLocation,
    perspective: glow::NativeUniformLocation,
    base_color: glow::NativeUniformLocation,
    unlit: glow::NativeUniformLocation,
    shaded: Option<glow::NativeUniformLocation>,
    have_a: Option<glow::NativeUniformLocation>,
    have_b: Option<glow::NativeUniformLocation>,
    uv_scale_a: Option<glow::NativeUniformLocation>,
    uv_scale_b: Option<glow::NativeUniformLocation>,
    /// One sampler location per slot, in `TextureSlot` order.
    samplers: [Option<glow::NativeUniformLocation>; SLOT_COUNT],
    uploaded_geometry: Option<u64>,
    /// GL textures per material, in `RenderModelPreview::materials` order.
    materials: Vec<MaterialGlTextures>,
    /// The geometry the uploaded `materials` belong to, so a reloaded model
    /// re-uploads rather than drawing the previous model's textures.
    uploaded_textures: Option<u64>,
}

/// One material's textures on the GPU, in `TextureSlot` order.
struct MaterialGlTextures {
    textures: [Option<glow::NativeTexture>; SLOT_COUNT],
    uv_scales: [f32; SLOT_COUNT],
}

impl Default for MaterialGlTextures {
    fn default() -> Self {
        Self {
            textures: Default::default(),
            uv_scales: [1.0; SLOT_COUNT],
        }
    }
}

impl ModelGlRenderer {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        let shader_version = eframe::egui_glow::ShaderVersion::get(gl);
        let modern = shader_version.is_new_shader_interface();
        let precision = shader_version
            .is_embedded()
            .then_some("precision mediump float;\n")
            .unwrap_or("");
        let (vertex_source, fragment_source) = model_shader_sources(
            shader_version.version_declaration(),
            modern,
            precision,
        );

        unsafe {
            let vertex = compile_model_shader(gl, glow::VERTEX_SHADER, &vertex_source)?;
            let fragment = compile_model_shader(gl, glow::FRAGMENT_SHADER, &fragment_source)?;
            let program = gl.create_program().map_err(|error| error.to_string())?;
            gl.attach_shader(program, vertex);
            gl.attach_shader(program, fragment);
            gl.link_program(program);
            gl.detach_shader(program, vertex);
            gl.detach_shader(program, fragment);
            gl.delete_shader(vertex);
            gl.delete_shader(fragment);
            if !gl.get_program_link_status(program) {
                let error = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(format!("model preview shader link failed: {error}"));
            }

            let vertex_array = gl
                .create_vertex_array()
                .map_err(|error| error.to_string())?;
            let vertex_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            let index_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            gl.bind_vertex_array(Some(vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buffer));
            let stride = std::mem::size_of::<RenderModelPreviewVertex>() as i32;
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
            ] {
                if let Some(location) = gl.get_attrib_location(program, name) {
                    gl.enable_vertex_attrib_array(location);
                    gl.vertex_attrib_pointer_f32(location, size, glow::FLOAT, false, stride, offset);
                }
            }
            gl.bind_vertex_array(None);

            let uniform = |name| {
                gl.get_uniform_location(program, name)
                    .ok_or_else(|| format!("model preview shader has no {name} uniform"))
            };
            Ok(Self {
                program,
                vertex_array,
                vertex_buffer,
                index_buffer,
                center: uniform("u_center")?,
                scale: uniform("u_scale")?,
                angles: uniform("u_angles")?,
                clip_scale: uniform("u_clip_scale")?,
                depth_scale: uniform("u_depth_scale")?,
                perspective: uniform("u_perspective")?,
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
                samplers: SAMPLER_UNIFORMS
                    .map(|name| gl.get_uniform_location(program, name)),
                uploaded_geometry: None,
                materials: Vec::new(),
                uploaded_textures: None,
            })
        }
    }

    unsafe fn paint(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        unsafe {
            if self.uploaded_geometry != Some(frame.geometry_id) {
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
                self.uploaded_geometry = Some(frame.geometry_id);
            }

            self.sync_textures(gl, frame);

            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vertex_array));
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
            gl.uniform_1_f32(Some(&self.perspective), frame.camera.perspective);

            if frame.render_mode.draws_shading() {
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
                gl.uniform_1_f32(Some(&self.unlit), 0.0);
                self.draw_batches(gl, frame, false);
            }
            if frame.render_mode.draws_wireframe() {
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);
                gl.line_width(1.0);
                gl.depth_func(glow::LEQUAL);
                gl.uniform_1_f32(Some(&self.unlit), 1.0);
                self.draw_batches(gl, frame, true);
            }

            // Polygon mode is not reset by egui_glow's generic callback-state
            // restoration and would otherwise turn subsequent UI meshes into wireframes.
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.use_program(None);
        }
    }

    /// Bring the GPU's textures in line with the frame's, re-uploading only
    /// when the model changed or its textures arrived.
    unsafe fn sync_textures(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        let wanted = frame.textures.as_ref().map(|_| frame.geometry_id);
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
                for (slot, _) in SLOT_PARAMETERS {
                    let index = slot as usize;
                    uploaded.uv_scales[index] = 1.0;
                    let Some(image) = material.get(slot) else {
                        continue;
                    };
                    uploaded.uv_scales[index] = image.scale;
                    uploaded.textures[index] = unsafe { upload_texture(gl, image) };
                }
                uploaded
            })
            .collect();
    }

    unsafe fn release_textures(&mut self, gl: &glow::Context) {
        for material in self.materials.drain(..) {
            for texture in material.textures.into_iter().flatten() {
                unsafe { gl.delete_texture(texture) };
            }
        }
    }

    /// Bind one material's textures, and tell the shader which slots are live.
    ///
    /// A slot with no texture is left unbound and flagged absent rather than
    /// bound to a dummy: sampling an unbound unit is defined to return black,
    /// which a `have` flag of zero makes the shader skip entirely.
    unsafe fn bind_material(&self, gl: &glow::Context, material: Option<&MaterialGlTextures>) {
        let mut have = [0.0f32; SLOT_COUNT];
        let mut scales = [1.0f32; SLOT_COUNT];
        for (slot, _) in SLOT_PARAMETERS {
            let index = slot as usize;
            let texture = material.and_then(|material| material.textures[index]);
            have[index] = if texture.is_some() { 1.0 } else { 0.0 };
            if let Some(material) = material {
                scales[index] = material.uv_scales[index];
            }
            unsafe {
                gl.active_texture(glow::TEXTURE0 + index as u32);
                gl.bind_texture(glow::TEXTURE_2D, texture);
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
                gl.uniform_4_f32(Some(location), have[4], 0.0, 0.0, 0.0);
            }
            if let Some(location) = &self.uv_scale_a {
                gl.uniform_4_f32(Some(location), scales[0], scales[1], scales[2], scales[3]);
            }
            if let Some(location) = &self.uv_scale_b {
                gl.uniform_4_f32(Some(location), scales[4], 1.0, 1.0, 1.0);
            }
        }
    }

    unsafe fn draw_batches(&self, gl: &glow::Context, frame: &ModelGpuFrame, wire: bool) {
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
            let color = if wire {
                Color32::from_rgb(38, 55, 65)
            } else {
                batch
                    .flat_color
                    .map(|[r, g, b]| Color32::from_rgb(r, g, b))
                    .unwrap_or_else(|| material_color(batch.material_index))
            };
            // The wireframe pass draws flat on purpose, so it never binds a
            // texture — and neither does a batch whose material did not resolve.
            let material = (!wire)
                .then(|| self.materials.get(batch.material_index as usize))
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

unsafe fn compile_model_shader(
    gl: &glow::Context,
    kind: u32,
    source: &str,
) -> Result<glow::NativeShader, String> {
    unsafe {
        let shader = gl.create_shader(kind).map_err(|error| error.to_string())?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if gl.get_shader_compile_status(shader) {
            Ok(shader)
        } else {
            let error = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            Err(format!("model preview shader compile failed: {error}"))
        }
    }
}

fn model_draw_range(
    index_start: u32,
    index_count: u32,
    total_indices: usize,
) -> Option<(i32, i32)> {
    let start = index_start as usize;
    let available = total_indices.checked_sub(start)?;
    let count = (index_count as usize).min(available);
    let count = count - count % 3;
    if count == 0 || count > i32::MAX as usize {
        return None;
    }
    let byte_offset = start.checked_mul(std::mem::size_of::<u32>())?;
    Some((i32::try_from(byte_offset).ok()?, count as i32))
}

fn slice_bytes<T>(slice: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice)) }
}

#[cfg(test)]
mod gpu_renderer_tests {
    use super::*;
    use blam_tags::math::{RealPoint2d, RealPoint3d, RealVector3d};
    use blam_tags::render_model::{GeometryPartType, RenderMeshPart, RenderVertex};

    #[test]
    fn dense_indices_use_full_u32_offsets_and_counts() {
        let start = 70_002_u32;
        let count = 120_003_u32;
        assert_eq!(
            model_draw_range(start, count, 250_000),
            Some(((start * 4) as i32, count as i32))
        );
    }

    #[test]
    fn draw_range_clamps_to_complete_valid_triangles() {
        assert_eq!(model_draw_range(6, 10, 14), Some((24, 6)));
        assert_eq!(model_draw_range(20, 3, 14), None);
    }

    /// The attribute offsets in `ModelGlRenderer::new` are hand-written against
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

    #[test]
    fn both_shader_dialects_declare_what_the_renderer_binds() {
        for (declaration, modern, precision) in [
            ("#version 330\n", true, ""),
            ("#version 100\n", false, "precision mediump float;\n"),
        ] {
            let (vertex, fragment) = model_shader_sources(declaration, modern, precision);

            // Every attribute the vertex array points at, and every uniform the
            // renderer looks up, has to actually be declared.
            for name in ["a_position", "a_normal", "a_texcoord", "a_tangent", "a_binormal"] {
                assert!(vertex.contains(name), "{name} missing from vertex shader");
            }
            for name in SAMPLER_UNIFORMS {
                assert!(fragment.contains(name), "{name} missing from fragment shader");
            }
            for name in ["u_have_a", "u_have_b", "u_uv_scale_a", "u_uv_scale_b", "u_shaded"] {
                assert!(fragment.contains(name), "{name} missing from fragment shader");
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
                "u_perspective",
            ] {
                assert!(vertex.contains(name), "{name} missing from vertex shader");
            }
            for name in ["u_base_color", "u_unlit"] {
                assert!(fragment.contains(name), "{name} missing from fragment shader");
            }

            // The varyings must be declared on both sides or the link fails.
            for name in ["v_uv", "v_normal", "v_tangent", "v_binormal"] {
                assert!(vertex.contains(name) && fragment.contains(name), "{name} not on both sides");
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
                fragment.contains("u_tex_alpha, v_uv * u_uv_scale_b.x).a < 0.5) discard"),
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
        assert_eq!(std::mem::size_of::<RenderModelPreviewVertex>(), 56);
        let vertex = RenderModelPreviewVertex::default();
        let base = std::ptr::addr_of!(vertex) as usize;
        let offset = |field: usize| field - base;
        assert_eq!(offset(std::ptr::addr_of!(vertex.normal) as usize), 12);
        assert_eq!(offset(std::ptr::addr_of!(vertex.texcoord) as usize), 24);
        assert_eq!(offset(std::ptr::addr_of!(vertex.tangent) as usize), 32);
        assert_eq!(offset(std::ptr::addr_of!(vertex.binormal) as usize), 44);
    }

    #[test]
    fn indexed_preview_keeps_shared_vertices_and_part_batches() {
        let vertex = |x, y| RenderVertex {
            position: RealPoint3d { x, y, z: 0.0 },
            texcoord: RealPoint2d { x, y },
            normal: RealVector3d {
                i: 0.0,
                j: 0.0,
                k: 1.0,
            },
            tangent: RealVector3d::ZERO,
            binormal: RealVector3d::ZERO,
            node_indices: [0; 4],
            node_weights: [0.0; 4],
            lightmap_texcoord: RealPoint2d { x: 0.0, y: 0.0 },
            vert_color: RealVector3d::ZERO,
        };
        let part = |index_start| RenderMeshPart {
            material_index: index_start as u16 / 3,
            index_start,
            index_count: 3,
            part_type: GeometryPartType::OpaqueNonShadowing,
            transparent_sorting_index: -1,
            sort_position: None,
        };
        let mesh = RenderMesh {
            vertices: vec![
                vertex(0.0, 0.0),
                vertex(1.0, 0.0),
                vertex(1.0, 1.0),
                vertex(0.0, 1.0),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            parts: vec![part(0), part(3)],
            rigid_node_index: None,
            water_data: None,
            prt_vertex_type: Default::default(),
            has_prt_vertex_stream: false,
            prt_ambient_stream: Vec::new(),
            has_vertex_color: false,
            use_region_index_for_sorting: false,
            has_lightmap_uvs: false,
        };
        let mut preview = RenderModelPreview {
            bounds_min: [f32::INFINITY; 3],
            bounds_max: [f32::NEG_INFINITY; 3],
            ..Default::default()
        };

        append_render_mesh_to_preview(&mut preview, &mesh, "body", "default");

        assert_eq!(preview.vertices.len(), 4);
        assert_eq!(preview.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(preview.batches.len(), 2);
        assert_eq!(
            (
                preview.batches[0].index_start,
                preview.batches[0].index_count
            ),
            (0, 3)
        );
        assert_eq!(
            (
                preview.batches[1].index_start,
                preview.batches[1].index_count
            ),
            (3, 3)
        );
    }
}

pub(super) const MARKER_AXIS_SCREEN_LENGTH: f32 = 15.0;

fn marker_axis_screen_deltas(camera: &PreviewCamera, axes: [[f32; 3]; 3]) -> [Vec2; 3] {
    axes.map(|axis| {
        let view_axis = camera.rotate_vector(axis);
        let screen = Vec2::new(view_axis[0], -view_axis[2]);
        let len = screen.length();
        if len <= 0.001 {
            Vec2::new(0.0, -MARKER_AXIS_SCREEN_LENGTH * 0.45)
        } else {
            screen / len * MARKER_AXIS_SCREEN_LENGTH
        }
    })
}

pub(super) fn draw_marker_axes(
    painter: &egui::Painter,
    origin: egui::Pos2,
    axis_deltas: [Vec2; 3],
) {
    let colors = [
        Color32::from_rgb(220, 35, 28),
        Color32::from_rgb(20, 180, 45),
        Color32::from_rgb(40, 85, 235),
    ];
    for (delta, color) in axis_deltas.into_iter().zip(colors) {
        let end = origin + delta;
        painter.line_segment(
            [origin, end],
            Stroke::new(2.5, Color32::from_rgba_unmultiplied(0, 0, 0, 150)),
        );
        painter.line_segment([origin, end], Stroke::new(1.35, color));
    }
}

pub(super) fn marker_axes_hovered(
    pos: egui::Pos2,
    origin: egui::Pos2,
    axis_deltas: [Vec2; 3],
) -> bool {
    screen_edge_length(pos, origin) <= 7.0
        || axis_deltas
            .into_iter()
            .any(|delta| point_segment_distance(pos, origin, origin + delta) <= 5.0)
}

pub(super) fn point_segment_distance(point: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let ap = point - a;
    let denom = ab.dot(ab);
    if denom <= f32::EPSILON {
        return screen_edge_length(point, a);
    }
    let t = (ap.dot(ab) / denom).clamp(0.0, 1.0);
    screen_edge_length(point, a + ab * t)
}

pub(super) fn screen_edge_length(a: egui::Pos2, b: egui::Pos2) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
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

struct ProjectedPoint {
    pos: egui::Pos2,
}

/// The camera's zoom bounds. Wide on purpose: 5× was plenty for a vehicle but
/// nothing on a structure BSP, where inspecting a doorway on a whole level
/// needs a couple hundred times the fitted view.
pub(in crate::app) const MIN_PREVIEW_SCALE: f32 = 0.02;
pub(in crate::app) const MAX_PREVIEW_SCALE: f32 = 500.0;

/// A preview's bounds center and bounding-sphere radius — the fit every
/// camera computation derives from.
fn preview_center_radius(preview: &RenderModelPreview) -> ([f32; 3], f32) {
    let (min, max) = (preview.bounds_min, preview.bounds_max);
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let extent = [
        (max[0] - min[0]).abs(),
        (max[1] - min[1]).abs(),
        (max[2] - min[2]).abs(),
    ];
    let radius =
        ((extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt() * 0.5)
            .max(0.001);
    (center, radius)
}

/// World → view: yaw about Z, then pitch about X — the one rotation this
/// preview uses, mirrored in the vertex shader's `rotate_view`.
fn rotate_view_vector(yaw: f32, pitch: f32, vector: [f32; 3]) -> [f32; 3] {
    let (sy, cy) = yaw.sin_cos();
    let x = vector[0] * cy - vector[1] * sy;
    let y = vector[0] * sy + vector[1] * cy;
    let (sp, cp) = pitch.sin_cos();
    [x, y * cp - vector[2] * sp, y * sp + vector[2] * cp]
}

/// View → world: the exact inverse of [`rotate_view_vector`] (un-pitch, then
/// un-yaw). What turns a screen-space pan or zoom-to-cursor offset back into
/// the world-space focus move it stands for.
pub(super) fn unrotate_view_vector(yaw: f32, pitch: f32, vector: [f32; 3]) -> [f32; 3] {
    let (sp, cp) = pitch.sin_cos();
    let y = vector[1] * cp + vector[2] * sp;
    let z = -vector[1] * sp + vector[2] * cp;
    let (sy, cy) = yaw.sin_cos();
    [vector[0] * cy + y * sy, -vector[0] * sy + y * cy, z]
}

struct PreviewCamera {
    rect: egui::Rect,
    /// Orbit point: bounds center plus the user's world-space focus offset.
    center: [f32; 3],
    /// Fit radius — deliberately the *bounds* radius, unaffected by focus, so
    /// the zoom slider keeps one meaning wherever the user pans.
    radius: f32,
    /// Depth-normalization radius: the bounds radius grown by the focus
    /// offset, so panned-to geometry can never fall outside the depth range.
    depth_radius: f32,
    yaw: f32,
    pitch: f32,
    scale: f32,
    perspective: bool,
}

impl PreviewCamera {
    fn new(data: &ModelPreviewData, state: &ModelPreviewState, rect: egui::Rect) -> Self {
        let (bounds_center, radius) = preview_center_radius(&data.preview);
        let center = [
            bounds_center[0] + state.focus[0],
            bounds_center[1] + state.focus[1],
            bounds_center[2] + state.focus[2],
        ];
        let focus_length = (state.focus[0] * state.focus[0]
            + state.focus[1] * state.focus[1]
            + state.focus[2] * state.focus[2])
            .sqrt();
        Self {
            rect,
            center,
            radius,
            depth_radius: radius + focus_length,
            yaw: state.yaw,
            pitch: state.pitch,
            scale: state.scale.clamp(MIN_PREVIEW_SCALE, MAX_PREVIEW_SCALE),
            perspective: state.perspective,
        }
    }

    fn project(&self, point: [f32; 3]) -> ProjectedPoint {
        let x = (point[0] - self.center[0]) * self.scale;
        let y = (point[1] - self.center[1]) * self.scale;
        let z = (point[2] - self.center[2]) * self.scale;
        let rotated = self.rotate_vector([x, y, z]);
        let fit = self.rect.width().min(self.rect.height()) / (self.radius * 2.2).max(0.001);
        // The same divide the vertex shader applies, so markers stay glued to
        // their geometry in either projection.
        let w = if self.perspective {
            let depth_scale = 1.0 / (self.depth_radius * self.scale * 1.05).max(0.001);
            (1.0 + rotated[1] * depth_scale * PERSPECTIVE_R_OVER_D).max(0.05)
        } else {
            1.0
        };
        let screen =
            self.rect.center() + Vec2::new(rotated[0] * fit / w, -rotated[2] * fit / w);
        ProjectedPoint { pos: screen }
    }

    fn gpu_uniforms(&self) -> ModelGpuCamera {
        let width = self.rect.width().max(1.0);
        let height = self.rect.height().max(1.0);
        let fit = width.min(height) / (self.radius * 2.2).max(0.001);
        ModelGpuCamera {
            center: self.center,
            scale: self.scale,
            yaw: self.yaw,
            pitch: self.pitch,
            clip_scale: [2.0 * fit / width, 2.0 * fit / height],
            // Orthographic projection: smaller view-space Y is nearer. The
            // focus-grown radius safely maps all depths inside NDC even when
            // the orbit point sits far from the bounds center.
            depth_scale: 1.0 / (self.depth_radius * self.scale * 1.05).max(0.001),
            perspective: if self.perspective { 1.0 } else { 0.0 },
        }
    }

    fn rotate_vector(&self, vector: [f32; 3]) -> [f32; 3] {
        rotate_view_vector(self.yaw, self.pitch, vector)
    }
}

/// Build flat preview geometry with draw batches grouped by region and
/// permutation. Ported from blam-tags so the GUI owns its preview type; the
/// render meshes are derived separately via `RenderModel::derive_render_meshes`.
pub(super) fn render_model_to_preview(
    model: &RenderModel,
    render_meshes: &[RenderMesh],
) -> RenderModelPreview {
    let node_world = preview_node_world_transforms(&model.nodes);
    let mut preview = RenderModelPreview {
        regions: model
            .regions
            .iter()
            .map(|region| RenderModelPreviewRegion {
                name: region.name.clone(),
                permutations: region
                    .permutations
                    .iter()
                    .map(|permutation| permutation.name.clone())
                    .collect(),
            })
            .collect(),
        // Index-aligned with `RenderMeshPart::material_index`, which is what
        // the batches carry, so a batch's shader is `materials[index]`.
        materials: model
            .materials
            .iter()
            .map(|material| RenderModelPreviewMaterial {
                shader_path: material.render_method.clone(),
                shader_group: material.render_method_group,
            })
            .collect(),
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };

    for region in &model.regions {
        for permutation in &region.permutations {
            let first_mesh = permutation.mesh_index.max(0) as usize;
            let mesh_count = permutation.mesh_count.max(0) as usize;
            for mesh_index in first_mesh..first_mesh.saturating_add(mesh_count) {
                let Some(mesh) = render_meshes.get(mesh_index) else {
                    continue;
                };
                append_render_mesh_to_preview(&mut preview, mesh, &region.name, &permutation.name);
            }
        }
    }

    if preview.vertices.is_empty() {
        preview.bounds_min = [0.0; 3];
        preview.bounds_max = [0.0; 3];
    }

    for group in &model.marker_groups {
        for marker in &group.markers {
            preview.markers.push(RenderModelPreviewMarker {
                name: group.name.clone(),
                position: transform_preview_marker_position(marker, &node_world),
                axes: transform_preview_marker_axes(marker, &node_world),
            });
        }
    }

    preview
}

/// Append one render mesh without flattening every triangle into three unique
/// vertices. The GPU renderer consumes indexed `u32` geometry directly, so
/// retaining the source topology is both faithful and dramatically smaller for
/// dense Campaign Evolved Nanite previews.
fn append_render_mesh_to_preview(
    preview: &mut RenderModelPreview,
    mesh: &RenderMesh,
    region_name: &str,
    permutation_name: &str,
) {
    let Ok(vertex_base) = u32::try_from(preview.vertices.len()) else {
        return;
    };
    if mesh
        .vertices
        .len()
        .checked_add(preview.vertices.len())
        .is_none_or(|count| count > u32::MAX as usize)
    {
        return;
    }

    preview.vertices.reserve(mesh.vertices.len());
    for vertex in &mesh.vertices {
        let position = point3_to_array(vertex.position);
        expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
        preview.vertices.push(RenderModelPreviewVertex {
            position,
            normal: vector3_to_array(vertex.normal),
            // Already decompressed: `derive_render_meshes` applies the mesh's
            // compression bounds, so these are real UVs rather than the packed
            // [0,1] the buffer stores for the UShort2N formats.
            texcoord: [vertex.texcoord.x, vertex.texcoord.y],
            tangent: vector3_to_array(vertex.tangent),
            binormal: vector3_to_array(vertex.binormal),
        });
    }

    for part in &mesh.parts {
        let source_start = part.index_start as usize;
        let source_end = source_start
            .saturating_add(part.index_count as usize)
            .min(mesh.indices.len());
        let Some(source_indices) = mesh.indices.get(source_start..source_end) else {
            continue;
        };
        let Ok(index_start) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        for triangle in source_indices.chunks_exact(3) {
            let convert = |index: u32| {
                ((index as usize) < mesh.vertices.len())
                    .then(|| vertex_base.checked_add(index))
                    .flatten()
            };
            if let (Some(a), Some(b), Some(c)) = (
                convert(triangle[0]),
                convert(triangle[1]),
                convert(triangle[2]),
            ) {
                preview.indices.extend_from_slice(&[a, b, c]);
            }
        }
        let Ok(index_end) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        let index_count = index_end - index_start;
        if index_count > 0 {
            preview.batches.push(RenderModelPreviewBatch {
                region_name: region_name.to_owned(),
                permutation_name: permutation_name.to_owned(),
                material_index: part.material_index,
                index_start,
                index_count,
                flat_color: None,
            });
        }
    }
}

pub(super) fn preview_node_world_transforms(nodes: &[Node]) -> Vec<(RealQuaternion, RealPoint3d)> {
    let mut world: Vec<(RealQuaternion, RealPoint3d)> = Vec::with_capacity(nodes.len());
    for node in nodes {
        let local_rot = node.default_rotation.normalized();
        let local_trans = node.default_translation;
        if node.parent_node >= 0
            && let Some((parent_rot, parent_trans)) = world.get(node.parent_node as usize).copied()
        {
            let rot = (parent_rot * local_rot).normalized();
            let trans = parent_trans + (parent_rot * local_trans.as_vector());
            world.push((rot, trans));
            continue;
        }
        world.push((local_rot, local_trans));
    }
    world
}

pub(super) fn transform_preview_marker_position(
    marker: &Marker,
    node_world: &[(RealQuaternion, RealPoint3d)],
) -> [f32; 3] {
    let local = marker.translation;
    let world = if marker.node_index >= 0 {
        node_world
            .get(marker.node_index as usize)
            .map(|(rot, trans)| *trans + (*rot * local.as_vector()))
            .unwrap_or(local)
    } else {
        local
    };
    point3_to_array(world)
}

pub(super) fn transform_preview_marker_axes(
    marker: &Marker,
    node_world: &[(RealQuaternion, RealPoint3d)],
) -> [[f32; 3]; 3] {
    let local_rot = marker.rotation.normalized();
    let world_rot = if marker.node_index >= 0 {
        node_world
            .get(marker.node_index as usize)
            .map(|(rot, _)| (*rot * local_rot).normalized())
            .unwrap_or(local_rot)
    } else {
        local_rot
    };
    [
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 1.0,
                    j: 0.0,
                    k: 0.0,
                },
        ),
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 0.0,
                    j: 1.0,
                    k: 0.0,
                },
        ),
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 0.0,
                    j: 0.0,
                    k: 1.0,
                },
        ),
    ]
}

pub(super) fn point3_to_array(p: RealPoint3d) -> [f32; 3] {
    [p.x, p.y, p.z]
}

pub(super) fn vector3_to_array(v: RealVector3d) -> [f32; 3] {
    [v.i, v.j, v.k]
}


/// Upload one decoded texture, with mipmaps and the wrap modes the shader
/// authored.
///
/// Mipmaps matter here beyond quality: a detail map tiling twenty times across
/// a model aliases into noise without them, which is exactly where a preview
/// looks worst.
unsafe fn upload_texture(gl: &glow::Context, image: &TextureImage) -> Option<glow::NativeTexture> {
    if image.width == 0 || image.height == 0 {
        return None;
    }
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            image.width as i32,
            image.height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            Some(&image.rgba),
        );
        gl.generate_mipmap(glow::TEXTURE_2D);
        let wrap = |repeat: bool| {
            if repeat {
                glow::REPEAT as i32
            } else {
                glow::CLAMP_TO_EDGE as i32
            }
        };
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap(image.repeat_x));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap(image.repeat_y));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR_MIPMAP_LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(texture)
    }
}
