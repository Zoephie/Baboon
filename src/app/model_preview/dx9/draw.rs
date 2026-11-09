//! The DX9 pass machinery on the shared core: preparing the programs, binding a pass's
//! uniforms, drawing each material's passes in order, the texaccum fill, the frame copy and bloom.

use super::super::*;
use super::shadow::*;
use super::super::gl::buffer::model_draw_range;
use super::super::gl::target::H2Target;
use super::super::gl::program::Linked;
use super::super::gl::scope::PostScope;
use super::super::dx9::passes::{
    H2BlendEquation, H2BlendFactor, H2Compare, H2Cull, H2DefaultLight, H2DrawPass, H2Layer,
    H2_GLSL_VERSION, H2_REFLECTION_SCALE,
    h2_normalization_face,
};
use super::super::h2::bloom::H2Bloom;
use super::super::dx9::passes::H2Clock;
use super::super::dx9::stage::H2VertexStage;
use crate::core::h2_pixel_shader::SamplerKind;
use eframe::glow::{self, HasContext as _};

/// The DX9 pipeline's GL state on the shared core: the linked pass programs,
/// the pass targets, the frame copy, the gel, bloom and the self shadow. The
/// core owns it, not `Dx9Render`, because Halo 3's shadow draws through it.
pub(in crate::app::model_preview) struct Dx9Gl {
    /// Linked pass programs. `None` records a failure, so it is not retried
    /// every frame and the material keeps drawing as it did.
    pub(in crate::app::model_preview) h2_programs: HashMap<(H2VertexStage, String), Option<Rc<Linked>>>,
    pub(in crate::app::model_preview) h2_normalization_cube: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) h2_target: Option<H2Target>,
    pub(in crate::app::model_preview) h2_frame_copy: Option<H2Target>,
    /// The gel and `H2GelRows` of the frustum light drawing now.
    pub(in crate::app::model_preview) h2_gel: Option<(Option<glow::NativeTexture>, [[f32; 4]; 4])>,
    /// Made on first use; `Some(None)` once making it failed, so a driver
    /// that cannot is not asked again (and nothing leaks) every frame.
    pub(in crate::app::model_preview) h2_bloom: Option<Option<H2Bloom>>,
    /// As `h2_bloom`.
    pub(in crate::app::model_preview) h2_shadow_map: Option<Option<(glow::NativeFramebuffer, glow::NativeTexture)>>,
    /// The frame's CE point lights with those its effects have made, which
    /// only the drawing knows; none while effects make no lights.
    /// The style of the self shadow drawn last, which its passes and a later
    /// apply bind the light from.
    pub(in crate::app::model_preview) shadow: Option<ShadowStyle>,
    pub(in crate::app::model_preview) ce_points_now: Option<Vec<[[f32; 4]; ce::lights::CE_POINT_LIGHT_ROWS]>>,
}

impl Dx9Gl {
    pub(in crate::app::model_preview) unsafe fn new(gl: &glow::Context) -> Self {
        Self {
            h2_programs: HashMap::new(),
            h2_normalization_cube: unsafe { upload_normalization_cube(gl) },
            h2_target: None,
            h2_frame_copy: None,
            h2_gel: None,
            h2_bloom: None,
            h2_shadow_map: None,
            ce_points_now: None,
            shadow: None,
        }
    }
}

pub(in crate::app::model_preview) const H2_GEL_MASK_UNIT: u32 = 10;

pub(in crate::app::model_preview) fn gl_blend_factor(factor: H2BlendFactor) -> u32 {
    match factor {
        H2BlendFactor::Zero => glow::ZERO,
        H2BlendFactor::One => glow::ONE,
        H2BlendFactor::SrcColor => glow::SRC_COLOR,
        H2BlendFactor::OneMinusSrcColor => glow::ONE_MINUS_SRC_COLOR,
        H2BlendFactor::SrcAlpha => glow::SRC_ALPHA,
        H2BlendFactor::OneMinusSrcAlpha => glow::ONE_MINUS_SRC_ALPHA,
        H2BlendFactor::DstColor => glow::DST_COLOR,
        H2BlendFactor::OneMinusDstColor => glow::ONE_MINUS_DST_COLOR,
        H2BlendFactor::DstAlpha => glow::DST_ALPHA,
        H2BlendFactor::OneMinusDstAlpha => glow::ONE_MINUS_DST_ALPHA,
        H2BlendFactor::SrcAlphaSaturate => glow::SRC_ALPHA_SATURATE,
        H2BlendFactor::ConstantColor => glow::CONSTANT_COLOR,
        H2BlendFactor::OneMinusConstantColor => glow::ONE_MINUS_CONSTANT_COLOR,
        H2BlendFactor::ConstantAlpha => glow::CONSTANT_ALPHA,
        H2BlendFactor::OneMinusConstantAlpha => glow::ONE_MINUS_CONSTANT_ALPHA,
    }
}

pub(in crate::app::model_preview) fn gl_blend_equation(equation: H2BlendEquation) -> u32 {
    match equation {
        H2BlendEquation::Add => glow::FUNC_ADD,
        H2BlendEquation::Subtract => glow::FUNC_SUBTRACT,
        H2BlendEquation::ReverseSubtract => glow::FUNC_REVERSE_SUBTRACT,
        H2BlendEquation::Min => glow::MIN,
        H2BlendEquation::Max => glow::MAX,
    }
}

pub(in crate::app::model_preview) fn gl_compare(compare: H2Compare) -> u32 {
    match compare {
        H2Compare::Never => glow::NEVER,
        H2Compare::Less => glow::LESS,
        H2Compare::Equal => glow::EQUAL,
        H2Compare::LessOrEqual => glow::LEQUAL,
        H2Compare::Greater => glow::GREATER,
        H2Compare::NotEqual => glow::NOTEQUAL,
        H2Compare::GreaterOrEqual => glow::GEQUAL,
        H2Compare::Always => glow::ALWAYS,
    }
}

/// Where a batch sorts from. The game keys a part by the depth of its rigid
/// node, and a skinned part by node 0, so skinned parts keep their order among
/// themselves: they all share one point here, and the sort is stable.
pub(in crate::app::model_preview) fn h2_sort_points(preview: &RenderModelPreview) -> Vec<[f32; 3]> {
    let center = preview.vertices.iter().fold([0.0f32; 3], |sum, vertex| {
        std::array::from_fn(|axis| sum[axis] + vertex.position[axis])
    });
    let count = preview.vertices.len().max(1) as f32;
    let center = center.map(|sum| sum / count);
    preview
        .batches
        .iter()
        .map(|batch| {
            let start = batch.index_start as usize;
            let end = start + batch.index_count as usize;
            let indices = preview.indices.get(start..end).unwrap_or(&[]);
            let vertex = |index: &u32| preview.vertices.get(*index as usize);
            let rigid = indices.first().and_then(vertex).is_some_and(|first| {
                indices.iter().filter_map(vertex).all(|other| {
                    other.node_indices[0] == first.node_indices[0]
                        && other.node_weights[1..].iter().all(|weight| *weight <= 0.0)
                })
            });
            if !rigid || indices.is_empty() {
                return center;
            }
            let sum = indices.iter().filter_map(vertex).fold([0.0f32; 3], |sum, vertex| {
                std::array::from_fn(|axis| sum[axis] + vertex.position[axis])
            });
            sum.map(|sum| sum / indices.len() as f32)
        })
        .collect()
}

/// Visible batches farthest first; the eye looks along +Y in view space.
pub(in crate::app::model_preview) fn h2_draw_order(frame: &ModelGpuFrame, points: &[[f32; 3]]) -> Vec<usize> {
    let camera = &frame.camera;
    let depth = |index: usize| {
        let point = points.get(index).copied().unwrap_or(camera.center);
        let offset = std::array::from_fn(|axis| point[axis] - camera.center[axis]);
        rotate_view_vector(camera.yaw, camera.pitch, offset)[1]
    };
    let mut order = frame.visible_batches.clone();
    order.sort_by(|a, b| depth(*b).total_cmp(&depth(*a)));
    order
}

/// The cube `GLOBAL vector-normalization` names. Bilinear, as a direction
/// lookup wants, and small: only its directions matter.
pub(in crate::app::model_preview) unsafe fn upload_normalization_cube(gl: &glow::Context) -> Option<glow::NativeTexture> {
    const SIZE: usize = 64;
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
        for face in 0..6 {
            gl.tex_image_2d(
                glow::TEXTURE_CUBE_MAP_POSITIVE_X + face as u32,
                0,
                glow::RGBA8 as i32,
                SIZE as i32,
                SIZE as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&h2_normalization_face(face, SIZE))),
            );
        }
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_R, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, name, value as i32);
        }
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
        Some(texture)
    }
}

impl RenderCore {
    /// Halo 2's bloom over `bloom_order`, after the wireframe and the sprites.
    pub(in crate::app::model_preview) unsafe fn draw_dx9_bloom(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize, bloom_order: Option<Vec<usize>>) {
        unsafe {
            if let Some(order) = bloom_order {
                gl.use_program(Some(self.program));
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                self.draw_h2_bloom(gl, frame, slot, &order);
            }

        }
    }

    fn h2_passes(frame: &ModelGpuFrame, material: usize, layer: H2Layer) -> &[H2DrawPass] {
        frame
            .textures
            .as_ref()
            .and_then(|textures| textures.get(material))
            .map_or(&[], |material| material.h2_layers[layer as usize].as_slice())
    }

    /// Whether the model is camouflaged: camo needs the model's shader for it.
    pub(in crate::app::model_preview) fn h2_camo_on(&self, frame: &ModelGpuFrame) -> bool {
        frame.active_camo
            && (0..frame.preview.materials.len())
                .any(|material| self.h2_material_ready(frame, material, H2Layer::ActiveCamo))
    }

    /// Whether a material's passes stand in for the slots. Transparent ones
    /// always do. Opaque ones need game shaders on, and a template with a
    /// lightmap layer needs that layer to have prepared.
    pub(in crate::app::model_preview) fn h2_replaces_slots(&self, frame: &ModelGpuFrame, material: usize) -> bool {
        // Halo 3's and ODST's materials draw their shadow passes through here
        // too, and those stand in wherever the translated passes do.
        if frame.game_shaders && self.h3_material(frame, material).is_some() {
            return true;
        }
        if self.h2_material_ready(frame, material, H2Layer::Transparent) {
            return true;
        }
        if self.h2_camo_on(frame) {
            return self.h2_material_ready(frame, material, H2Layer::ActiveCamo);
        }
        if !frame.game_shaders {
            return false;
        }
        let lightmap = frame.h2_lightmap_layer();
        self.h2_material_ready(frame, material, lightmap)
            || self.h2_material_ready(frame, material, H2Layer::Prt)
            || (Self::h2_passes(frame, material, lightmap).is_empty()
                && (self.h2_material_ready(frame, material, H2Layer::Overlay)
                    || (frame
                        .textures
                        .as_ref()
                        .and_then(|textures| textures.get(material))
                        .is_none_or(|material| material.get(TextureSlot::Base).is_none())
                        && self.h2_material_ready(frame, material, H2Layer::SelfIllumination))))
    }

    /// Whether a material draws through its passes rather than the slots: it
    /// has some, and every one linked.
    fn h2_material_ready(&self, frame: &ModelGpuFrame, material: usize, layer: H2Layer) -> bool {
        let passes = Self::h2_passes(frame, material, layer);
        self.shaders_supported
            && !passes.is_empty()
            && (layer != H2Layer::Prt || (frame.prt && frame.preview.h2.prt.is_some()))
            && passes.iter().all(|pass| {
                self.dx9.h2_programs
                    .get(&(pass.stage, pass.fragment.clone()))
                    .is_some_and(Option::is_some)
            })
    }

    /// Link the dx9 pass programs the frame's materials name: Halo 2's and
    /// Halo CE's passes, and the shadow passes Halo 3 and ODST borrow.
    pub(in crate::app::model_preview) unsafe fn prepare_h2_programs(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if !self.shaders_supported {
            return;
        }
        let Some(textures) = frame.textures.as_ref() else {
            return;
        };
        let passes = textures.iter().flat_map(|material| material.h2_layers.iter().flatten());
        for pass in passes {
            let key = (pass.stage, pass.fragment.clone());
            if self.dx9.h2_programs.contains_key(&key) {
                continue;
            }
            let vertex = pass.stage.source(H2_GLSL_VERSION);
            let linked = unsafe { link_h2_program(gl, self.program, &vertex, &pass.fragment) };
            let program = match linked {
                Ok(program) => Some(Rc::new(Linked::new(program))),
                Err(error) => {
                    eprintln!("model preview: {error}");
                    None
                }
            };
            self.dx9.h2_programs.insert(key, program);
        }
    }

    /// Draw every texaccum pass into the target, sized to the live viewport,
    /// and put the framebuffer, viewport and scissor back exactly as found.
    pub(in crate::app::model_preview) unsafe fn fill_h2_texaccum(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) {
        let needed = order.iter().any(|&index| {
            frame.preview.batches.get(index).is_some_and(|batch| {
                let material = batch.material_index as usize;
                self.h2_material_ready(frame, material, H2Layer::Texaccum)
                    || self.h2_material_ready(frame, material, H2Layer::LightAlbedo)
            })
        });
        if !needed {
            return;
        }
        unsafe {
            let state = PostScope::capture(gl);
            let size = state.size();
            if self.dx9.h2_target.as_ref().is_none_or(|target| target.size != size) {
                if let Some(old) = self.dx9.h2_target.take() {
                    old.delete(gl);
                }
                self.dx9.h2_target = H2Target::new(gl, size[0], size[1]);
            }
            if let Some(target) = &self.dx9.h2_target {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.viewport(0, 0, size[0], size[1]);
                gl.disable(glow::SCISSOR_TEST);
                // The game draws texaccum against the scene's depth: a shader
                // may lay its depth down here alone, with its later passes
                // testing for equal.
                let scene_depth = self.scene.as_ref().map(|scene| scene.depth);
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_STENCIL_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(scene_depth.unwrap_or(target.depth)),
                );
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear_depth_f32(1.0);
                gl.clear(if scene_depth.is_some() {
                    glow::COLOR_BUFFER_BIT
                } else {
                    glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT
                });
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::Texaccum, None);
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::LightAlbedo, None);
            }
            state.restore(gl);
        }
    }

    /// Write the bloom layer to the frame's alpha, and add the glow from it.
    unsafe fn draw_h2_bloom(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) {
        let Some((framebuffer, color, size)) =
            self.scene.as_ref().map(|scene| (scene.framebuffer, scene.color, scene.size))
        else {
            return;
        };
        unsafe {
            gl.color_mask(false, false, false, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.color_mask(true, true, true, true);
            self.draw_h2_passes(gl, frame, slot, order, H2Layer::Bloom, None);
            // `sub_6C18F0`: the hologram shader draws while its control is up.
            let hologram = frame.preview.h2.hologram_control.as_deref().is_some_and(|name| {
                frame.function_values.as_ref().and_then(|values| values.get(name)).is_some_and(|value| *value > 0.0)
            });
            if hologram {
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::HologramBloom, None);
            }
            if let Some(bloom) = self.dx9.h2_bloom.get_or_insert_with(|| H2Bloom::new(gl, H2_GLSL_VERSION)) {
                bloom.apply(gl, framebuffer, color, size, frame.show_backfaces);
            }
        }
    }

    /// Keep what lies behind the model, for what shows through it: as
    /// `sub_6BD480` does, before anything of the model has drawn.
    pub(in crate::app::model_preview) unsafe fn copy_h2_frame(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, order: &[usize]) {
        let needed = order.iter().any(|&index| {
            frame.preview.batches.get(index).is_some_and(|batch| {
                let material = batch.material_index as usize;
                H2Layer::ALL.iter().any(|layer| {
                    Self::h2_passes(frame, material, *layer)
                        .iter()
                        .any(|pass| pass.textures.iter().any(|texture| texture.frame_copy))
                })
            })
        });
        let Some(scene) = self.scene.as_ref().filter(|_| needed) else {
            return;
        };
        let (framebuffer, [width, height]) = (scene.framebuffer, scene.size);
        unsafe {
            if self.dx9.h2_frame_copy.as_ref().is_none_or(|copy| copy.size != [width, height]) {
                if let Some(old) = self.dx9.h2_frame_copy.take() {
                    old.delete(gl);
                }
                self.dx9.h2_frame_copy = H2Target::new(gl, width, height);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            }
            if let Some(copy) = &self.dx9.h2_frame_copy {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(copy.framebuffer));
                gl.blit_framebuffer(
                    0,
                    0,
                    width,
                    height,
                    0,
                    0,
                    width,
                    height,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            }
        }
    }

    /// Draw Halo 2 materials through their passes, in `order`. Leaves the GL
    /// state the wireframe pass and the UI expect.
    pub(in crate::app::model_preview) unsafe fn draw_h2_passes(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
        layer: H2Layer,
        light: Option<&H2DefaultLight>,
    ) {
        let eye = h2_eye_position(&frame.camera);
        let mut drew = false;
        let camouflaged = self.h2_camo_on(frame);
        for &batch_index in order {
            let Some(batch) = frame.preview.batches.get(batch_index) else {
                continue;
            };
            let material = batch.material_index as usize;
            if !self.h2_material_ready(frame, material, layer)
                || (layer != H2Layer::Transparent && !self.h2_replaces_slots(frame, material))
            {
                continue;
            }
            let Some((byte_offset, index_count)) = model_draw_range(
                batch.index_start,
                batch.index_count,
                frame.preview.indices.len(),
            ) else {
                continue;
            };
            for (pass_index, pass) in Self::h2_passes(frame, material, layer).iter().enumerate() {
                if camouflaged && pass.hidden_by_camo {
                    continue;
                }
                if pass.numeric.as_ref().is_some_and(|numeric| !numeric.shows(&frame.outputs_of(material))) {
                    continue;
                }
                let key = (pass.stage, pass.fragment.clone());
                let Some(Some(program)) = self.dx9.h2_programs.get(&key).cloned() else {
                    continue;
                };
                drew = true;
                unsafe {
                    gl.use_program(Some(program.program));
                    self.bind_h2_uniforms(gl, frame, material, &program, pass, eye, light);
                    let textures = self.slots[slot]
                        .materials
                        .get(material)
                        .and_then(|material| material.h2_textures[layer as usize].get(pass_index));
                    for (index, texture) in pass.textures.iter().enumerate() {
                        let uploaded = textures.and_then(|list| list.get(index)).copied().flatten();
                        gl.active_texture(glow::TEXTURE0 + texture.slot);
                        match texture.kind {
                            SamplerKind::Texture2D => {
                                let target = self.dx9.h2_target.as_ref().map(|target| target.color);
                                let gel = self.dx9.h2_gel.and_then(|(texture, _)| texture);
                                let bound = if texture.gel {
                                    gel.or(self.placeholder)
                                } else if texture.texaccum {
                                    target.or(self.placeholder)
                                } else if texture.frame_copy {
                                    let copy = self.dx9.h2_frame_copy.as_ref();
                                    copy.map(|copy| copy.color).or(self.placeholder)
                                } else {
                                    uploaded.or(self.placeholder)
                                };
                                gl.bind_texture(glow::TEXTURE_2D, bound);
                            }
                            SamplerKind::Cube => gl.bind_texture(
                                glow::TEXTURE_CUBE_MAP,
                                if texture.normalization {
                                    self.dx9.h2_normalization_cube
                                } else {
                                    uploaded.or(self.black_cube)
                                },
                            ),
                            SamplerKind::Volume => {
                                let fallback = if texture.falloff {
                                    self.white_volume
                                } else {
                                    self.black_volume
                                };
                                gl.bind_texture(glow::TEXTURE_3D, uploaded.or(fallback));
                            }
                        }
                        let location = program.location(gl, &format!("s{}", texture.slot));
                        if location.is_some() {
                            gl.uniform_1_i32(location.as_ref(), texture.slot as i32);
                        }
                    }
                    match pass.blend {
                        Some(blend) => {
                            gl.enable(glow::BLEND);
                            gl.blend_equation(gl_blend_equation(blend.equation));
                            gl.blend_func(
                                gl_blend_factor(blend.source),
                                gl_blend_factor(blend.destination),
                            );
                            let [a, r, g, b] = blend.constant.map(|value| value as f32 / 255.0);
                            gl.blend_color(r, g, b, a);
                        }
                        None => gl.disable(glow::BLEND),
                    }
                    match pass.depth.compare {
                        Some(compare) => {
                            gl.enable(glow::DEPTH_TEST);
                            gl.depth_func(gl_compare(compare));
                        }
                        None => gl.disable(glow::DEPTH_TEST),
                    }
                    gl.depth_mask(pass.depth.write);
                    if pass.depth_bias != 0.0 {
                        gl.enable(glow::POLYGON_OFFSET_FILL);
                        gl.polygon_offset(0.0, pass.depth_bias * DEPTH_UNITS);
                    } else {
                        gl.disable(glow::POLYGON_OFFSET_FILL);
                    }
                    let [r, g, b, a] = pass.channels;
                    gl.color_mask(r, g, b, a);
                    match pass.cull {
                        H2Cull::Frame => {}
                        H2Cull::Back | H2Cull::Front => {
                            let front = pass.cull == H2Cull::Front;
                            gl.enable(glow::CULL_FACE);
                            gl.cull_face(if front { glow::FRONT } else { glow::BACK });
                        }
                        H2Cull::None => gl.disable(glow::CULL_FACE),
                    }
                    gl.draw_elements(
                        glow::TRIANGLES,
                        index_count,
                        glow::UNSIGNED_INT,
                        byte_offset,
                    );
                    if pass.cull != H2Cull::Frame {
                        gl.cull_face(glow::BACK);
                        if frame.show_backfaces {
                            gl.disable(glow::CULL_FACE);
                        } else {
                            gl.enable(glow::CULL_FACE);
                        }
                    }
                }
            }
        }
        if drew {
            unsafe {
                gl.use_program(Some(self.program));
                gl.disable(glow::BLEND);
                gl.disable(glow::POLYGON_OFFSET_FILL);
                gl.blend_equation(glow::FUNC_ADD);
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LESS);
                gl.depth_mask(true);
                gl.color_mask(true, true, true, true);
                gl.active_texture(glow::TEXTURE0);
            }
        }
    }

    unsafe fn bind_h2_uniforms(
        &self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        material: usize,
        program: &Linked,
        pass: &H2DrawPass,
        eye: [f32; 3],
        light: Option<&H2DefaultLight>,
    ) {
        unsafe {
            if matches!(pass.stage, H2VertexStage::CeModel | H2VertexStage::CeGlassLit) {
                let object = frame.preview.materials.get(material).map_or(0, |material| material.object);
                let location = program.location(gl, "u_ce_point");
                let points = self.dx9.ce_points_now.as_ref().unwrap_or(&frame.ce_points);
                let rows = points.get(usize::from(object)).copied().unwrap_or_default();
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            if matches!(pass.stage, H2VertexStage::ShadowCaster | H2VertexStage::ShadowMask) {
                let location = program.location(gl, "u_shadow_matrix");
                // Set by the shadow pass these stages draw within or after.
                let style = self.dx9.shadow.unwrap_or_else(|| ShadowStyle::h2(frame));
                let (toward, shadow_alpha) = (style.toward, style.alpha);
                let matrix = h2_shadow_matrix(&frame.preview, toward);
                gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix);
                // A caster draws into the map: bound then, it would be read
                // and written at once.
                if pass.stage == H2VertexStage::ShadowMask
                    && let Some(Some((_, map))) = self.dx9.h2_shadow_map
                {
                    gl.active_texture(glow::TEXTURE0 + H2_SHADOW_MAP_UNIT);
                    gl.bind_texture(glow::TEXTURE_2D, Some(map));
                    let location = program.location(gl, "u_shadow_map");
                    gl.uniform_1_i32(location.as_ref(), H2_SHADOW_MAP_UNIT as i32);
                }
                if let Some(normal) = self.h3_normal_target {
                    gl.active_texture(glow::TEXTURE0 + normal.unit);
                    gl.bind_texture(glow::TEXTURE_2D, Some(normal.texture));
                    let location = program.location(gl, "u_normal_target");
                    gl.uniform_1_i32(location.as_ref(), normal.unit as i32);
                    let location = program.location(gl, "u_shadow_toward");
                    gl.uniform_3_f32_slice(location.as_ref(), &toward);
                    let location = program.location(gl, "u_shadow_size");
                    gl.uniform_1_f32(location.as_ref(), normal.map_size as f32);
                    let location = program.location(gl, "u_shadow_alpha");
                    gl.uniform_1_f32(location.as_ref(), shadow_alpha);
                }
            }
            for (name, axis) in [
                ("u_view_right", [1.0, 0.0, 0.0]),
                ("u_view_up", [0.0, 0.0, 1.0]),
                ("u_view_back", [0.0, -1.0, 0.0]),
            ] {
                let location = program.location(gl, name);
                if location.is_some() {
                    let vector = unrotate_view_vector(frame.camera.yaw, frame.camera.pitch, axis);
                    gl.uniform_3_f32_slice(location.as_ref(), &vector);
                }
            }
            if let Some(prt) = &frame.preview.h2.prt {
                let location = program.location(gl, "u_prt");
                if location.is_some() {
                    let clusters = match &frame.h2_prt_clusters {
                        Some([lit, shadowed]) => {
                            if light.is_some() { shadowed } else { lit }
                        }
                        None => &prt.clusters,
                    };
                    gl.uniform_4_f32_slice(location.as_ref(), clusters.as_flattened());
                    let direction = if frame.h2_prt_clusters.is_some() {
                        frame.h2_object_light()
                    } else {
                        prt.light
                    };
                    let location = program.location(gl, "u_prt_light");
                    gl.uniform_3_f32_slice(location.as_ref(), &direction);
                }
            }
            let camera = &frame.camera;
            let set_1 = |gl: &glow::Context, program: &Linked, name: &str, value: f32| {
                let location = program.location(gl, name);
                gl.uniform_1_f32(location.as_ref(), value);
            };
            set_1(gl, program, "u_scale", camera.scale);
            set_1(gl, program, "u_depth_scale", camera.depth_scale);
            set_1(gl, program, "u_perspective", camera.perspective);
            set_1(gl, program, "u_reflection_scale", H2_REFLECTION_SCALE);
            set_1(gl, program, "u_animated", if frame.bones.is_some() { 1.0 } else { 0.0 });
            let location = program.location(gl, "u_light_direct");
            let follows = matches!(pass.stage, H2VertexStage::CeModel | H2VertexStage::CeGlassLit)
                && frame.light_follows_camera;
            let direct = if !follows && (frame.default_lights || !pass.stage.is_lightmap()) {
                pass.light_direct
            } else {
                frame.h2_object_light().map(|value| -value)
            };
            gl.uniform_3_f32_slice(location.as_ref(), &direct);
            let location = program.location(gl, "u_light_indirect");
            gl.uniform_3_f32_slice(location.as_ref(), &pass.light_indirect);
            let location = program.location(gl, "u_center");
            gl.uniform_3_f32_slice(location.as_ref(), &camera.center);
            let location = program.location(gl, "u_eye");
            gl.uniform_3_f32_slice(location.as_ref(), &eye);
            if let Some(copy) = &self.dx9.h2_frame_copy {
                let location = program.location(gl, "u_view_size");
                gl.uniform_2_f32(location.as_ref(), copy.size[0] as f32, copy.size[1] as f32);
            }
            let location = program.location(gl, "u_angles");
            gl.uniform_2_f32(location.as_ref(), camera.yaw, camera.pitch);
            let location = program.location(gl, "u_clip_scale");
            gl.uniform_2_f32_slice(location.as_ref(), &camera.clip_scale);
            let location = program.location(gl, "u_depth_map");
            gl.uniform_2_f32_slice(location.as_ref(), &camera.depth_map);
            if let Some(rows) = frame.bones.as_ref() {
                let location = program.location(gl, "u_bones");
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            let location = program.location(gl, "u_texture_rows");
            let (outputs, change_colors) = (frame.outputs_of(material), frame.change_colors_of(material));
            let clock = H2Clock { time: frame.time, values: frame.function_values_of(material) };
            let rows = pass.texture_rows_at(clock, &outputs);
            gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            let (tint_facing, tint_glancing) = pass.tints_at(clock);
            let location = program.location(gl, "u_tint_facing");
            gl.uniform_4_f32_slice(location.as_ref(), &tint_facing);
            let location = program.location(gl, "u_tint_glancing");
            gl.uniform_4_f32_slice(location.as_ref(), &tint_glancing);
            for (register, value) in &pass.constants {
                let mut value = *value;
                pass.animate_constant(*register, &mut value, clock);
                if let Some(animation) = &pass.ce_animation {
                    animation.constant(
                        *register,
                        &mut value,
                        frame.time,
                        &change_colors,
                        &outputs,
                    );
                    if animation.eye_register == Some(*register) {
                        let [x, y, z] =
                            unrotate_view_vector(frame.camera.yaw, frame.camera.pitch, [0.0, -1.0, 0.0]);
                        value = [x, y, z, 1.0];
                    }
                }
                for (changed, component, channel) in &pass.change_colors {
                    if changed == register {
                        value[*component] = change_colors[*channel][*component];
                    }
                }
                for (changed, component) in &pass.camo_amounts {
                    if changed == register {
                        value[*component] = if frame.active_camo { 1.0 } else { 0.0 };
                    }
                }
                if let Some(light) = light {
                    for (changed, component) in &pass.light_colors {
                        if changed == register {
                            value[*component] = light.color[*component];
                        }
                    }
                }
                let location = program.location(gl, &format!("c{register}"));
                let location = location.or_else(|| program.location(gl, &format!("ce_registers[{register}]")));
                gl.uniform_4_f32_slice(location.as_ref(), &value);
            }
            if let Some(light) = light {
                let location = program.location(gl, "u_light_position");
                gl.uniform_3_f32_slice(location.as_ref(), &light.position);
                let location = program.location(gl, "u_light_falloff");
                gl.uniform_1_f32(location.as_ref(), 0.5 / light.radius);
            }
            if let Some((_, rows)) = &self.dx9.h2_gel {
                let location = program.location(gl, "u_gel_rows");
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            if let (H2VertexStage::LightGel, Some(location), Some((gel, _))) =
                (pass.stage, program.location(gl, "u_gel_map"), &self.dx9.h2_gel)
            {
                gl.active_texture(glow::TEXTURE0 + H2_GEL_MASK_UNIT);
                gl.bind_texture(glow::TEXTURE_2D, gel.or(self.placeholder));
                gl.uniform_1_i32(Some(&location), H2_GEL_MASK_UNIT as i32);
            }
            let location = program.location(gl, "zwriteout");
            gl.uniform_4_f32(location.as_ref(), 1.0, 1.0, 1.0, 1.0);
        }
    }
}
