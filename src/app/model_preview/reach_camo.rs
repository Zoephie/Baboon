//! Reach's active camouflage: `globals`' curves, and the constant and blend
//! `c_render_active_camo::render_object_callback` draws an object with. See
//! `docs/reach-preview-research/shield.md`, part B.



use blam_tags::TagFunction;

use super::dx9::passes::tag_function_at;
use super::h3_shaders::H3Pass;
use super::renderer::REACH_DARK_MULTIPLIER;
use crate::app::export::geometry::load_referenced_tag_from_source;
use crate::core::source::TagSource;

/// `active camo[0]`'s third person curves.
#[derive(Debug)]
pub(crate) struct ReachCamo {
    distortion: TagFunction,
    transparency: TagFunction,
}

/// How one object's camo draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReachCamoDraw {
    /// `k_ps_active_camo_factor` and `k_vs_active_camo_factor`.
    pub(crate) factor: [f32; 4],
    /// Opaque, depth written, with no lit base under it.
    pub(crate) opaque: bool,
}

impl ReachCamo {
    pub(crate) fn read(source: &TagSource) -> Option<Self> {
        let globals = load_referenced_tag_from_source(source, "globals\\globals", "globals", b"matg").ok()?;
        let root = globals.root();
        let camo = super::ce::attachments::elements(root, "active camo").into_iter().next()?;
        let curve = |name: &str| tag_function_at(&camo.descend(name)?, "function");
        Some(Self { distortion: curve("camo value to distortion")?, transparency: curve("camo value to transparency")? })
    }

    /// The draw at camo `amount` (the unit's 0 to 1, a byte in the game):
    /// `t = pin(1.25a - 0.25)` through both curves. Distortion is scaled to
    /// the 1152 by 720 screen the shader's offsets assume, and its Y turned
    /// over for a picture that runs up.
    pub(crate) fn draw(&self, amount: f32) -> ReachCamoDraw {
        let t = curve_input(amount);
        draw_of(self.distortion.evaluate(t, 0.0), self.transparency.evaluate(t, 0.0))
    }
}

/// `render_object_callback`'s curve input: the camo byte, past its first
/// fifth.
fn curve_input(amount: f32) -> f32 {
    let amount = (amount.clamp(0.0, 1.0) * 255.0).trunc() / 255.0;
    (1.25 * amount - 0.25).clamp(0.0, 1.0)
}

fn draw_of(distortion: f32, transparency: f32) -> ReachCamoDraw {
    ReachCamoDraw {
        factor: [transparency.clamp(0.0, 1.0), distortion / 64.0, -distortion / 36.0, 0.0],
        opaque: transparency >= 1.0,
    }
}

/// The camo pass, made to write what it samples back as the light it was.
/// The game copies its HDR target and writes it straight back; the preview's
/// copy is of the finished picture, which the composite took the root of.
pub(crate) fn camo_pass(pass: &H3Pass) -> Option<H3Pass> {
    const RETURN: &str = "return vec4(l_color.rgb, g_k_ps_active_camo_factor.x);";
    if !pass.pixel.contains(RETURN) {
        return None;
    }
    let pixel = pass.pixel.replace(
        RETURN,
        &format!(
            "vec3 l_camo_encoded = mix(l_color.rgb * 12.92, 1.055 * pow(l_color.rgb, vec3(1.0 / 2.4)) - 0.055, step(vec3(0.0031308), l_color.rgb));
return vec4(l_camo_encoded * l_camo_encoded / {REACH_DARK_MULTIPLIER:?}, g_k_ps_active_camo_factor.x);"
        ),
    );
    Some(H3Pass {
        fullscreen: pass.fullscreen,
        vertex: pass.vertex.clone(),
        pixel,
        uniforms: pass.uniforms.clone(),
        textures: pass.textures.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camo_below_a_fifth_reads_the_curves_at_zero() {
        assert_eq!(curve_input(0.2), 0.0);
        assert_eq!(curve_input(1.0), 1.0);
        let half = curve_input(0.6);
        assert!((half - (1.25 * (153.0 / 255.0) - 0.25)).abs() < 1e-6, "{half}");
    }

    #[test]
    fn only_full_transparency_draws_opaque() {
        let full = draw_of(3.0, 1.0);
        assert!(full.opaque);
        assert!((full.factor[1] - 3.0 / 64.0).abs() < 1e-6);
        assert!((full.factor[2] + 3.0 / 36.0).abs() < 1e-6, "Y turned over");
        let part = draw_of(3.0, 0.5);
        assert!(!part.opaque);
        assert_eq!(part.factor[0], 0.5);
    }

    /// With `BLAM_TEST_HREK`: the kit's curves read, and full camo draws.
    #[test]
    fn the_kits_camo_curves_read() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloReach),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let camo = ReachCamo::read(&source).expect("globals' active camo");
        let full = camo.draw(1.0);
        eprintln!("NOTE full camo {full:?}, half {:?}", camo.draw(0.5));
        assert!(full.factor[1] > 0.0, "it distorts");
    }
}
