use super::*;

/// Build an expected relative path from components, so the comparison uses
/// the native separator on any platform (the function joins via `PathBuf`).
fn rel(parts: &[&str]) -> PathBuf {
    parts.iter().collect()
}

#[test]
fn material_shader_source_path_strips_data_and_adds_fx() {
    assert_eq!(
        material_shader_source_relative_path(r"data\shaders\material_shaders\decals\base", 0),
        rel(&["shaders", "material_shaders", "decals", "base.fx"])
    );
}

#[test]
fn material_shader_source_path_preserves_existing_extension() {
    assert_eq!(
        material_shader_source_relative_path(
            r"data\shaders\material_shaders\include\core\lighting.hlsli",
            1
        ),
        rel(&[
            "shaders",
            "material_shaders",
            "include",
            "core",
            "lighting.hlsli"
        ])
    );
}

#[test]
fn material_shader_source_path_cannot_escape_output_folder() {
    assert_eq!(
        material_shader_source_relative_path(r"C:\data\..\shaders\bad:name\base", 2),
        rel(&["shaders", "bad_name", "base.fx"])
    );
}

#[test]
fn hlsl_include_source_path_preserves_hlsl_extension() {
    assert_eq!(
        hlsl_include_source_relative_path(r"rasterizer\hlsl\ssao.hlsl"),
        rel(&["rasterizer", "hlsl", "ssao.hlsl"])
    );
}

#[test]
fn hlsl_include_source_path_replaces_friendly_tag_extension() {
    assert_eq!(
        hlsl_include_source_relative_path(r"rasterizer\hlsl\ssao.hlsl_include"),
        rel(&["rasterizer", "hlsl", "ssao.hlsl"])
    );
}

#[test]
fn hlsl_include_source_path_adds_hlsl_extension() {
    assert_eq!(
        hlsl_include_source_relative_path(r"rasterizer\hlsl\ssao"),
        rel(&["rasterizer", "hlsl", "ssao.hlsl"])
    );
}
