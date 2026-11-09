//! Halo 4's shield impact: the `shield_impact` an object's damage sections
//! give it, drawn through `rasterizer_globals`' explicit shader 61 (`c_render_shield::render_object_internal`,
//! X360 0x83AE5FD8, MCC 0x1408EE840). The tag reads as Reach's does by name,
//! and the section is chosen as Reach chooses it; what differs is the shader,
//! which applies its own exposure, and `shield_vitality`, which the game takes
//! from the shield's damage section. See
//! `docs/h4-preview-research/shadow-camo-shields.md` §3.

use crate::app::model_preview::load::TagReader;
use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::TagFile;

use super::super::common::fields::elements;
use super::super::functions::FunctionValues;
use super::super::gen3::shaders::{Gen3Pass, Gen3Texture};
use super::super::gen3::shield::{OSCILLATION, PLASMA, ReachShieldSettings, shield_impact_path};
use crate::core::hlsl::emit::Stage;
use crate::app::model_preview::h4::materials::shaders::rewire_depth_fade;

/// `rasterizer_globals`' explicit shader 61, `shield_impact`.
const SHIELD_IMPACT: usize = 61;
/// What the preview's depth reads back as world distance with: as Reach's.
pub(crate) const DEPTH_DECODE: &str = super::super::gen3::shield::DEPTH_DECODE;
pub(crate) const WORLD_SCALE: &str = super::super::gen3::shield::WORLD_SCALE;

/// The shield's settings, its textures under Halo 4's names, and its pass.
#[derive(Debug)]
pub(crate) struct H4Shield {
    pub(crate) settings: ReachShieldSettings,
    pub(crate) textures: Vec<Gen3Texture>,
    pub(crate) pass: Arc<Gen3Pass>,
}

impl H4Shield {
    /// The constants `render_object_callback` submits at shield damage
    /// `damage`, with no hit landed: `shield_vitality` is the shield
    /// section's own, full at rest.
    pub(crate) fn values(
        &self,
        damage: f32,
        time: f32,
        values: Option<&FunctionValues>,
        center: [f32; 3],
    ) -> HashMap<&'static str, [f32; 4]> {
        let mut object = values.cloned().unwrap_or_default();
        object.insert("shield_vitality".to_owned(), 1.0);
        let state = self.settings.state(damage, time, Some(&object));
        self.settings.values(&state, center)
    }
}

/// The model's shield and its pass, if it has one: technique 0, as no hit
/// lands in the preview for the hit blob (technique 1) to shape.
pub(crate) fn read_h4_shield(reader: &TagReader, model: &TagFile) -> Option<Arc<H4Shield>> {
    let path = shield_impact_path(model)?;
    let settings = ReachShieldSettings::read(reader, &path)?;
    let pass = Arc::new(h4_shield_pass(reader, 0).ok()?);
    let rename = |name: &str| {
        match name {
            name if name == PLASMA[0] => "shield_impact_noise_texture1_texture",
            name if name == PLASMA[1] => "shield_impact_noise_texture2_texture",
            name if name == OSCILLATION[0] => "vs_shield_impact_noise_texture1_texture",
            name if name == OSCILLATION[1] => "vs_shield_impact_noise_texture2_texture",
            other => other,
        }
        .to_owned()
    };
    let textures = settings
        .textures
        .iter()
        .map(|texture| Gen3Texture {
            uniform: rename(&texture.uniform),
            image: Arc::clone(&texture.image),
            texels: texture.texels,
            mips: Arc::clone(&texture.mips),
            sampler: texture.sampler,
            frames: Vec::new(),
        })
        .collect();
    Some(Arc::new(H4Shield {
        settings,
        textures,
        pass,
    }))
}

/// Explicit 61's `technique` on the preview's skinned vertices, made to read
/// the preview's depth: `ComputeDepthFade`'s `psDepthConstants` decode is
/// the game's reversed depth, here the preview's window depth to world units.
pub(crate) fn h4_shield_pass(reader: &TagReader, technique: usize) -> Result<Gen3Pass, String> {
    let globals = reader.load("globals\\rasterizer_globals",
        "rasterizer_globals",
        b"rasg",
    )
    .map_err(|error| error.to_string())?;
    let explicit = elements(globals.root(), "explicit shaders");
    let entry = explicit.get(SHIELD_IMPACT).ok_or("no explicit shader 61")?;
    let path = entry
        .read_tag_ref_path("explicit pixel shader")
        .filter(|path| !path.is_empty())
        .ok_or("no pixel shader")?;
    let program = super::materials::shaders::H4ShaderSource::read(reader, &path)?
        .program(&super::materials::shaders::H4_DEFINES)?;
    let vertex =
        super::materials::shaders::translate_technique(&program, technique, "skinned", Stage::Vertex)?;
    let pixel =
        super::materials::shaders::translate_technique(&program, technique, "_default", Stage::Pixel)?;
    let mut pass = super::super::gen3::shaders::pass_of(vertex, pixel);
    rewire_depth_fade(&mut pass)?;
    Ok(pass)
}

#[cfg(test)]
mod tests {
    use crate::app::model_preview::harness::kit::kit_source;
    use crate::core::game::GameId;
    use crate::core::source::TagSource;
    use crate::app::export::load_referenced_tag_from_source;
    use super::*;

    fn model(source: &TagSource, path: &str) -> TagFile {
        load_referenced_tag_from_source(source, path, "model", b"hlmt").unwrap()
    }

    /// The Chief and the Elite take their live `shield` sections' impacts,
    /// past the Elite's armour lock and engineer sections that start
    /// inactive; neither shows at rest, and both show once hit. A Grunt has
    /// no shield.
    #[test]
    fn the_kits_bipeds_take_their_live_shield() {
        let Some(source) = kit_source(GameId::Halo4) else { return };
        let chief = model(
            &source,
            "objects\\characters\\storm_masterchief\\storm_masterchief",
        );
        assert_eq!(
            shield_impact_path(&chief).as_deref(),
            Some("fx\\reach\\shield_impacts\\spartan_3p")
        );
        let elite = model(
            &source,
            "objects\\characters\\storm_elite_ai\\storm_elite_ai",
        );
        assert_eq!(
            shield_impact_path(&elite).as_deref(),
            Some("fx\\reach\\shield_impacts\\elite_3p")
        );
        let grunt = model(&source, "objects\\characters\\storm_grunt\\storm_grunt");
        assert_eq!(shield_impact_path(&grunt), None);
        let shield = read_h4_shield(&crate::app::model_preview::load::TagReader::new(&source), &chief).expect("the Chief's shield");
        assert!(!shield.settings.showing(0.0));
        assert!(shield.settings.showing(0.5));
        assert_eq!(shield.textures.len(), 4);
        assert!(
            shield
                .textures
                .iter()
                .any(|texture| texture.uniform == "vs_shield_impact_noise_texture1_texture")
        );
        // `shield_vitality` reads full: the edge glow's intensity is on it.
        let values = shield.values(1.0, 0.5, None, [0.0; 3]);
        assert!(
            values["edge_glow"][..3]
                .iter()
                .any(|channel| *channel > 0.0),
            "{:?}",
            values["edge_glow"]
        );
    }
}
