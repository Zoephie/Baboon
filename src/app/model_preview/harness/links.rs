//! Per-game passes that only a GL driver can check: they translate and link.

use super::super::gl::link::link;
use super::super::*;
use super::gl::headless_gl;
use crate::core::game::GameId;
use crate::core::source::TagSource;
use eframe::glow::{self, HasContext as _};

/// Reach's `shadow_apply` links, and reads every input it is given.
#[test]
fn reach_shadow_apply_links() {
    let gl = headless_gl();
    let program = unsafe {
        crate::app::model_preview::gl::screen::screen_program(
            &gl,
            super::super::reach::shadow_pass::APPLY_FRAGMENT,
        )
    }
    .expect("the apply compiles and links");
    for name in [
        "scene_depth",
        "normals",
        "shadow_map",
        "screen_to_world",
        "shadow_view_projection",
        "hull_planes",
    ] {
        assert!(
            unsafe { gl.get_uniform_location(program, name) }.is_some(),
            "{name} is read"
        );
    }
}

/// Reach's shield passes, 61 and 119, translate with the preview's depth
/// read and the oscillation maps in their vertex stage, and link.
/// `BLAM_TEST_HREK` to the tags folder.
#[test]
fn reach_shield_passes_link() {
    let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::HaloReach),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let model = load_referenced_tag_from_source(
        &source,
        "objects\\characters\\spartans\\spartans",
        "model",
        b"hlmt",
    )
    .unwrap();
    let shield = super::super::reach::shield::read_reach_shield(
        &crate::app::model_preview::load::TagReader::new(&source),
        &model,
    )
    .expect("the Spartan's shield");
    let gl = headless_gl();
    let passes: Vec<_> = shield.passes().collect();
    assert_eq!(passes.len(), 2, "both explicit shaders");
    for pass in passes {
        assert!(
            pass.pixel
                .contains(super::super::gen3::shield::DEPTH_DECODE),
            "the depth read is the preview's"
        );
        assert!(
            !pass.pixel.contains("depth_constants.z - "),
            "Reach's own depth read is gone"
        );
        assert!(
            !pass
                .vertex
                .contains("GlobalTexture_shield_impact_noise_texture"),
            "the vertex stage reads oscillation"
        );
        unsafe {
            let stages = [
                (glow::VERTEX_SHADER, pass.vertex.as_str()),
                (glow::FRAGMENT_SHADER, pass.pixel.as_str()),
            ];
            let program = link(&gl, &stages, &[]).unwrap_or_else(|error| panic!("{error}"));
            for (name, _) in &pass.textures {
                let location = gl.get_uniform_location(program, name);
                eprintln!("NOTE shield TEXTURE {name} {}", location.is_some());
            }
        }
    }
}

/// Reach's four `light_apply` passes translate and link. `BLAM_TEST_HREK`
/// to the tags folder.
#[test]
fn reach_light_passes_link() {
    let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::HaloReach),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let mut sources = super::super::gen3::shaders::Gen3Sources::new(&source);
    for file in [
        "light_apply",
        "light_apply_frustum",
        "light_apply_specular",
        "light_apply_frustum_specular",
    ] {
        let pass = sources
            .reach_light_pass(file)
            .unwrap_or_else(|error| panic!("{file}: {error}"));
        for (name, _) in &pass.uniforms {
            eprintln!("NOTE {file} UNIFORM {name}");
        }
        for (name, _) in &pass.textures {
            eprintln!("NOTE {file} TEXTURE {name}");
        }
        unsafe {
            let stages = [
                (glow::VERTEX_SHADER, pass.vertex.as_str()),
                (glow::FRAGMENT_SHADER, pass.pixel.as_str()),
            ];
            link(&gl, &stages, &[]).unwrap_or_else(|error| panic!("{file}: {error}"));
        }
    }
}

/// Halo 4's screen-space light passes, explicit 187 and 204's sixteen
/// techniques each, compile and link; `BLAM_TEST_H4EK` to the tags folder.
#[test]
fn h4_light_passes_link() {
    let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    for physical in [false, true] {
        let program = super::super::h4::light_pass::h4_light_program(
            &crate::app::model_preview::load::TagReader::new(&source),
            physical,
        )
        .expect("the light shader");
        for technique in 0..16 {
            let pass = super::super::h4::light_pass::h4_light_pass(&program, technique)
                .unwrap_or_else(|error| panic!("{physical} {technique}: {error}"));
            unsafe {
                let stages = [
                    (glow::VERTEX_SHADER, pass.vertex.as_str()),
                    (glow::FRAGMENT_SHADER, pass.pixel.as_str()),
                ];
                link(&gl, &stages, &[])
                    .unwrap_or_else(|error| panic!("{physical} {technique}: {error}"));
            }
        }
    }
}

/// Halo 4's lens flare pass, explicit 46, compiles and links, and takes its
/// corners and texture coordinates as `POSITION` and `TEXCOORD0`.
#[test]
fn h4_flare_pass_links() {
    let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
        return;
    };
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let pass = super::super::h4::flares::h4_flare_pass(
        &crate::app::model_preview::load::TagReader::new(&source),
    )
    .expect("the flare pass");
    for (name, _) in &pass.uniforms {
        eprintln!("NOTE flare UNIFORM {name}");
    }
    assert!(
        pass.vertex.contains("in vec2 a_POSITION;") && pass.vertex.contains("a_TEXCOORD0"),
        "{}",
        pass.vertex
    );
    unsafe {
        let stages = [
            (glow::VERTEX_SHADER, pass.vertex.as_str()),
            (glow::FRAGMENT_SHADER, pass.pixel.as_str()),
        ];
        link(&gl, &stages, &[]).unwrap_or_else(|error| panic!("{error}"));
    }
}

/// Every kit effect material shader's vertex passes link with its `_default`
/// pixel shader; `BLAM_TEST_H4EK` to the tags folder.
#[test]
fn h4_effect_passes_link() {
    use super::super::h4::materials::shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
    use crate::core::hlsl::emit::Stage;
    let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
        return;
    };
    let folder = std::path::Path::new(&root).join("shaders/material_shaders/fx");
    let source = TagSource::LooseFolder {
        root: std::path::PathBuf::from(&root),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let mut failures = Vec::new();
    let mut linked = 0;
    for entry in std::fs::read_dir(&folder).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".material_shader") else {
            continue;
        };
        let path = format!("shaders\\material_shaders\\fx\\{stem}");
        let program = H4ShaderSource::read(
            &crate::app::model_preview::load::TagReader::new(&source),
            &path,
        )
        .and_then(|shader| shader.program(&H4_DEFINES))
        .unwrap();
        let Some(technique) = program
            .techniques
            .iter()
            .find(|technique| &*technique.name == "_default")
        else {
            continue;
        };
        let Ok(pixel) = translate_pass(&program, "_default", "_default", Stage::Pixel) else {
            continue;
        };
        for (pass, _) in technique
            .passes
            .iter()
            .filter(|(pass, _)| &**pass != "_default")
        {
            let vertex = translate_pass(&program, "_default", pass, Stage::Vertex).unwrap();
            let both = super::super::gen3::shaders::pass_of(vertex, pixel.clone());
            unsafe {
                let program = gl.create_program().unwrap();
                let mut ok = true;
                for (kind, text) in [
                    (glow::VERTEX_SHADER, &both.vertex),
                    (glow::FRAGMENT_SHADER, &both.pixel),
                ] {
                    let shader = gl.create_shader(kind).unwrap();
                    super::super::gl::glsl::record_source(text);
                    gl.shader_source(shader, text);
                    gl.compile_shader(shader);
                    if !gl.get_shader_compile_status(shader) {
                        let log = gl.get_shader_info_log(shader);
                        let line = log.lines().next().unwrap_or("").to_owned();
                        let at = line
                            .split(':')
                            .nth(2)
                            .and_then(|n| n.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        let bad = text
                            .lines()
                            .nth(at.saturating_sub(1))
                            .unwrap_or("")
                            .to_owned();
                        failures.push(format!(
                            "{stem}/{pass} {}: {line}\n    {bad}",
                            if kind == glow::VERTEX_SHADER {
                                "vs"
                            } else {
                                "ps"
                            }
                        ));
                        ok = false;
                    }
                    gl.attach_shader(program, shader);
                }
                if ok {
                    gl.link_program(program);
                    if gl.get_program_link_status(program) {
                        linked += 1;
                    } else {
                        failures.push(format!(
                            "{stem}/{pass} link: {}",
                            gl.get_program_info_log(program)
                        ));
                    }
                }
            }
        }
    }
    for failure in &failures {
        eprintln!("NOTE FAIL {failure}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert!(linked > 30, "linked {linked}");
}
