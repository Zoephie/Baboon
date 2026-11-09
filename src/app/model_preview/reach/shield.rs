//! Reach's shield impact: the model's `shield_impact` with both its explicit
//! passes, translated to read the preview's depth. The settings and constants
//! it shares with Halo 4 are in `gen3::shield`. See
//! `docs/reach-preview-research/shield.md`.

use crate::app::model_preview::load::TagReader;
use std::sync::Arc;

use blam_tags::TagFile;

use super::super::gen3::shaders::{Gen3Pass, Gen3Sources};
use crate::core::hlsl::parse::{Scalar, TextureKind, Type};
use super::super::gen3::shield::{DEPTH_DECODE, OSCILLATION, PLASMA, ReachShieldSettings, shield_impact_path};

/// The two explicit shaders and the settings that pick between them.
#[derive(Debug)]
pub(crate) struct ReachShield {
    pub(crate) settings: ReachShieldSettings,
    /// Explicit shader 61, `shield_impact`.
    pass: Arc<Gen3Pass>,
    /// 119, `shield_impact_procedural_uv`.
    procedural: Option<Arc<Gen3Pass>>,
}

impl ReachShield {
    pub(crate) fn pass(&self) -> &Arc<Gen3Pass> {
        self.procedural.as_ref().filter(|_| self.settings.procedural).unwrap_or(&self.pass)
    }

    /// 61, then 119 if it translated.
    #[cfg(test)]
    pub(crate) fn passes(&self) -> impl Iterator<Item = &Arc<Gen3Pass>> {
        std::iter::once(&self.pass).chain(&self.procedural)
    }
}

/// The model's shield, with both its passes, if it has one.
pub(crate) fn read_reach_shield(reader: &TagReader, model: &TagFile) -> Option<Arc<ReachShield>> {
    let path = shield_impact_path(model)?;
    let settings = ReachShieldSettings::read(reader, &path)?;
    let mut sources = Gen3Sources::new(&reader.source);
    let pass = Arc::new(shield_pass(&*sources.explicit_pass("shield_impact").ok()?));
    let procedural = sources.explicit_pass("shield_impact_procedural_uv").ok().map(|pass| Arc::new(shield_pass(&pass)));
    Some(Arc::new(ReachShield { settings, pass, procedural }))
}

/// The translated pass, made to read the preview's depth and to give its
/// vertex stage the oscillation textures.
fn shield_pass(pass: &Gen3Pass) -> Gen3Pass {
    let mut vertex = pass.vertex.clone();
    for (plasma, oscillation) in PLASMA.iter().zip(OSCILLATION) {
        vertex = vertex.replace(plasma, oscillation);
    }
    // `compute_depth_fade`: Reach's reversed float depth, in the preview's
    // terms. Its view depth (`view_depth` of the soft particles) runs from the
    // orbit point; the eye sits `.w` before it, and `.z` scales to world units.
    let pixel = pass.pixel.replace(
        "float l_scene_depth = (1.0 / (depth_constants.z - (l_depth_value.x * depth_constants.y)));",
        &format!(
            "float l_window_ndc = l_depth_value.x * 2.0 - 1.0;
float l_scene_depth = ((l_window_ndc - {DEPTH_DECODE}.y) / ({DEPTH_DECODE}.x - l_window_ndc * {DEPTH_DECODE}.z) + {DEPTH_DECODE}.w) / reach_shield_world_scale;"
        ),
    );
    let pixel = pixel.replacen(
        "uniform sampler2D depth_buffer;",
        &format!("uniform sampler2D depth_buffer;\nuniform vec4 {DEPTH_DECODE};\nuniform float reach_shield_world_scale;"),
        1,
    );
    let mut uniforms = pass.uniforms.clone();
    uniforms.push((DEPTH_DECODE.to_owned(), Type::Vector(Scalar::Float, 4)));
    uniforms.push(("reach_shield_world_scale".to_owned(), Type::Scalar(Scalar::Float)));
    let mut textures = pass.textures.clone();
    textures.extend(OSCILLATION.iter().map(|name| ((*name).to_owned(), TextureKind::Flat)));
    Gen3Pass { fullscreen: false, vertex, pixel, uniforms, textures }
}
