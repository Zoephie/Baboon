//! A render method's parameters: transforms, values, and the externs it binds.

use super::super::super::{Values, parameter_named};
use super::Gen3Value;
use blam_tags::render_method::{ParameterSource, RenderMethodAnimatedParameterType, ResolvedValue};

/// A map's `_xform`: its scale, then its offset. Tool's
/// `render_method_update_real_parameter` starts from identity and takes only
/// the shader's own animators; the option's default bitmap scale is not used.
pub(in crate::app::model_preview) fn gen3_xform(values: &Values<'_>, parameter: &str) -> [f32; 4] {
    // `update_real_parameter`: identity, whatever `default_bitmap_scale` says.
    let mut xform = [1.0, 1.0, 0.0, 0.0];
    let Some(found) = parameter_named(values.method, parameter) else {
        return xform;
    };
    for animated in &found.animated_parameters {
        let (Some(kind), Some(function)) = (animated.parameter_type, animated.function.as_ref())
        else {
            continue;
        };
        let value = function.evaluate(0.0, 0.0);
        match kind.get() {
            RenderMethodAnimatedParameterType::ScaleUniform => xform[..2].fill(value),
            RenderMethodAnimatedParameterType::ScaleX => xform[0] = value,
            RenderMethodAnimatedParameterType::ScaleY => xform[1] = value,
            RenderMethodAnimatedParameterType::TranslationX => xform[2] = value,
            RenderMethodAnimatedParameterType::TranslationY => xform[3] = value,
            _ => {}
        }
    }
    xform
}

/// The value a `UserParameter_` uniform holds in this shader.
pub(super) fn gen3_parameter_value(values: &Values<'_>, name: &str) -> Option<Gen3Value> {
    let resolved = values.resolved?.find(name)?;
    let ParameterSource::Inline(value) = &resolved.source else {
        return None;
    };
    Some(match value {
        ResolvedValue::Real(value) => Gen3Value::Floats([*value; 4]),
        // A constant slot as `update_constants` fills it, whatever the
        // engine's doc comment says: red in `i`, alpha in `l`.
        ResolvedValue::Color(slot) => Gen3Value::Floats(*slot),
        ResolvedValue::Int(value) => Gen3Value::Integer(*value),
        ResolvedValue::Bool(value) => Gen3Value::Integer(i32::from(*value)),
        ResolvedValue::Bitmap(_) => return None,
    })
}

/// The bitmap the engine binds for an extern texture, with no level around
/// the object: `rasterizer_globals`' material textures, and its default cube.
pub(super) fn gen3_extern_bitmap(name: &str) -> Option<&'static str> {
    match name {
        "cook torrance cc0236" => Some("rasterizer\\cc0236"),
        "cook torrance dd0236" => Some("rasterizer\\dd0236"),
        "cook torrance c78d78" => Some("rasterizer\\c78d78"),
        "dynamic environment map 1" | "dynamic environment map 2" => {
            Some("shaders\\default_bitmaps\\bitmaps\\default_dynamic_cube_map")
        }
        "active camo distortion texture" => Some("rasterizer\\active_camouflage_distortion"),
        _ => None,
    }
}

/// `render_method_submit`'s `get_extern_constant`, for an extern with no
/// frame in it.
pub(super) fn gen3_extern_change_color(name: &str) -> Option<usize> {
    [
        "change color primary",
        "change color secondary",
        "change color tertiary",
        "change color quaternary",
    ]
    .iter()
    .position(|known| *known == name)
}
