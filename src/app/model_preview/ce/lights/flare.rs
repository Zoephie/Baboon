//! A light's lens flare and how it is read.

use super::*;

/// A `lens_flare` tag.
#[derive(Debug, Clone)]
pub(crate) struct CeFlare {
    /// `runtime cosine falloff angle` and `cutoff`.
    pub(super) cones: [f32; 2],
    pub(super) fade: [f32; 2],
    pub(super) corona_scale: [f32; 2],
    /// `rotation function`, and its scale in radians.
    pub(super) rotation_function: i128,
    pub(super) rotation_scale: f32,
    /// How big a patch `rasterizer_lens_flares_submit_occlusion_tests` tests.
    pub(crate) occlusion_radius: f32,
    pub(super) occlusion_offset: OcclusionOffset,
    pub(super) reflections: Vec<CeReflection>,
    pub(crate) images: Vec<Arc<Image>>,
}

pub(super) fn read_flare(reader: &TagReader, path: &str) -> Option<CeFlare> {
    let tag = reader.load(path, "lens_flare", b"lens").ok()?;
    let root = tag.root();
    let bitmap = root
        .read_tag_ref_path("bitmap")
        .filter(|bitmap| !bitmap.is_empty())?;
    let mut images = Vec::new();
    let mut indices = Vec::new();
    let mut reflections = Vec::new();
    for reflection in elements(root, "reflections") {
        let index = reflection.read_int_any("bitmap index").unwrap_or(0).max(0) as usize;
        let image = indices
            .iter()
            .position(|known| *known == index)
            .or_else(|| {
                let image = decode_image(reader, &bitmap, SamplerKind::Texture2D, index)?;
                images.push(Arc::new(image));
                indices.push(index);
                Some(images.len() - 1)
            })?;
        let radius = reflection.read_real_bounds("radius");
        let brightness = reflection.read_fraction_bounds("brightness");
        reflections.push(CeReflection {
            align_with_screen_center: has_flag(
                reflection,
                "flags",
                "align rotation with screen center",
            ),
            radius_not_scaled_by_distance: has_flag(
                reflection,
                "flags",
                "radius NOT scaled by distance",
            ),
            radius_scaled_by_occlusion: has_flag(
                reflection,
                "flags",
                "radius scaled by occlusion factor",
            ),
            image,
            position: reflection.read_real("position").unwrap_or(0.0),
            rotation: reflection.read_real("rotation offset").unwrap_or(0.0),
            radius: [radius.lower, radius.upper],
            brightness: [brightness.lower, brightness.upper],
            brightness_scaled_by: reflection
                .read_int_any("brightness scaled by")
                .unwrap_or(0)
                .max(0) as usize,
            tint: argb(reflection, "tint color"),
            lower: argb(reflection, "color lower bound"),
            upper: argb(reflection, "color upper bound"),
            // Its second `flags` field holds these.
            interpolation: ce_color_interpolation(
                has_flag_in_any(reflection, "flags", "interpolate color in hsv"),
                has_flag_in_any(reflection, "flags", "...more colors"),
            ),
            animation_function: reflection
                .read_int_any("animation function")
                .unwrap_or(0)
                .max(0) as usize,
            animation_period: reflection.read_real("animation period").unwrap_or(0.0),
            animation_phase: reflection.read_real("animation phase").unwrap_or(0.0),
        });
    }
    let angle = |name: &str| root.read_real(name).unwrap_or(0.0).cos();
    Some(CeFlare {
        cones: [angle("falloff angle"), angle("cutoff angle")],
        fade: [
            root.read_real("near fade distance").unwrap_or(0.0),
            root.read_real("far fade distance").unwrap_or(0.0),
        ],
        corona_scale: [
            root.read_real("horizontal scale")
                .filter(|scale| *scale != 0.0)
                .unwrap_or(1.0),
            root.read_real("vertical scale")
                .filter(|scale| *scale != 0.0)
                .unwrap_or(1.0),
        ],
        rotation_function: root.read_int_any("rotation function").unwrap_or(0),
        // Tool's postprocess takes nothing, or a whole turn in degrees, for one.
        rotation_scale: match root.read_real("rotation function scale").unwrap_or(0.0) {
            scale if scale == 0.0 || scale == 360.0 => std::f32::consts::TAU,
            scale => scale,
        },
        occlusion_radius: root.read_real("occlusion radius").unwrap_or(0.0),
        occlusion_offset: OcclusionOffset::read(root),
        reflections,
        images,
    })
}
