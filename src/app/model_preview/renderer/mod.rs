//! Depth-tested Glow rendering, markers, and preview geometry conversion.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::common::camera::*;
use super::gl::texture::*;
use super::gl::buffer::*;
use super::common::sprite_gl::*;
use super::gl::scope::PostScope;
use super::*;
use super::dx9::draw::*;
use super::ce::draw::*;
use eframe::glow::{self, HasContext as _};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

mod game;
pub(in crate::app::model_preview) use game::*;
mod core;
pub(in crate::app::model_preview) use core::*;
mod slot;
pub(in crate::app::model_preview) use slot::*;
mod program;
pub(in crate::app::model_preview) use program::*;
mod grid;
pub(in crate::app::model_preview) use grid::*;

thread_local! {
    static MODEL_GL_RENDERER: RefCell<Option<CachedModelGlRenderer>> = const { RefCell::new(None) };
}

struct CachedModelGlRenderer {
    context_id: usize,
    renderer: Result<ModelGlRenderer, String>,
    /// Geometry whose drawing panicked, with what the panic said: it is not
    /// drawn again, and the panel says why.
    failed: HashMap<u64, String>,
}

thread_local! {
    /// Models whose played Halo CE event effects still have something
    /// running, as the last frame drawn left them.
    pub(in crate::app::model_preview) static CE_EVENTS_RUNNING: RefCell<std::collections::HashSet<u64>> = RefCell::new(Default::default());
}

/// Whether a played event effect on `geometry_id` still needs frames.
pub(in crate::app::model_preview) fn ce_events_running(geometry_id: u64) -> bool {
    CE_EVENTS_RUNNING.with(|running| running.borrow().contains(&geometry_id))
}

/// Why drawing `geometry_id` stopped, if it panicked.
pub(super) fn model_gl_failure(geometry_id: u64) -> Option<String> {
    MODEL_GL_RENDERER.with(|cell| cell.borrow().as_ref()?.failed.get(&geometry_id).cloned())
}

pub(in crate::app::model_preview) fn paint_model_gl(
    info: egui::PaintCallbackInfo,
    painter: &eframe::egui_glow::Painter,
    frame: &ModelGpuFrame,
) {
    let gl = painter.gl();
    let viewport = info.viewport_in_pixels();
    let clip = info.clip_rect_in_pixels();
    let left = viewport.left_px.max(clip.left_px);
    let bottom = viewport.from_bottom_px.max(clip.from_bottom_px);
    let right = (viewport.left_px + viewport.width_px).min(clip.left_px + clip.width_px);
    let top =
        (viewport.from_bottom_px + viewport.height_px).min(clip.from_bottom_px + clip.height_px);
    if right <= left || top <= bottom {
        return;
    }

    unsafe {
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(left, bottom, right - left, top - bottom);
        gl.color_mask(true, true, true, true);
        gl.depth_mask(true);
        let [r, g, b] = frame.background;
        gl.clear_color(r, g, b, 1.0);
        gl.clear_depth_f32(1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
    }

    MODEL_GL_RENDERER.with(|cell| {
        let context_id = Arc::as_ptr(gl) as usize;
        let mut cached = cell.borrow_mut();
        if cached
            .as_ref()
            .is_none_or(|cached| cached.context_id != context_id)
        {
            let created = ModelGlRenderer::new(gl);
            if let Err(error) = &created {
                eprintln!("{error}");
            }
            *cached = Some(CachedModelGlRenderer {
                context_id,
                renderer: created,
                failed: HashMap::new(),
            });
        }
        let Some(CachedModelGlRenderer { renderer: Ok(renderer), failed, .. }) = cached.as_mut() else {
            return;
        };
        if failed.contains_key(&frame.geometry_id) {
            return;
        }
        // A panic part way through leaves the scene target bound; what the
        // frame drew into goes back, so the rest of the window still draws.
        let state = unsafe { PostScope::capture(gl) };
        let painted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { renderer.paint(gl, frame) }));
        if let Err(panic) = painted {
            unsafe {
                state.restore(gl);
                gl.use_program(None);
                gl.bind_vertex_array(None);
            }
            let message = crate::app::shell::worker::panic_detail(panic.as_ref()).to_owned();
            eprintln!("model preview: drawing crashed: {message}");
            failed.insert(frame.geometry_id, message);
        }
    });
}


/// The preview's renderer: what every game draws with ([`RenderCore`]),
/// and each game's own drawing ([`GameRender`]), which it hands the frame to.
pub(super) struct ModelGlRenderer {
    core: RenderCore,
    games: GameRenders,
}

impl ModelGlRenderer {
    pub(super) fn new(gl: &glow::Context) -> Result<Self, String> {
        Ok(Self { core: RenderCore::new(gl)?, games: GameRenders::default() })
    }

    /// Draw `frame`: its slot, the game's preparation, the scene target, the
    /// shared program and grid, the game's shading, the wireframe, what the
    /// game draws over it, and the scene put back.
    pub(super) unsafe fn paint(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        // Set again if this frame steps the effects and something runs on.
        CE_EVENTS_RUNNING.with(|running| running.borrow_mut().remove(&frame.geometry_id));
        unsafe {
            let Some(slot) = self.core.acquire_frame_slot(gl, frame) else {
                return;
            };
            let game = self.games.of(frame);
            game.prepare(&mut FrameCx { gl, frame, core: &mut self.core, slot });
            let scene = self.core.begin_scene(gl, frame);
            // The game's passes need the scene target's stencil and copies;
            // without one, the frame draws as with them off.
            if scene.is_none() && frame.game_shaders && self.core.shaders_supported {
                let plain = ModelGpuFrame { game_shaders: false, ..frame.clone() };
                self.paint(gl, &plain);
                return;
            }
            self.core.begin_drawing(gl, frame, slot);
            let game = self.games.of(frame);
            if frame.render_mode.draws_shading() {
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
                gl.uniform_1_f32(Some(&self.core.unlit), 0.0);
                game.shade(&mut FrameCx { gl, frame, core: &mut self.core, slot });
            }
            if frame.render_mode.draws_wireframe() {
                self.core.draw_wireframe(gl, frame, slot);
            }
            game.finish(&mut FrameCx { gl, frame, core: &mut self.core, slot });
            self.core.end_frame(gl, scene);
        }
    }

    #[cfg(test)]
    pub(super) fn for_each_simulation(&self, geometry_id: u64, mut visit: impl FnMut(&str, &dyn std::fmt::Debug)) {
        self.core.for_each_simulation(geometry_id, &mut visit);
        if let Some(slot) = self.core.slot_of(geometry_id) {
            self.games.each_simulation(slot, self.core.slot_uploads(slot), &mut visit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::harness::kit::kit_root;
    use crate::core::game::GameId;


    #[test]
    fn the_camera_light_keeps_its_place_as_the_view_orbits() {
        let toward_eye = |yaw, pitch| unrotate_view_vector(yaw, pitch, [0.0, -1.0, 0.0]);
        let share = |yaw, pitch| {
            let (light, eye): ([f32; 3], [f32; 3]) = (h2_camera_light(yaw, pitch), toward_eye(yaw, pitch));
            (0..3).map(|axis| light[axis] * eye[axis]).sum::<f32>()
        };
        for (yaw, pitch) in [(0.0, 0.0), (1.5, 0.4), (3.1, -0.6), (-2.0, 1.0)] {
            assert!((share(yaw, pitch) - share(0.0, 0.0)).abs() < 1e-5);
        }
        assert!(share(0.0, 0.0) > 0.5);
    }

    #[test]
    fn dense_indices_use_full_u32_offsets_and_counts() {
        let start = 70_002_u32;
        let count = 120_003_u32;
        assert_eq!(
            model_draw_range(start, count, 250_000),
            Some(((start * 4) as i32, count as i32))
        );
    }

    /// With `BLAM_TEST_HREK`: the marine's radio antenna is drawn only while
    /// its `trooper_radio` backpack is.
    #[test]
    fn kit_reach_antenna_shows_only_with_its_permutation() {
        let Some(root) = kit_root(GameId::HaloReach) else { return };
        let path = "objects\\characters\\marine\\marine";
        let data = super::super::harness::kit::kit_preview(
            &root,
            crate::core::game::GameId::HaloReach,
            path,
            "biped",
            b"bipd",
        );
        let shown = |permutation: &str| {
            let mut state = ModelPreviewState::default();
            let selection = ModelRegionSelection { enabled: true, permutation: permutation.to_owned() };
            state.region_selections.insert("flair_backpack".to_owned(), selection);
            let shown = common::features::FeatureShown::of(&data.preview, &state.region_selections);
            shown.list(common::features::paths::ANTENNAS).to_vec()
        };
        assert_eq!(shown("trooper_radio"), [true]);
        assert_eq!(shown("trooper"), [false]);
    }

    /// The depth window must cover an animation's travel: the reach bound is
    /// at least the largest per-frame chain length, and an empty pose adds
    /// nothing.
    #[test]
    fn pose_reach_covers_the_travelling_frame() {
        let frame = |x: f32| {
            vec![
                PreviewNodeTransform {
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    translation: [x, 0.0, 0.9],
                    scale: 1.0,
                },
                PreviewNodeTransform {
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    translation: [0.0, 0.0, 0.25],
                    scale: 1.0,
                },
            ]
        };
        let pose = PreviewAnimationPose::new(0, vec![frame(0.0), frame(3.0)]);
        let reach = pose.reach;
        let travelled = (3.0f32 * 3.0 + 0.9 * 0.9).sqrt() + 0.25;
        assert!(
            reach >= travelled - 1e-4,
            "reach {reach} misses the travelled frame {travelled}"
        );
        let empty = PreviewAnimationPose::new(0, Vec::new());
        assert_eq!(empty.reach, 0.0);
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

    #[test]
    fn draw_range_clamps_to_complete_valid_triangles() {
        assert_eq!(model_draw_range(6, 10, 14), Some((24, 6)));
        assert_eq!(model_draw_range(20, 3, 14), None);
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

pub(in crate::app) fn material_color(index: u16) -> Color32 {
    const COLORS: &[(u8, u8, u8)] = &[
        (132, 168, 188),
        (176, 166, 128),
        (142, 182, 150),
        (180, 136, 134),
        (150, 145, 190),
        (186, 154, 104),
        (126, 174, 176),
    ];
    let (r, g, b) = COLORS[index as usize % COLORS.len()];
    Color32::from_rgb(r, g, b)
}

/// Halo 2's, 3's and Reach's antennas, whose maps follow Halo CE's in the
/// slot's, each with whether it is Reach's; Reach's far ends are at `tips`.
/// Those on a permutation not shown are left out.
pub(in crate::app::model_preview) fn h2_antennas(
    frame: &ModelGpuFrame,
    textures: &[Option<glow::NativeTexture>],
    bones: Option<&[[f32; 4]]>,
    view: &common::view::ViewFrame,
    tips: &[Option<[f32; 3]>],
) -> Vec<(Vec<crate::app::model_preview::common::sprites::AntennaQuad>, glow::NativeTexture, bool)> {
    let preview = &frame.preview;
    preview
        .antennas
        .iter()
        .zip(textures)
        .enumerate()
        .filter(|(index, _)| frame.shown.is(common::features::paths::ANTENNAS, *index))
        .filter_map(|(index, (antenna, texture))| {
            let tip = tips.get(index).copied().flatten();
            Some((antenna.quads(bones, view, tip), (*texture)?, antenna.is_reach()))
        })
        .collect()
}

