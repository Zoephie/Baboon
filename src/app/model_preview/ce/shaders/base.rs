//! What every Halo CE shader pass builds from: the kit's shader files, the default lights, flags, plain maps and simple passes.

use super::{
    Dx9Blend, Dx9BlendEquation, Dx9BlendFactor, Dx9Compare, Dx9Cull, Dx9Depth, Dx9DrawPass,
    Dx9DrawTexture, Dx9VertexStage, H2_TEXTURE_ROWS, Image, SamplerKind, TagReader, TagSource,
    has_flag_in_any,
};
use blam_tags::TagStruct;
use std::sync::Arc;

pub(super) const COMMON_FILE: &str = "model_common.h";

pub(super) const GENERIC_FILE: &str = "transparent_generic_shader.psh";

pub(super) const CHICAGO_FILE: &str = "chicago_shader.psh";

pub(super) const GLASS_TINT_FILE: &str = "transparent_glass_tint.psh";

pub(super) const GLASS_FLAT_FILE: &str = "transparent_glass_reflection_flat.psh";

pub(super) const GLASS_BUMPED_FILE: &str = "transparent_glass_reflection_bumped.psh";

pub(super) const GLASS_DIFFUSE_FILE: &str = "transparent_glass_diffuse.psh";

pub(super) const METER_FILE: &str = "transparent_meter.psh";

pub(super) const PLASMA_FILE: &str = "transparent_plasma.psh";

pub(super) const CAMO_FILE: &str = "active_camouflage_draw.psh";

pub(super) const WATER_OPACITY_FILE: &str = "transparent_water_opacity.psh";

pub(super) const WATER_REFLECTION_FILE: &str = "transparent_water_reflection.psh";

pub(super) const AMBIENT: [f32; 3] = [0.2, 0.2, 0.2];

pub(super) const FIRST_LIGHT_DIRECTION: [f32; 3] = [-0.577, -0.577, -0.577];

pub(super) const FIRST_LIGHT_COLOR: [f32; 3] = [1.0, 1.0, 1.0];

pub(super) const SECOND_LIGHT_DIRECTION: [f32; 3] = [0.0, 0.0, 1.0];

pub(super) const SECOND_LIGHT_COLOR: [f32; 3] = [0.4, 0.4, 0.5];

/// `reflection_tint_color`: alpha, then red, green and blue.
pub(super) const REFLECTION_TINT: [f32; 4] = [0.5, 1.0, 1.0, 1.0];

/// `AlphaTest_Greater7F`: between 0x7F and 0x80.
pub(super) const ALPHA_REFERENCE: f32 = 0.5;

/// What a map defaults to, as `rasterizer_set_texture_for_effect`'s usage has
/// it: the shape of `global_rasterizer_data`'s default bitmaps.
pub(super) fn flat(value: u8) -> Arc<Image> {
    Arc::new(Image::Flat(super::super::super::materials::TextureImage {
        rgba: vec![value, value, value, value],
        width: 1,
        height: 1,
        repeat_x: true,
        repeat_y: true,
        scale: [1.0, 1.0],
    }))
}

pub(super) const WHITE: u8 = 255;

pub(super) const GREY: u8 = 128;

pub(super) fn texture(slot: u32, kind: SamplerKind, image: Option<Arc<Image>>) -> Dx9DrawTexture {
    Dx9DrawTexture {
        slot,
        kind,
        image,
        normalization: false,
        texaccum: false,
        frame_copy: false,
        falloff: false,
        gel: false,
    }
}

/// A shader's option by name, with a list's position standing for its value.
pub(in crate::app::model_preview) fn option_index(
    root: TagStruct<'_>,
    field: &str,
    options: &[&str],
) -> i32 {
    let name = root.read_enum_option(field);
    options
        .iter()
        .position(|option| name.as_deref() == Some(*option))
        .unwrap_or(0) as i32
}

pub(super) fn read_shader_source(
    directory: &std::path::Path,
    name: &str,
) -> Result<String, String> {
    let path = directory.join(name);
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(super) fn shader_directory(reader: &TagReader) -> Option<std::path::PathBuf> {
    let TagSource::LooseFolder { root, .. } = &reader.source else {
        return None;
    };
    Some(root.parent()?.join("shaders").join("fx"))
}

/// A Halo CE shader's flags by name. Its root holds a `flags` field per
/// section (the shader's own and its kind's) and sometimes `extra flags`,
/// whose options don't repeat, so a name is asked of each.
#[derive(Clone, Copy)]
pub(in crate::app::model_preview) struct ShaderFlags<'a>(TagStruct<'a>);

impl<'a> ShaderFlags<'a> {
    pub(in crate::app::model_preview) fn of(root: TagStruct<'a>) -> Self {
        Self(root)
    }

    /// Whether the option `name` is set in any of the root's flags.
    pub(in crate::app::model_preview) fn has(&self, name: &str) -> bool {
        has_flag_in_any(self.0, "flags", name) || has_flag_in_any(self.0, "extra flags", name)
    }
}

/// A pass of the shaders below: nothing but what differs.
pub(super) struct SimplePass {
    pub(super) stage: Dx9VertexStage,
    pub(super) fragment: String,
    pub(super) blend: Dx9Blend,
    pub(super) two_sided: bool,
    pub(super) write_depth: bool,
    pub(super) channels: [bool; 4],
    pub(super) textures: Vec<Dx9DrawTexture>,
    pub(super) constants: Vec<(u32, [f32; 4])>,
    pub(super) rows: [[f32; 4]; H2_TEXTURE_ROWS],
}

impl SimplePass {
    pub(super) fn build(self) -> Dx9DrawPass {
        let depth = Dx9Depth {
            compare: Some(Dx9Compare::LessOrEqual),
            write: self.write_depth,
        };
        let mut pass = Dx9DrawPass::new(self.stage, self.fragment, depth);
        pass.blend = Some(self.blend);
        pass.cull = if self.two_sided {
            Dx9Cull::None
        } else {
            Dx9Cull::Back
        };
        pass.channels = self.channels;
        pass.textures = self.textures;
        pass.constants = self.constants;
        pass.texture_rows = self.rows;
        pass.light_direct = FIRST_LIGHT_DIRECTION;
        pass
    }
}

pub(super) fn plain_blend(source: Dx9BlendFactor, destination: Dx9BlendFactor) -> Dx9Blend {
    Dx9Blend {
        equation: Dx9BlendEquation::Add,
        source,
        destination,
        constant: [0; 4],
    }
}

pub(super) fn normalization(slot: u32) -> Dx9DrawTexture {
    let mut cube = texture(slot, SamplerKind::Cube, None);
    cube.normalization = true;
    cube
}

/// A flat normal, as the default vector bitmap has it.
pub(super) fn flat_normal() -> Arc<Image> {
    Arc::new(Image::Flat(super::super::super::materials::TextureImage {
        rgba: vec![128, 128, 255, 255],
        width: 1,
        height: 1,
        repeat_x: true,
        repeat_y: true,
        scale: [1.0, 1.0],
    }))
}
