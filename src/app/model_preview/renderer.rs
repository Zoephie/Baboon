//! Software projection, shading, markers, and preview geometry conversion.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;

pub(super) fn draw_model_viewport(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    desired_size: Vec2,
) {
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_rgb(228, 238, 244));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));

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
    collect_visible_triangles_into(
        data,
        &state.region_selections,
        state.show_backfaces,
        &camera,
        &mut state.projected_triangles,
    );
    state
        .projected_triangles
        .sort_by(|a, b| b.depth.total_cmp(&a.depth));

    if state.render_mode.draws_shading() {
        let mut mesh = egui::epaint::Mesh::default();
        mesh.vertices.reserve(state.projected_triangles.len() * 3);
        mesh.indices.reserve(state.projected_triangles.len() * 3);
        for tri in &state.projected_triangles {
            let start = mesh.vertices.len() as u32;
            for (point, fill) in tri.points.into_iter().zip(tri.fills) {
                mesh.colored_vertex(point, fill);
            }
            mesh.add_triangle(start, start + 1, start + 2);
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    let wire_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(20, 35, 45, 110));
    let wire_edge_limit = camera.screen_radius() * 0.55;
    if state.render_mode.draws_wireframe() {
        for tri in &state.projected_triangles {
            draw_wireframe_edges(&painter, tri.points, wire_edge_limit, wire_stroke);
        }
    }

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

fn collect_visible_triangles_into(
    data: &ModelPreviewData,
    region_selections: &HashMap<String, ModelRegionSelection>,
    show_backfaces: bool,
    camera: &PreviewCamera,
    out: &mut Vec<ModelProjectedTriangle>,
) {
    out.clear();
    out.reserve(data.draw_triangles.len());
    for triangle in &data.draw_triangles {
        let Some(batch) = data.preview.batches.get(triangle.batch_index) else {
            continue;
        };
        let Some(selection) = region_selections.get(&batch.region_name) else {
            continue;
        };
        if !selection.enabled || selection.permutation != batch.permutation_name {
            continue;
        }
        let pa = camera.project(triangle.positions[0]);
        let pb = camera.project(triangle.positions[1]);
        let pc = camera.project(triangle.positions[2]);
        if !show_backfaces && projected_signed_area(pa.pos, pb.pos, pc.pos) >= -0.25 {
            continue;
        }
        if projected_max_edge(pa.pos, pb.pos, pc.pos) > camera.screen_radius() * 0.9 {
            continue;
        }
        if !camera.rect.intersects(egui::Rect::from_min_max(
            egui::pos2(
                pa.pos.x.min(pb.pos.x).min(pc.pos.x),
                pa.pos.y.min(pb.pos.y).min(pc.pos.y),
            ),
            egui::pos2(
                pa.pos.x.max(pb.pos.x).max(pc.pos.x),
                pa.pos.y.max(pb.pos.y).max(pc.pos.y),
            ),
        )) {
            continue;
        }
        out.push(ModelProjectedTriangle {
            points: [pa.pos, pb.pos, pc.pos],
            depth: (pa.depth + pb.depth + pc.depth) / 3.0,
            fills: [
                shade_model_color(triangle.fill, camera.rotate_vector(triangle.normals[0])),
                shade_model_color(triangle.fill, camera.rotate_vector(triangle.normals[1])),
                shade_model_color(triangle.fill, camera.rotate_vector(triangle.normals[2])),
            ],
        });
    }
}

pub(super) fn draw_wireframe_edges(
    painter: &egui::Painter,
    points: [egui::Pos2; 3],
    max_edge: f32,
    stroke: Stroke,
) {
    for (a, b) in [
        (points[0], points[1]),
        (points[1], points[2]),
        (points[2], points[0]),
    ] {
        if screen_edge_length(a, b) <= max_edge {
            painter.line_segment([a, b], stroke);
        }
    }
}

pub(super) fn projected_signed_area(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

pub(super) fn projected_max_edge(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> f32 {
    screen_edge_length(a, b)
        .max(screen_edge_length(b, c))
        .max(screen_edge_length(c, a))
}

pub(super) fn screen_edge_length(a: egui::Pos2, b: egui::Pos2) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

pub(super) fn preview_edge_limit(min: [f32; 3], max: [f32; 3]) -> f32 {
    let dx = max[0] - min[0];
    let dy = max[1] - min[1];
    let dz = max[2] - min[2];
    let diagonal = (dx * dx + dy * dy + dz * dz).sqrt().max(0.001);
    diagonal * 0.45
}

pub(super) fn triangle_max_edge(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    edge_length(a, b)
        .max(edge_length(b, c))
        .max(edge_length(c, a))
}

pub(super) fn edge_length(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

pub(super) fn build_model_source_triangles(
    preview: &RenderModelPreview,
    max_preview_edge: f32,
) -> Vec<ModelSourceTriangle> {
    let mut out = Vec::with_capacity(preview.indices.len() / 3);
    for (batch_index, batch) in preview.batches.iter().enumerate() {
        let start = batch.index_start as usize;
        let end = start
            .saturating_add(batch.index_count as usize)
            .min(preview.indices.len());
        let fill = material_color(batch.material_index);
        for chunk in preview.indices[start..end].chunks_exact(3) {
            let Some(a) = preview.vertices.get(chunk[0] as usize) else {
                continue;
            };
            let Some(b) = preview.vertices.get(chunk[1] as usize) else {
                continue;
            };
            let Some(c) = preview.vertices.get(chunk[2] as usize) else {
                continue;
            };
            let [pa, pb, pc] = [a.position, b.position, c.position];
            let max_edge = triangle_max_edge(pa, pb, pc);
            if max_edge > max_preview_edge {
                continue;
            }
            let face_normal = triangle_normal(pa, pb, pc);
            out.push(ModelSourceTriangle {
                batch_index,
                positions: [pa, pb, pc],
                normals: [
                    usable_normal_or(a.normal, face_normal),
                    usable_normal_or(b.normal, face_normal),
                    usable_normal_or(c.normal, face_normal),
                ],
                fill,
            });
        }
    }
    out
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

pub(super) fn shade_model_color(base: Color32, normal_view: [f32; 3]) -> Color32 {
    let normal = normalize3(normal_view);
    let key = dot3(normal, normalize3([-0.35, -0.55, 0.76])).max(0.0);
    let fill = dot3(normal, normalize3([0.72, 0.22, 0.36])).max(0.0);
    let rim = (1.0 - normal[1].abs()).clamp(0.0, 1.0).powi(2);
    let overhead = (normal[2] * 0.5 + 0.5).clamp(0.0, 1.0);
    let shade = (0.42 + key * 0.46 + fill * 0.16 + rim * 0.10 + overhead * 0.08).clamp(0.32, 1.22);
    let highlight = (key * key * 22.0).clamp(0.0, 24.0);
    Color32::from_rgb(
        shade_channel(base.r(), shade, highlight),
        shade_channel(base.g(), shade, highlight),
        shade_channel(base.b(), shade, highlight),
    )
}

pub(super) fn shade_channel(value: u8, shade: f32, highlight: f32) -> u8 {
    ((value as f32 * shade + highlight).round()).clamp(0.0, 255.0) as u8
}

pub(super) fn usable_normal_or(normal: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    if length_squared3(normal) <= 0.0001 {
        return fallback;
    }
    let normalized = normalize3(normal);
    if length_squared3(normalized) > 0.25 {
        normalized
    } else {
        fallback
    }
}

pub(super) fn triangle_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    normalize3(cross3(sub3(b, a), sub3(c, a)))
}

pub(super) fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = length_squared3(v).sqrt();
    if len <= f32::EPSILON {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

pub(super) fn length_squared3(v: [f32; 3]) -> f32 {
    dot3(v, v)
}

pub(super) fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(super) fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

struct ProjectedPoint {
    pos: egui::Pos2,
    depth: f32,
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
        let mut y = (point[1] - self.center[1]) * self.scale;
        let mut z = (point[2] - self.center[2]) * self.scale;
        let rotated = self.rotate_vector([x, y, z]);
        x = rotated[0];
        y = rotated[1];
        z = rotated[2];
        let fit = self.rect.width().min(self.rect.height()) / (self.radius * 2.2).max(0.001);
        let screen = self.rect.center() + self.pan + Vec2::new(x * fit, -z * fit);
        ProjectedPoint {
            pos: screen,
            depth: y,
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

    fn screen_radius(&self) -> f32 {
        let fit = self.rect.width().min(self.rect.height()) / (self.radius * 2.2).max(0.001);
        self.radius * self.scale * fit
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
                for part in &mesh.parts {
                    let index_start = preview.indices.len() as u32;
                    for source_index in
                        part.index_start..part.index_start.saturating_add(part.index_count)
                    {
                        let Some(&vertex_index) = mesh.indices.get(source_index as usize) else {
                            continue;
                        };
                        let Some(vertex) = mesh.vertices.get(vertex_index as usize) else {
                            continue;
                        };
                        let position = point3_to_array(vertex.position);
                        let normal = vector3_to_array(vertex.normal);
                        expand_preview_bounds_local(
                            &mut preview.bounds_min,
                            &mut preview.bounds_max,
                            position,
                        );
                        let new_index = preview.vertices.len() as u32;
                        preview
                            .vertices
                            .push(RenderModelPreviewVertex { position, normal });
                        preview.indices.push(new_index);
                    }
                    let index_count = preview.indices.len() as u32 - index_start;
                    if index_count > 0 {
                        preview.batches.push(RenderModelPreviewBatch {
                            region_name: region.name.clone(),
                            permutation_name: permutation.name.clone(),
                            material_index: part.material_index,
                            index_start,
                            index_count,
                        });
                    }
                }
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
