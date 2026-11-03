//! Halo 3's and Reach's shaders, as their tools compile them.
//!
//! A `render_method_definition` lists categories, each naming a function that
//! the chosen option's function stands in for, and one source file is
//! compiled per entry point with those as defines: `shaders\<name>.fx` in
//! Halo 3, the definition's own `location` in Reach. The sources live in
//! `.hlsl_include` tags, read from the user's kit.

use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::bitmap::BitmapFormat;
use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode, BLEND_MODES, REACH_BLEND_MODES};

use blam_tags::TagStruct;
use blam_tags::tag_function::FunctionType;

use super::ce_attachments::elements;
use super::h2_functions::H2FunctionValues;
use super::h2_passes::H2Image;
use super::*;
use crate::core::hlsl::emit::{Stage, Translated, translate};
use crate::core::hlsl::parse::{TextureKind, Type, parse};
use crate::core::hlsl::pre::Preprocessor;
use crate::core::source::TagSource;

/// tool's tables of entry point, vertex type and deform names.
#[cfg(test)]
pub(crate) const VERTEX_RIGID: usize = 1;
pub(crate) const VERTEX_SKINNED: usize = 2;
const VERTEX_TYPES: [(&str, &str); 3] = [
    ("s_world_vertex", "deform_world"),
    ("s_rigid_vertex", "deform_rigid"),
    ("s_skinned_vertex", "deform_skinned"),
];

#[derive(Debug, Clone)]
struct H3Option {
    name: String,
    vertex_function: Option<String>,
    pixel_function: Option<String>,
}

#[derive(Debug, Clone)]
struct H3Category {
    name: String,
    vertex_function: Option<String>,
    pixel_function: Option<String>,
    options: Vec<H3Option>,
}

/// What Reach's tool reads of an entry point's first pass to pick the
/// options each stage compiles with (`sub_140342850` builds the shared
/// shaders; `get_compiled_vertex_shader` 0x14119A2A0 and
/// `get_compiled_pixel_shader` 0x14119A170 choose among them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct H3SharedStages {
    /// Flag 0x1: the pixel shader is shared, not the material's own.
    shared_pixel: bool,
    /// Flag 0x2: the vertex shaders only tell the default option of their
    /// category from any other, so only options 0 and 1 are compiled.
    vertex_two_options: bool,
    vertex_category: Option<usize>,
    pixel_category: Option<usize>,
}

/// A `render_method_definition`, as far as compiling goes.
#[derive(Debug, Clone)]
pub(crate) struct H3Definition {
    /// The family: `shader`, `terrain`..
    pub(crate) name: String,
    dialect: H3Dialect,
    /// The file it compiles, as an `#include` names it: Halo 3's `shader.fx`,
    /// Reach's `templated\shader.fx`.
    source_file: String,
    /// `use automatic macros`: categories and options named as defines too.
    automatic_macros: bool,
    categories: Vec<H3Category>,
    pub(crate) entry_points: Vec<String>,
    /// Reach's shared-shader rules by entry point; Halo 3 has none, and
    /// compiles both stages with the material's options.
    shared: HashMap<String, H3SharedStages>,
    /// The first `vertex_types` entry, which every pixel shader compiles with.
    pixel_vertex_type: usize,
}

pub(crate) fn read_h3_definition(source: &TagSource, path: &str) -> Option<H3Definition> {
    let dialect = H3Dialect::of(source);
    let tag = load_referenced_tag_from_source(source, path, "render_method_definition", b"rmdf").ok()?;
    let root = tag.root();
    let function = |holder: TagStruct<'_>, name: &str| holder.read_string_id(name).filter(|name| !name.is_empty());
    let categories = elements(root, "categories")
        .into_iter()
        .map(|category| H3Category {
            name: category.read_string_id("category name").unwrap_or_default(),
            vertex_function: function(category, "vertex function"),
            pixel_function: function(category, "pixel function"),
            options: elements(category, "options")
                .into_iter()
                .map(|option| H3Option {
                    name: option.read_string_id("option name").unwrap_or_default(),
                    vertex_function: function(option, "vertex function"),
                    pixel_function: function(option, "pixel function"),
                })
                .collect(),
        })
        .collect();
    // Reach lists entry points with no passes (glass's and mux's `default`),
    // which tool compiles nothing for and the source has no function for.
    let mut entry_points = Vec::new();
    let mut shared = HashMap::new();
    for entry in elements(root, "entry_points") {
        let passes = elements(entry, "passes");
        if dialect == H3Dialect::Reach && passes.is_empty() {
            continue;
        }
        let Some(name) = entry.read_enum_name("entry point") else {
            continue;
        };
        let name = entry_name(dialect, &name).to_owned();
        if dialect == H3Dialect::Reach {
            let first = passes[0];
            let flags = first.read_int_any("flags").unwrap_or(0);
            // `sub_140342850` asserts a stage depends on fewer than two.
            let category = |block: &str| {
                elements(first, block).first().and_then(|dependency| dependency.read_int_any("category"))
                    .and_then(|category| usize::try_from(category).ok())
            };
            shared.insert(name.clone(), H3SharedStages {
                shared_pixel: flags & 1 != 0,
                vertex_two_options: flags & 2 != 0,
                vertex_category: category("shared VS category dependencies"),
                pixel_category: category("category dependencies"),
            });
        }
        entry_points.push(name);
    }
    let name = path.rsplit('\\').next()?.to_owned();
    // Reach: `c_render_method::postprocess` (tool 0x141165700) compiles
    // `shaders\<location>`; `generate-shared-shaders` (0x140345BD0) falls back on
    // `templated\<name>` when it is empty. Halo 3 has no `location`.
    let source_file = match root.read_string("location") {
        Some(location) if !location.is_empty() => format!("{location}.fx"),
        Some(_) => format!("templated\\{name}.fx"),
        None => format!("{name}.fx"),
    };
    let automatic_macros = root.read_int_any("flags").unwrap_or(0) & 1 != 0;
    let pixel_vertex_type = elements(root, "vertex_types")
        .first()
        .and_then(|first| first.read_int_any("vertex type"))
        .and_then(|kind| usize::try_from(kind).ok())
        .filter(|kind| *kind < VERTEX_TYPES.len())
        .unwrap_or(0);
    Some(H3Definition { name, dialect, source_file, automatic_macros, categories, entry_points, shared, pixel_vertex_type })
}

impl H3Definition {
    pub(crate) fn is_decal(&self) -> bool {
        self.name == "decal"
    }

    /// The name of the option chosen in a category.
    pub(crate) fn chosen(&self, category: &str, options: &[usize]) -> Option<&str> {
        let at = self.categories.iter().position(|known| known.name == category)?;
        let option = options.get(at).copied().unwrap_or(0);
        Some(self.categories[at].options.get(option)?.name.as_str())
    }

    /// A decal's `render_pass`: it goes on before the lights or after them.
    pub(crate) fn after_lighting(&self, options: &[usize]) -> bool {
        self.chosen("render_pass", options) == Some("post_lighting")
    }

    /// The options one stage of `entry` compiles with: the material's own in
    /// Halo 3. In Reach a vertex shader is shared by every material of the
    /// family, compiled with each option at its default (none, under
    /// automatic macros) except the one category it depends on; a shared
    /// pixel shader the same.
    fn stage_options(&self, options: &[usize], entry: &str, stage: Stage) -> Vec<Option<usize>> {
        let own = || options.iter().copied().map(Some).collect();
        let Some(rules) = self.shared.get(entry) else {
            return own();
        };
        let (category, most) = match stage {
            Stage::Vertex => (rules.vertex_category, if rules.vertex_two_options { 2 } else { usize::MAX }),
            Stage::Pixel if rules.shared_pixel => (rules.pixel_category, usize::MAX),
            Stage::Pixel => return own(),
        };
        let default = (!self.automatic_macros).then_some(0);
        let mut chosen = vec![default; self.categories.len()];
        if let Some(category) = category.filter(|category| *category < chosen.len()) {
            let count = self.categories[category].options.len().min(most);
            let option = options.get(category).copied().unwrap_or(0);
            chosen[category] = Some(option.min(count.saturating_sub(1)));
        }
        chosen
    }

    /// The `blend_mode` category's choice, for a method held in another tag.
    pub(crate) fn blend(&self, options: &[usize]) -> H3Blend {
        self.blend_of(options, false)
    }

    /// The `blend_mode` category's choice, as each game's bake reads its name:
    /// Halo 3's `blend_mode_from_string` ignoring case, Reach's
    /// `get_alpha_blend_mode_by_name` (0x141167210) minding it. Without the
    /// category Reach blends `shader_glass` and nothing else.
    pub(crate) fn blend_of(&self, options: &[usize], glass: bool) -> H3Blend {
        let Some(name) = self.chosen("blend_mode", options) else {
            return if glass && self.dialect == H3Dialect::Reach { H3Blend::AlphaBlend } else { H3Blend::Opaque };
        };
        let index = match self.dialect {
            H3Dialect::Halo3 => {
                let name = name.to_ascii_lowercase();
                BLEND_MODES.iter().position(|mode| name.starts_with(mode))
            }
            // Reach's entries past `motion_blur_inhibit` are its own; no
            // Reach definition offers them as options, so none reach here.
            H3Dialect::Reach => REACH_BLEND_MODES[..13].iter().position(|mode| name.starts_with(mode)),
        };
        H3Blend::ALL[index.unwrap_or(0)]
    }
}

/// tool's name for an entry point, where the tag's enum spells it for people.
fn entry_name(dialect: H3Dialect, option: &str) -> &str {
    match (dialect, option) {
        (_, "vertex color lighting") => "static_per_vertex_color",
        (_, "water tessellation") => "water_tessellation",
        (_, "water shading") => "water_shading",
        // Reach tool's `entry_point` table, `off_14290E110`.
        (H3Dialect::Reach, "imposter_static_lighting_sh") => "imposter_static_sh",
        (H3Dialect::Reach, "imposter_static_lighting_prt_ambient") => "imposter_static_prt_ambient",
        (_, other) => other,
    }
}

/// The function each stage compiles. Some entry points share one. Reach's
/// are tool's `off_14290DCC0` (vertex) and `off_14290DBD0` (pixel).
fn entry_functions(dialect: H3Dialect, entry: &str) -> (String, String) {
    let pair = |vertex: &str, pixel: &str| (vertex.to_owned(), pixel.to_owned());
    match (dialect, entry) {
        (_, "static_prt_ambient" | "static_prt_linear" | "static_prt_quadratic") => {
            (format!("{entry}_vs"), "static_prt_ps".to_owned())
        }
        (H3Dialect::Halo3, "dynamic_light_cinematic") => pair("dynamic_light_cine_vs", "dynamic_light_cine_ps"),
        (H3Dialect::Reach, "dynamic_light") => pair("default_dynamic_light_vs", "dynamic_light_ps"),
        (H3Dialect::Reach, "dynamic_light_cinematic") => pair("default_dynamic_light_vs", "dynamic_light_cine_ps"),
        (H3Dialect::Reach, "dynamic_light_hq_shadows") => pair("default_dynamic_light_vs", "dynamic_light_hq_shadows_ps"),
        (H3Dialect::Reach, "dynamic_light_cinematic_hq_shadows") => {
            pair("default_dynamic_light_vs", "dynamic_light_cine_hq_shadows_ps")
        }
        (H3Dialect::Reach, "imposter_static_prt_ambient") => pair("imposter_static_prt_ambient_vs", "imposter_static_prt_ps"),
        (_, other) => (format!("{other}_vs"), format!("{other}_ps")),
    }
}

/// `sub_140C587A0`: what every shader compiles with. Reach's `sub_141171B10`
/// adds `_standard_vs`, without which every vertex shader body is compiled
/// out, and `disable_register_reorder`.
fn base_defines(dialect: H3Dialect, vertex_type: usize, stage: Stage) -> Vec<(String, String)> {
    let (vertex, deform) = VERTEX_TYPES[vertex_type];
    let stage = match stage {
        Stage::Vertex => "VERTEX_SHADER",
        Stage::Pixel => "PIXEL_SHADER",
    };
    let mut defines = vec![
        ("vertex_type", vertex),
        ("deform", deform),
        ("SHADER_30", "1"),
        ("pc", "1"),
        ("DX_VERSION", "11"),
        ("PARAM_ALLOC_FIRST_PASS", "1"),
        (stage, "1"),
    ];
    if dialect == H3Dialect::Reach {
        defines.extend([("_standard_vs", "1"), ("disable_register_reorder", "1")]);
    }
    defines.into_iter().map(|(name, value)| (name.to_owned(), value.to_owned())).collect()
}

/// `sub_140C589E0`: each category's function, as its option's. Reach's
/// `build_options_macro` (0x141171D10) also takes no option at all, which
/// writes no function and no choice but still names every option.
fn option_defines(definition: &H3Definition, options: &[Option<usize>]) -> Vec<(String, String)> {
    let mut defines = Vec::new();
    for (index, category) in definition.categories.iter().enumerate() {
        let chosen = options.get(index).copied().unwrap_or(Some(0));
        let chosen = chosen.map(|chosen| chosen.min(category.options.len().saturating_sub(1)));
        let option = chosen.and_then(|chosen| category.options.get(chosen));
        if option.is_none() && !(chosen.is_none() && definition.automatic_macros) {
            continue;
        }
        for (name, value) in [
            (&category.vertex_function, option.and_then(|option| option.vertex_function.as_ref())),
            (&category.pixel_function, option.and_then(|option| option.pixel_function.as_ref())),
        ] {
            if let (Some(name), Some(value)) = (name, value) {
                defines.push((name.clone(), value.clone()));
            }
        }
        if definition.automatic_macros {
            let prefix = format!("category_{}", category.name);
            if let Some(option) = option {
                defines.push((prefix.clone(), format!("{prefix}_option_{}", option.name)));
            }
            for (index, option) in category.options.iter().enumerate() {
                defines.push((format!("{prefix}_option_{}", option.name), index.to_string()));
            }
        }
    }
    defines
}

/// Whose tool lays out the sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H3Dialect {
    Halo3,
    Reach,
}

impl H3Dialect {
    fn of(source: &TagSource) -> Self {
        match source.game() {
            Some(GameId::HaloReach) => Self::Reach,
            _ => Self::Halo3,
        }
    }
}

/// The tag behind an `#include`. Halo 3 keeps every source in one folder:
/// `global.fx` is `shaders\global_fx`. Reach's `c_tag_d3dx_include`
/// (tool `sub_141219410`) keeps the folder and cuts the last extension:
/// `templated\albedo.fx` is `shaders\templated\albedo`.
fn include_tag(dialect: H3Dialect, name: &str) -> String {
    match dialect {
        H3Dialect::Halo3 => {
            let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
            format!("shaders\\{}", file.replace('.', "_"))
        }
        H3Dialect::Reach => {
            let path = name.replace('/', "\\");
            let stem = path.rsplit_once('.').map_or(path.as_str(), |(stem, _)| stem);
            format!("shaders\\{stem}")
        }
    }
}

/// A kit's sources, read once.
pub(crate) struct H3Sources {
    source: TagSource,
    dialect: H3Dialect,
    definitions: HashMap<String, Option<Arc<H3Definition>>>,
    files: HashMap<String, Option<Arc<str>>>,
    passes: HashMap<(String, [Vec<Option<usize>>; 2], String, bool), Arc<H3Pass>>,
}

impl H3Sources {
    pub(crate) fn new(source: &TagSource) -> Self {
        let dialect = H3Dialect::of(source);
        let source = source.clone();
        Self { source, dialect, definitions: HashMap::new(), files: HashMap::new(), passes: HashMap::new() }
    }

    pub(crate) fn definition(&mut self, path: &str) -> Option<Arc<H3Definition>> {
        let source = &self.source;
        self.definitions
            .entry(path.to_owned())
            .or_insert_with(|| read_h3_definition(source, path).map(Arc::new))
            .clone()
    }

    fn file(&mut self, name: &str) -> Option<Arc<str>> {
        let (source, dialect) = (&self.source, self.dialect);
        self.files
            .entry(name.to_owned())
            .or_insert_with(|| {
                let tag = load_referenced_tag_from_source(source, &include_tag(dialect, name), "hlsl_include", b"hlsl").ok()?;
                let data = tag.root().field("include file")?.as_data()?.to_vec();
                Some(Arc::from(String::from_utf8_lossy(&data).trim_end_matches('\0')))
            })
            .clone()
    }

    /// `sub_140C587A0`'s defines, then the family's file for one stage.
    fn stage(
        &mut self,
        definition: &H3Definition,
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
        if own_albedo && self.dialect == H3Dialect::Halo3 {
            defines.push(("maybe_calc_albedo".to_owned(), "1".to_owned()));
        }
        defines.extend(option_defines(definition, &definition.stage_options(options, entry, stage)));
        let main = format!("#include \"{}\"\n", definition.source_file);
        let (vertex, pixel) = entry_functions(self.dialect, entry);
        self.compile(&defines, &main, if stage == Stage::Vertex { &vertex } else { &pixel }, stage)
    }

    fn compile(
        &mut self,
        defines: &[(String, String)],
        main: &str,
        entry: &str,
        stage: Stage,
    ) -> Result<Translated, String> {
        let borrowed: Vec<_> = defines.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        let mut include = |name: &str| self.file(name);
        let tokens = Preprocessor::new(&borrowed, &mut include).run(main)?;
        translate(&parse(&tokens)?, entry, stage)
    }

    /// One of the engine's own `.hlsl` passes, on a skinned vertex.
    pub(crate) fn explicit_pass(&mut self, file: &str) -> Result<Arc<H3Pass>, String> {
        let main = format!("#include \"{file}.hlsl\"\n");
        let vertex_defines = base_defines(self.dialect, VERTEX_SKINNED, Stage::Vertex);
        let pixel_defines = base_defines(self.dialect, VERTEX_SKINNED, Stage::Pixel);
        let vertex = self.compile(&vertex_defines, &main, "default_vs", Stage::Vertex)?;
        let pixel = self.compile(&pixel_defines, &main, "default_ps", Stage::Pixel)?;
        Ok(Arc::new(pass_of(vertex, pixel)))
    }

    /// A `shader_screen`'s pixel shader, fed by a triangle over the picture
    /// rather than the world vertices its family's `default_vs` reads.
    pub(crate) fn screen_pass(&mut self, definition: &H3Definition, options: &[usize]) -> Result<Arc<H3Pass>, String> {
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
        definition: &H3Definition,
        options: &[usize],
        entry: &str,
        vertex_type: usize,
    ) -> Result<H3Pass, String> {
        let (vertex, pixel) = self.stages(definition, options, entry, vertex_type, false)?;
        Ok(pass_of(vertex, pixel))
    }

    fn stages(
        &mut self,
        definition: &H3Definition,
        options: &[usize],
        entry: &str,
        vertex_type: usize,
        own_albedo: bool,
    ) -> Result<(Translated, Translated), String> {
        let vertex = self.stage(definition, options, entry, vertex_type, Stage::Vertex, own_albedo)?;
        let pixel_type = definition.pixel_vertex_type;
        let pixel = self.stage(definition, options, entry, pixel_type, Stage::Pixel, own_albedo)?;
        Ok((vertex, pixel))
    }
}

/// `default_vs` of `screen.fx`, for a triangle that covers the picture.
const SCREEN_VERTEX: &str = "#version 410 core
out vec4 v_TEXCOORD0;
void main() {
    vec2 corner = vec2(gl_VertexID & 1, gl_VertexID >> 1) * 4.0 - 1.0;
    gl_Position = vec4(corner, 0.0, 1.0);
    vec2 uv = vec2(corner.x * 0.5 + 0.5, 0.5 - corner.y * 0.5);
    v_TEXCOORD0 = vec4(uv, uv);
}
";

/// What a pass names, whichever stage reads it.
#[derive(Debug)]
pub(crate) struct H3Pass {
    /// Over the whole picture, with no geometry: a `shader_screen`.
    pub(crate) fullscreen: bool,
    pub(crate) vertex: String,
    pub(crate) pixel: String,
    pub(crate) uniforms: Vec<(String, Type)>,
    pub(crate) textures: Vec<(String, TextureKind)>,
}

/// The game's `Nodes[70]` holds one mesh's node map, uploaded a mesh at a time.
/// The preview's vertices name the skeleton's own nodes, all at once, with the
/// last kept spare.
const H3_NODES: usize = MAX_PREVIEW_BONES + 1;

/// A skinned vertex without weights rides the spare node, which holds still.
/// Direct3D lets a stage write less of a semantic than the next reads, as
/// `cortana.fx`'s `static_sh_vs` does with `COLOR1`, the rest reading 0; GLSL
/// will not link that. The vertex stage writes them all instead.
fn widen_varyings(vertex: &str, pixel: &str) -> String {
    let mut source = vertex.to_owned();
    for line in vertex.lines() {
        let Some((width, name)) = line.strip_prefix("out vec").and_then(|rest| rest.split_once(' ')) else {
            continue;
        };
        let name = name.trim_end_matches(';');
        let Ok(width) = width.parse::<usize>() else {
            continue;
        };
        let wanted = (2..=4).find(|wider| *wider > width && pixel.lines().any(|line| line == format!("in vec{wider} {name};")));
        let Some(wider) = wanted else {
            continue;
        };
        let zeros = ", 0.0".repeat(wider - width);
        source = source.replace(line, &format!("out vec{wider} {name};"));
        let prefix = format!("{name} = ");
        let widened: Vec<String> = source
            .lines()
            .map(|line| match line.trim_start().strip_prefix(&prefix).and_then(|value| value.strip_suffix(';')) {
                Some(value) => format!("    {name} = vec{wider}({value}{zeros});"),
                None => line.to_owned(),
            })
            .collect();
        source = widened.join("\n");
    }
    source
}

fn pass_of(vertex: Translated, pixel: Translated) -> H3Pass {
    let mut uniforms = vertex.uniforms;
    let nodes = uniforms.iter_mut().find_map(|(name, ty)| match ty {
        Type::Array(_, Some(nodes)) if name == "Nodes" => Some(nodes),
        _ => None,
    });
    let spare = nodes.map(|nodes| {
        *nodes = H3_NODES;
        H3_NODES - 1
    });
    for uniform in pixel.uniforms {
        if !uniforms.iter().any(|known| known.0 == uniform.0) {
            uniforms.push(uniform);
        }
    }
    let mut textures = vertex.textures;
    for texture in pixel.textures {
        if !textures.iter().any(|known| known.0 == texture.0) {
            textures.push(texture);
        }
    }
    // Flat in the source, three rows to a node.
    // The albedo pass lays depth that later passes test equal against, from
    // other shaders' arithmetic: Direct3D's `precise` positions, in GLSL terms.
    let invariant = vertex.source.replacen('\n', "\ninvariant gl_Position;\n", 1);
    let source = widen_varyings(&invariant, &pixel.source).replace("Nodes[210]", &format!("Nodes[{}]", H3_NODES * 3));
    let source = source.replace(
        "hlsl_p0.node_weights = a_BLENDWEIGHT;",
        "hlsl_p0.node_weights = a_BLENDWEIGHT;
    if (dot(a_BLENDWEIGHT, vec4(1.0)) <= 0.0) {
        hlsl_p0.node_weights = vec4(1.0, 0.0, 0.0, 0.0);
        hlsl_p0.node_indices = uvec4(H3_STILL_NODE);
    }",
    );
    let source = source.replace("H3_STILL_NODE", &format!("{}u", spare.unwrap_or(0)));
    // A vertex's occlusion ratio divides by the light's response to its
    // normal. Level probes keep that well above its floor, but the default
    // probe is lit nearly from one side, so on knobbly geometry a ratio of 100
    // interpolates across pixels that are properly lit. Occlusion only dims.
    let source = source.replace(
        "v_TEXCOORD7 = hlsl_result.prt_ravi_diff;",
        "v_TEXCOORD7 = hlsl_result.prt_ravi_diff;\n    v_TEXCOORD7.x = min(v_TEXCOORD7.x, 1.0);",
    );
    // The game's camo reads the frame ahead of its tone curve; the preview's
    // copy of what lies behind has been through it. This is that curve undone,
    // the root of `1.5x - 0.5x^3`.
    let pixel_source = pixel.source.replace(
        "vec3 l_true_scene_color = l_ldr_color.xyz;",
        "vec3 l_true_scene_color = 2.0 * sin(asin(clamp(l_ldr_color.xyz, 0.0, 1.0)) / 3.0);",
    );
    H3Pass { fullscreen: false, vertex: source, pixel: pixel_source, uniforms, textures }
}

impl H3Sources {
    /// An entry point's pass, kept for the next material of its template.
    pub(crate) fn pass(
        &mut self,
        definition: &H3Definition,
        options: &[usize],
        entry: &str,
        own_albedo: bool,
    ) -> Result<Arc<H3Pass>, String> {
        // Reach materials that differ only where a shared stage looks away
        // compile the same pass.
        let options_of = |stage| definition.stage_options(options, entry, stage);
        let key = (definition.name.clone(), [options_of(Stage::Vertex), options_of(Stage::Pixel)], entry.to_owned(), own_albedo);
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

/// `global_render_method`'s `blend_mode`, in its option order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H3Blend {
    Opaque,
    Additive,
    Multiply,
    AlphaBlend,
    DoubleMultiply,
    PreMultiplied,
    Maximum,
    MultiplyAdd,
    AddSrcTimesDstAlpha,
    AddSrcTimesSrcAlpha,
    InvAlphaBlend,
    MotionBlurStatic,
    MotionBlurInhibit,
    AddSrcTimesSrcAlphaAccumAlpha,
}

impl H3Blend {
    /// In `BLEND_MODES`' order.
    const ALL: [Self; BLEND_MODES.len()] = [
        Self::Opaque,
        Self::Additive,
        Self::Multiply,
        Self::AlphaBlend,
        Self::DoubleMultiply,
        Self::PreMultiplied,
        Self::Maximum,
        Self::MultiplyAdd,
        Self::AddSrcTimesDstAlpha,
        Self::AddSrcTimesSrcAlpha,
        Self::InvAlphaBlend,
        Self::MotionBlurStatic,
        Self::MotionBlurInhibit,
        Self::AddSrcTimesSrcAlphaAccumAlpha,
    ];
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum H3Value {
    Floats([f32; 4]),
    Integer(i32),
}

#[derive(Debug)]
pub(crate) struct H3Texture {
    pub(crate) uniform: String,
    pub(crate) image: Arc<H2Image>,
    pub(crate) texels: H3Texels,
    /// The levels past `image`, as the bitmap stores them.
    pub(crate) mips: Arc<Vec<H2Image>>,
    /// What the tag asks of the sampler; none for the engine's own textures.
    pub(crate) sampler: Option<H3Sampler>,
    /// The bitmap's other images, in order with `image` standing at its index,
    /// for a `FrameIndex` animation to choose among. Empty without one.
    pub(crate) frames: Vec<Arc<H2Image>>,
}

/// A texture's address and filter modes, from the bake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct H3Sampler {
    pub(crate) address: [BitmapAddressMode; 2],
    pub(crate) filter: BitmapFilterMode,
}

/// What a texel means to the card, from the bitmap's format and curve as
/// `bitmap_format_to_hardware_format_pc_unchecked` chooses its DXGI format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H3Texels {
    Linear,
    /// The card decodes the `sRGB` curve as it samples.
    Srgb,
    /// `_SNORM`: the shaders read -1..1 directly, and DirectX 11's
    /// `BUMP_CONVERT` does nothing to them.
    Signed,
}

impl H3Texels {
    pub(crate) fn of(format: BitmapFormat, srgb_curve: bool) -> Self {
        match format {
            BitmapFormat::DxnSnorm
            | BitmapFormat::V8u8
            | BitmapFormat::Q8w8v8u8
            | BitmapFormat::V16u16 => Self::Signed,
            _ if srgb_curve => Self::Srgb,
            _ => Self::Linear,
        }
    }

    /// `bitmap_format_is_default_srgb`, for a bitmap whose curve is unknown.
    pub(crate) fn default_srgb(format: BitmapFormat) -> bool {
        !matches!(
            format,
            BitmapFormat::SoftwareRgbfp32
                | BitmapFormat::V8u8
                | BitmapFormat::Abgrfp32
                | BitmapFormat::Abgrfp16
                | BitmapFormat::Q8w8v8u8
                | BitmapFormat::A2r10g10b10
                | BitmapFormat::A16b16g16r16
                | BitmapFormat::V16u16
                | BitmapFormat::Dxn
                | BitmapFormat::DxnSnorm
                | BitmapFormat::Ctx1
        )
    }
}

/// A shader's passes and everything they read.
#[derive(Debug)]
pub(crate) struct H3Material {
    pub(crate) albedo: Option<Arc<H3Pass>>,
    pub(crate) lit: Arc<H3Pass>,
    /// ODST's maps bound to the picture itself, by extern mode.
    pub(crate) screen_maps: Vec<String>,
    /// A decal family: `lit` is its `default`, drawn into the albedo target
    /// unless its `render_pass` puts it after the lights.
    pub(crate) decal: bool,
    pub(crate) after_lighting: bool,
    /// `sort layer`, for what blends.
    pub(crate) sort_layer: i32,
    /// `static_prt_ambient`, `_linear` and `_quadratic`.
    pub(crate) prt: [Option<Arc<H3Pass>>; 3],
    pub(crate) camo: Option<Arc<H3Pass>>,
    pub(crate) dynamic_light: Option<Arc<H3Pass>>,
    pub(crate) blend: H3Blend,
    pub(crate) values: HashMap<String, H3Value>,
    pub(crate) textures: Vec<H3Texture>,
    /// Uniforms holding the object's change colors, and which channel.
    pub(crate) change_colors: Vec<(String, usize)>,
    pub(crate) animations: Vec<H3Animation>,
    pub(crate) shield: Option<Arc<H3Shield>>,
}

/// `shield_impact.hlsl` and the settings it runs on.
#[derive(Debug)]
pub(crate) struct H3Shield {
    pub(crate) pass: Arc<H3Pass>,
    /// ODST's `shield_impact_procedural_uv`, for settings that ask for it.
    pub(crate) procedural: Option<Arc<H3Pass>>,
    pub(crate) settings: Arc<H3ShieldSettings>,
}

/// A `shield_impact` tag, as constants and its two noise maps by uniform suffix.
#[derive(Debug)]
pub(crate) struct H3ShieldSettings {
    pub(crate) procedural: bool,
    pub(crate) values: HashMap<String, H3Value>,
    pub(crate) textures: Vec<H3Texture>,
}

/// Which part of a constant an animation writes, as `update_constants` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H3Animated {
    First,
    Second,
    ScaleUniform,
    Third,
    Fourth,
    Color,
    /// Which of a bitmap's images binds.
    Frame,
}

/// One of a shader's `animated parameters`.
#[derive(Debug)]
pub(crate) struct H3Animation {
    pub(crate) uniform: String,
    pub(crate) writes: H3Animated,
    pub(crate) function: blam_tags::TagFunction,
    pub(crate) input: String,
    pub(crate) range: String,
    pub(crate) period: f32,
    /// Which input names run on the clock, by game and shader.
    pub(crate) clock: H3Clock,
}

/// How `evaluate_function` reads an animated parameter's input when it names
/// the clock rather than an object function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum H3Clock {
    /// Halo 3 (`evaluate_function`, March 2007 build): `unique_id` is time
    /// shifted by the object's index, so like objects run out of step.
    #[default]
    Halo3,
    /// ODST (`sub_140946C30`): `unique_id` shifted by the object's index
    /// times 1.37317.
    Odst,
    /// An ODST screen shader (`sub_14096D0E0`): `time` and `render_time` are
    /// the clock as well.
    OdstScreen,
}

impl H3Clock {
    /// Seconds `unique_id` runs ahead for the object of `owner`, or none if
    /// `input` is not a clock here. The game uses the object's datum index;
    /// the preview's owner (0 the opened object) stands in for it.
    fn phase(self, input: &str, owner: u8) -> Option<f32> {
        let owner = f32::from(owner);
        match (self, input) {
            (Self::Halo3, "unique_id") => Some(owner),
            (Self::Odst | Self::OdstScreen, "unique_id") => Some(owner * 1.37317),
            (Self::OdstScreen, "time" | "render_time") => Some(0.0),
            _ => None,
        }
    }
}

impl H3Animation {
    /// `evaluate_function`: an object's named values, or else time around a
    /// period, which only a periodic function runs past 1.
    fn sample(&self, time: f32, owner: u8, values: Option<&H2FunctionValues>) -> (f32, f32) {
        // `object_get_function_value`: no name at all reads full, so a parameter
        // with an input and no range runs on its function's upper curve.
        let named = |name: &str| {
            if name.is_empty() {
                return 1.0;
            }
            values.and_then(|values| values.get(name)).copied().unwrap_or(0.0)
        };
        let phase = self.clock.phase(&self.input, owner);
        if phase.is_none() && (!self.input.is_empty() || !self.range.is_empty()) {
            return (named(&self.input), named(&self.range));
        }
        let input = (time + phase.unwrap_or(0.0)) / self.period;
        if self.function.function_type() == FunctionType::Periodic {
            (input, 0.0)
        } else {
            (input.rem_euclid(1.0), 0.0)
        }
    }

    /// Write this into `value`, the constant as it stands.
    pub(crate) fn apply(&self, value: &mut [f32; 4], time: f32, owner: u8, values: Option<&H2FunctionValues>) {
        let (input, range) = self.sample(time, owner, values);
        if self.writes == H3Animated::Color {
            let color = self.function.evaluate_color(input, range);
            value[..3].copy_from_slice(&[color.red, color.green, color.blue]);
            return;
        }
        let output = self.function.evaluate(input, range);
        match self.writes {
            H3Animated::ScaleUniform => value[..2].fill(output),
            H3Animated::First => value[0] = output,
            H3Animated::Second => value[1] = output,
            H3Animated::Third => value[2] = output,
            _ => value[3] = output,
        }
    }

    /// `render_method_submit_volatile_per_node`'s choice among `count` images.
    pub(crate) fn frame(&self, count: usize, time: f32, owner: u8, values: Option<&H2FunctionValues>) -> usize {
        let (input, range) = self.sample(time, owner, values);
        let frame = (self.function.evaluate(input, range) % count as f32).floor() as isize;
        frame.clamp(0, count as isize - 1) as usize
    }

    /// Whether this moves with no one touching the object.
    pub(crate) fn moves(&self) -> bool {
        self.input.is_empty() && self.range.is_empty() && self.function.as_constant().is_none()
    }
}

#[cfg(test)]
mod texel_tests {
    use super::*;

    fn animation(input: &str, range: &str) -> H3Animation {
        H3Animation {
            uniform: String::new(),
            writes: H3Animated::First,
            function: blam_tags::TagFunction::parse(&[0; 32]).unwrap(),
            input: input.to_owned(),
            range: range.to_owned(),
            period: 1.0,
            clock: H3Clock::Halo3,
        }
    }

    #[test]
    fn an_empty_name_beside_a_named_one_reads_full() {
        let values = H2FunctionValues::from([("power".to_owned(), 0.25), ("heat".to_owned(), 0.5)]);
        let values = Some(&values);
        assert_eq!(animation("power", "").sample(0.0, 0, values), (0.25, 1.0));
        assert_eq!(animation("", "heat").sample(0.0, 0, values), (1.0, 0.5));
        assert_eq!(animation("power", "heat").sample(0.0, 0, values), (0.25, 0.5));
        assert_eq!(animation("missing", "").sample(0.0, 0, values), (0.0, 1.0));
    }

    #[test]
    fn render_time_is_the_clock_only_in_odst_screen_shaders() {
        let values = H2FunctionValues::new();
        let mut timed = animation("render_time", "");
        assert_eq!(timed.sample(0.25, 0, Some(&values)), (0.0, 1.0), "an object function, and none is set");
        timed.clock = H3Clock::Odst;
        assert_eq!(timed.sample(0.25, 0, Some(&values)), (0.0, 1.0));
        timed.clock = H3Clock::OdstScreen;
        assert_eq!(timed.sample(0.25, 0, Some(&values)), (0.25, 0.0));
    }

    #[test]
    fn unique_id_runs_each_object_out_of_step() {
        let mut unique = animation("unique_id", "");
        unique.period = 4.0;
        assert_eq!(unique.sample(1.0, 0, None), (0.25, 0.0));
        assert_eq!(unique.sample(1.0, 2, None), (0.75, 0.0), "Halo 3: time plus the object's index");
        unique.clock = H3Clock::Odst;
        assert_eq!(unique.sample(1.0, 2, None).0, (1.0 + 2.0 * 1.37317) / 4.0);
    }

    #[test]
    fn nothing_named_runs_on_time() {
        assert_eq!(animation("", "").sample(0.25, 0, None), (0.25, 0.0));
    }

    #[test]
    fn the_dxgi_tables_signed_formats_read_signed_whatever_the_curve() {
        for format in [BitmapFormat::DxnSnorm, BitmapFormat::V8u8, BitmapFormat::Q8w8v8u8, BitmapFormat::V16u16] {
            assert_eq!(H3Texels::of(format, false), H3Texels::Signed);
            assert_eq!(H3Texels::of(format, true), H3Texels::Signed);
        }
    }

    #[test]
    fn unsigned_formats_follow_the_curve() {
        assert_eq!(H3Texels::of(BitmapFormat::A8r8g8b8, true), H3Texels::Srgb);
        assert_eq!(H3Texels::of(BitmapFormat::A8r8g8b8, false), H3Texels::Linear);
        assert_eq!(H3Texels::of(BitmapFormat::Dxn, false), H3Texels::Linear);
    }

    #[test]
    fn an_unknown_curve_falls_back_on_the_format() {
        assert!(H3Texels::default_srgb(BitmapFormat::Dxt1));
        assert!(H3Texels::default_srgb(BitmapFormat::A8r8g8b8));
        assert!(!H3Texels::default_srgb(BitmapFormat::DxnSnorm));
        assert!(!H3Texels::default_srgb(BitmapFormat::Abgrfp16));
    }
}

#[cfg(test)]
mod varying_tests {
    use super::widen_varyings;

    #[test]
    fn a_vertex_stage_writes_as_much_as_the_pixel_stage_reads() {
        let vertex = "out vec3 v_COLOR1;\nout vec2 v_TEXCOORD0;\nvoid main() {\n    v_COLOR1 = a.xyz;\n    v_TEXCOORD0 = b;\n}";
        let pixel = "in vec4 v_COLOR1;\nin vec2 v_TEXCOORD0;";
        let widened = widen_varyings(vertex, pixel);
        assert!(widened.contains("out vec4 v_COLOR1;"), "{widened}");
        assert!(widened.contains("v_COLOR1 = vec4(a.xyz, 0.0);"), "{widened}");
        assert!(widened.contains("out vec2 v_TEXCOORD0;"));
        assert!(widened.contains("v_TEXCOORD0 = b;"));
        assert_eq!(widen_varyings(vertex, "in vec3 v_COLOR1;\nin vec2 v_TEXCOORD0;"), vertex);
    }
}

#[cfg(test)]
mod stage_option_tests {
    use super::*;

    fn category(name: &str, options: &[&str]) -> H3Category {
        let option = |name: &&str| H3Option {
            name: (*name).to_owned(),
            vertex_function: None,
            pixel_function: Some(format!("calc_{name}")),
        };
        H3Category {
            name: name.to_owned(),
            vertex_function: None,
            pixel_function: Some(format!("calc_{name}_ps")),
            options: options.iter().map(option).collect(),
        }
    }

    fn definition(automatic_macros: bool, shared: &[(&str, H3SharedStages)]) -> H3Definition {
        H3Definition {
            name: "shader".to_owned(),
            dialect: H3Dialect::Reach,
            source_file: "templated\\shader.fx".to_owned(),
            automatic_macros,
            categories: vec![
                category("albedo", &["default", "detail", "constant"]),
                category("blend_mode", &["opaque", "additive", "multiply", "alpha_blend"]),
            ],
            entry_points: shared.iter().map(|(entry, _)| (*entry).to_owned()).collect(),
            shared: shared.iter().map(|(entry, rules)| ((*entry).to_owned(), *rules)).collect(),
            pixel_vertex_type: 0,
        }
    }

    #[test]
    fn reach_reads_blend_names_minding_case_and_blends_glass_without_a_category() {
        let mut reach = definition(false, &[]);
        reach.categories[1].options[1].name = "ADDITIVE".to_owned();
        assert_eq!(reach.blend(&[0, 1]), H3Blend::Opaque, "Reach minds case");
        assert_eq!(reach.blend(&[0, 3]), H3Blend::AlphaBlend);
        reach.dialect = H3Dialect::Halo3;
        assert_eq!(reach.blend(&[0, 1]), H3Blend::Additive, "Halo 3 does not");

        let mut glass = definition(false, &[]);
        glass.categories.truncate(1);
        assert_eq!(glass.blend_of(&[0], true), H3Blend::AlphaBlend);
        assert_eq!(glass.blend_of(&[0], false), H3Blend::Opaque);
        glass.dialect = H3Dialect::Halo3;
        assert_eq!(glass.blend_of(&[0], true), H3Blend::Opaque);
    }

    #[test]
    fn halo_3_compiles_both_stages_with_the_materials_options() {
        let halo3 = definition(false, &[]);
        assert_eq!(halo3.stage_options(&[2, 3], "static_sh", Stage::Vertex), [Some(2), Some(3)]);
        assert_eq!(halo3.stage_options(&[2, 3], "static_sh", Stage::Pixel), [Some(2), Some(3)]);
    }

    #[test]
    fn a_reach_vertex_shader_keeps_only_its_category_and_only_two_of_its_options() {
        let rules = H3SharedStages { vertex_two_options: true, vertex_category: Some(1), ..Default::default() };
        let reach = definition(false, &[("static_sh", rules)]);
        assert_eq!(reach.stage_options(&[2, 3], "static_sh", Stage::Vertex), [Some(0), Some(1)], "alpha_blend shares additive's");
        assert_eq!(reach.stage_options(&[2, 0], "static_sh", Stage::Vertex), [Some(0), Some(0)]);
        assert_eq!(reach.stage_options(&[2, 3], "static_sh", Stage::Pixel), [Some(2), Some(3)], "the material's own pixel shader");
        let uncapped = definition(false, &[("static_sh", H3SharedStages { vertex_two_options: false, ..rules })]);
        assert_eq!(uncapped.stage_options(&[2, 3], "static_sh", Stage::Vertex), [Some(0), Some(3)]);
    }

    #[test]
    fn a_shared_pixel_shader_reads_only_its_category_and_automatic_macros_default_to_none() {
        let rules = H3SharedStages { shared_pixel: true, pixel_category: Some(0), ..Default::default() };
        let reach = definition(true, &[("shadow_generate", rules)]);
        assert_eq!(reach.stage_options(&[2, 3], "shadow_generate", Stage::Pixel), [Some(2), None]);
        assert_eq!(reach.stage_options(&[2, 3], "shadow_generate", Stage::Vertex), [None, None]);
    }

    #[test]
    fn no_option_names_every_option_but_chooses_none() {
        let reach = definition(true, &[]);
        let defines = option_defines(&reach, &[None, Some(3)]);
        let has = |name: &str| defines.iter().any(|(known, _)| known == name);
        assert!(!has("calc_albedo_ps"), "no function without an option: {defines:?}");
        assert!(!has("category_albedo"), "{defines:?}");
        assert!(has("category_albedo_option_detail"), "{defines:?}");
        assert!(defines.contains(&("calc_blend_mode_ps".to_owned(), "calc_alpha_blend".to_owned())));
        assert!(defines.contains(&("category_blend_mode".to_owned(), "category_blend_mode_option_alpha_blend".to_owned())));
    }
}
