//! The GPU core every game draws through: the slots, the main program and the shared frame steps.

use super::*;
use super::super::common::sprite_gl::{SpriteGl, link_sprite_gl};
use super::super::gl::buffer::model_draw_range;
use super::super::gl::texture::{upload_black_cube, upload_black_flat, upload_placeholder_texture, upload_solid_volume};
use super::super::gl::target::H2Target;
use super::super::common::view::ce_flare_view;
use super::super::gl::link::link;
use super::super::gl::scope::PostScope;
use super::super::gl::target::ColorTarget;
use super::super::particles::{H3ParticleGl, ReachCheapGl};
use super::super::dx9::passes::H2Layer;
use super::super::dx9::draw::Dx9Gl;
use eframe::glow;

/// The GL state the preview draws with: the shared program, slots and
/// targets, and, until each game's drawing moves to its own folder, every
/// game's own state too.
pub(in crate::app::model_preview) struct RenderCore {
    /// The normals the gen3 albedo pass laid down, which Halo 3's dx9
    /// shadow apply reads: the gen3 targets' own, kept as they are remade.
    pub(in crate::app::model_preview) h3_normal_target: Option<super::super::dx9::shadow::ShadowNormalTarget>,
    /// What a gen3 array sampler reads with nothing of its own.
    pub(in crate::app::model_preview) black_array: Option<glow::NativeTexture>,
    /// As `Dx9Gl::h2_bloom`.
    pub(in crate::app::model_preview) sprite_gl: Option<Option<SpriteGl>>,
    /// As `Dx9Gl::h2_bloom`.
    pub(in crate::app::model_preview) particle_gl: Option<Option<H3ParticleGl>>,
    /// As `Dx9Gl::h2_bloom`.
    pub(in crate::app::model_preview) reach_cheap_gl: Option<Option<ReachCheapGl>>,
    /// Particles step once a frame, however many calls draw them.
    pub(in crate::app::model_preview) sprites_advanced: bool,
    /// Halo 2's `target_particle_distortion`, copied from the frame.
    pub(in crate::app::model_preview) h2_distortion_copy: Option<ColorTarget>,
    /// Whether this context's GLSL can draw the games' shaders, which every
    /// pipeline's scene needs; see `H2_GLSL_VERSION`.
    pub(in crate::app::model_preview) shaders_supported: bool,
    /// The DX9 pipeline's programs, targets and maps, which Halo 2 and
    /// Halo CE draw with and Halo 3's shadow draws through.
    pub(in crate::app::model_preview) dx9: Dx9Gl,
    /// Whether this context can draw the gen3 games' shaders: the translated
    /// shaders want GLSL 4.10.
    pub(in crate::app::model_preview) gen3_supported: bool,
    /// What a map nothing fills reads: an unmapped particle stage, a
    /// composite with no bloom built.
    pub(in crate::app::model_preview) black_flat: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) black_cube: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) black_volume: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) white_volume: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) scene: Option<H2Target>,
    pub(in crate::app::model_preview) program: glow::NativeProgram,
    pub(in crate::app::model_preview) slots: Vec<ModelGpuSlot>,
    pub(super) lru: ModelSlotLru,
    pub(in crate::app::model_preview) center: glow::NativeUniformLocation,
    pub(in crate::app::model_preview) scale: glow::NativeUniformLocation,
    pub(super) angles: glow::NativeUniformLocation,
    pub(in crate::app::model_preview) clip_scale: glow::NativeUniformLocation,
    pub(in crate::app::model_preview) depth_scale: glow::NativeUniformLocation,
    pub(in crate::app::model_preview) depth_map: glow::NativeUniformLocation,
    pub(in crate::app::model_preview) perspective: glow::NativeUniformLocation,
    /// Optional like the texture uniforms: a driver may strip the skinning
    /// path from a program whose draws never enable it.
    pub(in crate::app::model_preview) animated: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) bones: Option<glow::NativeUniformLocation>,
    pub(super) base_color: glow::NativeUniformLocation,
    pub(super) unlit: glow::NativeUniformLocation,
    pub(super) shaded: Option<glow::NativeUniformLocation>,
    pub(super) have_a: Option<glow::NativeUniformLocation>,
    pub(super) have_b: Option<glow::NativeUniformLocation>,
    pub(super) uv_scale_a: Option<glow::NativeUniformLocation>,
    pub(super) uv_scale_b: Option<glow::NativeUniformLocation>,
    pub(super) uv_scale_c: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) detail: Option<glow::NativeUniformLocation>,
    /// One sampler location per slot, in `TextureSlot` order.
    pub(in crate::app::model_preview) samplers: [Option<glow::NativeUniformLocation>; SLOT_COUNT],
    /// Bound to every slot a material lacks; see [`Self::bind_material`].
    pub(in crate::app::model_preview) placeholder: Option<glow::NativeTexture>,
}

impl RenderCore {
    /// Each simulation `geometry_id`'s slot keeps between frames, by name,
    /// for the refactor's G-sim gate.
    #[cfg(test)]
    pub(in crate::app::model_preview) fn for_each_simulation(&self, geometry_id: u64, visit: impl FnMut(&str, &dyn std::fmt::Debug)) {
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
    pub(super) fn slot_of(&self, geometry_id: u64) -> Option<usize> {
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

    pub(super) fn new(gl: &glow::Context) -> Result<Self, String> {
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
                black_cube: upload_black_cube(gl),
                particle_gl: None,
                reach_cheap_gl: None,
                sprites_advanced: false,
                h2_distortion_copy: None,
                black_volume: upload_solid_volume(gl, glow::TEXTURE_3D, [0, 0, 0, 255]),
                white_volume: upload_solid_volume(gl, glow::TEXTURE_3D, [255; 4]),
                dx9: Dx9Gl::new(gl),
                scene: None,
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
    pub(super) unsafe fn acquire_frame_slot(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) -> Option<usize> {
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
            self.dx9.ce_points_now = None;
            Some(slot)
        }
    }

    /// The shared program's state, camera and pose, then the ground grid.
    pub(super) unsafe fn begin_drawing(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
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
    pub(in crate::app::model_preview) unsafe fn draw_slot_batches(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        unsafe { self.draw_batches(gl, frame, false, Self::bound_materials(frame, &self.slots[slot].materials)) }
    }

    /// The wireframe over what shaded, in the shared program.
    pub(super) unsafe fn draw_wireframe(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
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

    /// The frame's end: fill polygons again, `scene` copied to where the
    /// frame belonged, and nothing left bound.
    pub(super) unsafe fn end_frame(&mut self, gl: &glow::Context, scene: Option<PostScope>) {
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
    pub(super) fn bound_materials<'a>(frame: &ModelGpuFrame, uploaded: &'a [MaterialGlTextures]) -> &'a [MaterialGlTextures] {
        if frame.textures.is_some() { uploaded } else { &[] }
    }

    /// With game shaders on, send the whole frame to the scene target. What it
    /// replaces is handed back for `finish_scene`.
    pub(super) unsafe fn begin_scene(
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
    pub(super) unsafe fn finish_scene(&self, gl: &glow::Context, state: PostScope) {
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

    /// Bind one material's textures, and tell the shader which slots are live.
    ///
    /// A slot with no texture is flagged absent, which makes the shader skip
    /// it, and gets the 1x1 placeholder bound rather than nothing. Leaving
    /// the slot on texture 0 is legal GL, but macOS's driver checks every
    /// active sampler at each draw, whether or not the shader branch reads it,
    /// and logs "unit 0 GLD_TEXTURE_INDEX_2D is unloadable" for any batch
    /// without a base map. That includes every batch while the textures are
    /// still loading.
    pub(super) unsafe fn bind_material(&self, gl: &glow::Context, material: Option<&MaterialGlTextures>) {
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

    pub(super) unsafe fn draw_batches(
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
