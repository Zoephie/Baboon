//! A decorator set as its own material: its `texture`, which the game
//! samples for colour and cuts out by alpha (`clip(alpha - 0.3)` in Halo 3's
//! `decorators.hlsl` on DX11, 0.5 in Reach's and Halo 4's).
//!
//! With Game Shaders, a set draws through the explicit shader its `render
//! shader` names, translated from its source, which the pipeline draws
//! instanced from the set's placements: Halo 3's and Reach's from their
//! `hlsl_include` tags, Halo 4's from the source its `material_shader`
//! carries.

use super::super::common::fields::elements;
use super::super::functions::FunctionValues;
use super::super::gen3::materials::gen3_image;
use super::super::gen3::shaders::{
    Gen3Blend, Gen3Material, Gen3MaterialDialect, Gen3MaterialExtension, Gen3Pass, Gen3Sources,
    Gen3Texture, pass_of,
};
use super::super::h4::materials::shaders::{H4_DEFINES, H4ShaderSource, translate_pass_feeding};
use super::super::load::TagReader;
use super::super::materials::{SlotBitmap, cached_bitmap};
use super::super::{MaterialTextures, ResolveCaches, TextureSlot};
use super::{DecoratorShader, WindDefinition};
use crate::core::game::GameId;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::hlsl::emit::Stage;
use crate::core::hlsl::parse::{Scalar, Type};
use blam_tags::TagFile;
use std::sync::Arc;

/// `rasterizer_globals`' `default bitmaps` element `setup_wind_constants`
/// binds when a wind names no gust noise (`get_default_texture_ref(10)`):
/// `random4_warp`.
const DEFAULT_WIND_BITMAP: usize = 10;

/// `set`'s texture, as the base map and the map that cuts it out, and with
/// Game Shaders its translated pass.
pub(in crate::app::model_preview) fn resolve_decorator_material(
    reader: &TagReader,
    set: &TagFile,
    caches: &mut ResolveCaches,
) -> MaterialTextures {
    let path = set
        .root()
        .read_tag_ref_path("texture")
        .map(|path| path.trim().to_owned())
        .unwrap_or_default();
    if path.is_empty() {
        return MaterialTextures::failed("this decorator set names no texture");
    }
    let binding = SlotBitmap {
        path: path.clone(),
        image_index: 0,
        repeat_x: true,
        repeat_y: true,
        scale: [1.0, 1.0],
    };
    let Some(image) = cached_bitmap(reader, &binding, caches) else {
        return MaterialTextures::failed(format!("could not read its texture, {path}"));
    };
    let mut textures = MaterialTextures::default();
    textures.slots[TextureSlot::Base as usize] = Some(image.clone());
    textures.slots[TextureSlot::AlphaTest as usize] = Some(image);
    if reader.game_shaders() {
        match game_material(reader, set, &path, caches) {
            Ok(material) => textures.gen3 = Some(Arc::new(material)),
            Err(error) => textures.game_shaders_error = Some(error),
        }
    }
    textures
}

/// The set's explicit shader, compiled from the kit's source, with its
/// texture and the wind's gust noise.
fn game_material(
    reader: &TagReader,
    set: &TagFile,
    texture: &str,
    caches: &mut ResolveCaches,
) -> Result<Gen3Material, String> {
    let name = set.root().read_enum_option("render shader");
    let shader = match &name {
        Some(name) => DecoratorShader::from_option_name(name),
        None => Some(DecoratorShader::first_option(reader.source.game())),
    };
    let shader = shader.ok_or_else(|| {
        format!(
            "no decorator shader is named \"{}\"",
            name.unwrap_or_default()
        )
    })?;
    let (lit, dialect) = if reader.source.game() == Some(GameId::Halo4) {
        let lit = halo_4_pass(reader, shader.source_file())?;
        (
            lit,
            Gen3MaterialDialect::Halo4(Arc::new(DecoratorExtension)),
        )
    } else {
        let sources = caches
            .gen3_sources
            .get_or_insert_with(|| Gen3Sources::new(&reader.source));
        (
            sources.decorator_pass(shader.source_file())?,
            sources.dialect().into(),
        )
    };
    // Each game names its samplers by its own scope: Halo 3's are local,
    // Reach's wind texture global, and Halo 4's `diffuse_textureTexture` and
    // `wind_texture_texture`.
    let sampler = |wanted: &str| {
        lit.textures
            .iter()
            .map(|(name, _)| name)
            .find(|name| name.contains(wanted))
            .cloned()
    };
    let diffuse = sampler("diffuse_texture").ok_or("its shader samples no diffuse texture")?;
    let mut textures = vec![texture_of(reader, &diffuse, texture, caches)?];
    if let Some(wind) = sampler("wind_texture") {
        let gusts = WindDefinition::default_for(reader)
            .map(|wind| wind.gust_bitmap)
            .filter(|path| !path.is_empty())
            .or_else(|| default_bitmap(reader, DEFAULT_WIND_BITMAP))
            .ok_or("no gust noise for the wind")?;
        textures.push(texture_of(reader, &wind, &gusts, caches)?);
    }
    Ok(Gen3Material {
        dialect,
        albedo: None,
        lit,
        fringe: None,
        screen_maps: Vec::new(),
        decal: false,
        decal_modulates_specular: false,
        after_lighting: false,
        sort_layer: 0,
        prt: [None, None, None],
        camo: None,
        shadow_generate: None,
        dynamic_light: None,
        blend: Gen3Blend::Opaque,
        values: Default::default(),
        textures,
        change_colors: Vec::new(),
        animations: Vec::new(),
        shield: None,
        decorator: true,
    })
}

/// The bitmap at `path` bound to the sampler `uniform`.
fn texture_of(
    reader: &TagReader,
    uniform: &str,
    path: &str,
    caches: &mut ResolveCaches,
) -> Result<Gen3Texture, String> {
    let (image, texels, mips) = gen3_image(reader, path, 0, SamplerKind::Texture2D, caches)
        .ok_or_else(|| format!("could not read {path}"))?;
    Ok(Gen3Texture {
        uniform: uniform.to_owned(),
        image,
        texels,
        mips,
        sampler: None,
        frames: Vec::new(),
    })
}

/// `rasterizer_globals`' default bitmap `index`.
fn default_bitmap(reader: &TagReader, index: usize) -> Option<String> {
    let globals = reader
        .load("globals\\rasterizer_globals", "rasterizer_globals", b"rasg")
        .ok()?;
    let defaults = elements(globals.root(), "default bitmaps");
    defaults
        .get(index)?
        .read_tag_ref_path("default bitmaps")
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
}

/// Where Halo 4 keeps its decorator shaders, which its `rasterizer_globals`
/// name among its explicit shaders.
const HALO_4_DECORATORS: &str = "shaders\\material_shaders\\explicit_shaders\\decorators";

/// Halo 4's `file`, translated from the source its `material_shader`
/// carries: `_default`'s `decorator` pass, the packed `POSITION1` fed as an
/// integer, and its `RGBk` output decoded.
fn halo_4_pass(reader: &TagReader, file: &str) -> Result<Arc<Gen3Pass>, String> {
    let source = H4ShaderSource::read(reader, &format!("{HALO_4_DECORATORS}\\{file}"))?;
    let program = source.program(&H4_DEFINES)?;
    let vertex = translate_pass_feeding(
        &program,
        "_default",
        "decorator",
        Stage::Vertex,
        &["POSITION1"],
    )?;
    let pixel = translate_pass_feeding(&program, "_default", "decorator", Stage::Pixel, &[])?;
    let mut pass = pass_of(vertex, pixel);
    decode_rgbk(&mut pass)?;
    Ok(Arc::new(pass))
}

/// Halo 4's decorators write `RGBk` (`PackRGBk`), which the game reads back
/// before its exposure; every other Halo 4 pass writes its colour exposed,
/// as the preview's target holds it. The output is decoded as `UnpackRGBk`
/// (`core_functions.fxh`) does and exposed as `apply_exposure` does.
fn decode_rgbk(pass: &mut Gen3Pass) -> Result<(), String> {
    const OUTPUT: &str = "    o_target0 = hlsl_result;";
    if !pass.pixel.contains(OUTPUT) {
        return Err("the decorator's output is not where it was".to_owned());
    }
    pass.pixel = pass.pixel.replace(
        OUTPUT,
        "    o_target0 = vec4(hlsl_result.rgb * hlsl_result.a * 31.875 * ps_view_exposure.x, 1.0);",
    );
    pass.pixel = pass.pixel.replacen(
        "layout(location = 0) out vec4 o_target0;",
        "layout(location = 0) out vec4 o_target0;\nuniform vec4 ps_view_exposure;",
        1,
    );
    pass.uniforms.push((
        "ps_view_exposure".to_owned(),
        Type::Vector(Scalar::Float, 4),
    ));
    Ok(())
}

/// What a Halo 4 decorator carries as its material's extension: nothing of
/// a material's own, as its pass is the set's and its values the set's.
#[derive(Debug)]
struct DecoratorExtension;

impl Gen3MaterialExtension for DecoratorExtension {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn frame(&self, _: &str, _: Option<&FunctionValues>, _: f32, _: u8) -> Option<usize> {
        None
    }

    fn moves(&self) -> bool {
        false
    }

    fn inputs(&self) -> Vec<&str> {
        Vec::new()
    }

    fn after_inputs(&self) -> Vec<&str> {
        Vec::new()
    }
}
