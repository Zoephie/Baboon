//! Halo 3's objects, as `render_objects` draws them: an albedo pass that lays
//! albedo and normal into targets, a lit pass reading them back into an HDR
//! accumulation target, and the final composite's tone curve onto the scene.

use super::gl::program::Linked;
use super::gl::target::{Made, Sampling, Texels};
use super::h3_bloom::H3Bloom;
use super::gen3::prt::{H3PrtKind, second_order_rotation};
use std::borrow::Cow;

use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};

use super::gen3::shaders::{H3Animated, H3Blend, H3Dialect, H3Material, H3Pass, H3Sampler, H3Shield, H3Texels, H3Value};
use super::reach_lighting::{
    REACH_COMPOSITE_FRAGMENT, REACH_DARK_MULTIPLIER, reach_composite_gamma, reach_frame_value, reach_noise_offset,
    reach_noise_params,
};
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
pub(in crate::app) const EXPOSURE: f32 = 0.669_432_94;

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

const DISTORTION_FRAGMENT: &str = "#version 410 core
uniform sampler2D scene;
uniform sampler2D displacement;
out vec4 color;
void main() {
    vec2 lean = texelFetch(displacement, ivec2(gl_FragCoord.xy), 0).xy;
    if (lean == vec2(0.0)) {
        discard;
    }
    vec2 place = gl_FragCoord.xy / vec2(textureSize(scene, 0));
    color = texture(scene, place + vec2(lean.x, -lean.y));
}
";

const COMPOSITE_FRAGMENT: &str = "#version 410 core
uniform sampler2D accumulation;
uniform sampler2D bloom;
uniform vec4 tone_curve_constants;
uniform mat4 hue_saturation;
uniform float contrast;
uniform float exposure;
uniform float view_scale;
out vec4 color;
void main() {
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    // A wider lens frames the same picture smaller, about the middle.
    place = (place - 0.5) * view_scale + 0.5;
    vec4 accumulated = texture(accumulation, place);
    if (place.x < 0.0 || place.x > 1.0 || place.y < 0.0 || place.y > 1.0) {
        accumulated = vec4(0.0);
    }
    vec3 blend = (accumulated.rgb + texture(bloom, place).rgb) * exposure;
    blend = (vec4(blend, 1.0) * hue_saturation).rgb;
    float luminance = dot(blend, vec3(0.333));
    if (luminance > 0.0) {
        blend *= pow(luminance, contrast) / luminance;
    }
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

/// `c_screen_postprocess::blur_display`, for how much of it covers the frame.
const BLUR_FRAGMENT: &str = "#version 410 core
uniform sampler2D picture;
uniform vec2 spread;
uniform float cover;
out vec4 color;
void main() {
    vec2 pixel = 1.0 / vec2(textureSize(picture, 0));
    vec2 place = gl_FragCoord.xy * pixel;
    vec3 sum = vec3(0.0);
    float total = 0.0;
    for (int y = -8; y <= 8; y++) {
        for (int x = -8; x <= 8; x++) {
            vec2 at = vec2(x, y) / 4.0;
            float weight = exp(-0.5 * dot(at, at));
            sum += texture(picture, place + at * spread * pixel * 8.0).rgb * weight;
            total += weight;
        }
    }
    color = vec4(sum / total, cover);
}
";

/// `displacement_motion_blur`'s `default_ps`, with the previous frame's view
/// being this one's with the world moved by `moved`. Every fragment here has
/// the alpha the shader's mask looks for.
const SUCK_FRAGMENT: &str = "#version 410 core
uniform sampler2D picture;
uniform sampler2D depth;
uniform mat4 screen_to_world;
uniform mat4 world_to_screen;
uniform vec3 moved;
uniform float most_pixels;
uniform float blur_step;
uniform vec2 center_falloff;
out vec4 color;
void main() {
    vec2 size = vec2(textureSize(picture, 0));
    ivec2 at = ivec2(gl_FragCoord.xy);
    vec3 center = texelFetch(picture, at, 0).rgb;
    vec2 from_center = (gl_FragCoord.xy - size * 0.5) * sqrt(center_falloff) / size;
    float weight = clamp(clamp(dot(from_center, from_center), 0.0, 1.0) - 0.1, 0.0, 1.0);
    color = vec4(center, 1.0);
    if (weight > 0.0) {
        float raw = texelFetch(depth, at, 0).r;
        vec4 place = screen_to_world * vec4(gl_FragCoord.xy / size * 2.0 - 1.0, raw * 2.0 - 1.0, 1.0);
        vec4 before = world_to_screen * vec4(place.xyz / place.w + moved, 1.0);
        vec2 delta = gl_FragCoord.xy - (before.xy / before.w * 0.5 + 0.5) * size;
        // Still and with no blur allowed is 0 / 0, which would poison the steps.
        float scale = clamp(most_pixels / max(length(delta), 1e-6), 0.0, 1.0);
        vec2 step_by = delta * blur_step * scale * weight;
        vec3 sum = center;
        for (int sample_at = 1; sample_at <= 6; sample_at++) {
            ivec2 there = clamp(ivec2(gl_FragCoord.xy + step_by * float(sample_at)), ivec2(0), ivec2(size) - 1);
            sum += texelFetch(picture, there, 0).rgb;
        }
        color.rgb = sum / 7.0;
    }
}
";

/// `hud_camera_nightvision`'s `default_ps`, its narrow filter. The game's depth
/// runs far to near, so the edges are of `game_depth`, not of this one.
const VISION_FRAGMENT: &str = "#version 410 core
uniform sampler2D depth;
uniform sampler2D mask;
uniform mat4 screen_to_world;
uniform vec3 eye;
uniform vec3 forward;
uniform vec2 planes;
uniform float falloff;
uniform vec3 ping;
uniform vec4 default_color;
uniform vec4 ping_color;
out vec4 color;
vec3 world_at(ivec2 at, float raw) {
    vec2 size = vec2(textureSize(depth, 0));
    vec4 place = screen_to_world * vec4((vec2(at) + 0.5) / size * 2.0 - 1.0, raw * 2.0 - 1.0, 1.0);
    return place.xyz / place.w;
}
float game_depth(ivec2 at) {
    float raw = texelFetch(depth, at, 0).r;
    if (raw >= 1.0) {
        return 0.0;
    }
    float along = dot(world_at(at, raw) - eye, forward);
    return planes.x * (planes.y - along) / (along * (planes.y - planes.x));
}
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    float raw = texelFetch(depth, at, 0).r;
    vec2 place = gl_FragCoord.xy / vec2(textureSize(depth, 0));
    float cover = texture(mask, vec2(place.x, 1.0 - place.y)).r;
    if (raw >= 1.0 || cover <= 0.0) {
        discard;
    }
    float middle = game_depth(at);
    vec2 laplacian = vec2(
        game_depth(at + ivec2(1, 0)) + game_depth(at - ivec2(1, 0)) - 2.0 * middle,
        game_depth(at + ivec2(0, 1)) + game_depth(at - ivec2(0, 1)) - 2.0 * middle);
    float gradient = clamp(length(laplacian) / middle, 0.0, 1.0);
    if (gradient <= 1.0 / 255.0) {
        discard;
    }
    if (laplacian.x + laplacian.y > 0.0) {
        gradient *= ping_color.a;
    }
    float distance_away = length(world_at(at, raw) - eye);
    cover *= pow(2.0, -pow(distance_away * falloff, 8.0));
    float after = ping.x - distance_away;
    float boost = pow(clamp(1.0 + ping.z * after, 0.0, 1.0), 4.0) * step(distance_away, ping.x);
    cover *= step(distance_away, ping.y);
    color = vec4(gradient * (default_color.rgb + ping_color.rgb * boost), default_color.a) * cover;
}
";

/// The preview lens' half angle, in `PERSPECTIVE_FOV_Y`'s terms.
const HALF_LENS: f32 = 30.0 * std::f32::consts::PI / 180.0;

/// The targets a frame draws through.
pub(super) struct H3Targets {
    pub(super) size: [i32; 2],
    pub(super) albedo_framebuffer: glow::NativeFramebuffer,
    accumulation_framebuffer: glow::NativeFramebuffer,
    pub(super) albedo: glow::NativeTexture,
    pub(super) normal: glow::NativeTexture,
    accumulation: glow::NativeTexture,
    /// A copy of the scene behind the object, for active camo.
    behind: glow::NativeTexture,
    /// The finished frame as light, for what draws over all of it.
    picture: glow::NativeTexture,
    picture_framebuffer: glow::NativeFramebuffer,
    /// `behind`, to blit into.
    behind_framebuffer: glow::NativeFramebuffer,
    /// A copy of the depth, for what fades against it.
    pub(super) depth_copy: glow::NativeTexture,
    pub(super) depth_copy_framebuffer: glow::NativeFramebuffer,
    /// `apply_distortions`' two: how far each pixel leans, and what it reads.
    distortion: glow::NativeTexture,
    distortion_framebuffer: glow::NativeFramebuffer,
    scene_copy: glow::NativeTexture,
    scene_copy_framebuffer: glow::NativeFramebuffer,
}

/// A full-screen program for `fragment`, or none if it fails to compile or
/// link. A failure is logged once and remembered, so callers that ask again
/// each frame neither rebuild nor log it again.
pub(super) unsafe fn screen_program(gl: &glow::Context, fragment: &str) -> Option<glow::NativeProgram> {
    thread_local! {
        static FAILED: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
    }
    if FAILED.with(|failed| failed.borrow().contains(fragment)) {
        return None;
    }
    let fail = |log: String| {
        eprintln!("model preview: screen shader: {log}");
        FAILED.with(|failed| failed.borrow_mut().insert(fragment.to_owned()));
        None
    };
    unsafe {
        let stages = [(glow::VERTEX_SHADER, COMPOSITE_VERTEX), (glow::FRAGMENT_SHADER, fragment)];
        match super::gl::link::link(gl, &stages, &[]) {
            Ok(program) => Some(program),
            Err(log) => fail(log),
        }
    }
}

impl H3Targets {
    unsafe fn new(gl: &glow::Context, size: [i32; 2], depth: glow::NativeRenderbuffer) -> Option<Self> {
        let mut made = Made::new(gl);
        unsafe {
            let mut target = |internal| made.texture(size, Texels::float(internal), Sampling::NEAREST);
            let albedo = target(glow::RGBA16F)?;
            let normal = target(glow::RGBA16F)?;
            let accumulation = target(glow::RGBA16F)?;
            let behind = target(glow::SRGB8_ALPHA8)?;
            let picture = target(glow::SRGB8_ALPHA8)?;
            let picture_framebuffer = made.framebuffer(&[(glow::COLOR_ATTACHMENT0, picture)])?;
            let behind_framebuffer = made.framebuffer(&[(glow::COLOR_ATTACHMENT0, behind)])?;
            let albedo_framebuffer =
                made.framebuffer(&[(glow::COLOR_ATTACHMENT0, albedo), (glow::COLOR_ATTACHMENT1, normal)])?;
            gl.draw_buffers(&[glow::COLOR_ATTACHMENT0, glow::COLOR_ATTACHMENT1]);
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_STENCIL_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
            let accumulation_framebuffer = made.framebuffer(&[(glow::COLOR_ATTACHMENT0, accumulation)])?;
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_STENCIL_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
            let depth_copy =
                made.depth(size, Texels::DEPTH24_STENCIL8, Sampling::NEAREST, glow::DEPTH_STENCIL_ATTACHMENT)?;
            let distortion = made.color(size, Texels::float(glow::RG16F), Sampling::NEAREST)?;
            let scene_copy = made.color(size, Texels::RGBA16F, Sampling::NEAREST)?;
            made.keep();
            Some(Self {
                size,
                albedo_framebuffer,
                accumulation_framebuffer,
                albedo,
                normal,
                accumulation,
                behind,
                picture,
                picture_framebuffer,
                behind_framebuffer,
                depth_copy: depth_copy.texture,
                depth_copy_framebuffer: depth_copy.framebuffer,
                distortion: distortion.texture,
                distortion_framebuffer: distortion.framebuffer,
                scene_copy: scene_copy.texture,
                scene_copy_framebuffer: scene_copy.framebuffer,
            })
        }
    }

    unsafe fn delete(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.albedo_framebuffer);
            gl.delete_framebuffer(self.accumulation_framebuffer);
            gl.delete_framebuffer(self.depth_copy_framebuffer);
            gl.delete_framebuffer(self.distortion_framebuffer);
            gl.delete_framebuffer(self.scene_copy_framebuffer);
            gl.delete_framebuffer(self.picture_framebuffer);
            gl.delete_framebuffer(self.behind_framebuffer);
            for texture in [
                self.albedo,
                self.normal,
                self.accumulation,
                self.behind,
                self.picture,
                self.depth_copy,
                self.distortion,
                self.scene_copy,
            ] {
                gl.delete_texture(texture);
            }
        }
    }
}

/// A linked pass and where its uniforms landed.
pub(super) struct H3Program {
    /// Held so its address, the program's key, stays its own.
    _pass: Arc<H3Pass>,
    linked: Linked,
}

impl std::ops::Deref for H3Program {
    type Target = Linked;

    fn deref(&self) -> &Linked {
        &self.linked
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
pub(super) fn view_projection(camera: &ModelGpuCamera) -> [f32; 16] {
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
pub(super) fn h3_shield_showing(values: Option<&functions::FunctionValues>) -> bool {
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

/// The matrix a Halo 4 hologram's light rig turns by: the skinning matrix of
/// the object's node 1 (`min(1, count - 1)`, `handle_object_part`), which is
/// `world × inverse_bind` and so none at all in the bind pose. With the light
/// following the camera the rig also turns about the vertical by the eye's
/// heading, so the key light keeps the side it was authored on toward the
/// viewer; this is the preview's own.
fn h4_hologram_node(frame: &ModelGpuFrame) -> [[f32; 4]; 3] {
    let node = 1.min(frame.preview.nodes.len().saturating_sub(1));
    let mut rows = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    if let Some(posed) = frame.bones.as_ref().and_then(|bones| bones.get(node * 3..node * 3 + 3)) {
        rows.copy_from_slice(posed);
    }
    if !frame.light_follows_camera {
        return rows;
    }
    let camera = camera_to_world(frame);
    let heading = camera[6].atan2(camera[2]);
    let (sin, cos) = heading.sin_cos();
    let turn = [[cos, -sin, 0.0], [sin, cos, 0.0], [0.0, 0.0, 1.0]];
    std::array::from_fn(|row| {
        let mut turned: [f32; 4] = std::array::from_fn(|column| (0..3).map(|k| turn[row][k] * rows[k][column]).sum());
        turned[3] = rows[row][3];
        turned
    })
}

/// What a particle's `lighting` option reads.
pub(super) fn h3_lighting_constants(frame: &ModelGpuFrame) -> [[f32; 4]; 10] {
    lighting_constants(&frame_probe(frame))
}

/// What the rasterizer's globals hold for an object here, by uniform name.
/// `stops` is what Reach's area screen effects come to this frame.
pub(super) fn frame_value(frame: &ModelGpuFrame, dialect: H3Dialect, name: &str, size: [i32; 2], stops: f32) -> Option<H3Value> {
    let [width, height] = size.map(|side| side as f32);
    let floats = |value: [f32; 4]| Some(H3Value::Floats(value));
    // Halo 4's engine constants have names of their own.
    if dialect == H3Dialect::Halo4 {
        if name == "ps_lighting_constants" && frame.light_follows_camera {
            let camera = camera_to_world(frame);
            let backward = [camera[2], camera[6], camera[10]];
            return Some(H3Value::FloatArray(super::h4::lighting::h4_turned_vmf(backward).to_vec()));
        }
        return super::h4::lighting::h4_frame_value(name);
    }
    if dialect == H3Dialect::Reach {
        if let Some(value) = reach_frame_value(frame, name, stops) {
            return Some(value);
        }
        // Reach's lighting constants past its four vMF are not an object's.
        if name.starts_with("p_lighting_constant_") || name.starts_with("v_lighting_constant_") {
            return None;
        }
    }
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
        "pixel_kill_enabled" => Some(H3Value::Integer(1)),
        "simple_light_count" | "dynamic_light_shadowing" => Some(H3Value::Integer(0)),
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

/// Reach's `Camera_To_World`: right, up, backward and the eye in its
/// columns, rowed for HLSL, which reads `Camera_Position` and the camera's
/// axes out of it (`hlsl_constant_global_list`).
pub(super) fn camera_to_world(frame: &ModelGpuFrame) -> [f32; 16] {
    let (yaw, pitch) = (frame.camera.yaw, frame.camera.pitch);
    let right = unrotate_view_vector(yaw, pitch, [1.0, 0.0, 0.0]);
    let up = unrotate_view_vector(yaw, pitch, [0.0, 0.0, 1.0]);
    let backward = unrotate_view_vector(yaw, pitch, [0.0, -1.0, 0.0]);
    let eye = h2_eye_position(&frame.camera);
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

/// The rotation Halo 4's albedo pass encodes normals in and its lit pass
/// decodes them from (`EncodeWorldspaceNormal`, `DecodeWorldspaceNormal`).
/// The two only need to agree, but the encoding fails for a normal at -z, so
/// it is the camera's: whatever faces the camera has z toward it. Rowed for
/// HLSL, whose `mul(normal, axis)` turns world into camera space.
fn h4_normal_axis(frame: &ModelGpuFrame) -> [f32; 9] {
    let camera = camera_to_world(frame);
    [camera[0], camera[1], camera[2], camera[4], camera[5], camera[6], camera[8], camera[9], camera[10]]
}

/// The matrices Halo 4's passes read, by name: `mul(position, matrix)` as
/// Halo 3's `View_Projection`, the camera's axes and eye in
/// `camera_to_world`'s columns (`vs_view_camera_right` is `._m00_m10_m20`).
pub(super) fn h4_matrix(frame: &ModelGpuFrame, name: &str) -> Option<Vec<f32>> {
    match name {
        "vs_view_view_projection_matrix" => Some(view_projection(&frame.camera).to_vec()),
        "vs_view_camera_to_world_matrix" | "ps_camera_to_world_matrix" => Some(camera_to_world(frame).to_vec()),
        "ps_worldspace_normal_axis" => Some(h4_normal_axis(frame).to_vec()),
        _ => None,
    }
}

/// A translated uniform's value, by its type.
pub(super) unsafe fn set_uniform_value(gl: &glow::Context, location: Option<&glow::NativeUniformLocation>, ty: &Type, value: &H3Value) {
    unsafe { set_value(gl, location, ty, value) }
}

pub(super) unsafe fn set_value(gl: &glow::Context, location: Option<&glow::NativeUniformLocation>, ty: &Type, value: &H3Value) {
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
            (Type::Array(element, Some(length)), H3Value::FloatArray(rows))
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

impl RenderCore {
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

    pub(in crate::app::model_preview) fn h3_draws(&self, frame: &ModelGpuFrame) -> bool {
        frame.game_shaders
            && self.h3_supported
            && frame.textures.iter().any(|textures| textures.iter().any(|texture| texture.h3.is_some()))
    }

    /// Run `draw` with `pass`'s program, linking it the first time, and keep
    /// it for the next frame whatever `draw` returns. `None` when the pass
    /// does not link. The program leaves the cache while `draw` runs, so
    /// `draw` can use the renderer too.
    pub(super) unsafe fn with_h3_program<R>(
        &mut self,
        gl: &glow::Context,
        pass: &Arc<H3Pass>,
        draw: impl FnOnce(&mut Self, &H3Program) -> R,
    ) -> Option<R> {
        let program = unsafe { self.h3_program(gl, pass) }?;
        Some(draw(self, &program))
    }

    /// `pass`'s program, linked the first time it is asked for. It stays in
    /// the cache while lent, so a pass drawn inside another's draw finds it.
    unsafe fn h3_program(&mut self, gl: &glow::Context, pass: &Arc<H3Pass>) -> Option<Rc<H3Program>> {
        let key = Arc::as_ptr(pass) as usize;
        match self.h3_programs.get(&key) {
            Some(Ok(program)) => return Some(Rc::clone(program)),
            Some(Err(_)) => return None,
            None => {}
        }
        unsafe {
            let stages = [(glow::VERTEX_SHADER, pass.vertex.as_str()), (glow::FRAGMENT_SHADER, pass.pixel.as_str())];
            let mut attributes = Vec::new();
            for (name, preview) in ATTRIBUTES {
                if let Some(location) = gl.get_attrib_location(self.program, preview) {
                    attributes.push((location, name));
                }
            }
            attributes.extend(PRT_ATTRIBUTES.into_iter().zip(PRT_NAMES));
            match super::gl::link::link(gl, &stages, &attributes) {
                Ok(program) => {
                    let program = Rc::new(H3Program { _pass: Arc::clone(pass), linked: Linked::new(program) });
                    self.h3_programs.insert(key, Ok(Rc::clone(&program)));
                    Some(program)
                }
                Err(error) => {
                    eprintln!("h3 program: {error}");
                    self.h3_programs.insert(key, Err(Arc::clone(pass)));
                    None
                }
            }
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
            let reach = frame.preview.reach_shield.as_ref().map(|shield| shield.settings.textures.iter());
            let h4 = frame.preview.h4_shield.as_ref().map(|shield| shield.textures.iter());
            for texture in reach.into_iter().flatten().chain(h4.into_iter().flatten()) {
                if let Some(found) = upload_h2_image_linear(gl, &texture.image, texture.texels, &texture.mips, texture.sampler) {
                    uploaded.shield.insert(texture.uniform.clone(), found);
                }
            }
            if let Some(settings) = settings {
                for texture in &settings.textures {
                    if let Some(found) = upload_h2_image_linear(gl, &texture.image, texture.texels, &texture.mips, texture.sampler) {
                        uploaded.shield.insert(texture.uniform.clone(), found);
                    }
                }
            }
            uploaded.gels = frame
                .preview
                .h3_lights
                .iter()
                .map(|light| light.gel.as_ref().and_then(|gel| upload_h2_image_linear(gl, gel, H3Texels::Srgb, &[], None)))
                .collect();
            for textures in frame.textures.iter().flat_map(|textures| textures.iter()) {
                let mut map = HashMap::new();
                if let Some(material) = &textures.h3 {
                    for texture in &material.textures {
                        if let Some(uploaded) = upload_h2_image_linear(gl, &texture.image, texture.texels, &texture.mips, texture.sampler) {
                            map.insert(texture.uniform.clone(), uploaded);
                        }
                        for (at, image) in texture.frames.iter().enumerate() {
                            if let Some(uploaded) = upload_h2_image_linear(gl, image, texture.texels, &[], texture.sampler) {
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
        let object = batch.material_index as usize;
        let owner = frame.preview.materials.get(object).map_or(0, |material| material.object);
        // Halo 4: the sampler's frame-index function, on the object's values.
        if let Some(submit) = material.h4.as_ref().filter(|_| count > 0) {
            let drawn = super::h4::submit::DrawnObject { values: frame.function_values_of(object), time: frame.time, owner };
            if let Some(index) = submit.frame(uniform, &drawn) {
                return frame_key(uniform, index);
            }
        }
        let animation = material.animations.iter().find(|animation| {
            animation.writes == H3Animated::Frame && animation.uniform == uniform
        });
        match animation.filter(|_| count > 0) {
            Some(animation) => frame_key(uniform, animation.frame(count, frame.time, owner, frame.function_values_of(object))),
            None => uniform.to_owned(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn h3_draw_pass(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        batch: &RenderModelPreviewBatch,
        material: &H3Material,
        pass: &Arc<H3Pass>,
        shield: Option<&H3Shield>,
    ) {
        let range = model_draw_range(batch.index_start, batch.index_count, frame.preview.indices.len());
        if range.is_none() && !pass.fullscreen {
            return;
        }
        unsafe {
            self.with_h3_program(gl, pass, |this, program| {
                gl.use_program(Some(program.program));
                let size = this.h3_targets.as_ref().map_or([1, 1], |targets| targets.size);
                let h4_values = material.h4.as_ref().map(|submit| {
                    let object = batch.material_index as usize;
                    let owner = frame.preview.materials.get(object).map_or(0, |material| material.object);
                    let drawn =
                        super::h4::submit::DrawnObject { values: frame.function_values_of(object), time: frame.time, owner };
                    let albedo = material.albedo.as_ref().is_some_and(|albedo| Arc::ptr_eq(albedo, pass));
                    let fringe = material.fringe.as_ref().is_some_and(|fringe| Arc::ptr_eq(fringe, pass));
                    let draw = match this.h4_draw {
                        Some(draw) => draw,
                        None if albedo => super::h4::submit::H4Draw::Albedo,
                        None if fringe => super::h4::submit::H4Draw::Fringe,
                        None => super::h4::submit::H4Draw::Lit,
                    };
                    let mut values = submit.draw_values(draw, &drawn, h4_hologram_node(frame));
                    // Only the opened object's count is read; attached Halo 4
                    // objects bring geometry alone.
                    let count = if owner == 0 { frame.preview.h4_change_color_count } else { 0 };
                    let colors = super::h4::submit::object_parameters(frame.change_colors_of(object), count);
                    values.insert("ps_material_object_parameters".to_owned(), H3Value::FloatArray(colors));
                    values
                });
                for (name, ty) in &pass.uniforms {
                    let location = program.location(gl, name);
                    if location.is_none() {
                        continue;
                    }
                    match name.as_str() {
                        "View_Projection" => {
                            let matrix = this.reach_view_projection.unwrap_or_else(|| view_projection(&frame.camera));
                            gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix);
                            continue;
                        }
                        "View" => {
                            gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &view_matrix(frame));
                            continue;
                        }
                        "Camera_To_World" => {
                            gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &camera_to_world(frame));
                            continue;
                        }
                        "Nodes" | "vs_model_skinning_matrices" => {
                            this.h3_nodes(gl, frame, location.as_ref(), ty);
                            continue;
                        }
                        "simple_lights" => {
                            if let Some((lit, _)) = &this.h3_light {
                                gl.uniform_4_f32_slice(location.as_ref(), lit.rows.as_flattened());
                            } else if material.dialect == H3Dialect::Reach && !this.reach_inline.is_empty() {
                                let inline = this.reach_inline.as_flattened().as_flattened();
                                gl.uniform_4_f32_slice(location.as_ref(), inline);
                            }
                            continue;
                        }
                        "Shadow_Projection" => {
                            if let Some((lit, _)) = &this.h3_light {
                                gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &lit.projection);
                            }
                            continue;
                        }
                        _ => {}
                    }
                    // The shadow's generate draws from the light.
                    if material.dialect == H3Dialect::Halo4
                        && name == "vs_view_view_projection_matrix"
                        && let Some(matrix) = this.reach_view_projection
                    {
                        gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix);
                        continue;
                    }
                    if material.dialect == H3Dialect::Halo4
                        && let Some(matrix) = h4_matrix(frame, name)
                    {
                        match matrix.len() {
                            9 => gl.uniform_matrix_3_f32_slice(location.as_ref(), false, &matrix),
                            _ => gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix),
                        }
                        continue;
                    }
                    // The dynamic pass borrows this constant for its gel's xform.
                    if name == "p_lighting_constant_4" && this.h3_light.is_some() {
                        gl.uniform_4_f32(location.as_ref(), 1.0, 1.0, 0.0, 0.0);
                        continue;
                    }
                    let change_color = material.change_colors.iter().find(|(uniform, _)| uniform == name);
                    let change_color = change_color.map(|(_, channel)| {
                        let [r, g, b] = frame.change_colors_of(batch.material_index as usize)[*channel];
                        H3Value::Floats([r, g, b, 1.0])
                    });
                    let reach_light =
                        (material.dialect == H3Dialect::Reach).then(|| this.reach_simple_light(name)).flatten();
                    let mut value = change_color
                        .or(reach_light)
                        .or_else(|| frame_value(frame, material.dialect, name, size, this.reach_stops))
                        .or_else(|| h4_values.as_ref().and_then(|values| values.get(name)).cloned())
                        .or_else(|| material.values.get(name).cloned());
                    let object = batch.material_index as usize;
                    let screen_values = this.h3_screen_alpha.map(|alpha| {
                        let mut values = frame.function_values_of(object).cloned().unwrap_or_default();
                        values.insert("alpha".to_owned(), alpha);
                        values
                    });
                    let values = screen_values.as_ref().or(frame.function_values_of(object));
                    let owner = frame.preview.materials.get(object).map_or(0, |material| material.object);
                    for animation in material.animations.iter().filter(|animation| animation.uniform == *name) {
                        let mut slot = match value {
                            Some(H3Value::Floats(slot)) => slot,
                            _ => [0.0; 4],
                        };
                        animation.apply(&mut slot, frame.time, owner, values);
                        value = Some(H3Value::Floats(slot));
                    }
                    if let Some(value) = value {
                        set_value(gl, location.as_ref(), ty, &value);
                    }
                    if let Some(value) = this.reach_pass_values.as_ref().and_then(|values| values.get(name.as_str())) {
                        set_value(gl, location.as_ref(), ty, &H3Value::Floats(*value));
                    }
                    let settings = frame.preview.h3_shield.as_ref().or(shield.map(|shield| &shield.settings));
                    if let Some(value) = shield.and(settings).and_then(|settings| settings.values.get(name)) {
                        set_value(gl, location.as_ref(), ty, value);
                    }
                    if name == "shield_dynamic_quantities" {
                        let reading = |name: &str| {
                            frame.function_values.as_ref().and_then(|values| values.get(name)).copied().unwrap_or(0.0)
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
                let targets = this.h3_targets.as_ref();
                let uploaded = this.slots[slot].h3_textures.materials.get(batch.material_index as usize);
                for (unit, (name, kind)) in pass.textures.iter().enumerate() {
                    gl.active_texture(glow::TEXTURE0 + unit as u32);
                    let texture = match name.as_str() {
                        "albedo_texture" | "ps_view_albedo" => targets.map(|targets| targets.albedo),
                        "normal_texture" | "ps_view_normal" => targets.map(|targets| targets.normal),
                        // No level casts shadows into Halo 4's mask: white.
                        "ps_view_shadow_mask" => this.h4_shadow_mask.or(this.placeholder),
                        "GlobalTexture_scene_ldr_texture" => targets.map(|targets| targets.behind),
                        // Halo 4's camo: what the object hides.
                        "sceneTexture_texture" => this.h4_camo_scene.or(targets.map(|targets| targets.behind)),
                        "depth_buffer" | "psDepthSampler_texture" => this.h3_depth_copy,
                        "shadow_mask_texture" => this.reach_shadow_mask,
                        other if material.screen_maps.iter().any(|map| other.ends_with(map.as_str())) => {
                            targets.map(|targets| targets.behind)
                        }
                        "UserParameterTexture_dynamic_light_gel_texture" => {
                            this.h3_light.as_ref().and_then(|(_, gel)| *gel)
                        }
                        other => uploaded.and_then(|map| {
                            map.get(&this.h3_frame_key(frame, batch, material, other)).or_else(|| map.get(other))
                        }).copied().or_else(|| {
                            let noise = other.strip_prefix("LocalTexture_").unwrap_or(other);
                            this.slots[slot].h3_textures.shield.get(noise).copied()
                        }),
                    };
                    match kind {
                        TextureKind::Line => gl.bind_texture(glow::TEXTURE_1D, texture),
                        TextureKind::Flat => gl.bind_texture(glow::TEXTURE_2D, texture.or(this.placeholder)),
                        TextureKind::Cube => gl.bind_texture(glow::TEXTURE_CUBE_MAP, texture.or(this.black_cube)),
                        TextureKind::Volume => gl.bind_texture(glow::TEXTURE_3D, texture.or(this.black_volume)),
                        // The kit's arrays (light probes, the dominant light's
                        // intensity, water) are built by the game, never authored.
                        TextureKind::FlatArray => gl.bind_texture(glow::TEXTURE_2D_ARRAY, this.h3_black_array),
                        // Reach's per-vertex light probes, which only level
                        // geometry's `static_per_vertex` reads.
                        TextureKind::Bytes => gl.bind_texture(glow::TEXTURE_BUFFER, None),
                    }
                    let location = program.location(gl, name);
                    gl.uniform_1_i32(location.as_ref(), unit as i32);
                }
                let group = frame.preview.materials.get(batch.material_index as usize).map(|known| known.shader_group);
                let two_sided = !frame.show_backfaces && reach_draws_two_sided(material, pass, group);
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

    /// `cloth_render_opaque` builds its mesh from where the sheet is now.
    pub(super) unsafe fn move_cloth(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        if frame.preview.h3_cloth.is_empty() {
            return;
        }
        let uploaded = &mut self.slots[slot];
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let written = uploaded.simulations.cloth.advance(&frame.preview.h3_cloth, bones, frame.time);
        for (first, moved) in written {
            unsafe { uploaded.rewrite_vertices(gl, frame, first, moved) };
        }
    }

    /// The object's passes, between the preview's own draw and the scene's end.
    pub(in crate::app::model_preview) unsafe fn draw_h3(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize, order: &[usize]) {
        // Direct3D 11 always filters across a cube's faces; OpenGL has to be asked.
        unsafe { gl.enable(glow::TEXTURE_CUBE_MAP_SEAMLESS) };
        let Some(scene) = self.scene.as_ref() else {
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
            self.move_cloth(gl, frame, slot);
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
            h3_set_cull(gl, frame);
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
            // Halo 4's holograms draw apart from the scene (`RenderHolograms`).
            let (mut holograms, mut batches): (Vec<_>, Vec<_>) =
                batches.into_iter().partition(|(_, material)| super::h4_holograms::draws_as_hologram(material));
            let reach = frame.preview.game == Some(GameId::HaloReach);
            let halo4 = frame.preview.game == Some(GameId::Halo4);
            // Halo 4's full camo (`render_object_mesh_parts`): a camo object's
            // parts draw only into the shadow and as camo, so nothing of it
            // lands in the light; below full transparency its lit base does.
            let h4_camo = frame
                .preview
                .h4_camo
                .as_ref()
                .filter(|_| halo4 && frame.active_camo)
                .map(|camo| camo.draw(1.0, frame.time));
            let camo_batches: Vec<_> = match h4_camo {
                Some(draw) if !draw.has_base => batches.drain(..).chain(holograms.drain(..)).collect(),
                Some(_) => batches.clone(),
                None => Vec::new(),
            };
            // Reach shadows into a mask ahead of the lights, which is not
            // built, so its self shadow does not run. Halo 4's shadow is not
            // built yet.
            let (bloomed, self_shadow) = (frame.bloom, frame.self_shadow && !reach && !halo4);
            // `screen_effect_sample`, ahead of the scene: Reach's
            // `c_player_view::render` hands its exposure stops to
            // `setup_camera_fx_parameters` before anything draws.
            let (screen, shaders) = {
                let uploaded = &mut self.slots[slot];
                let values = |object: u8| frame.function_values_of_object(object);
                let view = ce_flare_view(&frame.camera);
                let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
                let effects = &frame.preview.h3_screen_effects;
                gen3::screen_effects::h3_screen_settings(effects, values, &mut uploaded.simulations.effect_onsets, bones, &view, frame.time)
            };
            self.reach_stops = if reach { screen.stops() } else { 0.0 };
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
            for (batch, material) in
                batches.iter().filter(|(_, material)| material.decal && !material.after_lighting)
            {
                h3_set_blend(gl, material.blend);
                // `render_mesh_part_default` (Reach X360 0x826754D8): a
                // decal that leaves the specular mask writes the albedo's RGB
                // alone. One that modulates writes alpha too, under a separate
                // alpha blend this preview does not set [inferred: it keeps
                // the colour blend's].
                let keeps_mask = material.dialect == H3Dialect::Reach && !material.decal_modulates_specular;
                gl.color_mask_draw_buffer(0, true, true, true, !keeps_mask);
                // The blend's mask opens every target, so the normals'
                // is set after it. Halo 4's normal decals (`DECAL_OUTPUT_NORMAL`)
                // write the encoded normal's two channels, z and w masked
                // (`decals.fxh`); nothing else here writes normals.
                let normals = material.dialect == H3Dialect::Halo4 && material.lit.pixel.contains("o_target1");
                gl.color_mask_draw_buffer(1, normals, normals, false, false);
                self.h3_draw_pass(gl, frame, slot, batch, material, &material.lit, None);
            }
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.color_mask(true, true, true, true);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.disable(glow::BLEND);
            // `RenderHolograms`' generate pass: after the albedo and its
            // decals, before the lights.
            let hologram = if holograms.is_empty() {
                None
            } else {
                self.h4_generate_holograms(gl, frame, slot, &holograms, albedo_target, size)
            };
            // `render_fullscreen_shadow_buffer`: Reach's mask, between the
            // albedo and the static lighting that reads it.
            let reach_mask =
                if reach && frame.self_shadow { self.draw_reach_shadow(gl, frame, slot, &batches) } else { None };
            // `render_fullscreen_shadow_buffer`: Halo 4's mask, between the
            // albedo and the static lighting that reads it.
            let h4_mask = if halo4 && frame.self_shadow && h4_camo.is_none_or(|draw| draw.has_base) {
                self.draw_h4_shadow(gl, frame, slot, &batches)
            } else {
                None
            };
            // Generated from the light, for the apply pass below.
            let shadowed = self_shadow && {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                let drawn = self.draw_h2_shadow(gl, frame, slot, order);
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                drawn
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(accumulation_target));
            gl.disable(glow::BLEND);
            h3_set_cull(gl, frame);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(true, true, true, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            // `build_simple_light_draw_list`, before the static pass reads it.
            let reach_lit = if reach { self.reach_frame_lights(frame, slot) } else { Vec::new() };
            self.set_reach_inline_lights(&reach_lit);
            let (opaque, blended): (Vec<_>, Vec<_>) =
                batches
                    .iter()
                    .filter(|(_, material)| !material.decal || material.after_lighting)
                    .partition(|(_, material)| material.blend == H3Blend::Opaque);
            if let Some(mask) = reach_mask {
                self.reach_shadow_mask = Some(mask);
                self.reach_pass_values = Some([("shadow_mask_enabled", [1.0; 4])].into_iter().collect());
            }
            if let Some(mask) = h4_mask {
                self.h4_shadow_mask = Some(mask);
                let floor = [0.0, super::super::h4::shadow::DIRECT_FLOOR, 0.0, 0.0];
                self.reach_pass_values = Some([("ps_bsp_lightmap_scale_constants", floor)].into_iter().collect());
            }
            for (batch, material) in opaque {
                let pass = h3_lit_pass(frame, batch, material);
                self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
            }
            self.reach_shadow_mask = None;
            self.h4_shadow_mask = None;
            self.reach_pass_values = None;
            if shadowed {
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::ShadowApply, None);
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LEQUAL);
                h3_set_cull(gl, frame);
            }
            gl.enable(glow::BLEND);
            gl.depth_mask(false);
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            // `submit_visibility_and_render`: additive, each light on its own.
            gl.blend_func(glow::ONE, glow::ONE);
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            for (index, light) in frame.preview.h3_lights.iter().enumerate() {
                if !frame.h3_lights_shown.get(index).copied().unwrap_or(true) {
                    continue;
                }
                let values = frame.function_values_of_object(light.object);
                let colors = frame.change_colors_of_object(light.object);
                let onset = self.slots[slot].simulations.effect_onsets.of(light.object, light.scale_name(), values, frame.time);
                let Some(lit) = light.lit(values, &colors, bones, frame.time, onset) else {
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
            // `MidnightLightParfait::Render`: Halo 4's lights after the static
            // lighting, before what blends.
            if halo4 {
                self.draw_h4_lights(gl, frame, accumulation_target);
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                h3_set_cull(gl, frame);
                gl.depth_func(glow::LEQUAL);
                gl.depth_mask(false);
                gl.blend_func(glow::ONE, glow::ONE);
            }
            // `c_lights_view::render`: after the static lighting, before what blends.
            if reach {
                self.draw_reach_screen_lights(gl, frame, &reach_lit, accumulation_target);
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                h3_set_cull(gl, frame);
                gl.depth_func(glow::LEQUAL);
                gl.depth_mask(false);
                gl.blend_func(glow::ONE, glow::ONE);
            }
            // `c_transparency_renderer`: what blends, far to near, sprites among
            // the materials.
            let sprites = self.draws_object_sprites(frame);
            if sprites {
                if let Some(targets) = self.h3_targets.as_ref() {
                    let [width, height] = targets.size;
                    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                    gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(targets.depth_copy_framebuffer));
                    let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
                    gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, mask, glow::NEAREST);
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                    self.h3_depth_copy = Some(targets.depth_copy);
                    // `submit_distortions`.
                    self.h3_distorted = false;
                    if frame.preview.h3_particles.iter().any(|system| system.look.shader.distortion.is_some()) {
                        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(targets.distortion_framebuffer));
                        gl.clear_color(0.0, 0.0, 0.0, 0.0);
                        gl.clear(glow::COLOR_BUFFER_BIT);
                        self.draw_object_sprites(gl, frame, slot, SpriteStage::Distortions);
                        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(accumulation_target));
                    }
                }
            }
            // Each with the pass it draws and the blend it draws under: its
            // own, or the alpha blend of an `opaque_alpha_blend` fringe.
            let fringes = batches.iter().filter_map(|(batch, material)| {
                let fringe = material.fringe.as_ref()?;
                Some((batch, material, fringe, H3Blend::AlphaBlend))
            });
            let mut blended: Vec<_> = blended
                .into_iter()
                .map(|(batch, material)| (batch, material, &material.lit, material.blend))
                .chain(fringes)
                .map(|(batch, material, pass, blend)| {
                    (h3_layered_distance(material.sort_layer, batch_distance(frame, batch)), batch, material, pass, blend)
                })
                .collect();
            blended.sort_by(|(a, ..), (b, ..)| b.total_cmp(a));
            // `QueueHologramApply`: the holograms' composite is one more
            // transparent, at their sphere.
            let mut hologram = hologram;
            let mut composite_hologram = |this: &mut Self, nearer_than: f32| {
                if let Some(picture) = hologram.take_if(|picture| h3_layered_distance(2, picture.distance) > nearer_than) {
                    this.h4_composite_holograms(gl, frame, slot, &picture, size);
                }
            };
            for (distance, batch, material, pass, blend) in blended {
                composite_hologram(self, distance);
                if sprites {
                    self.draw_object_sprites(gl, frame, slot, SpriteStage::FartherThan(distance));
                }
                gl.enable(glow::BLEND);
                gl.depth_mask(false);
                gl.depth_func(glow::LEQUAL);
                h3_set_cull(gl, frame);
                gl.color_mask(true, true, true, true);
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                h3_set_blend(gl, blend);
                if material.decal {
                    gl.enable(glow::POLYGON_OFFSET_FILL);
                    gl.polygon_offset(DECAL_SLOPE_BIAS, DECAL_DEPTH_BIAS * DEPTH_UNITS);
                }
                self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
                gl.disable(glow::POLYGON_OFFSET_FILL);
            }
            composite_hologram(self, f32::NEG_INFINITY);
            if sprites {
                self.draw_object_sprites(gl, frame, slot, SpriteStage::FartherThan(f32::NEG_INFINITY));
                if self.h3_distorted {
                    self.h3_apply_distortions(gl, accumulation_target, size);
                    gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                }
            }
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.enable(glow::BLEND);
            gl.depth_mask(false);
            // Neither the camo nor the shield can count on a transparent having
            // put the model's own geometry back after the lights and sprites.
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            let mut bloom = None;
            // Reach's: full camo, through `globals`' curves.
            let reach_camo = frame.preview.reach_camo.as_ref().filter(|_| reach).map(|camo| camo.draw(1.0));
            if let Some(draw) = reach_camo {
                let values = [("k_vs_active_camo_factor", draw.factor), ("p_lighting_constant_9", draw.factor)];
                self.reach_pass_values = Some(values.into_iter().collect());
            }
            if frame.active_camo && !halo4 && (!reach || reach_camo.is_some()) {
                // Camo shows what is behind it, which the game blooms the same
                // with or without the object. A preview's backdrop has no bloom
                // of its own, so what camo hides is left dark for it.
                if bloomed {
                    gl.blend_func(glow::ZERO, glow::ZERO);
                    for (batch, material) in &batches {
                        if let Some(pass) = material.camo.as_ref() {
                            self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
                        }
                    }
                    bloom = self.h3_built_bloom(gl, accumulation_target, accumulation, size, reach, halo4.then_some(frame));
                    gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                    gl.enable(glow::DEPTH_TEST);
                    gl.enable(glow::BLEND);
                }
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                // `render_object_callback`: fully transparent camo has no base
                // to blend over, and draws opaque in its place.
                let opaque = reach_camo.is_some_and(|draw| draw.opaque);
                if opaque {
                    gl.disable(glow::BLEND);
                    gl.depth_mask(true);
                }
                for (batch, material) in &batches {
                    if let Some(pass) = material.camo.as_ref() {
                        self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
                    }
                }
                gl.enable(glow::BLEND);
                gl.depth_mask(false);
            }
            self.reach_pass_values = None;
            // `render_object_context`'s shield impact, over every lit part.
            if h3_shield_showing(frame.function_values.as_ref()) {
                h3_set_blend(gl, H3Blend::Additive);
                for (batch, material) in &batches {
                    if let Some(shield) = material.shield.as_ref() {
                        // `render_object_shield_impact`: by the settings that win.
                        let settings = frame.preview.h3_shield.as_ref().unwrap_or(&shield.settings);
                        let pass = shield.procedural.as_ref().filter(|_| settings.procedural).unwrap_or(&shield.pass);
                        self.h3_draw_pass(gl, frame, slot, batch, material, pass, Some(shield));
                    }
                }
                gl.color_mask(true, true, true, true);
            }
            if reach {
                self.draw_reach_shield(gl, frame, slot);
            }
            if halo4 {
                self.draw_h4_shield(gl, frame, slot);
            }
            // `c_player_view::render`: after the transparents, before the bloom
            // and the tone curve, so these are in the same light as the scene.
            if sprites {
                self.draw_object_sprites(gl, frame, slot, SpriteStage::Rest);
                self.draw_h4_flares(gl, frame, slot);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            if bloomed && bloom.is_none() {
                bloom = self.h3_built_bloom(gl, framebuffer, accumulation, size, reach, halo4.then_some(frame));
            }
            if reach || halo4 {
                // The object's depth, which tells the composite what it covers.
                let depth = self.h3_targets.as_ref().map(|targets| {
                    let [width, height] = targets.size;
                    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                    gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(targets.depth_copy_framebuffer));
                    let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
                    gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, mask, glow::NEAREST);
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                    targets.depth_copy
                });
                if halo4 {
                    self.h4_composite(gl, frame, accumulation, bloom, depth, screen);
                    if let Some(draw) = h4_camo {
                        self.draw_h4_camo(gl, frame, slot, &camo_batches, draw, size);
                    }
                } else {
                    let noise = frame.preview.reach_screen_noise.as_ref();
                    self.reach_composite(gl, accumulation, bloom, depth, screen, noise);
                }
            } else {
                self.h3_composite(gl, accumulation, bloom, screen);
            }
            self.reach_stops = 0.0;
            self.h3_finish_picture(gl, frame, slot, framebuffer, size, &screen, &shaders);
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
        }
    }

    /// Halo 4's `c_render_shield::render_object_internal`: every part that
    /// takes the shield through explicit 61, added (`ONE, ONE`) over what
    /// lies behind, no depth written, no face culled. The pass applies its
    /// own exposure.
    unsafe fn draw_h4_shield(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        let Some(shield) = frame.preview.h4_shield.clone() else {
            return;
        };
        let damage = frame.function_values.as_ref().and_then(|values| values.get(H3_SHIELD_DAMAGE)).copied().unwrap_or(0.0);
        if !shield.settings.showing(damage) {
            return;
        }
        let parts: Vec<_> = frame
            .shield_batches
            .iter()
            .filter_map(|&index| frame.preview.batches.get(index))
            .filter_map(|batch| {
                let index = batch.material_index as usize;
                let material = self.h3_material(frame, index)?;
                let textures = frame.textures.as_ref()?.get(index)?;
                super::reach::shield::draws_shield(textures.shield_only, textures.never_shield, material.decal)
                    .then(|| (batch.clone(), Arc::clone(material)))
            })
            .collect();
        if parts.is_empty() {
            return;
        }
        let (low, high) = (frame.preview.bounds_min, frame.preview.bounds_max);
        let center = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let mut values = shield.values(damage, frame.time, frame.function_values.as_ref(), center);
        let camera = &frame.camera;
        let eye = if camera.perspective > 0.0 { 1.0 / camera.perspective } else { 1.0e5 };
        values.insert(
            super::h4::shield::DEPTH_DECODE,
            [camera.depth_scale * camera.depth_map[0], camera.depth_map[1], camera.perspective, eye],
        );
        values.insert(super::h4::shield::WORLD_SCALE, [camera.scale; 4]);
        unsafe {
            if let Some(targets) = self.h3_targets.as_ref() {
                let [width, height] = targets.size;
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(targets.depth_copy_framebuffer));
                let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
                gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, mask, glow::NEAREST);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                self.h3_depth_copy = Some(targets.depth_copy);
            }
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.color_mask(true, true, true, true);
            self.reach_pass_values = Some(values);
            for (batch, material) in &parts {
                self.h3_draw_pass(gl, frame, slot, batch, material, &shield.pass, None);
            }
            self.reach_pass_values = None;
            h3_set_cull(gl, frame);
        }
    }

    /// `c_render_shield::render_object_internal`: every part that takes the
    /// shield, through 61 or 119, added over what lies behind with no depth
    /// written and no face culled.
    unsafe fn draw_reach_shield(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        let Some(shield) = frame.preview.reach_shield.clone() else {
            return;
        };
        let damage = frame.function_values.as_ref().and_then(|values| values.get(H3_SHIELD_DAMAGE)).copied().unwrap_or(0.0);
        if !shield.settings.showing(damage) {
            return;
        }
        let parts: Vec<_> = frame
            .shield_batches
            .iter()
            .filter_map(|&index| frame.preview.batches.get(index))
            .filter_map(|batch| {
                let index = batch.material_index as usize;
                let material = self.h3_material(frame, index)?;
                let textures = frame.textures.as_ref()?.get(index)?;
                super::reach::shield::draws_shield(textures.shield_only, textures.never_shield, material.decal)
                    .then(|| (batch.clone(), Arc::clone(material)))
            })
            .collect();
        if parts.is_empty() {
            return;
        }
        let (low, high) = (frame.preview.bounds_min, frame.preview.bounds_max);
        let center = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let state = shield.settings.state(damage, frame.time, frame.function_values.as_ref());
        let mut values = shield.settings.values(&state, center);
        let camera = &frame.camera;
        let eye = if camera.perspective > 0.0 { 1.0 / camera.perspective } else { 1.0e5 };
        values.insert(
            super::reach::shield::DEPTH_DECODE,
            [camera.depth_scale * camera.depth_map[0], camera.depth_map[1], camera.perspective, eye],
        );
        values.insert(super::reach::shield::WORLD_SCALE, [camera.scale; 4]);
        unsafe {
            // What the pass fades against: everything drawn so far.
            if let Some(targets) = self.h3_targets.as_ref() {
                let [width, height] = targets.size;
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(targets.depth_copy_framebuffer));
                let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
                gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, mask, glow::NEAREST);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                self.h3_depth_copy = Some(targets.depth_copy);
            }
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.color_mask(true, true, true, true);
            self.reach_pass_values = Some(values);
            for (batch, material) in &parts {
                self.h3_draw_pass(gl, frame, slot, batch, material, shield.pass(), None);
            }
            self.reach_pass_values = None;
            h3_set_cull(gl, frame);
        }
    }

    unsafe fn h3_built_bloom(
        &mut self,
        gl: &glow::Context,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
        reach: bool,
        halo4: Option<&ModelGpuFrame>,
    ) -> Option<glow::NativeTexture> {
        unsafe {
            let vertices = self.h3_empty_array(gl);
            if let Some(frame) = halo4 {
                let grading = self.h4_grading_texture(gl, frame);
                if self.h4_bloom.is_none() {
                    self.h4_bloom = super::h4_bloom::H4Bloom::new(gl, self.h3_no_bloom, self.white_volume);
                }
                let settings = &super::h4_bloom::VIEW_BLOOM;
                return self.h4_bloom.as_mut()?.build(gl, vertices, framebuffer, accumulation, size, grading.as_ref(), settings);
            }
            if reach {
                if self.reach_bloom.is_none() {
                    self.reach_bloom = super::reach_bloom::ReachBloom::new(gl, self.h3_no_bloom);
                }
                return self.reach_bloom.as_mut()?.build(gl, vertices, framebuffer, accumulation, size);
            }
            if self.h3_bloom.is_none() {
                self.h3_bloom = H3Bloom::new(gl, self.h3_no_bloom);
            }
            self.h3_bloom.as_mut()?.build(gl, vertices, framebuffer, accumulation, size)
        }
    }

    /// `apply_distortions`: each pixel the distortions leaned reads the scene
    /// from where they leaned it.
    unsafe fn h3_apply_distortions(
        &mut self,
        gl: &glow::Context,
        accumulation_target: glow::NativeFramebuffer,
        size: [i32; 2],
    ) {
        unsafe {
            if self.h3_distortion_program.is_none() {
                self.h3_distortion_program = screen_program(gl, DISTORTION_FRAGMENT);
            }
            let (Some(program), Some(targets)) = (self.h3_distortion_program, self.h3_targets.as_ref()) else {
                return;
            };
            let [width, height] = size;
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(accumulation_target));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(targets.scene_copy_framebuffer));
            gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(accumulation_target));
            gl.use_program(Some(program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(targets.scene_copy));
            gl.uniform_1_i32(gl.get_uniform_location(program, "scene").as_ref(), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(targets.distortion));
            gl.uniform_1_i32(gl.get_uniform_location(program, "displacement").as_ref(), 1);
            gl.active_texture(glow::TEXTURE0);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
        }
    }

    unsafe fn h3_composite(
        &mut self,
        gl: &glow::Context,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: gen3::screen_effects::H3ScreenSettings,
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
            gl.bind_texture(glow::TEXTURE_2D, bloom.or(self.h3_no_bloom));
            let location = gl.get_uniform_location(program, "bloom");
            gl.uniform_1_i32(location.as_ref(), 1);
            gl.active_texture(glow::TEXTURE0);
            let location = gl.get_uniform_location(program, "tone_curve_constants");
            gl.uniform_4_f32_slice(location.as_ref(), &TONE_CURVE);
            let matrix = screen.color_matrix();
            let columns: [f32; 16] = std::array::from_fn(|at| matrix[at % 4][at / 4]);
            let location = gl.get_uniform_location(program, "hue_saturation");
            gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &columns);
            gl.uniform_1_f32(gl.get_uniform_location(program, "contrast").as_ref(), screen.gamma());
            gl.uniform_1_f32(gl.get_uniform_location(program, "exposure").as_ref(), screen.exposure());
            gl.uniform_1_f32(gl.get_uniform_location(program, "view_scale").as_ref(), screen.view_scale(HALF_LENS));
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            // The accumulation is premultiplied: what adds light covers nothing.
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
        }
    }

    /// Halo 4's colour grading lookup, uploaded once per preview, and how a
    /// pass reads its texel centres (`SetColorGradingParameters`).
    unsafe fn h4_grading_texture(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) -> Option<super::h4_bloom::H4BloomGrading> {
        unsafe {
            let grading = frame.preview.h4_color_grading.as_ref();
            let key = grading.map(|(image, ..)| Arc::as_ptr(image) as usize);
            if self.h4_grading.map(|(known, _)| Some(known)) != Some(key) {
                if let Some((_, texture)) = self.h4_grading.take() {
                    gl.delete_texture(texture);
                }
                // Sampler 8: bilinear, clamped.
                let sampler = H3Sampler { address: [BitmapAddressMode::Clamp; 2], filter: BitmapFilterMode::Bilinear };
                let uploaded = grading.and_then(|(image, _, _)| upload_h2_image_linear(gl, image, H3Texels::Linear, &[], Some(sampler)));
                self.h4_grading = key.zip(uploaded);
            }
            let size = match grading.map(|(image, ..)| &**image) {
                Some(H2Image::Volume { width, height, depth, .. }) => [*width, *height, *depth],
                _ => return None,
            };
            let (scale, offset) = super::super::h4::lighting::h4_grading_scale_offset(size);
            self.h4_grading.map(|(_, texture)| super::h4_bloom::H4BloomGrading { texture, scale, offset })
        }
    }

    /// Halo 4's `final_composite`: see `h4::lighting::H4_COMPOSITE_FRAGMENT`.
    unsafe fn h4_composite(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        depth: Option<glow::NativeTexture>,
        screen: gen3::screen_effects::H3ScreenSettings,
    ) {
        use super::super::h4::lighting::{H4_COMPOSITE_FRAGMENT, H4_DARK_MULTIPLIER};
        unsafe {
            let grading = self.h4_grading_texture(gl, frame);
            if self.h4_composite_program.is_none() {
                self.h4_composite_program = screen_program(gl, H4_COMPOSITE_FRAGMENT);
            }
            let Some(program) = self.h4_composite_program else {
                return;
            };
            gl.use_program(Some(program));
            let bind = |unit: u32, target: u32, texture: Option<glow::NativeTexture>, name: &str| {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(target, texture);
                gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit as i32);
            };
            bind(0, glow::TEXTURE_2D, Some(accumulation), "accumulation");
            bind(1, glow::TEXTURE_2D, bloom.or(self.h3_no_bloom), "bloom");
            bind(2, glow::TEXTURE_2D, depth, "scene_depth");
            bind(3, glow::TEXTURE_3D, grading.as_ref().map(|grading| grading.texture).or(self.white_volume), "grading");
            gl.active_texture(glow::TEXTURE0);
            let (scale, offset) = grading.as_ref().map_or(([1.0; 3], [0.0; 3]), |grading| (grading.scale, grading.offset));
            gl.uniform_3_f32_slice(gl.get_uniform_location(program, "grading_scale").as_ref(), &scale);
            gl.uniform_3_f32_slice(gl.get_uniform_location(program, "grading_offset").as_ref(), &offset);
            gl.uniform_1_f32(gl.get_uniform_location(program, "dark_multiplier").as_ref(), H4_DARK_MULTIPLIER);
            let filmic = super::super::h4::lighting::h4_filmic_params(super::super::h4::lighting::H4_FILMIC_CURVE, false);
            gl.uniform_1_f32_slice(gl.get_uniform_location(program, "filmic").as_ref(), &filmic);
            gl.uniform_1_f32(gl.get_uniform_location(program, "view_scale").as_ref(), screen.view_scale(HALF_LENS));
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
        }
    }

    /// `c_render_active_camo::render_object_callback` for a Halo 4 object,
    /// over the finished picture: each part's `active_camo` entry point reads
    /// a copy of the picture so far (`resolve_framebuffer_for_active_camo`),
    /// `alpha_blend`. The game reads and writes its light ahead of the tone
    /// curve; the preview's backdrop is the picture, and at full camo nothing
    /// of the object is in it, so reading and writing the picture's bytes is
    /// the same. Fully transparent, nothing of the object is under it,
    /// and the nearest part alone shows, as its depth decides. Otherwise its
    /// lit base wrote depth, which the camo meets nudged toward the eye by
    /// `camo depth bias function`'s constant.
    unsafe fn draw_h4_camo(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        batches: &[(RenderModelPreviewBatch, Arc<H3Material>)],
        draw: super::super::h4::camo::H4CamoDraw,
        size: [i32; 2],
    ) {
        /// `camo depth bias function`: the kit's constant, in a 24-bit
        /// buffer's units.
        const DEPTH_BIAS: f32 = -2e-5 * 16_777_216.0;
        let [width, height] = size.map(|side| side as f32);
        self.reach_pass_values = Some(
            [
                ("ps_activeCamoFactor", draw.factor),
                ("ps_distortionTextureTransform", draw.transform),
                ("distortionTextureEnabled", [f32::from(u8::from(draw.textured)); 4]),
                ("ps_textureSize", [width, height, 1.0 / width, 1.0 / height]),
            ]
            .into_iter()
            .collect(),
        );
        unsafe {
            if self.h4_camo_scene.is_none() {
                self.h4_camo_scene = gl.create_texture().ok();
            }
            gl.bind_texture(glow::TEXTURE_2D, self.h4_camo_scene);
            gl.copy_tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8, 0, 0, size[0], size[1], 0);
            for (name, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            h3_set_cull(gl, frame);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            let camo = |this: &mut Self| {
                for (batch, material) in batches {
                    if let Some(pass) = material.camo.as_ref() {
                        this.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
                    }
                }
            };
            if draw.has_base {
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(0.0, DEPTH_BIAS);
            } else {
                gl.depth_mask(true);
                gl.clear(glow::DEPTH_BUFFER_BIT);
                gl.disable(glow::BLEND);
                gl.color_mask(false, false, false, false);
                camo(self);
                gl.color_mask(true, true, true, true);
            }
            gl.depth_mask(false);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            camo(self);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.depth_mask(true);
        }
        self.reach_pass_values = None;
    }

    /// Reach's `final_composite`: see [`REACH_COMPOSITE_FRAGMENT`].
    unsafe fn reach_composite(
        &mut self,
        gl: &glow::Context,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        depth: Option<glow::NativeTexture>,
        screen: gen3::screen_effects::H3ScreenSettings,
        noise: Option<&materials::H3Bitmap>,
    ) {
        unsafe {
            if self.reach_noise.is_none()
                && let Some((image, texels, mips)) = noise
            {
                // Sampler 7: wrapped, point.
                let sampler = H3Sampler { address: [BitmapAddressMode::Wrap; 2], filter: BitmapFilterMode::Point };
                self.reach_noise = upload_h2_image_linear(gl, image, *texels, mips, Some(sampler));
            }
            if self.reach_composite_program.is_none() {
                self.reach_composite_program = screen_program(gl, REACH_COMPOSITE_FRAGMENT);
            }
            let Some(program) = self.reach_composite_program else {
                return;
            };
            gl.use_program(Some(program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(accumulation));
            gl.uniform_1_i32(gl.get_uniform_location(program, "accumulation").as_ref(), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, bloom.or(self.h3_no_bloom));
            gl.uniform_1_i32(gl.get_uniform_location(program, "bloom").as_ref(), 1);
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, depth);
            gl.uniform_1_i32(gl.get_uniform_location(program, "scene_depth").as_ref(), 2);
            gl.active_texture(glow::TEXTURE3);
            gl.bind_texture(glow::TEXTURE_2D, self.reach_noise.or(self.h3_no_bloom));
            gl.uniform_1_i32(gl.get_uniform_location(program, "noise").as_ref(), 3);
            gl.active_texture(glow::TEXTURE0);
            let matrix = screen.color_matrix();
            let columns: [f32; 16] = std::array::from_fn(|at| matrix[at % 4][at / 4]);
            gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(program, "hue_saturation").as_ref(), false, &columns);
            let gamma = reach_composite_gamma(screen.gamma());
            gl.uniform_1_f32(gl.get_uniform_location(program, "gamma").as_ref(), gamma);
            // With no map to read, no noise.
            let [bright, dark] = if self.reach_noise.is_some() { screen.noise() } else { [0.0; 2] };
            let params = reach_noise_params(bright, dark);
            gl.uniform_4_f32_slice(gl.get_uniform_location(program, "noise_params").as_ref(), &params);
            // `real_random`'s two draws, new each frame.
            self.reach_noise_frame = self.reach_noise_frame.wrapping_add(1);
            let frame_number = self.reach_noise_frame as f32;
            let random = [0, 1].map(|salt| reach::shield::real_random(frame_number, salt));
            let [x, y] = reach_noise_offset(self.reach_noise_frame, random);
            gl.uniform_2_f32(gl.get_uniform_location(program, "noise_offset").as_ref(), x, y);
            gl.uniform_1_f32(gl.get_uniform_location(program, "dark_multiplier").as_ref(), REACH_DARK_MULTIPLIER);
            gl.uniform_1_f32(gl.get_uniform_location(program, "view_scale").as_ref(), screen.view_scale(HALF_LENS));
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
        }
    }

    /// `c_player_view`'s last steps: `render_screen_shaders`, then the blur, on
    /// a surface that holds light rather than the encoded frame, as the game's
    /// sRGB one does.
    #[allow(clippy::too_many_arguments)]
    unsafe fn h3_finish_picture(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        framebuffer: glow::NativeFramebuffer,
        size: [i32; 2],
        screen: &gen3::screen_effects::H3ScreenSettings,
        shaders: &[gen3::screen_effects::H3ScreenShader],
    ) {
        let blur = screen.blur();
        let vision = frame.preview.h3_vision.clone().filter(|_| screen.vision() > 0.0);
        let suck = frame.preview.h3_motion_blur.zip(screen.suck());
        if !shaders.iter().any(gen3::screen_effects::H3ScreenShader::at_frame_end)
            && blur.is_none()
            && vision.is_none()
            && suck.is_none()
        {
            return;
        }
        let Some(targets) = self.h3_targets.as_ref() else {
            return;
        };
        let (behind, picture_framebuffer, behind_framebuffer) =
            (targets.behind, targets.picture_framebuffer, targets.behind_framebuffer);
        let (accumulation_framebuffer, depth_copy, depth_copy_framebuffer) =
            (targets.accumulation_framebuffer, targets.depth_copy, targets.depth_copy_framebuffer);
        unsafe {
            // Raw bytes across, so what was encoded reads back as light.
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(picture_framebuffer));
            gl.disable(glow::FRAMEBUFFER_SRGB);
            gl.blit_framebuffer(0, 0, size[0], size[1], 0, 0, size[0], size[1], glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(picture_framebuffer));
            gl.enable(glow::FRAMEBUFFER_SRGB);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.depth_mask(false);
            gl.color_mask(true, true, true, false);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            // A shader reads the picture through texcoords that run top down.
            let screen_copy = |gl: &glow::Context, upright: bool| {
                gl.disable(glow::FRAMEBUFFER_SRGB);
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(picture_framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(behind_framebuffer));
                let (top, bottom) = if upright { (0, size[1]) } else { (size[1], 0) };
                gl.blit_framebuffer(0, 0, size[0], size[1], 0, top, size[0], bottom, glow::COLOR_BUFFER_BIT, glow::NEAREST);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(picture_framebuffer));
                gl.enable(glow::FRAMEBUFFER_SRGB);
            };
            let copy_depth = |gl: &glow::Context| {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(accumulation_framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(depth_copy_framebuffer));
                let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
                gl.blit_framebuffer(0, 0, size[0], size[1], 0, 0, size[0], size[1], mask, glow::NEAREST);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(picture_framebuffer));
            };
            // `distortion_apply` comes before everything else here.
            if let Some((blur, moved)) = suck {
                copy_depth(gl);
                screen_copy(gl, true);
                self.h3_draw_suck(gl, frame, &blur, moved, behind, depth_copy);
            }
            for shader in shaders.iter().filter(|shader| shader.at_frame_end()) {
                let screen = u32::from_be_bytes(*b"rmss");
                let found = frame
                    .preview
                    .materials
                    .iter()
                    .position(|material| material.shader_group == screen && material.shader_path == shader.path);
                let textures = frame.textures.as_ref();
                let material = found.and_then(|at| textures?.get(at)?.h3.as_ref().map(|material| (at, material)));
                let Some((at, material)) = material else {
                    continue;
                };
                screen_copy(gl, false);
                h3_set_blend(gl, material.blend);
                self.h3_screen_alpha = Some(shader.alpha);
                let batch = RenderModelPreviewBatch { material_index: at as u16, ..Default::default() };
                self.h3_draw_pass(gl, frame, slot, &batch, material, &material.lit, None);
                self.h3_screen_alpha = None;
            }
            if let Some((spread, cover)) = blur {
                if self.h3_blur_program.is_none() {
                    self.h3_blur_program = screen_program(gl, BLUR_FRAGMENT);
                }
                if let Some(program) = self.h3_blur_program {
                    screen_copy(gl, true);
                    gl.use_program(Some(program));
                    gl.active_texture(glow::TEXTURE0);
                    gl.bind_texture(glow::TEXTURE_2D, Some(behind));
                    gl.uniform_1_i32(gl.get_uniform_location(program, "picture").as_ref(), 0);
                    gl.uniform_2_f32_slice(gl.get_uniform_location(program, "spread").as_ref(), &spread);
                    gl.uniform_1_f32(gl.get_uniform_location(program, "cover").as_ref(), cover);
                    gl.enable(glow::BLEND);
                    gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                    gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
            }
            gl.disable(glow::FRAMEBUFFER_SRGB);
            if let Some(vision) = vision {
                copy_depth(gl);
                self.h3_draw_vision(gl, frame, slot, &vision, screen.vision(), depth_copy);
            }
            gl.color_mask(true, true, true, true);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(picture_framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(framebuffer));
            gl.blit_framebuffer(0, 0, size[0], size[1], 0, 0, size[0], size[1], glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.enable(glow::DEPTH_TEST);
        }
    }

    /// `c_view::distortion_apply`'s motion blur, for what moved the view and not
    /// for the preview's own camera.
    unsafe fn h3_draw_suck(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        blur: &h3::vision::H3MotionBlur,
        moved: [f32; 3],
        picture: glow::NativeTexture,
        depth: glow::NativeTexture,
    ) {
        let matrix = view_projection(&frame.camera);
        let Some(to_world) = h3::vision::inverted(&matrix) else {
            return;
        };
        unsafe {
            if self.h3_suck_program.is_none() {
                self.h3_suck_program = screen_program(gl, SUCK_FRAGMENT);
            }
            let Some(program) = self.h3_suck_program else {
                return;
            };
            gl.use_program(Some(program));
            let spot = |name: &str| gl.get_uniform_location(program, name);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(depth));
            gl.uniform_1_i32(spot("depth").as_ref(), 1);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(picture));
            gl.uniform_1_i32(spot("picture").as_ref(), 0);
            gl.uniform_matrix_4_f32_slice(spot("screen_to_world").as_ref(), false, &to_world);
            gl.uniform_matrix_4_f32_slice(spot("world_to_screen").as_ref(), false, &matrix);
            gl.uniform_3_f32_slice(spot("moved").as_ref(), &moved);
            gl.uniform_1_f32(spot("most_pixels").as_ref(), blur.most_pixels);
            gl.uniform_1_f32(spot("blur_step").as_ref(), blur.step);
            gl.uniform_2_f32_slice(spot("center_falloff").as_ref(), &blur.center_falloff);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    /// `c_hud_camera_view`: over the frame as it will show, not over its light.
    unsafe fn h3_draw_vision(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        vision: &h3::vision::H3Vision,
        amount: f32,
        depth: glow::NativeTexture,
    ) {
        let time = frame.time;
        let clock = &mut self.slots[slot].simulations.h3_vision_clock;
        let on_for = match *clock {
            Some((last, seconds)) if time >= last => seconds + (time - last),
            _ => 0.0,
        };
        *clock = Some((time, on_for));
        let constants = vision.constants(amount, on_for);
        let matrix = view_projection(&frame.camera);
        let Some(to_world) = h3::vision::inverted(&matrix) else {
            return;
        };
        unsafe {
            if self.h3_vision_program.is_none() {
                self.h3_vision_program = screen_program(gl, VISION_FRAGMENT);
            }
            if self.h3_vision_mask.is_none() {
                let Some((image, texels, mips)) = vision.mask() else {
                    return;
                };
                self.h3_vision_mask = upload_h2_image_linear(gl, image, *texels, mips, None);
            }
            let (Some(program), Some(mask)) = (self.h3_vision_program, self.h3_vision_mask) else {
                return;
            };
            let view = ce_flare_view(&frame.camera);
            gl.use_program(Some(program));
            let spot = |name: &str| gl.get_uniform_location(program, name);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(depth));
            gl.uniform_1_i32(spot("depth").as_ref(), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(mask));
            gl.uniform_1_i32(spot("mask").as_ref(), 1);
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_matrix_4_f32_slice(spot("screen_to_world").as_ref(), false, &to_world);
            gl.uniform_3_f32_slice(spot("eye").as_ref(), &view.eye);
            gl.uniform_3_f32_slice(spot("forward").as_ref(), &view.forward);
            gl.uniform_2_f32(spot("planes").as_ref(), h3::vision::GAME_NEAR, h3::vision::GAME_FAR);
            gl.uniform_1_f32(spot("falloff").as_ref(), constants.falloff);
            gl.uniform_3_f32_slice(spot("ping").as_ref(), &constants.ping);
            gl.uniform_4_f32_slice(spot("default_color").as_ref(), &constants.color);
            gl.uniform_4_f32_slice(spot("ping_color").as_ref(), &constants.ping_color);
            // `_alpha_blend_multiply_add`, which is premultiplied.
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    pub(super) unsafe fn h3_empty_array(&mut self, gl: &glow::Context) -> glow::NativeVertexArray {
        unsafe {
            *self.empty_vertex_array.get_or_insert_with(|| gl.create_vertex_array().expect("a vertex array"))
        }
    }
}

#[cfg(test)]
mod layer_tests {
    use super::h3_layered_distance;

    #[test]
    fn a_later_layer_draws_after_however_far_it_lies() {
        let far_post = h3_layered_distance(3, 500.0);
        let near_normal = h3_layered_distance(2, 1.0);
        assert!(far_post < near_normal);
        assert!(h3_layered_distance(2, 5.0) > h3_layered_distance(2, 1.0));
    }
}

/// A distance to sort by, far first, that puts `sort layer` before it.
pub(super) fn h3_layered_distance(layer: i32, distance: f32) -> f32 {
    distance - layer as f32 * SORT_LAYER_SPAN
}

/// More than any preview is deep.
const SORT_LAYER_SPAN: f32 = 1e6;

/// How far along the view a batch's middle lies, as sprites measure theirs.
fn batch_distance(frame: &ModelGpuFrame, batch: &RenderModelPreviewBatch) -> f32 {
    let preview = &frame.preview;
    let start = batch.index_start as usize;
    let indices = preview.indices.get(start..start + batch.index_count as usize).unwrap_or(&[]);
    let places = indices.iter().filter_map(|index| preview.vertices.get(*index as usize));
    let (mut sum, mut count) = ([0.0; 3], 0.0);
    for vertex in places {
        sum = std::array::from_fn(|axis| sum[axis] + vertex.position[axis]);
        count += 1.0;
    }
    let view = ce_flare_view(&frame.camera);
    let middle: [f32; 3] = sum.map(|axis| axis / f32::max(count, 1.0));
    (0..3).map(|axis| (middle[axis] - view.eye[axis]) * view.forward[axis]).sum()
}

/// Whether Reach draws this part with no face culled: `render_mesh_part_default`
/// takes its cull mode from `c_render_method::get_culling_mode` (X360
/// 0x826E9E40), none for `shader_foliage` (`rmfl`, 0x726D666C) and back faces
/// for every other group. Only the material's own passes; a shield or shadow
/// drawn over the part keeps its own culling.
fn reach_draws_two_sided(material: &H3Material, pass: &Arc<H3Pass>, group: Option<u32>) -> bool {
    let foliage = group == Some(u32::from_be_bytes(*b"rmfl"));
    let own = [material.albedo.as_ref(), Some(&material.lit), material.camo.as_ref(), material.dynamic_light.as_ref()]
        .into_iter()
        .chain(material.prt.iter().map(Option::as_ref))
        .flatten()
        .any(|known| Arc::ptr_eq(known, pass));
    material.dialect == H3Dialect::Reach && foliage && own
}

/// `c_object_renderer::render_albedo` and its like: `_cull_mode_cw` throughout.
/// Tags lean on it, storing some faces twice with opposite winding, and with
/// both drawn the back one takes the depth tie and its lighting with it.
pub(super) unsafe fn h3_set_cull(gl: &glow::Context, frame: &ModelGpuFrame) {
    unsafe {
        if frame.show_backfaces {
            gl.disable(glow::CULL_FACE);
        } else {
            gl.enable(glow::CULL_FACE);
            gl.cull_face(glow::BACK);
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

pub(super) unsafe fn h3_set_blend(gl: &glow::Context, blend: H3Blend) {
    let ([source, destination, source_alpha, destination_alpha], equation) = h3_blend_state(blend);
    unsafe {
        gl.blend_equation(equation);
        gl.blend_func_separate(source, destination, source_alpha, destination_alpha);
        gl.color_mask(true, true, true, !h3_adds_light(blend));
    }
}

/// `d3d11_sampler_state_cache::get`: the border is transparent black, and
/// anisotropic filters are trilinear with a limit.
pub(super) unsafe fn h3_apply_sampler(gl: &glow::Context, target: u32, sampler: H3Sampler) {
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
    let prt = frame.preview.h3_prt.as_ref().filter(|_| frame.prt);
    let kind = prt.map_or(H3PrtKind::None, |prt| prt.kind_at(vertex));
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
pub(super) unsafe fn upload_h2_image_linear(
    gl: &glow::Context,
    image: &H2Image,
    texels: H3Texels,
    mips: &[H2Image],
    sampler: Option<H3Sampler>,
) -> Option<glow::NativeTexture> {
    // GL reads as much as the size says, whatever the slice holds.
    if !image.whole() {
        return None;
    }
    let mips = if mips.iter().all(H2Image::whole) { mips } else { &[] };
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
                for (level, mip) in mips.iter().enumerate() {
                    if let H2Image::Flat(mip) = mip {
                        let rgba = signed_bytes(&mip.rgba, texels);
                        let (width, height) = (mip.width as i32, mip.height as i32);
                        let pixels = glow::PixelUnpackData::Slice(Some(&rgba));
                        gl.tex_image_2d(glow::TEXTURE_2D, level as i32 + 1, format, width, height, 0, glow::RGBA, kind, pixels);
                    }
                }
                if !mips.is_empty() {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAX_LEVEL, mips.len() as i32);
                }
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
                for (level, mip) in mips.iter().enumerate() {
                    if let H2Image::Cube { size, faces } = mip {
                        for (face, rgba) in faces.iter().enumerate() {
                            let rgba = signed_bytes(rgba, texels);
                            let target = glow::TEXTURE_CUBE_MAP_POSITIVE_X + face as u32;
                            let side = *size as i32;
                            let pixels = glow::PixelUnpackData::Slice(Some(&rgba));
                            gl.tex_image_2d(target, level as i32 + 1, format, side, side, 0, glow::RGBA, kind, pixels);
                        }
                    }
                }
                if !mips.is_empty() {
                    gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, glow::TEXTURE_MAX_LEVEL, mips.len() as i32);
                }
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

    /// Reach's foliage draws both faces of its own passes; any other group
    /// (a halogram among them), Halo 3's foliage, and a pass laid over the
    /// part keep culling.
    #[test]
    fn only_reach_foliages_own_passes_draw_two_sided() {
        let pass = || Arc::new(H3Pass { fullscreen: false, vertex: String::new(), pixel: String::new(), uniforms: Vec::new(), textures: Vec::new() });
        let material = |dialect| H3Material {
            dialect,
            albedo: None,
            lit: pass(),
            screen_maps: Vec::new(),
            decal: false,
            decal_modulates_specular: false,
            after_lighting: false,
            sort_layer: 0,
            prt: [None, None, Some(pass())],
            camo: None,
            shadow_generate: None,
            dynamic_light: None,
            blend: H3Blend::Additive,
            values: Default::default(),
            textures: Vec::new(),
            change_colors: Vec::new(),
            animations: Vec::new(),
            shield: None,
            h4: None,
            fringe: None,
        };
        let foliage = Some(u32::from_be_bytes(*b"rmfl"));
        let reach = material(H3Dialect::Reach);
        assert!(reach_draws_two_sided(&reach, &reach.lit, foliage));
        assert!(reach_draws_two_sided(&reach, reach.prt[2].as_ref().unwrap(), foliage));
        assert!(!reach_draws_two_sided(&reach, &pass(), foliage), "a shield over it culls");
        assert!(!reach_draws_two_sided(&reach, &reach.lit, Some(u32::from_be_bytes(*b"rmhg"))), "a halogram culls");
        let halo3 = material(H3Dialect::Halo3);
        assert!(!reach_draws_two_sided(&halo3, &halo3.lit, foliage));
    }

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
            functions::FunctionValues::from([(name.to_owned(), value)])
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
