//! The GPU slots a preview's buffers and textures live in, least recently drawn first out.

use super::super::dx9::passes::{Dx9DrawPass, H2_LAYER_COUNT};
use super::super::gl::buffer::slice_bytes;
use super::super::gl::texture::{upload_image, upload_texture};
use super::super::particles::{sprite_maps, upload_cheap_render_texture};
use super::*;
use crate::app::model_preview::common::image::Sampler;
use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};
use eframe::glow;

/// How many models stay on the GPU at once: geometry, ground grid and
/// textures. Past this, the one drawn longest ago gives up its slot.
pub(super) const MODEL_GPU_SLOTS: usize = 4;

/// Which slot each model draws from, least recently drawn out first.
///
/// A slot per model keeps two previews on screen from replacing each other's
/// upload every frame, which would re-send both models' vertices, indices
/// and textures to the GPU each time.
#[derive(Default)]
pub(super) struct ModelSlotLru {
    /// `(geometry id, last drawn)`, one per slot in use.
    pub(super) slots: Vec<(u64, u64)>,
    pub(super) clock: u64,
}

impl ModelSlotLru {
    /// The slot for `geometry_id`, and whether it must be uploaded: a new
    /// slot, or one taken from the model drawn longest ago.
    pub(super) fn acquire(&mut self, geometry_id: u64, capacity: usize) -> (usize, bool) {
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
    pub(super) fn abandon(&mut self, index: usize) {
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
    pub(in crate::app::model_preview) particles: particles::ParticleField,
    /// Reach's cheap particles.
    pub(in crate::app::model_preview) reach_cheap: particles::cheap::CheapField,
    pub(in crate::app::model_preview) h3_strips: gen3::effects::strips::Gen3StripField,
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
    pub(super) fn each(&self, mut visit: impl FnMut(&str, &dyn std::fmt::Debug)) {
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
pub(in crate::app::model_preview) struct ModelGpuSlot {
    /// What runs from frame to frame, restarted together.
    pub(in crate::app::model_preview) simulations: SlotSimulations,
    pub(in crate::app::model_preview) vertex_array: glow::NativeVertexArray,
    pub(super) vertex_buffer: glow::NativeBuffer,
    /// Halo 2 per-vertex PRT, unused by every other model.
    pub(super) prt_buffer: glow::NativeBuffer,
    pub(super) index_buffer: glow::NativeBuffer,
    /// The ground grid's own vertex array and buffer, built per geometry
    /// (its spacing and extent follow the model's bounds).
    pub(super) grid_vertex_array: glow::NativeVertexArray,
    pub(super) grid_vertex_buffer: glow::NativeBuffer,
    pub(super) grid_vertex_count: i32,
    /// Vertex index where the world-axis lines begin; see `grid_line_vertices`.
    pub(super) grid_axis_start: i32,
    pub(super) grid_built: bool,
    /// The `mesh_id` the buffers hold.
    pub(in crate::app::model_preview) mesh_id: Option<u64>,
    /// How many models the slot has taken, which each game's own state for
    /// the slot follows.
    pub(in crate::app::model_preview) uploads: u64,
    /// Per batch, where its transparent passes sort from.
    pub(in crate::app::model_preview) sort_points: Vec<[f32; 3]>,
    /// GL textures per material, in `RenderModelPreview::materials` order.
    pub(in crate::app::model_preview) materials: Vec<MaterialGlTextures>,
    /// The `textures_id` the uploaded `materials` belong to, so a reloaded
    /// model, or another model given this slot, re-uploads rather than drawing
    /// the wrong textures.
    pub(super) uploaded_textures: Option<u64>,
    /// Halo 3's and Halo 4's lens flares: per flare, then per image, of
    /// `flare_geometry`.
    pub(in crate::app::model_preview) flare_textures: Vec<Vec<Option<glow::NativeTexture>>>,
    pub(in crate::app::model_preview) h3_textures: gen3::pipeline::Gen3SlotTextures,
    pub(in crate::app::model_preview) flare_geometry: Option<u64>,
    pub(in crate::app::model_preview) antenna_textures: Vec<Option<glow::NativeTexture>>,
    pub(in crate::app::model_preview) volume_textures: Vec<Option<glow::NativeTexture>>,
    /// The cheap particles' library's render texture array.
    pub(in crate::app::model_preview) reach_cheap_texture: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) strip_textures: Vec<[Option<glow::NativeTexture>; 4]>,
    pub(in crate::app::model_preview) particle_textures: Vec<[Option<glow::NativeTexture>; 4]>,
}

impl ModelGpuSlot {
    pub(super) unsafe fn new(
        gl: &glow::Context,
        program: glow::NativeProgram,
    ) -> Result<Self, String> {
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
                sort_points: Vec::new(),
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

    pub(super) unsafe fn upload_geometry(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        self.mesh_id = Some(frame.mesh_id);
        self.sort_points = sort_points(&frame.preview);
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
                    gl.buffer_data_u8_slice(
                        glow::ARRAY_BUFFER,
                        slice_bytes(&prt.vertices),
                        glow::STATIC_DRAW,
                    );
                    let stride = std::mem::size_of::<[f32; gen3::prt::COEFFICIENTS]>() as i32;
                    for (index, attribute) in gen3::pipeline::PRT_ATTRIBUTES.into_iter().enumerate()
                    {
                        gl.enable_vertex_attrib_array(attribute);
                        // The first holds a fourth for the linear shader, in the
                        // second's first place.
                        let size = if index == 0 { 4 } else { 3 };
                        gl.vertex_attrib_pointer_f32(
                            attribute,
                            size,
                            glow::FLOAT,
                            false,
                            stride,
                            index as i32 * 12,
                        );
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
    pub(in crate::app::model_preview) unsafe fn sync_sprite_textures(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
    ) {
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
            let maps = |look: &particles::SpriteShader| sprite_maps(gl, look);
            self.particle_textures = frame
                .preview
                .particles
                .iter()
                .map(|system| maps(&system.look.shader))
                .collect();
            self.strip_textures = frame
                .preview
                .gen3
                .strips
                .iter()
                .map(|strip| maps(strip.shader()))
                .collect();
            if let Some(texture) = self.reach_cheap_texture.take() {
                gl.delete_texture(texture);
            }
            let render = frame
                .preview
                .gen3
                .cheap_emitters
                .first()
                .and_then(|at| at.library().render_texture());
            self.reach_cheap_texture =
                render.and_then(|render| upload_cheap_render_texture(gl, render));
            for texture in self.flare_textures.drain(..).flatten().flatten() {
                gl.delete_texture(texture);
            }
            self.flare_textures = frame
                .preview
                .gen3
                .flares
                .iter()
                .map(|at| {
                    let (image, texels, mips) = at.bitmap();
                    vec![crate::app::model_preview::gl::texture::upload_image_linear(
                        gl, image, *texels, mips, None,
                    )]
                })
                .chain(super::game::game_flare_images(gl, frame))
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
                        let filter = if antenna.is_reach() {
                            BitmapFilterMode::Trilinear
                        } else {
                            BitmapFilterMode::Bilinear
                        };
                        let sampler = Sampler {
                            address: [BitmapAddressMode::Clamp; 2],
                            filter,
                        };
                        crate::app::model_preview::gl::texture::upload_image_linear(
                            gl,
                            image,
                            *texels,
                            mips,
                            Some(sampler),
                        )
                    }
                    None => upload_image(gl, &antenna.image),
                })
                .collect();
            for texture in self.volume_textures.drain(..).flatten() {
                gl.delete_texture(texture);
            }
            self.volume_textures = frame
                .preview
                .gen3
                .light_volumes
                .iter()
                .map(|at| {
                    let (image, texels, mips) = at.bitmap()?;
                    crate::app::model_preview::gl::texture::upload_image_linear(
                        gl,
                        image,
                        *texels,
                        mips,
                        at.sampler(),
                    )
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

    pub(super) fn restart_simulations(&mut self) {
        self.simulations = SlotSimulations::default();
    }

    /// Build the ground grid for this slot's model, once.
    pub(super) unsafe fn sync_grid(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
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
    pub(super) unsafe fn sync_textures(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
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
                let upload = |passes: &[Dx9DrawPass]| -> Vec<Vec<Option<glow::NativeTexture>>> {
                    passes
                        .iter()
                        .map(|pass| {
                            pass.textures
                                .iter()
                                .map(|texture| {
                                    let image = texture.image.as_ref()?;
                                    unsafe { upload_image(gl, image) }
                                })
                                .collect()
                        })
                        .collect()
                };
                uploaded.h2_textures =
                    std::array::from_fn(|layer| upload(&material.h2_layers[layer]));
                uploaded
            })
            .collect();
    }

    pub(super) unsafe fn release_textures(&mut self, gl: &glow::Context) {
        for material in self.materials.drain(..) {
            let passes = material
                .h2_textures
                .into_iter()
                .flatten()
                .flatten()
                .flatten();
            for texture in material.textures.into_iter().flatten().chain(passes) {
                unsafe { gl.delete_texture(texture) };
            }
        }
    }
}

/// One material's textures on the GPU, in `TextureSlot` order.
pub(in crate::app::model_preview) struct MaterialGlTextures {
    pub(in crate::app::model_preview) textures: [Option<glow::NativeTexture>; SLOT_COUNT],
    /// By `Dx9Layer` and pass, one texture per entry of its `textures`.
    pub(in crate::app::model_preview) h2_textures:
        [Vec<Vec<Option<glow::NativeTexture>>>; H2_LAYER_COUNT],
    pub(super) uv_scales: [[f32; 2]; SLOT_COUNT],
    pub(in crate::app::model_preview) detail: DetailComposition,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
