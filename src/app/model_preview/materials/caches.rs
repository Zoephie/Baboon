//! The caches a resolve shares across a model's materials.

use super::super::super::TagNameIndex;
use super::super::TextureImage;
use super::{Gen3Bitmap, Gen3Shield, Gen3Sources, Image, SamplerKind};
use blam_tags::render_method::{RenderMethod, RenderMethodDefinition, RenderMethodOption};
use std::collections::HashMap;
use std::sync::Arc;

/// Loaders shared across one model's materials.
///
/// A render_model's materials overwhelmingly share a render-method definition
/// and its options — all seven of masterchief's resolve through
/// `shaders\shader.render_method_definition` — so caching across the batch
/// turns N of those reads into one.
#[derive(Default)]
pub(in crate::app::model_preview) struct ResolveCaches {
    /// Halo 4's: its material shaders, bitmap kinds, hologram screens and
    /// camo texture.
    pub(in crate::app::model_preview) h4: super::super::h4::materials::H4ResolveCaches,
    pub(in crate::app::model_preview) gen3_shield: Option<Option<Arc<Gen3Shield>>>,
    pub(in crate::app::model_preview) h3_sources: Option<Gen3Sources>,
    /// Images decoded for Halo 3 passes, with whether each is sRGB.
    pub(in crate::app::model_preview) h3_images: HashMap<(String, i16, usize), Option<Gen3Bitmap>>,
    pub(in crate::app::model_preview) definitions:
        HashMap<String, Option<Arc<RenderMethodDefinition>>>,
    pub(in crate::app::model_preview) options: HashMap<String, Option<Arc<RenderMethodOption>>>,
    /// Reach's reference shaders by path; Spartan armor shares a few.
    pub(in crate::app::model_preview) references: HashMap<String, Option<Arc<RenderMethod>>>,
    /// Decoded bitmaps by `(path, image index)`. Shared maps are common —
    /// masterchief's visor variants all reach for the same detail map.
    pub(in crate::app::model_preview) bitmaps: HashMap<(String, i16), Option<TextureImage>>,
    /// Halo 2's: its shader templates and the kit's camo shader.
    pub(in crate::app::model_preview) h2: super::super::h2::shader::resolve::H2ResolveCaches,
    /// Pass maps by path and shape; a shader samples each in only one shape.
    /// By path, shape and image.
    pub(in crate::app::model_preview) h2_images:
        HashMap<(String, SamplerKind, usize), Option<Arc<Image>>>,
    /// Halo CE's: its camouflage, bitmap image counts and model shader.
    pub(in crate::app::model_preview) ce: super::super::ce::shaders::resolve::CeResolveCaches,
    /// The kit's own group → extension names. The cross-game table answers
    /// with the H3+ meaning on a collision, and Halo CE's
    /// `shader_transparent_meter` (`smet`) is Halo 4's `structure_meta` there.
    pub(in crate::app::model_preview) kit_names: Option<TagNameIndex>,
}
