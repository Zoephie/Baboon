//! Halo 4: its material shaders and what each draw submits, its objects'
//! features (lights, flares, camo, shield, shadow, particles, tracers and
//! holograms), and its place in the gen3 pipeline (`render.rs`): its frame
//! values and lighting, the passes of each, its bloom and composite.

mod features;
pub(in crate::app::model_preview) mod materials;
pub(in crate::app::model_preview) mod effects;
pub(in crate::app::model_preview) mod object;
pub(in crate::app::model_preview) mod attached;
pub(in crate::app::model_preview) mod bloom;
pub(super) mod camo;
pub(super) mod camo_pass;
pub(in crate::app::model_preview) mod composite;
pub(super) mod flares;
pub(in crate::app::model_preview) mod holograms;
pub(super) mod lighting;
pub(in crate::app::model_preview) mod light_pass;
pub(in crate::app::model_preview) mod pass_hooks;
pub(super) mod lights;
pub(in crate::app::model_preview) mod render;
pub(super) mod shadow;
pub(in crate::app::model_preview) mod shadow_pass;
pub(super) mod shield;
pub(in crate::app::model_preview) mod slot;
pub(super) mod tracers;

/// What only a Halo 4 object brings to its preview: its lights and their
/// passes, flares, camo, shield, self shadow, tracers, colour grading and
/// bounding sphere.
#[derive(Debug, Clone, Default)]
pub(crate) struct H4Features {
    /// Halo 4's attached `light`s and `cheap_light`s.
    pub lights: Vec<super::h4::lights::H4Light>,
    pub cheap_lights: Vec<super::h4::lights::H4CheapLight>,
    /// Explicit 187 and 204's screen-space passes, by (physically correct,
    /// technique).
    pub light_passes: std::collections::BTreeMap<(bool, usize), std::sync::Arc<super::gen3::shaders::H3Pass>>,
    /// Halo 4's attached `lens_flare`s, and explicit 46's pass that draws them.
    pub flares: Vec<super::h4::flares::H4FlareAt>,
    pub flare_pass: Option<std::sync::Arc<super::gen3::shaders::H3Pass>>,
    /// `globals`' active camo, for a Halo 4 object.
    pub camo: Option<std::sync::Arc<super::h4::camo::H4Camo>>,
    /// A Halo 4 object's shield impact.
    pub shield: Option<std::sync::Arc<super::h4::shield::H4Shield>>,
    /// Whether a Halo 4 object shadows itself, and how.
    pub shadow_caster: Option<super::h4::shadow::H4ShadowCaster>,
    /// Halo 4's point-to-point tracers.
    pub tracers: Vec<super::h4::tracers::H4TracerAt>,
    /// Halo 4's final composite colour grading: the default camera fx
    /// settings' 16-cube lookup.
    pub color_grading: Option<super::gen3::materials::H3Bitmap>,
    /// A Halo 4 object's `bounding offset` (in its node 0's frame) and
    /// `bounding radius`, which its holograms frame their screen effects by.
    pub bounding_sphere: Option<([f32; 3], f32)>,
    /// How many change colours a Halo 4 object has (its `change colors`
    /// block, at most four): those its materials read as theirs, the rest
    /// as white.
    pub change_color_count: u8,
}
