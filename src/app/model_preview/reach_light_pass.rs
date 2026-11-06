//! Reach's screen-space lights on the lit frame (`c_lights_view::render`):
//! cheap lights, then each screen-space `light`, added through its
//! `light_apply` explicit shader. The inline lights go to the static pass as
//! `simple_lights` instead. See `docs/reach-preview-research/lights.md`.

use super::reach_lighting::{REACH_EXPOSURE, REACH_HDR_DIVISOR, reach_frame_value};
use super::reach::lights::{CHEAP_ALBEDO_BLEND, ReachLightMethod, ReachLitLight, inline_rows};
use super::*;
use gen3::shaders::H3Value;

type Matrix = [[f32; 4]; 4];

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|row| std::array::from_fn(|column| (0..4).map(|at| a[row][at] * b[at][column]).sum()))
}

/// Gauss-Jordan with partial pivoting; none for a singular matrix.
fn inverse(matrix: &Matrix) -> Option<Matrix> {
    let mut left = *matrix;
    let mut right: Matrix = std::array::from_fn(|row| std::array::from_fn(|column| f32::from(u8::from(row == column))));
    for column in 0..4 {
        let pivot = (column..4).max_by(|a, b| left[*a][column].abs().total_cmp(&left[*b][column].abs()))?;
        if left[pivot][column].abs() < 1e-12 {
            return None;
        }
        left.swap(column, pivot);
        right.swap(column, pivot);
        let scale = 1.0 / left[column][column];
        for at in 0..4 {
            left[column][at] *= scale;
            right[column][at] *= scale;
        }
        for row in (0..4).filter(|row| *row != column) {
            let factor = left[row][column];
            for at in 0..4 {
                left[row][at] -= factor * left[column][at];
                right[row][at] -= factor * right[column][at];
            }
        }
    }
    Some(right)
}

/// `screen_to_relative_world`: a pixel and its stored depth to the world, less
/// `origin`, as the game's pixel-to-world matrix times a translation. Built
/// from the same projection the albedo pass wrote depth with; rows of the
/// matrix that takes `(x, y, depth, 1)` to homogeneous world.
pub(super) fn screen_to_relative_world(view_projection: &[f32; 16], viewport: [i32; 2], origin: [f32; 3]) -> Option<Matrix> {
    let clip: Matrix = std::array::from_fn(|row| std::array::from_fn(|column| view_projection[column * 4 + row]));
    let [width, height] = viewport.map(|side| side.max(1) as f32);
    let pixel_to_ndc: Matrix = [
        [2.0 / width, 0.0, 0.0, -1.0],
        [0.0, 2.0 / height, 0.0, -1.0],
        [0.0, 0.0, 2.0, -1.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let [x, y, z] = origin;
    let translate: Matrix = [[1.0, 0.0, 0.0, -x], [0.0, 1.0, 0.0, -y], [0.0, 0.0, 1.0, -z], [0.0, 0.0, 0.0, 1.0]];
    Some(multiply(&translate, &multiply(&inverse(&clip)?, &pixel_to_ndc)))
}

/// `render_screenspace_light`'s near fade: out within `near_fading` of the
/// camera, full at twice it.
fn near_fade(light: &ReachLitLight, eye: [f32; 3]) -> f32 {
    if light.near_fading <= 0.0 {
        return 1.0;
    }
    let distance = (0..3).map(|axis| (light.position[axis] - eye[axis]).powi(2)).sum::<f32>().sqrt();
    (distance / light.near_fading - 1.0).clamp(0.0, 1.0)
}

impl ModelGlRenderer {
    /// Every Reach light of the frame, lit: its attached lights by age since
    /// their scale came on, and its cheap lights by render time.
    pub(super) fn reach_frame_lights(&mut self, frame: &ModelGpuFrame, slot: usize) -> Vec<ReachLitLight> {
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let mut lit = Vec::new();
        let lights = frame.preview.reach_lights.len();
        let shown = |index: usize| frame.reach_lights_shown.get(index).copied().unwrap_or(true);
        for (index, light) in frame.preview.reach_cheap_lights.iter().enumerate() {
            if !shown(lights + index) {
                continue;
            }
            let values = frame.h2_values_of_object(light.object);
            let colors = frame.change_colors_of_object(light.object);
            lit.extend(light.lit(values, &colors, bones, frame.time));
        }
        for (index, light) in frame.preview.reach_lights.iter().enumerate() {
            if !shown(index) {
                continue;
            }
            let values = frame.h2_values_of_object(light.object);
            let colors = frame.change_colors_of_object(light.object);
            let onset = self.slots[slot].h3_onsets.of(light.object, light.scale_name(), values, frame.time);
            lit.extend(light.lit(values, &colors, bones, frame.time - onset));
        }
        lit
    }

    /// The inline lights, as the static pass reads them.
    pub(super) fn set_reach_inline_lights(&mut self, lit: &[ReachLitLight]) {
        self.reach_inline = inline_rows(lit);
    }

    /// `render_cheap_screenspace_lights_from_draw_list`, then the screen-space
    /// `light`s, into `accumulation_target`. Its depth is copied first, for the
    /// pass to read.
    pub(super) unsafe fn draw_reach_screen_lights(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        lit: &[ReachLitLight],
        accumulation_target: glow::NativeFramebuffer,
    ) {
        let screen: Vec<&ReachLitLight> =
            lit.iter().filter(|light| light.method == ReachLightMethod::Screenspace).collect();
        let Some(targets) = self.h3_targets.as_ref() else {
            return;
        };
        if screen.is_empty() {
            return;
        }
        let (size, albedo, normal, depth_copy) = (targets.size, targets.albedo, targets.normal, targets.depth_copy);
        let eye = h2_eye_position(&frame.camera);
        let view_projection = h3_draw::view_projection(&frame.camera);
        let depth_copy_framebuffer = targets.depth_copy_framebuffer;
        unsafe {
            self.begin_screen_lights(gl, size, accumulation_target, depth_copy_framebuffer);
            for light in screen {
                let combo = usize::from(light.frustum) + 2 * usize::from(light.specular);
                let Some(pass) = frame.preview.reach_light_passes.get(combo).cloned().flatten() else {
                    continue;
                };
                let fade = near_fade(light, eye);
                let Some(matrix) = screen_to_relative_world(&view_projection, size, light.origin) else {
                    continue;
                };
                if fade <= 1e-4 {
                    continue;
                }
                let linked = self.with_h3_program(gl, &pass, |this, program| {
                    gl.use_program(Some(program.program));
                    // Tag lights come down by the HDR target's stops; cheap
                    // lights only by exposure (`render_cheap_screenspace_lights`).
                    let exposure = if light.cheap { REACH_EXPOSURE } else { REACH_EXPOSURE / REACH_HDR_DIVISOR };
                    let [r, g, b] = light.color.map(|channel| channel * exposure * fade);
                    let to_light: [f32; 3] = std::array::from_fn(|axis| light.origin[axis] - eye[axis]);
                    for (name, ty) in &pass.uniforms {
                        let location = program.location(gl, name);
                        if location.is_none() {
                            continue;
                        }
                        match name.as_str() {
                            "screen_to_relative_world" => {
                                gl.uniform_matrix_4_f32_slice(location.as_ref(), false, matrix.as_flattened());
                            }
                            "light_rotation" => {
                                gl.uniform_matrix_3_f32_slice(location.as_ref(), true, light.rotation.as_flattened());
                            }
                            "light_colour_falloff_power" => {
                                gl.uniform_4_f32(location.as_ref(), r, g, b, light.falloff_power)
                            }
                            "light_attenuation" => gl.uniform_4_f32_slice(location.as_ref(), &light.attenuation),
                            "camera_to_light" => {
                                gl.uniform_4_f32(location.as_ref(), to_light[0], to_light[1], to_light[2], 0.0)
                            }
                            "material_coeff" => gl.uniform_4_f32(location.as_ref(), CHEAP_ALBEDO_BLEND, 0.0, 0.0, 0.0),
                            other => {
                                if let Some(value) = reach_frame_value(frame, other, this.reach_stops) {
                                    h3_draw::set_uniform_value(gl, location.as_ref(), ty, &value);
                                }
                            }
                        }
                    }
                    Self::bind_screen_light_textures(gl, &pass, program, |name| match name {
                        "GlobalTexture_depth_sampler" => Some(Some(depth_copy)),
                        "GlobalTexture_albedo_sampler" => Some(Some(albedo)),
                        "GlobalTexture_normal_sampler" => Some(Some(normal)),
                        _ => None,
                    });
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                });
                if linked.is_none() {
                    continue;
                }
            }
            Self::end_screen_lights(gl);
        }
    }

    /// `simple_lights` and `simple_light_count` for a Reach static pass.
    pub(super) fn reach_simple_light(&self, name: &str) -> Option<H3Value> {
        match name {
            "simple_light_count" => Some(H3Value::Integer(self.reach_inline.len() as i32)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matrix_times_its_inverse_is_the_identity() {
        let matrix: Matrix = [[2.0, 0.0, 1.0, 3.0], [0.0, 1.0, 0.0, -2.0], [1.0, 0.0, 3.0, 0.5], [0.0, 0.25, 0.0, 1.0]];
        let product = multiply(&matrix, &inverse(&matrix).unwrap());
        for (row, values) in product.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                let identity = f32::from(u8::from(row == column));
                assert!((value - identity).abs() < 1e-5, "{product:?}");
            }
        }
        assert!(inverse(&[[0.0; 4]; 4]).is_none());
    }

    #[test]
    fn a_pixel_and_its_depth_land_back_where_the_projection_put_them() {
        // A projection that halves x, keeps y, and puts depth in z with w 1.
        let projection: [f32; 16] = [0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, 1.0];
        let world = [0.8, -0.4, 2.0];
        let ndc = [world[0] * 0.5, world[1], world[2] * 0.1];
        let viewport = [200, 100];
        let pixel = [(ndc[0] + 1.0) * 0.5 * 200.0, (ndc[1] + 1.0) * 0.5 * 100.0, (ndc[2] + 1.0) * 0.5];
        let origin = [0.25, 0.5, 1.0];
        let rows = screen_to_relative_world(&projection, viewport, origin).unwrap();
        let point = [pixel[0], pixel[1], pixel[2], 1.0];
        let out: [f32; 4] = std::array::from_fn(|row| (0..4).map(|at| rows[row][at] * point[at]).sum());
        for axis in 0..3 {
            assert!((out[axis] / out[3] - (world[axis] - origin[axis])).abs() < 1e-4, "{out:?}");
        }
    }

    #[test]
    fn a_near_fading_light_is_out_up_close_and_full_at_twice_its_distance() {
        let mut light = ReachLitLight {
            method: ReachLightMethod::Screenspace,
            specular: false,
            frustum: false,
            origin: [0.0; 3],
            position: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            left: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            color: [1.0; 3],
            falloff_power: 5.0,
            attenuation: [1.0; 4],
            rotation: [[0.0; 3]; 3],
            range: [1.0, 0.0],
            cheap: false,
            near_fading: 2.0,
        };
        assert_eq!(near_fade(&light, [1.0, 0.0, 0.0]), 0.0);
        assert_eq!(near_fade(&light, [3.0, 0.0, 0.0]), 0.5);
        assert_eq!(near_fade(&light, [5.0, 0.0, 0.0]), 1.0);
        light.near_fading = 0.0;
        assert_eq!(near_fade(&light, [0.1, 0.0, 0.0]), 1.0);
    }
}
