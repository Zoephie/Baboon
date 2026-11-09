//! The gen3 pipeline's own GL state: its targets, programs and screen
//! passes, and what its shared pass binder reads for Reach's and Halo 4's
//! materials until a per-dialect resolver takes that over.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use eframe::glow;

/// What the gen3 pipeline keeps between and within frames. `Gen3Pipeline`
/// owns it, and the pipeline's stages are its methods, each handed the core.
#[derive(Default)]
pub(in crate::app::model_preview) struct Gen3State {
    /// The frame's targets: albedo and normals, the lit scene, what lies
    /// behind camo, the depth copy and the distortion.
    pub(in crate::app::model_preview) targets: Option<super::super::gen3::pipeline::H3Targets>,
    /// Linked passes by their address, or the pass that failed to link: each
    /// holds its pass, so no other pass can come to have its address.
    pub(in crate::app::model_preview) programs: HashMap<usize, Result<Rc<super::super::gen3::pipeline::H3Program>, Arc<super::shaders::H3Pass>>>,
    /// The light drawing now, and its gel.
    pub(in crate::app::model_preview) light: Option<(super::super::h3::lights::H3LitLight, Option<glow::NativeTexture>)>,
    /// Whether this frame's particles drew any distortion to apply.
    pub(in crate::app::model_preview) distorted: bool,
    /// `apply_distortions`' program.
    pub(in crate::app::model_preview) distortion_program: Option<glow::NativeProgram>,
    /// This frame's copy of the H3 scene's depth, for soft particles; set
    /// only while an H3 picture draws its sprites.
    pub(in crate::app::model_preview) depth_copy: Option<glow::NativeTexture>,
    /// The screen effects' blur program.
    pub(in crate::app::model_preview) blur_program: Option<glow::NativeProgram>,
    /// How much of its effect the `shader_screen` drawing now is.
    pub(in crate::app::model_preview) screen_alpha: Option<f32>,
    /// What a full-screen triangle draws from.
    pub(in crate::app::model_preview) empty_vertex_array: Option<glow::NativeVertexArray>,
    /// Reach's inline lights this frame, `simple_lights` rows.
    pub(in crate::app::model_preview) inline_lights: Vec<[[f32; 4]; 5]>,
    /// Constants a Reach or Halo 4 pass of the frame sets over the material's:
    /// the self shadow's, the shield's and the camo's.
    pub(in crate::app::model_preview) pass_values: Option<std::collections::HashMap<&'static str, [f32; 4]>>,
    /// The self shadow mask the lit pass reads: Reach's or Halo 4's, made
    /// before it and dropped after.
    pub(in crate::app::model_preview) shadow_mask: Option<glow::NativeTexture>,
    /// The exposure stops Reach's area screen effects come to this frame,
    /// set before its scene draws; 0 outside a Reach picture.
    pub(in crate::app::model_preview) stops: f32,
    /// The frame's sprites, sorted the first time they draw and dropped
    /// when the frame's last ones have, or when the next frame begins.
    pub(in crate::app::model_preview) sprite_queue: Option<super::sprites::SpriteQueue>,
    /// The light's view a self shadow's depth draws through in place of the
    /// camera, while it draws: Reach's and Halo 4's.
    pub(in crate::app::model_preview) shadow_view_projection: Option<[f32; 16]>,
    /// The picture as Halo 4's camo reads it, bytes as they are.
    pub(in crate::app::model_preview) camo_scene: Option<glow::NativeTexture>,
    /// Which draw of a Halo 4 material is under way, when not its albedo or
    /// own pass (a hologram's or `opaque_alpha_blend`'s fringe).
    pub(in crate::app::model_preview) submit_draw: Option<super::super::h4::materials::submit::H4Draw>,
}
