//! The probe, the dominant light and the static and dynamic lighting passes.

use super::*;

/// `c_geometry_sampler::m_default_lightprobe_sample`, the first nine terms of
/// each channel: the light of an object with no level around it.
pub(super) const DEFAULT_PROBE: [[f32; 9]; 3] = [
    [1.89376, -1.44314, 0.19557, 0.51599, -0.30808, -0.04173, -0.46665, 0.29195, 0.10501],
    [2.13512, -1.79217, 0.15439, 0.51479, -0.27409, 0.07048, -0.62705, 0.25751, -0.01816],
    [2.25851, -2.07371, 0.05633, 0.44103, -0.19508, 0.23114, -0.7642, 0.18839, -0.22193],
];

/// `get_render_exposure` at no stops and a white point of one.
pub(in crate::app) const EXPOSURE: f32 = 0.669_432_94;

/// `setup_default_lighting`'s light straight down, in white.
pub(super) const DOMINANT_DIRECTION: [f32; 4] = [0.0, 0.0, -1.0, 1.0];

pub(super) const DOMINANT_INTENSITY: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// `calculate_and_set_ravi_constants_internal`.
pub(super) fn lighting_constants(probe: &[[f32; 9]; 3]) -> [[f32; 4]; 10] {
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

/// `calculate_dominant_light_from_lightprobe`: the probe's first order, by
/// luminance, and how much of the probe lies along it.
pub(super) fn dominant_light(probe: &[[f32; 9]; 3]) -> ([f32; 4], [f32; 4], f32) {
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
pub(in crate::app::model_preview) fn h3_shadow_light(frame: &ModelGpuFrame) -> ([f32; 3], f32) {
    let ([x, y, z, _], _, contrast) = dominant_light(&frame_probe(frame));
    ([x, y, z], h3_shadow_alpha(contrast))
}

pub(super) fn h3_shadow_alpha(contrast: f32) -> f32 {
    (contrast.clamp(2.0, 8.0) - 2.0) / 6.0
}

/// The default probe turned about the vertical by `angle`, the sky staying up.
pub(super) fn turned_probe(angle: f32) -> [[f32; 9]; 3] {
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
pub(super) fn frame_probe(frame: &ModelGpuFrame) -> [[f32; 9]; 3] {
    if !frame.light_follows_camera {
        return DEFAULT_PROBE;
    }
    let [x, y, _] = camera_light(frame.camera.yaw, frame.camera.pitch);
    turned_probe(y.atan2(x) - probe_heading())
}

/// What a particle's `lighting` option reads.
pub(in crate::app::model_preview) fn h3_lighting_constants(frame: &ModelGpuFrame) -> [[f32; 4]; 10] {
    lighting_constants(&frame_probe(frame))
}

impl Gen3State {
    /// The static lighting into the accumulation target: the frame's inline
    /// lights (`build_simple_light_draw_list`), the opaque parts' lit pass,
    /// then what the game draws after the opaque parts. What blends, for the
    /// transparents.
    pub(super) unsafe fn static_lighting<'b>(
        &mut self,
        core: &mut RenderCore,
        game: &mut dyn Gen3Game,
        drawing: &'b Gen3Frame<'_>,
    ) -> Vec<&'b Gen3Batch> {
        let Gen3Frame { gl, frame, slot, order, .. } = *drawing;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(drawing.accumulation_target));
            gl.disable(glow::BLEND);
            h3_set_cull(gl, frame);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(true, true, true, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            // `build_simple_light_draw_list`, before the static pass reads it.
            self.inline_lights = game.inline_lights(&mut Gen3Cx { gl, frame, core, slot, state: self });
            let (opaque, blended): (Vec<_>, Vec<_>) =
                drawing.batches
                    .iter()
                    .filter(|(_, material)| !material.decal || material.after_lighting)
                    .partition(|(_, material)| material.blend == H3Blend::Opaque);
            game.lit_values(&mut Gen3Cx { gl, frame, core, slot, state: self });
            for (batch, material) in opaque {
                let pass = h3_lit_pass(frame, batch, material);
                self.draw_pass(core, gl, frame, slot, batch, material, pass, None);
            }
            self.shadow_mask = None;
            self.pass_values = None;
            game.after_opaque(&mut Gen3Cx { gl, frame, core, slot, state: self }, order);
            blended
        }
    }

    /// `submit_visibility_and_render`: the object's lights, additive, each
    /// on its own, which the game draws.
    pub(super) unsafe fn dynamic_lights(&mut self, core: &mut RenderCore, game: &mut dyn Gen3Game, drawing: &Gen3Frame<'_>) {
        let Gen3Frame { gl, frame, slot, .. } = *drawing;
        unsafe {
            gl.enable(glow::BLEND);
            gl.depth_mask(false);
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.blend_func(glow::ONE, glow::ONE);
            game.dynamic_lights(&mut Gen3Cx { gl, frame, core, slot, state: self }, &drawing.batches);
        }
    }
}
