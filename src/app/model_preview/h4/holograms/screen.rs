//! A hologram's targets and its screen: bounds, sphere and picture.

use super::*;

/// `g_hologramBoundsScale` and `g_hologramBoundsExpand`'s defaults.
const BOUNDS_SCALE: f32 = 1.0;

const BOUNDS_EXPAND: f32 = 0.0;

/// `final_combine_hologram`'s tone curve: the hologram camera fx's is not
/// `enabled` (flags 0x11), so it is identity (`SetFilmicToneCurveParameters`,
/// X360 0x83AFD0A0). `hologram_composite` undoes the view's.
pub(super) const IDENTITY_CURVE: [f32; 5] = [0.0, 1.0, 0.0, 0.0, 1.0];

/// The surfaces: `_surface_hologram_bias` (62) with its depth (63), and
/// `_surface_hologram_temp` (61), all RGBA8 at the picture's size.
pub(in crate::app::model_preview) struct H4Holograms {
    pub(super) size: [i32; 2],
    pub(super) color: glow::NativeTexture,
    depth: glow::NativeTexture,
    pub(super) framebuffer: glow::NativeFramebuffer,
    pub(super) combined: glow::NativeTexture,
    pub(super) combined_framebuffer: glow::NativeFramebuffer,
    /// The screen materials' bitmaps, and the post they were uploaded from.
    /// Holding the post keeps its address from going to a later preview's,
    /// so a new load always uploads its own.
    pub(super) uploaded: Option<(Arc<H4HologramPost>, HashMap<String, glow::NativeTexture>)>,
}

impl H4Holograms {
    pub(super) unsafe fn new(gl: &glow::Context, size: [i32; 2]) -> Option<Self> {
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

    pub(super) unsafe fn delete(self, gl: &glow::Context) {
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
    pub(super) coverage: [f32; 4],
    /// The scissor, `[x0, y0, x1, y1]`.
    pub(super) crop: [i32; 4],
    /// `ps_material_generic_parameters[1]`: `(u0, u1, t0, t1)`.
    pub(super) texcoords: [f32; 4],
    /// `GetHologramBloomIntensityScale`.
    pub(super) bloom_scale: f32,
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
pub(super) fn hologram_sphere(frame: &ModelGpuFrame) -> ([f32; 3], f32) {
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
pub(super) fn scaled_sphere(
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
    pub(super) screen: HologramScreen,
    pub(super) post: Arc<H4HologramPost>,
    /// The object whose functions the screen materials read
    /// (`g_hologramRenderContext`, the first biped's).
    pub(super) owner: u8,
    /// How far along the view the sphere's centre lies, for its place among
    /// the transparents (`QueueHologramApply`).
    pub(in crate::app::model_preview) distance: f32,
}

/// A hologram's material is drawn by `RenderHolograms` when its screen
/// materials were read.
pub(in crate::app::model_preview) fn draws_as_hologram(material: &Gen3Material) -> bool {
    material.h4().is_some_and(|submit| submit.is_hologram() && submit.post().is_some())
}

/// `alpha_blend_max` (22), the blend pass's state: an alpha blend whose alpha
/// keeps the larger [inferred from the name; MCC's state table not read].
pub(super) unsafe fn set_alpha_blend_max(gl: &glow::Context) {
    unsafe {
        gl.blend_equation_separate(glow::FUNC_ADD, glow::MAX);
        gl.blend_func_separate(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::ONE, glow::ONE);
    }
}
