//! The DX9 pass machinery on the shared core: preparing the programs, binding a pass's
//! uniforms, drawing each material's passes in order, the texaccum fill and the frame copy.

use super::super::common::streams::VertexStreamKind;
use super::super::dx9::passes::Dx9Clock;
use super::super::dx9::passes::{
    DX9_REFLECTION_SCALE, Dx9BlendEquation, Dx9BlendFactor, Dx9Compare, Dx9Cull, Dx9DrawPass,
    Dx9Layer, Dx9Light, PASS_GLSL_VERSION, normalization_face,
};
use super::super::dx9::stage::Dx9VertexStage;
use super::super::gl::buffer::model_draw_range;
use super::super::gl::pass_state::PassState;
use super::super::gl::program::Linked;
use super::super::gl::scope::PostScope;
use super::super::gl::target::SceneTarget;
use super::super::load::has_vertex_stream;
use super::super::{
    DEPTH_UNITS, ModelGpuFrame, RenderCore, RenderModelPreview, TextureSlot, eye_position,
    link_pass_program, rotate_view_vector, unrotate_view_vector,
};
use super::binds::Dx9Binds;
use super::shadow::{SHADOW_MAP_UNIT, ShadowStyle, shadow_matrix};
use crate::core::h2_pixel_shader::SamplerKind;
use eframe::glow::{self, HasContext as _};
use std::collections::HashMap;
use std::rc::Rc;

/// The DX9 pipeline's GL state on the shared core: the linked pass programs,
/// the pass targets, the frame copy, the gel, bloom and the self shadow. The
/// core owns it, not a `Dx9Pipeline`, because Halo 3's shadow draws through it.
pub(in crate::app::model_preview) struct Dx9Gl {
    /// Linked pass programs. `None` records a failure, so it is not retried
    /// every frame and the material keeps drawing as it did.
    pub(in crate::app::model_preview) pass_programs:
        HashMap<(Dx9VertexStage, String), Option<Rc<Linked>>>,
    pub(in crate::app::model_preview) normalization_cube: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) texaccum_target: Option<SceneTarget>,
    pub(in crate::app::model_preview) frame_copy: Option<SceneTarget>,
    /// The gel and `H2GelRows` of the frustum light drawing now.
    pub(in crate::app::model_preview) gel_now: Option<(Option<glow::NativeTexture>, [[f32; 4]; 4])>,
    /// Made on first use; `Some(None)` once making it failed, so a driver
    /// that cannot is not asked again (and nothing leaks) every frame.
    pub(in crate::app::model_preview) shadow_map:
        Option<Option<(glow::NativeFramebuffer, glow::NativeTexture)>>,
    /// The style of the self shadow drawn last, which its passes and a later
    /// apply bind the light from.
    pub(in crate::app::model_preview) shadow: Option<ShadowStyle>,
}

impl Dx9Gl {
    pub(in crate::app::model_preview) unsafe fn new(gl: &glow::Context) -> Self {
        Self {
            pass_programs: HashMap::new(),
            normalization_cube: unsafe { upload_normalization_cube(gl) },
            texaccum_target: None,
            frame_copy: None,
            gel_now: None,
            shadow_map: None,
            shadow: None,
        }
    }
}

pub(in crate::app::model_preview) const GEL_UNIT: u32 = 10;

pub(in crate::app::model_preview) fn gl_blend_factor(factor: Dx9BlendFactor) -> u32 {
    match factor {
        Dx9BlendFactor::Zero => glow::ZERO,
        Dx9BlendFactor::One => glow::ONE,
        Dx9BlendFactor::SrcColor => glow::SRC_COLOR,
        Dx9BlendFactor::OneMinusSrcColor => glow::ONE_MINUS_SRC_COLOR,
        Dx9BlendFactor::SrcAlpha => glow::SRC_ALPHA,
        Dx9BlendFactor::OneMinusSrcAlpha => glow::ONE_MINUS_SRC_ALPHA,
        Dx9BlendFactor::DstColor => glow::DST_COLOR,
        Dx9BlendFactor::OneMinusDstColor => glow::ONE_MINUS_DST_COLOR,
        Dx9BlendFactor::DstAlpha => glow::DST_ALPHA,
        Dx9BlendFactor::OneMinusDstAlpha => glow::ONE_MINUS_DST_ALPHA,
        Dx9BlendFactor::SrcAlphaSaturate => glow::SRC_ALPHA_SATURATE,
        Dx9BlendFactor::ConstantColor => glow::CONSTANT_COLOR,
        Dx9BlendFactor::OneMinusConstantColor => glow::ONE_MINUS_CONSTANT_COLOR,
        Dx9BlendFactor::ConstantAlpha => glow::CONSTANT_ALPHA,
        Dx9BlendFactor::OneMinusConstantAlpha => glow::ONE_MINUS_CONSTANT_ALPHA,
    }
}

pub(in crate::app::model_preview) fn gl_blend_equation(equation: Dx9BlendEquation) -> u32 {
    match equation {
        Dx9BlendEquation::Add => glow::FUNC_ADD,
        Dx9BlendEquation::Subtract => glow::FUNC_SUBTRACT,
        Dx9BlendEquation::ReverseSubtract => glow::FUNC_REVERSE_SUBTRACT,
        Dx9BlendEquation::Min => glow::MIN,
        Dx9BlendEquation::Max => glow::MAX,
    }
}

pub(in crate::app::model_preview) fn gl_compare(compare: Dx9Compare) -> u32 {
    match compare {
        Dx9Compare::Never => glow::NEVER,
        Dx9Compare::Less => glow::LESS,
        Dx9Compare::Equal => glow::EQUAL,
        Dx9Compare::LessOrEqual => glow::LEQUAL,
        Dx9Compare::Greater => glow::GREATER,
        Dx9Compare::NotEqual => glow::NOTEQUAL,
        Dx9Compare::GreaterOrEqual => glow::GEQUAL,
        Dx9Compare::Always => glow::ALWAYS,
    }
}

/// A pass's own state, set before it draws: its blend, depth test and write,
/// depth bias, channels and face culling, each as the pass names it.
pub(in crate::app::model_preview) fn dx9_pass_state(pass: &Dx9DrawPass) -> PassState {
    let state = match pass.blend {
        Some(blend) => {
            let [alpha, red, green, blue] = blend.constant.map(|value| value as f32 / 255.0);
            PassState::new()
                .enable(glow::BLEND)
                .blend_equation(gl_blend_equation(blend.equation))
                .blend_func(
                    gl_blend_factor(blend.source),
                    gl_blend_factor(blend.destination),
                )
                .blend_color([red, green, blue, alpha])
        }
        None => PassState::new().disable(glow::BLEND),
    };
    let state = match pass.depth.compare {
        Some(compare) => state
            .enable(glow::DEPTH_TEST)
            .depth_func(gl_compare(compare)),
        None => state.disable(glow::DEPTH_TEST),
    };
    let state = state.depth_mask(pass.depth.write);
    let state = if pass.depth_bias != 0.0 {
        state
            .enable(glow::POLYGON_OFFSET_FILL)
            .polygon_offset(0.0, pass.depth_bias * DEPTH_UNITS)
    } else {
        state.disable(glow::POLYGON_OFFSET_FILL)
    };
    let state = state.color_mask(pass.channels);
    match pass.cull {
        Dx9Cull::Frame => state,
        Dx9Cull::Back => state.enable(glow::CULL_FACE).cull_face(glow::BACK),
        Dx9Cull::Front => state.enable(glow::CULL_FACE).cull_face(glow::FRONT),
        Dx9Cull::None => state.disable(glow::CULL_FACE),
    }
}

/// After a pass that culled its own way: the back face named again, and
/// culling as the frame shows the model.
pub(in crate::app::model_preview) fn dx9_after_pass_state(
    pass: &Dx9DrawPass,
    show_backfaces: bool,
) -> PassState {
    if pass.cull == Dx9Cull::Frame {
        return PassState::new();
    }
    PassState::new()
        .cull_face(glow::BACK)
        .culling_unless(show_backfaces)
}

/// After the passes drew: the shared program, and the state the wireframe
/// pass and the UI expect.
pub(in crate::app::model_preview) fn dx9_passes_done_state(
    program: glow::NativeProgram,
) -> PassState {
    PassState::new()
        .use_program(program)
        .disable(glow::BLEND)
        .disable(glow::POLYGON_OFFSET_FILL)
        .blend_equation(glow::FUNC_ADD)
        .enable(glow::DEPTH_TEST)
        .depth_func(glow::LESS)
        .depth_mask(true)
        .color_mask([true; 4])
        .active_texture(glow::TEXTURE0)
}

/// Where a batch sorts from. The game keys a part by the depth of its rigid
/// node, and a skinned part by node 0, so skinned parts keep their order among
/// themselves: they all share one point here, and the sort is stable.
pub(in crate::app::model_preview) fn sort_points(preview: &RenderModelPreview) -> Vec<[f32; 3]> {
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
            let sum = indices
                .iter()
                .filter_map(vertex)
                .fold([0.0f32; 3], |sum, vertex| {
                    std::array::from_fn(|axis| sum[axis] + vertex.position[axis])
                });
            sum.map(|sum| sum / indices.len() as f32)
        })
        .collect()
}

/// Visible batches farthest first; the eye looks along +Y in view space.
pub(in crate::app::model_preview) fn draw_order(
    frame: &ModelGpuFrame,
    points: &[[f32; 3]],
) -> Vec<usize> {
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
pub(in crate::app::model_preview) unsafe fn upload_normalization_cube(
    gl: &glow::Context,
) -> Option<glow::NativeTexture> {
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
                glow::PixelUnpackData::Slice(Some(&normalization_face(face, SIZE))),
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
    fn dx9_passes_of(frame: &ModelGpuFrame, material: usize, layer: Dx9Layer) -> &[Dx9DrawPass] {
        frame
            .textures
            .as_ref()
            .and_then(|textures| textures.get(material))
            .map_or(&[], |material| {
                material.dx9_layers[layer as usize].as_slice()
            })
    }

    /// Whether the model is camouflaged: camo needs the model's shader for it.
    pub(in crate::app::model_preview) fn dx9_camo_on(&self, frame: &ModelGpuFrame) -> bool {
        frame.active_camo
            && (0..frame.preview.materials.len())
                .any(|material| self.dx9_material_ready(frame, material, Dx9Layer::ActiveCamo))
    }

    /// Whether a material's passes stand in for the slots. Transparent ones
    /// always do. Opaque ones need game shaders on, and a template with a
    /// lightmap layer needs that layer to have prepared.
    pub(in crate::app::model_preview) fn dx9_replaces_slots(
        &self,
        frame: &ModelGpuFrame,
        material: usize,
    ) -> bool {
        if self.dx9_material_ready(frame, material, Dx9Layer::Transparent) {
            return true;
        }
        if self.dx9_camo_on(frame) {
            return self.dx9_material_ready(frame, material, Dx9Layer::ActiveCamo);
        }
        if !frame.game_shaders {
            return false;
        }
        let lightmap = frame.dx9_lightmap_layer();
        self.dx9_material_ready(frame, material, lightmap)
            || self.dx9_material_ready(frame, material, Dx9Layer::Prt)
            || (Self::dx9_passes_of(frame, material, lightmap).is_empty()
                && (self.dx9_material_ready(frame, material, Dx9Layer::Overlay)
                    || (frame
                        .textures
                        .as_ref()
                        .and_then(|textures| textures.get(material))
                        .is_none_or(|material| material.get(TextureSlot::Base).is_none())
                        && self.dx9_material_ready(frame, material, Dx9Layer::SelfIllumination))))
    }

    /// Whether a material draws through its passes rather than the slots: it
    /// has some, and every one linked.
    fn dx9_material_ready(&self, frame: &ModelGpuFrame, material: usize, layer: Dx9Layer) -> bool {
        let passes = Self::dx9_passes_of(frame, material, layer);
        self.shaders_supported
            && !passes.is_empty()
            && (layer != Dx9Layer::Prt
                || (frame.prt && has_vertex_stream(&frame.preview, VertexStreamKind::Dx9Prt)))
            && passes.iter().all(|pass| {
                self.dx9
                    .pass_programs
                    .get(&(pass.stage, pass.fragment.clone()))
                    .is_some_and(Option::is_some)
            })
    }

    /// Link the dx9 pass programs the frame's materials name: Halo 2's and
    /// Halo CE's passes, and the shadow passes Halo 3 and ODST borrow.
    pub(in crate::app::model_preview) unsafe fn prepare_dx9_programs(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
    ) {
        if !self.shaders_supported {
            return;
        }
        let Some(textures) = frame.textures.as_ref() else {
            return;
        };
        let passes = textures
            .iter()
            .flat_map(|material| material.dx9_layers.iter().flatten());
        for pass in passes {
            let key = (pass.stage, pass.fragment.clone());
            if self.dx9.pass_programs.contains_key(&key) {
                continue;
            }
            let vertex = pass.stage.source(PASS_GLSL_VERSION);
            let linked = unsafe { link_pass_program(gl, self.program, &vertex, &pass.fragment) };
            let program = match linked {
                Ok(program) => Some(Rc::new(Linked::new(program))),
                Err(error) => {
                    eprintln!("model preview: {error}");
                    None
                }
            };
            self.dx9.pass_programs.insert(key, program);
        }
    }

    /// Draw every texaccum pass into the target, sized to the live viewport,
    /// and put the framebuffer, viewport and scissor back exactly as found.
    pub(in crate::app::model_preview) unsafe fn fill_dx9_texaccum(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
        own: &dyn Dx9Binds,
    ) {
        let needed = order.iter().any(|&index| {
            frame.preview.batches.get(index).is_some_and(|batch| {
                let material = batch.material_index as usize;
                self.dx9_material_ready(frame, material, Dx9Layer::Texaccum)
                    || self.dx9_material_ready(frame, material, Dx9Layer::LightAlbedo)
            })
        });
        if !needed {
            return;
        }
        unsafe {
            let state = PostScope::capture(gl);
            let size = state.size();
            if self
                .dx9
                .texaccum_target
                .as_ref()
                .is_none_or(|target| target.size != size)
            {
                if let Some(old) = self.dx9.texaccum_target.take() {
                    old.delete(gl);
                }
                self.dx9.texaccum_target = SceneTarget::new(gl, size[0], size[1]);
            }
            if let Some(target) = &self.dx9.texaccum_target {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.viewport(0, 0, size[0], size[1]);
                PassState::new().disable(glow::SCISSOR_TEST).apply(gl);
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
                self.draw_dx9_layer(gl, frame, slot, order, Dx9Layer::Texaccum, None, own);
                self.draw_dx9_layer(gl, frame, slot, order, Dx9Layer::LightAlbedo, None, own);
            }
            state.restore(gl);
        }
    }

    /// Keep what lies behind the model, for what shows through it: as
    /// `sub_6BD480` does, before anything of the model has drawn.
    pub(in crate::app::model_preview) unsafe fn copy_dx9_frame(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        order: &[usize],
    ) {
        let needed = order.iter().any(|&index| {
            frame.preview.batches.get(index).is_some_and(|batch| {
                let material = batch.material_index as usize;
                Dx9Layer::ALL.iter().any(|layer| {
                    Self::dx9_passes_of(frame, material, *layer)
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
            if self
                .dx9
                .frame_copy
                .as_ref()
                .is_none_or(|copy| copy.size != [width, height])
            {
                if let Some(old) = self.dx9.frame_copy.take() {
                    old.delete(gl);
                }
                self.dx9.frame_copy = SceneTarget::new(gl, width, height);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            }
            if let Some(copy) = &self.dx9.frame_copy {
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
    pub(in crate::app::model_preview) unsafe fn draw_dx9_layer(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
        layer: Dx9Layer,
        light: Option<&Dx9Light>,
        own: &dyn Dx9Binds,
    ) {
        let eye = eye_position(&frame.camera);
        let mut drew = false;
        let camouflaged = self.dx9_camo_on(frame);
        for &batch_index in order {
            let Some(batch) = frame.preview.batches.get(batch_index) else {
                continue;
            };
            let material = batch.material_index as usize;
            if !self.dx9_material_ready(frame, material, layer)
                || (layer != Dx9Layer::Transparent && !self.passes_replace_slots(frame, material))
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
            for (pass_index, pass) in Self::dx9_passes_of(frame, material, layer)
                .iter()
                .enumerate()
            {
                if camouflaged && pass.hidden_by_camo {
                    continue;
                }
                if pass
                    .numeric
                    .as_ref()
                    .is_some_and(|numeric| !numeric.shows(&frame.outputs_of(material)))
                {
                    continue;
                }
                let key = (pass.stage, pass.fragment.clone());
                let Some(Some(program)) = self.dx9.pass_programs.get(&key).cloned() else {
                    continue;
                };
                drew = true;
                unsafe {
                    PassState::new().use_program(program.program).apply(gl);
                    self.bind_dx9_uniforms(gl, frame, material, &program, pass, eye, light, own);
                    let textures = self.slots[slot]
                        .materials
                        .get(material)
                        .and_then(|material| material.dx9_textures[layer as usize].get(pass_index));
                    for (index, texture) in pass.textures.iter().enumerate() {
                        let uploaded = textures.and_then(|list| list.get(index)).copied().flatten();
                        let (target, bound) = match texture.kind {
                            SamplerKind::Texture2D => {
                                let target =
                                    self.dx9.texaccum_target.as_ref().map(|target| target.color);
                                let gel = self.dx9.gel_now.and_then(|(texture, _)| texture);
                                let bound = if texture.gel {
                                    gel.or(self.placeholder)
                                } else if texture.texaccum {
                                    target.or(self.placeholder)
                                } else if texture.frame_copy {
                                    let copy = self.dx9.frame_copy.as_ref();
                                    copy.map(|copy| copy.color).or(self.placeholder)
                                } else {
                                    uploaded.or(self.placeholder)
                                };
                                (glow::TEXTURE_2D, bound)
                            }
                            SamplerKind::Cube => (
                                glow::TEXTURE_CUBE_MAP,
                                if texture.normalization {
                                    self.dx9.normalization_cube
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
                                (glow::TEXTURE_3D, uploaded.or(fallback))
                            }
                        };
                        let state = PassState::new()
                            .active_texture(glow::TEXTURE0 + texture.slot)
                            .bind_texture(target, bound);
                        // A sampler the program compiled away keeps its unit unset.
                        let location = program.location(gl, &format!("s{}", texture.slot));
                        match location {
                            Some(location) => {
                                state.uniform_1_i32(Some(location), texture.slot as i32)
                            }
                            None => state,
                        }
                        .apply(gl);
                    }
                    dx9_pass_state(pass).apply(gl);
                    gl.draw_elements(
                        glow::TRIANGLES,
                        index_count,
                        glow::UNSIGNED_INT,
                        byte_offset,
                    );
                    dx9_after_pass_state(pass, frame.show_backfaces).apply(gl);
                }
            }
        }
        if drew {
            unsafe { dx9_passes_done_state(self.program).apply(gl) }
        }
    }

    unsafe fn bind_dx9_uniforms(
        &self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        material: usize,
        program: &Linked,
        pass: &Dx9DrawPass,
        eye: [f32; 3],
        light: Option<&Dx9Light>,
        own: &dyn Dx9Binds,
    ) {
        unsafe {
            own.bind_own_lights(gl, frame, material, program, pass);
            if matches!(
                pass.stage,
                Dx9VertexStage::ShadowCaster | Dx9VertexStage::ShadowMask
            ) {
                let location = program.location(gl, "u_shadow_matrix");
                // Set by the shadow pass these stages draw within or after.
                let style = self.dx9.shadow.unwrap_or_else(|| ShadowStyle::dx9(frame));
                let (toward, shadow_alpha) = (style.toward, style.alpha);
                let matrix = shadow_matrix(&frame.preview, toward);
                gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix);
                // A caster draws into the map: bound then, it would be read
                // and written at once.
                if pass.stage == Dx9VertexStage::ShadowMask
                    && let Some(Some((_, map))) = self.dx9.shadow_map
                {
                    let location = program.location(gl, "u_shadow_map");
                    PassState::new()
                        .sampler(SHADOW_MAP_UNIT, glow::TEXTURE_2D, Some(map), location)
                        .apply(gl);
                }
                if let Some(normal) = style.normal {
                    let location = program.location(gl, "u_normal_target");
                    PassState::new()
                        .sampler(
                            normal.unit,
                            glow::TEXTURE_2D,
                            Some(normal.texture),
                            location,
                        )
                        .apply(gl);
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
            own.bind_own_vertex_lighting(gl, frame, program, light);
            let camera = &frame.camera;
            let set_1 = |gl: &glow::Context, program: &Linked, name: &str, value: f32| {
                let location = program.location(gl, name);
                gl.uniform_1_f32(location.as_ref(), value);
            };
            set_1(gl, program, "u_scale", camera.scale);
            set_1(gl, program, "u_depth_scale", camera.depth_scale);
            set_1(gl, program, "u_perspective", camera.perspective);
            set_1(gl, program, "u_reflection_scale", DX9_REFLECTION_SCALE);
            set_1(
                gl,
                program,
                "u_animated",
                if frame.bones.is_some() { 1.0 } else { 0.0 },
            );
            let location = program.location(gl, "u_light_direct");
            let follows = own.light_follows_camera(frame, pass);
            let direct = if !follows && (frame.default_lights || !pass.stage.is_lightmap()) {
                pass.light_direct
            } else {
                frame.dx9_object_light().map(|value| -value)
            };
            gl.uniform_3_f32_slice(location.as_ref(), &direct);
            let location = program.location(gl, "u_light_indirect");
            gl.uniform_3_f32_slice(location.as_ref(), &pass.light_indirect);
            let location = program.location(gl, "u_center");
            gl.uniform_3_f32_slice(location.as_ref(), &camera.center);
            let location = program.location(gl, "u_eye");
            gl.uniform_3_f32_slice(location.as_ref(), &eye);
            if let Some(copy) = &self.dx9.frame_copy {
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
            let (outputs, change_colors) =
                (frame.outputs_of(material), frame.change_colors_of(material));
            let clock = Dx9Clock {
                time: frame.time,
                values: frame.function_values_of(material),
            };
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
                    animation.constant(*register, &mut value, frame.time, &change_colors, &outputs);
                    if animation.eye_register() == Some(*register) {
                        let [x, y, z] = unrotate_view_vector(
                            frame.camera.yaw,
                            frame.camera.pitch,
                            [0.0, -1.0, 0.0],
                        );
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
                let location =
                    location.or_else(|| program.location(gl, &format!("ce_registers[{register}]")));
                gl.uniform_4_f32_slice(location.as_ref(), &value);
            }
            if let Some(light) = light {
                let location = program.location(gl, "u_light_position");
                gl.uniform_3_f32_slice(location.as_ref(), &light.position);
                let location = program.location(gl, "u_light_falloff");
                gl.uniform_1_f32(location.as_ref(), 0.5 / light.radius);
            }
            if let Some((_, rows)) = &self.dx9.gel_now {
                let location = program.location(gl, "u_gel_rows");
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            if let (Dx9VertexStage::LightGel, Some(location), Some((gel, _))) = (
                pass.stage,
                program.location(gl, "u_gel_map"),
                &self.dx9.gel_now,
            ) {
                PassState::new()
                    .sampler(
                        GEL_UNIT,
                        glow::TEXTURE_2D,
                        gel.or(self.placeholder),
                        Some(location),
                    )
                    .apply(gl);
            }
            let location = program.location(gl, "zwriteout");
            gl.uniform_4_f32(location.as_ref(), 1.0, 1.0, 1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::DEPTH_UNITS;
    use super::super::super::dx9::passes::{Dx9Blend, Dx9Depth};
    use super::super::super::gl::pass_state::StateCall;
    use super::*;

    /// A blended, biased, front-culled pass sets its state in the order the
    /// game's does, with the blend constant's ARGB read as RGBA; a pass that
    /// culls by the frame names no face and has nothing to put back.
    #[test]
    fn a_pass_sets_its_blend_depth_bias_channels_and_culling_in_order() {
        let depth = Dx9Depth {
            compare: Some(Dx9Compare::LessOrEqual),
            write: false,
        };
        let mut pass = Dx9DrawPass::new(Dx9VertexStage::CeMeter, String::new(), depth);
        pass.blend = Some(Dx9Blend {
            equation: Dx9BlendEquation::Add,
            source: Dx9BlendFactor::ConstantColor,
            destination: Dx9BlendFactor::One,
            constant: [255, 0, 51, 102],
        });
        pass.depth_bias = 2.0;
        pass.channels = [true, true, true, false];
        pass.cull = Dx9Cull::Front;
        assert_eq!(
            dx9_pass_state(&pass).calls(),
            [
                StateCall::Enable(glow::BLEND),
                StateCall::BlendEquation(glow::FUNC_ADD),
                StateCall::BlendFunc(glow::CONSTANT_COLOR, glow::ONE),
                StateCall::BlendColor([0.0, 0.2, 0.4, 1.0]),
                StateCall::Enable(glow::DEPTH_TEST),
                StateCall::DepthFunc(glow::LEQUAL),
                StateCall::DepthMask(false),
                StateCall::Enable(glow::POLYGON_OFFSET_FILL),
                StateCall::PolygonOffset(0.0, 2.0 * DEPTH_UNITS),
                StateCall::ColorMask([true, true, true, false]),
                StateCall::Enable(glow::CULL_FACE),
                StateCall::CullFace(glow::FRONT),
            ]
        );
        assert_eq!(
            dx9_after_pass_state(&pass, false).calls(),
            [
                StateCall::CullFace(glow::BACK),
                StateCall::Enable(glow::CULL_FACE)
            ]
        );
        pass.cull = Dx9Cull::Frame;
        assert!(dx9_after_pass_state(&pass, false).calls().is_empty());
        assert!(
            !dx9_pass_state(&pass)
                .calls()
                .iter()
                .any(|call| matches!(call, StateCall::CullFace(_)))
        );
    }
}
