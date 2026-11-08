//! The GL state one pass hands the next, as a value: the calls a hand-off
//! makes, in the order it makes them, so each hand-off is named once and can
//! be read, compared and applied.

use eframe::glow::{self, HasContext as _};

/// One GL state call a hand-off makes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) enum StateCall {
    Enable(u32),
    Disable(u32),
    BlendEquation(u32),
    BlendFunc(u32, u32),
    BlendFuncSeparate(u32, u32, u32, u32),
    ColorMask([bool; 4]),
    DepthMask(bool),
    DepthFunc(u32),
    CullFace(u32),
    UseProgram(Option<glow::NativeProgram>),
    BindVertexArray(Option<glow::NativeVertexArray>),
    BindBuffer(u32, Option<glow::NativeBuffer>),
    ActiveTexture(u32),
    BindTexture(u32, Option<glow::NativeTexture>),
}

/// The state a pass hands over: only the calls it makes, in its order. Built
/// call by call; [`apply`](PassState::apply) makes them.
#[derive(Debug, Clone, Default, PartialEq)]
pub(in crate::app::model_preview) struct PassState {
    calls: Vec<StateCall>,
}

impl PassState {
    pub(in crate::app::model_preview) fn new() -> Self {
        Self::default()
    }

    fn with(mut self, call: StateCall) -> Self {
        self.calls.push(call);
        self
    }

    pub(in crate::app::model_preview) fn enable(self, capability: u32) -> Self {
        self.with(StateCall::Enable(capability))
    }

    pub(in crate::app::model_preview) fn disable(self, capability: u32) -> Self {
        self.with(StateCall::Disable(capability))
    }

    pub(in crate::app::model_preview) fn blend_equation(self, mode: u32) -> Self {
        self.with(StateCall::BlendEquation(mode))
    }

    pub(in crate::app::model_preview) fn blend_func(self, source: u32, destination: u32) -> Self {
        self.with(StateCall::BlendFunc(source, destination))
    }

    pub(in crate::app::model_preview) fn blend_func_separate(
        self,
        source: u32,
        destination: u32,
        source_alpha: u32,
        destination_alpha: u32,
    ) -> Self {
        self.with(StateCall::BlendFuncSeparate(source, destination, source_alpha, destination_alpha))
    }

    pub(in crate::app::model_preview) fn color_mask(self, mask: [bool; 4]) -> Self {
        self.with(StateCall::ColorMask(mask))
    }

    pub(in crate::app::model_preview) fn depth_mask(self, write: bool) -> Self {
        self.with(StateCall::DepthMask(write))
    }

    pub(in crate::app::model_preview) fn depth_func(self, function: u32) -> Self {
        self.with(StateCall::DepthFunc(function))
    }

    pub(in crate::app::model_preview) fn use_program(self, program: glow::NativeProgram) -> Self {
        self.with(StateCall::UseProgram(Some(program)))
    }

    pub(in crate::app::model_preview) fn bind_vertex_array(self, array: glow::NativeVertexArray) -> Self {
        self.with(StateCall::BindVertexArray(Some(array)))
    }

    pub(in crate::app::model_preview) fn bind_buffer(self, target: u32, buffer: glow::NativeBuffer) -> Self {
        self.with(StateCall::BindBuffer(target, Some(buffer)))
    }

    pub(in crate::app::model_preview) fn active_texture(self, unit: u32) -> Self {
        self.with(StateCall::ActiveTexture(unit))
    }

    pub(in crate::app::model_preview) fn bind_texture(self, target: u32, texture: Option<glow::NativeTexture>) -> Self {
        self.with(StateCall::BindTexture(target, texture))
    }

    /// `texture` bound to `target` on `unit`, then unit 0 active again: how
    /// a pass binds a map beside the one on unit 0.
    pub(in crate::app::model_preview) fn texture_on(self, unit: u32, target: u32, texture: Option<glow::NativeTexture>) -> Self {
        self.active_texture(unit).bind_texture(target, texture).active_texture(glow::TEXTURE0)
    }

    /// Faces culled as the model shows them, the back face named: off with
    /// back faces shown, else on, culling the back. The gen3 passes' rule.
    pub(in crate::app::model_preview) fn cull_back_unless(self, show_backfaces: bool) -> Self {
        if show_backfaces {
            self.disable(glow::CULL_FACE)
        } else {
            self.enable(glow::CULL_FACE).with(StateCall::CullFace(glow::BACK))
        }
    }

    /// Culling on or off as the model shows its back faces, leaving which
    /// face it culls as it was. What the end of the sprites hands back.
    pub(in crate::app::model_preview) fn culling_unless(self, show_backfaces: bool) -> Self {
        if show_backfaces {
            self.disable(glow::CULL_FACE)
        } else {
            self.enable(glow::CULL_FACE)
        }
    }

    /// The calls, in order.
    #[cfg(test)]
    pub(in crate::app::model_preview) fn calls(&self) -> &[StateCall] {
        &self.calls
    }

    /// Make the calls, in order.
    pub(in crate::app::model_preview) unsafe fn apply(&self, gl: &glow::Context) {
        for call in &self.calls {
            unsafe {
                match *call {
                    StateCall::Enable(capability) => gl.enable(capability),
                    StateCall::Disable(capability) => gl.disable(capability),
                    StateCall::BlendEquation(mode) => gl.blend_equation(mode),
                    StateCall::BlendFunc(source, destination) => gl.blend_func(source, destination),
                    StateCall::BlendFuncSeparate(source, destination, source_alpha, destination_alpha) => {
                        gl.blend_func_separate(source, destination, source_alpha, destination_alpha)
                    }
                    StateCall::ColorMask([red, green, blue, alpha]) => gl.color_mask(red, green, blue, alpha),
                    StateCall::DepthMask(write) => gl.depth_mask(write),
                    StateCall::DepthFunc(function) => gl.depth_func(function),
                    StateCall::CullFace(face) => gl.cull_face(face),
                    StateCall::UseProgram(program) => gl.use_program(program),
                    StateCall::BindVertexArray(array) => gl.bind_vertex_array(array),
                    StateCall::BindBuffer(target, buffer) => gl.bind_buffer(target, buffer),
                    StateCall::ActiveTexture(unit) => gl.active_texture(unit),
                    StateCall::BindTexture(target, texture) => gl.bind_texture(target, texture),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_keeps_its_calls_in_the_order_they_were_named() {
        let state = PassState::new().depth_mask(false).enable(glow::BLEND).cull_back_unless(false).culling_unless(true);
        assert_eq!(
            state.calls(),
            [
                StateCall::DepthMask(false),
                StateCall::Enable(glow::BLEND),
                StateCall::Enable(glow::CULL_FACE),
                StateCall::CullFace(glow::BACK),
                StateCall::Disable(glow::CULL_FACE),
            ]
        );
        assert_eq!(PassState::new().culling_unless(false).calls(), [StateCall::Enable(glow::CULL_FACE)], "no face named");
    }

    #[test]
    fn a_texture_on_another_unit_leaves_unit_0_active() {
        assert_eq!(
            PassState::new().texture_on(glow::TEXTURE2, glow::TEXTURE_3D, None).calls(),
            [
                StateCall::ActiveTexture(glow::TEXTURE2),
                StateCall::BindTexture(glow::TEXTURE_3D, None),
                StateCall::ActiveTexture(glow::TEXTURE0),
            ]
        );
    }
}
