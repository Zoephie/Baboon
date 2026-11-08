//! Manual shader tools, each run by its own variable: one pass's inputs, and
//! sweeps of a family's or a kit's passes through translation and the driver.

use super::super::*;
use eframe::glow::{self, HasContext as _};
use std::sync::Arc;
use super::gl::headless_gl;
use super::shot::*;
use crate::core::game::GameId;
use crate::core::source::TagSource;

/// What one pass reads: `H3_PASS=<family>|<template>|<entry>` and
/// `BLAM_TEST_H3EK` to the tags folder.
#[test]
fn h3_pass_inputs() {
    use super::super::gen3::shaders::{H3Sources, VERTEX_SKINNED, read_h3_definition};
    let Ok(spec) = std::env::var("H3_PASS") else {
        return;
    };
    let root = std::env::var("BLAM_TEST_H3EK").unwrap();
    let game = if root.contains("reach") { GameId::HaloReach } else { GameId::Halo3 };
    let source = TagSource::LooseFolder {
        root: root.clone().into(),
        game: Some(game),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    // `explicit|<file>` translates one of the explicit shaders.
    let pass = if let Some(file) = spec.strip_prefix("explicit|") {
        H3Sources::new(&source).explicit_pass(file).unwrap()
    } else {
        let [family, template, entry]: [&str; 3] = spec.split('|').collect::<Vec<_>>().try_into().unwrap();
        let definition = read_h3_definition(&crate::app::model_preview::load::TagReader::new(&source), &format!("shaders\\{family}")).unwrap();
        let options: Vec<usize> = template.split('_').filter_map(|digit| digit.parse().ok()).collect();
        Arc::new(H3Sources::new(&source).translate_entry(&definition, &options, entry, VERTEX_SKINNED).unwrap())
    };
    for (name, ty) in &pass.uniforms {
        eprintln!("NOTE UNIFORM {name} {ty:?}");
    }
    for (name, kind) in &pass.textures {
        eprintln!("NOTE TEXTURE {name} {kind:?}");
    }
    if std::env::var_os("H3_PASS_SOURCE").is_some() {
        eprintln!("{}\n-----\n{}", pass.vertex, pass.pixel);
    }
}

/// Every template of a family through translation and the driver. Set
/// `H3_SWEEP=<limit>` and `BLAM_TEST_H3EK` to the tags folder.
#[test]
fn h3_sweep() {
    use super::super::gen3::shaders::{H3Sources, VERTEX_RIGID, VERTEX_SKINNED, read_h3_definition};
    let Ok(limit) = std::env::var("H3_SWEEP") else {
        return;
    };
    let limit: usize = limit.parse().unwrap();
    let root = std::env::var("BLAM_TEST_H3EK").unwrap();
    let family = std::env::var("H3_FAMILY").unwrap_or_else(|_| "shader".to_owned());
    let source = TagSource::LooseFolder {
        root: root.clone().into(),
        game: Some(if root.contains("odst") {
            GameId::Halo3Odst
        } else if root.contains("reach") {
            GameId::HaloReach
        } else {
            GameId::Halo3
        }),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let definition = read_h3_definition(&crate::app::model_preview::load::TagReader::new(&source), &format!("shaders\\{family}")).unwrap();
    let mut sources = H3Sources::new(&source);
    let folder = format!("{root}/shaders/{family}_templates");
    let mut templates: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter_map(|name| name.strip_suffix(".render_method_template").map(str::to_owned))
        .collect();
    templates.sort();
    let (mut good, mut translation, mut compile) = (0, 0, 0);
    let mut seen: HashMap<String, usize> = HashMap::new();
    for template in templates.iter().take(limit) {
        let options: Vec<usize> = template.split('_').filter_map(|digit| digit.parse().ok()).collect();
        for entry in &definition.entry_points {
            for vertex_type in [VERTEX_RIGID, VERTEX_SKINNED] {
                match sources.translate_entry(&definition, &options, entry, vertex_type) {
                    Err(error) => {
                        translation += 1;
                        let key: String = error.chars().take(140).collect();
                        if *seen.entry(key.clone()).and_modify(|count| *count += 1).or_insert(1) == 1 {
                            eprintln!("NOTE TR {template} {entry}: {key}");
                        }
                    }
                    Ok(pass) => {
                        let mut failed = false;
                        let program = unsafe { gl.create_program().unwrap() };
                        let mut shaders = Vec::new();
                        for (kind, text) in [(glow::VERTEX_SHADER, &pass.vertex), (glow::FRAGMENT_SHADER, &pass.pixel)] {
                            unsafe {
                                let shader = gl.create_shader(kind).unwrap();
                                shaders.push(shader);
                                gl.shader_source(shader, text);
                                gl.compile_shader(shader);
                                if !gl.get_shader_compile_status(shader) {
                                    failed = true;
                                    let log = gl.get_shader_info_log(shader);
                                    let first = log.lines().next().unwrap_or("").to_owned();
                                    if let Some(line) = first.split(':').nth(2).and_then(|n| n.trim().parse::<usize>().ok()) {
                                        let bad = text.lines().nth(line - 1).unwrap_or("");
                                        let key: String = first.chars().skip(8).filter(|c| !c.is_ascii_digit()).take(70).collect();
                                        if *seen.entry(key).and_modify(|count| *count += 1).or_insert(1) == 1 {
                                            eprintln!("NOTE GL {template} {entry}: {first}\n    {}", bad.chars().take(170).collect::<String>());
                                        }
                                    }
                                }
                                gl.attach_shader(program, shader);
                            }
                        }
                        // A stage built from other options than its partner can
                        // disagree on what passes between them.
                        unsafe {
                            if !failed {
                                gl.link_program(program);
                                if !gl.get_program_link_status(program) {
                                    failed = true;
                                    let log = gl.get_program_info_log(program);
                                    let key: String = log.lines().next().unwrap_or("").chars().take(120).collect();
                                    if *seen.entry(format!("LINK {key}")).and_modify(|count| *count += 1).or_insert(1) == 1 {
                                        eprintln!("NOTE LINK {template} {entry}: {key}");
                                    }
                                }
                            }
                            for shader in shaders {
                                gl.delete_shader(shader);
                            }
                            gl.delete_program(program);
                        }
                        if failed { compile += 1 } else { good += 1 }
                    }
                }
            }
        }
    }
    let mut common: Vec<_> = seen.into_iter().collect();
    common.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (error, count) in common.iter().take(12) {
        eprintln!("NOTE TOP {count} {error}");
    }
    eprintln!("NOTE SWEEP good {good} translation {translation} compile {compile}");
}

/// Every Halo 4 material shader's object passes through translation and the
/// driver. Set `H4_SWEEP=<limit>` and `BLAM_TEST_H4EK` to the tags folder;
/// `H4_FOLDER` narrows to a folder under `shaders\material_shaders`.
#[test]
fn h4_sweep() {
    use super::super::h4::materials::shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
    use crate::core::hlsl::emit::Stage;
    let Ok(limit) = std::env::var("H4_SWEEP") else {
        return;
    };
    let limit: usize = limit.parse().unwrap();
    let root = std::env::var("BLAM_TEST_H4EK").unwrap();
    let folder = std::env::var("H4_FOLDER").unwrap_or_else(|_| "materials".to_owned());
    let techniques: Vec<String> = std::env::var("H4_TECHNIQUES")
        .unwrap_or_else(|_| "albedo,static_probe,single_pass_single_probe".to_owned())
        .split(',')
        .map(str::to_owned)
        .collect();
    let source = TagSource::LooseFolder {
        root: root.clone().into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let gl = headless_gl();
    let mut tags = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(format!("{root}/shaders/material_shaders/{folder}"))];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "material_shader") {
                let relative = path.strip_prefix(&root).unwrap().with_extension("");
                tags.push(relative.to_string_lossy().replace('/', "\\"));
            }
        }
    }
    tags.sort();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut note = |kind: &str, tag: &str, what: &str, error: &str| {
        let key: String = format!("{kind} {}", error.chars().take(140).collect::<String>());
        if *seen.entry(key.clone()).and_modify(|count| *count += 1).or_insert(1) == 1 {
            eprintln!("NOTE {kind} {tag} {what}: {}", error.chars().take(400).collect::<String>());
        }
    };
    let compile = |kind: u32, text: &str| -> Result<glow::NativeShader, String> {
        unsafe {
            let shader = gl.create_shader(kind).unwrap();
            gl.shader_source(shader, text);
            gl.compile_shader(shader);
            if gl.get_shader_compile_status(shader) {
                return Ok(shader);
            }
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            let first = log.lines().next().unwrap_or("").to_owned();
            let bad = first
                .split(':')
                .nth(2)
                .and_then(|line| line.trim().parse::<usize>().ok())
                .and_then(|line| text.lines().nth(line - 1))
                .unwrap_or("");
            Err(format!("{first} || {}", bad.trim()))
        }
    };
    if let Ok(needle) = std::env::var("H4_DUMP") {
        let shader = H4ShaderSource::read(&crate::app::model_preview::load::TagReader::new(&source), &tags[0]).unwrap();
        let tokens = shader.tokens(&H4_DEFINES).unwrap();
        let text: Vec<&str> = tokens.iter().map(|token| &*token.text).collect();
        let at = text.iter().position(|token| *token == needle).unwrap_or(0);
        eprintln!("NOTE DUMP {}", text[at.saturating_sub(80)..(at + 40).min(text.len())].join(" "));
        return;
    }
    let (mut good, mut parse_failed, mut translation, mut driver) = (0, 0, 0, 0);
    for tag in tags.iter().take(limit) {
        let program = match H4ShaderSource::read(&crate::app::model_preview::load::TagReader::new(&source), tag).and_then(|shader| shader.program(&H4_DEFINES)) {
            Ok(program) => program,
            Err(error) => {
                parse_failed += 1;
                note("PARSE", tag, "", &error);
                continue;
            }
        };
        for technique in &techniques {
            if !program.techniques.iter().any(|candidate| &*candidate.name == technique.as_str()) {
                continue;
            }
            let pixel = match translate_pass(&program, technique, "_default", Stage::Pixel) {
                Ok(pixel) => pixel,
                Err(error) => {
                    translation += 1;
                    note("TR", tag, &format!("{technique} ps"), &error);
                    continue;
                }
            };
            let Ok(pixel_shader) = compile(glow::FRAGMENT_SHADER, &pixel.source).map_err(|error| {
                driver += 1;
                note("GL", tag, &format!("{technique} ps"), &error);
            }) else {
                continue;
            };
            for pass in ["rigid", "skinned", "rigid_boned"] {
                let vertex = match translate_pass(&program, technique, pass, Stage::Vertex) {
                    Ok(vertex) => vertex,
                    Err(error) => {
                        translation += 1;
                        note("TR", tag, &format!("{technique} {pass}"), &error);
                        continue;
                    }
                };
                let vertex_shader = match compile(glow::VERTEX_SHADER, &vertex.source) {
                    Ok(shader) => shader,
                    Err(error) => {
                        driver += 1;
                        note("GL", tag, &format!("{technique} {pass}"), &error);
                        continue;
                    }
                };
                unsafe {
                    let linked = gl.create_program().unwrap();
                    gl.attach_shader(linked, vertex_shader);
                    gl.attach_shader(linked, pixel_shader);
                    gl.link_program(linked);
                    if gl.get_program_link_status(linked) {
                        good += 1;
                    } else {
                        driver += 1;
                        note("LINK", tag, &format!("{technique} {pass}"), &gl.get_program_info_log(linked));
                    }
                    gl.delete_program(linked);
                    gl.delete_shader(vertex_shader);
                }
            }
            unsafe { gl.delete_shader(pixel_shader) };
        }
    }
    let mut common: Vec<_> = seen.into_iter().collect();
    common.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (error, count) in common.iter().take(15) {
        eprintln!("NOTE TOP {count} {error}");
    }
    eprintln!("NOTE SWEEP {} tags: good {good} parse {parse_failed} translation {translation} driver {driver}", tags.len().min(limit));
}

/// What one Halo 4 pass reads: `H4_PASS=<material_shader>|<technique>|<vertex pass>`
/// and `BLAM_TEST_H4EK`. `H4_PASS_SOURCE` prints the GLSL too.
#[test]
fn h4_pass_inputs() {
    use super::super::h4::materials::shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
    use crate::core::hlsl::emit::Stage;
    let Ok(spec) = std::env::var("H4_PASS") else {
        return;
    };
    let root = std::env::var("BLAM_TEST_H4EK").unwrap();
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let [shader, technique, pass]: [&str; 3] = spec.split('|').collect::<Vec<_>>().try_into().unwrap();
    let program = H4ShaderSource::read(&crate::app::model_preview::load::TagReader::new(&source), shader).unwrap().program(&H4_DEFINES).unwrap();
    for technique in &program.techniques {
        let passes: Vec<String> = technique
            .passes
            .iter()
            .map(|(name, shaders)| format!("{name}[{}]", shaders.iter().map(|shader| &*shader.stage).collect::<Vec<_>>().join(",")))
            .collect();
        eprintln!("NOTE TECHNIQUE {} {}", technique.name, passes.join(" "));
    }
    for stage in [Stage::Vertex, Stage::Pixel] {
        let translated = translate_pass(&program, technique, pass, stage).unwrap();
        for (name, ty) in &translated.uniforms {
            eprintln!("NOTE {stage:?} UNIFORM {name} {ty:?}");
        }
        for (name, kind) in &translated.textures {
            eprintln!("NOTE {stage:?} TEXTURE {name} {kind:?}");
        }
        for attribute in &translated.attributes {
            eprintln!("NOTE {stage:?} IO {} {} {:?}", attribute.semantic, attribute.glsl, attribute.ty);
        }
        if std::env::var_os("H4_PASS_SOURCE").is_some() {
            eprintln!("{}", translated.source);
        }
    }
}

/// Every Halo 4 `.model` through the shot harness, each caught on its own,
/// or with `H4_KIT_OBJECTS=1` every object tag by its kind. Set
/// `H4_KIT=<limit>`, `H2_SHOT_H4=1` and `BLAM_TEST_H2EK` to the tags folder;
/// translation and link failures print as `h3 program failed`.
#[test]
fn h4_kit_sweep() {
    let (Ok(limit), Ok(root)) = (std::env::var("H4_KIT"), std::env::var("BLAM_TEST_H2EK")) else {
        return;
    };
    let limit: usize = limit.parse().unwrap();
    let source = TagSource::LooseFolder {
        root: root.clone().into(),
        game: Some(GameId::Halo4),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    // `H4_KIT_OBJECTS=1` takes the object tags instead, each by its kind:
    // their attachments, held weapons and effects load as well.
    let objects = std::env::var_os("H4_KIT_OBJECTS").is_some();
    const KINDS: [(&str, &str); 11] = [
        ("biped", "biped"),
        ("vehicle", "vehicle"),
        ("weapon", "weapon"),
        ("equipment", "equipment"),
        ("scenery", "scenery"),
        ("crate", "crate"),
        ("device_machine", "machine"),
        ("device_control", "control"),
        ("effect_scenery", "effect_scenery"),
        ("giant", "giant"),
        ("projectile", "projectile"),
    ];
    let mut models: Vec<(String, Option<String>)> = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(&root)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };
            let kind = if objects {
                match KINDS.iter().find(|(known, _)| *known == extension) {
                    Some((_, kind)) => Some((*kind).to_owned()),
                    None => continue,
                }
            } else if extension == "model" {
                None
            } else {
                continue;
            };
            let relative = path.strip_prefix(&root).unwrap().with_extension("");
            models.push((relative.to_string_lossy().replace('/', "\\"), kind));
        }
    }
    models.sort();
    let gl = Arc::new(headless_gl());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    let options = ShotOptions::new(GameId::Halo4);
    let (mut good, mut failed) = (0, Vec::new());
    for (model, kind) in models.iter().take(limit) {
        let shot = Shot {
            model: model.clone(),
            shaders: true,
            lights: true,
            yaw: -1.57,
            pitch: 0.3,
            kind: kind.clone(),
        };
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(&gl, &mut renderer, &source, &shot, &options)));
        match caught {
            Ok(_) => good += 1,
            Err(panic) => {
                let detail = crate::app::shell::worker::panic_detail(&*panic);
                eprintln!("NOTE FAIL {model}: {}", detail.chars().take(200).collect::<String>());
                failed.push(model.clone());
                // A panic mid-frame can leave state bound; start clean.
                renderer = ModelGlRenderer::new(&gl).unwrap();
            }
        }
    }
    eprintln!("NOTE KIT {} models: good {good}, failed {}", models.len().min(limit), failed.len());
}
