//! The lights an effect brings.

use super::*;

/// A light an effect's part makes with `light_new_unattached`: it lives for
/// the tag's `duration`, fading by its `falloff function`.
#[derive(Debug, Clone)]
pub(crate) struct CeEffectLightDef {
    pub(crate) light: CeLight,
    /// In ticks, as tool's postprocess (`sub_49CF90`) leaves it.
    pub(super) duration: f32,
    pub(super) falloff: i32,
}

pub(in crate::app::model_preview) fn read_effect_light(reader: &TagReader, path: &str) -> Option<CeEffectLightDef> {
    let mut light = read_light_tag(reader, path)?;
    let tag = reader.load(path, "light", b"ligh").ok()?;
    let root = tag.root();
    // Its flags are made 3 whatever the tag's: it is always dynamic.
    light.dynamic = true;
    Some(CeEffectLightDef {
        light,
        duration: root.read_real("duration").unwrap_or(0.0) * 30.0,
        falloff: root.read_int_any("falloff function").unwrap_or(0) as i32,
    })
}

impl CeEffectLightDef {
    /// `lights_preprocess_scene` for an unattached light `age` ticks old:
    /// none once past its duration.
    pub(crate) fn at(&self, object: u8, node: usize, position: [f32; 3], forward: [f32; 3], age: u32, scale: f32) -> Option<CeLight> {
        let age = age as f32;
        if age > self.duration {
            return None;
        }
        let value = (1.0 - transition_evaluate(self.falloff, age / self.duration)) * scale;
        Some(CeLight { object, node, position, forward, fixed: Some(value), ..self.light.clone() })
    }

    pub(crate) fn alive(&self, age: u32) -> bool {
        age as f32 <= self.duration
    }
}
