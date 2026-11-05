//! Halo 4's active camouflage: `globals`' `active camo[0]`, and the constants
//! `c_render_active_camo::render_object_callback` (X360 0x83B65270, MCC
//! 0x140914160) hands each material's `active_camo` entry point. See
//! `docs/h4-preview-research/shadow-camo-shields.md` §2.

use blam_tags::{TagFieldData, TagFunction};

use super::h2_passes::tag_function_at;
use crate::app::export::geometry::load_referenced_tag_from_source;
use crate::core::source::TagSource;

/// `active camo[0]`'s third person curves and its distortion texture's
/// placement.
#[derive(Debug)]
pub(crate) struct H4Camo {
    distortion: TagFunction,
    transparency: TagFunction,
    texture_strength: TagFunction,
    scale: [f32; 2],
    speed: [f32; 2],
    /// `camo distortion texture`.
    pub(crate) texture: Option<String>,
}

/// One object's camo constants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H4CamoDraw {
    /// `ps_activeCamoFactor`: transparency, the screen distortion twice, the
    /// texture's strength.
    pub(crate) factor: [f32; 4],
    /// `ps_distortionTextureTransform`.
    pub(crate) transform: [f32; 4],
    /// `distortionTextureEnabled`.
    pub(crate) textured: bool,
    /// Under full transparency there is no lit base.
    pub(crate) has_base: bool,
}

fn read_vector(holder: blam_tags::TagStruct<'_>, name: &str) -> Option<[f32; 2]> {
    match holder.field(name)?.value()? {
        TagFieldData::RealVector2d(vector) => Some([vector.i, vector.j]),
        TagFieldData::RealPoint2d(point) => Some([point.x, point.y]),
        _ => None,
    }
}

impl H4Camo {
    pub(crate) fn read(source: &TagSource) -> Option<Self> {
        let globals =
            load_referenced_tag_from_source(source, "globals\\globals", "globals", b"matg").ok()?;
        let root = globals.root();
        let camo = super::ce_attachments::elements(root, "active camo")
            .into_iter()
            .next()?;
        let curve = |name: &str| tag_function_at(&camo.descend(name)?, "function");
        Some(Self {
            distortion: curve("camo value to distortion")?,
            transparency: curve("camo value to transparency")?,
            texture_strength: curve("camo distortion texture strength")?,
            scale: read_vector(camo, "camo distortion scale").unwrap_or([1.0; 2]),
            speed: read_vector(camo, "camo distortion translate speed").unwrap_or([0.0; 2]),
            texture: camo
                .read_tag_ref_path("camo distortion texture")
                .filter(|path| !path.is_empty()),
        })
    }

    /// The draw at camo `amount` (the unit's 0 to 1, a byte in the game),
    /// straight through the curves: Halo 4 drops Reach's `1.25a - 0.25`. The
    /// distortion's Y is turned over for a picture that runs up. `time` is
    /// `s_render_time`'s seconds, which scroll the texture.
    pub(crate) fn draw(&self, amount: f32, time: f32) -> H4CamoDraw {
        let input = ((amount.clamp(0.0, 1.0) * 255.0).trunc() / 255.0).clamp(0.0, 1.0);
        let distortion = self.distortion.evaluate(input, 0.0);
        let transparency = self.transparency.evaluate(input, 0.0).clamp(0.0, 1.0);
        let strength = self.texture_strength.evaluate(input, 0.0);
        H4CamoDraw {
            factor: [transparency, distortion, -distortion, strength],
            transform: [
                self.scale[0],
                self.scale[1],
                self.speed[0] * time,
                self.speed[1] * time,
            ],
            textured: strength * distortion > 1e-4 && self.texture.is_some(),
            has_base: transparency < 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kit's curves at full camo (`shadow-camo-shields.md` §2.2): fully
    /// transparent, distortion 0.8914, texture strength 0.2971, the texture on
    /// and scrolling at 0.01 a second; half camo still has its lit base.
    #[test]
    fn the_kits_full_camo_reads_its_curves_straight() {
        let root = std::path::PathBuf::from(crate::core::test_kits::tag_path("halo4_mcc", ""));
        if !root.join("globals").is_dir() {
            eprintln!("skipped: set BLAM_TEST_H4EK to a Halo 4 kit's tags folder");
            return;
        }
        let source = TagSource::LooseFolder {
            root,
            game: Some(crate::core::game::GameId::Halo4),
            definitions_root: crate::core::test_kits::definitions().to_path_buf(),
        };
        let camo = H4Camo::read(&source).expect("globals' active camo");
        let full = camo.draw(1.0, 100.0);
        assert_eq!(full.factor[0], 1.0);
        assert!((full.factor[1] - 0.8914).abs() < 1e-3, "{full:?}");
        assert_eq!(full.factor[2], -full.factor[1], "Y turned over");
        assert!((full.factor[3] - 0.2971).abs() < 1e-3, "{full:?}");
        assert!(full.textured && !full.has_base);
        assert!((full.transform[2] - 1.0).abs() < 1e-5, "{full:?}");
        // No remap: a tenth reads the curve at a tenth, not at zero.
        let tenth = camo.draw(0.1, 0.0);
        assert!(tenth.factor[0] > 0.3 && tenth.has_base, "{tenth:?}");
        assert_eq!(camo.draw(0.0, 0.0).factor[0], 0.0);
    }
}
