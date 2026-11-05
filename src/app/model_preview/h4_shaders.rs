//! Halo 4's material shaders, from the source each tag carries.
//!
//! A `material_shader` embeds every file its offline compile opened, its
//! `.fx` first, under `source shader files` (`data\shaders\material_shaders\..`
//! paths). The `.fx` is a D3DX effect: one `technique11` per entry point, a
//! `_default` pass naming the pixel shader and a pass per vertex kind
//! (`world`, `rigid`, `skinned`, `rigid_boned`, `rigid_blendshaped`,
//! `skinned_blendshaped`) naming its vertex shader. `DX_VERSION=11` is the
//! only define the compile needs; `pc` follows from it in `core.fxh`.

use std::collections::HashMap;
use std::sync::Arc;

use super::ce_attachments::elements;
use crate::app::export::geometry::load_referenced_tag_from_source;
use crate::core::hlsl::emit::{Stage, Translated, translate_with};
use crate::core::hlsl::parse::{Program, parse};
use crate::core::hlsl::pre::{Preprocessor, Token};
use crate::core::source::TagSource;

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
    pub(crate) fn read(source: &TagSource, path: &str) -> Result<Self, String> {
        let tag = load_referenced_tag_from_source(source, path, "material_shader", b"mats")
            .map_err(|error| format!("{path}: {error}"))?;
        let mut files = HashMap::new();
        let mut main = None;
        for file in elements(tag.root(), "source shader files") {
            let Some(name) = file.read_long_string("shader path") else { continue };
            let data = file.field("shader data").and_then(|field| field.as_data().map(<[u8]>::to_vec)).unwrap_or_default();
            // The tool kept the bytes as read: CRLF line ends, a trailing NUL.
            let text: String = String::from_utf8_lossy(&data).chars().filter(|&c| c != '\0' && c != '\r').collect();
            let name: Arc<str> = Arc::from(name);
            main.get_or_insert_with(|| name.clone());
            files.insert(normalize_path(&name), (name, Arc::from(text)));
        }
        let main = main.ok_or_else(|| format!("{path}: no source shader files"))?;
        Ok(Self { main, files })
    }

    /// An `#include`: beside the including file first, then the include root.
    pub(crate) fn resolve(&self, name: &str, includer: &str) -> Option<(Arc<str>, Arc<str>)> {
        let folder = includer.rsplit_once(['\\', '/']).map_or("", |(folder, _)| folder);
        [format!("{folder}\\{name}"), format!("{INCLUDE_ROOT}\\{name}")]
            .iter()
            .find_map(|candidate| self.files.get(&normalize_path(candidate)))
            .map(|(path, text)| (path.clone(), text.clone()))
    }

    /// The whole effect, preprocessed.
    pub(crate) fn tokens(&self, defines: &[(&str, &str)]) -> Result<Vec<Token>, String> {
        let (main, text) = self.files.get(&normalize_path(&self.main)).cloned().ok_or("main file missing")?;
        let mut include = |name: &str, includer: &str| self.resolve(name, if includer.is_empty() { &main } else { includer });
        Preprocessor::new(defines, &mut include).run(&text)
    }

    /// The whole effect, preprocessed and parsed.
    pub(crate) fn program(&self, defines: &[(&str, &str)]) -> Result<Program, String> {
        parse(&self.tokens(defines)?)
    }
}

/// The defines every PC material shader compiles with.
pub(crate) const H4_DEFINES: [(&str, &str); 1] = [("DX_VERSION", "11")];

/// One stage of a technique's pass, translated.
pub(crate) fn translate_pass(program: &Program, technique: &str, pass: &str, stage: Stage) -> Result<Translated, String> {
    let stage_name = match stage {
        Stage::Vertex => "vs",
        Stage::Pixel => "ps",
    };
    // A material technique's pixel shader is its `_default` pass, beside one
    // vertex pass per vertex type; an explicit shader's one pass holds both.
    let pass = match stage {
        Stage::Pixel if program.pass_function(technique, "_default", stage_name).is_some() => "_default",
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
pub(crate) fn translate_technique(program: &Program, index: usize, pass: &str, stage: Stage) -> Result<Translated, String> {
    let stage_name = match stage {
        Stage::Vertex => "vs",
        Stage::Pixel => "ps",
    };
    let technique = program.techniques.get(index).ok_or_else(|| format!("no technique {index}"))?;
    let (_, shaders) = technique.passes.iter().find(|(name, _)| &**name == pass).ok_or_else(|| format!("no pass {pass} in technique {index}"))?;
    let shader = shaders.iter().find(|shader| &*shader.stage == stage_name).ok_or_else(|| format!("no {stage_name} in technique {index}/{pass}"))?;
    translate_with(program, &shader.function, stage, &shader.arguments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::game::GameId;

    /// Every object material shader in the kit reads its own sources and
    /// translates the passes an object draws. Water (`materials\water`,
    /// level geometry) reads its triangles from a `StructuredBuffer`, which
    /// waits on the effects work.
    #[test]
    fn every_kit_object_material_shader_translates() {
        let root = std::path::PathBuf::from(crate::core::test_kits::tag_path("halo4_mcc", ""));
        let folder = root.join("shaders/material_shaders/materials");
        if !folder.is_dir() {
            eprintln!("skipped: set BLAM_TEST_H4EK to a Halo 4 kit's tags folder");
            return;
        }
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
                } else if path.extension().is_some_and(|extension| extension == "material_shader") {
                    let relative = path.strip_prefix(&root).unwrap().with_extension("");
                    tags.push(relative.to_string_lossy().replace('/', "\\"));
                }
            }
        }
        assert!(tags.len() > 200, "the kit's material shaders: {}", tags.len());
        let mut failures = Vec::new();
        let mut passes = 0;
        for tag in &tags {
            if tag.contains("\\water\\") {
                continue;
            }
            let program = match H4ShaderSource::read(&source, tag).and_then(|shader| shader.program(&H4_DEFINES)) {
                Ok(program) => program,
                Err(error) => {
                    failures.push(format!("{tag}: {error}"));
                    continue;
                }
            };
            for technique in ["albedo", "static_probe", "single_pass_single_probe"] {
                if !program.techniques.iter().any(|candidate| &*candidate.name == technique) {
                    continue;
                }
                for (pass, stage) in [("_default", Stage::Pixel), ("rigid", Stage::Vertex), ("skinned", Stage::Vertex)] {
                    match translate_pass(&program, technique, pass, stage) {
                        Ok(_) => passes += 1,
                        Err(error) => failures.push(format!("{tag} {technique} {pass}: {error}")),
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{} failures, first: {:?}", failures.len(), &failures[..failures.len().min(5)]);
        assert!(passes > 1500, "translated passes: {passes}");
    }

    /// The screen-space light shaders (`rasterizer_globals` explicit 187 and
    /// 204) compile their sixteen techniques from `default_ps`'s uniform
    /// switches; each translates, and the switches reach the GLSL as
    /// constants. A shader whose technique asks no switch (`write_depth`,
    /// explicit 17) still translates, and one past the last does not.
    #[test]
    fn the_screen_space_light_techniques_translate_with_their_switches() {
        let root = std::path::PathBuf::from(crate::core::test_kits::tag_path("halo4_mcc", ""));
        if !root.join("shaders/material_shaders/explicit_shaders/lighting").is_dir() {
            eprintln!("skipped: set BLAM_TEST_H4EK to a Halo 4 kit's tags folder");
            return;
        }
        let source = TagSource::LooseFolder {
            root,
            game: Some(GameId::Halo4),
            definitions_root: crate::core::test_kits::definitions().to_path_buf(),
        };
        for path in ["gobo_specular", "pointphysicallycorrect"] {
            let path = format!("shaders\\material_shaders\\explicit_shaders\\lighting\\{path}");
            let program = H4ShaderSource::read(&source, &path).unwrap().program(&H4_DEFINES).unwrap();
            assert_eq!(program.techniques.len(), 16, "{path}");
            for index in 0..16 {
                let pixel = translate_technique(&program, index, "tiny_position", Stage::Pixel)
                    .unwrap_or_else(|error| panic!("{path} {index}: {error}"));
                // `useGobo` is the first switch.
                let gobo = if index & 1 != 0 { "true" } else { "false" };
                assert!(pixel.source.contains(&format!("hlsl_p1 = {gobo};")), "{path} {index}:\n{}", pixel.source);
                translate_technique(&program, index, "tiny_position", Stage::Vertex)
                    .unwrap_or_else(|error| panic!("{path} {index} vertex: {error}"));
            }
            assert!(translate_technique(&program, 16, "tiny_position", Stage::Pixel).is_err());
        }
    }

    #[test]
    fn paths_fold_case_folders_and_parents() {
        assert_eq!(
            normalize_path("data\\shaders\\material_shaders\\include\\parameters\\..\\core\\Core.fxh"),
            "data\\shaders\\material_shaders\\include\\core\\core.fxh"
        );
        assert_eq!(normalize_path("a/./b//c"), "a\\b\\c");
    }

    #[test]
    fn includes_look_beside_the_includer_before_the_root() {
        let file = |path: &str, text: &str| (normalize_path(path), (Arc::from(path), Arc::from(text)));
        let source = H4ShaderSource {
            main: Arc::from("data\\shaders\\material_shaders\\materials\\a.fx"),
            files: HashMap::from([
                file("data\\shaders\\material_shaders\\materials\\a.fx", "main"),
                file("data\\shaders\\material_shaders\\materials\\common.fxh", "beside"),
                file("data\\shaders\\material_shaders\\include\\common.fxh", "root"),
                file("data\\shaders\\material_shaders\\include\\core\\core.fxh", "core"),
            ]),
        };
        let materials = "data\\shaders\\material_shaders\\materials\\a.fx";
        assert_eq!(&*source.resolve("common.fxh", materials).unwrap().1, "beside");
        let core = "data\\shaders\\material_shaders\\include\\core\\core.fxh";
        assert_eq!(&*source.resolve("common.fxh", core).unwrap().1, "root");
        assert_eq!(&*source.resolve("core/core.fxh", materials).unwrap().1, "core");
        assert!(source.resolve("missing.fxh", materials).is_none());
    }
}

