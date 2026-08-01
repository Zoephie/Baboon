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

    if response.dragged_by(egui::PointerButton::Middle) {
        state.pan += response.drag_delta();
    } else if response.dragged_by(egui::PointerButton::Primary) {
        let delta = response.drag_delta();
        if ui.input(|i| i.modifiers.shift) {
            state.pan += delta;
        } else {
            state.yaw += delta.x * 0.01;
            state.pitch = (state.pitch + delta.y * 0.01).clamp(-1.45, 1.45);
        }
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() > f32::EPSILON {
            state.scale = (state.scale * (scroll / 450.0).exp()).clamp(0.05, 5.0);
        }
    }

    let camera = PreviewCamera::new(data, state, rect);
    let preview = Arc::clone(&data.preview);
    let visible_batches = preview
        .batches
        .iter()
        .enumerate()
        .filter_map(|(index, batch)| {
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
    pan_clip: [f32; 2],
    depth_scale: f32,
}

struct ModelGpuFrame {
    preview: Arc<RenderModelPreview>,
    geometry_id: u64,
    visible_batches: Vec<usize>,
    camera: ModelGpuCamera,
    render_mode: ModelRenderMode,
    show_backfaces: bool,
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

struct ModelGlRenderer {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    vertex_buffer: glow::NativeBuffer,
    index_buffer: glow::NativeBuffer,
    center: glow::NativeUniformLocation,
    scale: glow::NativeUniformLocation,
    angles: glow::NativeUniformLocation,
    clip_scale: glow::NativeUniformLocation,
    pan_clip: glow::NativeUniformLocation,
    depth_scale: glow::NativeUniformLocation,
    base_color: glow::NativeUniformLocation,
    unlit: glow::NativeUniformLocation,
    uploaded_geometry: Option<u64>,
}

impl ModelGlRenderer {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        let shader_version = eframe::egui_glow::ShaderVersion::get(gl);
        let modern = shader_version.is_new_shader_interface();
        let precision = shader_version
            .is_embedded()
            .then_some("precision mediump float;\n")
            .unwrap_or("");
        let (attribute, varying_out, varying_in, fragment_output, output_name) = if modern {
            ("in", "out", "in", "out vec4 out_color;", "out_color")
        } else {
            ("attribute", "varying", "varying", "", "gl_FragColor")
        };
        let vertex_source = format!(
            "{}{precision}\
             {attribute} vec3 a_position;\n\
             {attribute} vec3 a_normal;\n\
             uniform vec3 u_center;\n\
             uniform float u_scale;\n\
             uniform vec2 u_angles;\n\
             uniform vec2 u_clip_scale;\n\
             uniform vec2 u_pan_clip;\n\
             uniform float u_depth_scale;\n\
             uniform vec3 u_base_color;\n\
             uniform float u_unlit;\n\
             {varying_out} vec3 v_color;\n\
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
                 gl_Position = vec4(view.x * u_clip_scale.x + u_pan_clip.x, view.z * u_clip_scale.y + u_pan_clip.y, view.y * u_depth_scale, 1.0);\n\
                 vec3 source_normal = length(a_normal) > 0.0001 ? normalize(a_normal) : vec3(0.0, 0.0, 1.0);\n\
                 vec3 normal_view = normalize(rotate_view(source_normal));\n\
                 float key = max(dot(normal_view, normalize(vec3(-0.35, -0.55, 0.76))), 0.0);\n\
                 float fill = max(dot(normal_view, normalize(vec3(0.72, 0.22, 0.36))), 0.0);\n\
                 float rim = pow(clamp(1.0 - abs(normal_view.y), 0.0, 1.0), 2.0);\n\
                 float overhead = clamp(normal_view.z * 0.5 + 0.5, 0.0, 1.0);\n\
                 float shade = clamp(0.42 + key * 0.46 + fill * 0.16 + rim * 0.10 + overhead * 0.08, 0.32, 1.22);\n\
                 vec3 shaded = clamp(u_base_color * shade + vec3(key * key * (22.0 / 255.0)), 0.0, 1.0);\n\
                 v_color = mix(shaded, u_base_color, u_unlit);\n\
             }}\n",
            shader_version.version_declaration()
        );
        let fragment_source = format!(
            "{}{precision}\
             {varying_in} vec3 v_color;\n\
             {fragment_output}\n\
             void main() {{ {output_name} = vec4(v_color, 1.0); }}\n",
            shader_version.version_declaration()
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
                pan_clip: uniform("u_pan_clip")?,
                depth_scale: uniform("u_depth_scale")?,
                base_color: uniform("u_base_color")?,
                unlit: uniform("u_unlit")?,
                uploaded_geometry: None,
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

            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vertex_array));
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
            gl.uniform_2_f32(
                Some(&self.pan_clip),
                frame.camera.pan_clip[0],
                frame.camera.pan_clip[1],
            );
            gl.uniform_1_f32(Some(&self.depth_scale), frame.camera.depth_scale);

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
                material_color(batch.material_index)
            };
            unsafe {
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

    #[test]
    fn gpu_vertex_layout_is_two_tightly_packed_vec3_values() {
        assert_eq!(std::mem::size_of::<RenderModelPreviewVertex>(), 24);
        let vertex = RenderModelPreviewVertex::default();
        let base = std::ptr::addr_of!(vertex) as usize;
        let normal = std::ptr::addr_of!(vertex.normal) as usize;
        assert_eq!(normal - base, 12);
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

pub(super) fn material_color(index: u16) -> Color32 {
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

struct PreviewCamera {
    rect: egui::Rect,
    center: [f32; 3],
    radius: f32,
    yaw: f32,
    pitch: f32,
    scale: f32,
    pan: Vec2,
}

impl PreviewCamera {
    fn new(data: &ModelPreviewData, state: &ModelPreviewState, rect: egui::Rect) -> Self {
        let min = data.preview.bounds_min;
        let max = data.preview.bounds_max;
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
        Self {
            rect,
            center,
            radius,
            yaw: state.yaw,
            pitch: state.pitch,
            scale: state.scale,
            pan: state.pan,
        }
    }

    fn project(&self, point: [f32; 3]) -> ProjectedPoint {
        let mut x = (point[0] - self.center[0]) * self.scale;
        let y = (point[1] - self.center[1]) * self.scale;
        let mut z = (point[2] - self.center[2]) * self.scale;
        let rotated = self.rotate_vector([x, y, z]);
        x = rotated[0];
        z = rotated[2];
        let fit = self.rect.width().min(self.rect.height()) / (self.radius * 2.2).max(0.001);
        let screen = self.rect.center() + self.pan + Vec2::new(x * fit, -z * fit);
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
            pan_clip: [2.0 * self.pan.x / width, -2.0 * self.pan.y / height],
            // Orthographic projection: smaller view-space Y is nearer. The
            // model's bounding-sphere radius safely maps all depths inside NDC.
            depth_scale: 1.0 / (self.radius * self.scale * 1.05).max(0.001),
        }
    }

    fn rotate_vector(&self, vector: [f32; 3]) -> [f32; 3] {
        let mut x = vector[0];
        let mut y = vector[1];
        let mut z = vector[2];
        let (sy, cy) = self.yaw.sin_cos();
        let yaw_x = x * cy - y * sy;
        let yaw_y = x * sy + y * cy;
        x = yaw_x;
        y = yaw_y;
        let (sp, cp) = self.pitch.sin_cos();
        let pitch_y = y * cp - z * sp;
        let pitch_z = y * sp + z * cp;
        y = pitch_y;
        z = pitch_z;
        [x, y, z]
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
