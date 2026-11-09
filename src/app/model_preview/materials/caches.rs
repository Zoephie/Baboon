//! The caches a resolve shares across a model's materials.

use super::*;

/// Loaders shared across one model's materials.
///
/// A render_model's materials overwhelmingly share a render-method definition
/// and its options — all seven of masterchief's resolve through
/// `shaders\shader.render_method_definition` — so caching across the batch
/// turns N of those reads into one.
#[derive(Default)]
pub(in crate::app::model_preview) struct ResolveCaches {
    /// Halo 4's material shaders by path, read and parsed once.
    pub(in crate::app::model_preview) h4_shaders:
        HashMap<String, Result<super::super::h4::materials::H4Shader, Arc<str>>>,
    /// Halo 4's bitmaps by path: whether each exists, and is a cube.
    pub(in crate::app::model_preview) h4_bitmap_kinds: HashMap<String, Option<bool>>,
    /// `rasterizer_globals`' two hologram screen materials, read once.
    pub(in crate::app::model_preview) h4_hologram_post:
        Option<Option<Arc<crate::app::model_preview::h4::materials::submit::H4HologramPost>>>,
    /// `globals`' `camo distortion texture`, read once.
    pub(in crate::app::model_preview) h4_camo_texture: Option<Option<String>>,
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
    /// Halo 2 shader templates' parameter defaults, by template path. 1,637
    /// shaders share a few dozen templates; `tex_bump` alone serves 296.
    pub(in crate::app::model_preview) h2_templates:
        HashMap<String, Option<Arc<H2TemplateDefaults>>>,
    /// The kit's `global shader`, read at most once; see [`prepare_h2_camo_passes`].
    pub(in crate::app::model_preview) h2_camo_shader: Option<Option<Arc<TagFile>>>,
    /// Pass maps by path and shape; a shader samples each in only one shape.
    /// By path, shape and image.
    pub(in crate::app::model_preview) h2_images:
        HashMap<(String, SamplerKind, usize), Option<Arc<Image>>>,
    /// Halo CE's camouflage, read once.
    pub(in crate::app::model_preview) ce_camo: Option<Option<CeCamoSettings>>,
    /// How many images a Halo CE bitmap holds.
    pub(in crate::app::model_preview) ce_image_counts: HashMap<String, usize>,
    /// The kit's model shader as GLSL, read once. `None` until asked for.
    pub(in crate::app::model_preview) ce_model_common: Option<Option<CeModelCommon>>,
    /// The kit's own group → extension names. The cross-game table answers
    /// with the H3+ meaning on a collision, and Halo CE's
    /// `shader_transparent_meter` (`smet`) is Halo 4's `structure_meta` there.
    pub(in crate::app::model_preview) kit_names: Option<TagNameIndex>,
}
