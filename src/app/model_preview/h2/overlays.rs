//! The shaders a Halo 2 `.model` draws again over its render model.

use super::super::dx9::passes::Dx9Layer;
use super::super::*;
use blam_tags::TagFile;

/// The shaders a `.model` lays over its render model: its hologram shader
/// and its shield shaders, each drawn again over every section.
pub(in crate::app::model_preview) fn add_overlay_shaders(
    model_tag: &TagFile,
    preview: &mut RenderModelPreview,
) {
    // `sub_6C18F0`: while the model's `hologram control function` is above
    // nothing, `render_hologram_group` draws it again with its hologram shader
    // over every section. That shader's bloom passes join the frame's bloom.
    let shader_group = u32::from_be_bytes(*b"shad");
    let reference = model_tag
        .root()
        .field("hologram shader")
        .and_then(|field| field.value());
    if let Some(blam_tags::TagFieldData::TagReference(reference)) = reference
        && let Some((group, shader_path)) = reference.group_tag_and_name
        && group == shader_group
        && !shader_path.is_empty()
    {
        let control = model_tag
            .root()
            .read_string_id("hologram control function")
            .filter(|name| !name.is_empty());
        if let Some(control) = control {
            preview.h2.hologram_control = Some(control);
            preview.materials.push(RenderModelPreviewMaterial {
                shader_path,
                shader_group,
                overlay: false,
                shield: Some(Dx9Layer::HologramBloom),
                permutation: 0,
                object: 0,
            });
        }
    }
    // `sub_6C18F0` lays its shield shaders over it the same way.
    let damage = model_tag
        .root()
        .field("new damage info")
        .and_then(|field| field.as_block());
    if let Some(damage) = damage.and_then(|block| block.element(0)) {
        let shaders = [
            ("shield damaged shader", Dx9Layer::ShieldDamaged),
            ("overshield shader", Dx9Layer::Overshield),
        ];
        for (name, layer) in shaders {
            let Some(shader_path) = damage
                .read_tag_ref_path(name)
                .filter(|path| !path.is_empty())
            else {
                continue;
            };
            preview.materials.push(RenderModelPreviewMaterial {
                shader_path,
                shader_group,
                overlay: false,
                shield: Some(layer),
                permutation: 0,
                object: 0,
            });
        }
    }
}
