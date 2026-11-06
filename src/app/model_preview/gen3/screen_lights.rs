//! What Reach's and Halo 4's screen-space lights share around each light's
//! own constants: the scene's depth to read, the additive state they draw
//! under, and the scene textures each pass names.

use eframe::glow::{self, HasContext as _};
use super::super::renderer::*;
use super::shaders::H3Pass;
use super::pipeline::H3Program;

impl RenderCore {
    /// Ready the screen lights over `accumulation`, `size` of it: its depth
    /// copied for the passes to read, then no depth test or culling, adding
    /// one to one into its colour alone, through the empty vertex array.
    pub(in crate::app::model_preview) unsafe fn begin_screen_lights(
        &mut self,
        gl: &glow::Context,
        size: [i32; 2],
        accumulation: glow::NativeFramebuffer,
        depth_copy: glow::NativeFramebuffer,
    ) {
        unsafe {
            Self::copy_scene_depth(gl, size, accumulation, depth_copy);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(accumulation));
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);
            // `RenderSSLightInternal`'s colour write mask 7.
            gl.color_mask(true, true, true, false);
            let empty = self.h3_empty_array(gl);
            gl.bind_vertex_array(Some(empty));
        }
    }

    /// Bind each of `pass`'s textures that `texture` answers for to its own
    /// unit, in the pass's order; one it does not answer for keeps its unit
    /// and is left unset.
    pub(in crate::app::model_preview) unsafe fn bind_screen_light_textures(
        gl: &glow::Context,
        pass: &H3Pass,
        program: &H3Program,
        texture: impl Fn(&str) -> Option<Option<glow::NativeTexture>>,
    ) {
        for (unit, (name, _)) in pass.textures.iter().enumerate() {
            unsafe {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                let Some(bound) = texture(name) else {
                    continue;
                };
                gl.bind_texture(glow::TEXTURE_2D, bound);
                gl.uniform_1_i32(program.location(gl, name).as_ref(), unit as i32);
            }
        }
    }

    /// The scene's state again once the lights are drawn.
    pub(in crate::app::model_preview) unsafe fn end_screen_lights(gl: &glow::Context) {
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.enable(glow::DEPTH_TEST);
        }
    }
}

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
pub(in crate::app::model_preview) fn screen_to_relative_world(view_projection: &[f32; 16], viewport: [i32; 2], origin: [f32; 3]) -> Option<Matrix> {
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
}
