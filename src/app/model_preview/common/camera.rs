//! The preview's camera: where the eye sits, how it projects, and the
//! view-space helpers every game's drawing places things by.

use eframe::egui::{self, Vec2};

use super::super::*;

#[derive(Clone, Copy)]
pub(in crate::app::model_preview) struct ModelGpuCamera {
    pub(in crate::app::model_preview) center: [f32; 3],
    pub(in crate::app::model_preview) scale: f32,
    pub(in crate::app::model_preview) yaw: f32,
    pub(in crate::app::model_preview) pitch: f32,
    pub(in crate::app::model_preview) clip_scale: [f32; 2],
    pub(in crate::app::model_preview) depth_scale: f32,
    /// Linear remap `z = t · x + y` (with `t = view.y · depth_scale`) that
    /// spreads the extended render distance across NDC depth; see
    /// `gpu_uniforms`.
    pub(in crate::app::model_preview) depth_map: [f32; 2],
    /// `1 / eye` in scaled view units: `w = 1 + view.y · this`. 0.0 is the
    /// orthographic view.
    pub(in crate::app::model_preview) perspective: f32,
}

/// Perspective's vertical field of view (across the viewport's shorter side),
/// fixed so zooming moves the eye rather than narrowing the lens.
///
/// The eye sits on the view axis at the distance where this lens frames the
/// focus plane exactly as the orthographic view does at the same zoom, so
/// `w = 1 + view.y / eye` is 1 at the orbit point: toggling the projection
/// never jumps, and pan and zoom-to-cursor keep their orthographic math
/// there. The fit spans `1.1 · radius` each side of the focus in *scaled*
/// view units at every zoom, so in those units `eye = 1.1 · radius /
/// tan(fov / 2)` is constant — and in world units it shrinks as `1 / scale`.
/// That is the dolly: zooming in walks the eye toward the orbit point and the
/// perspective deepens, where the old fixed eye at two radii only magnified
/// the picture and flattened toward orthographic the closer one looked.
pub(in crate::app::model_preview) const PERSPECTIVE_FOV_Y: f32 = 60.0 * std::f32::consts::PI / 180.0;

/// The perspective near plane, as a fraction of the eye's distance to the
/// orbit point. Small, because a dolly brings the eye right up to (and
/// inside) geometry, and anything nearer than this is clipped away — but not
/// smaller: depth precision falls with the near plane, and at the deepest
/// zoom on a level 1% left distant walls fighting.
pub(in crate::app::model_preview) const PERSPECTIVE_NEAR_FRACTION: f32 = 0.05;

/// How many depth windows of render distance the clip range covers, beyond
/// the one the framing math is built around. Depth clipping is remapped —
/// not the eye — so a larger margin costs only depth-buffer precision and
/// never flattens the perspective. See `depth_map` in `gpu_uniforms`.
pub(in crate::app::model_preview) const PREVIEW_RENDER_DISTANCE: f32 = 8.0;

/// Over the viewer's shoulder, so orbiting never leaves the near side unlit.
/// The preview's own, in view axes: right, forward and up.
pub(in crate::app::model_preview) fn camera_light(yaw: f32, pitch: f32) -> [f32; 3] {
    const LIGHT: [f32; 3] = [-0.4, -0.7, 0.6];
    let length = LIGHT.iter().map(|value| value * value).sum::<f32>().sqrt();
    unrotate_view_vector(yaw, pitch, LIGHT.map(|value| value / length))
}

/// The eye in model space, which the pass stages measure Fresnel from. An
/// orthographic view has no eye, so one far away along the view axis stands in.
pub(in crate::app::model_preview) fn eye_position(camera: &ModelGpuCamera) -> [f32; 3] {
    let distance = if camera.perspective > 0.0 { 1.0 / camera.perspective } else { 1.0e5 };
    let eye = unrotate_view_vector(camera.yaw, camera.pitch, [0.0, -distance, 0.0]);
    std::array::from_fn(|axis| camera.center[axis] + eye[axis] / camera.scale.max(1e-6))
}

/// Polygon offset units in one whole of window depth, on a 24 bit buffer.
pub(in crate::app::model_preview) const DEPTH_UNITS: f32 = (1 << 24) as f32;

/// `u_depth` for [`glsl_view_depth`]: the projection's depth terms, then
/// the preview's scale, which brings its units back to the world's.
pub(in crate::app::model_preview) fn scene_depth_terms(camera: &ModelGpuCamera) -> [f32; 4] {
    [camera.depth_scale * camera.depth_map[0], camera.depth_map[1], camera.perspective, camera.scale]
}

pub(in crate::app::model_preview) struct ProjectedPoint {
    pub(in crate::app::model_preview) pos: egui::Pos2,
    /// In front of the perspective near plane; always true orthographically.
    pub(in crate::app::model_preview) in_front: bool,
}

/// The camera's zoom bounds. Wide on purpose: 5× was plenty for a vehicle but
/// nothing on a structure BSP, where inspecting a doorway on a whole level
/// needs a couple hundred times the fitted view.
/// How far the view pulls back: half its shorter side, in world units. The
/// same for every model, whose own size only sets where zoom 1× is.
pub(in crate::app::model_preview) const WIDEST_PREVIEW_VIEW: f32 = 10_000.0;

pub(in crate::app) const MAX_PREVIEW_SCALE: f32 = 500.0;

/// A preview's bounds center and bounding-sphere radius — the fit every
/// camera computation derives from.
pub(in crate::app) fn min_preview_scale(preview: &RenderModelPreview) -> f32 {
    let (_, radius) = preview_center_radius(preview);
    (radius * 1.1 / WIDEST_PREVIEW_VIEW).min(1.0)
}

pub(in crate::app::model_preview) fn preview_center_radius(preview: &RenderModelPreview) -> ([f32; 3], f32) {
    let (min, max) = (preview.bounds_min, preview.bounds_max);
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let extent = [
        (max[0] - min[0]).abs(),
        (max[1] - min[1]).abs(),
        (max[2] - min[2]).abs(),
    ];
    let radius = ((extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt()
        * 0.5)
        .max(0.001);
    (center, radius)
}

/// World → view: yaw about Z, then pitch about X — the one rotation this
/// preview uses, mirrored in the vertex shader's `rotate_view`.
pub(in crate::app::model_preview) fn rotate_view_vector(yaw: f32, pitch: f32, vector: [f32; 3]) -> [f32; 3] {
    let (sy, cy) = yaw.sin_cos();
    let x = vector[0] * cy - vector[1] * sy;
    let y = vector[0] * sy + vector[1] * cy;
    let (sp, cp) = pitch.sin_cos();
    [x, y * cp - vector[2] * sp, y * sp + vector[2] * cp]
}

/// View → world: the exact inverse of [`rotate_view_vector`] (un-pitch, then
/// un-yaw). What turns a screen-space pan or zoom-to-cursor offset back into
/// the world-space focus move it stands for.
pub(in crate::app::model_preview) fn unrotate_view_vector(yaw: f32, pitch: f32, vector: [f32; 3]) -> [f32; 3] {
    let (sp, cp) = pitch.sin_cos();
    let y = vector[1] * cp + vector[2] * sp;
    let z = -vector[1] * sp + vector[2] * cp;
    let (sy, cy) = yaw.sin_cos();
    [vector[0] * cy + y * sy, -vector[0] * sy + y * cy, z]
}

pub(in crate::app::model_preview) struct PreviewCamera {
    pub(in crate::app::model_preview) rect: egui::Rect,
    /// Orbit point: bounds center plus the user's world-space focus offset.
    pub(in crate::app::model_preview) center: [f32; 3],
    /// Fit radius — deliberately the *bounds* radius, unaffected by focus, so
    /// the zoom slider keeps one meaning wherever the user pans.
    pub(in crate::app::model_preview) radius: f32,
    /// Depth-normalization radius: the bounds radius grown by the focus
    /// offset, so panned-to geometry can never fall outside the depth range.
    pub(in crate::app::model_preview) depth_radius: f32,
    pub(in crate::app::model_preview) yaw: f32,
    pub(in crate::app::model_preview) pitch: f32,
    pub(in crate::app::model_preview) scale: f32,
    pub(in crate::app::model_preview) perspective: bool,
}

impl PreviewCamera {
    pub(in crate::app::model_preview) fn new(data: &ModelPreviewData, state: &ModelPreviewState, rect: egui::Rect) -> Self {
        let (bounds_center, radius) = preview_center_radius(&data.preview);
        let center = [
            bounds_center[0] + state.focus[0],
            bounds_center[1] + state.focus[1],
            bounds_center[2] + state.focus[2],
        ];
        let focus_length = (state.focus[0] * state.focus[0]
            + state.focus[1] * state.focus[1]
            + state.focus[2] * state.focus[2])
            .sqrt();
        // An animation can carry the model well outside its bind-pose bounds
        // (boarding and assassination clips travel several world units), and
        // geometry past the depth window vanishes wholesale — sliced away
        // plane-first as the pose travels. Grow the window by the pose's own
        // reach; the cost is depth precision this preview has to spare.
        let reach = state
            .animation
            .pose
            .as_deref()
            .map(|pose| pose.reach)
            .unwrap_or(0.0);
        let scale = state.scale.clamp(min_preview_scale(&data.preview), MAX_PREVIEW_SCALE);
        Self {
            rect,
            center,
            radius,
            // What the view spans must draw however small the model is.
            depth_radius: (radius + focus_length + reach).max(radius * 1.1 / scale),
            yaw: state.yaw,
            pitch: state.pitch,
            scale,
            perspective: state.perspective,
        }
    }

    pub(in crate::app::model_preview) fn project(&self, point: [f32; 3]) -> ProjectedPoint {
        let x = (point[0] - self.center[0]) * self.scale;
        let y = (point[1] - self.center[1]) * self.scale;
        let z = (point[2] - self.center[2]) * self.scale;
        let rotated = self.rotate_vector([x, y, z]);
        let fit = self.rect.width().min(self.rect.height()) / (self.radius * 2.2).max(0.001);
        // The same divide the vertex shader applies, so markers stay glued to
        // their geometry in either projection.
        let w = 1.0 + rotated[1] * self.inverse_eye();
        // The GPU clips at the near plane; an overlay point past it is behind
        // (or level with) the eye and has no place on screen.
        let in_front = w >= PERSPECTIVE_NEAR_FRACTION;
        let w = w.max(PERSPECTIVE_NEAR_FRACTION);
        let screen = self.rect.center() + Vec2::new(rotated[0] * fit / w, -rotated[2] * fit / w);
        ProjectedPoint {
            pos: screen,
            in_front,
        }
    }

    /// Eye distance from the orbit point in scaled view units; see
    /// [`PERSPECTIVE_FOV_Y`].
    pub(in crate::app::model_preview) fn eye_distance(&self) -> f32 {
        (self.radius * 1.1).max(0.001) / (PERSPECTIVE_FOV_Y * 0.5).tan()
    }

    /// `1 / eye` for perspective, 0 for the orthographic view.
    pub(in crate::app::model_preview) fn inverse_eye(&self) -> f32 {
        if self.perspective {
            1.0 / self.eye_distance()
        } else {
            0.0
        }
    }

    pub(in crate::app::model_preview) fn gpu_uniforms(&self) -> ModelGpuCamera {
        let width = self.rect.width().max(1.0);
        let height = self.rect.height().max(1.0);
        let fit = width.min(height) / (self.radius * 2.2).max(0.001);
        ModelGpuCamera {
            center: self.center,
            scale: self.scale,
            yaw: self.yaw,
            pitch: self.pitch,
            clip_scale: [2.0 * fit / width, 2.0 * fit / height],
            // Orthographic projection: smaller view-space Y is nearer. The
            // focus-grown radius safely maps all depths inside NDC even when
            // the orbit point sits far from the bounds center.
            depth_scale: 1.0 / (self.depth_radius * self.scale * 1.05).max(0.001),
            // Depth remap `z = t·x + y` with `t = view.y · depth_scale`;
            // see `depth_map_coefficients`.
            depth_map: depth_map_coefficients(
                self.perspective.then(|| {
                    self.eye_distance() / (self.depth_radius * self.scale * 1.05).max(0.001)
                }),
            ),
            perspective: self.inverse_eye(),
        }
    }

    pub(in crate::app::model_preview) fn rotate_vector(&self, vector: [f32; 3]) -> [f32; 3] {
        rotate_view_vector(self.yaw, self.pitch, vector)
    }
}

/// The `[x, y]` of the shader's depth remap `z = t·x + y`, with `t = view.y ·
/// depth_scale` (documented at the `depth_map` field of `ModelGpuCamera`).
///
/// `t` is depth in units of the depth window, and the far plane sits at
/// `t = K` (`PREVIEW_RENDER_DISTANCE`) in both projections. Orthographic maps
/// `[-K, K]` straight onto NDC. Perspective takes `eye` — the eye's distance,
/// in the same units — and solves the line through `z(n) = -w(n)` and
/// `z(K) = w(K)`, with `w(t) = 1 + t / eye` and the near plane `n` just in
/// front of the eye. `z / w` is monotone in `t` between them, so the depth
/// test orders correctly however close the dolly brings the eye.
pub(in crate::app::model_preview) fn depth_map_coefficients(eye: Option<f32>) -> [f32; 2] {
    let k = PREVIEW_RENDER_DISTANCE;
    let Some(eye) = eye.filter(|eye| eye.is_finite() && *eye > 0.0) else {
        return [1.0 / k, 0.0];
    };
    let near = -eye * (1.0 - PERSPECTIVE_NEAR_FRACTION);
    let w = |t: f32| 1.0 + t / eye;
    let x = (w(k) + w(near)) / (k - near);
    [x, -w(near) - x * near]
}

/// The vertex shader's `clip_position`.
pub(in crate::app::model_preview) fn ce_flare_clip(camera: &ModelGpuCamera, point: [f32; 3]) -> [f32; 4] {
    let scaled = std::array::from_fn(|axis| (point[axis] - camera.center[axis]) * camera.scale);
    let view = rotate_view_vector(camera.yaw, camera.pitch, scaled);
    [
        view[0] * camera.clip_scale[0],
        view[2] * camera.clip_scale[1],
        view[1] * camera.depth_scale * camera.depth_map[0] + camera.depth_map[1],
        1.0 + view[1] * camera.perspective,
    ]
}

/// How many pixels wide plus high a screen-facing sprite of `half` extents
/// is at `center`: `lens_flare_project_billboard`'s half extents across the
/// viewport, as Reach's `lens_flare_render_internal` measures them.
pub(in crate::app::model_preview) fn ce_flare_pixels(camera: &ModelGpuCamera, viewport: [i32; 2], center: [f32; 3], half: [f32; 2]) -> f32 {
    let w = ce_flare_clip(camera, center)[3];
    if w <= 0.0 {
        return 0.0;
    }
    let across = half[0] * camera.scale * camera.clip_scale[0] / w * viewport[0] as f32;
    let down = half[1] * camera.scale * camera.clip_scale[1] / w * viewport[1] as f32;
    across + down
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_light_keeps_its_place_as_the_view_orbits() {
        let toward_eye = |yaw, pitch| unrotate_view_vector(yaw, pitch, [0.0, -1.0, 0.0]);
        let share = |yaw, pitch| {
            let (light, eye): ([f32; 3], [f32; 3]) = (camera_light(yaw, pitch), toward_eye(yaw, pitch));
            (0..3).map(|axis| light[axis] * eye[axis]).sum::<f32>()
        };
        for (yaw, pitch) in [(0.0, 0.0), (1.5, 0.4), (3.1, -0.6), (-2.0, 1.0)] {
            assert!((share(yaw, pitch) - share(0.0, 0.0)).abs() < 1e-5);
        }
        assert!(share(0.0, 0.0) > 0.5);
    }

    /// The extended render distance must clip exactly at its declared bounds
    /// in both projections, wherever the dolly has put the eye: near plane
    /// just in front of the eye, far plane at t = K, and post-divide depth
    /// monotone between them so the depth test orders correctly.
    #[test]
    fn depth_remap_clips_at_the_near_and_far_planes_for_any_eye() {
        let k = PREVIEW_RENDER_DISTANCE;

        // Orthographic: w = 1, so z = t·x + y must span NDC over t ∈ [-K, K].
        let [x, y] = depth_map_coefficients(None);
        assert!(((-k) * x + y - -1.0).abs() < 1e-5);
        assert!((k * x + y - 1.0).abs() < 1e-5);

        // Perspective, from the default framing's eye down to a dolly deep
        // inside a level.
        for eye in [1.9, 0.5, 0.02, 0.0005] {
            let [x, y] = depth_map_coefficients(Some(eye));
            let z = |t: f32| t * x + y;
            let w = |t: f32| 1.0 + t / eye;
            let near = -eye * (1.0 - PERSPECTIVE_NEAR_FRACTION);
            let tolerance = 1e-4 * w(k);
            assert!(
                (z(near) + w(near)).abs() < tolerance,
                "eye {eye}: near plane moved"
            );
            assert!(
                (z(k) - w(k)).abs() < tolerance,
                "eye {eye}: far plane short of K"
            );
            let samples = (0..=64)
                .map(|step| near + (k - near) * step as f32 / 64.0)
                .map(|t| z(t) / w(t))
                .collect::<Vec<_>>();
            // Strictly increasing wherever f32 can tell the samples apart;
            // at the deepest dolly the far samples round together, so ties
            // are allowed there but never a reversal.
            assert!(
                samples.windows(2).all(|pair| if eye >= 0.02 {
                    pair[1] > pair[0]
                } else {
                    pair[1] >= pair[0]
                }),
                "eye {eye}: post-divide depth not monotone"
            );
        }
    }

    #[test]
    fn a_models_size_does_not_change_how_far_the_view_pulls_back() {
        let widest = |half: f32| {
            let preview = RenderModelPreview {
                bounds_min: [-half; 3],
                bounds_max: [half; 3],
                ..Default::default()
            };
            let (_, radius) = preview_center_radius(&preview);
            (radius * 1.1 / min_preview_scale(&preview), radius * 1.1 / 0.02)
        };
        let (tiny, tiny_at_a_fixed_scale) = widest(0.01);
        let (level, level_at_a_fixed_scale) = widest(500.0);
        assert!((tiny - level).abs() < 1.0, "{tiny} {level}");
        assert!(tiny_at_a_fixed_scale * 1000.0 < level_at_a_fixed_scale);
    }

    fn camera(scale: f32, perspective: bool) -> PreviewCamera {
        PreviewCamera {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0)),
            center: [0.0; 3],
            radius: 1.0,
            depth_radius: 1.0,
            // Unrotated: view space is world space, the eye looks along +Y.
            yaw: 0.0,
            pitch: 0.0,
            scale,
            perspective,
        }
    }

    /// How far from the viewport center a point lands, in pixels.
    fn screen_offset(camera: &PreviewCamera, point: [f32; 3]) -> f32 {
        (camera.project(point).pos - camera.rect.center()).length()
    }

    /// On the orbit point's plane, perspective frames exactly like the
    /// orthographic view at every zoom, so toggling never jumps and the
    /// pan / zoom-to-cursor math holds in both.
    #[test]
    fn perspective_matches_orthographic_at_the_focus_plane() {
        for scale in [0.05, 1.0, 20.0, 400.0] {
            let point = [0.3 / scale, 0.0, 0.2 / scale];
            let flat = camera(scale, false).project(point).pos;
            let deep = camera(scale, true).project(point).pos;
            assert!(
                (flat - deep).length() < 1e-3,
                "scale {scale}: {flat:?} vs {deep:?}"
            );
        }
    }

    /// The reported defect: zooming in only magnified the picture, so the
    /// perspective stayed the same strength and up close looked flat. With
    /// the eye dollying in, a point the same world distance nearer the
    /// camera than the orbit point grows ever larger relative to one on the
    /// focus plane.
    #[test]
    fn zooming_in_dollies_the_eye_and_deepens_the_perspective() {
        let convergence = |scale: f32| {
            let camera = camera(scale, true);
            let offset = 0.1 / scale;
            let nearer = screen_offset(&camera, [offset, -0.2, 0.0]);
            let on_plane = screen_offset(&camera, [offset, 0.0, 0.0]);
            nearer / on_plane
        };
        let (wide, close) = (convergence(1.0), convergence(8.0));
        assert!(wide > 1.05, "no perspective at the default zoom: {wide}");
        assert!(
            close > wide * 1.5,
            "zoom did not deepen the perspective: {wide} → {close}"
        );

        // The lens itself does not change: at every zoom the eye sits where
        // a 60° field of view frames the focus plane, `1.1 · radius /
        // tan(30°)` out in scaled units — `1 / scale` of that in world units.
        for scale in [1.0, 8.0, 100.0] {
            let eye_world = camera(scale, true).eye_distance() / scale;
            let expected = 1.1 / (30.0f32.to_radians().tan() * scale);
            assert!((eye_world - expected).abs() < 1e-5 * expected.max(1.0));
        }
    }

    /// Overlays behind the eye are dropped rather than drawn stretched across
    /// the viewport; the orthographic view has no eye and keeps everything.
    #[test]
    fn overlay_points_behind_the_eye_are_not_drawn() {
        let scale = 10.0;
        let perspective = camera(scale, true);
        let eye_world = perspective.eye_distance() / scale;
        let in_front = [0.0, -eye_world * 0.5, 0.0];
        let behind = [0.0, -eye_world * 1.5, 0.0];
        assert!(perspective.project(in_front).in_front);
        assert!(!perspective.project(behind).in_front);
        assert!(camera(scale, false).project(behind).in_front);
    }

    /// The attribute offsets in `RenderCore::new` are hand-written against
    /// this layout, and nothing else checks them — a reordered or resized field
    /// would silently feed the shader the wrong bytes and show as a model that
    /// renders but looks wrong.
    /// A malformed shader disables the whole preview and reports it only on
    /// stderr, so these check the structure no GPU is here to check.
    /// The pan and zoom-to-cursor math turns screen deltas back into world
    /// moves through `unrotate_view_vector`; if it drifts from the forward
    /// rotation, panning smears diagonally and zoom-to-cursor orbits away
    /// from the pointer instead of onto it.
    #[test]
    fn unrotate_inverts_rotate_for_any_view_angles() {
        for (yaw, pitch) in [(0.0, 0.0), (-0.45, 0.25), (1.2, -1.4), (3.0, 0.9)] {
            for vector in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, -2.0, 5.0]] {
                let there = rotate_view_vector(yaw, pitch, vector);
                let back = unrotate_view_vector(yaw, pitch, there);
                for axis in 0..3 {
                    assert!(
                        (back[axis] - vector[axis]).abs() < 1e-5,
                        "round trip failed at yaw {yaw} pitch {pitch}: {vector:?} -> {back:?}"
                    );
                }
            }
        }
    }
}
