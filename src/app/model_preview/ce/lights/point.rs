//! Point lights, baked to the register rows the shader passes read.

use super::*;

pub(crate) use super::super::super::dx9::stage::CE_POINT_LIGHT_ROWS;

/// `lights_prepare_for_object_dynamic` for one object: the two lights strongest
/// at `center`, within `reach` of their radius.
pub(in crate::app::model_preview) fn point_light_rows(
    lights: &[CeLight],
    object: u8,
    center: [f32; 3],
    reach: f32,
    state: impl Fn(u8) -> ([f32; 4], [[f32; 3]; 4]),
    bones: Option<&[[f32; 4]]>,
) -> [[f32; 4]; CE_POINT_LIGHT_ROWS] {
    let mut found: Vec<([[f32; 4]; 3], f32)> = Vec::new();
    for light in lights {
        if light.spares_owner && light.object == object {
            continue;
        }
        let (outputs, colors) = state(light.object);
        let Some((rows, radius)) = light.point_rows(&outputs, &colors, bones) else {
            continue;
        };
        let offset: [f32; 3] = std::array::from_fn(|axis| rows[0][axis] - center[axis]);
        let distance = dot(offset, offset).sqrt();
        if distance >= radius + reach {
            continue;
        }
        let brightness = (rows[2][0] + rows[2][1] + rows[2][2]) / 3.0;
        found.push((
            rows,
            brightness * (1.0 - distance * distance / (radius * radius)),
        ));
    }
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out = [[0.0; 4]; CE_POINT_LIGHT_ROWS];
    for (index, (rows, _)) in found.iter().take(2).enumerate() {
        out[index * 3..index * 3 + 3].copy_from_slice(rows);
    }
    out
}

/// What tool's light postprocess makes of a pair of `falloff` and `cutoff`
/// angles: their cosines, or none for a light that shines all ways.
pub(super) fn baked_cone(falloff: f32, cutoff: f32) -> Option<[f32; 2]> {
    use std::f32::consts::PI;
    let (falloff, cutoff) = if cutoff < UNSET {
        (PI, PI)
    } else {
        (falloff.min(PI), cutoff.min(PI))
    };
    let falloff = if falloff > cutoff { 0.0 } else { falloff };
    (falloff < PI).then(|| [falloff.cos(), cutoff.cos()])
}

/// The same for color bounds with nothing in them, which turn white. Only the
/// color channels count; alpha stays.
pub(super) fn baked_colors(mut lower: [f32; 4], mut upper: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    if lower[..3]
        .iter()
        .chain(&upper[..3])
        .all(|channel| *channel < UNSET)
    {
        lower[..3].fill(1.0);
        upper[..3].fill(1.0);
    }
    (lower, upper)
}

/// A `light` tag as tool's postprocess leaves it, placed nowhere yet.
pub(super) fn read_light_tag(reader: &TagReader, path: &str) -> Option<CeLight> {
    let light = reader.load(path, "light", b"ligh").ok()?;
    let root = light.root();
    let flare_path = root
        .read_tag_ref_path("lens flare")
        .filter(|flare| !flare.is_empty());
    let flare = flare_path
        .and_then(|flare| read_flare(reader, &flare))
        .map(Arc::new);
    // Tags on disk lack what tool's postprocess gives them for the game.
    let modifier = root.read_real_bounds("radius modifier");
    let radius_bounds = if modifier.lower < UNSET && modifier.upper < UNSET {
        [1.0, 1.0]
    } else {
        [modifier.lower, modifier.upper]
    };
    let radius = root
        .read_real("radius")
        .filter(|radius| *radius != 0.0)
        .unwrap_or(1.0);
    let angle = |name: &str| root.read_real(name).unwrap_or(0.0);
    let (lower, upper) = baked_colors(
        argb(root, "color lower bound"),
        argb(root, "color upper bound"),
    );
    Some(CeLight {
        object: 0,
        node: 0,
        position: [0.0; 3],
        forward: [1.0, 0.0, 0.0],
        primary: None,
        change_color: None,
        lower,
        upper,
        interpolation: ce_color_interpolation(
            has_flag(root, "interpolation flags", "blend in hsv"),
            has_flag(root, "interpolation flags", "...more colors"),
        ),
        dynamic: has_flag(root, "flags", "dynamic"),
        spares_owner: has_flag(root, "flags", "don't light own object"),
        radius,
        radius_bounds,
        cone: baked_cone(angle("falloff angle"), angle("cutoff angle")),
        flare,
        fixed: None,
    })
}
