//! Halo CE's `flag` widget: a sheet of points hung from the object's
//! markers, each moved by `point_physics_update` and pulled back to its
//! neighbours, drawn as a mesh of its own with the red or blue shader. Read
//! from the Xbox 360 build's symbols: `flag_new`, `flag_set_attachment_shape`,
//! `flag_set_trailing_shape`, `flag_tesselate_region`, `flag_update_attachment`,
//! `flag_update`, `flags_update`, `flag_render` and `flag_render_proper`.

use blam_tags::{TagFile, TagStruct};

use super::attachments::{elements, object_block};
use super::lights::posed_on;
use super::particles::CePointPhysics;
use super::super::*;
use crate::core::source::TagSource;

/// The time `flag_render` gives the update that settles a flag first.
const SETTLE: f32 = 5.0;
/// The radius `flag_update` gives each point.
const POINT_RADIUS: f32 = 0.02;
/// The neighbours each point is pulled toward: left, below and above.
const NEIGHBOURS: [(isize, isize); 3] = [(-1, 0), (0, 1), (0, -1)];

/// One of the tag's `attachment points`.
#[derive(Debug, Clone, Copy)]
struct FlagAttachment {
    height_to_next: i32,
    node: usize,
    position: [f32; 3],
}

/// A `flag` on the opened object, with the vertices it adds to the model.
#[derive(Debug, Clone)]
pub(crate) struct CeFlagAt {
    width: usize,
    height: usize,
    cell: [f32; 2],
    physics: CePointPhysics,
    wind_noise: f32,
    attachments: Vec<FlagAttachment>,
    /// The first of its vertices in the preview's.
    first_vertex: usize,
}

fn read_flag(
    source: &TagSource,
    path: &str,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    preview: &mut RenderModelPreview,
) -> Option<CeFlagAt> {
    let tag = load_referenced_tag_from_source(source, path, "flag", b"flag").ok()?;
    let root = tag.root();
    let width = root.read_int_any("width").unwrap_or(0);
    let height = root.read_int_any("height").unwrap_or(0);
    // `flag_new`: one too big, or with no blue shader, does nothing.
    let blue = root.read_tag_ref_with_group("blue flag shader").filter(|(_, path)| !path.is_empty());
    // Fewer than two points a side would have no cells; the game divides by zero.
    if width * height >= 225 || !(2..40).contains(&width) || height < 2 {
        return None;
    }
    let blue = blue?;
    // `flag_render_proper`: the red shader for an object of team 0, else
    // blue; the preview's object has no team.
    let (shader_group, shader_path) =
        root.read_tag_ref_with_group("red flag shader").filter(|(_, path)| !path.is_empty()).unwrap_or(blue);
    let physics_path = root.read_tag_ref_path("physics").filter(|path| !path.is_empty())?;
    let physics = load_referenced_tag_from_source(source, &physics_path, "point_physics", b"pphy").ok()?;
    let attachments: Vec<FlagAttachment> = elements(root, "attachment points")
        .iter()
        .map(|point| {
            let name = point.read_string("marker name").unwrap_or_default();
            // `object_get_marker_by_name`: what is not found is the origin.
            let (node, position) = markers
                .iter()
                .find(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(&name))
                .map_or((0, [0.0; 3]), |marker| (marker.node_index.max(0) as usize, marker.position));
            FlagAttachment {
                height_to_next: point.read_int_any("height_to_next_attachment").unwrap_or(0) as i32,
                node,
                position,
            }
        })
        .collect();
    if attachments.is_empty() {
        return None;
    }
    let (width, height) = (width as usize, height as usize);
    let cells = cell_shapes(root, width, height, &attachments);
    let first_vertex = preview.vertices.len();
    let index_start = preview.indices.len() as u32;
    // `flag_render_proper`: u across the width, v down the height.
    preview.vertices.extend((0..width).flat_map(|x| {
        (0..height).map(move |y| RenderModelPreviewVertex {
            texcoord: [x as f32 / (width - 1) as f32, y as f32 / (height - 1) as f32],
            ..Default::default()
        })
    }));
    let at = |x: usize, y: usize| (first_vertex + x * height + y) as u32;
    for x in 0..width - 1 {
        for y in 0..height - 1 {
            let triangles: &[[(usize, usize); 3]] = match cells[x * (height - 1) + y] {
                0 => &[[(x, y), (x + 1, y), (x, y + 1)], [(x, y + 1), (x + 1, y), (x + 1, y + 1)]],
                2 => &[[(x, y), (x, y + 1), (x + 1, y)]],
                3 => &[[(x, y), (x, y + 1), (x + 1, y + 1)]],
                4 => &[[(x, y), (x + 1, y + 1), (x + 1, y)]],
                5 => &[[(x, y + 1), (x + 1, y + 1), (x + 1, y)]],
                _ => &[],
            };
            for triangle in triangles {
                preview.indices.extend(triangle.iter().map(|&(x, y)| at(x, y)));
            }
        }
    }
    preview.materials.push(RenderModelPreviewMaterial { shader_path, shader_group, object: owner, ..Default::default() });
    let name = path.rsplit('\\').next().unwrap_or(path).to_owned();
    let permutation = "flag".to_owned();
    preview.regions.push(RenderModelPreviewRegion { name: name.clone(), permutations: vec![permutation.clone()] });
    preview.batches.push(RenderModelPreviewBatch {
        region_name: name,
        permutation_name: permutation,
        material_index: (preview.materials.len() - 1) as u16,
        index_start,
        index_count: preview.indices.len() as u32 - index_start,
        flat_color: None,
        layer: ModelPreviewLayer::Render,
        sort_position: None,
    });
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    Some(CeFlagAt {
        width,
        height,
        cell: [real("cell width"), real("cell height")],
        physics: CePointPhysics::read(&physics),
        wind_noise: real("wind noise"),
        attachments,
        first_vertex,
    })
}

/// `flag_tesselate_region`: across a square of cells, the diagonal takes
/// `shape`, the cells past it none, and the rest stay whole.
fn tesselate_region(cells: &mut [i32], width: usize, height: usize, x: i32, y: i32, size: i32, shape: i32) {
    for at_x in x..x + size {
        for at_y in y..y + size {
            if at_x < 0 || at_y < 0 || at_x >= width as i32 - 1 || at_y >= height as i32 - 1 {
                continue;
            }
            let across = if shape == 4 || shape == 5 { at_x - x } else { size - at_x + x - 1 };
            let down = if shape == 4 || shape == 2 { at_y - y } else { y - at_y + size - 1 };
            cells[at_x as usize * (height - 1) + at_y as usize] = if across == down { shape } else { i32::from(down > across) };
        }
    }
}

/// Each cell's shape: 0 two triangles, 1 none, 2 to 5 one triangle, as
/// `flag_set_attachment_shape` and `flag_set_trailing_shape` cut them.
fn cell_shapes(root: TagStruct<'_>, width: usize, height: usize, attachments: &[FlagAttachment]) -> Vec<i32> {
    let mut cells = vec![0; (width - 1) * (height - 1)];
    let (w, h) = (width as i32, height as i32);
    if root.read_int_any("attached edge shape").unwrap_or(0) != 0 {
        let mut row = 0;
        for attachment in attachments {
            if row >= h {
                break;
            }
            let span = attachment.height_to_next.clamp(0, h - row) & !1;
            let size = span >> 1;
            for x in 0..size {
                for y in row..row + size {
                    if x < w - 1 && y < h - 1 {
                        let down = y - row;
                        cells[x as usize * (height - 1) + y as usize] = if x == down { 4 } else { i32::from(down > x) };
                    }
                }
            }
            tesselate_region(&mut cells, width, height, 0, size + row, size, 5);
            row += span;
        }
    }
    let trailing = root.read_int_any("trailing edge shape").unwrap_or(0) as i32;
    if trailing != 0 {
        let size = if trailing == 3 || trailing == 4 { h - 1 } else { h >> 1 };
        let offset = root.read_int_any("trailing edge shape offset").unwrap_or(0) as i32;
        let x = (w + offset - size - 1).max(0);
        match trailing {
            1 => {
                tesselate_region(&mut cells, width, height, x, 0, size, 2);
                tesselate_region(&mut cells, width, height, x, size, size, 3);
            }
            2 => {
                tesselate_region(&mut cells, width, height, x, 0, size, 3);
                tesselate_region(&mut cells, width, height, x, size, size, 2);
            }
            3 => tesselate_region(&mut cells, width, height, x, 0, size, 3),
            4 => tesselate_region(&mut cells, width, height, x, 0, size, 2),
            _ => {}
        }
    }
    cells
}

/// The flags among `object`'s widgets, added to `preview` as meshes.
/// `markers` are the object's own, in the preview's space; `owner` is 0 for
/// the opened object and 1 for the weapon it holds.
pub(in crate::app::model_preview) fn read_ce_flags(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    preview: &mut RenderModelPreview,
) -> Vec<CeFlagAt> {
    let Some(block) = object_block(object, "widgets") else {
        return Vec::new();
    };
    let group = u32::from_be_bytes(*b"flag");
    let paths: Vec<String> = elements(block, "widgets")
        .iter()
        .filter_map(|widget| widget.read_tag_ref_with_group("reference"))
        .filter(|(kind, path)| *kind == group && !path.is_empty())
        .map(|(_, path)| path)
        .collect();
    paths.iter().filter_map(|path| read_flag(source, path, owner, markers, preview)).collect()
}

/// Each flag's first vertex and its vertices' places and normals.
type FlagMeshes = Vec<(usize, Vec<([f32; 3], [f32; 3])>)>;

/// A point of the sheet: `p` and `v`.
#[derive(Debug, Clone, Copy, Default)]
struct FlagPoint {
    place: [f32; 3],
    velocity: [f32; 3],
}

/// A flag as `flag_datum` keeps it.
#[derive(Debug, Clone, Default)]
struct RunningFlag {
    points: Vec<FlagPoint>,
    first_attachment: [f32; 3],
    random: u32,
}

/// The flags of the opened object, as they move.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeFlagField {
    time: Option<f32>,
    ticks: super::particles::CeTicks,
    running: Vec<RunningFlag>,
}

impl RunningFlag {
    /// `seed_random_direction3d`.
    fn direction(&mut self) -> [f32; 3] {
        let mut next = || {
            self.random = self.random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.random >> 8) as f32 / 16_777_216.0 * 2.0 - 1.0
        };
        loop {
            let direction = [next(), next(), next()];
            let length = (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2]).sqrt();
            if length > 0.0001 && length <= 1.0 {
                return direction.map(|axis| axis / length);
            }
        }
    }

    /// `flag_update`: every point moved, then set where its neighbours,
    /// and down the attached edge the markers, would have it.
    fn update(&mut self, flag: &CeFlagAt, attached: &[[f32; 3]], ground: f32, dt: f32) {
        let (width, height) = (flag.width, flag.height);
        // `flag_update_attachment`: rows between two attachment points are
        // drawn toward the line between them; each one's own row is pinned.
        let mut pinned: Vec<Option<usize>> = vec![None; height];
        let mut force_points: Vec<Option<[f32; 3]>> = vec![None; height];
        let mut row = 0i32;
        for (index, attachment) in flag.attachments.iter().enumerate() {
            if row >= height as i32 {
                break;
            }
            let span = attachment.height_to_next.clamp(0, height as i32 - row) & !1;
            let end = row + span;
            pinned[row as usize] = Some(index);
            let next = attached[(index + 1).min(attached.len() - 1)];
            for at in row..end {
                let t = (at - row) as f32 / (end - row) as f32;
                force_points[at as usize] = Some(std::array::from_fn(|axis| next[axis] * t + attached[index][axis] * (1.0 - t)));
            }
            row = end;
        }
        // A jump of the first marker carries the sheet with it.
        let jump: [f32; 3] = std::array::from_fn(|axis| attached[0][axis] - self.first_attachment[axis]);
        if jump.iter().any(|axis| (*axis as i32).abs() > 1) {
            for point in &mut self.points {
                point.place = std::array::from_fn(|axis| point.place[axis] + jump[axis]);
            }
        }
        self.first_attachment = attached[0];
        let rest: [f32; 3] =
            NEIGHBOURS.map(|(dx, dy)| ((dx as f32 * flag.cell[0]).powi(2) + (dy as f32 * flag.cell[1]).powi(2)).sqrt());
        let noise = flag.physics.air_friction() * flag.wind_noise * 0.0004;
        for x in 0..width {
            for y in 0..height {
                let index = x * height + y;
                let force = self.direction().map(|axis| axis * noise);
                let FlagPoint { place: old, mut velocity } = self.points[index];
                let mut place = old;
                flag.physics.step_with_force(&mut place, &mut velocity, force, POINT_RADIUS, ground, dt);
                if x != 0 || pinned[y].is_none() {
                    let mut total = 0.0;
                    let mut sum = [0.0f32; 3];
                    let mut found = 0;
                    for (&(dx, dy), &length) in NEIGHBOURS.iter().zip(&rest) {
                        let (nx, ny) = (x as isize + dx, y as isize + dy);
                        if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                            continue;
                        }
                        let neighbour = self.points[nx as usize * height + ny as usize].place;
                        let mut away: [f32; 3] = std::array::from_fn(|axis| place[axis] - neighbour[axis]);
                        let distance = (away[0] * away[0] + away[1] * away[1] + away[2] * away[2]).sqrt();
                        if distance.abs() >= 0.0001 {
                            away = away.map(|axis| axis / distance);
                        }
                        let weight = if x != 0 && found == 0 { 4.0 } else { 1.0 };
                        found += 1;
                        total += weight;
                        for axis in 0..3 {
                            sum[axis] += (neighbour[axis] + away[axis] * length) * weight;
                        }
                    }
                    // The rows the attachments leave out have no point to
                    // pull toward; the game reads what its stack holds there.
                    if x == 0
                        && let Some(force_point) = force_points[y]
                    {
                        total += 4.0;
                        for axis in 0..3 {
                            sum[axis] += force_point[axis] * 4.0;
                        }
                    }
                    if total > 0.0 {
                        place = sum.map(|axis| axis / total);
                    }
                } else if let Some(pin) = pinned[y] {
                    place = attached[pin];
                }
                self.points[index] = FlagPoint { place, velocity: std::array::from_fn(|axis| (place[axis] - old[axis]) / dt) };
            }
        }
    }
}

impl CeFlagField {
    /// The flags brought up to `time`: settled once, as `flag_render` does on
    /// the first frame, then a `flags_update` on each frame ticks ran, by
    /// them, as `game_frame` runs `widgets_update`. Returns each flag's
    /// first vertex and its vertices as they now stand.
    pub(crate) fn advance(
        &mut self,
        flags: &[CeFlagAt],
        bones: Option<&[[f32; 4]]>,
        ground: f32,
        time: f32,
    ) -> FlagMeshes {
        let attached_of = |flag: &CeFlagAt| -> Vec<[f32; 3]> {
            flag.attachments.iter().map(|attachment| posed_on(attachment.node, attachment.position, [0.0; 3], bones).0).collect()
        };
        if self.running.len() != flags.len() || self.time.is_none_or(|last| time < last) {
            self.running = flags
                .iter()
                .enumerate()
                .map(|(at, flag)| {
                    let mut running = RunningFlag {
                        points: vec![FlagPoint::default(); flag.width * flag.height],
                        first_attachment: [0.0; 3],
                        random: at as u32 * 977 + 1,
                    };
                    running.update(flag, &attached_of(flag), ground, SETTLE);
                    running
                })
                .collect();
            // Settled is where the game starts it, so there is nothing to
            // simulate before now.
            self.time = Some(time);
            self.ticks = Default::default();
        }
        let now = resume_point(self.time.unwrap_or(time), time, 0.0);
        for dt in frame_steps(now, time) {
            let ticks = self.ticks.run(dt);
            if ticks == 0 {
                continue;
            }
            let dt = ticks as f32 * super::particles::STEP;
            for (flag, running) in flags.iter().zip(&mut self.running) {
                running.update(flag, &attached_of(flag), ground, dt);
            }
        }
        self.time = Some(time);
        flags.iter().zip(&self.running).map(|(flag, running)| (flag.first_vertex, mesh(flag, running))).collect()
    }
}

/// `flag_render_proper`'s vertices: each point with the normal of the cell
/// it starts, the last row and column taking the one before.
fn mesh(flag: &CeFlagAt, running: &RunningFlag) -> Vec<([f32; 3], [f32; 3])> {
    let (width, height) = (flag.width, flag.height);
    let place = |x: usize, y: usize| running.points[x * height + y].place;
    (0..width)
        .flat_map(|x| (0..height).map(move |y| (x, y)))
        .map(|(x, y)| {
            let (cx, cy) = (x.min(width - 2), y.min(height - 2));
            let origin = place(cx, cy);
            let down: [f32; 3] = std::array::from_fn(|axis| place(cx, cy + 1)[axis] - origin[axis]);
            let across: [f32; 3] = std::array::from_fn(|axis| place(cx + 1, cy)[axis] - origin[axis]);
            let mut normal = [
                across[1] * down[2] - across[2] * down[1],
                across[2] * down[0] - across[0] * down[2],
                across[0] * down[1] - across[1] * down[0],
            ];
            let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            if length.abs() >= 0.0001 {
                normal = normal.map(|axis| axis / length);
            }
            (place(x, y), normal)
        })
        .collect()
}
