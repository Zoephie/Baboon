use super::*;
use crate::app::model_preview::harness::kit::kit_root;
use crate::core::game::GameId;
use crate::core::h2_pixel_shader::{PixelShaderOptions, translate};

const STAGES: [H2VertexStage; 47] = [
    H2VertexStage::CeActiveCamo,
    H2VertexStage::CePlasma,
    H2VertexStage::CeGlassLit,
    H2VertexStage::CeGlassFlat,
    H2VertexStage::CeMeter,
    H2VertexStage::CeWaterOpacity,
    H2VertexStage::CeModel,
    H2VertexStage::CeTransparent,
    H2VertexStage::CeTransparentReflection,
    H2VertexStage::CeTransparentObject,
    H2VertexStage::CeTransparentViewer,
    H2VertexStage::CeTransparentScreen,
    H2VertexStage::LightDiffuseTarget,
    H2VertexStage::LightDiffuseDetail,
    H2VertexStage::LightDiffuse,
    H2VertexStage::LightGel,
    H2VertexStage::LightGelTarget,
    H2VertexStage::LightGelDetail,
    H2VertexStage::LightSpecular,
    H2VertexStage::LightFrustum,
    H2VertexStage::LightFrustumTarget,
    H2VertexStage::LightFrustumDetail,
    H2VertexStage::Emissive,
    H2VertexStage::Environment,
    H2VertexStage::LightmapTarget,
    H2VertexStage::LightmapTargetEye,
    H2VertexStage::ActiveCamo,
    H2VertexStage::ShadowCaster,
    H2VertexStage::ShadowMask,
    H2VertexStage::TextureCamera,
    H2VertexStage::Water,
    H2VertexStage::Hologram,
    H2VertexStage::Prt,
    H2VertexStage::PrtDetail,
    H2VertexStage::LightmapEye,
    H2VertexStage::LightmapEyeBlend,
    H2VertexStage::Texaccum,
    H2VertexStage::Lightmap,
    H2VertexStage::LightmapEnv,
    H2VertexStage::LightmapMark,
    H2VertexStage::LightmapEnvStencil,
    H2VertexStage::LightmapMarkStencil,
    H2VertexStage::LightmapStencil,
    H2VertexStage::Default,
    H2VertexStage::Reflection,
    H2VertexStage::Offset,
    H2VertexStage::Bumped,
];

#[test]
fn every_transparent_vertex_shader_has_a_stage() {
    let dir = "rasterizer\\vertex_shaders_dx9\\";
    for (name, stage) in [
        ("transparent_generic_default", H2VertexStage::Default),
        ("transparent_generic_reflection", H2VertexStage::Reflection),
        ("transparent_generic_reflection_sky", H2VertexStage::Reflection),
        ("transparent_generic_offset", H2VertexStage::Offset),
        ("stencil_env_bumped_tangentspace", H2VertexStage::Bumped),
    ] {
        assert_eq!(H2VertexStage::from_path(&format!("{dir}{name}")), Some(stage));
    }
    assert_eq!(H2VertexStage::from_path("rasterizer\\unknown"), None);
    assert_eq!(H2VertexStage::from_path("Rasterizer\\Hologram"), Some(H2VertexStage::Hologram));
}

/// Stages must link with what the translator produces, so a varying
/// renamed on one side fails here rather than as a preview drawing nothing.
#[test]
fn every_stage_links_with_a_translated_pixel_shader() {
    if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
        eprintln!("skipping: glslangValidator is not on PATH");
        return;
    }
    let source = "float4 c1 : register(C1);\nfloat4 main(\n)\n{\n    float4 r0;\n    \
                      r0= color0 * color1 + tc0 + tc1 + tc2 + tc3 + tc7 + fog;\n    \
                      return r0 * c1;\n}\n";
    let fragment =
        translate(source, "#version 330\n", &PixelShaderOptions::default()).unwrap().glsl;
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
    let Some(root) = kit_root(GameId::Halo2) else { return };
    if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
        eprintln!("skipping: glslangValidator is not on PATH");
        return;
    }
    let file = std::path::Path::new(&root)
        .join("rasterizer/pixel_shaders_dx9")
        .join("shaders_shader_passes_texture_mapping_default_texture_zwrite_0.psh");
    let source = String::from_utf8_lossy(&std::fs::read(file).unwrap()).into_owned();
    let dir = crate::core::test_kits::unique_temp_dir("h2_texaccum_stage");
    let (vertex, pixel) = (dir.join("pass.vert"), dir.join("pass.frag"));
    std::fs::write(&vertex, H2VertexStage::Texaccum.source("#version 330\n")).unwrap();
    let shader = translate(&source, "#version 330\n", &PixelShaderOptions::default()).unwrap();
    std::fs::write(&pixel, shader.glsl).unwrap();
    let output = std::process::Command::new("glslangValidator")
        .arg("-l")
        .arg(&vertex)
        .arg(&pixel)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
}

/// With `BLAM_TEST_H2EK`: real object lightmap shaders link with their
/// stages, in every specular variant.
#[test]
fn lightmap_stages_link_with_the_kit_shaders() {
    let Some(root) = kit_root(GameId::Halo2) else { return };
    if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
        eprintln!("skipping: glslangValidator is not on PATH");
        return;
    }
    let directory = std::path::Path::new(&root).join("rasterizer/pixel_shaders_dx9");
    let dir = crate::core::test_kits::unique_temp_dir("h2_lightmap_stages");
    for (pass, stage) in [
        ("lightmap_tex_detail", H2VertexStage::Lightmap),
        ("lightmap_tex_env", H2VertexStage::LightmapEnv),
        ("lightmap_tex_four_change_color", H2VertexStage::LightmapMark),
        ("lightmap_tex_env_detail", H2VertexStage::LightmapEnvStencil),
        ("lightmap_tex_detail_blend_specular", H2VertexStage::LightmapMarkStencil),
        ("lightmap_honor_guard", H2VertexStage::LightmapStencil),
        ("lightmap_env", H2VertexStage::LightmapTarget),
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
