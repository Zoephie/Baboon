//! A set's placements on the GPU for Game Shaders: each type's mesh, and the
//! instances packed as the decorator shader reads them, drawn instanced the
//! way `c_structure_renderer::render_decorators` draws a type: its mesh once
//! per placement.

use super::super::ModelGpuFrame;
use super::super::gen3::pipeline::{DEFAULT_PROBE, dominant_light, sh_eval_direction};
use super::super::gen3::shaders::Gen3Value;
use super::super::gl::buffer::slice_bytes;
use super::instances::{INSTANCE_BYTES, InstanceLayout, PackedInstances, pack};
use super::wind::WindClock;
use super::{DecoratorSet, Placement};
use crate::core::game::GameId;
use eframe::glow;
use eframe::glow::HasContext as _;

/// One input of a packed instance (`instances`), as GL reads it.
struct InstanceInput {
    /// Past the preview's own seven attributes, which it numbers itself from
    /// 0, and short of the PRT streams' 11 to 15.
    location: u32,
    name: &'static str,
    components: i32,
    kind: u32,
    normalized: bool,
    /// Fed as an integer (`translate_with_integer_inputs`), its bits intact.
    integer: bool,
    offset: i32,
}

/// Halo 3's and ODST's: `TEXCOORD1` to `3`, the position as whole `uint16`
/// steps (the translated shader reads them as floats), the rotation and the
/// light.
const HALO_3_INPUTS: [InstanceInput; 3] = [
    InstanceInput {
        location: 8,
        name: "a_TEXCOORD1",
        components: 4,
        kind: glow::UNSIGNED_SHORT,
        normalized: false,
        integer: false,
        offset: 0,
    },
    InstanceInput {
        location: 9,
        name: "a_TEXCOORD2",
        components: 4,
        kind: glow::UNSIGNED_BYTE,
        normalized: true,
        integer: false,
        offset: 8,
    },
    InstanceInput {
        location: 10,
        name: "a_TEXCOORD3",
        components: 4,
        kind: glow::UNSIGNED_BYTE,
        normalized: true,
        integer: false,
        offset: 12,
    },
];

/// Reach's: the packed position, the aux bytes, the rotation and the light.
const REACH_INPUTS: [InstanceInput; 4] = [
    InstanceInput {
        location: 7,
        name: "a_POSITION1",
        components: 1,
        kind: glow::UNSIGNED_INT,
        normalized: false,
        integer: true,
        offset: 0,
    },
    InstanceInput {
        location: 8,
        name: "a_COLOR2",
        components: 4,
        kind: glow::UNSIGNED_BYTE,
        normalized: false,
        integer: false,
        offset: 4,
    },
    InstanceInput {
        location: 9,
        name: "a_NORMAL1",
        components: 4,
        kind: glow::BYTE,
        normalized: true,
        integer: false,
        offset: 8,
    },
    InstanceInput {
        location: 10,
        name: "a_COLOR1",
        components: 4,
        kind: glow::UNSIGNED_BYTE,
        normalized: true,
        integer: false,
        offset: 12,
    },
];

/// Every instance input's location, which a pass links its name to.
pub(in crate::app::model_preview) const INSTANCE_ATTRIBUTES: [(u32, &str); 7] = [
    (HALO_3_INPUTS[0].location, HALO_3_INPUTS[0].name),
    (HALO_3_INPUTS[1].location, HALO_3_INPUTS[1].name),
    (HALO_3_INPUTS[2].location, HALO_3_INPUTS[2].name),
    (REACH_INPUTS[0].location, REACH_INPUTS[0].name),
    (REACH_INPUTS[1].location, REACH_INPUTS[1].name),
    (REACH_INPUTS[2].location, REACH_INPUTS[2].name),
    (REACH_INPUTS[3].location, REACH_INPUTS[3].name),
];

fn inputs_of(layout: InstanceLayout) -> &'static [InstanceInput] {
    match layout {
        InstanceLayout::Halo3 => &HALO_3_INPUTS,
        // Halo 4's read as Reach's.
        InstanceLayout::Reach | InstanceLayout::Halo4 => &REACH_INPUTS,
    }
}

/// The layout `game`'s shader reads; Halo 3's for any other.
fn layout_of(game: Option<GameId>) -> InstanceLayout {
    match game {
        Some(GameId::HaloReach) => InstanceLayout::Reach,
        Some(GameId::Halo4) => InstanceLayout::Halo4,
        _ => InstanceLayout::Halo3,
    }
}

/// One vertex of a type's mesh: its position scaled by the layout's vertex
/// scale, which with a compression scale of 1 is the game's constant
/// compression scale (2, Halo 4's 5), then its texcoord and normal.
const VERTEX_FLOATS: usize = 8;

/// The ground a placement's colour leans toward by its ground tint: the
/// level's, which a set on its own has none of.
const GROUND: [f32; 3] = [0.5; 3];

/// The sun the game sets before a level's sky gives its own
/// (`render_decorators`): straight up, white.
const SUN_DIRECTION: [f32; 4] = [0.0, 0.0, 1.0, 0.0];

const SUN_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.0];

/// A slot's decorator buffers, for the geometry `mesh_id` names.
pub(in crate::app::model_preview) struct DecoratorBuffers {
    mesh_id: u64,
    vertex_array: glow::NativeVertexArray,
    vertex_buffer: glow::NativeBuffer,
    instance_buffer: glow::NativeBuffer,
    /// Each type's first vertex and vertex count.
    meshes: Vec<(i32, i32)>,
    packed: PackedInstances,
    layout: InstanceLayout,
    /// The wind, run on to the frame being drawn.
    wind: WindClock,
}

impl DecoratorBuffers {
    /// `held`, built for `frame`'s set if it holds another's, with its wind
    /// run on to the frame's time. `None` without a set, or when GL refuses
    /// the buffers. `positions` are the main program's attribute locations
    /// for `a_position`, `a_texcoord` and `a_normal`, which a pass links the
    /// decorator's `POSITION`, `TEXCOORD0` and `NORMAL` to.
    pub(in crate::app::model_preview) unsafe fn sync<'a>(
        held: &'a mut Option<Self>,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        locations: [Option<u32>; 3],
    ) -> Option<&'a mut Self> {
        let set = frame.preview.decorators.as_ref()?;
        if held
            .as_ref()
            .is_none_or(|buffers| buffers.mesh_id != frame.mesh_id)
        {
            let built = match held.take() {
                Some(mut buffers) => {
                    unsafe { buffers.upload(gl, frame.mesh_id, frame.preview.game, set, locations) }
                        .then_some(buffers)
                }
                None => unsafe { Self::new(gl) }.and_then(|mut buffers| {
                    unsafe { buffers.upload(gl, frame.mesh_id, frame.preview.game, set, locations) }
                        .then_some(buffers)
                }),
            };
            *held = built;
        }
        let buffers = held.as_mut()?;
        if let Some(wind) = &set.wind {
            buffers.wind.advance(wind, frame.time);
        } else {
            buffers.wind.time = frame.time;
        }
        Some(buffers)
    }

    unsafe fn new(gl: &glow::Context) -> Option<Self> {
        unsafe {
            Some(Self {
                mesh_id: 0,
                vertex_array: gl.create_vertex_array().ok()?,
                vertex_buffer: gl.create_buffer().ok()?,
                instance_buffer: gl.create_buffer().ok()?,
                meshes: Vec::new(),
                packed: PackedInstances::default(),
                layout: InstanceLayout::Halo3,
                wind: WindClock::default(),
            })
        }
    }

    /// Upload `set`'s meshes and packed placements, and lay out the vertex
    /// array; whether the main program has the attributes to lay out.
    unsafe fn upload(
        &mut self,
        gl: &glow::Context,
        mesh_id: u64,
        game: Option<GameId>,
        set: &DecoratorSet,
        locations: [Option<u32>; 3],
    ) -> bool {
        let [Some(position), texcoord, normal] = locations else {
            return false;
        };
        self.layout = layout_of(game);
        let vertex_scale = self.layout.vertex_scale();
        let mut vertices: Vec<f32> = Vec::new();
        self.meshes.clear();
        for mesh in &set.meshes {
            let first = (vertices.len() / VERTEX_FLOATS) as i32;
            for &index in &mesh.indices {
                let index = index as usize;
                vertices.extend(mesh.positions[index].map(|value| value * vertex_scale));
                vertices.extend(mesh.texcoords[index]);
                vertices.extend(mesh.normals[index]);
            }
            self.meshes
                .push((first, (vertices.len() / VERTEX_FLOATS) as i32 - first));
        }
        let light = baked_light();
        self.packed = pack(
            self.layout,
            &set.placements,
            |placement| placement_light(placement, light),
            set.types.len(),
        );
        self.mesh_id = mesh_id;
        self.wind = WindClock::default();
        let stride = (VERTEX_FLOATS * size_of::<f32>()) as i32;
        unsafe {
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertex_buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                slice_bytes(&vertices),
                glow::STATIC_DRAW,
            );
            gl.enable_vertex_attrib_array(position);
            gl.vertex_attrib_pointer_f32(position, 3, glow::FLOAT, false, stride, 0);
            for (location, size, offset) in [(texcoord, 2, 12), (normal, 3, 20)] {
                if let Some(location) = location {
                    gl.enable_vertex_attrib_array(location);
                    gl.vertex_attrib_pointer_f32(
                        location,
                        size,
                        glow::FLOAT,
                        false,
                        stride,
                        offset,
                    );
                }
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.instance_buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &self.packed.bytes, glow::STATIC_DRAW);
            // Another game's set may have had the array before.
            for (location, _) in INSTANCE_ATTRIBUTES {
                gl.disable_vertex_attrib_array(location);
            }
            for input in inputs_of(self.layout) {
                gl.enable_vertex_attrib_array(input.location);
                gl.vertex_attrib_divisor(input.location, 1);
            }
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        true
    }

    /// Draw type `type_index`'s mesh once per placement of it.
    pub(in crate::app::model_preview) unsafe fn draw(&self, gl: &glow::Context, type_index: usize) {
        let (Some(&(first, count)), Some(&(first_instance, instances))) = (
            self.meshes.get(type_index),
            self.packed.types.get(type_index),
        ) else {
            return;
        };
        if count == 0 || instances == 0 {
            return;
        }
        let base = (first_instance * INSTANCE_BYTES) as i32;
        let stride = INSTANCE_BYTES as i32;
        unsafe {
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.instance_buffer));
            for input in inputs_of(self.layout) {
                let offset = base + input.offset;
                if input.integer {
                    gl.vertex_attrib_pointer_i32(
                        input.location,
                        input.components,
                        input.kind,
                        stride,
                        offset,
                    );
                } else {
                    gl.vertex_attrib_pointer_f32(
                        input.location,
                        input.components,
                        input.kind,
                        input.normalized,
                        stride,
                        offset,
                    );
                }
            }
            gl.draw_arrays_instanced(glow::TRIANGLES, first, count, instances as i32);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
    }
}

/// What the decorator shader reads of the set, by uniform: its placements'
/// compression, its own shading values, the sun and the wind. `None` for a
/// uniform the engine's globals give.
pub(in crate::app::model_preview) fn decorator_value(
    set: &DecoratorSet,
    buffers: &DecoratorBuffers,
    name: &str,
) -> Option<Gen3Value> {
    let floats = |value: [f32; 4]| Some(Gen3Value::Floats(value));
    let [offset, scale] =
        [buffers.packed.offset, buffers.packed.scale].map(|[x, y, z]| [x, y, z, 1.0]);
    match name {
        "instance_compression_offset" => floats(offset),
        "instance_compression_scale" => floats(scale),
        // No fade: the distance the game fades over is the Distance card's.
        "LOD_constants" => floats([0.0, 1.0, 0.0, 0.0]),
        "translucency" => floats(set.translucency),
        "wave_flow" => floats(set.wave),
        "contrast" => floats([set.shaded[0], set.shaded[1], 0.0, 0.0]),
        // Reach's `setup_wetness_constants_special_objects(view, 1)`, which
        // `render_single_pass_lighting` calls before the decorators: dry, 1
        // in all but `y` (1.3 × the exposure), which they don't read.
        "k_ps_wetness_coefficients" => floats([1.0; 4]),
        "sun_direction" => floats(SUN_DIRECTION),
        "sun_color" => floats(SUN_COLOR),
        "wind_data" | "wind_data2" => {
            let [data, data2] = match &set.wind {
                Some(wind) => buffers.wind.constants(wind),
                None => [[0.0; 4], [0.0, 0.0, 0.0, buffers.wind.time]],
            };
            floats(if name == "wind_data" { data } else { data2 })
        }
        _ => None,
    }
}

/// The light a level's lightmap would bake into a decorator, which the
/// preview's own probe stands in for: the game's bake reads the lightmap's
/// probe toward its dominant light.
fn baked_light() -> [f32; 3] {
    let (direction, _, _) = dominant_light(&DEFAULT_PROBE);
    let basis = sh_eval_direction([direction[0], direction[1], direction[2]]);
    DEFAULT_PROBE.map(|channel| {
        channel
            .iter()
            .zip(basis)
            .map(|(a, b)| a * b)
            .sum::<f32>()
            .max(0.0)
    })
}

/// A placement's baked colour: its tint leaning toward the ground by its
/// ground tint, lit.
fn placement_light(placement: &Placement, light: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|channel| {
        let tint = placement.tint[channel]
            + (GROUND[channel] - placement.tint[channel]) * placement.ground_tint;
        tint * light[channel]
    })
}
