//! A set's placements on the GPU for Game Shaders: each type's mesh, and the
//! instances packed as the decorator shader reads them, drawn instanced the
//! way `c_structure_renderer::render_decorators` draws a type: its mesh once
//! per placement.

use super::super::ModelGpuFrame;
use super::super::gen3::pipeline::{DEFAULT_PROBE, dominant_light, sh_eval_direction};
use super::super::gen3::shaders::Gen3Value;
use super::super::gl::buffer::slice_bytes;
use super::instances::{INSTANCE_BYTES, PackedInstances, pack};
use super::wind::WindClock;
use super::{DecoratorSet, Placement};
use eframe::glow;
use eframe::glow::HasContext as _;

/// Where `TEXCOORD1` to `TEXCOORD3`, the instance's position, rotation and
/// light, arrive: past the preview's own attributes, which it numbers
/// itself from 0, and short of the PRT streams' 11 to 15.
pub(in crate::app::model_preview) const INSTANCE_ATTRIBUTES: [(u32, &str); 3] =
    [(8, "a_TEXCOORD1"), (9, "a_TEXCOORD2"), (10, "a_TEXCOORD3")];

/// One vertex of a type's mesh: its position at twice its size, which with
/// a compression scale of 1 is the game's constant compression scale of 2,
/// then its texcoord and normal.
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
                    unsafe { buffers.upload(gl, frame.mesh_id, set, locations) }.then_some(buffers)
                }
                None => unsafe { Self::new(gl) }.and_then(|mut buffers| {
                    unsafe { buffers.upload(gl, frame.mesh_id, set, locations) }.then_some(buffers)
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
        set: &DecoratorSet,
        locations: [Option<u32>; 3],
    ) -> bool {
        let [Some(position), texcoord, normal] = locations else {
            return false;
        };
        let mut vertices: Vec<f32> = Vec::new();
        self.meshes.clear();
        for mesh in &set.meshes {
            let first = (vertices.len() / VERTEX_FLOATS) as i32;
            for &index in &mesh.indices {
                let index = index as usize;
                vertices.extend(mesh.positions[index].map(|value| value * 2.0));
                vertices.extend(mesh.texcoords[index]);
                vertices.extend(mesh.normals[index]);
            }
            self.meshes
                .push((first, (vertices.len() / VERTEX_FLOATS) as i32 - first));
        }
        let light = baked_light();
        self.packed = pack(
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
            for (location, _) in INSTANCE_ATTRIBUTES {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_divisor(location, 1);
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
        let [(position, _), (rotation, _), (light, _)] = INSTANCE_ATTRIBUTES;
        unsafe {
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.instance_buffer));
            // `R16G16B16A16_UINT`, which the translated shader reads as floats.
            gl.vertex_attrib_pointer_f32(
                position,
                4,
                glow::UNSIGNED_SHORT,
                false,
                INSTANCE_BYTES as i32,
                base,
            );
            gl.vertex_attrib_pointer_f32(
                rotation,
                4,
                glow::UNSIGNED_BYTE,
                true,
                INSTANCE_BYTES as i32,
                base + 8,
            );
            gl.vertex_attrib_pointer_f32(
                light,
                4,
                glow::UNSIGNED_BYTE,
                true,
                INSTANCE_BYTES as i32,
                base + 12,
            );
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
