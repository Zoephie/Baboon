//! VISR's outline and the motion suck over Halo 3's and ODST's finished
//! picture: their GLSL and their draws, which the gen3 pipeline's
//! `before_screen_shaders` and `after_blur` hooks run.

use eframe::glow::{self, HasContext as _};

use super::super::common::view::ce_flare_view;
use super::super::frame::ModelGpuFrame;
use super::super::gen3::effects::screen::Gen3ScreenSettings;
use super::super::gen3::pipeline::{PictureCopies, view_projection};
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::state::Gen3State;
use super::super::gl::screen::screen_program;
use super::super::gl::texture::upload_image_linear;
use super::render::H3Game;
use super::vision::{GAME_FAR, GAME_NEAR, H3MotionBlur, H3Vision, inverted};

/// `displacement_motion_blur`'s `default_ps`, with the previous frame's view
/// being this one's with the world moved by `moved`. Every fragment here has
/// the alpha the shader's mask looks for.
const SUCK_FRAGMENT: &str = include_str!("shaders/suck.frag");

/// `hud_camera_nightvision`'s `default_ps`, its narrow filter. The game's depth
/// runs far to near, so the edges are of `game_depth`, not of this one.
const VISION_FRAGMENT: &str = include_str!("shaders/vision.frag");

impl H3Game {
    /// Whether the frame draws VISR or the suck over its picture.
    pub(super) fn draws_picture_effects(
        frame: &ModelGpuFrame,
        screen: &Gen3ScreenSettings,
    ) -> bool {
        let vision = frame.preview.h3.vision.is_some() && screen.vision() > 0.0;
        let suck = frame.preview.h3.motion_blur.is_some() && screen.suck().is_some();
        vision || suck
    }

    /// `distortion_apply` comes before everything else over the picture.
    pub(super) unsafe fn draw_picture_suck(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        screen: &Gen3ScreenSettings,
        copies: &PictureCopies,
    ) {
        let Some((blur, moved)) = cx.frame.preview.h3.motion_blur.zip(screen.suck()) else {
            return;
        };
        unsafe {
            copies.copy_depth(cx.gl);
            copies.screen_copy(cx.gl, true);
            self.draw_suck(
                cx.gl,
                cx.state,
                cx.frame,
                &blur,
                moved,
                copies.behind,
                copies.depth_copy,
            );
        }
    }

    /// VISR, after the blur.
    pub(super) unsafe fn draw_picture_vision(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        screen: &Gen3ScreenSettings,
        copies: &PictureCopies,
    ) {
        let Some(vision) = cx
            .frame
            .preview
            .h3
            .vision
            .clone()
            .filter(|_| screen.vision() > 0.0)
        else {
            return;
        };
        unsafe {
            copies.copy_depth(cx.gl);
            let on_for = self
                .slots
                .of(cx.slot, cx.core.slot_uploads(cx.slot))
                .vision_on_for(cx.frame.time);
            self.draw_vision(
                cx.gl,
                cx.state,
                cx.frame,
                &vision,
                screen.vision(),
                on_for,
                copies.depth_copy,
            );
        }
    }

    /// `c_view::distortion_apply`'s motion blur, for what moved the view and not
    /// for the preview's own camera.
    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_suck(
        &mut self,
        gl: &glow::Context,
        state: &mut Gen3State,
        frame: &ModelGpuFrame,
        blur: &H3MotionBlur,
        moved: [f32; 3],
        picture: glow::NativeTexture,
        depth: glow::NativeTexture,
    ) {
        let matrix = view_projection(&frame.camera);
        let Some(to_world) = inverted(&matrix) else {
            return;
        };
        unsafe {
            if self.suck_program.is_none() {
                self.suck_program = screen_program(gl, SUCK_FRAGMENT);
            }
            let Some(program) = self.suck_program else {
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
            gl.bind_vertex_array(Some(state.empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    /// `c_hud_camera_view`: over the frame as it will show, not over its light.
    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_vision(
        &mut self,
        gl: &glow::Context,
        state: &mut Gen3State,
        frame: &ModelGpuFrame,
        vision: &H3Vision,
        amount: f32,
        on_for: f32,
        depth: glow::NativeTexture,
    ) {
        let constants = vision.constants(amount, on_for);
        let matrix = view_projection(&frame.camera);
        let Some(to_world) = inverted(&matrix) else {
            return;
        };
        unsafe {
            if self.vision_program.is_none() {
                self.vision_program = screen_program(gl, VISION_FRAGMENT);
            }
            if self.vision_mask.is_none() {
                let Some((image, texels, mips)) = vision.mask() else {
                    return;
                };
                self.vision_mask = upload_image_linear(gl, image, *texels, mips, None);
            }
            let (Some(program), Some(mask)) = (self.vision_program, self.vision_mask) else {
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
            gl.uniform_2_f32(spot("planes").as_ref(), GAME_NEAR, GAME_FAR);
            gl.uniform_1_f32(spot("falloff").as_ref(), constants.falloff);
            gl.uniform_3_f32_slice(spot("ping").as_ref(), &constants.ping);
            gl.uniform_4_f32_slice(spot("default_color").as_ref(), &constants.color);
            gl.uniform_4_f32_slice(spot("ping_color").as_ref(), &constants.ping_color);
            // `_alpha_blend_multiply_add`, which is premultiplied.
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(state.empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::gen3::render::Gen3Game;
    use super::super::super::reach::render::ReachGame;
    use super::*;
    use crate::app::model_preview::{
        ModelPreviewState, RenderModelPreview, frame::build_frame, model_preview_data,
    };

    fn frame_with(motion_blur: Option<H3MotionBlur>) -> ModelGpuFrame {
        let mut preview = RenderModelPreview {
            game: Some(crate::core::game::GameId::Halo3Odst),
            ..Default::default()
        };
        preview.h3.motion_blur = motion_blur;
        let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
        let rect = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(64.0, 64.0),
        );
        build_frame(
            &data,
            &ModelPreviewState::default(),
            rect,
            0.0,
            [0.0; 3],
            &[],
        )
    }

    /// The gen3 pipeline finishes a picture with ODST's motion suck only
    /// through Halo 3's hooks, and only when the object brings one and the
    /// screen effects move the view; no other gen3 game's hooks draw it.
    #[test]
    fn only_halo_3s_hooks_draw_the_motion_suck_over_the_picture() {
        let blur = H3MotionBlur {
            most_pixels: 4.0,
            step: 0.5,
            center_falloff: [1.0, 1.0],
        };
        let (with, without) = (frame_with(Some(blur)), frame_with(None));
        let moving = Gen3ScreenSettings::seeing(0.0, [0.0, 0.0, 1.0]);
        let still = Gen3ScreenSettings::default();
        let h3 = H3Game::default();
        assert!(Gen3Game::picture_effects(&h3, &with, &moving));
        assert!(
            !Gen3Game::picture_effects(&h3, &with, &still),
            "the view does not move"
        );
        assert!(
            !Gen3Game::picture_effects(&h3, &without, &moving),
            "the object brings no motion blur"
        );
        assert!(
            !Gen3Game::picture_effects(&ReachGame::default(), &with, &moving),
            "Reach draws none"
        );
    }
}
