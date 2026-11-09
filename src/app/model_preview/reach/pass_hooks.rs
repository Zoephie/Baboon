//! What Reach's passes read that the shared binder does not: the frame's
//! inline lights and foliage's two faces.

use std::sync::Arc;

use super::super::gen3::pipeline::{Gen3PassHooks, reach_draws_two_sided};
use super::super::gen3::shaders::{H3Dialect, H3Material, H3Pass, H3Value};

/// Reach's pass hooks.
pub(in crate::app::model_preview) struct ReachPassHooks;

impl Gen3PassHooks for ReachPassHooks {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    /// A Reach static pass reads the frame's inline lights as `simple_lights`.
    fn inline_simple_lights<'a>(&self, material: &H3Material, inline: &'a [[[f32; 4]; 5]]) -> Option<&'a [f32]> {
        (material.dialect() == H3Dialect::Reach && !inline.is_empty()).then(|| inline.as_flattened().as_flattened())
    }

    /// `simple_light_count` for a Reach static pass.
    fn light_value(&self, material: &H3Material, name: &str, inline: &[[[f32; 4]; 5]]) -> Option<H3Value> {
        if material.dialect() != H3Dialect::Reach {
            return None;
        }
        match name {
            "simple_light_count" => Some(H3Value::Integer(inline.len() as i32)),
            _ => None,
        }
    }

    fn two_sided(&self, material: &H3Material, pass: &Arc<H3Pass>, group: Option<u32>) -> bool {
        reach_draws_two_sided(material, pass, group)
    }
}
