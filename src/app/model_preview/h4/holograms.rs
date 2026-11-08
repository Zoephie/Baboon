//! Halo 4's holograms, as `c_render_hologram::RenderHolograms` draws them
//! (X360 0x829B63C8, MCC 0x1408FB6B0; `docs/h4-preview-research/holograms.md`).
//! They leave the scene's passes for a surface of their own, depth-tested
//! against a copy of the scene's depth (MCC), take a bloom of their own, go
//! through `final_combine_hologram` (explicit shader 22) and blend into the
//! scene's light through `hologram_composite` (24) as one transparent, placed
//! at their sphere.
//!
//! The screen passes run on a texcoord that climbs the picture, as GL's
//! targets are stored. The game's screen rectangles run down it, so each is
//! handed over mirrored: a coverage `(x, top, width, height)` becomes
//! `(x, H - top, width, -height)`, and the shader's aligned texcoord and
//! distortion come out as the game's.

use super::super::gen3::state::Gen3State;
use super::render::H4Game;
use std::collections::HashMap;
use std::sync::Arc;

use eframe::glow::{self, HasContext};

use super::super::gen3::pipeline::{frame_value, h3_set_cull, set_value, upload_h2_image_linear};
use super::super::gl::target::{Made, Sampling, Texels};
use super::super::gen3::shaders::{H3Dialect, H3Material, H3Pass, H3Value};
use super::bloom::H4BloomSettings;
use super::submit::{DrawnObject, H4Draw, H4HologramPost, H4ScreenMaterial};
use super::super::renderer::*;
use super::super::common::camera::*;
use super::super::common::view::ce_flare_view;
use super::super::*;

/// `g_hologramBoundsScale` and `g_hologramBoundsExpand`'s defaults.
const BOUNDS_SCALE: f32 = 1.0;
const BOUNDS_EXPAND: f32 = 0.0;

/// `final_combine_hologram`'s tone curve: the hologram camera fx's is not
/// `enabled` (flags 0x11), so it is identity (`SetFilmicToneCurveParameters`,
/// X360 0x83AFD0A0). `hologram_composite` undoes the view's.
const IDENTITY_CURVE: [f32; 5] = [0.0, 1.0, 0.0, 0.0, 1.0];

/// The surfaces: `_surface_hologram_bias` (62) with its depth (63), and
/// `_surface_hologram_temp` (61), all RGBA8 at the picture's size.
pub(in crate::app::model_preview) struct H4Holograms {
    size: [i32; 2],
    color: glow::NativeTexture,
    depth: glow::NativeTexture,
    framebuffer: glow::NativeFramebuffer,
    combined: glow::NativeTexture,
    combined_framebuffer: glow::NativeFramebuffer,
    /// The screen materials' bitmaps, for the post they were uploaded from.
    uploaded: Option<(usize, HashMap<String, glow::NativeTexture>)>,
}

impl H4Holograms {
    unsafe fn new(gl: &glow::Context, size: [i32; 2]) -> Option<Self> {
        let mut made = Made::new(gl);
        unsafe {
            let color = made.texture(size, Texels::float(glow::RGBA8), Sampling::LINEAR_CLAMPED)?;
            let combined = made.texture(size, Texels::float(glow::RGBA8), Sampling::LINEAR_CLAMPED)?;
            // Only drawn into, never sampled.
            let depth = made.texture(size, Texels::DEPTH24_STENCIL8, Sampling::NEAREST)?;
            let framebuffer =
                made.framebuffer(&[(glow::COLOR_ATTACHMENT0, color), (glow::DEPTH_STENCIL_ATTACHMENT, depth)])?;
            let combined_framebuffer = made.framebuffer(&[(glow::COLOR_ATTACHMENT0, combined)])?;
            made.keep();
            Some(Self { size, color, depth, framebuffer, combined, combined_framebuffer, uploaded: None })
        }
    }

    unsafe fn delete(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_framebuffer(self.combined_framebuffer);
            for texture in [self.color, self.depth, self.combined] {
                gl.delete_texture(texture);
            }
            for texture in self.uploaded.into_iter().flat_map(|(_, map)| map.into_values()) {
                gl.delete_texture(texture);
            }
        }
    }
}

/// Where the holograms land on the picture, in GL's pixels (y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) struct HologramScreen {
    /// `ps_material_generic_parameters[0]`, mirrored (see the module).
    coverage: [f32; 4],
    /// The scissor, `[x0, y0, x1, y1]`.
    crop: [i32; 4],
    /// `ps_material_generic_parameters[1]`: `(u0, u1, t0, t1)`.
    texcoords: [f32; 4],
    /// `GetHologramBloomIntensityScale`.
    bloom_scale: f32,
}

/// `GetHologramScreenBounds` (X360 0x83B2DE68), `GetHologramScreenCoverage`
/// (0x83B2E210) and `GetHologramBloomIntensityScale` (0x83B2E170) for one
/// sphere, its centre `center` and radius `radius`. `window` maps a world
/// point to GL pixels and its view depth, or none behind the eye; `right` and
/// `up` are the camera's; `vertical_fov` is in radians and `size` the
/// picture's. None when nothing of it is on the picture.
pub(in crate::app::model_preview) fn hologram_screen(
    center: [f32; 3],
    radius: f32,
    window: impl Fn([f32; 3]) -> Option<([f32; 2], f32)>,
    right: [f32; 3],
    up: [f32; 3],
    camera_inside: bool,
    vertical_fov: f32,
    size: [i32; 2],
) -> Option<HologramScreen> {
    let [width, height] = size.map(|side| side as f32);
    let along = |axis: [f32; 3], by: f32| std::array::from_fn(|i| center[i] + axis[i] * by);
    // The world box of one sphere encloses both the last frame's sphere and
    // this one, which are the same here.
    let corners = (0..8).map(|corner: usize| {
        std::array::from_fn(|axis| center[axis] + if corner >> axis & 1 == 1 { radius } else { -radius })
    });
    let sphere = [along(right, radius), along(right, -radius), along(up, radius), along(up, -radius)];
    // A point behind the eye lands at the origin, as the game's does.
    let project = |point: [f32; 3]| window(point).unwrap_or(([0.0; 2], 0.0));
    let enclose = |points: &mut dyn Iterator<Item = ([f32; 2], f32)>| {
        points.fold([f32::MAX, f32::MIN, f32::MAX, f32::MIN, f32::MAX, f32::MIN], |b, ([x, y], z)| {
            [b[0].min(x), b[1].max(x), b[2].min(y), b[3].max(y), b[4].min(z), b[5].max(z)]
        })
    };
    let scale_rect = enclose(&mut sphere.iter().map(|point| project(*point)));
    let screen_rect = enclose(&mut corners.chain(sphere).map(project));
    let [x0, x1, y0, y1, z0, z1] = scale_rect;
    // Window y runs down the picture in the game: its top is GL's highest y.
    let coverage = [x0, y1, x1 - x0, -(y1 - y0)];
    let force_full = camera_inside || BOUNDS_SCALE <= 0.0;
    let (crop, texcoords) = if force_full {
        ([0, 0, size[0], size[1]], [0.0, 1.0, 0.0, 1.0])
    } else {
        let [sx0, sx1, sy0, sy1, ..] = screen_rect;
        let ex = BOUNDS_EXPAND + BOUNDS_SCALE * (sx1 - sx0) - (sx1 - sx0);
        let ey = BOUNDS_EXPAND + BOUNDS_SCALE * (sy1 - sy0) - (sy1 - sy0);
        let crop = [
            ((sx0 - ex).floor() as i32).max(0),
            ((sy0 - ey).floor() as i32).max(0),
            ((sx1 + ex).ceil() as i32).min(size[0]),
            ((sy1 + ey).ceil() as i32).min(size[1]),
        ];
        if crop[2] <= crop[0] || crop[3] <= crop[1] {
            return None;
        }
        let texcoords = [crop[0] as f32 / width, crop[2] as f32 / width, crop[1] as f32 / height, crop[3] as f32 / height];
        (crop, texcoords)
    };
    // `render_camera_build_projection`: `projection+0xC0` is
    // `1 / (vfov / 0.86 · 720 / H)`; the scale averages the near and far
    // depths of the sphere's four points.
    let per_depth = (vertical_fov / 0.86) * (720.0 / height);
    let bloom_scale = ((z0 * per_depth).max(1.0) + (z1 * per_depth).max(1.0)) / 2.0;
    Some(HologramScreen { coverage, crop, texcoords, bloom_scale })
}

/// `GenerateHologramBipedBoundsCallback`'s sphere for the opened object:
/// `object_compute_bounding_sphere_recursive` (X360 0x84A489C0) puts its
/// `bounding offset` through node 0's matrix and takes its `bounding radius`
/// (at scale 1), then `GetScaledBounds` scales the radius by node
/// `min(1, count - 1)`'s skinning matrix, which carries an animation's scale.
/// With no bounding sphere read, the preview's bounds stand in.
fn hologram_sphere(frame: &ModelGpuFrame) -> ([f32; 3], f32) {
    let preview = &frame.preview;
    let Some((offset, radius)) = preview.h4.bounding_sphere else {
        let (low, high) = (preview.bounds_min, preview.bounds_max);
        let center: [f32; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        return (center, (0..3).map(|axis| (high[axis] - center[axis]).powi(2)).sum::<f32>().sqrt());
    };
    let bones = frame.bones.as_deref();
    let skinning = |node: usize| -> Option<[[f32; 4]; 3]> { bones?.get(node * 3..node * 3 + 3)?.try_into().ok() };
    let node_count = preview.nodes.len();
    let root_bind = preview.nodes.first().map(|node| node.inverse_bind);
    scaled_sphere(offset, radius, root_bind, skinning(0), skinning(1.min(node_count.saturating_sub(1))))
}

/// The sphere from the tag's offset and radius, node 0's inverse bind and
/// skinning rows (none at rest), and the scaled node's skinning rows.
fn scaled_sphere(
    offset: [f32; 3],
    radius: f32,
    root_inverse_bind: Option<[[f32; 4]; 3]>,
    root_skinning: Option<[[f32; 4]; 3]>,
    scaled_skinning: Option<[[f32; 4]; 3]>,
) -> ([f32; 3], f32) {
    use crate::app::model_preview::common::math::{IDENTITY, place_of, rigid_inverse};
    use blam_tags::math::Matrix4;
    let rows = |rows: [[f32; 4]; 3]| Matrix4 { m: [rows[0], rows[1], rows[2], [0.0, 0.0, 0.0, 1.0]] };
    // Node 0's world matrix is its skinning matrix times its bind pose.
    let bind = root_inverse_bind.map_or(IDENTITY, |inverse| rigid_inverse(&rows(inverse)));
    let world = root_skinning.map_or(bind, |skinning| rows(skinning) * bind);
    let point = Matrix4 { m: [[1.0, 0.0, 0.0, offset[0]], [0.0, 1.0, 0.0, offset[1]], [0.0, 0.0, 1.0, offset[2]], [0.0, 0.0, 0.0, 1.0]] };
    let center = place_of(&(world * point));
    // `real_matrix4x3`'s scale: the length of an axis.
    let scale = scaled_skinning.map_or(1.0, |rows| (0..3).map(|row| rows[row][0] * rows[row][0]).sum::<f32>().sqrt());
    (center, radius * scale)
}

/// What the generate pass leaves for the composite.
pub(in crate::app::model_preview) struct HologramPicture {
    screen: HologramScreen,
    post: Arc<H4HologramPost>,
    /// The object whose functions the screen materials read
    /// (`g_hologramRenderContext`, the first biped's).
    owner: u8,
    /// How far along the view the sphere's centre lies, for its place among
    /// the transparents (`QueueHologramApply`).
    pub(in crate::app::model_preview) distance: f32,
}

/// A hologram's material is drawn by `RenderHolograms` when its screen
/// materials were read.
pub(in crate::app::model_preview) fn draws_as_hologram(material: &H3Material) -> bool {
    material.h4().is_some_and(|submit| submit.is_hologram() && submit.post().is_some())
}

/// `alpha_blend_max` (22), the blend pass's state: an alpha blend whose alpha
/// keeps the larger [inferred from the name; MCC's state table not read].
unsafe fn set_alpha_blend_max(gl: &glow::Context) {
    unsafe {
        gl.blend_equation_separate(glow::FUNC_ADD, glow::MAX);
        gl.blend_func_separate(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::ONE, glow::ONE);
    }
}

impl H4Game {
    /// `RenderHolograms`' generate pass, after the albedo (`c_player_view::render`
    /// X360 0x82A3B5F4): the holograms into their own surface, then its bloom
    /// and `final_combine_hologram`. `scene` holds the depth to test against.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn generate_holograms(
        &mut self,
        core: &mut RenderCore,
        state: &mut Gen3State,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        holograms: &[(RenderModelPreviewBatch, Arc<H3Material>)],
        scene: glow::NativeFramebuffer,
        size: [i32; 2],
    ) -> Option<HologramPicture> {
        let (first, material) = holograms.first()?;
        let post = Arc::clone(material.h4()?.post()?);
        let owner = frame.preview.materials.get(first.material_index as usize).map_or(0, |material| material.object);
        let (center, radius) = hologram_sphere(frame);
        let view = ce_flare_view(&frame.camera);
        let window = |point: [f32; 3]| {
            let clip = ce_flare_clip(&frame.camera, point);
            let depth: f32 = (0..3).map(|axis| (point[axis] - view.eye[axis]) * view.forward[axis]).sum();
            (clip[3] > 1e-4 && depth > 1e-4).then(|| {
                let ndc = [clip[0] / clip[3], clip[1] / clip[3]];
                ([(ndc[0] * 0.5 + 0.5) * size[0] as f32, (ndc[1] * 0.5 + 0.5) * size[1] as f32], depth)
            })
        };
        // The lens, from how far one unit up the view at the centre's depth
        // reaches on the picture.
        let depth: f32 = (0..3).map(|axis| (center[axis] - view.eye[axis]) * view.forward[axis]).sum();
        let ahead: [f32; 3] = std::array::from_fn(|axis| view.eye[axis] + view.forward[axis] * depth.max(1e-3));
        let raised: [f32; 3] = std::array::from_fn(|axis| ahead[axis] + view.up[axis]);
        let reach = |point| {
            let clip = ce_flare_clip(&frame.camera, point);
            clip[1] / clip[3].max(1e-6)
        };
        let ndc = (reach(raised) - reach(ahead)).abs().max(1e-6);
        let vertical_fov = 2.0 * (1.0 / (depth.max(1e-3) * ndc)).atan();
        let inside = (0..3).all(|axis| (view.eye[axis] - center[axis]).abs() <= radius);
        let screen = hologram_screen(center, radius, window, view.right, view.up, inside, vertical_fov, size)?;
        unsafe {
            if self.surfaces.as_ref().is_none_or(|holograms| holograms.size != size) {
                if let Some(old) = self.surfaces.take() {
                    old.delete(gl);
                }
                self.surfaces = H4Holograms::new(gl, size);
            }
            let (color, framebuffer, combined_framebuffer) = {
                let holograms = self.surfaces.as_ref()?;
                (holograms.color, holograms.framebuffer, holograms.combined_framebuffer)
            };
            // MCC: the scene's depth copied in (`copy_depth`), the colour
            // cleared to nothing.
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(scene));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(framebuffer));
            let [width, height] = size;
            let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
            gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, mask, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.viewport(0, 0, width, height);
            gl.color_mask(true, true, true, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            let [x0, y0, x1, y1] = screen.crop;
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(x0, y0, x1 - x0, y1 - y0);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            // `handle_object_part`: what sorts after the bloom (layer 3)
            // waits for it.
            let post_bloom = |material: &H3Material| material.sort_layer == 3;
            // Pass 0, "hologram opaque": depth written, back faces culled,
            // no blending.
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            h3_set_cull(gl, frame);
            state.submit_draw = Some(H4Draw::HologramOpaque);
            for (batch, material) in holograms.iter().filter(|(_, material)| !post_bloom(material)) {
                state.draw_pass(core, gl, frame, slot, batch, material, &material.lit, None);
            }
            state.submit_draw = None;
            // Mode 1 (`g_disableHologramOrderIndependentTransparency`, the
            // default) then draws pass 2, "hologram blend": blended holograms
            // alone, tested but not written, through `alpha_blend_max`.
            gl.enable(glow::BLEND);
            set_alpha_blend_max(gl);
            gl.depth_mask(false);
            state.submit_draw = Some(H4Draw::HologramBlend);
            for (batch, material) in holograms.iter().filter(|(_, material)| {
                !post_bloom(material) && material.h4().is_some_and(|submit| submit.is_blended_hologram())
            }) {
                state.draw_pass(core, gl, frame, slot, batch, material, &material.lit, None);
            }
            state.submit_draw = None;
            gl.blend_equation(glow::FUNC_ADD);
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            // Its bloom, by the hologram camera fx.
            let vertices = state.empty_array(gl);
            if self.bloom.is_none() {
                self.bloom = super::bloom::H4Bloom::new(gl, core.black_flat, core.white_volume);
            }
            let settings = H4BloomSettings::hologram(screen.bloom_scale);
            let bloom = self.bloom.as_mut()?.build(gl, vertices, framebuffer, color, size, None, &settings);
            // Pass 3, "hologram postbloom": layer 3, by its own blend.
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(x0, y0, x1 - x0, y1 - y0);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            gl.enable(glow::DEPTH_TEST);
            for (batch, material) in holograms.iter().filter(|(_, material)| post_bloom(material)) {
                let blends = material.blend != super::super::gen3::shaders::H3Blend::Opaque;
                if blends {
                    gl.enable(glow::BLEND);
                    super::super::gen3::pipeline::h3_set_blend(gl, material.blend);
                } else {
                    gl.disable(glow::BLEND);
                }
                gl.depth_mask(!blends);
                state.draw_pass(core, gl, frame, slot, batch, material, &material.lit, None);
            }
            gl.disable(glow::BLEND);
            gl.color_mask(true, true, true, true);
            // `postprocess_final_composite(22)` into `_surface_hologram_temp`.
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(combined_framebuffer));
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.disable(glow::DEPTH_TEST);
            gl.depth_mask(false);
            let window_constants = [width as f32, height as f32, 1.0 / width as f32, 1.0 / height as f32];
            let generic = vec![screen.coverage, screen.texcoords, [0.0; 4], [0.0; 4]];
            let engine = [
                ("ps_player_window_constants", H3Value::Floats(window_constants)),
                ("ps_material_generic_parameters", H3Value::FloatArray(generic)),
                ("ps_filmic_tone_curve_params", H3Value::FloatArray(IDENTITY_CURVE.map(|value| [value; 4]).to_vec())),
            ];
            let textures = [("ps_surface_sampler_texture", Some(color)), ("ps_bloom_sampler_texture", bloom.or(core.black_flat))];
            self.screen_pass(core, state, gl, frame, &post, &post.combine, owner, &engine, &textures, size);
            gl.disable(glow::SCISSOR_TEST);
            gl.depth_mask(true);
            gl.enable(glow::DEPTH_TEST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene));
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            Some(HologramPicture { screen, post, owner, distance: depth })
        }
    }

    /// `hologram_apply_callback` (X360 0x83AF4CF0): `hologram_composite` over
    /// the crop into the scene's light, premultiplied (blend mode 5), with no
    /// depth. Leaves the blend as it found it off.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn composite_holograms(
        &mut self,
        core: &mut RenderCore,
        state: &mut Gen3State,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        picture: &HologramPicture,
        size: [i32; 2],
    ) {
        let Some(combined) = self.surfaces.as_ref().map(|holograms| holograms.combined) else {
            return;
        };
        unsafe {
            use super::lighting::{H4_FILMIC_CURVE, h4_filmic_params};
            let view_inverse = h4_filmic_params(H4_FILMIC_CURVE, true);
            let [x0, y0, x1, y1] = picture.screen.crop;
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(x0, y0, x1 - x0, y1 - y0);
            gl.disable(glow::DEPTH_TEST);
            gl.depth_mask(false);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.color_mask(true, true, true, true);
            let engine = [
                ("ps_filmic_tone_curve_params", H3Value::FloatArray(view_inverse.map(|value| [value; 4]).to_vec())),
                ("ps_material_blend_constant", H3Value::Floats([0.0; 4])),
            ];
            let textures = [("source_samplerTexture", Some(combined))];
            let post = Arc::clone(&picture.post);
            self.screen_pass(core, state, gl, frame, &post, &post.composite, picture.owner, &engine, &textures, size);
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::DEPTH_TEST);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
        }
    }

    /// One screen material's pass over the picture: the engine's values
    /// first, then the frame's, then the material's functions on `owner`'s
    /// values, then its bake.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn screen_pass(
        &mut self,
        core: &mut RenderCore,
        state: &mut Gen3State,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        post: &Arc<H4HologramPost>,
        material: &H4ScreenMaterial,
        owner: u8,
        engine: &[(&str, H3Value)],
        targets: &[(&str, Option<glow::NativeTexture>)],
        size: [i32; 2],
    ) {
        unsafe {
            let key = Arc::as_ptr(post) as usize;
            let holograms = self.surfaces.as_mut().expect("the hologram surfaces");
            if holograms.uploaded.as_ref().is_none_or(|(known, _)| *known != key) {
                for texture in holograms.uploaded.take().into_iter().flat_map(|(_, map)| map.into_values()) {
                    gl.delete_texture(texture);
                }
                let mut map = HashMap::new();
                for screen in [&post.combine, &post.composite] {
                    for texture in &screen.textures {
                        if let Some(uploaded) = upload_h2_image_linear(gl, &texture.image, texture.texels, &texture.mips, texture.sampler) {
                            map.insert(texture.uniform.clone(), uploaded);
                        }
                    }
                }
                holograms.uploaded = Some((key, map));
            }
            let uploaded = holograms.uploaded.as_ref().map(|(_, map)| map.clone()).unwrap_or_default();
            let pass: &Arc<H3Pass> = &material.pass;
            state.with_program(core, gl, pass, |state, this, program| {
                gl.use_program(Some(program.program));
                let drawn = DrawnObject { values: frame.function_values_of_object(owner), time: frame.time, owner };
                let node = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
                let functions = material.submit.draw_values(H4Draw::Lit, &drawn, node);
                for (name, ty) in &pass.uniforms {
                    let location = program.location(gl, name);
                    if location.is_none() {
                        continue;
                    }
                    let value = engine
                        .iter()
                        .find(|(known, _)| known == name)
                        .map(|(_, value)| value.clone())
                        .or_else(|| frame_value(frame, H3Dialect::Halo4, name, size, 0.0))
                        .or_else(|| functions.get(name).cloned())
                        .or_else(|| material.values.get(name).cloned());
                    if let Some(value) = value {
                        set_value(gl, location.as_ref(), ty, &value);
                    }
                }
                for (unit, (name, _)) in pass.textures.iter().enumerate() {
                    gl.active_texture(glow::TEXTURE0 + unit as u32);
                    let texture = targets
                        .iter()
                        .find(|(known, _)| known == name)
                        .map(|(_, texture)| *texture)
                        .unwrap_or_else(|| uploaded.get(name).copied())
                        .or(this.placeholder);
                    gl.bind_texture(glow::TEXTURE_2D, texture);
                    gl.uniform_1_i32(program.location(gl, name).as_ref(), unit as i32);
                }
                gl.active_texture(glow::TEXTURE0);
                gl.bind_vertex_array(Some(state.empty_array(gl)));
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sphere rides node 0 and shrinks with node 1's scale: at rest it is
    /// the offset from node 0's bind place; posed half size and moved up, the
    /// radius halves and the centre follows.
    #[test]
    fn the_sphere_follows_the_root_and_the_animations_scale() {
        // Node 0 bound one unit up: its inverse bind takes it back down.
        let inverse_bind = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, -1.0]];
        let (center, radius) = scaled_sphere([0.0, 0.0, 0.3], 0.35, Some(inverse_bind), None, None);
        assert!((center[2] - 1.3).abs() < 1e-6 && radius == 0.35);
        let half = [[0.5, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.0], [0.0, 0.0, 0.5, 2.0]];
        let (center, radius) = scaled_sphere([0.0, 0.0, 0.3], 0.35, Some(inverse_bind), Some(half), Some(half));
        assert!((radius - 0.175).abs() < 1e-6, "{radius}");
        assert!((center[2] - (0.5 * 1.3 + 2.0)).abs() < 1e-5, "{center:?}");
    }

    /// A sphere straight ahead, 10 away, 1 across, through a 90° lens on a
    /// 200 × 100 picture whose pixels are square at that depth.
    fn ahead(point: [f32; 3]) -> Option<([f32; 2], f32)> {
        let depth = point[1];
        (depth > 0.0).then(|| ([100.0 + point[0] / depth * 100.0, 50.0 + point[2] / depth * 50.0], depth))
    }

    #[test]
    fn coverage_is_the_spheres_box_with_the_games_top_as_gl_y() {
        let screen = hologram_screen([0.0, 10.0, 0.0], 1.0, ahead, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], false, 1.0, [200, 100])
            .unwrap();
        assert_eq!(screen.coverage, [90.0, 55.0, 20.0, -10.0], "x, the top in GL's y, width, and height running down");
        // The box's near corners reach further out than the sphere's points.
        let [x0, y0, x1, y1] = screen.crop;
        assert!(x0 < 90 && x1 > 110 && y0 < 45 && y1 > 55, "{:?}", screen.crop);
        assert_eq!(screen.texcoords[0], x0 as f32 / 200.0);
        assert_eq!(screen.texcoords[3], y1 as f32 / 100.0);
    }

    /// Inside its box the whole picture serves; off the picture, nothing.
    #[test]
    fn the_camera_inside_takes_the_whole_picture_and_off_screen_takes_none() {
        let inside = hologram_screen([0.0, 10.0, 0.0], 1.0, ahead, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], true, 1.0, [200, 100])
            .unwrap();
        assert_eq!((inside.crop, inside.texcoords), ([0, 0, 200, 100], [0.0, 1.0, 0.0, 1.0]));
        let aside = |point: [f32; 3]| ahead(point).map(|([x, y], depth)| ([x + 1000.0, y], depth));
        assert!(hologram_screen([0.0, 10.0, 0.0], 1.0, aside, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], false, 1.0, [200, 100]).is_none());
    }

    /// `max(1, d · vfov / 0.86 · 720 / H)`: near, the bloom keeps its own
    /// strength; far, it grows with the distance.
    #[test]
    fn the_bloom_grows_with_distance_past_where_it_starts() {
        let at = |depth: f32| {
            let point = |p: [f32; 3]| ahead([p[0], p[1] - 10.0 + depth, p[2]]);
            hologram_screen([0.0, 10.0, 0.0], 1.0, point, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], false, 1.0, [200, 720])
                .map(|screen| screen.bloom_scale)
        };
        assert_eq!(at(0.5), Some(1.0));
        let far = at(8.6).unwrap();
        assert!((far - 10.0).abs() < 1e-3, "{far}");
    }
}
