//! A gen3 material's shield settings.

use super::super::super::{ResolveCaches, TagReader, decode_image};
use super::{
    Gen3Shield, Gen3ShieldSettings, Gen3Sources, Gen3Texture, Gen3Value, ImageTexels, SamplerKind,
    has_flag,
};
use std::sync::Arc;

/// A `shield_impact` tag's constants and noise, as `render_object_context`
/// submits them.
pub(in crate::app::model_preview) fn read_shield_settings(
    reader: &TagReader,
    path: &str,
) -> Option<Arc<Gen3ShieldSettings>> {
    let tag = reader.load(path, "shield_impact", b"shit").ok()?;
    let root = tag.root();
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let tinted = |color: &str, intensity: &str| {
        let (color, intensity) = (root.read_rgb(color), real(intensity));
        Gen3Value::Floats([
            color.red * intensity,
            color.green * intensity,
            color.blue * intensity,
            0.0,
        ])
    };
    let mut values = super::super::super::gen3::shaders::Gen3Values::new();
    let mut set = |name: &str, value: Gen3Value| {
        values.insert(name.to_owned(), value);
    };
    set(
        "extrusion_distance",
        Gen3Value::Floats([real("Extrusion Distance"); 4]),
    );
    // `render_object_shield_impact`: z is ODST's, for the second noise.
    let secondary = real(
        if has_flag(root, "flags", "use secondary (non-uniform) texture scale") {
            "Secondary texture scale"
        } else {
            "Texture Scale"
        },
    );
    set(
        "texture_quantities",
        Gen3Value::Floats([real("Texture Scale"), real("Scroll Speed"), secondary, 0.0]),
    );
    set(
        "shield_impact_edge_fade",
        Gen3Value::Floats([real("Edge Fade"); 4]),
    );
    for (uniform, sharpness, scale, threshold) in [
        (
            "plasma1_settings",
            "Plasma Sharpness 1",
            "Plasma Scale 1",
            "Plasma Threshold 1",
        ),
        (
            "plasma2_settings",
            "Plasma Sharpness 2",
            "Plasma Scale 2",
            "Plasma Threshold 2",
        ),
    ] {
        set(
            uniform,
            Gen3Value::Floats([real(sharpness), real(scale), real(threshold), 0.0]),
        );
    }
    set(
        "overshield_color1",
        tinted("Overshield Color 1", "Overshield Intensity 1"),
    );
    set(
        "overshield_color2",
        tinted("Overshield Color 2", "Overshield Intensity 2"),
    );
    set(
        "overshield_ambient_color",
        tinted("Overshield Ambient Color", "Overshield Ambient Intensity"),
    );
    set(
        "shield_impact_color1",
        tinted("Impact Color 1", "Impact Intensity 1"),
    );
    set(
        "shield_impact_color2",
        tinted("Impact Color 2", "Impact Intensity 2"),
    );
    set(
        "shield_impact_ambient_color",
        tinted("Impact Ambient Color", "Impact Ambient Intensity 2"),
    );
    let mut textures = Vec::new();
    for (field, uniform) in [
        (
            "Shield Impact Noise Texture 1",
            "shield_impact_noise_texture1",
        ),
        (
            "Shield Impact Noise Texture 2",
            "shield_impact_noise_texture2",
        ),
    ] {
        let bitmap = root
            .read_tag_ref_path(field)
            .filter(|bitmap| !bitmap.is_empty())?;
        let image = Arc::new(decode_image(reader, &bitmap, SamplerKind::Texture2D, 0)?);
        textures.push(Gen3Texture {
            uniform: uniform.to_owned(),
            image,
            texels: ImageTexels::Linear,
            mips: Arc::default(),
            sampler: None,
            frames: Vec::new(),
        });
    }
    Some(Arc::new(Gen3ShieldSettings {
        procedural: has_flag(root, "flags", "use procedural uv generation"),
        values,
        textures,
    }))
}

/// The pass, on the global settings a model that names none of its own uses.
pub(super) fn gen3_shield(
    reader: &TagReader,
    caches: &mut ResolveCaches,
) -> Option<Arc<Gen3Shield>> {
    if let Some(found) = &caches.gen3_shield {
        return found.clone();
    }
    let settings = read_shield_settings(reader, "globals\\global_shield_impact_settings");
    let sources = caches
        .h3_sources
        .get_or_insert_with(|| Gen3Sources::new(&reader.source));
    let shield = settings.and_then(|settings| {
        let pass = sources.explicit_pass("shield_impact").ok()?;
        let procedural = sources.explicit_pass("shield_impact_procedural_uv").ok();
        Some(Arc::new(Gen3Shield {
            pass,
            procedural,
            settings,
        }))
    });
    caches.gen3_shield = Some(shield.clone());
    shield
}
