//! Halo 3's and ODST's self shadow: drawn through the dx9 passes with its
//! own map size, bias and light, and applied after the opaque pass.

use super::super::dx9::shadow::{ShadowNormalTarget, ShadowStyle};
use super::super::frame::ModelGpuFrame;
use super::super::gen3;
use super::super::gen3::pipeline::{dominant_light, frame_probe, gen3_shadow_alpha};

/// Where Halo 3's and ODST's shadow apply reads the albedo pass's normals.
pub(in crate::app::model_preview) const H3_NORMAL_TARGET_UNIT: u32 = 9;

/// `c_rasterizer::render_globals.k_shadow_resolution`.
pub(in crate::app::model_preview) const H3_SHADOW_MAP_SIZE: i32 = 512;

/// `set_shadow_generate_mode`'s bias, the second in depth units.
pub(in crate::app::model_preview) const H3_SHADOW_SLOPE_BIAS: f32 = 1.414;

pub(in crate::app::model_preview) const H3_SHADOW_DEPTH_BIAS: f32 = 0.003;

/// Halo 3's and ODST's self shadow through the dx9 passes: a smaller map,
/// biased, from the dominant probe light, and applied after the opaque pass
/// rather than masked in the stencil.
pub(in crate::app::model_preview) fn h3_shadow_style(
    frame: &ModelGpuFrame,
    targets: Option<&gen3::pipeline::Gen3Targets>,
) -> ShadowStyle {
    let (toward, alpha) = h3_shadow_light(frame);
    ShadowStyle {
        map_size: H3_SHADOW_MAP_SIZE,
        bias: Some((
            H3_SHADOW_SLOPE_BIAS,
            H3_SHADOW_DEPTH_BIAS * (1 << 24) as f32,
        )),
        stencil_mask: false,
        toward,
        alpha,
        normal: targets.map(|targets| ShadowNormalTarget {
            texture: targets.normal,
            unit: H3_NORMAL_TARGET_UNIT,
            map_size: H3_SHADOW_MAP_SIZE,
        }),
    }
}

/// Which way an object's shadow falls from and how dark it is: the dominant
/// light, whose shadow fades out as the probe grows even.
pub(in crate::app::model_preview) fn h3_shadow_light(frame: &ModelGpuFrame) -> ([f32; 3], f32) {
    let ([x, y, z, _], _, contrast) = dominant_light(&frame_probe(frame));
    ([x, y, z], gen3_shadow_alpha(contrast))
}
