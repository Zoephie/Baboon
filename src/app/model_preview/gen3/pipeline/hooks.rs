//! What a game sets in a gen3 pass that the shared binder does not: the
//! hooks `draw_pass` asks as it binds a pass's uniforms and textures, which
//! each game's pipeline keeps on its state.

use super::super::shaders::Gen3Values;
use super::*;

/// Where a game's passes differ inside [`Gen3State::draw_pass`]. Each
/// default is Halo 3's and ODST's: nothing of their own.
pub(in crate::app::model_preview) trait Gen3PassHooks:
    std::any::Any
{
    /// The hooks as themselves, for the game's own stages to reach what
    /// they keep.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Values the game works out for this draw of `pass`, which a uniform
    /// reads after the engine's globals and before the material's own.
    fn draw_values(
        &self,
        _frame: &ModelGpuFrame,
        _batch: &RenderModelPreviewBatch,
        _material: &Gen3Material,
        _pass: &Arc<Gen3Pass>,
    ) -> Option<Gen3Values> {
        None
    }

    /// `simple_lights` while no light of the pipeline's draws: the frame's
    /// inline rows, if the game's pass reads them.
    fn inline_simple_lights<'a>(
        &self,
        _material: &Gen3Material,
        _inline: &'a [[[f32; 4]; 5]],
    ) -> Option<&'a [f32]> {
        None
    }

    /// A matrix the game's passes read by a name of their own, set ahead
    /// of every value; whether it set `name`.
    #[allow(clippy::too_many_arguments)]
    unsafe fn set_matrix(
        &self,
        _gl: &glow::Context,
        _location: Option<&glow::NativeUniformLocation>,
        _frame: &ModelGpuFrame,
        _material: &Gen3Material,
        _name: &str,
        _shadow_view_projection: Option<[f32; 16]>,
    ) -> bool {
        false
    }

    /// A value of the game's lights for `name`, ahead of the engine's
    /// globals.
    fn light_value(
        &self,
        _material: &Gen3Material,
        _name: &str,
        _inline: &[[[f32; 4]; 5]],
    ) -> Option<Gen3Value> {
        None
    }

    /// What the game's camo reads as the picture behind the object, if it
    /// copies its own.
    fn camo_scene(&self) -> Option<glow::NativeTexture> {
        None
    }

    /// Whether `pass` of `material`, of shader group `group`, draws both
    /// faces.
    fn two_sided(
        &self,
        _material: &Gen3Material,
        _pass: &Arc<Gen3Pass>,
        _group: Option<u32>,
    ) -> bool {
        false
    }
}

/// Halo 3's and ODST's: no hooks of their own.
pub(in crate::app::model_preview) struct NoPassHooks;

impl Gen3PassHooks for NoPassHooks {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
