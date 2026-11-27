//! The kit's shader sources and their includes.

use super::super::super::TagReader;
use super::defines::{base_defines, option_defines};
use super::definition::{Gen3Definition, VERTEX_SKINNED, entry_functions, read_h3_definition};
use super::dialect::RenderMethodDialect;
use super::pass::{Gen3Pass, SCREEN_VERTEX, pass_of};
use super::{Preprocessor, Stage, TagSource, Translated, parse};
use crate::core::hlsl::emit::{translate, translate_with_integer_inputs};
use crate::core::hlsl::parse::Program;
use crate::core::hlsl::pre::Lines;
use std::collections::HashMap;
use std::sync::Arc;

/// The tag behind an `#include`. Halo 3 keeps every source in one folder:
/// `global.fx` is `shaders\global_fx`. Reach's `c_tag_d3dx_include`
/// (tool `sub_141219410`) keeps the folder and cuts the last extension:
/// `templated\albedo.fx` is `shaders\templated\albedo`.
fn include_tag(dialect: RenderMethodDialect, name: &str) -> String {
    match dialect {
        RenderMethodDialect::Halo3 => {
            let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
            format!("shaders\\{}", file.replace('.', "_"))
        }
        RenderMethodDialect::Reach => {
            let path = name.replace('/', "\\");
            let stem = path
                .rsplit_once('.')
                .map_or(path.as_str(), |(stem, _)| stem);
            format!("shaders\\{stem}")
        }
    }
}

/// Headers a kit's sources include that the kit has no tag of their own for,
/// with what the game's compiled shaders show they held. Reach's
/// `decorators.hlsl` includes `decorators\decorators.h`, which the include
/// rule above cuts to `decorators\decorators`, the decorator source itself;
/// the header only sets the alpha test, and Reach's compiled decorator
/// shaders clip at 0.5.
const HEADER_SHIMS: [(RenderMethodDialect, &str, &str); 1] = [(
    RenderMethodDialect::Reach,
    "decorators\\decorators.h",
    "#define k_decorator_alpha_test_threshold 0.5f\n",
)];

/// A kit's sources, read once.
pub(crate) struct Gen3Sources {
    /// Its own reader: the definitions and files below keep what it reads.
    reader: TagReader,
    dialect: RenderMethodDialect,
    definitions: HashMap<String, Option<Arc<Gen3Definition>>>,
    /// Each `hlsl_include` read, split into lines once for every compile.
    files: HashMap<String, Option<Arc<Lines>>>,
    passes: HashMap<(String, [Vec<Option<usize>>; 2], String, bool), Arc<Gen3Pass>>,
}

impl Gen3Sources {
    pub(crate) fn new(source: &TagSource) -> Self {
        let dialect = RenderMethodDialect::of(source);
        let reader = TagReader::new(source);
        Self {
            reader,
            dialect,
            definitions: HashMap::new(),
            files: HashMap::new(),
            passes: HashMap::new(),
        }
    }

    pub(crate) fn dialect(&self) -> RenderMethodDialect {
        self.dialect
    }

    pub(crate) fn definition(&mut self, path: &str) -> Option<Arc<Gen3Definition>> {
        let reader = &self.reader;
        self.definitions
            .entry(path.to_owned())
            .or_insert_with(|| read_h3_definition(reader, path).map(Arc::new))
            .clone()
    }

    fn file(&mut self, name: &str) -> Option<Arc<Lines>> {
        let (reader, dialect) = (&self.reader, self.dialect);
        self.files
            .entry(name.to_owned())
            .or_insert_with(|| {
                let shim = HEADER_SHIMS.iter().find(|(shimmed, header, _)| {
                    *shimmed == dialect && header.eq_ignore_ascii_case(&name.replace('/', "\\"))
                });
                if let Some((_, _, text)) = shim {
                    return Some(Arc::new(Lines::of(text)));
                }
                let tag = reader
                    .load(&include_tag(dialect, name), "hlsl_include", b"hlsl")
                    .ok()?;
                let data = tag.root().field("include file")?.as_data()?.to_vec();
                Some(Arc::new(Lines::of(
                    String::from_utf8_lossy(&data).trim_end_matches('\0'),
                )))
            })
            .clone()
    }

    /// `sub_140C587A0`'s defines, then the family's file for one stage.
    fn stage(
        &mut self,
        definition: &Gen3Definition,
        options: &[usize],
        entry: &str,
        vertex_type: usize,
        stage: Stage,
        own_albedo: bool,
    ) -> Result<Translated, String> {
        let mut defines = base_defines(self.dialect, vertex_type, stage);
        defines.push(("entry_point".to_owned(), entry.to_owned()));
        // `sub_140C587A0`: a blended shader has no albedo pass to read back.
        // Reach's sources never read it; its blended shaders go by `blend_type`.
        if own_albedo && self.dialect == RenderMethodDialect::Halo3 {
            defines.push(("maybe_calc_albedo".to_owned(), "1".to_owned()));
        }
        defines.extend(option_defines(
            definition,
            &definition.stage_options(options, entry, stage),
        ));
        let main = format!("#include \"{}\"\n", definition.source_file);
        let (vertex, pixel) = entry_functions(self.dialect, entry);
        self.compile(
            &defines,
            &main,
            if stage == Stage::Vertex {
                &vertex
            } else {
                &pixel
            },
            stage,
        )
    }

    pub(in crate::app::model_preview) fn compile(
        &mut self,
        defines: &[(String, String)],
        main: &str,
        entry: &str,
        stage: Stage,
    ) -> Result<Translated, String> {
        translate(&self.program(defines, main)?, entry, stage)
    }

    /// [`Self::compile`], with the vertex inputs `integer_inputs` names fed
    /// as integers ([`translate_with_integer_inputs`]).
    fn compile_feeding(
        &mut self,
        defines: &[(String, String)],
        main: &str,
        entry: &str,
        stage: Stage,
        integer_inputs: &[&str],
    ) -> Result<Translated, String> {
        translate_with_integer_inputs(&self.program(defines, main)?, entry, stage, integer_inputs)
    }

    /// `main` preprocessed with `defines` and its includes, and parsed.
    fn program(&mut self, defines: &[(String, String)], main: &str) -> Result<Program, String> {
        let borrowed: Vec<_> = defines
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let mut include = |name: &str, _: &str| self.file(name).map(|lines| (Arc::from(name), lines));
        let tokens = Preprocessor::new(&borrowed, &mut include).run(main)?;
        parse(&tokens)
    }

    /// A decorator set's explicit shader, `file`: Halo 3's `shaders\<file>`,
    /// Reach's `shaders\decorators\<file>`. Reach's packs a placement's
    /// position into one `uint`, `POSITION1`, which arrives as one.
    pub(crate) fn decorator_pass(&mut self, file: &str) -> Result<Arc<Gen3Pass>, String> {
        let (main, integer_inputs): (String, &[&str]) = match self.dialect {
            RenderMethodDialect::Halo3 => (format!("#include \"{file}.hlsl\"\n"), &[]),
            RenderMethodDialect::Reach => (format!("#include \"decorators\\{file}.hlsl\"\n"), &["POSITION1"]),
        };
        let vertex_defines = base_defines(self.dialect, VERTEX_SKINNED, Stage::Vertex);
        let pixel_defines = base_defines(self.dialect, VERTEX_SKINNED, Stage::Pixel);
        let vertex = self.compile_feeding(&vertex_defines, &main, "default_vs", Stage::Vertex, integer_inputs)?;
        let pixel = self.compile(&pixel_defines, &main, "default_ps", Stage::Pixel)?;
        Ok(Arc::new(pass_of(vertex, pixel)))
    }

    /// One of the engine's own `.hlsl` passes, on a skinned vertex.
    pub(crate) fn explicit_pass(&mut self, file: &str) -> Result<Arc<Gen3Pass>, String> {
        // Reach keeps its explicit shaders in `shaders\explicit`.
        let main = match self.dialect {
            RenderMethodDialect::Halo3 => format!("#include \"{file}.hlsl\"\n"),
            RenderMethodDialect::Reach => format!("#include \"explicit\\{file}.hlsl\"\n"),
        };
        let vertex_defines = base_defines(self.dialect, VERTEX_SKINNED, Stage::Vertex);
        let pixel_defines = base_defines(self.dialect, VERTEX_SKINNED, Stage::Pixel);
        let vertex = self.compile(&vertex_defines, &main, "default_vs", Stage::Vertex)?;
        let pixel = self.compile(&pixel_defines, &main, "default_ps", Stage::Pixel)?;
        Ok(Arc::new(pass_of(vertex, pixel)))
    }

    /// A `shader_screen`'s pixel shader, fed by a triangle over the picture
    /// rather than the world vertices its family's `default_vs` reads.
    pub(crate) fn screen_pass(
        &mut self,
        definition: &Gen3Definition,
        options: &[usize],
    ) -> Result<Arc<Gen3Pass>, String> {
        let (vertex, pixel) = self.stages(definition, options, "default", VERTEX_SKINNED, false)?;
        let mut pass = pass_of(vertex, pixel);
        pass.vertex = SCREEN_VERTEX.to_owned();
        pass.fullscreen = true;
        Ok(Arc::new(pass))
    }

    /// An entry point's pass, as the draw would link it.
    #[cfg(test)]
    pub(crate) fn translate_entry(
        &mut self,
        definition: &Gen3Definition,
        options: &[usize],
        entry: &str,
        vertex_type: usize,
    ) -> Result<Gen3Pass, String> {
        let (vertex, pixel) = self.stages(definition, options, entry, vertex_type, false)?;
        Ok(pass_of(vertex, pixel))
    }

    fn stages(
        &mut self,
        definition: &Gen3Definition,
        options: &[usize],
        entry: &str,
        vertex_type: usize,
        own_albedo: bool,
    ) -> Result<(Translated, Translated), String> {
        let vertex = self.stage(
            definition,
            options,
            entry,
            vertex_type,
            Stage::Vertex,
            own_albedo,
        )?;
        let pixel_type = definition.pixel_vertex_type;
        let pixel = self.stage(
            definition,
            options,
            entry,
            pixel_type,
            Stage::Pixel,
            own_albedo,
        )?;
        Ok((vertex, pixel))
    }
}

impl Gen3Sources {
    /// An entry point's pass, kept for the next material of its template.
    pub(crate) fn pass(
        &mut self,
        definition: &Gen3Definition,
        options: &[usize],
        entry: &str,
        own_albedo: bool,
    ) -> Result<Arc<Gen3Pass>, String> {
        // Reach materials that differ only where a shared stage looks away
        // compile the same pass.
        let options_of = |stage| definition.stage_options(options, entry, stage);
        let key = (
            definition.name.clone(),
            [options_of(Stage::Vertex), options_of(Stage::Pixel)],
            entry.to_owned(),
            own_albedo,
        );
        if let Some(pass) = self.passes.get(&key) {
            return Ok(Arc::clone(pass));
        }
        let (vertex, pixel) =
            self.stages(definition, options, entry, VERTEX_SKINNED, own_albedo)?;
        let pass = Arc::new(pass_of(vertex, pixel));
        self.passes.insert(key, Arc::clone(&pass));
        Ok(pass)
    }
}
