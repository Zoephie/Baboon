//! What a pass's uniforms read: the frame's values, the view and node matrices, and the slot's uploaded maps.

use super::super::super::{
    HashMap, ModelGpuCamera, ModelGpuFrame, RenderCore, RenderModelPreviewBatch, eye_position,
    rotate_view_vector, unrotate_view_vector,
};
use super::lighting::{
    DOMINANT_DIRECTION, DOMINANT_INTENSITY, EXPOSURE, dominant_light, frame_probe,
    lighting_constants,
};
use super::{
    Gen3Animated, Gen3Material, Gen3State, Gen3Value, ImageTexels, Scalar, Type,
    upload_image_linear,
};
use eframe::glow;
use eframe::glow::HasContext as _;

/// Uploaded textures of a geometry, by material then name.
#[derive(Default)]
pub(in crate::app::model_preview) struct Gen3SlotTextures {
    id: Option<u64>,
    pub(super) materials: Vec<HashMap<String, glow::NativeTexture>>,
    /// The shield's noise, by uniform.
    pub(super) shield: HashMap<String, glow::NativeTexture>,
    /// Each of `h3.lights`' gels, of `id`.
    pub(in crate::app::model_preview) gels: Vec<Option<glow::NativeTexture>>,
}

/// `clip_position` as the matrix `mul(position, View_Projection)` wants: GLSL's
/// columns are HLSL's rows here, so this is a plain column matrix.
pub(in crate::app::model_preview) fn view_projection(camera: &ModelGpuCamera) -> [f32; 16] {
    let scale = camera.scale;
    let clip = |vector: [f32; 3], offset: [f32; 3]| {
        let view = rotate_view_vector(camera.yaw, camera.pitch, vector);
        let origin = rotate_view_vector(camera.yaw, camera.pitch, offset);
        (view, origin)
    };
    let mut matrix = [0.0; 16];
    let center = camera.center.map(|axis| -axis * scale);
    for axis in 0..3 {
        let mut unit = [0.0; 3];
        unit[axis] = scale;
        let (view, _) = clip(unit, center);
        matrix[axis * 4] = view[0] * camera.clip_scale[0];
        matrix[axis * 4 + 1] = view[2] * camera.clip_scale[1];
        matrix[axis * 4 + 2] = view[1] * camera.depth_scale * camera.depth_map[0];
        matrix[axis * 4 + 3] = view[1] * camera.perspective;
    }
    let (_, origin) = clip([0.0; 3], center);
    matrix[12] = origin[0] * camera.clip_scale[0];
    matrix[13] = origin[2] * camera.clip_scale[1];
    matrix[14] = origin[1] * camera.depth_scale * camera.depth_map[0] + camera.depth_map[1];
    matrix[15] = 1.0 + origin[1] * camera.perspective;
    matrix
}

/// Where one image of an animated bitmap is uploaded under.
pub(super) fn frame_key(uniform: &str, frame: usize) -> String {
    format!("{uniform}#{frame}")
}

/// What the engine's globals hold for an object of a gen3 game, by uniform
/// name, at `size` and the frame's exposure `stops`. Each game gives its own.
pub(in crate::app::model_preview) type FrameValues =
    fn(&ModelGpuFrame, &str, [i32; 2], f32) -> Option<Gen3Value>;

/// Halo 3's and ODST's [`FrameValues`], which read no stops.
pub(in crate::app::model_preview) fn halo3_frame_values(
    frame: &ModelGpuFrame,
    name: &str,
    size: [i32; 2],
    _stops: f32,
) -> Option<Gen3Value> {
    frame_value(frame, name, size)
}

/// What the rasterizer's globals hold for an object here, by uniform name:
/// Halo 3's, which Reach's fall back to.
pub(in crate::app::model_preview) fn frame_value(
    frame: &ModelGpuFrame,
    name: &str,
    size: [i32; 2],
) -> Option<Gen3Value> {
    let [width, height] = size.map(|side| side as f32);
    let floats = |value: [f32; 4]| Some(Gen3Value::Floats(value));
    if let Some(index) = name
        .strip_prefix("p_lighting_constant_")
        .or_else(|| name.strip_prefix("v_lighting_constant_"))
        .and_then(|index| index.parse::<usize>().ok())
    {
        return floats(lighting_constants(&frame_probe(frame))[index]);
    }
    let eye = eye_position(&frame.camera);
    match name {
        "v_always_true" | "p_shader_pc_specular_enabled" | "actually_calc_albedo" => {
            Some(Gen3Value::Integer(1))
        }
        "Position_Compression_Scale" => floats([1.0; 4]),
        "screenspace_xform" => floats([1.0, 1.0, 0.0, 0.0]),
        "UV_Compression_Scale_Offset" => floats([1.0, 1.0, 0.0, 0.0]),
        "k_ps_dominant_light_direction" if frame.light_follows_camera => {
            floats(dominant_light(&frame_probe(frame)).0)
        }
        "k_ps_dominant_light_intensity" if frame.light_follows_camera => {
            floats(dominant_light(&frame_probe(frame)).1)
        }
        "k_ps_dominant_light_direction" => floats(DOMINANT_DIRECTION),
        "k_ps_dominant_light_intensity" => floats(DOMINANT_INTENSITY),
        "Camera_Position_PS" => floats([eye[0], eye[1], eye[2], 0.0]),
        // `setup_render_target_globals_with_exposure`, at the default
        // `camera_fx_settings`' stops.
        "g_exposure" => floats([EXPOSURE, 128.0, 1.0, 1.0]),
        "g_alt_exposure" => floats([1.0, EXPOSURE, 0.0, 0.0]),
        // `compute_scattering` skips the atmosphere below zero.
        "v_atmosphere_constant_1" => floats([0.0, 0.0, 0.0, -1.0]),
        "dynamic_environment_blend" => floats([1.0, 0.0, 0.0, 0.0]),
        // `render_albedo_decals`.
        "fade" => floats([1.0; 4]),
        "sprite" => floats([0.0, 0.0, 1.0, 1.0]),
        "pixel_kill_enabled" => Some(Gen3Value::Integer(1)),
        "simple_light_count" | "dynamic_light_shadowing" => Some(Gen3Value::Integer(0)),
        // A window of the whole target.
        "ps_global_viewport_res" | "ps_global_render_resolution" | "texture_size" => {
            floats([width, height, 0.0, 0.0])
        }
        "ps_global_render_pixel_size" => floats([1.0 / width, 1.0 / height, 0.0, 0.0]),
        "ps_global_viewport_res_multipliers" => floats([1.0, 1.0, 0.0, 0.0]),
        "ps_global_viewport_bounds_uv" => floats([0.0, 0.0, 1.0, 1.0]),
        // `render_object_context`: fully on, at unit screen scale. Its screen
        // runs down and this one up, so the refraction's Y turns over.
        "k_ps_active_camo_factor" => floats([1.0, 1.0, -1.0, 0.0]),
        _ => None,
    }
}

/// `View`: forward, left, up and the eye in its columns, rowed for HLSL.
pub(super) fn view_matrix(frame: &ModelGpuFrame) -> [f32; 16] {
    let (yaw, pitch) = (frame.camera.yaw, frame.camera.pitch);
    let forward = unrotate_view_vector(yaw, pitch, [0.0, 1.0, 0.0]);
    let left = unrotate_view_vector(yaw, pitch, [-1.0, 0.0, 0.0]);
    let up = unrotate_view_vector(yaw, pitch, [0.0, 0.0, 1.0]);
    let eye = eye_position(&frame.camera);
    let mut matrix = [0.0; 16];
    for row in 0..3 {
        matrix[row * 4] = forward[row];
        matrix[row * 4 + 1] = left[row];
        matrix[row * 4 + 2] = up[row];
        matrix[row * 4 + 3] = eye[row];
    }
    matrix[15] = 1.0;
    matrix
}

/// Reach's `Camera_To_World`: right, up, backward and the eye in its
/// columns, rowed for HLSL, which reads `Camera_Position` and the camera's
/// axes out of it (`hlsl_constant_global_list`).
pub(in crate::app::model_preview) fn camera_to_world(frame: &ModelGpuFrame) -> [f32; 16] {
    let (yaw, pitch) = (frame.camera.yaw, frame.camera.pitch);
    let right = unrotate_view_vector(yaw, pitch, [1.0, 0.0, 0.0]);
    let up = unrotate_view_vector(yaw, pitch, [0.0, 0.0, 1.0]);
    let backward = unrotate_view_vector(yaw, pitch, [0.0, -1.0, 0.0]);
    let eye = eye_position(&frame.camera);
    let mut matrix = [0.0; 16];
    for row in 0..3 {
        matrix[row * 4] = right[row];
        matrix[row * 4 + 1] = up[row];
        matrix[row * 4 + 2] = backward[row];
        matrix[row * 4 + 3] = eye[row];
    }
    matrix[15] = 1.0;
    matrix
}

/// A translated uniform's value, by its type.
pub(in crate::app::model_preview) unsafe fn set_value(
    gl: &glow::Context,
    location: Option<&glow::NativeUniformLocation>,
    ty: &Type,
    value: &Gen3Value,
) {
    unsafe {
        match (ty, value) {
            (Type::Scalar(Scalar::Float), Gen3Value::Floats(v)) => gl.uniform_1_f32(location, v[0]),
            (Type::Vector(Scalar::Float, 2), Gen3Value::Floats(v)) => {
                gl.uniform_2_f32(location, v[0], v[1])
            }
            (Type::Vector(Scalar::Float, 3), Gen3Value::Floats(v)) => {
                gl.uniform_3_f32(location, v[0], v[1], v[2])
            }
            (Type::Vector(Scalar::Float, 4), Gen3Value::Floats(v)) => {
                gl.uniform_4_f32_slice(location, v)
            }
            (Type::Scalar(Scalar::Float), Gen3Value::Integer(v)) => {
                gl.uniform_1_f32(location, *v as f32)
            }
            (Type::Scalar(Scalar::Uint), Gen3Value::Integer(v)) => {
                gl.uniform_1_u32(location, *v as u32)
            }
            (Type::Scalar(_), Gen3Value::Integer(v)) => gl.uniform_1_i32(location, *v),
            (Type::Scalar(_), Gen3Value::Floats(v)) => gl.uniform_1_i32(location, v[0] as i32),
            (Type::Array(element, Some(length)), Gen3Value::FloatArray(rows))
                if **element == Type::Vector(Scalar::Float, 4) =>
            {
                let mut data = rows.clone();
                data.resize(*length, [0.0; 4]);
                gl.uniform_4_f32_slice(location, data.as_flattened());
            }
            _ => {}
        }
    }
}

impl Gen3State {
    pub(super) unsafe fn upload(
        &mut self,
        core: &mut RenderCore,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) {
        let uploaded = &mut core.slots[slot].gen3_textures;
        if uploaded.id == Some(frame.textures_id) {
            return;
        }
        unsafe {
            for texture in uploaded
                .materials
                .drain(..)
                .flat_map(|map| map.into_values())
            {
                gl.delete_texture(texture);
            }
            for texture in uploaded.gels.drain(..).flatten() {
                gl.delete_texture(texture);
            }
            for (_, texture) in uploaded.shield.drain() {
                gl.delete_texture(texture);
            }
            let shields = frame.textures.iter().flat_map(|textures| textures.iter());
            let global = shields
                .filter_map(|texture| texture.h3.as_ref()?.shield.as_ref())
                .next();
            let settings = frame
                .preview
                .h3
                .shield
                .as_ref()
                .or(global.map(|shield| &shield.settings));
            let reach = frame
                .preview
                .reach
                .shield
                .as_ref()
                .map(|shield| shield.settings.textures.iter());
            let h4 = frame
                .preview
                .h4
                .shield
                .as_ref()
                .map(|shield| shield.textures.iter());
            for texture in reach.into_iter().flatten().chain(h4.into_iter().flatten()) {
                if let Some(found) = upload_image_linear(
                    gl,
                    &texture.image,
                    texture.texels,
                    &texture.mips,
                    texture.sampler,
                ) {
                    uploaded.shield.insert(texture.uniform.clone(), found);
                }
            }
            if let Some(settings) = settings {
                for texture in &settings.textures {
                    if let Some(found) = upload_image_linear(
                        gl,
                        &texture.image,
                        texture.texels,
                        &texture.mips,
                        texture.sampler,
                    ) {
                        uploaded.shield.insert(texture.uniform.clone(), found);
                    }
                }
            }
            uploaded.gels = frame
                .preview
                .h3
                .lights
                .iter()
                .map(|light| {
                    light
                        .gel
                        .as_ref()
                        .and_then(|gel| upload_image_linear(gl, gel, ImageTexels::Srgb, &[], None))
                })
                .collect();
            for textures in frame.textures.iter().flat_map(|textures| textures.iter()) {
                let mut map = HashMap::new();
                if let Some(material) = &textures.h3 {
                    for texture in &material.textures {
                        if let Some(uploaded) = upload_image_linear(
                            gl,
                            &texture.image,
                            texture.texels,
                            &texture.mips,
                            texture.sampler,
                        ) {
                            map.insert(texture.uniform.clone(), uploaded);
                        }
                        for (at, image) in texture.frames.iter().enumerate() {
                            if let Some(uploaded) =
                                upload_image_linear(gl, image, texture.texels, &[], texture.sampler)
                            {
                                map.insert(frame_key(&texture.uniform, at), uploaded);
                            }
                        }
                    }
                }
                uploaded.materials.push(map);
            }
        }
        uploaded.id = Some(frame.textures_id);
    }

    /// The uploaded name for `uniform` on this draw, a chosen image if it
    /// animates.
    pub(super) fn uploaded_key(
        &self,
        frame: &ModelGpuFrame,
        batch: &RenderModelPreviewBatch,
        material: &Gen3Material,
        uniform: &str,
    ) -> String {
        let count = material
            .textures
            .iter()
            .find(|texture| texture.uniform == uniform);
        let count = count.map_or(0, |texture| texture.frames.len());
        let object = batch.material_index as usize;
        let owner = frame
            .preview
            .materials
            .get(object)
            .map_or(0, |material| material.object);
        // Halo 4: the sampler's frame-index function, on the object's values.
        if let Some(submit) = material.h4().filter(|_| count > 0) {
            let drawn = super::super::super::h4::materials::submit::DrawnObject {
                values: frame.function_values_of(object),
                time: frame.time,
                owner,
            };
            if let Some(index) = submit.frame(uniform, &drawn) {
                return frame_key(uniform, index);
            }
        }
        let animation = material.animations.iter().find(|animation| {
            animation.writes == Gen3Animated::Frame && animation.uniform == uniform
        });
        match animation.filter(|_| count > 0) {
            Some(animation) => frame_key(
                uniform,
                animation.frame(count, frame.time, owner, frame.function_values_of(object)),
            ),
            None => uniform.to_owned(),
        }
    }

    pub(super) unsafe fn nodes(
        &self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        location: Option<&glow::NativeUniformLocation>,
        ty: &Type,
    ) {
        let Type::Array(_, Some(nodes)) = ty else {
            return;
        };
        let Type::Array(element, _) = ty else {
            return;
        };
        let rows = match &**element {
            Type::Array(_, Some(rows)) => *rows,
            _ => 3,
        };
        let mut data = vec![[0.0f32; 4]; nodes * rows];
        for node in 0..*nodes {
            for row in 0..3.min(rows) {
                data[node * rows + row][row] = 1.0;
            }
        }
        if let Some(bones) = frame.bones.as_ref() {
            // The last node stays put, for geometry on no node at all.
            let spare = (nodes - 1) * rows;
            let length = bones.len().min(spare);
            data[..length].copy_from_slice(&bones[..length]);
        }
        unsafe {
            gl.uniform_4_f32_slice(location, data.as_flattened());
        }
    }
}
