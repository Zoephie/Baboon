//! The pipeline's stages: the frame begun, the albedo and transparent passes, camo and the shield.

use super::*;
use crate::app::model_preview::gl::blit::blit_depth;

/// The object function values behind the shield's two quantities.
pub(in crate::app::model_preview) const H3_SHIELD_DAMAGE: &str = "current_shield_damage";

pub(in crate::app::model_preview) const H3_OVERSHIELD: &str = "object_overshield_amount";

/// Whether the shield is showing, whose plasma scrolls with game time.
pub(in crate::app::model_preview) fn gen3_shield_showing(
    values: Option<&functions::FunctionValues>,
) -> bool {
    let reading = |name| {
        values
            .and_then(|values| values.get(name))
            .copied()
            .unwrap_or(0.0)
    };
    reading(H3_SHIELD_DAMAGE) > 0.0 || reading(H3_OVERSHIELD) > 0.0
}

impl RenderCore {
    /// A material's translated passes, if this context can run them.
    pub(in crate::app::model_preview) fn gen3_material<'a>(
        &self,
        frame: &'a ModelGpuFrame,
        material: usize,
    ) -> Option<&'a Arc<Gen3Material>> {
        if !self.gen3_supported {
            return None;
        }
        frame.textures.as_ref()?.get(material)?.h3.as_ref()
    }

    pub(in crate::app::model_preview) fn gen3_draws(&self, frame: &ModelGpuFrame) -> bool {
        frame.game_shaders
            && self.gen3_supported
            && frame
                .textures
                .iter()
                .any(|textures| textures.iter().any(|texture| texture.h3.is_some()))
    }

    /// `cloth_render_opaque` builds its mesh from where the sheet is now.
    pub(in crate::app::model_preview) unsafe fn move_cloth(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) {
        if frame.preview.cloth.is_empty() {
            return;
        }
        let uploaded = &mut self.slots[slot];
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let written = uploaded
            .simulations
            .cloth
            .advance(&frame.preview.cloth, bones, frame.time);
        for (first, moved) in written {
            unsafe { uploaded.rewrite_vertices(gl, frame, first, moved) };
        }
    }
}

/// What a gen3 frame's stages share: the frame and its slot, the targets'
/// size and names, the batches the pipeline draws, and the screen effects.
pub(super) struct Gen3Frame<'a> {
    pub(super) gl: &'a glow::Context,
    pub(super) frame: &'a ModelGpuFrame,
    pub(super) slot: usize,
    pub(super) order: &'a [usize],
    size: [i32; 2],
    framebuffer: glow::NativeFramebuffer,
    albedo_target: glow::NativeFramebuffer,
    pub(super) accumulation_target: glow::NativeFramebuffer,
    accumulation: glow::NativeTexture,
    pub(super) batches: Vec<Gen3Batch>,
    bloomed: bool,
    screen: gen3::effects::screen::Gen3ScreenSettings,
    shaders: Vec<gen3::effects::screen::Gen3ScreenShader>,
}

impl Gen3State {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn draw_pass(
        &mut self,
        core: &mut RenderCore,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        batch: &RenderModelPreviewBatch,
        material: &Gen3Material,
        pass: &Arc<Gen3Pass>,
        shield: Option<&Gen3Shield>,
    ) {
        let range = model_draw_range(
            batch.index_start,
            batch.index_count,
            frame.preview.indices.len(),
        );
        if range.is_none() && !pass.fullscreen {
            return;
        }
        unsafe {
            self.with_program(core, gl, pass, |state, this, program| {
                gl.use_program(Some(program.program));
                let size = state
                    .targets
                    .as_ref()
                    .map_or([1, 1], |targets| targets.size);
                let draw_values = state.hooks.draw_values(frame, batch, material, pass);
                for (name, ty) in &pass.uniforms {
                    let location = program.location(gl, name);
                    if location.is_none() {
                        continue;
                    }
                    match name.as_str() {
                        "View_Projection" => {
                            let matrix = state
                                .shadow_view_projection
                                .unwrap_or_else(|| view_projection(&frame.camera));
                            gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix);
                            continue;
                        }
                        "View" => {
                            gl.uniform_matrix_4_f32_slice(
                                location.as_ref(),
                                false,
                                &view_matrix(frame),
                            );
                            continue;
                        }
                        "Camera_To_World" => {
                            gl.uniform_matrix_4_f32_slice(
                                location.as_ref(),
                                false,
                                &camera_to_world(frame),
                            );
                            continue;
                        }
                        "Nodes" | "vs_model_skinning_matrices" => {
                            state.nodes(gl, frame, location.as_ref(), ty);
                            continue;
                        }
                        "simple_lights" => {
                            if let Some(lit) = &state.light {
                                gl.uniform_4_f32_slice(location.as_ref(), lit.rows.as_flattened());
                            } else if let Some(inline) = state
                                .hooks
                                .inline_simple_lights(material, &state.inline_lights)
                            {
                                gl.uniform_4_f32_slice(location.as_ref(), inline);
                            }
                            continue;
                        }
                        "Shadow_Projection" => {
                            if let Some(lit) = &state.light {
                                gl.uniform_matrix_4_f32_slice(
                                    location.as_ref(),
                                    false,
                                    &lit.projection,
                                );
                            }
                            continue;
                        }
                        _ => {}
                    }
                    if state.hooks.set_matrix(
                        gl,
                        location.as_ref(),
                        frame,
                        material,
                        name,
                        state.shadow_view_projection,
                    ) {
                        continue;
                    }
                    // The dynamic pass borrows this constant for its gel's xform.
                    if name == "p_lighting_constant_4" && state.light.is_some() {
                        gl.uniform_4_f32(location.as_ref(), 1.0, 1.0, 0.0, 0.0);
                        continue;
                    }
                    let change_color = material
                        .change_colors
                        .iter()
                        .find(|(uniform, _)| uniform == name);
                    let change_color = change_color.map(|(_, channel)| {
                        let [r, g, b] =
                            frame.change_colors_of(batch.material_index as usize)[*channel];
                        Gen3Value::Floats([r, g, b, 1.0])
                    });
                    let light = state
                        .hooks
                        .light_value(material, name, &state.inline_lights);
                    let mut value = change_color
                        .or(light)
                        .or_else(|| (state.frame_values)(frame, name, size, state.stops))
                        .or_else(|| {
                            draw_values
                                .as_ref()
                                .and_then(|values| values.get(name))
                                .cloned()
                        })
                        .or_else(|| material.values.get(name).cloned());
                    let object = batch.material_index as usize;
                    let screen_values = state.screen_alpha.map(|alpha| {
                        let mut values = frame
                            .function_values_of(object)
                            .cloned()
                            .unwrap_or_default();
                        values.insert("alpha".to_owned(), alpha);
                        values
                    });
                    let values = screen_values.as_ref().or(frame.function_values_of(object));
                    let owner = frame
                        .preview
                        .materials
                        .get(object)
                        .map_or(0, |material| material.object);
                    for animation in material
                        .animations
                        .iter()
                        .filter(|animation| animation.uniform == *name)
                    {
                        let mut slot = match value {
                            Some(Gen3Value::Floats(slot)) => slot,
                            _ => [0.0; 4],
                        };
                        animation.apply(&mut slot, frame.time, owner, values);
                        value = Some(Gen3Value::Floats(slot));
                    }
                    if let Some(value) = value {
                        set_value(gl, location.as_ref(), ty, &value);
                    }
                    if let Some(value) = state
                        .pass_values
                        .as_ref()
                        .and_then(|values| values.get(name.as_str()))
                    {
                        set_value(gl, location.as_ref(), ty, &Gen3Value::Floats(*value));
                    }
                    let settings = frame
                        .preview
                        .h3
                        .shield
                        .as_ref()
                        .or(shield.map(|shield| &shield.settings));
                    if let Some(value) = shield
                        .and(settings)
                        .and_then(|settings| settings.values.get(name))
                    {
                        set_value(gl, location.as_ref(), ty, value);
                    }
                    if name == "shield_dynamic_quantities" {
                        let reading = |name: &str| {
                            frame
                                .function_values
                                .as_ref()
                                .and_then(|values| values.get(name))
                                .copied()
                                .unwrap_or(0.0)
                        };
                        // `render_object_context`: the overshield in fifths of a byte.
                        let quantities = [
                            frame.time,
                            reading(H3_SHIELD_DAMAGE),
                            reading(H3_OVERSHIELD),
                            0.0,
                        ];
                        gl.uniform_4_f32_slice(location.as_ref(), &quantities);
                    }
                    if name == "bound_sphere" {
                        let (low, high) = (frame.preview.bounds_min, frame.preview.bounds_max);
                        let center: [f32; 3] =
                            std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
                        let reach: [f32; 3] = std::array::from_fn(|axis| high[axis] - center[axis]);
                        let radius =
                            (reach[0] * reach[0] + reach[1] * reach[1] + reach[2] * reach[2])
                                .sqrt();
                        gl.uniform_4_f32(
                            location.as_ref(),
                            center[0],
                            center[1],
                            center[2],
                            radius,
                        );
                    }
                }
                let targets = state.targets.as_ref();
                let uploaded = this.slots[slot]
                    .h3_textures
                    .materials
                    .get(batch.material_index as usize);
                for (unit, (name, kind)) in pass.textures.iter().enumerate() {
                    gl.active_texture(glow::TEXTURE0 + unit as u32);
                    let texture = match name.as_str() {
                        "albedo_texture" | "ps_view_albedo" => {
                            targets.map(|targets| targets.albedo)
                        }
                        "normal_texture" | "ps_view_normal" => {
                            targets.map(|targets| targets.normal)
                        }
                        // No level casts shadows into Halo 4's mask: white.
                        "ps_view_shadow_mask" => state.shadow_mask.or(this.placeholder),
                        "GlobalTexture_scene_ldr_texture" => targets.map(|targets| targets.behind),
                        // Halo 4's camo: what the object hides.
                        "sceneTexture_texture" => state
                            .hooks
                            .camo_scene()
                            .or(targets.map(|targets| targets.behind)),
                        "depth_buffer" | "psDepthSampler_texture" => state.depth_copy,
                        "shadow_mask_texture" => state.shadow_mask,
                        other
                            if material
                                .screen_maps
                                .iter()
                                .any(|map| other.ends_with(map.as_str())) =>
                        {
                            targets.map(|targets| targets.behind)
                        }
                        "UserParameterTexture_dynamic_light_gel_texture" => {
                            state.light.as_ref().and_then(|light| light.gel)
                        }
                        other => uploaded
                            .and_then(|map| {
                                map.get(&state.uploaded_key(frame, batch, material, other))
                                    .or_else(|| map.get(other))
                            })
                            .copied()
                            .or_else(|| {
                                let noise = other.strip_prefix("LocalTexture_").unwrap_or(other);
                                this.slots[slot].h3_textures.shield.get(noise).copied()
                            }),
                    };
                    match kind {
                        TextureKind::Line => gl.bind_texture(glow::TEXTURE_1D, texture),
                        TextureKind::Flat => {
                            gl.bind_texture(glow::TEXTURE_2D, texture.or(this.placeholder))
                        }
                        TextureKind::Cube => {
                            gl.bind_texture(glow::TEXTURE_CUBE_MAP, texture.or(this.black_cube))
                        }
                        TextureKind::Volume => {
                            gl.bind_texture(glow::TEXTURE_3D, texture.or(this.black_volume))
                        }
                        // The kit's arrays (light probes, the dominant light's
                        // intensity, water) are built by the game, never authored.
                        TextureKind::FlatArray => {
                            gl.bind_texture(glow::TEXTURE_2D_ARRAY, this.black_array)
                        }
                        // Reach's per-vertex light probes, which only level
                        // geometry's `static_per_vertex` reads.
                        TextureKind::Bytes => gl.bind_texture(glow::TEXTURE_BUFFER, None),
                    }
                    let location = program.location(gl, name);
                    gl.uniform_1_i32(location.as_ref(), unit as i32);
                }
                let group = frame
                    .preview
                    .materials
                    .get(batch.material_index as usize)
                    .map(|known| known.shader_group);
                let two_sided =
                    !frame.show_backfaces && state.hooks.two_sided(material, pass, group);
                if two_sided {
                    gl.disable(glow::CULL_FACE);
                }
                match range {
                    Some((offset, count)) if !pass.fullscreen => {
                        gl.draw_elements(glow::TRIANGLES, count, glow::UNSIGNED_INT, offset);
                    }
                    _ => gl.draw_arrays(glow::TRIANGLES, 0, 3),
                }
                if two_sided {
                    gl.enable(glow::CULL_FACE);
                }
            });
        }
    }

    /// The object's passes, between the preview's own draw and the scene's end.
    /// The gen3 pipeline, `game` filling in where the gen3 games differ.
    pub(in crate::app::model_preview) unsafe fn draw(
        &mut self,
        core: &mut RenderCore,
        game: &mut dyn Gen3Game,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) {
        unsafe {
            let Some(drawing) = self.begin_frame(core, game, gl, frame, slot, order) else {
                return;
            };
            self.albedo_pass(core, &drawing);
            self.albedo_decals(core, &drawing);
            game.after_albedo(
                &mut Gen3Cx {
                    gl,
                    frame,
                    core,
                    slot,
                    state: self,
                },
                drawing.albedo_target,
                drawing.size,
            );
            game.self_shadow(
                &mut Gen3Cx {
                    gl,
                    frame,
                    core,
                    slot,
                    state: self,
                },
                &drawing.batches,
                order,
                drawing.framebuffer,
            );
            let blended = self.static_lighting(core, game, &drawing);
            self.dynamic_lights(core, game, &drawing);
            game.screen_lights(
                &mut Gen3Cx {
                    gl,
                    frame,
                    core,
                    slot,
                    state: self,
                },
                drawing.accumulation_target,
            );
            let sprites = self.begin_transparents(core, game, &drawing);
            self.blended_transparents(core, game, &drawing, blended, sprites);
            let mut bloom = self.camo(core, game, &drawing);
            self.pass_values = None;
            self.shield_impact(core, &drawing);
            game.shield(&mut Gen3Cx {
                gl,
                frame,
                core,
                slot,
                state: self,
            });
            // `c_player_view::render`: after the transparents, before the bloom
            // and the tone curve, so these are in the same light as the scene.
            if sprites {
                self.draw_sprites(core, game, gl, frame, slot, SpriteStage::Rest);
                game.late_sprites(&mut Gen3Cx {
                    gl,
                    frame,
                    core,
                    slot,
                    state: self,
                });
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(drawing.framebuffer));
            if drawing.bloomed && bloom.is_none() {
                bloom = game.bloom(
                    &mut Gen3Cx {
                        gl,
                        frame,
                        core,
                        slot,
                        state: self,
                    },
                    drawing.framebuffer,
                    drawing.accumulation,
                    drawing.size,
                );
            }
            let (framebuffer, accumulation, screen, size) = (
                drawing.framebuffer,
                drawing.accumulation,
                drawing.screen,
                drawing.size,
            );
            game.composite(
                &mut Gen3Cx {
                    gl,
                    frame,
                    core,
                    slot,
                    state: self,
                },
                framebuffer,
                accumulation,
                bloom,
                screen,
                size,
            );
            self.stops = 0.0;
            self.finish_picture(
                core,
                game,
                gl,
                frame,
                slot,
                framebuffer,
                size,
                &screen,
                &drawing.shaders,
            );
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
        }
    }

    /// The frame's targets, sized to the scene, and what the pipeline draws
    /// with: the slot's maps, the cloth moved, what camo hides, the batches
    /// with the game's own taken out, and the screen effects sampled. `None`
    /// when there is nothing to draw into.
    unsafe fn begin_frame<'a>(
        &mut self,
        core: &mut RenderCore,
        game: &mut dyn Gen3Game,
        gl: &'a glow::Context,
        frame: &'a ModelGpuFrame,
        slot: usize,
        order: &'a [usize],
    ) -> Option<Gen3Frame<'a>> {
        // Direct3D 11 always filters across a cube's faces; OpenGL has to be asked.
        unsafe { gl.enable(glow::TEXTURE_CUBE_MAP_SEAMLESS) };
        let scene = core.scene.as_ref()?;
        let (size, depth, framebuffer) = (scene.size, scene.depth, scene.framebuffer);
        unsafe {
            if self
                .targets
                .as_ref()
                .is_none_or(|targets| targets.size != size)
            {
                if let Some(old) = self.targets.take() {
                    old.delete(gl);
                }
                self.targets = Gen3Targets::new(gl, size, depth);
            }
            let Some(targets) = self.targets.as_ref() else {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                return None;
            };
            let (albedo_target, accumulation_target) =
                (targets.albedo_framebuffer, targets.accumulation_framebuffer);
            let accumulation = targets.accumulation;
            self.upload(core, gl, frame, slot);
            core.move_cloth(gl, frame, slot);
            if frame.active_camo {
                // What the object hides, before anything of it lands. The copy
                // is sRGB so that sampling it reads the linear light it was.
                let behind = self.targets.as_ref().map(|targets| targets.behind);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.bind_texture(glow::TEXTURE_2D, behind);
                gl.copy_tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::SRGB8_ALPHA8,
                    0,
                    0,
                    size[0],
                    size[1],
                    0,
                );
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            gl.disable(glow::BLEND);
            gen3_set_cull(gl, frame);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(true, true, true, true);
            let batches: Vec<_> = order
                .iter()
                .filter_map(|&index| frame.preview.batches.get(index))
                .filter_map(|batch| {
                    let material = core.gen3_material(frame, batch.material_index as usize)?;
                    Some((batch.clone(), Arc::clone(material)))
                })
                .collect();
            let mut batches = batches;
            game.split(frame, &mut batches);
            let bloomed = frame.bloom;
            // `screen_effect_sample`, ahead of the scene: Reach's
            // `c_player_view::render` hands its exposure stops to
            // `setup_camera_fx_parameters` before anything draws.
            let (screen, shaders) = {
                let uploaded = &mut core.slots[slot];
                let values = |object: u8| frame.function_values_of_object(object);
                let view = ce_flare_view(&frame.camera);
                let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
                let effects = &frame.preview.gen3.screen_effects;
                gen3::effects::screen::gen3_screen_settings(
                    effects,
                    values,
                    &mut uploaded.simulations.effect_onsets,
                    bones,
                    &view,
                    frame.time,
                )
            };
            self.stops = game.stops(&screen);
            Some(Gen3Frame {
                gl,
                frame,
                slot,
                order,
                size,
                framebuffer,
                albedo_target,
                accumulation_target,
                accumulation,
                batches,
                bloomed,
                screen,
                shaders,
            })
        }
    }

    /// The albedo pass, into its own target. It lays depth down for the lit
    /// pass to meet exactly; the preview's own draw rounds differently.
    unsafe fn albedo_pass(&mut self, core: &mut RenderCore, drawing: &Gen3Frame<'_>) {
        let Gen3Frame {
            gl, frame, slot, ..
        } = *drawing;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(drawing.albedo_target));
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            for (batch, material) in &drawing.batches {
                if let Some(pass) = material.albedo.as_ref() {
                    self.draw_pass(core, gl, frame, slot, batch, material, pass, None);
                }
            }
        }
    }

    /// `render_albedo_decals`, with `_z_buffer_mode_decals`' bias.
    unsafe fn albedo_decals(&mut self, core: &mut RenderCore, drawing: &Gen3Frame<'_>) {
        let Gen3Frame {
            gl, frame, slot, ..
        } = *drawing;
        unsafe {
            gl.enable(glow::BLEND);
            gl.depth_mask(false);
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(DECAL_SLOPE_BIAS, DECAL_DEPTH_BIAS * DEPTH_UNITS);
            for (batch, material) in drawing
                .batches
                .iter()
                .filter(|(_, material)| material.decal && !material.after_lighting)
            {
                gen3_set_blend(gl, material.blend);
                // `render_mesh_part_default` (Reach X360 0x826754D8): a
                // decal that leaves the specular mask writes the albedo's RGB
                // alone. One that modulates writes alpha too, under a separate
                // alpha blend this preview does not set [inferred: it keeps
                // the colour blend's].
                let keeps_mask =
                    material.dialect() == Gen3Dialect::Reach && !material.decal_modulates_specular;
                gl.color_mask_draw_buffer(0, true, true, true, !keeps_mask);
                // The blend's mask opens every target, so the normals'
                // is set after it. Halo 4's normal decals (`DECAL_OUTPUT_NORMAL`)
                // write the encoded normal's two channels, z and w masked
                // (`decals.fxh`); nothing else here writes normals.
                let normals = material.dialect() == Gen3Dialect::Halo4
                    && material.lit.pixel.contains("o_target1");
                gl.color_mask_draw_buffer(1, normals, normals, false, false);
                self.draw_pass(core, gl, frame, slot, batch, material, &material.lit, None);
            }
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.color_mask(true, true, true, true);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.disable(glow::BLEND);
        }
    }

    /// `c_transparency_renderer`'s setup: the scene's depth copied for the
    /// soft sprites, and `submit_distortions` into their own target. Whether
    /// the frame draws its object's sprites.
    unsafe fn begin_transparents(
        &mut self,
        core: &mut RenderCore,
        game: &mut dyn Gen3Game,
        drawing: &Gen3Frame<'_>,
    ) -> bool {
        let Gen3Frame {
            gl, frame, slot, ..
        } = *drawing;
        let sprites = core.draws_object_sprites(frame);
        unsafe {
            if sprites {
                if let Some(targets) = self.targets.as_ref() {
                    blit_depth(
                        gl,
                        targets.accumulation_framebuffer,
                        targets.depth_copy_framebuffer,
                        targets.size,
                    );
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                    self.depth_copy = Some(targets.depth_copy);
                    // `submit_distortions`.
                    self.distorted = false;
                    if frame
                        .preview
                        .particles
                        .iter()
                        .any(|system| system.look.shader.distortion.is_some())
                    {
                        gl.bind_framebuffer(
                            glow::FRAMEBUFFER,
                            Some(targets.distortion_framebuffer),
                        );
                        gl.clear_color(0.0, 0.0, 0.0, 0.0);
                        gl.clear(glow::COLOR_BUFFER_BIT);
                        self.draw_sprites(core, game, gl, frame, slot, SpriteStage::Distortions);
                        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(drawing.accumulation_target));
                    }
                }
            }
        }
        sprites
    }

    /// `c_transparency_renderer`: what blends, far to near, sprites among
    /// the materials, then the sprites nearer than all of them and the
    /// distortions applied.
    unsafe fn blended_transparents(
        &mut self,
        core: &mut RenderCore,
        game: &mut dyn Gen3Game,
        drawing: &Gen3Frame<'_>,
        blended: Vec<&Gen3Batch>,
        sprites: bool,
    ) {
        let Gen3Frame {
            gl,
            frame,
            slot,
            size,
            ..
        } = *drawing;
        unsafe {
            // Each with the pass it draws and the blend it draws under: its
            // own, or the alpha blend of an `opaque_alpha_blend` fringe.
            let fringes = drawing.batches.iter().filter_map(|(batch, material)| {
                let fringe = material.fringe.as_ref()?;
                Some((batch, material, fringe, Gen3Blend::AlphaBlend))
            });
            let mut blended: Vec<_> = blended
                .into_iter()
                .map(|(batch, material)| (batch, material, &material.lit, material.blend))
                .chain(fringes)
                .map(|(batch, material, pass, blend)| {
                    (
                        layered_distance(material.sort_layer, batch_distance(frame, batch)),
                        batch,
                        material,
                        pass,
                        blend,
                    )
                })
                .collect();
            blended.sort_by(|(a, ..), (b, ..)| b.total_cmp(a));
            for (distance, batch, material, pass, blend) in blended {
                game.blended(
                    &mut Gen3Cx {
                        gl,
                        frame,
                        core,
                        slot,
                        state: self,
                    },
                    distance,
                    size,
                );
                if sprites {
                    self.draw_sprites(
                        core,
                        game,
                        gl,
                        frame,
                        slot,
                        SpriteStage::FartherThan(distance),
                    );
                }
                blended_material_state(frame.show_backfaces, core.slots[slot].vertex_array)
                    .apply(gl);
                gen3_set_blend(gl, blend);
                if material.decal {
                    gl.enable(glow::POLYGON_OFFSET_FILL);
                    gl.polygon_offset(DECAL_SLOPE_BIAS, DECAL_DEPTH_BIAS * DEPTH_UNITS);
                }
                self.draw_pass(core, gl, frame, slot, batch, material, pass, None);
                gl.disable(glow::POLYGON_OFFSET_FILL);
            }
            game.blended(
                &mut Gen3Cx {
                    gl,
                    frame,
                    core,
                    slot,
                    state: self,
                },
                f32::NEG_INFINITY,
                size,
            );
            if sprites {
                self.draw_sprites(
                    core,
                    game,
                    gl,
                    frame,
                    slot,
                    SpriteStage::FartherThan(f32::NEG_INFINITY),
                );
                if self.distorted {
                    self.apply_distortions(gl, drawing.accumulation_target, size);
                    gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
                }
            }
        }
    }

    /// The game's camo, over what lies behind it. Camo shows what is behind
    /// it, which the game blooms the same with or without the object, so
    /// with bloom on the bloom is built here, under the camo. The bloom,
    /// when built.
    unsafe fn camo(
        &mut self,
        core: &mut RenderCore,
        game: &mut dyn Gen3Game,
        drawing: &Gen3Frame<'_>,
    ) -> Option<glow::NativeTexture> {
        let Gen3Frame {
            gl,
            frame,
            slot,
            size,
            ..
        } = *drawing;
        let mut bloom = None;
        unsafe {
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.enable(glow::BLEND);
            gl.depth_mask(false);
            // Neither the camo nor the shield can count on a transparent having
            // put the model's own geometry back after the lights and sprites.
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            if let Some(opaque) = game.camo(&mut Gen3Cx {
                gl,
                frame,
                core,
                slot,
                state: self,
            }) {
                // A preview's backdrop has no bloom of its own, so what camo
                // hides is left dark for it.
                if drawing.bloomed {
                    gl.blend_func(glow::ZERO, glow::ZERO);
                    for (batch, material) in &drawing.batches {
                        if let Some(pass) = material.camo.as_ref() {
                            self.draw_pass(core, gl, frame, slot, batch, material, pass, None);
                        }
                    }
                    bloom = game.bloom(
                        &mut Gen3Cx {
                            gl,
                            frame,
                            core,
                            slot,
                            state: self,
                        },
                        drawing.accumulation_target,
                        drawing.accumulation,
                        size,
                    );
                    gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
                    gl.enable(glow::DEPTH_TEST);
                    gl.enable(glow::BLEND);
                }
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                // `render_object_callback`: fully transparent camo has no base
                // to blend over, and draws opaque in its place.
                if opaque {
                    gl.disable(glow::BLEND);
                    gl.depth_mask(true);
                }
                for (batch, material) in &drawing.batches {
                    if let Some(pass) = material.camo.as_ref() {
                        self.draw_pass(core, gl, frame, slot, batch, material, pass, None);
                    }
                }
                gl.enable(glow::BLEND);
                gl.depth_mask(false);
            }
        }
        bloom
    }

    /// `render_object_context`'s shield impact, over every lit part.
    unsafe fn shield_impact(&mut self, core: &mut RenderCore, drawing: &Gen3Frame<'_>) {
        let Gen3Frame {
            gl, frame, slot, ..
        } = *drawing;
        if !gen3_shield_showing(frame.function_values.as_ref()) {
            return;
        }
        unsafe {
            gen3_set_blend(gl, Gen3Blend::Additive);
            for (batch, material) in &drawing.batches {
                if let Some(shield) = material.shield.as_ref() {
                    // `render_object_shield_impact`: by the settings that win.
                    let settings = frame.preview.h3.shield.as_ref().unwrap_or(&shield.settings);
                    let pass = shield
                        .procedural
                        .as_ref()
                        .filter(|_| settings.procedural)
                        .unwrap_or(&shield.pass);
                    self.draw_pass(core, gl, frame, slot, batch, material, pass, Some(shield));
                }
            }
            gl.color_mask(true, true, true, true);
        }
    }

    /// The object's depth, which tells a composite what it covers, copied
    /// from the lit target; `framebuffer` left bound.
    pub(in crate::app::model_preview) unsafe fn object_depth(
        &mut self,
        gl: &glow::Context,
        framebuffer: glow::NativeFramebuffer,
    ) -> Option<glow::NativeTexture> {
        self.targets.as_ref().map(|targets| unsafe {
            blit_depth(
                gl,
                targets.accumulation_framebuffer,
                targets.depth_copy_framebuffer,
                targets.size,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            targets.depth_copy
        })
    }

    /// `c_render_shield::render_object_internal`: every part that takes the
    /// shield, through 61 or 119, added over what lies behind with no depth
    /// written and no face culled.
    pub(in crate::app::model_preview) unsafe fn draw_reach_shield(
        &mut self,
        core: &mut RenderCore,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) {
        let Some(shield) = frame.preview.reach.shield.clone() else {
            return;
        };
        let damage = frame
            .function_values
            .as_ref()
            .and_then(|values| values.get(H3_SHIELD_DAMAGE))
            .copied()
            .unwrap_or(0.0);
        if !shield.settings.showing(damage) {
            return;
        }
        let parts = super::super::shield::shield_parts(core, frame);
        if parts.is_empty() {
            return;
        }
        let (low, high) = (frame.preview.bounds_min, frame.preview.bounds_max);
        let center = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let state = shield
            .settings
            .state(damage, frame.time, frame.function_values.as_ref());
        let values = shield.settings.values(&state, center);
        unsafe { self.draw_shield_parts(core, gl, frame, slot, &parts, shield.pass(), values) }
    }
}
