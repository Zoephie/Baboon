//! What Reach's passes read that the shared binder does not: the frame's
//! inline lights and foliage's two faces.

use std::sync::Arc;

use super::super::gen3::pipeline::Gen3PassHooks;
use super::super::gen3::shaders::{Gen3Dialect, Gen3Material, Gen3Pass, Gen3Value};

/// Reach's pass hooks.
pub(in crate::app::model_preview) struct ReachPassHooks;

impl Gen3PassHooks for ReachPassHooks {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    /// A Reach static pass reads the frame's inline lights as `simple_lights`.
    fn inline_simple_lights<'a>(&self, material: &Gen3Material, inline: &'a [[[f32; 4]; 5]]) -> Option<&'a [f32]> {
        (material.dialect() == Gen3Dialect::Reach && !inline.is_empty()).then_some(inline.as_flattened().as_flattened())
    }

    /// `simple_light_count` for a Reach static pass.
    fn light_value(&self, material: &Gen3Material, name: &str, inline: &[[[f32; 4]; 5]]) -> Option<Gen3Value> {
        if material.dialect() != Gen3Dialect::Reach {
            return None;
        }
        match name {
            "simple_light_count" => Some(Gen3Value::Integer(inline.len() as i32)),
            _ => None,
        }
    }

    fn two_sided(&self, material: &Gen3Material, pass: &Arc<Gen3Pass>, group: Option<u32>) -> bool {
        reach_draws_two_sided(material, pass, group)
    }
}

/// Whether Reach draws this part with no face culled: `render_mesh_part_default`
/// takes its cull mode from `c_render_method::get_culling_mode` (X360
/// 0x826E9E40), none for `shader_foliage` (`rmfl`, 0x726D666C) and back faces
/// for every other group. Only the material's own passes; a shield or shadow
/// drawn over the part keeps its own culling.
pub(in crate::app::model_preview) fn reach_draws_two_sided(material: &Gen3Material, pass: &Arc<Gen3Pass>, group: Option<u32>) -> bool {
    let foliage = group == Some(u32::from_be_bytes(*b"rmfl"));
    let own = [material.albedo.as_ref(), Some(&material.lit), material.camo.as_ref(), material.dynamic_light.as_ref()]
        .into_iter()
        .chain(material.prt.iter().map(Option::as_ref))
        .flatten()
        .any(|known| Arc::ptr_eq(known, pass));
    material.dialect() == Gen3Dialect::Reach && foliage && own
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::gen3::shaders::{Gen3Blend, Gen3MaterialDialect};

    /// Reach's foliage draws both faces of its own passes; any other group
    /// (a halogram among them), Halo 3's foliage, and a pass laid over the
    /// part keep culling.
    #[test]
    fn only_reach_foliages_own_passes_draw_two_sided() {
        let pass = || Arc::new(Gen3Pass { fullscreen: false, vertex: String::new(), pixel: String::new(), uniforms: Vec::new(), textures: Vec::new() });
        let material = |dialect| Gen3Material {
            dialect,
            albedo: None,
            lit: pass(),
            screen_maps: Vec::new(),
            decal: false,
            decal_modulates_specular: false,
            after_lighting: false,
            sort_layer: 0,
            prt: [None, None, Some(pass())],
            camo: None,
            shadow_generate: None,
            dynamic_light: None,
            blend: Gen3Blend::Additive,
            values: Default::default(),
            textures: Vec::new(),
            change_colors: Vec::new(),
            animations: Vec::new(),
            shield: None,
            fringe: None,
        };
        let foliage = Some(u32::from_be_bytes(*b"rmfl"));
        let reach = material(Gen3MaterialDialect::Reach);
        assert!(reach_draws_two_sided(&reach, &reach.lit, foliage));
        assert!(reach_draws_two_sided(&reach, reach.prt[2].as_ref().unwrap(), foliage));
        assert!(!reach_draws_two_sided(&reach, &pass(), foliage), "a shield over it culls");
        assert!(!reach_draws_two_sided(&reach, &reach.lit, Some(u32::from_be_bytes(*b"rmhg"))), "a halogram culls");
        let halo3 = material(Gen3MaterialDialect::Halo3);
        assert!(!reach_draws_two_sided(&halo3, &halo3.lit, foliage));
    }
}
