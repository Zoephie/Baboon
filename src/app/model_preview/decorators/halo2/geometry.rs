//! Halo 2's placements built into a model, as its cache builder
//! (`sub_6B20A0`) and model draw (`sub_94AE50`) build them.
//!
//! The screen-facing and axis-rotating quads turn toward the viewer in their
//! vertex shader (`decorator.vertex_shader`'s slots 7 and 8). Baked here, for
//! Solid and Textured, they face the way that shader would turn them for an
//! eye looking along +Y.

use super::super::super::common::geometry::{Overflow, VertexBase, expand_preview_bounds_local};
use super::super::super::h2::H2MaterialLook;
use super::super::super::h2::shader::decorator_stage::DecoratorSlot;
use super::super::super::{
    RenderModelPreview, RenderModelPreviewBatch, RenderModelPreviewMaterial,
    RenderModelPreviewRegion, RenderModelPreviewVertex,
};
use super::super::bake::{PERMUTATION, type_name};
use super::super::stream::DecoratorStreamVertex;
use super::{ClassKind, Halo2Set, Permutation, Plate, Record, Sprite, WHOLE_IMAGE};

/// Shader references' group.
const SHADER_GROUP: [u8; 4] = *b"shad";

/// The eye a baked quad faces: looking along +Y, up +Z.
const EYE_RIGHT: [f32; 3] = [1.0, 0.0, 0.0];
const EYE_UP: [f32; 3] = [0.0, 0.0, 1.0];
const EYE_FORWARD: [f32; 3] = [0.0, 1.0, 0.0];

/// How far above the ground a floating decal is laid (`sub_6B0090`).
const DECAL_LIFT: f32 = 0.001;

/// A quad's corners, in the order the cache builder writes them
/// (`0xCC1178`, `0xCC11D8`), and its two triangles.
const CORNERS: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
const QUAD: [u32; 6] = [0, 2, 1, 2, 0, 3];

/// The name a type of `set` goes by: its class's and permutation's.
pub(crate) fn halo_2_type_name(set: &Halo2Set, class: usize, permutation: usize) -> String {
    let class = &set.classes[class];
    format!("{} / {}", class.name, class.permutations[permutation].name)
}

/// The light a model's placement keeps (`c203`), which a level's lightmap
/// would give it: straight down, as the preview's default light.
const MODEL_LIGHT: [f32; 3] = [0.0, 0.0, -1.0];

/// How much of a model's light is that direction's (`c203.w`), from its
/// placement's float between the set's least and most lighting scale. The
/// kit's levels' placements keep about 0.96, which the defaults of 0.4 and 2
/// make this.
const MODEL_ACCURACY: f32 = 0.35;

/// A placement's lightmap colour, which a level's lightmap gives it.
const LIGHTMAP_COLOR: f32 = 0.5;

/// The classification of its vertex shader a class draws through
/// (`sub_94BB80`).
fn slot_of(kind: ClassKind) -> DecoratorSlot {
    match kind {
        ClassKind::Model => DecoratorSlot::Rigid,
        ClassKind::FloatingDecal | ClassKind::ProjectedDecal => DecoratorSlot::Decal,
        ClassKind::ScreenFacingQuad => DecoratorSlot::ScreenFacing,
        ClassKind::AxisRotatingQuad => DecoratorSlot::AxisRotating,
        ClassKind::CrossQuad => DecoratorSlot::World,
    }
}

/// A placement's colour (`sub_6AEF00`): between its permutation's two tints
/// by its tint bits, times its own tint by the base map tint percentage,
/// times its lightmap colour by the lightmap tint percentage, half where
/// that is none. A level keeps a placement's own tint and lightmap colour;
/// on its own its tint is white and its lightmap colour mid-grey, which the
/// decorator pixel shaders' doubling (`2 · T0 · D0`) leaves the texture's
/// own colour.
fn placement_color(permutation: &Permutation, record: &Record) -> [f32; 3] {
    let along = f32::from(record.tint) / 7.0;
    let [first, second] = permutation.tints;
    let (base_tint, lightmap_tint) = (permutation.base_map_tint, permutation.lightmap_tint);
    std::array::from_fn(|channel| {
        let tint = (1.0 - along) * first[channel] + second[channel] * along;
        let base = tint * (base_tint + (1.0 - base_tint));
        let lightmap = LIGHTMAP_COLOR * lightmap_tint + (1.0 - lightmap_tint) * 0.5;
        base * lightmap
    })
}

/// A colour as a vertex's `D3DCOLOR` holds it (`sub_4B7E50`): each channel
/// cut to a byte, and no alpha.
fn vertex_color(color: [f32; 3]) -> [f32; 4] {
    let [r, g, b] = color.map(|channel| ((channel * 255.0) as i32 as u8) as f32 / 255.0);
    [r, g, b, 0.0]
}

/// `set`'s records as a model: a region per permutation, a batch per
/// permutation (per placement for a model, whose constants are its own),
/// each drawn with its shader through its class's vertex shader.
pub(in crate::app::model_preview) fn bake_halo_2(set: &Halo2Set) -> RenderModelPreview {
    let mut preview = RenderModelPreview {
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };
    // A shader drawn through more than one classification is a material for each.
    let mut materials: Vec<(usize, DecoratorSlot)> = Vec::new();
    for (class_index, class) in set.classes.iter().enumerate() {
        for (permutation_index, permutation) in class.permutations.iter().enumerate() {
            let name = type_name(
                set.type_index(class_index, permutation_index),
                &halo_2_type_name(set, class_index, permutation_index),
            );
            preview.regions.push(RenderModelPreviewRegion {
                name: name.clone(),
                permutations: vec![PERMUTATION.to_owned()],
            });
            let (Some(kind), Some(shader)) = (
                class.kind,
                permutation.shader.filter(|&at| at < set.shaders.len()),
            ) else {
                continue;
            };
            let slot = slot_of(kind);
            let material = materials
                .iter()
                .position(|&known| known == (shader, slot))
                .unwrap_or_else(|| {
                    materials.push((shader, slot));
                    materials.len() - 1
                }) as u16;
            let plate = set.plates.get(shader).and_then(Option::as_ref);
            let mut index_start = preview.indices.len() as u32;
            let batch = |preview: &mut RenderModelPreview, index_start: u32| {
                let index_count = preview.indices.len() as u32 - index_start;
                (index_count > 0).then(|| {
                    preview.batches.push(RenderModelPreviewBatch {
                        region_name: name.clone(),
                        permutation_name: PERMUTATION.to_owned(),
                        material_index: material,
                        index_start,
                        index_count,
                        ..Default::default()
                    });
                    preview.batches.len() - 1
                })
            };
            for record in set.records.iter().filter(|record| {
                record.class == class_index && record.permutation == permutation_index
            }) {
                let color = placement_color(permutation, record);
                let mut built = Built::default();
                match kind {
                    ClassKind::Model => model(set, permutation, record, color, &mut built),
                    ClassKind::FloatingDecal | ClassKind::ProjectedDecal => {
                        decal(plate, permutation, record, color, &mut built)
                    }
                    ClassKind::ScreenFacingQuad
                    | ClassKind::AxisRotatingQuad
                    | ClassKind::CrossQuad => {
                        quad(kind, plate, permutation, record, color, &mut built)
                    }
                }
                built.append(&mut preview);
                if kind == ClassKind::Model {
                    if let Some(at) = batch(&mut preview, index_start) {
                        let [r, g, b] = color;
                        let [x, y, z] = MODEL_LIGHT;
                        preview.h2.decorator_rigid.insert(
                            at,
                            [
                                [x, y, z, MODEL_ACCURACY],
                                // Its vertices are placed already: the matrix is none.
                                [1.0, 0.0, 0.0, 0.0],
                                [0.0, 1.0, 0.0, 0.0],
                                [0.0, 0.0, 1.0, 0.0],
                                [r, g, b, 1.0],
                            ],
                        );
                    }
                    index_start = preview.indices.len() as u32;
                }
            }
            if kind != ClassKind::Model {
                batch(&mut preview, index_start);
            }
        }
    }
    preview.materials = materials
        .into_iter()
        .map(|(shader, slot)| RenderModelPreviewMaterial {
            shader_path: set.shaders[shader].clone(),
            shader_group: u32::from_be_bytes(SHADER_GROUP),
            h2: H2MaterialLook {
                decorator: Some(slot),
                ..Default::default()
            },
            ..Default::default()
        })
        .collect();
    preview
}

/// One placement's vertices and triangles, before they join the model.
#[derive(Default)]
struct Built {
    vertices: Vec<RenderModelPreviewVertex>,
    stream: Vec<DecoratorStreamVertex>,
    indices: Vec<u32>,
}

impl Built {
    fn vertex(
        &mut self,
        position: [f32; 3],
        normal: [f32; 3],
        texcoord: [f32; 2],
        stream: DecoratorStreamVertex,
    ) -> u32 {
        self.stream.push(stream);
        self.vertices.push(RenderModelPreviewVertex {
            position,
            normal,
            texcoord,
            ..Default::default()
        });
        (self.vertices.len() - 1) as u32
    }

    /// A model's vertex, with its tangent frame.
    fn framed(
        &mut self,
        position: [f32; 3],
        frame: [[f32; 3]; 3],
        texcoord: [f32; 2],
        stream: DecoratorStreamVertex,
    ) -> u32 {
        self.stream.push(stream);
        let [normal, tangent, binormal] = frame;
        self.vertices.push(RenderModelPreviewVertex {
            position,
            normal,
            texcoord,
            tangent,
            binormal,
            ..Default::default()
        });
        (self.vertices.len() - 1) as u32
    }

    fn append(self, preview: &mut RenderModelPreview) {
        let Some(base) = VertexBase::of(
            preview.vertices.len(),
            self.vertices.len(),
            Overflow::RefusesAppend,
        ) else {
            return;
        };
        for vertex in &self.vertices {
            expand_preview_bounds_local(
                &mut preview.bounds_min,
                &mut preview.bounds_max,
                vertex.position,
            );
        }
        preview.vertices.extend(self.vertices);
        preview.decorator_stream.extend(self.stream);
        preview
            .indices
            .extend(self.indices.iter().map(|&index| base.offset(index)));
    }
}

/// A model at its placement: `[forward · S | left · S | up · S | position]`
/// (`c204` to `c206`), the axes a yaw about up (`sub_4BA710`), or, aligned
/// to the ground's normal, about that.
fn model(
    set: &Halo2Set,
    permutation: &Permutation,
    record: &Record,
    color: [f32; 3],
    built: &mut Built,
) {
    let Some(range) = set.models.get(permutation.index) else {
        return;
    };
    let Some(indices) = set.indices.get(range.clone()) else {
        return;
    };
    let scale = record.scale(permutation.scale);
    let (sin, cos) = record.yaw().sin_cos();
    let [forward, left, up] = if permutation.align_to_normal {
        let up = normalized(record.normal());
        let along = perpendicular(up);
        let across = cross(up, along);
        let forward = std::array::from_fn(|axis| across[axis] * sin + along[axis] * cos);
        let left = std::array::from_fn(|axis| across[axis] * cos - along[axis] * sin);
        [forward, left, up]
    } else {
        [[cos, sin, 0.0], [-sin, cos, 0.0], [0.0, 0.0, 1.0]]
    };
    let origin = record.position();
    let turn = |[x, y, z]: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|axis| forward[axis] * x + left[axis] * y + up[axis] * z)
    };
    for triangle in indices.chunks_exact(3) {
        if triangle
            .iter()
            .any(|&index| index as usize >= set.vertices.len())
        {
            continue;
        }
        for &index in triangle {
            let vertex = set.vertices[index as usize];
            let placed = turn(vertex.position);
            let position = std::array::from_fn(|axis| origin[axis] + placed[axis] * scale);
            let frame = [vertex.normal, vertex.tangent, vertex.binormal].map(turn);
            let stream = DecoratorStreamVertex {
                origin: position,
                color: vertex_color(color),
                ..Default::default()
            };
            let at = built.framed(position, frame, vertex.texcoord, stream);
            built.indices.push(at);
        }
    }
}

/// The sprite a placement shows (`sub_6B20A0`): of its permutation's
/// sequence, picked by its place's and tint's bits.
fn sprite(plate: Option<&Plate>, permutation: &Permutation, record: &Record) -> Sprite {
    let sprites = plate
        .and_then(|plate| plate.sequences.get(permutation.index))
        .map_or(&[][..], Vec::as_slice);
    if sprites.is_empty() {
        return WHOLE_IMAGE;
    }
    let pick = record
        .place
        .iter()
        .map(|&bits| usize::from(bits))
        .sum::<usize>()
        + usize::from(record.tint);
    sprites[pick % sprites.len()]
}

/// A sprite's size at `scale`, and where its registration point falls
/// across and down it: `W`, `H`, `rx` and `ry`.
fn sprite_size(plate: Option<&Plate>, sprite: Sprite, scale: f32) -> ([f32; 2], [f32; 2]) {
    let [left, right, top, bottom] = sprite.bounds;
    let size = plate.map_or([0.0; 2], |plate| plate.size);
    let wide = (right - left) * scale * size[0] / 256.0;
    let high = (bottom - top) * scale * size[1] / 256.0;
    let across = |offset: f32, span: f32| if span != 0.0 { offset / span } else { 0.5 };
    (
        [wide, high],
        [
            across(sprite.registration[0], right - left),
            across(sprite.registration[1], bottom - top),
        ],
    )
}

/// A quad (`sub_6B04F0`): standing on its placement, its offset from it
/// `(0, (y − rx)·W, (z − (1 − ry))·H)`. The screen-facing one lays that
/// along the eye's right and up, the axis-rotating one along its axis and
/// the side the eye sees; a cross quad stands it in two planes, turned by
/// its yaw. Each draws both faces here, as it turns to face the eye in game.
fn quad(
    kind: ClassKind,
    plate: Option<&Plate>,
    permutation: &Permutation,
    record: &Record,
    color: [f32; 3],
    built: &mut Built,
) {
    let sprite = sprite(plate, permutation, record);
    let ([wide, high], [rx, ry]) = sprite_size(plate, sprite, record.scale(permutation.scale));
    let [left, right, top, bottom] = sprite.bounds;
    let origin = record.position();
    let axis = if permutation.align_to_normal {
        normalized(record.normal())
    } else {
        [0.0, 0.0, 1.0]
    };
    let side = normalized(cross(axis, EYE_FORWARD));
    let (sin, cos) = record.yaw().sin_cos();
    let planes: &[[[f32; 3]; 2]] = match kind {
        ClassKind::ScreenFacingQuad => &[[EYE_RIGHT.map(|value| -value), EYE_UP]],
        ClassKind::AxisRotatingQuad => &[[side, axis]],
        _ => &[[[-sin, cos, 0.0], axis], [[cos, sin, 0.0], axis]],
    };
    for &[across, upward] in planes {
        let mut corners = [0; 4];
        for (corner, [y, z]) in corners.iter_mut().zip(CORNERS) {
            let offset = [(y - rx) * wide, (z - (1.0 - ry)) * high];
            let position = std::array::from_fn(|at| {
                origin[at] + across[at] * offset[0] + upward[at] * offset[1]
            });
            // The axis-rotating quad reads its sprite the other way across.
            let flipped = kind == ClassKind::AxisRotatingQuad;
            let u = if (y > 0.0) != flipped { left } else { right };
            let v = if z > 0.0 { top } else { bottom };
            // The cache builder's offset and axis, which the screen-facing
            // and axis-rotating vertex shaders turn to the eye; a cross
            // quad's corners are baked where they stand.
            let stream = match kind {
                ClassKind::CrossQuad => DecoratorStreamVertex {
                    origin: position,
                    color: vertex_color(color),
                    ..Default::default()
                },
                _ => DecoratorStreamVertex {
                    origin,
                    color: vertex_color(color),
                    offset: [0.0, offset[0], offset[1]],
                    axis,
                },
            };
            *corner = built.vertex(position, cross(across, upward), [u, v], stream);
        }
        for face in [QUAD, [QUAD[0], QUAD[2], QUAD[1], QUAD[3], QUAD[5], QUAD[4]]] {
            built.indices.extend(face.map(|at| corners[at as usize]));
        }
    }
}

/// A decal on the ground beneath its placement (`sub_6B0090`): its corners
/// `((x − rx)·W, (y − ry)·H, 0)` in the frame of the ground's
/// perpendicular turned by its yaw, the ground's left of that and its
/// normal, lifted off the ground a little. A projected decal is clipped to
/// the level in game; on flat ground it is the same quad. `upright` stands a
/// decal's sprite up a wall (its tangent `+Z`), which flat ground has none
/// of to stand along.
fn decal(
    plate: Option<&Plate>,
    permutation: &Permutation,
    record: &Record,
    color: [f32; 3],
    built: &mut Built,
) {
    let sprite = sprite(plate, permutation, record);
    let ([wide, high], [rx, ry]) = sprite_size(plate, sprite, record.scale(permutation.scale));
    let [left, right, top, bottom] = sprite.bounds;
    let normal = [0.0, 0.0, 1.0];
    let (sin, cos) = record.yaw().sin_cos();
    let along = perpendicular(normal);
    let across = cross(normal, along);
    let forward: [f32; 3] = std::array::from_fn(|axis| along[axis] * cos + across[axis] * sin);
    let sideways = cross(normal, forward);
    let [x, y, _] = record.position();
    let ground = [x, y, DECAL_LIFT];
    let mut corners = [0; 4];
    for (corner, [cx, cy]) in corners.iter_mut().zip(CORNERS) {
        let offset = [(cx - rx) * wide, (cy - ry) * high];
        let position = std::array::from_fn(|at| {
            ground[at] + forward[at] * offset[0] + sideways[at] * offset[1]
        });
        let u = if cx > 0.0 { left } else { right };
        let v = if cy > 0.0 { bottom } else { top };
        let stream = DecoratorStreamVertex {
            origin: position,
            color: vertex_color(color),
            ..Default::default()
        };
        *corner = built.vertex(position, normal, [u, v], stream);
    }
    for face in [QUAD, [QUAD[0], QUAD[2], QUAD[1], QUAD[3], QUAD[5], QUAD[4]]] {
        built.indices.extend(face.map(|at| corners[at as usize]));
    }
}

/// `perpendicular3d` [inferred]: the vector's turn about whichever axis it
/// lies least along.
fn perpendicular([x, y, z]: [f32; 3]) -> [f32; 3] {
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    if ax <= ay && ax <= az {
        [0.0, -z, y]
    } else if ay <= ax && ay <= az {
        [-z, 0.0, x]
    } else {
        [y, -x, 0.0]
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalized(vector: [f32; 3]) -> [f32; 3] {
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length > 0.0 {
        vector.map(|value| value / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}
