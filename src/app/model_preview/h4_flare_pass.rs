//! Halo 4's lens flares on the lit frame (`lens_flares_render`, X360
//! 0x82A708D0, MCC 0x1408D1030): after the transparents, before the bloom,
//! each flare's occlusion patch counted against the depth, then its
//! reflections added through explicit shader 46. See
//! `docs/h4-preview-research/lights.md` §4.

use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};

use super::gen3::shaders::{H3Sampler, H3Value};
use super::h4::flares::{H4FlareDraw, H4FlareView};
use super::*;

/// The camera as the flares read it: a point's normalized device position,
/// and the projection's scale there that `LensFlareProjectBillboard` hands
/// the shader, so a size over the point's depth times it is the size on
/// screen.
fn project(
    camera: &ModelGpuCamera,
    eye: [f32; 3],
    forward: [f32; 3],
    point: [f32; 3],
) -> Option<([f32; 2], [f32; 2])> {
    let clip = ce_flare_clip(camera, point);
    let depth: f32 = (0..3)
        .map(|axis| (point[axis] - eye[axis]) * forward[axis])
        .sum();
    if clip[3] <= 1e-6 || depth <= 1e-6 {
        return None;
    }
    let focal = [0, 1].map(|axis| camera.scale * camera.clip_scale[axis] * depth / clip[3]);
    Some(([clip[0] / clip[3], clip[1] / clip[3]], focal))
}

/// Each flare's images, after Halo 3's and Reach's in the slot's.
pub(super) unsafe fn h4_flare_images(
    gl: &glow::Context,
    frame: &ModelGpuFrame,
) -> Vec<Vec<Option<glow::NativeTexture>>> {
    // `rasterizer_draw_screen_facing_lens_flare`: clamped, bilinear.
    let sampler = H3Sampler {
        address: [BitmapAddressMode::Clamp; 2],
        filter: BitmapFilterMode::Bilinear,
    };
    frame
        .preview
        .h4_flares
        .iter()
        .map(|at| {
            at.flare
                .images
                .iter()
                .map(|(image, texels, mips)| unsafe {
                    h3_draw::upload_h2_image_linear(gl, image, *texels, mips, Some(sampler))
                })
                .collect()
        })
        .collect()
}

impl ModelGlRenderer {
    /// `lens_flares_submit_occlusions` and `lens_flares_render` for the
    /// object's flares, into the bound light target, which holds the
    /// object's depth. A pane paints on demand, so each patch's count is
    /// waited for rather than read a frame late.
    pub(super) unsafe fn draw_h4_flares(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) {
        let Some(pass) = frame.preview.h4_flare_pass.clone() else {
            return;
        };
        if frame.preview.h4_flares.is_empty() {
            return;
        }
        unsafe {
            let Some(flares) = self
                .sprite_gl
                .get_or_insert_with(|| link_sprite_gl(gl, self.program))
                .take()
            else {
                return;
            };
            let camera = &frame.camera;
            let view = ce_flare_view(camera);
            let viewport = self
                .h3_targets
                .as_ref()
                .map_or([1, 1], |targets| targets.size)
                .map(|side| side as f32);
            let projector = |point: [f32; 3]| project(camera, view.eye, view.forward, point);
            let flare_view = H4FlareView {
                eye: view.eye,
                forward: view.forward,
                right: view.right,
                up: view.up,
                viewport,
                project: &projector,
            };
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            let shown = |index: usize| {
                frame
                    .h3_flares_shown
                    .get(frame.preview.h3_flares.len() + index)
                    .copied()
                    .unwrap_or(true)
            };
            // The patches: depth tested, nothing written.
            gl.use_program(Some(flares.program));
            gl.bind_vertex_array(Some(flares.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(flares.buffer));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, self.placeholder);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gl.disable(glow::CULL_FACE);
            gl.color_mask(false, false, false, false);
            let mut drawn: Vec<(usize, Vec<H4FlareDraw>)> = Vec::new();
            for (index, at) in frame.preview.h4_flares.iter().enumerate() {
                if !shown(index) {
                    continue;
                }
                let values = frame.function_values_of_object(at.object);
                let colors = frame.change_colors_of_object(at.object);
                let passed = at
                    .occlusion_patch(values, &colors, bones, &flare_view, frame.time)
                    .map(|patch| {
                        let [a, b, c, d] = patch.corners.map(|corner| {
                            let [x, y, z, w] = ce_flare_clip(camera, corner);
                            [x, y, z, w, 0.0, 0.0]
                        });
                        let strip = [a, d, b, c];
                        gl.buffer_data_u8_slice(
                            glow::ARRAY_BUFFER,
                            slice_bytes(&strip),
                            glow::STREAM_DRAW,
                        );
                        gl.begin_query(glow::SAMPLES_PASSED, flares.queries[0]);
                        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                        gl.end_query(glow::SAMPLES_PASSED);
                        // As `ce_flare_occlusion`: the core entry point, no
                        // query buffer bound, so the offset is the destination.
                        let mut samples = 0_u64;
                        let result = &raw mut samples as usize;
                        gl.get_query_parameter_u64_with_offset(
                            flares.queries[0],
                            glow::QUERY_RESULT,
                            result,
                        );
                        (samples as f32, patch.expected)
                    });
                let visibility = at.visibility(passed);
                let draws = at.draws(values, &colors, bones, &flare_view, frame.time, visibility);
                if !draws.is_empty() {
                    drawn.push((index, draws));
                }
            }
            self.sprite_gl = Some(Some(flares));
            gl.color_mask(true, true, true, false);
            gl.disable(glow::DEPTH_TEST);
            if drawn.is_empty() {
                return;
            }
            let linked = self.with_h3_program(gl, &pass, |this, program| {
                // `draw_textured_screen_quad`'s corners and texture coordinates.
                let (vertex_array, buffer) = match this.h4_flare_geometry {
                    Some(geometry) => geometry,
                    None => {
                        let (Ok(vertex_array), Ok(buffer)) =
                            (gl.create_vertex_array(), gl.create_buffer())
                        else {
                            return;
                        };
                        this.h4_flare_geometry = Some((vertex_array, buffer));
                        (vertex_array, buffer)
                    }
                };
                gl.bind_vertex_array(Some(vertex_array));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
                for (attribute, offset) in [("a_POSITION", 0), ("a_TEXCOORD0", 8)] {
                    if let Some(location) = gl.get_attrib_location(program.program, attribute) {
                        gl.enable_vertex_attrib_array(location);
                        gl.vertex_attrib_pointer_f32(location, 2, glow::FLOAT, false, 16, offset);
                    }
                }
                gl.use_program(Some(program.program));
                // `set_alpha_blend_mode(1)`: added.
                gl.enable(glow::BLEND);
                gl.blend_equation(glow::FUNC_ADD);
                gl.blend_func(glow::ONE, glow::ONE);
                let size = this
                    .h3_targets
                    .as_ref()
                    .map_or([1, 1], |targets| targets.size);
                let textures = this.slots[slot].flare_textures.clone();
                let first = frame.preview.ce_lights.len()
                    + frame.preview.h2_flares.len()
                    + frame.preview.h3_flares.len();
                for (index, draws) in drawn {
                    let images = textures.get(first + index);
                    for draw in draws {
                        let Some(texture) = images
                            .and_then(|images| images.get(draw.image))
                            .copied()
                            .flatten()
                        else {
                            continue;
                        };
                        let [flip_u, flip_v] = draw.flip.map(f32::from);
                        let corner = |x: f32, y: f32| {
                            let u = (x + 1.0) * 0.5;
                            let v = (1.0 - y) * 0.5;
                            [x, y, (u - flip_u).abs(), (v - flip_v).abs()]
                        };
                        let quad = [
                            corner(-1.0, -1.0),
                            corner(1.0, -1.0),
                            corner(-1.0, 1.0),
                            corner(1.0, 1.0),
                        ];
                        gl.buffer_data_u8_slice(
                            glow::ARRAY_BUFFER,
                            slice_bytes(&quad),
                            glow::STREAM_DRAW,
                        );
                        for (name, ty) in &pass.uniforms {
                            let location = program.location(gl, name);
                            if location.is_none() {
                                continue;
                            }
                            let floats = |value: [f32; 4]| Some(H3Value::Floats(value));
                            let value = match name.as_str() {
                                "centerRotation" => floats(draw.center_rotation),
                                "flareScale" => floats(draw.scale),
                                "flareOrigin_offsetBounds" => floats(draw.origin_bounds),
                                "transformedAxes" => floats(draw.axes),
                                "mirrorReflectionAcrossFlare" => Some(H3Value::Integer(0)),
                                "modulationFactor" => floats(draw.modulation),
                                "tintColor" => floats(draw.tint),
                                // `c_surface::get_alpha_multiplier`: 1 here.
                                "ps_scale" => floats([1.0; 4]),
                                other => h3_draw::frame_value(
                                    frame,
                                    gen3::shaders::H3Dialect::Halo4,
                                    other,
                                    size,
                                    0.0,
                                ),
                            };
                            if let Some(value) = value {
                                h3_draw::set_uniform_value(gl, location.as_ref(), ty, &value);
                            }
                        }
                        for (unit, (name, _)) in pass.textures.iter().enumerate() {
                            gl.active_texture(glow::TEXTURE0 + unit as u32);
                            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                            gl.uniform_1_i32(program.location(gl, name).as_ref(), unit as i32);
                        }
                        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                        // `mirror across flare`: again, reflected through it.
                        if draw.mirror {
                            let location = program.location(gl, "mirrorReflectionAcrossFlare");
                            gl.uniform_1_i32(location.as_ref(), 1);
                            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                        }
                    }
                }
            });
            if linked.is_none() {
                return;
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            gl.use_program(Some(self.program));
        }
    }
}
