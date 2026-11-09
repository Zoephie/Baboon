//! A material's decoded textures and detail composition.

use super::*;

/// One decoded texture, ready to become a GL texture.
#[derive(Debug, Clone)]
pub(crate) struct TextureImage {
    pub rgba: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Whether the sampler should repeat rather than clamp, per axis. Halo
    /// authors detail maps to tile many times over a surface, so getting this
    /// wrong smears one texel across the whole model.
    pub repeat_x: bool,
    pub repeat_y: bool,
    /// UV multiplier for this slot, per axis: 329 of the 2,468 Halo 2
    /// parameters that author an x/y scale pair tile the two differently, and
    /// Halo CE's `map u/v scale` and `detail map v scale` are per axis too.
    pub scale: [f32; 2],
}

impl TextureImage {
    /// Whether its pixels fill its size, which is how much GL reads.
    pub(crate) fn whole(&self) -> bool {
        rgba_fills(&self.rgba, &[self.width, self.height])
    }
}

/// Whether `rgba` holds four bytes for every texel of `dimensions`.
pub(crate) fn rgba_fills(rgba: &[u8], dimensions: &[usize]) -> bool {
    let texels = dimensions
        .iter()
        .try_fold(4usize, |bytes, side| bytes.checked_mul(*side));
    texels.is_some_and(|bytes| rgba.len() >= bytes)
}

/// How a material's detail map combines with its base map.
///
/// `LinearBiasedMultiply` is the preview's long-standing Halo 3 / Reach
/// combine and stays theirs. The other three are Halo CE's `detail function`s
/// (`fx/model_common.h`), computed as the engine did, on the stored (gamma)
/// values; Halo 2 uses the first of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DetailFunction {
    #[default]
    LinearBiasedMultiply,
    /// `base · detail · 2`; mid-grey leaves the base untouched.
    BiasedMultiply,
    /// `base · detail`.
    Multiply,
    /// `base + 2 · detail − 1`.
    BiasedAdd,
}

/// What masks a material's detail map, as Halo CE's `detail mask` numbers it:
/// 0 is none; then reflection, self-illumination, change-colour and the
/// multipurpose alpha, each as inverse (odd) then direct (even). Each reads one
/// multipurpose-map channel (`b`, `g`, `a`, `r`) after the optional Xbox
/// channel reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct DetailComposition {
    pub function: DetailFunction,
    pub mask: u8,
    /// `use xbox multipurpose channel order`: sample the map as `agrb`.
    pub xbox_channel_order: bool,
}

/// Every texture one material draws with.
#[derive(Debug, Clone, Default)]
pub(crate) struct MaterialTextures {
    pub slots: [Option<TextureImage>; SLOT_COUNT],
    pub detail: DetailComposition,
    /// Why this material has no textures, when it has none. Kept so the panel
    /// can say what went wrong instead of silently drawing it untextured.
    pub error: Option<String>,
    /// Set when the render-method definition could not be read and the slots
    /// came from the shader's own parameter block alone. Everything the shader
    /// authors is still found; what is missed are values that would have come
    /// from an option default — including the detail maps' tiling, which lives
    /// there and defaults to 16.
    pub used_shader_parameters_only: bool,
    /// Halo 2 passes by `Dx9Layer`, drawn in place of the slots above; empty
    /// for other games and outside a loose kit, whose `.psh` sources they need.
    pub h2_layers: [Vec<Dx9DrawPass>; H2_LAYER_COUNT],
    /// Halo 3's own shader, translated from the kit's source.
    pub h3: Option<Arc<Gen3Material>>,
    /// Reach's `only render for shields`: `render_model_postprocess` (the
    /// 360's) makes its parts a shield's, which no ordinary pass draws.
    pub shield_only: bool,
    /// Reach's `never render for shields`: an ordinary part the shield skips.
    pub never_shield: bool,
    /// Why it draws bare unless its passes do.
    pub bare_without_passes: Option<String>,
}

impl MaterialTextures {
    pub(crate) fn get(&self, slot: TextureSlot) -> Option<&TextureImage> {
        self.slots[slot as usize].as_ref()
    }

    pub(in crate::app::model_preview) fn failed(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Default::default()
        }
    }
}
