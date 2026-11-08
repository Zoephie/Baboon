//! ODST's VISR: `c_hud_camera_view`'s outline over the finished frame, its
//! depth's edges tinted by what each object is to the player, with a ping that
//! sweeps out from the eye.

use blam_tags::{TagFile, TagStruct};

use super::super::common::named_function::tag_function_at;
use super::super::gen3::materials::{H3Bitmap, read_h3_bitmap};
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;

/// `global_z_near` and `global_z_far`, which the outline's depth is measured by.
pub(crate) const GAME_NEAR: f32 = 0.0078125;
pub(crate) const GAME_FAR: f32 = 10240.0;

/// The stencil's class of a fragment with no object over it, which the view
/// clears to 254 and `(raw + 32) >> 6` takes to the last class.
const NOTHING_CLASS: usize = 4;

/// `hud_camera_view`'s table by object type, and `game_team_is_friend` against
/// the player. A preview has no scenario, so no team but the player's own is
/// anyone's ally or enemy, and what has no `default team` of its own is on none.
/// A creature is no unit: ODST's `creature_new` (tag_test 0x140AE8DF0) gives
/// it the `default team` beside its own `flags`, which `object_get_team` reads.
fn vision_class(object: &TagFile) -> usize {
    let root = object.root();
    let team = match &object.header.group_tag.to_be_bytes() {
        b"bipd" | b"gint" => root.descend("unit").and_then(|unit| unit.field("default team")),
        b"crea" => root.field("default team"),
        b"vehi" | b"weap" | b"eqip" | b"mach" | b"ctrl" => return 0,
        b"argd" | b"term" => return 3,
        _ => return NOTHING_CLASS,
    };
    usize::from(team.is_some_and(|team| team.enum_is("player")))
}

/// `vision_mode`'s values for what the preview draws: `on_foot`, one class.
#[derive(Debug)]
pub(crate) struct H3Vision {
    radius: f32,
    speed: f32,
    width: f32,
    delay: f32,
    color: [f32; 3],
    ping_color: [f32; 3],
    alpha: f32,
    intensity: f32,
    ping_intensity: f32,
    dimming: f32,
    mask: Option<H3Bitmap>,
}

/// What `sub_14059C370` hands the shader.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H3VisionConstants {
    pub(crate) falloff: f32,
    pub(crate) ping: [f32; 3],
    pub(crate) color: [f32; 4],
    pub(crate) ping_color: [f32; 4],
}

impl H3Vision {
    pub(crate) fn mask(&self) -> Option<&H3Bitmap> {
        self.mask.as_ref()
    }

    /// `(radius + width) / speed + delay`: one sweep and its pause, in seconds.
    fn period(&self) -> f32 {
        ((self.radius + self.width + self.delay * self.speed) / self.speed).max(0.01)
    }

    /// `seconds` is how long it has been on.
    pub(crate) fn constants(&self, amount: f32, seconds: f32) -> H3VisionConstants {
        // The first sweep comes up from nothing; after that it only wraps.
        let turns = seconds / self.period();
        let turns = if turns < 1.0 { turns } else { 1.0 + turns.fract() };
        let span = self.speed * self.delay + self.width + self.radius;
        let reached = span * turns;
        let tinted = |color: [f32; 3], intensity: f32| color.map(|channel| channel * amount * intensity);
        let [red, green, blue] = tinted(self.color, self.intensity);
        let [ping_red, ping_green, ping_blue] = tinted(self.ping_color, self.ping_intensity);
        H3VisionConstants {
            falloff: 8f32.powf(0.125) / self.radius,
            ping: [reached - (reached / span).floor() * span, reached.min(self.radius), -1.0 / self.width],
            color: [red, green, blue, amount * self.alpha],
            ping_color: [ping_red, ping_green, ping_blue, self.dimming],
        }
    }
}

fn color_of(tag: TagStruct<'_>, name: &str) -> Option<[f32; 3]> {
    let holder = tag.descend(name)?;
    let color = tag_function_at(&holder, "Mapping")?.evaluate_color(0.0, 0.0);
    Some([color.red, color.green, color.blue])
}

/// `rasterizer_globals`' `vision_mode`, for an object of `model_tag`.
pub(in crate::app::model_preview) fn read_h3_vision(reader: &TagReader, object: &TagFile) -> Option<H3Vision> {
    let globals =
        reader.load("globals\\rasterizer_globals", "rasterizer_globals", b"rasg").ok()?;
    let path = globals.root().read_tag_ref_path("Vision Mode Settings").filter(|path| !path.is_empty())?;
    let tag = reader.load(&path, "vision_mode", b"vmdx").ok()?;
    let root = tag.root();
    let class = vision_class(object);
    let real = |name: String| root.read_real(&name).unwrap_or(0.0);
    let mask = root.read_tag_ref_path("mask bitmap").filter(|path| !path.is_empty())?;
    Some(H3Vision {
        radius: real("on_foot_radius".into()),
        speed: real("on_foot ping speed".into()),
        width: real("on_foot ping width".into()),
        delay: real("on_foot ping delay".into()),
        color: color_of(root, &format!("default color {class}"))?,
        ping_color: color_of(root, &format!("ping color {class}"))?,
        alpha: real(format!("alpha {class}")),
        intensity: real(format!("intensity {class}")),
        ping_intensity: real(format!("ping intensity {class}")),
        dimming: real(format!("overlapping dimming factor {class}")),
        mask: read_h3_bitmap(reader, &mask, 0, SamplerKind::Texture2D),
    })
}

/// `rasterizer_globals`' motion blur as `displacement_motion_blur` wants it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H3MotionBlur {
    /// `pixel_blur_constants.y`: the longest smear, in pixels.
    pub(crate) most_pixels: f32,
    /// `pixel_blur_constants.z`: one sample's step along it.
    pub(crate) step: f32,
    /// `center falloff`, as squared in `crosshair_constants`.
    pub(crate) center_falloff: [f32; 2],
}

impl H3MotionBlur {
    fn new(max_blur: f32, blur_scale: f32, center_falloff: [f32; 2]) -> Self {
        let doubled = blur_scale * 2.0;
        Self { most_pixels: max_blur * 1792.0 * 0.25 / doubled, step: doubled / 6.0, center_falloff }
    }
}

pub(in crate::app::model_preview) fn read_h3_motion_blur(reader: &TagReader) -> Option<H3MotionBlur> {
    let globals =
        reader.load("globals\\rasterizer_globals", "rasterizer_globals", b"rasg").ok()?;
    let root = globals.root();
    let real = |name: &str| root.read_real(name);
    Some(H3MotionBlur::new(
        real("max blur")?,
        real("blur scale")?,
        [real("center falloff x")?, real("center falloff y")?],
    ))
}

/// A column matrix's inverse, for `screen_to_world`; a singular one is none.
pub(crate) fn inverted(matrix: &[f32; 16]) -> Option<[f32; 16]> {
    let mut left: [[f64; 4]; 4] = std::array::from_fn(|row| std::array::from_fn(|column| matrix[column * 4 + row] as f64));
    let mut right: [[f64; 4]; 4] =
        std::array::from_fn(|row| std::array::from_fn(|column| f64::from(u8::from(row == column))));
    for at in 0..4 {
        let pivot = (at..4).max_by(|a, b| left[*a][at].abs().total_cmp(&left[*b][at].abs()))?;
        if left[pivot][at] == 0.0 {
            return None;
        }
        left.swap(at, pivot);
        right.swap(at, pivot);
        let scale = left[at][at].recip();
        left[at] = left[at].map(|value| value * scale);
        right[at] = right[at].map(|value| value * scale);
        for row in (0..4).filter(|row| *row != at) {
            let by = left[row][at];
            left[row] = std::array::from_fn(|column| left[row][column] - left[at][column] * by);
            right[row] = std::array::from_fn(|column| right[row][column] - right[at][column] * by);
        }
    }
    Some(std::array::from_fn(|index| right[index % 4][index / 4] as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vision() -> H3Vision {
        H3Vision {
            radius: 30.0,
            speed: 10.0,
            width: 30.0,
            delay: 0.0,
            color: [0.5, 0.25, 1.0],
            ping_color: [1.0, 0.0, 0.5],
            alpha: 0.5,
            intensity: 20.0,
            ping_intensity: 10.0,
            dimming: 0.05,
            mask: None,
        }
    }

    /// A creature's team is its own `default team`, not a unit's: ODST's
    /// glowbug is on the player's, its lod banshee the covenant's.
    #[test]
    fn a_creature_is_classed_by_its_own_default_team() {
        let Ok(root) = std::env::var("BLAM_TEST_ODSTEK") else {
            eprintln!("skipping: BLAM_TEST_ODSTEK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::Halo3Odst),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let class = |path: &str| {
            let tag = load_referenced_tag_from_source(&source, path, "creature", b"crea").expect(path);
            vision_class(&tag)
        };
        assert_eq!(class("objects\\levels\\multi\\guardian\\glowbug\\glowbug"), 1);
        assert_eq!(class("objects\\characters\\ambient_life\\lod_banshee\\lod_banshee"), 0);
    }

    #[test]
    fn motion_blur_takes_the_shipped_globals_to_fifty_pixels_in_six_steps() {
        let blur = H3MotionBlur::new(0.08, 0.35, [7.0, 7.0]);
        assert!((blur.most_pixels - 51.2).abs() < 1e-3, "{}", blur.most_pixels);
        assert!((blur.step - 0.7 / 6.0).abs() < 1e-6);
        assert_ne!(H3MotionBlur::new(0.08, 0.7, [7.0, 7.0]).most_pixels, blur.most_pixels);
    }

    #[test]
    fn the_first_sweep_grows_its_reach_and_later_ones_only_wrap() {
        let vision = vision();
        let early = vision.constants(1.0, 1.0);
        assert_eq!(early.ping[0], 10.0);
        assert_eq!(early.ping[1], 10.0, "nothing past the first wavefront yet");
        let later = vision.constants(1.0, 7.0);
        assert!((later.ping[0] - 10.0).abs() < 1e-3, "{}", later.ping[0]);
        assert_eq!(later.ping[1], 30.0, "the second time round it all shows");
        assert_eq!(later.ping[2], -1.0 / 30.0);
    }

    #[test]
    fn a_delay_holds_the_sweep_past_its_radius() {
        let mut held = vision();
        held.delay = 2.0;
        assert_eq!(held.period(), 8.0);
        assert_eq!(vision().period(), 6.0);
    }

    #[test]
    fn the_amount_scales_light_but_not_the_dimming() {
        let vision = vision();
        let half = vision.constants(0.5, 0.0);
        assert_eq!(half.color, [5.0, 2.5, 10.0, 0.25]);
        assert_eq!(half.ping_color, [5.0, 0.0, 2.5, 0.05]);
    }

    #[test]
    fn the_falloff_is_an_eighth_power_gone_by_the_radius() {
        let at_radius = (-(30.0 * vision().constants(1.0, 0.0).falloff).powi(8)).exp2();
        assert!((at_radius - 1.0 / 256.0).abs() < 1e-4, "{at_radius}");
    }

    #[test]
    fn an_inverse_undoes_and_a_singular_matrix_has_none() {
        let matrix = [2.0, 0.0, 0.0, 0.5, 0.0, 3.0, 1.0, 0.0, 0.0, 1.0, 4.0, 0.0, 1.0, 2.0, 3.0, 1.0];
        let back = inverted(&matrix).unwrap();
        for row in 0..4 {
            for column in 0..4 {
                let product: f32 = (0..4).map(|at| matrix[at * 4 + row] * back[column * 4 + at]).sum();
                assert!((product - f32::from(u8::from(row == column))).abs() < 1e-5);
            }
        }
        assert_eq!(inverted(&[0.0; 16]), None);
    }
}

