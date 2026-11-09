//! The gen3 pipeline's own GL state: its targets, programs and screen
//! passes, and what its shared pass binder reads for Reach's and Halo 4's
//! materials until a per-dialect resolver takes that over.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use eframe::glow;

/// A light a pass draws by, one at a time and additive.
pub(in crate::app::model_preview) struct PassLight {
    /// `s_simple_light`, as `simple_lights` holds it.
    pub(in crate::app::model_preview) rows: [[f32; 4]; 5],
    /// `Shadow_Projection`, a plain column matrix.
    pub(in crate::app::model_preview) projection: [f32; 16],
    /// Its gel, for `dynamic_light_gel_texture`.
    pub(in crate::app::model_preview) gel: Option<glow::NativeTexture>,
}

/// What the gen3 pipeline keeps between and within frames. `Gen3Pipeline`
/// owns it, and the pipeline's stages are its methods, each handed the core.
pub(in crate::app::model_preview) struct Gen3State {
    /// The game's engine globals, which its passes read.
    pub(in crate::app::model_preview) frame_values: super::pipeline::FrameValues,
    /// The frame's targets: albedo and normals, the lit scene, what lies
    /// behind camo, the depth copy and the distortion.
    pub(in crate::app::model_preview) targets: Option<super::super::gen3::pipeline::Gen3Targets>,
    /// Linked passes by their address, or the pass that failed to link: each
    /// holds its pass, so no other pass can come to have its address.
    pub(in crate::app::model_preview) programs: HashMap<usize, Result<Rc<super::super::gen3::pipeline::Gen3Program>, Arc<super::shaders::Gen3Pass>>>,
    /// The light drawing now, which a game's dynamic lights set for each.
    pub(in crate::app::model_preview) light: Option<PassLight>,
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
    /// What the game sets in its passes that the shared binder does not.
    pub(in crate::app::model_preview) hooks: Box<dyn super::pipeline::Gen3PassHooks>,
}

impl Default for Gen3State {
    fn default() -> Self {
        Self {
            frame_values: super::pipeline::halo3_frame_values,
            targets: Default::default(),
            programs: Default::default(),
            light: Default::default(),
            distorted: Default::default(),
            distortion_program: Default::default(),
            depth_copy: Default::default(),
            blur_program: Default::default(),
            screen_alpha: Default::default(),
            empty_vertex_array: Default::default(),
            inline_lights: Default::default(),
            pass_values: Default::default(),
            shadow_mask: Default::default(),
            stops: Default::default(),
            sprite_queue: Default::default(),
            shadow_view_projection: Default::default(),
            hooks: Box::new(super::pipeline::NoPassHooks),
        }
    }
}
