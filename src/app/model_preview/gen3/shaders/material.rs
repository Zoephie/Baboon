//! A material's passes and what they bind: textures, values and the shield.

use super::animation::Gen3Animation;
use super::dialect::{Gen3Dialect, RenderMethodDialect};
use super::pass::{Gen3Blend, Gen3Pass};
use super::{Image, ImageTexels, Sampler};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Gen3Value {
    Floats([f32; 4]),
    Integer(i32),
    /// A `float4[n]` uniform, as Halo 4's `ps_material_shader_transforms`.
    FloatArray(Vec<[f32; 4]>),
}

#[derive(Debug)]
pub(crate) struct Gen3Texture {
    pub(crate) uniform: String,
    pub(crate) image: Arc<Image>,
    pub(crate) texels: ImageTexels,
    /// The levels past `image`, as the bitmap stores them.
    pub(crate) mips: Arc<Vec<Image>>,
    /// What the tag asks of the sampler; none for the engine's own textures.
    pub(crate) sampler: Option<Sampler>,
    /// The bitmap's other images, in order with `image` standing at its index,
    /// for a `FrameIndex` animation to choose among. Empty without one.
    pub(crate) frames: Vec<Arc<Image>>,
}

/// A shader's passes and everything they read.
#[derive(Debug)]
pub(crate) struct Gen3Material {
    /// Whose frame lights it, with Halo 4's per-draw functions and hologram
    /// rig: read through [`Gen3Material::dialect`] and [`Gen3Material::h4`].
    pub(crate) dialect: Gen3MaterialDialect,
    pub(crate) albedo: Option<Arc<Gen3Pass>>,
    pub(crate) lit: Arc<Gen3Pass>,
    /// Halo 4's `opaque_alpha_blend` draws twice: `lit` with the opaque
    /// objects, then this, blended among the transparents.
    pub(crate) fringe: Option<Arc<Gen3Pass>>,
    /// ODST's maps bound to the picture itself, by extern mode.
    pub(crate) screen_maps: Vec<String>,
    /// A decal family: `lit` is its `default`, drawn into the albedo target
    /// unless its `render_pass` puts it after the lights.
    pub(crate) decal: bool,
    /// A Reach decal whose `specular` is `modulate`: only those write the
    /// albedo target's alpha, which holds the specular mask.
    pub(crate) decal_modulates_specular: bool,
    pub(crate) after_lighting: bool,
    /// `sort layer`, for what blends.
    pub(crate) sort_layer: i32,
    /// `static_prt_ambient`, `_linear` and `_quadratic`.
    pub(crate) prt: [Option<Arc<Gen3Pass>>; 3],
    pub(crate) camo: Option<Arc<Gen3Pass>>,
    /// Reach's `shadow_generate`, for its own shadow map.
    pub(crate) shadow_generate: Option<Arc<Gen3Pass>>,
    pub(crate) dynamic_light: Option<Arc<Gen3Pass>>,
    pub(crate) blend: Gen3Blend,
    pub(crate) values: Gen3Values,
    pub(crate) textures: Vec<Gen3Texture>,
    /// Uniforms holding the object's change colors, and which channel.
    pub(crate) change_colors: Vec<(String, usize)>,
    pub(crate) animations: Vec<Gen3Animation>,
    pub(crate) shield: Option<Arc<Gen3Shield>>,
}

/// A material's dialect, with what only Halo 4's carry: its per-draw
/// functions and hologram rig.
#[derive(Debug)]
pub(crate) enum Gen3MaterialDialect {
    Halo3,
    Reach,
    Halo4(Arc<super::super::super::h4::materials::submit::H4Submit>),
}

impl From<RenderMethodDialect> for Gen3MaterialDialect {
    fn from(dialect: RenderMethodDialect) -> Self {
        match dialect {
            RenderMethodDialect::Halo3 => Self::Halo3,
            RenderMethodDialect::Reach => Self::Reach,
        }
    }
}

impl Gen3Material {
    /// Whose frame lights it.
    pub(crate) fn dialect(&self) -> Gen3Dialect {
        match self.dialect {
            Gen3MaterialDialect::Halo3 => Gen3Dialect::Halo3,
            Gen3MaterialDialect::Reach => Gen3Dialect::Reach,
            Gen3MaterialDialect::Halo4(_) => Gen3Dialect::Halo4,
        }
    }

    /// Halo 4's per-draw functions and hologram rig.
    pub(crate) fn h4(&self) -> Option<&Arc<super::super::super::h4::materials::submit::H4Submit>> {
        match &self.dialect {
            Gen3MaterialDialect::Halo4(submit) => Some(submit),
            Gen3MaterialDialect::Halo3 | Gen3MaterialDialect::Reach => None,
        }
    }

    pub(crate) fn h4_mut(
        &mut self,
    ) -> Option<&mut Arc<super::super::super::h4::materials::submit::H4Submit>> {
        match &mut self.dialect {
            Gen3MaterialDialect::Halo4(submit) => Some(submit),
            Gen3MaterialDialect::Halo3 | Gen3MaterialDialect::Reach => None,
        }
    }
}

/// Uniform values by name. Ordered only so the loader's snapshot prints them
/// the same every run; everything here looks them up.
pub(crate) type Gen3Values = std::collections::BTreeMap<String, Gen3Value>;

/// `shield_impact.hlsl` and the settings it runs on.
#[derive(Debug)]
pub(crate) struct Gen3Shield {
    pub(crate) pass: Arc<Gen3Pass>,
    /// ODST's `shield_impact_procedural_uv`, for settings that ask for it.
    pub(crate) procedural: Option<Arc<Gen3Pass>>,
    pub(crate) settings: Arc<Gen3ShieldSettings>,
}

/// A `shield_impact` tag, as constants and its two noise maps by uniform suffix.
#[derive(Debug)]
pub(crate) struct Gen3ShieldSettings {
    pub(crate) procedural: bool,
    pub(crate) values: Gen3Values,
    pub(crate) textures: Vec<Gen3Texture>,
}

#[cfg(test)]
mod texel_tests {
    use super::super::*;
    use blam_tags::bitmap::BitmapFormat;

    fn animation(input: &str, range: &str) -> Gen3Animation {
        Gen3Animation {
            uniform: String::new(),
            writes: Gen3Animated::First,
            function: blam_tags::TagFunction::parse(&[0; 32]).unwrap(),
            input: input.to_owned(),
            range: range.to_owned(),
            period: 1.0,
            clock: Gen3Clock::Halo3,
        }
    }

    #[test]
    fn an_empty_name_beside_a_named_one_reads_full() {
        let values = FunctionValues::from([("power".to_owned(), 0.25), ("heat".to_owned(), 0.5)]);
        let values = Some(&values);
        assert_eq!(animation("power", "").sample(0.0, 0, values), (0.25, 1.0));
        assert_eq!(animation("", "heat").sample(0.0, 0, values), (1.0, 0.5));
        assert_eq!(
            animation("power", "heat").sample(0.0, 0, values),
            (0.25, 0.5)
        );
        assert_eq!(animation("missing", "").sample(0.0, 0, values), (0.0, 1.0));
    }

    #[test]
    fn render_time_is_the_clock_only_in_odst_screen_shaders() {
        let values = FunctionValues::new();
        let mut timed = animation("render_time", "");
        assert_eq!(
            timed.sample(0.25, 0, Some(&values)),
            (0.0, 1.0),
            "an object function, and none is set"
        );
        timed.clock = Gen3Clock::Odst;
        assert_eq!(timed.sample(0.25, 0, Some(&values)), (0.0, 1.0));
        timed.clock = Gen3Clock::OdstScreen;
        assert_eq!(timed.sample(0.25, 0, Some(&values)), (0.25, 0.0));
    }

    #[test]
    fn unique_id_runs_each_object_out_of_step() {
        let mut unique = animation("unique_id", "");
        unique.period = 4.0;
        assert_eq!(unique.sample(1.0, 0, None), (0.25, 0.0));
        assert_eq!(
            unique.sample(1.0, 2, None),
            (0.75, 0.0),
            "Halo 3: time plus the object's index"
        );
        unique.clock = Gen3Clock::Odst;
        assert_eq!(unique.sample(1.0, 2, None).0, (1.0 + 2.0 * 1.37317) / 4.0);
    }

    #[test]
    fn reachs_unique_id_holds_still_unless_the_function_is_periodic() {
        let mut unique = animation("unique_id", "");
        unique.period = 4.0;
        unique.clock = Gen3Clock::Reach;
        let held = (2.0f32 * 1.37317).rem_euclid(1.0);
        assert_eq!(unique.sample(1.0, 2, None), (held, 0.0));
        assert_eq!(unique.sample(3.0, 2, None), (held, 0.0), "no time in it");
        unique.clock = Gen3Clock::Odst;
        assert_ne!(
            unique.sample(1.0, 2, None),
            unique.sample(3.0, 2, None),
            "ODST's runs on"
        );
    }

    #[test]
    fn nothing_named_runs_on_time() {
        assert_eq!(animation("", "").sample(0.25, 0, None), (0.25, 0.0));
    }

    #[test]
    fn the_dxgi_tables_signed_formats_read_signed_whatever_the_curve() {
        for format in [
            BitmapFormat::DxnSnorm,
            BitmapFormat::V8u8,
            BitmapFormat::Q8w8v8u8,
            BitmapFormat::V16u16,
        ] {
            assert_eq!(ImageTexels::of(format, false), ImageTexels::Signed);
            assert_eq!(ImageTexels::of(format, true), ImageTexels::Signed);
        }
    }

    #[test]
    fn unsigned_formats_follow_the_curve() {
        assert_eq!(
            ImageTexels::of(BitmapFormat::A8r8g8b8, true),
            ImageTexels::Srgb
        );
        assert_eq!(
            ImageTexels::of(BitmapFormat::A8r8g8b8, false),
            ImageTexels::Linear
        );
        assert_eq!(
            ImageTexels::of(BitmapFormat::Dxn, false),
            ImageTexels::Linear
        );
    }

    #[test]
    fn halo4_reads_srgb_only_through_formats_with_an_srgb_twin() {
        assert_eq!(
            ImageTexels::of_halo4(BitmapFormat::Dxt1, 5),
            ImageTexels::Srgb
        );
        assert_eq!(
            ImageTexels::of_halo4(BitmapFormat::A8r8g8b8, 0),
            ImageTexels::Srgb,
            "unknown curve, colour format"
        );
        // Halo 3's rule would read these sRGB; Halo 4 has no sRGB view for them.
        assert_eq!(
            ImageTexels::of_halo4(BitmapFormat::Dxt5a, 5),
            ImageTexels::Linear
        );
        assert_eq!(
            ImageTexels::of_halo4(BitmapFormat::A4r4g4b4, 1),
            ImageTexels::Linear
        );
        assert_eq!(
            ImageTexels::of_halo4(BitmapFormat::Dxt5, 2),
            ImageTexels::Linear,
            "gamma 2.0 is never sRGB"
        );
        assert_eq!(
            ImageTexels::of_halo4(BitmapFormat::Dxt5, 3),
            ImageTexels::Linear,
            "nor linear"
        );
        assert_eq!(
            ImageTexels::of_halo4(BitmapFormat::Dxn, 0),
            ImageTexels::Signed,
            "dxn is BC5_SNORM"
        );
        assert_eq!(
            ImageTexels::of(BitmapFormat::Dxn, false),
            ImageTexels::Linear,
            "Halo 3 reads its `dxn` differently"
        );
    }

    #[test]
    fn an_unknown_curve_falls_back_on_the_format() {
        assert!(ImageTexels::default_srgb(BitmapFormat::Dxt1));
        assert!(ImageTexels::default_srgb(BitmapFormat::A8r8g8b8));
        assert!(!ImageTexels::default_srgb(BitmapFormat::DxnSnorm));
        assert!(!ImageTexels::default_srgb(BitmapFormat::Abgrfp16));
    }
}
