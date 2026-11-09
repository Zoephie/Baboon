use super::*;
use crate::app::model_preview::harness::kit::kit_root;
use crate::core::game::GameId;
use crate::core::h2_pixel_shader::{PixelShaderOptions, translate};

const STAGES: [Dx9VertexStage; 47] = [
    Dx9VertexStage::CeActiveCamo,
    Dx9VertexStage::CePlasma,
    Dx9VertexStage::CeGlassLit,
    Dx9VertexStage::CeGlassFlat,
    Dx9VertexStage::CeMeter,
    Dx9VertexStage::CeWaterOpacity,
    Dx9VertexStage::CeModel,
    Dx9VertexStage::CeTransparent,
    Dx9VertexStage::CeTransparentReflection,
    Dx9VertexStage::CeTransparentObject,
    Dx9VertexStage::CeTransparentViewer,
    Dx9VertexStage::CeTransparentScreen,
    Dx9VertexStage::LightDiffuseTarget,
    Dx9VertexStage::LightDiffuseDetail,
    Dx9VertexStage::LightDiffuse,
    Dx9VertexStage::LightGel,
    Dx9VertexStage::LightGelTarget,
    Dx9VertexStage::LightGelDetail,
    Dx9VertexStage::LightSpecular,
    Dx9VertexStage::LightFrustum,
    Dx9VertexStage::LightFrustumTarget,
    Dx9VertexStage::LightFrustumDetail,
    Dx9VertexStage::Emissive,
    Dx9VertexStage::Environment,
    Dx9VertexStage::LightmapTarget,
    Dx9VertexStage::LightmapTargetEye,
    Dx9VertexStage::ActiveCamo,
    Dx9VertexStage::ShadowCaster,
    Dx9VertexStage::ShadowMask,
    Dx9VertexStage::TextureCamera,
    Dx9VertexStage::Water,
    Dx9VertexStage::Hologram,
    Dx9VertexStage::Prt,
    Dx9VertexStage::PrtDetail,
    Dx9VertexStage::LightmapEye,
    Dx9VertexStage::LightmapEyeBlend,
    Dx9VertexStage::Texaccum,
    Dx9VertexStage::Lightmap,
    Dx9VertexStage::LightmapEnv,
    Dx9VertexStage::LightmapMark,
    Dx9VertexStage::LightmapEnvStencil,
    Dx9VertexStage::LightmapMarkStencil,
    Dx9VertexStage::LightmapStencil,
    Dx9VertexStage::Default,
    Dx9VertexStage::Reflection,
    Dx9VertexStage::Offset,
    Dx9VertexStage::Bumped,
];

#[test]
fn every_transparent_vertex_shader_has_a_stage() {
    let dir = "rasterizer\\vertex_shaders_dx9\\";
    for (name, stage) in [
        ("transparent_generic_default", Dx9VertexStage::Default),
        ("transparent_generic_reflection", Dx9VertexStage::Reflection),
        (
            "transparent_generic_reflection_sky",
            Dx9VertexStage::Reflection,
        ),
        ("transparent_generic_offset", Dx9VertexStage::Offset),
        ("stencil_env_bumped_tangentspace", Dx9VertexStage::Bumped),
    ] {
        assert_eq!(
            Dx9VertexStage::from_path(&format!("{dir}{name}")),
            Some(stage)
        );
    }
    assert_eq!(Dx9VertexStage::from_path("rasterizer\\unknown"), None);
    assert_eq!(
        Dx9VertexStage::from_path("Rasterizer\\Hologram"),
        Some(Dx9VertexStage::Hologram)
    );
}

/// Stages must link with what the translator produces, so a varying
/// renamed on one side fails here rather than as a preview drawing nothing.
#[test]
fn every_stage_links_with_a_translated_pixel_shader() {
    if std::process::Command::new("glslangValidator")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: glslangValidator is not on PATH");
        return;
    }
    let source = "float4 c1 : register(C1);\nfloat4 main(\n)\n{\n    float4 r0;\n    \
                      r0= color0 * color1 + tc0 + tc1 + tc2 + tc3 + tc7 + fog;\n    \
                      return r0 * c1;\n}\n";
    let fragment = translate(source, "#version 330\n", &PixelShaderOptions::default())
        .unwrap()
        .glsl;
    let dir = crate::core::test_kits::unique_temp_dir("h2_vertex_stages");
    let pixel = dir.join("pass.frag");
    std::fs::write(&pixel, fragment).unwrap();
    for stage in STAGES {
        let vertex = dir.join("pass.vert");
        std::fs::write(&vertex, stage.source("#version 330\n")).unwrap();
        let output = std::process::Command::new("glslangValidator")
            .arg("-l")
            .arg(&vertex)
            .arg(&pixel)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{stage:?} does not link:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

/// With `BLAM_TEST_H2EK`: the texaccum shader links with its stage.
#[test]
fn the_texaccum_stage_links_with_the_kit_shader() {
    let Some(root) = kit_root(GameId::Halo2) else {
        return;
    };
    if std::process::Command::new("glslangValidator")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: glslangValidator is not on PATH");
        return;
    }
    let file = std::path::Path::new(&root)
        .join("rasterizer/pixel_shaders_dx9")
        .join("shaders_shader_passes_texture_mapping_default_texture_zwrite_0.psh");
    let source = String::from_utf8_lossy(&std::fs::read(file).unwrap()).into_owned();
    let dir = crate::core::test_kits::unique_temp_dir("h2_texaccum_stage");
    let (vertex, pixel) = (dir.join("pass.vert"), dir.join("pass.frag"));
    std::fs::write(&vertex, Dx9VertexStage::Texaccum.source("#version 330\n")).unwrap();
    let shader = translate(&source, "#version 330\n", &PixelShaderOptions::default()).unwrap();
    std::fs::write(&pixel, shader.glsl).unwrap();
    let output = std::process::Command::new("glslangValidator")
        .arg("-l")
        .arg(&vertex)
        .arg(&pixel)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// With `BLAM_TEST_H2EK`: real object lightmap shaders link with their
/// stages, in every specular variant.
#[test]
fn lightmap_stages_link_with_the_kit_shaders() {
    let Some(root) = kit_root(GameId::Halo2) else {
        return;
    };
    if std::process::Command::new("glslangValidator")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: glslangValidator is not on PATH");
        return;
    }
    let directory = std::path::Path::new(&root).join("rasterizer/pixel_shaders_dx9");
    let dir = crate::core::test_kits::unique_temp_dir("h2_lightmap_stages");
    for (pass, stage) in [
        ("lightmap_tex_detail", Dx9VertexStage::Lightmap),
        ("lightmap_tex_env", Dx9VertexStage::LightmapEnv),
        (
            "lightmap_tex_four_change_color",
            Dx9VertexStage::LightmapMark,
        ),
        (
            "lightmap_tex_env_detail",
            Dx9VertexStage::LightmapEnvStencil,
        ),
        (
            "lightmap_tex_detail_blend_specular",
            Dx9VertexStage::LightmapMarkStencil,
        ),
        ("lightmap_honor_guard", Dx9VertexStage::LightmapStencil),
        ("lightmap_env", Dx9VertexStage::LightmapTarget),
    ] {
        let file = directory.join(format!("shaders_shader_passes_lightmap_{pass}_3.psh"));
        let source = String::from_utf8_lossy(&std::fs::read(file).unwrap()).into_owned();
        let vertex = dir.join("pass.vert");
        std::fs::write(&vertex, stage.source("#version 330\n")).unwrap();
        for specular_type in 0..4 {
            let options = PixelShaderOptions {
                cases: std::collections::HashMap::from([(
                    "specular_type".to_owned(),
                    specular_type,
                )]),
                alpha_test: None,
            };
            let Ok(shader) = translate(&source, "#version 330\n", &options) else {
                continue;
            };
            let pixel = dir.join("pass.frag");
            std::fs::write(&pixel, shader.glsl).unwrap();
            let output = std::process::Command::new("glslangValidator")
                .arg("-l")
                .arg(&vertex)
                .arg(&pixel)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{pass} specular {specular_type} does not link:\n{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}

/// Every stage's source, and the shadow passes' fragments, through the
/// shader dump (`BABOON_SHADER_DUMP`), so a change to how they are built is
/// checked even for the stages no shot draws.
#[test]
fn every_stage_source_goes_through_the_shader_dump() {
    use crate::app::model_preview::gl::glsl::record_source;
    for stage in STAGES {
        record_source(&stage.source("#version 330\n"));
    }
    for (_, pass) in crate::app::model_preview::dx9::passes::dx9_shadow_passes() {
        record_source(&pass.fragment);
    }
}
