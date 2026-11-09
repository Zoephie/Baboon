//! What a frame holds of Halo 2's.

use super::super::dx9::passes::h2_prt_object_lights;
use super::super::frame::ModelGpuFrame;

/// What a frame holds of Halo 2's.
#[derive(Clone, Default)]
pub(in crate::app::model_preview) struct H2Frame {
    /// A PRT model's constants on the object path: lit, then in shadow.
    pub(in crate::app::model_preview) prt_clusters: Option<[Vec<[f32; 4]>; 2]>,
}

/// The frame's PRT constants, once the rest of it is set.
pub(in crate::app::model_preview) fn prepare(frame: &mut ModelGpuFrame) {
    frame.h2.prt_clusters = object_prt_clusters(frame);
}

fn object_prt_clusters(frame: &ModelGpuFrame) -> Option<[Vec<[f32; 4]>; 2]> {
    let prt = frame
        .preview
        .h2
        .prt
        .as_ref()
        .filter(|_| frame.prt && frame.game_shaders && !frame.default_lights)?;
    let (lights, ambient) = h2_prt_object_lights(frame.h2_object_light());
    Some([
        prt.clusters_lit_by(&lights, ambient),
        prt.clusters_lit_by(&lights[..1], ambient),
    ])
}
