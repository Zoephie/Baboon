//! Halo 3's objects, as `render_objects` draws them: an albedo pass that lays
//! albedo and normal into targets, a lit pass reading them back into an HDR
//! accumulation target, and the final composite's tone curve onto the scene.

use super::h3_bloom::H3Bloom;
use super::h3_prt::{H3PrtKind, second_order_rotation};
use std::borrow::Cow;

use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};

use super::h3_shaders::{H3Animated, H3Blend, H3Material, H3Pass, H3Sampler, H3Shield, H3Texels, H3Value};
use super::*;
use crate::core::hlsl::parse::{Scalar, TextureKind, Type};

/// `c_geometry_sampler::m_default_lightprobe_sample`, the first nine terms of
/// each channel: the light of an object with no level around it.
const DEFAULT_PROBE: [[f32; 9]; 3] = [
    [1.89376, -1.44314, 0.19557, 0.51599, -0.30808, -0.04173, -0.46665, 0.29195, 0.10501],
    [2.13512, -1.79217, 0.15439, 0.51479, -0.27409, 0.07048, -0.62705, 0.25751, -0.01816],
    [2.25851, -2.07371, 0.05633, 0.44103, -0.19508, 0.23114, -0.7642, 0.18839, -0.22193],
];

/// The object function values behind the shield's two quantities.
pub(super) const H3_SHIELD_DAMAGE: &str = "current_shield_damage";
pub(super) const H3_OVERSHIELD: &str = "object_overshield_amount";

/// `g_decal_slope_depth_bias` and `g_decal_depth_bias`.
const DECAL_SLOPE_BIAS: f32 = -0.5;
const DECAL_DEPTH_BIAS: f32 = -0.000005;

/// `final_composite`'s `tone_curve_constants` at a white point of one: the
/// numbers in its comment are the curve before this scale.
const TONE_CURVE: [f32; 4] = {
    let scale = 1.493_801_6;
    [1.493_801_6 / scale, 1.004_149_4 * scale, 0.0, -0.15 * scale * scale * scale]
};

/// `get_render_exposure` at no stops and a white point of one.
const EXPOSURE: f32 = 0.669_432_94;

/// `setup_default_lighting`'s light straight down, in white.
const DOMINANT_DIRECTION: [f32; 4] = [0.0, 0.0, -1.0, 1.0];
const DOMINANT_INTENSITY: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// Which attributes of the preview's vertex each semantic reads.
const ATTRIBUTES: [(&str, &str); 7] = [
    ("a_POSITION", "a_position"),
    ("a_TEXCOORD0", "a_texcoord"),
    ("a_NORMAL", "a_normal"),
    ("a_TANGENT", "a_tangent"),
    ("a_BINORMAL", "a_binormal"),
    ("a_BLENDINDICES", "a_node_indices"),
    ("a_BLENDWEIGHT", "a_node_weights"),
];

/// Where `static_prt_*`'s coefficients arrive, past the preview's own.
pub(super) const PRT_ATTRIBUTES: [u32; 3] = [11, 12, 13];
const PRT_NAMES: [&str; 3] = ["a_BLENDWEIGHT1", "a_BLENDWEIGHT2", "a_BLENDWEIGHT3"];

const COMPOSITE_VERTEX: &str = "#version 410 core
void main() {
    vec2 corner = vec2(gl_VertexID & 1, gl_VertexID >> 1) * 4.0 - 1.0;
    gl_Position = vec4(corner, 0.0, 1.0);
}
";

const COMPOSITE_FRAGMENT: &str = "#version 410 core
uniform sampler2D accumulation;
uniform sampler2D bloom;
uniform vec4 tone_curve_constants;
out vec4 color;
void main() {
    vec4 accumulated = texelFetch(accumulation, ivec2(gl_FragCoord.xy), 0);
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    vec3 blend = accumulated.rgb + texture(bloom, place).rgb;
    vec3 clamped = min(blend, tone_curve_constants.xxx);
    vec3 toned = ((clamped * tone_curve_constants.w + tone_curve_constants.z) * clamped
        + tone_curve_constants.y) * clamped;
    // The game's surfaces are sRGB, so this happens as it writes.
    toned = clamp(toned, 0.0, 1.0);
    color.rgb = mix(12.92 * toned, 1.055 * pow(toned, vec3(1.0 / 2.4)) - 0.055,
        step(0.0031308, toned));
    color.a = accumulated.a;
}
";

/// The targets a frame draws through.
pub(super) struct H3Targets {
    size: [i32; 2],
    albedo_framebuffer: glow::NativeFramebuffer,
    accumulation_framebuffer: glow::NativeFramebuffer,
    albedo: glow::NativeTexture,
    normal: glow::NativeTexture,
    accumulation: glow::NativeTexture,
    /// A copy of the scene behind the object, for active camo.
    behind: glow::NativeTexture,
}

pub(super) unsafe fn screen_program(gl: &glow::Context, fragment: &str) -> Option<glow::NativeProgram> {
    unsafe {
        let program = gl.create_program().ok()?;
        for (kind, source) in [(glow::VERTEX_SHADER, COMPOSITE_VERTEX), (glow::FRAGMENT_SHADER, fragment)] {
            let shader = gl.create_shader(kind).ok()?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        gl.link_program(program);
        Some(program)
    }
}

unsafe fn target_texture(gl: &glow::Context, size: [i32; 2], format: u32) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            format as i32,
            size[0],
            size[1],
            0,
            glow::RGBA,
            glow::FLOAT,
            glow::PixelUnpackData::Slice(None),
        );
        for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::NEAREST as i32);
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(texture)
    }
}

impl H3Targets {
    unsafe fn new(gl: &glow::Context, size: [i32; 2], depth: glow::NativeRenderbuffer) -> Option<Self> {
        unsafe {
            let albedo = target_texture(gl, size, glow::RGBA16F)?;
            let normal = target_texture(gl, size, glow::RGBA16F)?;
            let accumulation = target_texture(gl, size, glow::RGBA16F)?;
            let behind = target_texture(gl, size, glow::SRGB8_ALPHA8)?;
            let albedo_framebuffer = gl.create_framebuffer().ok()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(albedo_framebuffer));
            for (attachment, texture) in
                [(glow::COLOR_ATTACHMENT0, albedo), (glow::COLOR_ATTACHMENT1, normal)]
            {
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, attachment, glow::TEXTURE_2D, Some(texture), 0);
            }
            gl.draw_buffers(&[glow::COLOR_ATTACHMENT0, glow::COLOR_ATTACHMENT1]);
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            let accumulation_framebuffer = gl.create_framebuffer().ok()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(accumulation_framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(accumulation),
                0,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            Some(Self { size, albedo_framebuffer, accumulation_framebuffer, albedo, normal, accumulation, behind })
        }
    }

    unsafe fn delete(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.albedo_framebuffer);
            gl.delete_framebuffer(self.accumulation_framebuffer);
            for texture in [self.albedo, self.normal, self.accumulation, self.behind] {
                gl.delete_texture(texture);
            }
        }
    }
}

/// A linked pass and where its uniforms landed.
pub(super) struct H3Program {
    /// Held so its address, the program's key, stays its own.
    _pass: Arc<H3Pass>,
    program: glow::NativeProgram,
    locations: HashMap<String, Option<glow::NativeUniformLocation>>,
}

impl H3Program {
    unsafe fn location(&mut self, gl: &glow::Context, name: &str) -> Option<glow::NativeUniformLocation> {
        if let Some(found) = self.locations.get(name) {
            return found.clone();
        }
        let found = unsafe { gl.get_uniform_location(self.program, name) };
        self.locations.insert(name.to_owned(), found.clone());
        found
    }
}

/// Uploaded textures of a geometry, by material then name.
#[derive(Default)]
pub(super) struct H3SlotTextures {
    id: Option<u64>,
    materials: Vec<HashMap<String, glow::NativeTexture>>,
    /// The shield's noise, by uniform.
    shield: HashMap<String, glow::NativeTexture>,
    /// Each of `h3_lights`' gels, of `id`.
    gels: Vec<Option<glow::NativeTexture>>,
}

/// `clip_position` as the matrix `mul(position, View_Projection)` wants: GLSL's
/// columns are HLSL's rows here, so this is a plain column matrix.
fn view_projection(camera: &ModelGpuCamera) -> [f32; 16] {
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

/// `calculate_and_set_ravi_constants_internal`.
fn lighting_constants(probe: &[[f32; 9]; 3]) -> [[f32; 4]; 10] {
    let [r, g, b] = probe;
    let root = 3f32.sqrt();
    [
        [r[0], g[0], b[0], 0.0],
        [r[3], r[1], -r[2], 0.0],
        [g[3], g[1], -g[2], 0.0],
        [b[3], b[1], -b[2], 0.0],
        [-r[4], r[5], r[7], 0.0],
        [-g[4], g[5], g[7], 0.0],
        [-b[4], b[5], b[7], 0.0],
        [-r[8], r[8], -(r[6] * root), r[6] * root],
        [-g[8], g[8], -(g[6] * root), g[6] * root],
        [-b[8], b[8], -(b[6] * root), b[6] * root],
    ]
}

/// Whether the shield is showing, whose plasma scrolls with game time.
pub(super) fn h3_shield_showing(values: Option<&h2_functions::H2FunctionValues>) -> bool {
    let reading = |name| values.and_then(|values| values.get(name)).copied().unwrap_or(0.0);
    reading(H3_SHIELD_DAMAGE) > 0.0 || reading(H3_OVERSHIELD) > 0.0
}

/// Where one image of an animated bitmap is uploaded under.
fn frame_key(uniform: &str, frame: usize) -> String {
    format!("{uniform}#{frame}")
}

/// `calculate_dominant_light_from_lightprobe`: the probe's first order, by
/// luminance, and how much of the probe lies along it.
fn dominant_light(probe: &[[f32; 9]; 3]) -> ([f32; 4], [f32; 4], f32) {
    let [r, g, b] = probe;
    let weighted = |at: usize| 0.212_656 * r[at] + 0.715_158 * g[at] + 0.072_185_6 * b[at];
    let toward = [-weighted(3), -weighted(1), weighted(2)];
    let length = toward.iter().map(|value| value * value).sum::<f32>().sqrt().max(1e-6);
    let [x, y, z] = toward.map(|value| value / length);
    // `sh_eval_direction`, in the probe's order.
    let (first, second) = (0.488_603, 0.546_274);
    let root = 3f32.sqrt();
    let eval = [
        0.282_095,
        -first * y,
        first * z,
        -first * x,
        second * 2.0 * x * y,
        -second * 2.0 * y * z,
        second * (3.0 * z * z - 1.0) / root,
        -second * 2.0 * x * z,
        second * (x * x - y * y),
    ];
    let denominator = eval.iter().map(|value| value * value).sum::<f32>().max(0.0001);
    let intensity = probe.map(|channel| {
        let sum: f32 = eval.iter().zip(&channel).map(|(a, b)| a * b).sum();
        sum.max(0.0) / denominator
    });
    // How much of the green the one light leaves unexplained.
    let (error, most) = eval.iter().zip(&probe[1]).fold((0.0, 0.0), |(error, most), (a, b)| {
        let difference = intensity[1] * a - b;
        (error + difference * difference, most + b * b * 2.0)
    });
    let contrast = if most > 0.0001 { (1.0 - error / most).powi(4) * 10.0 } else { 10.0 };
    ([x, y, z, 1.0], [intensity[0], intensity[1], intensity[2], 1.0], contrast)
}

/// Which way an object's shadow falls from and how dark it is: the dominant
/// light, whose shadow fades out as the probe grows even.
pub(super) fn h3_shadow_light(frame: &ModelGpuFrame) -> ([f32; 3], f32) {
    let ([x, y, z, _], _, contrast) = dominant_light(&frame_probe(frame));
    ([x, y, z], h3_shadow_alpha(contrast))
}

fn h3_shadow_alpha(contrast: f32) -> f32 {
    (contrast.clamp(2.0, 8.0) - 2.0) / 6.0
}

/// The default probe turned about the vertical by `angle`, the sky staying up.
fn turned_probe(angle: f32) -> [[f32; 9]; 3] {
    let (sin, cos) = angle.sin_cos();
    let second = second_order_rotation([[cos, -sin, 0.0], [sin, cos, 0.0], [0.0, 0.0, 1.0]]);
    DEFAULT_PROBE.map(|mut channel| {
        let (x, y) = (channel[3], channel[1]);
        (channel[3], channel[1]) = (cos * x - sin * y, sin * x + cos * y);
        let stored: [f32; 5] = std::array::from_fn(|at| channel[4 + at]);
        for (value, row) in channel[4..].iter_mut().zip(&second) {
            *value = row.iter().zip(&stored).map(|(a, b)| a * b).sum();
        }
        channel
    })
}

/// Which way, about the vertical, the default probe is brightest.
fn probe_heading() -> f32 {
    let mean = |at: usize| DEFAULT_PROBE.iter().map(|channel| channel[at]).sum::<f32>() / 3.0;
    (-mean(1)).atan2(-mean(3))
}

/// The probe an object is lit by. With the light following the camera it
/// turns to come from over the viewer's shoulder; this is the preview's own.
fn frame_probe(frame: &ModelGpuFrame) -> [[f32; 9]; 3] {
    if !frame.light_follows_camera {
        return DEFAULT_PROBE;
    }
    let [x, y, _] = h2_camera_light(frame.camera.yaw, frame.camera.pitch);
    turned_probe(y.atan2(x) - probe_heading())
}

/// What the rasterizer's globals hold for an object here, by uniform name.
fn frame_value(frame: &ModelGpuFrame, name: &str, size: [i32; 2]) -> Option<H3Value> {
    let [width, height] = size.map(|side| side as f32);
    let floats = |value: [f32; 4]| Some(H3Value::Floats(value));
    if let Some(index) = name
        .strip_prefix("p_lighting_constant_")
        .or_else(|| name.strip_prefix("v_lighting_constant_"))
        .and_then(|index| index.parse::<usize>().ok())
    {
        return floats(lighting_constants(&frame_probe(frame))[index]);
    }
    let eye = h2_eye_position(&frame.camera);
    match name {
        "v_always_true" | "p_shader_pc_specular_enabled" | "actually_calc_albedo" => Some(H3Value::Integer(1)),
        "Position_Compression_Scale" => floats([1.0; 4]),
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
        "pixel_kill_enabled" => Some(H3Value::Integer(1)),
        "simple_light_count" | "dynamic_light_shadowing" => Some(H3Value::Integer(0)),
        // A window of the whole target.
        "ps_global_viewport_res" | "ps_global_render_resolution" | "texture_size" => {
            floats([width, height, 0.0, 0.0])
        }
        "ps_global_render_pixel_size" => floats([1.0 / width, 1.0 / height, 0.0, 0.0]),
        "ps_global_viewport_res_multipliers" => floats([1.0, 1.0, 0.0, 0.0]),
        "ps_global_viewport_bounds_uv" => floats([0.0, 0.0, 1.0, 1.0]),
        // `render_object_context`: fully on, at unit screen scale.
        "k_ps_active_camo_factor" => floats([1.0, 1.0, 1.0, 0.0]),
        _ => None,
    }
}

/// `View`: forward, left, up and the eye in its columns, rowed for HLSL.
fn view_matrix(frame: &ModelGpuFrame) -> [f32; 16] {
    let (yaw, pitch) = (frame.camera.yaw, frame.camera.pitch);
    let forward = unrotate_view_vector(yaw, pitch, [0.0, 1.0, 0.0]);
    let left = unrotate_view_vector(yaw, pitch, [-1.0, 0.0, 0.0]);
    let up = unrotate_view_vector(yaw, pitch, [0.0, 0.0, 1.0]);
    let eye = h2_eye_position(&frame.camera);
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

unsafe fn set_value(gl: &glow::Context, location: Option<&glow::NativeUniformLocation>, ty: &Type, value: &H3Value) {
    unsafe {
        match (ty, value) {
            (Type::Scalar(Scalar::Float), H3Value::Floats(v)) => gl.uniform_1_f32(location, v[0]),
            (Type::Vector(Scalar::Float, 2), H3Value::Floats(v)) => gl.uniform_2_f32(location, v[0], v[1]),
            (Type::Vector(Scalar::Float, 3), H3Value::Floats(v)) => gl.uniform_3_f32(location, v[0], v[1], v[2]),
            (Type::Vector(Scalar::Float, 4), H3Value::Floats(v)) => gl.uniform_4_f32_slice(location, v),
            (Type::Scalar(Scalar::Float), H3Value::Integer(v)) => gl.uniform_1_f32(location, *v as f32),
            (Type::Scalar(Scalar::Uint), H3Value::Integer(v)) => gl.uniform_1_u32(location, *v as u32),
            (Type::Scalar(_), H3Value::Integer(v)) => gl.uniform_1_i32(location, *v),
            (Type::Scalar(_), H3Value::Floats(v)) => gl.uniform_1_i32(location, v[0] as i32),
            _ => {}
        }
    }
}

impl ModelGlRenderer {
    /// A material's translated passes, if this context can run them.
    pub(super) fn h3_material<'a>(&self, frame: &'a ModelGpuFrame, material: usize) -> Option<&'a Arc<H3Material>> {
        if !self.h3_supported {
            return None;
        }
        frame.textures.as_ref()?.get(material)?.h3.as_ref()
    }

    /// The normals the albedo pass laid down, for the shadow pass to read.
    pub(super) fn h3_normal_target(&self) -> Option<glow::NativeTexture> {
        self.h3_targets.as_ref().map(|targets| targets.normal)
    }

    pub(super) fn h3_draws(&self, frame: &ModelGpuFrame) -> bool {
        frame.game_shaders
            && self.h3_supported
            && frame.textures.iter().any(|textures| textures.iter().any(|texture| texture.h3.is_some()))
    }

    unsafe fn h3_program(&mut self, gl: &glow::Context, pass: &Arc<H3Pass>) -> Option<H3Program> {
        let key = Arc::as_ptr(pass) as usize;
        if let Some(program) = self.h3_programs.remove(&key) {
            return program;
        }
        unsafe {
            let stages = [(glow::VERTEX_SHADER, &pass.vertex), (glow::FRAGMENT_SHADER, &pass.pixel)];
            let program = gl.create_program().ok()?;
            let mut shaders = Vec::new();
            for (kind, source) in stages {
                let shader = gl.create_shader(kind).ok()?;
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    eprintln!("h3 shader failed: {}", gl.get_shader_info_log(shader));
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }
            for (name, preview) in ATTRIBUTES {
                if let Some(location) = gl.get_attrib_location(self.program, preview) {
                    gl.bind_attrib_location(program, location, name);
                }
            }
            for (name, location) in PRT_NAMES.into_iter().zip(PRT_ATTRIBUTES) {
                gl.bind_attrib_location(program, location, name);
            }
            gl.link_program(program);
            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }
            if !gl.get_program_link_status(program) {
                eprintln!("h3 program failed: {}", gl.get_program_info_log(program));
                gl.delete_program(program);
                self.h3_programs.insert(key, None);
                return None;
            }
            Some(H3Program { _pass: Arc::clone(pass), program, locations: HashMap::new() })
        }
    }

    unsafe fn h3_upload(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        let uploaded = &mut self.slots[slot].h3_textures;
        if uploaded.id == Some(frame.textures_id) {
            return;
        }
        unsafe {
            for texture in uploaded.materials.drain(..).flat_map(|map| map.into_values()) {
                gl.delete_texture(texture);
            }
            for texture in uploaded.gels.drain(..).flatten() {
                gl.delete_texture(texture);
            }
            for (_, texture) in uploaded.shield.drain() {
                gl.delete_texture(texture);
            }
            let shields = frame.textures.iter().flat_map(|textures| textures.iter());
            let global = shields.filter_map(|texture| texture.h3.as_ref()?.shield.as_ref()).next();
            let settings = frame.preview.h3_shield.as_ref().or(global.map(|shield| &shield.settings));
            if let Some(settings) = settings {
                for texture in &settings.textures {
                    if let Some(found) = upload_h2_image_linear(gl, &texture.image, texture.texels, texture.sampler) {
                        uploaded.shield.insert(texture.uniform.clone(), found);
                    }
                }
            }
            uploaded.gels = frame
                .preview
                .h3_lights
                .iter()
                .map(|light| light.gel.as_ref().and_then(|gel| upload_h2_image_linear(gl, gel, H3Texels::Srgb, None)))
                .collect();
            for textures in frame.textures.iter().flat_map(|textures| textures.iter()) {
                let mut map = HashMap::new();
                if let Some(material) = &textures.h3 {
                    for texture in &material.textures {
                        if let Some(uploaded) = upload_h2_image_linear(gl, &texture.image, texture.texels, texture.sampler) {
                            map.insert(texture.uniform.clone(), uploaded);
                        }
                        for (at, image) in texture.frames.iter().enumerate() {
                            if let Some(uploaded) = upload_h2_image_linear(gl, image, texture.texels, texture.sampler) {
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
    fn h3_frame_key(
        &self,
        frame: &ModelGpuFrame,
        batch: &RenderModelPreviewBatch,
        material: &H3Material,
        uniform: &str,
    ) -> String {
        let count = material.textures.iter().find(|texture| texture.uniform == uniform);
        let count = count.map_or(0, |texture| texture.frames.len());
        let animation = material.animations.iter().find(|animation| {
            animation.writes == H3Animated::Frame && animation.uniform == uniform
        });
        match animation.filter(|_| count > 0) {
            Some(animation) => {
                let values = frame.h2_values_of(batch.material_index as usize);
                frame_key(uniform, animation.frame(count, frame.time, values))
            }
            None => uniform.to_owned(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn h3_draw_pass(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        batch: &RenderModelPreviewBatch,
        material: &H3Material,
        pass: &Arc<H3Pass>,
        shield: Option<&H3Shield>,
    ) {
        let Some((offset, count)) =
            model_draw_range(batch.index_start, batch.index_count, frame.preview.indices.len())
        else {
            return;
        };
        unsafe {
            let Some(mut program) = self.h3_program(gl, pass) else {
                return;
            };
            gl.use_program(Some(program.program));
            let size = self.h3_targets.as_ref().map_or([1, 1], |targets| targets.size);
            for (name, ty) in &pass.uniforms {
                let location = program.location(gl, name);
                if location.is_none() {
                    continue;
                }
                match name.as_str() {
                    "View_Projection" => {
                        gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &view_projection(&frame.camera));
                        continue;
                    }
                    "View" => {
                        gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &view_matrix(frame));
                        continue;
                    }
                    "Nodes" => {
                        self.h3_nodes(gl, frame, location.as_ref(), ty);
                        continue;
                    }
                    "simple_lights" => {
                        if let Some((lit, _)) = &self.h3_light {
                            gl.uniform_4_f32_slice(location.as_ref(), lit.rows.as_flattened());
                        }
                        continue;
                    }
                    "Shadow_Projection" => {
                        if let Some((lit, _)) = &self.h3_light {
                            gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &lit.projection);
                        }
                        continue;
                    }
                    _ => {}
                }
                // The dynamic pass borrows this constant for its gel's xform.
                if name == "p_lighting_constant_4" && self.h3_light.is_some() {
                    gl.uniform_4_f32(location.as_ref(), 1.0, 1.0, 0.0, 0.0);
                    continue;
                }
                let change_color = material.change_colors.iter().find(|(uniform, _)| uniform == name);
                let change_color = change_color.map(|(_, channel)| {
                    let [r, g, b] = frame.change_colors_of(batch.material_index as usize)[*channel];
                    H3Value::Floats([r, g, b, 1.0])
                });
                let mut value = change_color
                    .or_else(|| frame_value(frame, name, size))
                    .or_else(|| material.values.get(name).cloned());
                let object = batch.material_index as usize;
                let values = frame.h2_values_of(object);
                for animation in material.animations.iter().filter(|animation| animation.uniform == *name) {
                    let mut slot = match value {
                        Some(H3Value::Floats(slot)) => slot,
                        _ => [0.0; 4],
                    };
                    animation.apply(&mut slot, frame.time, values);
                    value = Some(H3Value::Floats(slot));
                }
                if let Some(value) = value {
                    set_value(gl, location.as_ref(), ty, &value);
                }
                let settings = frame.preview.h3_shield.as_ref().or(shield.map(|shield| &shield.settings));
                if let Some(value) = shield.and(settings).and_then(|settings| settings.values.get(name)) {
                    set_value(gl, location.as_ref(), ty, value);
                }
                if name == "shield_dynamic_quantities" {
                    let reading = |name: &str| {
                        frame.h2_values.as_ref().and_then(|values| values.get(name)).copied().unwrap_or(0.0)
                    };
                    // `render_object_context`: the overshield in fifths of a byte.
                    let quantities = [frame.time, reading(H3_SHIELD_DAMAGE), reading(H3_OVERSHIELD), 0.0];
                    gl.uniform_4_f32_slice(location.as_ref(), &quantities);
                }
                if name == "bound_sphere" {
                    let (low, high) = (frame.preview.bounds_min, frame.preview.bounds_max);
                    let center: [f32; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
                    let reach: [f32; 3] = std::array::from_fn(|axis| high[axis] - center[axis]);
                    let radius = (reach[0] * reach[0] + reach[1] * reach[1] + reach[2] * reach[2]).sqrt();
                    gl.uniform_4_f32(location.as_ref(), center[0], center[1], center[2], radius);
                }
            }
            let targets = self.h3_targets.as_ref();
            let uploaded = self.slots[slot].h3_textures.materials.get(batch.material_index as usize);
            for (unit, (name, kind)) in pass.textures.iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                let texture = match name.as_str() {
                    "albedo_texture" => targets.map(|targets| targets.albedo),
                    "normal_texture" => targets.map(|targets| targets.normal),
                    "GlobalTexture_scene_ldr_texture" => targets.map(|targets| targets.behind),
                    "UserParameterTexture_dynamic_light_gel_texture" => {
                        self.h3_light.as_ref().and_then(|(_, gel)| *gel)
                    }
                    other => uploaded.and_then(|map| {
                        map.get(&self.h3_frame_key(frame, batch, material, other)).or_else(|| map.get(other))
                    }).copied().or_else(|| {
                        let noise = other.strip_prefix("LocalTexture_").unwrap_or(other);
                        self.slots[slot].h3_textures.shield.get(noise).copied()
                    }),
                };
                match kind {
                    TextureKind::Flat => gl.bind_texture(glow::TEXTURE_2D, texture.or(self.placeholder)),
                    TextureKind::Cube => gl.bind_texture(glow::TEXTURE_CUBE_MAP, texture.or(self.h2_black_cube)),
                    TextureKind::Volume => gl.bind_texture(glow::TEXTURE_3D, texture.or(self.h2_black_volume)),
                    TextureKind::FlatArray => {}
                }
                let location = program.location(gl, name);
                gl.uniform_1_i32(location.as_ref(), unit as i32);
            }
            gl.draw_elements(glow::TRIANGLES, count, glow::UNSIGNED_INT, offset);
            self.h3_programs.insert(Arc::as_ptr(pass) as usize, Some(program));
        }
    }

    unsafe fn h3_nodes(
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

    /// The object's passes, between the preview's own draw and the scene's end.
    pub(super) unsafe fn draw_h3(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize, order: &[usize]) {
        let Some(scene) = self.h2_scene.as_ref() else {
            return;
        };
        let (size, depth, framebuffer) = (scene.size, scene.depth, scene.framebuffer);
        unsafe {
            if self.h3_targets.as_ref().is_none_or(|targets| targets.size != size) {
                if let Some(old) = self.h3_targets.take() {
                    old.delete(gl);
                }
                self.h3_targets = H3Targets::new(gl, size, depth);
            }
            let Some(targets) = self.h3_targets.as_ref() else {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                return;
            };
            let (albedo_target, accumulation_target) =
                (targets.albedo_framebuffer, targets.accumulation_framebuffer);
            let accumulation = targets.accumulation;
            self.h3_upload(gl, frame, slot);
            if frame.active_camo {
                // What the object hides, before anything of it lands. The copy
                // is sRGB so that sampling it reads the linear light it was.
                let behind = self.h3_targets.as_ref().map(|targets| targets.behind);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.bind_texture(glow::TEXTURE_2D, behind);
                gl.copy_tex_image_2d(glow::TEXTURE_2D, 0, glow::SRGB8_ALPHA8, 0, 0, size[0], size[1], 0);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(true, true, true, true);
            let batches: Vec<_> = order
                .iter()
                .filter_map(|&index| frame.preview.batches.get(index))
                .filter_map(|batch| {
                    let material = self.h3_material(frame, batch.material_index as usize)?;
                    Some((batch.clone(), Arc::clone(material)))
                })
                .collect();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(albedo_target));
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear_depth_f32(1.0);
            // The albedo pass lays depth down for the lit one to meet exactly;
            // the preview's own draw rounds differently.
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            for (batch, material) in &batches {
                if let Some(pass) = material.albedo.as_ref() {
                    self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
                }
            }
            // `render_albedo_decals`, with `_z_buffer_mode_decals`' bias.
            gl.enable(glow::BLEND);
            gl.depth_mask(false);
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(DECAL_SLOPE_BIAS, DECAL_DEPTH_BIAS * DEPTH_UNITS);
            gl.color_mask_draw_buffer(1, false, false, false, false);
            for (batch, material) in
                batches.iter().filter(|(_, material)| material.decal && !material.after_lighting)
            {
                h3_set_blend(gl, material.blend);
                self.h3_draw_pass(gl, frame, slot, batch, material, &material.lit, None);
            }
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.color_mask(true, true, true, true);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.disable(glow::BLEND);
            // Generated from the light, for the apply pass below.
            let shadowed = frame.self_shadow && {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                let drawn = self.draw_h2_shadow(gl, frame, slot, order);
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                drawn
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(accumulation_target));
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(true, true, true, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            let (opaque, blended): (Vec<_>, Vec<_>) =
                batches
                    .iter()
                    .filter(|(_, material)| !material.decal || material.after_lighting)
                    .partition(|(_, material)| material.blend == H3Blend::Opaque);
            for (batch, material) in opaque {
                let pass = h3_lit_pass(frame, batch, material);
                self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
            }
            if shadowed {
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::ShadowApply, None);
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LEQUAL);
                gl.disable(glow::CULL_FACE);
            }
            gl.enable(glow::BLEND);
            gl.depth_mask(false);
            for (batch, material) in blended {
                h3_set_blend(gl, material.blend);
                if material.decal {
                    gl.enable(glow::POLYGON_OFFSET_FILL);
                    gl.polygon_offset(DECAL_SLOPE_BIAS, DECAL_DEPTH_BIAS * DEPTH_UNITS);
                }
                self.h3_draw_pass(gl, frame, slot, batch, material, &material.lit, None);
                gl.disable(glow::POLYGON_OFFSET_FILL);
            }
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            // `submit_visibility_and_render`: additive, each light on its own.
            gl.blend_func(glow::ONE, glow::ONE);
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            for (index, light) in frame.preview.h3_lights.iter().enumerate() {
                let values = frame.h2_values_of_object(light.object);
                let colors = if light.object == 0 { frame.change_colors } else { [[1.0; 3]; 4] };
                let Some(lit) = light.lit(values, &colors, bones, frame.time) else {
                    continue;
                };
                self.h3_light = Some((lit, self.slots[slot].h3_textures.gels.get(index).copied().flatten()));
                for (batch, material) in &batches {
                    if let Some(pass) = material.dynamic_light.as_ref() {
                        self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
                    }
                }
                self.h3_light = None;
            }
            let mut bloom = None;
            if frame.active_camo {
                // Camo shows what is behind it, which the game blooms the same
                // with or without the object. A preview's backdrop has no bloom
                // of its own, so what camo hides is left dark for it.
                if frame.bloom {
                    gl.blend_func(glow::ZERO, glow::ZERO);
                    for (batch, material) in &batches {
                        if let Some(pass) = material.camo.as_ref() {
                            self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
                        }
                    }
                    bloom = self.h3_built_bloom(gl, accumulation_target, accumulation, size);
                    gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                    gl.enable(glow::DEPTH_TEST);
                    gl.enable(glow::BLEND);
                }
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                for (batch, material) in &batches {
                    if let Some(pass) = material.camo.as_ref() {
                        self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
                    }
                }
            }
            // `render_object_context`'s shield impact, over every lit part.
            if h3_shield_showing(frame.h2_values.as_ref()) {
                h3_set_blend(gl, H3Blend::Additive);
                for (batch, material) in &batches {
                    if let Some(shield) = material.shield.as_ref() {
                        self.h3_draw_pass(gl, frame, slot, batch, material, &shield.pass, Some(shield));
                    }
                }
                gl.color_mask(true, true, true, true);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            if frame.bloom && bloom.is_none() {
                bloom = self.h3_built_bloom(gl, framebuffer, accumulation, size);
            }
            self.h3_composite(gl, accumulation, bloom);
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
        }
    }

    unsafe fn h3_built_bloom(
        &mut self,
        gl: &glow::Context,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            if self.h3_bloom.is_none() {
                self.h3_bloom = H3Bloom::new(gl);
            }
            let vertices = self.h3_empty_array(gl);
            self.h3_bloom.as_mut()?.build(gl, vertices, framebuffer, accumulation, size)
        }
    }

    unsafe fn h3_composite(
        &mut self,
        gl: &glow::Context,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
    ) {
        unsafe {
            if self.h3_composite_program.is_none() {
                self.h3_composite_program = screen_program(gl, COMPOSITE_FRAGMENT);
            }
            let Some(program) = self.h3_composite_program else {
                return;
            };
            gl.use_program(Some(program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(accumulation));
            let location = gl.get_uniform_location(program, "accumulation");
            gl.uniform_1_i32(location.as_ref(), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, bloom);
            let location = gl.get_uniform_location(program, "bloom");
            gl.uniform_1_i32(location.as_ref(), 1);
            gl.active_texture(glow::TEXTURE0);
            let location = gl.get_uniform_location(program, "tone_curve_constants");
            gl.uniform_4_f32_slice(location.as_ref(), &TONE_CURVE);
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            // The accumulation is premultiplied: what adds light covers nothing.
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
        }
    }

    unsafe fn h3_empty_array(&mut self, gl: &glow::Context) -> glow::NativeVertexArray {
        unsafe {
            *self.h3_empty_vertices.get_or_insert_with(|| gl.create_vertex_array().expect("a vertex array"))
        }
    }
}

/// `set_alpha_blend_mode_no_cache`: source, destination, source alpha and
/// destination alpha factors, and the equation. Motion blur's alpha factors
/// read a blend factor no target here keeps, so its alpha stays as the source.
fn h3_blend_state(blend: H3Blend) -> ([u32; 4], u32) {
    let add = glow::FUNC_ADD;
    match blend {
        H3Blend::Opaque => ([glow::ONE, glow::ZERO, glow::ONE, glow::ZERO], add),
        H3Blend::Additive => ([glow::ONE; 4], add),
        H3Blend::Multiply => ([glow::DST_COLOR, glow::ZERO, glow::DST_ALPHA, glow::ZERO], add),
        H3Blend::AlphaBlend => {
            ([glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA], add)
        }
        H3Blend::InvAlphaBlend => {
            ([glow::ONE_MINUS_SRC_ALPHA, glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::SRC_ALPHA], add)
        }
        H3Blend::DoubleMultiply => {
            ([glow::DST_COLOR, glow::SRC_COLOR, glow::DST_ALPHA, glow::SRC_ALPHA], add)
        }
        H3Blend::PreMultiplied | H3Blend::MultiplyAdd => {
            ([glow::ONE, glow::ONE_MINUS_SRC_ALPHA, glow::ONE, glow::ONE_MINUS_SRC_ALPHA], add)
        }
        H3Blend::Maximum => ([glow::ONE; 4], glow::MAX),
        H3Blend::AddSrcTimesDstAlpha => ([glow::DST_ALPHA, glow::ONE, glow::DST_ALPHA, glow::ONE], add),
        H3Blend::AddSrcTimesSrcAlpha | H3Blend::AddSrcTimesSrcAlphaAccumAlpha => {
            ([glow::SRC_ALPHA, glow::ONE, glow::SRC_ALPHA, glow::ONE], add)
        }
        H3Blend::MotionBlurStatic | H3Blend::MotionBlurInhibit => {
            ([glow::ONE, glow::ZERO, glow::ONE, glow::ZERO], add)
        }
    }
}

/// Whether a mode only adds to the frame. The game's alpha means nothing in a
/// frame it shows whole, but here alpha is what covers the preview's background.
fn h3_adds_light(blend: H3Blend) -> bool {
    matches!(
        blend,
        H3Blend::Additive
            | H3Blend::Maximum
            | H3Blend::AddSrcTimesDstAlpha
            | H3Blend::AddSrcTimesSrcAlpha
            | H3Blend::AddSrcTimesSrcAlphaAccumAlpha
    )
}

unsafe fn h3_set_blend(gl: &glow::Context, blend: H3Blend) {
    let ([source, destination, source_alpha, destination_alpha], equation) = h3_blend_state(blend);
    unsafe {
        gl.blend_equation(equation);
        gl.blend_func_separate(source, destination, source_alpha, destination_alpha);
        gl.color_mask(true, true, true, !h3_adds_light(blend));
    }
}

/// `d3d11_sampler_state_cache::get`: the border is transparent black, and
/// anisotropic filters are trilinear with a limit.
unsafe fn h3_apply_sampler(gl: &glow::Context, target: u32, sampler: H3Sampler) {
    const TEXTURE_MAX_ANISOTROPY: u32 = 0x84FE;
    let address = |mode| match mode {
        BitmapAddressMode::Clamp => glow::CLAMP_TO_EDGE,
        BitmapAddressMode::Mirror => glow::MIRRORED_REPEAT,
        BitmapAddressMode::BlackBorder => glow::CLAMP_TO_BORDER,
        _ => glow::REPEAT,
    } as i32;
    let (minify, magnify, anisotropy) = match sampler.filter {
        BitmapFilterMode::Point => (glow::NEAREST_MIPMAP_NEAREST, glow::NEAREST, 1.0),
        BitmapFilterMode::Bilinear | BitmapFilterMode::LightprobeTextureArray => {
            (glow::LINEAR_MIPMAP_NEAREST, glow::LINEAR, 1.0)
        }
        BitmapFilterMode::Anisotropic2Expensive => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, 2.0),
        BitmapFilterMode::Anisotropic3Expensive => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, 3.0),
        BitmapFilterMode::Anisotropic4Expensive => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, 4.0),
        _ => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, 1.0),
    };
    unsafe {
        gl.tex_parameter_i32(target, glow::TEXTURE_MIN_FILTER, minify as i32);
        gl.tex_parameter_i32(target, glow::TEXTURE_MAG_FILTER, magnify as i32);
        gl.tex_parameter_f32(target, TEXTURE_MAX_ANISOTROPY, anisotropy);
        if target != glow::TEXTURE_CUBE_MAP {
            gl.tex_parameter_i32(target, glow::TEXTURE_WRAP_S, address(sampler.address[0]));
            gl.tex_parameter_i32(target, glow::TEXTURE_WRAP_T, address(sampler.address[1]));
            gl.tex_parameter_i32(target, glow::TEXTURE_WRAP_R, address(sampler.address[0]));
            gl.tex_parameter_f32_slice(target, glow::TEXTURE_BORDER_COLOR, &[0.0; 4]);
        }
    }
}

/// What `render_mesh_part_default` lights a part with.
fn h3_lit_pass<'a>(frame: &ModelGpuFrame, batch: &RenderModelPreviewBatch, material: &'a H3Material) -> &'a Arc<H3Pass> {
    let vertex = frame.preview.indices.get(batch.index_start as usize).copied().unwrap_or(0);
    let kind = frame.preview.h3_prt.as_ref().map_or(H3PrtKind::None, |prt| prt.kind_at(vertex));
    let slot = match kind {
        H3PrtKind::Ambient => Some(0),
        H3PrtKind::Linear => Some(1),
        H3PrtKind::Quadratic => Some(2),
        H3PrtKind::None => None,
    };
    slot.and_then(|slot| material.prt[slot].as_ref()).unwrap_or(&material.lit)
}

/// The engine decodes signed texels biased by 128; the card wants them back.
fn signed_bytes(rgba: &[u8], texels: H3Texels) -> Cow<'_, [u8]> {
    match texels {
        H3Texels::Signed => rgba.iter().map(|byte| byte.wrapping_sub(128)).collect(),
        _ => Cow::Borrowed(rgba),
    }
}

/// `upload_h2_image` with an sRGB format, for the curve the card decodes.
unsafe fn upload_h2_image_linear(
    gl: &glow::Context,
    image: &H2Image,
    texels: H3Texels,
    sampler: Option<H3Sampler>,
) -> Option<glow::NativeTexture> {
    let (format, kind) = match texels {
        H3Texels::Linear => (glow::RGBA8, glow::UNSIGNED_BYTE),
        H3Texels::Srgb => (glow::SRGB8_ALPHA8, glow::UNSIGNED_BYTE),
        H3Texels::Signed => (glow::RGBA8_SNORM, glow::BYTE),
    };
    let format = format as i32;
    let finish = |target| unsafe {
        gl.generate_mipmap(target);
        gl.tex_parameter_i32(target, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
        gl.tex_parameter_i32(target, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
    };
    unsafe {
        let texture = gl.create_texture().ok()?;
        match image {
            H2Image::Flat(image) => {
                if image.width == 0 || image.height == 0 {
                    gl.delete_texture(texture);
                    return None;
                }
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    format,
                    image.width as i32,
                    image.height as i32,
                    0,
                    glow::RGBA,
                    kind,
                    glow::PixelUnpackData::Slice(Some(&signed_bytes(&image.rgba, texels))),
                );
                finish(glow::TEXTURE_2D);
                let wrap = |repeat: bool| if repeat { glow::REPEAT } else { glow::CLAMP_TO_EDGE } as i32;
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap(image.repeat_x));
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap(image.repeat_y));
                if let Some(sampler) = sampler {
                    h3_apply_sampler(gl, glow::TEXTURE_2D, sampler);
                }
            }
            H2Image::Cube { size, faces } => {
                gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
                for (face, rgba) in faces.iter().enumerate() {
                    gl.tex_image_2d(
                        glow::TEXTURE_CUBE_MAP_POSITIVE_X + face as u32,
                        0,
                        format,
                        *size as i32,
                        *size as i32,
                        0,
                        glow::RGBA,
                        kind,
                        glow::PixelUnpackData::Slice(Some(&signed_bytes(rgba, texels))),
                    );
                }
                finish(glow::TEXTURE_CUBE_MAP);
                if let Some(sampler) = sampler {
                    h3_apply_sampler(gl, glow::TEXTURE_CUBE_MAP, sampler);
                }
            }
            H2Image::Volume { width, height, depth, rgba } => {
                gl.bind_texture(glow::TEXTURE_3D, Some(texture));
                gl.tex_image_3d(
                    glow::TEXTURE_3D,
                    0,
                    format,
                    *width as i32,
                    *height as i32,
                    *depth as i32,
                    0,
                    glow::RGBA,
                    kind,
                    glow::PixelUnpackData::Slice(Some(&signed_bytes(rgba, texels))),
                );
                finish(glow::TEXTURE_3D);
                if let Some(sampler) = sampler {
                    h3_apply_sampler(gl, glow::TEXTURE_3D, sampler);
                }
            }
        }
        Some(texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shadow_fades_as_the_probe_grows_even() {
        assert_eq!(h3_shadow_alpha(2.0), 0.0);
        assert_eq!(h3_shadow_alpha(1.0), 0.0);
        assert_eq!(h3_shadow_alpha(5.0), 0.5);
        assert_eq!(h3_shadow_alpha(10.0), 1.0);
        let mut even = DEFAULT_PROBE;
        even.iter_mut().for_each(|channel| channel[1..].fill(0.0));
        let contrast = |probe: &[[f32; 9]; 3]| dominant_light(probe).2;
        assert!(contrast(&DEFAULT_PROBE) > contrast(&even), "{}", contrast(&even));
    }

    #[test]
    fn the_tone_curve_takes_white_to_white() {
        let [limit, first, second, third] = TONE_CURVE;
        let toned = |value: f32| {
            let value = value.min(limit);
            ((value * third + second) * value + first) * value
        };
        assert!((limit - 1.0).abs() < 1e-5);
        assert!((toned(1.0) - 1.0).abs() < 1e-4, "{}", toned(1.0));
        assert!((toned(0.5) - 0.6875).abs() < 1e-3, "{}", toned(0.5));
        assert!((toned(3.0) - 1.0).abs() < 1e-4, "{}", toned(3.0));
    }

    #[test]
    fn a_shield_shows_once_either_reading_rises() {
        let values = |name: &str, value: f32| {
            h2_functions::H2FunctionValues::from([(name.to_owned(), value)])
        };
        assert!(h3_shield_showing(Some(&values(H3_SHIELD_DAMAGE, 0.5))));
        assert!(h3_shield_showing(Some(&values(H3_OVERSHIELD, 0.5))));
        assert!(!h3_shield_showing(Some(&values(H3_OVERSHIELD, 0.0))));
        assert!(!h3_shield_showing(Some(&values("body_vitality", 1.0))));
        assert!(!h3_shield_showing(None));
    }

    #[test]
    fn only_modes_that_add_light_leave_coverage_alone() {
        assert!(h3_adds_light(H3Blend::Additive));
        assert!(h3_adds_light(H3Blend::AddSrcTimesSrcAlpha));
        assert!(!h3_adds_light(H3Blend::AlphaBlend));
        assert!(!h3_adds_light(H3Blend::Opaque));
    }

    #[test]
    fn the_dominant_light_points_at_the_bright_side_and_turns_with_it() {
        let (fixed, intensity, _) = dominant_light(&DEFAULT_PROBE);
        assert!(fixed[1] > 0.9 && fixed[0] < 0.0, "{fixed:?}");
        assert!(intensity[..3].iter().all(|value| *value > 0.0), "{intensity:?}");
        let (turned, again, _) = dominant_light(&turned_probe(std::f32::consts::FRAC_PI_2));
        assert!((turned[0] + fixed[1]).abs() < 1e-4 && (turned[1] - fixed[0]).abs() < 1e-4, "{turned:?}");
        assert!((again[0] - intensity[0]).abs() < 1e-3);
    }

    #[test]
    fn frames_have_their_own_upload_names() {
        assert_ne!(frame_key("base", 0), frame_key("base", 1));
        assert_ne!(frame_key("base", 0), "base");
    }

    #[test]
    fn no_turn_leaves_the_probe_alone() {
        for (turned, stored) in turned_probe(0.0).iter().flatten().zip(DEFAULT_PROBE.iter().flatten()) {
            assert!((turned - stored).abs() < 1e-5);
        }
    }

    #[test]
    fn a_turn_moves_the_bright_side_and_keeps_the_sky_and_the_energy() {
        let turned = turned_probe(std::f32::consts::FRAC_PI_2);
        let [x, y] = [turned[0][3], turned[0][1]];
        let [stored_x, stored_y] = [DEFAULT_PROBE[0][3], DEFAULT_PROBE[0][1]];
        assert!((x + stored_y).abs() < 1e-5 && (y - stored_x).abs() < 1e-5, "{x} {y}");
        assert_eq!(turned[0][0], DEFAULT_PROBE[0][0]);
        assert_eq!(turned[0][2], DEFAULT_PROBE[0][2]);
        let energy = |channel: &[f32; 9]| channel[4..].iter().map(|value| value * value).sum::<f32>();
        assert!((energy(&turned[0]) - energy(&DEFAULT_PROBE[0])).abs() < 1e-4);
        assert!((turned[0][4] - DEFAULT_PROBE[0][4]).abs() > 0.01);
    }

    #[test]
    fn signed_texels_lose_the_engines_bias() {
        assert_eq!(&*signed_bytes(&[128, 255, 0, 1], H3Texels::Signed), &[0, 127, 128, 129]);
    }

    #[test]
    fn other_texels_pass_through_untouched() {
        assert_eq!(&*signed_bytes(&[128, 255], H3Texels::Linear), &[128, 255]);
        assert_eq!(&*signed_bytes(&[128, 255], H3Texels::Srgb), &[128, 255]);
    }
}
