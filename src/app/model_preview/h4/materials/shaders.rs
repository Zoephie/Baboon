//! Halo 4's material shaders, from the source each tag carries.
//!
//! A `material_shader` embeds every file its offline compile opened, its
//! `.fx` first, under `source shader files` (`data\shaders\material_shaders\..`
//! paths). The `.fx` is a D3DX effect: one `technique11` per entry point, a
//! `_default` pass naming the pixel shader and a pass per vertex kind
//! (`world`, `rigid`, `skinned`, `rigid_boned`, `rigid_blendshaped`,
//! `skinned_blendshaped`) naming its vertex shader. `DX_VERSION=11` is the
//! only define the compile needs; `pc` follows from it in `core.fxh`.

use crate::app::model_preview::load::TagReader;
use std::collections::HashMap;
use std::sync::Arc;

use super::super::super::common::fields::elements;
use super::super::super::gen3::shaders::Gen3Pass;
use crate::app::model_preview::h4::shield::{DEPTH_DECODE, WORLD_SCALE};
use crate::core::hlsl::emit::{Stage, Translated, translate_with};
use crate::core::hlsl::parse::{Program, parse};
use crate::core::hlsl::parse::{Scalar, Type};
use crate::core::hlsl::pre::{Preprocessor, Token};

/// Where an `#include` falls back to when the including file's own folder
/// doesn't hold it.
const INCLUDE_ROOT: &str = "data\\shaders\\material_shaders\\include";

/// A path as the tool wrote it, folded to one spelling: backslashes, `.` and
/// `..` resolved, lowercase (Windows paths ignore case).
pub(crate) fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['\\', '/']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("\\").to_ascii_lowercase()
}

/// One `material_shader`'s embedded sources.
#[derive(Debug)]
pub(crate) struct H4ShaderSource {
    /// The `.fx` the material shader was compiled from.
    pub(crate) main: Arc<str>,
    /// Normalized path to the path as stored and the text.
    files: HashMap<String, (Arc<str>, Arc<str>)>,
}

impl H4ShaderSource {
    pub(crate) fn read(reader: &TagReader, path: &str) -> Result<Self, String> {
        let tag = reader
            .load(path, "material_shader", b"mats")
            .map_err(|error| format!("{path}: {error}"))?;
        let mut files = HashMap::new();
        let mut main = None;
        for file in elements(tag.root(), "source shader files") {
            let Some(name) = file.read_long_string("shader path") else {
                continue;
            };
            let data = file
                .field("shader data")
                .and_then(|field| field.as_data().map(<[u8]>::to_vec))
                .unwrap_or_default();
            // The tool kept the bytes as read: CRLF line ends, a trailing NUL.
            let text: String = String::from_utf8_lossy(&data)
                .chars()
                .filter(|&c| c != '\0' && c != '\r')
                .collect();
            let name: Arc<str> = Arc::from(name);
            main.get_or_insert_with(|| name.clone());
            files.insert(normalize_path(&name), (name, Arc::from(text)));
        }
        let main = main.ok_or_else(|| format!("{path}: no source shader files"))?;
        Ok(Self { main, files })
    }

    /// An `#include`: beside the including file first, then the include root.
    pub(crate) fn resolve(&self, name: &str, includer: &str) -> Option<(Arc<str>, Arc<str>)> {
        let folder = includer
            .rsplit_once(['\\', '/'])
            .map_or("", |(folder, _)| folder);
        [
            format!("{folder}\\{name}"),
            format!("{INCLUDE_ROOT}\\{name}"),
        ]
        .iter()
        .find_map(|candidate| self.files.get(&normalize_path(candidate)))
        .map(|(path, text)| (path.clone(), text.clone()))
    }

    /// The whole effect, preprocessed.
    pub(crate) fn tokens(&self, defines: &[(&str, &str)]) -> Result<Vec<Token>, String> {
        let (main, text) = self
            .files
            .get(&normalize_path(&self.main))
            .cloned()
            .ok_or("main file missing")?;
        let mut include = |name: &str, includer: &str| {
            self.resolve(name, if includer.is_empty() { &main } else { includer })
        };
        Preprocessor::new(defines, &mut include).run(&text)
    }

    /// The whole effect, preprocessed and parsed.
    pub(crate) fn program(&self, defines: &[(&str, &str)]) -> Result<Program, String> {
        parse(&self.tokens(defines)?)
    }
}

/// The defines every PC material shader compiles with: the DX11 path, and
/// `BLEND_MODE_<mode>` for each `alpha_blend_mode_enum` option by its index,
/// which effect shaders compare `vs_alpha_blend_mode` and
/// `ps_alpha_blend_mode` against (`blend_modes.fxh`) [inferred: the engine's
/// compile passes them].
pub(crate) const H4_DEFINES: [(&str, &str); 26] = [
    ("DX_VERSION", "11"),
    ("BLEND_MODE_opaque", "0"),
    ("BLEND_MODE_additive", "1"),
    ("BLEND_MODE_multiply", "2"),
    ("BLEND_MODE_alpha_blend", "3"),
    ("BLEND_MODE_double_multiply", "4"),
    ("BLEND_MODE_pre_multiplied_alpha", "5"),
    ("BLEND_MODE_maximum", "6"),
    ("BLEND_MODE_multiply_add", "7"),
    ("BLEND_MODE_add_src_times_dstalpha", "8"),
    ("BLEND_MODE_add_src_times_srcalpha", "9"),
    ("BLEND_MODE_inv_alpha_blend", "10"),
    ("BLEND_MODE_motion_blur_static", "11"),
    ("BLEND_MODE_motion_blur_inhibit", "12"),
    ("BLEND_MODE_apply_shadow_into_shadow_mask", "13"),
    ("BLEND_MODE_alpha_blend_constant", "14"),
    ("BLEND_MODE_overdraw_apply", "15"),
    ("BLEND_MODE_wet_screen_effect", "16"),
    ("BLEND_MODE_minimum", "17"),
    ("BLEND_MODE_revsubtract", "18"),
    ("BLEND_MODE_forge_lightmap", "19"),
    ("BLEND_MODE_forge_lightmap_inv", "20"),
    ("BLEND_MODE_replace_all_channels", "21"),
    ("BLEND_MODE_alpha_blend_max", "22"),
    ("BLEND_MODE_opaque_alpha_blend", "23"),
    ("BLEND_MODE_alpha_blend_additive_transparent", "24"),
];

/// One stage of a technique's pass, translated.
pub(crate) fn translate_pass(
    program: &Program,
    technique: &str,
    pass: &str,
    stage: Stage,
) -> Result<Translated, String> {
    let stage_name = match stage {
        Stage::Vertex => "vs",
        Stage::Pixel => "ps",
    };
    // A material technique's pixel shader is its `_default` pass, beside one
    // vertex pass per vertex type; an explicit shader's one pass holds both.
    let pass = match stage {
        Stage::Pixel
            if program
                .pass_function(technique, "_default", stage_name)
                .is_some() =>
        {
            "_default"
        }
        _ => pass,
    };
    let shader = program
        .pass_function(technique, pass, stage_name)
        .ok_or_else(|| format!("no {stage_name} in {technique}/{pass}"))?;
    translate_with(program, &shader.function, stage, &shader.arguments)
}

/// One stage of the `index`th technique's `pass`, for an explicit shader
/// whose techniques go unnamed and are picked by their place
/// (`MAKE_SS_LIGHT_TECHNIQUE`'s sixteen, entry `flags`).
pub(crate) fn translate_technique(
    program: &Program,
    index: usize,
    pass: &str,
    stage: Stage,
) -> Result<Translated, String> {
    let stage_name = match stage {
        Stage::Vertex => "vs",
        Stage::Pixel => "ps",
    };
    let technique = program
        .techniques
        .get(index)
        .ok_or_else(|| format!("no technique {index}"))?;
    let (_, shaders) = technique
        .passes
        .iter()
        .find(|(name, _)| &**name == pass)
        .ok_or_else(|| format!("no pass {pass} in technique {index}"))?;
    let shader = shaders
        .iter()
        .find(|shader| &*shader.stage == stage_name)
        .ok_or_else(|| format!("no {stage_name} in technique {index}/{pass}"))?;
    translate_with(program, &shader.function, stage, &shader.arguments)
}

/// `ComputeDepthFade`'s decode of the game's reversed depth through
/// `psDepthConstants`, made to read the preview's window depth back as world
/// distance from the eye through [`DEPTH_DECODE`] and [`WORLD_SCALE`].
pub(crate) fn rewire_depth_fade(pass: &mut Gen3Pass) -> Result<(), String> {
    const DECODE: &str = "float l_sceneDepth = (1.0 / ((psDepthConstants.x + psDepthConstants.y) - (psDepthConstants.y * l_depthValue.x)));";
    if !pass.pixel.contains(DECODE) {
        return Err("the depth fade's decode is not where it was".to_owned());
    }
    pass.pixel = pass.pixel.replace(
        DECODE,
        &format!(
            "float l_window_ndc = l_depthValue.x * 2.0 - 1.0;
float l_sceneDepth = ((l_window_ndc - {DEPTH_DECODE}.y) / ({DEPTH_DECODE}.x - l_window_ndc * {DEPTH_DECODE}.z) + {DEPTH_DECODE}.w) / {WORLD_SCALE};"
        ),
    );
    pass.pixel = pass.pixel.replacen(
        "uniform vec4 psDepthConstants;",
        &format!("uniform vec4 psDepthConstants;\nuniform vec4 {DEPTH_DECODE};\nuniform float {WORLD_SCALE};"),
        1,
    );
    pass.uniforms
        .push((DEPTH_DECODE.to_owned(), Type::Vector(Scalar::Float, 4)));
    pass.uniforms
        .push((WORLD_SCALE.to_owned(), Type::Scalar(Scalar::Float)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::harness::kit::{kit_root, kit_source};
    use crate::core::game::GameId;
    use crate::core::source::TagSource;

    /// Every object material shader in the kit reads its own sources and
    /// translates the passes an object draws. Water (`materials\water`,
    /// level geometry) reads its triangles from a `StructuredBuffer`, which
    /// waits on the effects work.
    #[test]
    fn every_kit_object_material_shader_translates() {
        let Some(root) = kit_root(GameId::Halo4) else {
            return;
        };
        let folder = root.join("shaders/material_shaders/materials");
        let source = TagSource::LooseFolder {
            root: root.clone(),
            game: Some(GameId::Halo4),
            definitions_root: crate::core::test_kits::definitions().to_path_buf(),
        };
        let mut tags = Vec::new();
        let mut stack = vec![folder];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .extension()
                    .is_some_and(|extension| extension == "material_shader")
                {
                    let relative = path.strip_prefix(&root).unwrap().with_extension("");
                    tags.push(relative.to_string_lossy().replace('/', "\\"));
                }
            }
        }
        assert!(
            tags.len() > 200,
            "the kit's material shaders: {}",
            tags.len()
        );
        let mut failures = Vec::new();
        let mut passes = 0;
        for tag in &tags {
            if tag.contains("\\water\\") {
                continue;
            }
            let program = match H4ShaderSource::read(
                &crate::app::model_preview::load::TagReader::new(&source),
                tag,
            )
            .and_then(|shader| shader.program(&H4_DEFINES))
            {
                Ok(program) => program,
                Err(error) => {
                    failures.push(format!("{tag}: {error}"));
                    continue;
                }
            };
            for technique in ["albedo", "static_probe", "single_pass_single_probe"] {
                if !program
                    .techniques
                    .iter()
                    .any(|candidate| &*candidate.name == technique)
                {
                    continue;
                }
                for (pass, stage) in [
                    ("_default", Stage::Pixel),
                    ("rigid", Stage::Vertex),
                    ("skinned", Stage::Vertex),
                ] {
                    match translate_pass(&program, technique, pass, stage) {
                        Ok(_) => passes += 1,
                        Err(error) => failures.push(format!("{tag} {technique} {pass}: {error}")),
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures, first: {:?}",
            failures.len(),
            &failures[..failures.len().min(5)]
        );
        assert!(passes > 1500, "translated passes: {passes}");
    }

    /// The screen-space light shaders (`rasterizer_globals` explicit 187 and
    /// 204) compile their sixteen techniques from `default_ps`'s uniform
    /// switches; each translates, and the switches reach the GLSL as
    /// constants. A shader whose technique asks no switch (`write_depth`,
    /// explicit 17) still translates, and one past the last does not.
    #[test]
    fn the_screen_space_light_techniques_translate_with_their_switches() {
        let Some(source) = kit_source(GameId::Halo4) else {
            return;
        };
        for path in ["gobo_specular", "pointphysicallycorrect"] {
            let path = format!("shaders\\material_shaders\\explicit_shaders\\lighting\\{path}");
            let program = H4ShaderSource::read(
                &crate::app::model_preview::load::TagReader::new(&source),
                &path,
            )
            .unwrap()
            .program(&H4_DEFINES)
            .unwrap();
            assert_eq!(program.techniques.len(), 16, "{path}");
            for index in 0..16 {
                let pixel = translate_technique(&program, index, "tiny_position", Stage::Pixel)
                    .unwrap_or_else(|error| panic!("{path} {index}: {error}"));
                // `useGobo` is the first switch.
                let gobo = if index & 1 != 0 { "true" } else { "false" };
                assert!(
                    pixel.source.contains(&format!("hlsl_p1 = {gobo};")),
                    "{path} {index}:\n{}",
                    pixel.source
                );
                translate_technique(&program, index, "tiny_position", Stage::Vertex)
                    .unwrap_or_else(|error| panic!("{path} {index} vertex: {error}"));
            }
            assert!(translate_technique(&program, 16, "tiny_position", Stage::Pixel).is_err());
        }
    }

    /// Every effect material shader in the kit (`shaders\\material_shaders\\fx`)
    /// translates its effect passes: particles' `particle` and
    /// `particle_model` vertex shaders, tracers' and light volumes' own,
    /// and the `_default` pixel shader. Their vertex shaders read their
    /// state from a `StructuredBuffer`.
    #[test]
    fn every_kit_effect_material_shader_translates() {
        let Some(root) = kit_root(GameId::Halo4) else {
            return;
        };
        let folder = root.join("shaders/material_shaders/fx");
        let source = TagSource::LooseFolder {
            root: root.clone(),
            game: Some(GameId::Halo4),
            definitions_root: crate::core::test_kits::definitions().to_path_buf(),
        };
        let mut failures = Vec::new();
        let mut passes = 0;
        for entry in std::fs::read_dir(&folder).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".material_shader") else {
                continue;
            };
            let path = format!("shaders\\material_shaders\\fx\\{stem}");
            let program = match H4ShaderSource::read(
                &crate::app::model_preview::load::TagReader::new(&source),
                &path,
            )
            .and_then(|shader| shader.program(&H4_DEFINES))
            {
                Ok(program) => program,
                Err(error) => {
                    failures.push(format!("{stem}: {error}"));
                    continue;
                }
            };
            let Some(technique) = program
                .techniques
                .iter()
                .find(|technique| &*technique.name == "_default")
            else {
                eprintln!(
                    "NOTE {stem}: techniques {:?}",
                    program
                        .techniques
                        .iter()
                        .map(|t| t.name.clone())
                        .collect::<Vec<_>>()
                );
                continue;
            };
            let pass_names: Vec<_> = technique
                .passes
                .iter()
                .map(|(name, _)| name.clone())
                .collect();
            for pass in pass_names {
                let stage = if &*pass == "_default" {
                    Stage::Pixel
                } else {
                    Stage::Vertex
                };
                match translate_pass(&program, "_default", &pass, stage) {
                    Ok(translated) => {
                        passes += 1;
                        if stage == Stage::Vertex && !translated.source.contains("usamplerBuffer") {
                            eprintln!("NOTE {stem}/{pass}: no buffer");
                        }
                    }
                    Err(error) => failures.push(format!("{stem} {pass}: {error}")),
                }
            }
        }
        for failure in &failures {
            eprintln!("NOTE FAIL {failure}");
        }
        assert!(failures.is_empty(), "{} failures", failures.len());
        assert!(passes > 40, "translated passes: {passes}");
    }

    #[test]
    fn paths_fold_case_folders_and_parents() {
        assert_eq!(
            normalize_path(
                "data\\shaders\\material_shaders\\include\\parameters\\..\\core\\Core.fxh"
            ),
            "data\\shaders\\material_shaders\\include\\core\\core.fxh"
        );
        assert_eq!(normalize_path("a/./b//c"), "a\\b\\c");
    }

    #[test]
    fn includes_look_beside_the_includer_before_the_root() {
        let file =
            |path: &str, text: &str| (normalize_path(path), (Arc::from(path), Arc::from(text)));
        let source = H4ShaderSource {
            main: Arc::from("data\\shaders\\material_shaders\\materials\\a.fx"),
            files: HashMap::from([
                file("data\\shaders\\material_shaders\\materials\\a.fx", "main"),
                file(
                    "data\\shaders\\material_shaders\\materials\\common.fxh",
                    "beside",
                ),
                file(
                    "data\\shaders\\material_shaders\\include\\common.fxh",
                    "root",
                ),
                file(
                    "data\\shaders\\material_shaders\\include\\core\\core.fxh",
                    "core",
                ),
            ]),
        };
        let materials = "data\\shaders\\material_shaders\\materials\\a.fx";
        assert_eq!(
            &*source.resolve("common.fxh", materials).unwrap().1,
            "beside"
        );
        let core = "data\\shaders\\material_shaders\\include\\core\\core.fxh";
        assert_eq!(&*source.resolve("common.fxh", core).unwrap().1, "root");
        assert_eq!(
            &*source.resolve("core/core.fxh", materials).unwrap().1,
            "core"
        );
        assert!(source.resolve("missing.fxh", materials).is_none());
    }
}
