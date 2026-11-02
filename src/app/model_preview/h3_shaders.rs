//! Halo 3's shaders, as tool compiles them.
//!
//! A `render_method_definition` lists categories, each naming a function that
//! the chosen option's function stands in for, and `shaders\<name>.fx` is
//! compiled per entry point with those as defines. The sources live in
//! `.hlsl_include` tags, read from the user's kit.

use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::bitmap::BitmapFormat;
use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode, BLEND_MODES};

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

/// A `render_method_definition`, as far as compiling goes.
#[derive(Debug, Clone)]
pub(crate) struct H3Definition {
    /// The `.fx` this family compiles: `shader`, `terrain`..
    name: String,
    /// `use automatic macros`: categories and options named as defines too.
    automatic_macros: bool,
    categories: Vec<H3Category>,
    pub(crate) entry_points: Vec<String>,
    /// The first `vertex_types` entry, which every pixel shader compiles with.
    pixel_vertex_type: usize,
}

pub(crate) fn read_h3_definition(source: &TagSource, path: &str) -> Option<H3Definition> {
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
    let entry_points = elements(root, "entry_points")
        .into_iter()
        .filter_map(|entry| entry.read_enum_name("entry point"))
        .map(|entry| entry_name(&entry).to_owned())
        .collect();
    let name = path.rsplit('\\').next()?.to_owned();
    let automatic_macros = root.read_int_any("flags").unwrap_or(0) & 1 != 0;
    let pixel_vertex_type = elements(root, "vertex_types")
        .first()
        .and_then(|first| first.read_int_any("vertex type"))
        .and_then(|kind| usize::try_from(kind).ok())
        .filter(|kind| *kind < VERTEX_TYPES.len())
        .unwrap_or(0);
    Some(H3Definition { name, automatic_macros, categories, entry_points, pixel_vertex_type })
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

    /// The `blend_mode` category's choice.
    pub(crate) fn blend(&self, options: &[usize]) -> H3Blend {
        let name = self.chosen("blend_mode", options).unwrap_or_default().to_ascii_lowercase();
        let index = BLEND_MODES.iter().position(|mode| name.starts_with(mode)).unwrap_or(0);
        H3Blend::ALL[index]
    }
}

/// tool's name for an entry point, where the tag's enum spells it for people.
fn entry_name(option: &str) -> &str {
    match option {
        "vertex color lighting" => "static_per_vertex_color",
        "water tessellation" => "water_tessellation",
        "water shading" => "water_shading",
        other => other,
    }
}

/// The function each stage compiles. Some entry points share one.
fn entry_functions(entry: &str) -> (String, String) {
    match entry {
        "static_prt_ambient" | "static_prt_linear" | "static_prt_quadratic" => {
            (format!("{entry}_vs"), "static_prt_ps".to_owned())
        }
        "dynamic_light_cinematic" => ("dynamic_light_cine_vs".to_owned(), "dynamic_light_cine_ps".to_owned()),
        other => (format!("{other}_vs"), format!("{other}_ps")),
    }
}

/// `sub_140C587A0`: what every shader compiles with.
fn base_defines(vertex_type: usize, stage: Stage) -> Vec<(String, String)> {
    let (vertex, deform) = VERTEX_TYPES[vertex_type];
    let stage = match stage {
        Stage::Vertex => "VERTEX_SHADER",
        Stage::Pixel => "PIXEL_SHADER",
    };
    [
        ("vertex_type", vertex),
        ("deform", deform),
        ("SHADER_30", "1"),
        ("pc", "1"),
        ("DX_VERSION", "11"),
        ("PARAM_ALLOC_FIRST_PASS", "1"),
        (stage, "1"),
    ]
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .to_vec()
}

/// `sub_140C589E0`: each category's function, as its option's.
fn option_defines(definition: &H3Definition, options: &[usize]) -> Vec<(String, String)> {
    let mut defines = Vec::new();
    for (index, category) in definition.categories.iter().enumerate() {
        let chosen = options.get(index).copied().unwrap_or(0);
        let chosen = chosen.min(category.options.len().saturating_sub(1));
        let Some(option) = category.options.get(chosen) else {
            continue;
        };
        for (name, value) in [
            (&category.vertex_function, &option.vertex_function),
            (&category.pixel_function, &option.pixel_function),
        ] {
            if let (Some(name), Some(value)) = (name, value) {
                defines.push((name.clone(), value.clone()));
            }
        }
        if definition.automatic_macros {
            let prefix = format!("category_{}", category.name);
            defines.push((prefix.clone(), format!("{prefix}_option_{}", option.name)));
            for (index, option) in category.options.iter().enumerate() {
                defines.push((format!("{prefix}_option_{}", option.name), index.to_string()));
            }
        }
    }
    defines
}

/// The tag behind an `#include`: `global.fx` is `shaders\global_fx`.
fn include_tag(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    format!("shaders\\{}", file.replace('.', "_"))
}

/// A kit's sources, read once.
pub(crate) struct H3Sources {
    source: TagSource,
    definitions: HashMap<String, Option<Arc<H3Definition>>>,
    files: HashMap<String, Option<Arc<str>>>,
    passes: HashMap<(String, Vec<usize>, String, bool), Arc<H3Pass>>,
}

impl H3Sources {
    pub(crate) fn new(source: &TagSource) -> Self {
        let source = source.clone();
        Self { source, definitions: HashMap::new(), files: HashMap::new(), passes: HashMap::new() }
    }

    pub(crate) fn definition(&mut self, path: &str) -> Option<Arc<H3Definition>> {
        let source = &self.source;
        self.definitions
            .entry(path.to_owned())
            .or_insert_with(|| read_h3_definition(source, path).map(Arc::new))
            .clone()
    }

    fn file(&mut self, name: &str) -> Option<Arc<str>> {
        let source = &self.source;
        self.files
            .entry(name.to_owned())
            .or_insert_with(|| {
                let tag = load_referenced_tag_from_source(source, &include_tag(name), "hlsl_include", b"hlsl").ok()?;
                let data = tag.root().field("include file")?.as_data()?.to_vec();
                Some(Arc::from(String::from_utf8_lossy(&data).trim_end_matches('\0')))
            })
            .clone()
    }

    /// `sub_140C587A0`'s defines, then `<family>.fx` for one stage.
    fn stage(
        &mut self,
        definition: &H3Definition,
        options: &[usize],
        entry: &str,
        vertex_type: usize,
        stage: Stage,
        own_albedo: bool,
    ) -> Result<Translated, String> {
        let mut defines = base_defines(vertex_type, stage);
        defines.push(("entry_point".to_owned(), entry.to_owned()));
        // `sub_140C587A0`: a blended shader has no albedo pass to read back.
        if own_albedo {
            defines.push(("maybe_calc_albedo".to_owned(), "1".to_owned()));
        }
        defines.extend(option_defines(definition, options));
        let main = format!("#include \"{}.fx\"\n", definition.name);
        let (vertex, pixel) = entry_functions(entry);
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
        let vertex_defines = base_defines(VERTEX_SKINNED, Stage::Vertex);
        let pixel_defines = base_defines(VERTEX_SKINNED, Stage::Pixel);
        let vertex = self.compile(&vertex_defines, &main, "default_vs", Stage::Vertex)?;
        let pixel = self.compile(&pixel_defines, &main, "default_ps", Stage::Pixel)?;
        Ok(Arc::new(pass_of(vertex, pixel)))
    }

    /// An entry point's two stages.
    #[cfg(test)]
    pub(crate) fn translate_entry(
        &mut self,
        definition: &H3Definition,
        options: &[usize],
        entry: &str,
        vertex_type: usize,
    ) -> Result<(Translated, Translated), String> {
        self.stages(definition, options, entry, vertex_type, false)
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

/// What a pass names, whichever stage reads it.
#[derive(Debug)]
pub(crate) struct H3Pass {
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
    let source = vertex.source.replace("Nodes[210]", &format!("Nodes[{}]", H3_NODES * 3));
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
    H3Pass { vertex: source, pixel: pixel_source, uniforms, textures }
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
        let key = (definition.name.clone(), options.to_vec(), entry.to_owned(), own_albedo);
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
}

impl H3Animation {
    /// `evaluate_function`: an object's named values, or else time around a
    /// period, which only a periodic function runs past 1.
    fn sample(&self, time: f32, values: Option<&H2FunctionValues>) -> (f32, f32) {
        // `object_get_function_value`: no name at all reads full, so a parameter
        // with an input and no range runs on its function's upper curve.
        let named = |name: &str| {
            if name.is_empty() {
                return 1.0;
            }
            values.and_then(|values| values.get(name)).copied().unwrap_or(0.0)
        };
        let timed = self.input == "unique_id";
        if !timed && (!self.input.is_empty() || !self.range.is_empty()) {
            return (named(&self.input), named(&self.range));
        }
        let input = time / self.period;
        if self.function.function_type() == FunctionType::Periodic {
            (input, 0.0)
        } else {
            (input.rem_euclid(1.0), 0.0)
        }
    }

    /// Write this into `value`, the constant as it stands.
    pub(crate) fn apply(&self, value: &mut [f32; 4], time: f32, values: Option<&H2FunctionValues>) {
        let (input, range) = self.sample(time, values);
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
    pub(crate) fn frame(&self, count: usize, time: f32, values: Option<&H2FunctionValues>) -> usize {
        let (input, range) = self.sample(time, values);
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
        }
    }

    #[test]
    fn an_empty_name_beside_a_named_one_reads_full() {
        let values = H2FunctionValues::from([("power".to_owned(), 0.25), ("heat".to_owned(), 0.5)]);
        let values = Some(&values);
        assert_eq!(animation("power", "").sample(0.0, values), (0.25, 1.0));
        assert_eq!(animation("", "heat").sample(0.0, values), (1.0, 0.5));
        assert_eq!(animation("power", "heat").sample(0.0, values), (0.25, 0.5));
        assert_eq!(animation("missing", "").sample(0.0, values), (0.0, 1.0));
    }

    #[test]
    fn nothing_named_runs_on_time() {
        assert_eq!(animation("", "").sample(0.25, None), (0.25, 0.0));
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
