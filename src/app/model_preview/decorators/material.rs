//! A decorator set as its own material: its `texture`, which the game
//! samples for colour and cuts out by alpha (`clip(alpha - 0.3)` in Halo 3's
//! `decorators.hlsl` on DX11, 0.5 in Reach's and Halo 4's).
//!
//! With Game Shaders, Halo 3's and ODST's sets draw through the explicit
//! shader their `render shader` names, translated from its source, which the
//! pipeline draws instanced from the set's placements.

use super::super::common::fields::elements;
use super::super::gen3::materials::gen3_image;
use super::super::gen3::shaders::{
    Gen3Blend, Gen3Material, Gen3MaterialDialect, Gen3Sources, Gen3Texture, RenderMethodDialect,
};
use super::super::load::TagReader;
use super::super::materials::{SlotBitmap, cached_bitmap};
use super::super::{MaterialTextures, ResolveCaches, TextureSlot};
use super::{DecoratorShader, WindDefinition};
use crate::core::h2_pixel_shader::SamplerKind;
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
    let sources = caches
        .gen3_sources
        .get_or_insert_with(|| Gen3Sources::new(&reader.source));
    if sources.dialect() != RenderMethodDialect::Halo3 {
        return Err("only Halo 3's and ODST's decorator shaders are translated yet".to_owned());
    }
    let name = set.root().read_enum_option("render shader");
    let shader = match &name {
        Some(name) => DecoratorShader::from_option_name(name),
        None => Some(DecoratorShader::first_option(reader.source.game())),
    };
    let file = shader
        .and_then(DecoratorShader::halo_3_source)
        .ok_or_else(|| {
            format!(
                "no Halo 3 decorator shader is named \"{}\"",
                name.unwrap_or_default()
            )
        })?;
    let lit = sources.explicit_pass(file)?;
    let mut textures = vec![texture_of(
        reader,
        "LocalTexture_diffuse_texture",
        texture,
        caches,
    )?];
    if lit
        .textures
        .iter()
        .any(|(name, _)| name == "LocalTexture_wind_texture")
    {
        let gusts = WindDefinition::default_for(reader)
            .map(|wind| wind.gust_bitmap)
            .filter(|path| !path.is_empty())
            .or_else(|| default_bitmap(reader, DEFAULT_WIND_BITMAP))
            .ok_or("no gust noise for the wind")?;
        textures.push(texture_of(
            reader,
            "LocalTexture_wind_texture",
            &gusts,
            caches,
        )?);
    }
    Ok(Gen3Material {
        dialect: Gen3MaterialDialect::Halo3,
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
