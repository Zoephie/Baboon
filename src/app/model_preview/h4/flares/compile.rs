//! The lens flare pass, compiled from the kit's shader.

use super::*;

/// `rasterizer_globals`' explicit shader 46, `lens_flare`, whose one
/// technique's `screen` pass draws every reflection.
const LENS_FLARE: usize = 46;

pub(crate) fn h4_flare_pass(reader: &TagReader) -> Result<super::super::super::gen3::shaders::H3Pass, String> {
    use crate::core::hlsl::emit::Stage;
    let globals = reader.load("globals\\rasterizer_globals",
        "rasterizer_globals",
        b"rasg",
    )
    .map_err(|error| error.to_string())?;
    let explicit = elements(globals.root(), "explicit shaders");
    let entry = explicit.get(LENS_FLARE).ok_or("no explicit shader 46")?;
    let path = |field: &str| {
        entry
            .read_tag_ref_path(field)
            .filter(|path| !path.is_empty())
            .ok_or(format!("no {field}"))
    };
    let program = |path: &str| -> Result<_, String> {
        super::super::materials::shaders::H4ShaderSource::read(reader, path)?
            .program(&super::super::materials::shaders::H4_DEFINES)
    };
    let vertex = super::super::materials::shaders::translate_technique(
        &program(&path("explicit vertex shader")?)?,
        0,
        "screen",
        Stage::Vertex,
    )?;
    let pixel = super::super::materials::shaders::translate_technique(
        &program(&path("explicit pixel shader")?)?,
        0,
        "screen",
        Stage::Pixel,
    )?;
    Ok(super::super::super::gen3::shaders::pass_of(vertex, pixel))
}
