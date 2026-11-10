//! Per-vertex streams beside the model's vertices, such as a game's PRT
//! coefficients: uploaded into the slot's own buffer with their attributes,
//! and kept parallel to the vertices as overlays, cloth and attached objects
//! add to them. The games' streams are listed in `load::games`.

use super::super::RenderModelPreview;
use eframe::glow;

/// Which stream: what a pipeline asks for when it draws by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum VertexStreamKind {
    /// The dx9 `Prt` layer's clusters and weights.
    Dx9Prt,
    /// The gen3 shaders' baked coefficients.
    Gen3Prt,
}

/// A stream a game's models carry beside their vertices.
pub(in crate::app::model_preview) trait VertexStream: Sync {
    fn kind(&self) -> VertexStreamKind;

    /// Whether `preview` carries it.
    fn present(&self, preview: &RenderModelPreview) -> bool;

    /// Upload `preview`'s into `buffer` and point the stream's attributes at
    /// it, or turn them off where `preview` has none.
    ///
    /// # Safety
    ///
    /// The slot's vertex array must be bound, in the current context.
    unsafe fn upload(
        &self,
        gl: &glow::Context,
        preview: &RenderModelPreview,
        buffer: glow::NativeBuffer,
    );

    /// Stay parallel to `count` added vertices that have none of it.
    fn pad(&self, preview: &mut RenderModelPreview, count: usize);

    /// Take on an attached object's own for its `count` vertices, or pad.
    fn append_attached(
        &self,
        preview: &mut RenderModelPreview,
        _attached: &RenderModelPreview,
        count: usize,
    ) {
        self.pad(preview, count);
    }
}
