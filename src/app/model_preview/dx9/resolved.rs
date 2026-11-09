//! What the DX9 pipeline does to a model's resolved materials: the layers
//! an object's overlay and damage shaders share with every material of it,
//! the object lightmap's fallback, and the preview's shadow passes.

use super::super::{MaterialTextures, RenderModelPreviewMaterial};
use super::passes::{Dx9DrawPass, Dx9Layer, dx9_shadow_passes};
use super::stage::Dx9VertexStage;

/// Every resolved material's DX9 layers, finished once the model's are all
/// resolved.
pub(in crate::app::model_preview) fn finish_dx9_layers(
    materials: &[RenderModelPreviewMaterial],
    textures: &mut [MaterialTextures],
) {
    share_object_layers(materials, textures);
    for texture in textures {
        // A pass with no object implementation still has the fallback's.
        if texture.h2_layers[Dx9Layer::ObjectLightmap as usize].is_empty() {
            texture.h2_layers[Dx9Layer::ObjectLightmap as usize] =
                texture.h2_layers[Dx9Layer::Lightmap as usize].clone();
        }
        let lit = [Dx9Layer::Lightmap, Dx9Layer::Prt]
            .iter()
            .flat_map(|layer| &texture.h2_layers[*layer as usize])
            .any(|pass| pass.stage != Dx9VertexStage::CeModel);
        if lit || texture.h3.is_some() {
            for (layer, pass) in dx9_shadow_passes() {
                texture.h2_layers[layer as usize] = vec![pass];
            }
        }
    }
}

/// A model's hologram shader is on no batch: every material of its object
/// takes its passes. So do its damage shaders, which take their turn over the
/// object; a hologram shader's bloom passes go to the frame's bloom with the
/// rest. A held weapon is an object of its own, without them.
pub(in crate::app::model_preview) fn share_object_layers(
    materials: &[RenderModelPreviewMaterial],
    textures: &mut [MaterialTextures],
) {
    let transparent = Dx9Layer::Transparent as usize;
    let bloom = Dx9Layer::Bloom as usize;
    if let Some(overlay) = materials.iter().position(|material| material.ce.overlay) {
        let passes = std::mem::take(&mut textures[overlay].h2_layers[transparent]);
        share_with_object(
            materials,
            textures,
            materials[overlay].object,
            Dx9Layer::Hologram as usize,
            &passes,
        );
    }
    for (layer, from) in [
        (Dx9Layer::ShieldDamaged, transparent),
        (Dx9Layer::Overshield, transparent),
        (Dx9Layer::HologramBloom, bloom),
    ] {
        if let Some(owner) = materials
            .iter()
            .position(|material| material.h2.shield == Some(layer))
        {
            let passes = std::mem::take(&mut textures[owner].h2_layers[from]);
            share_with_object(
                materials,
                textures,
                materials[owner].object,
                layer as usize,
                &passes,
            );
        }
    }
}

/// Give every material of `object` a copy of `passes` as its `layer`.
fn share_with_object(
    materials: &[RenderModelPreviewMaterial],
    textures: &mut [MaterialTextures],
    object: u8,
    layer: usize,
    passes: &[Dx9DrawPass],
) {
    for (material, texture) in materials.iter().zip(textures) {
        if material.object == object {
            texture.h2_layers[layer] = passes.to_vec();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The model's hologram and damage passes go to its own materials, never
    /// to a held weapon's.
    #[test]
    fn a_models_overlay_passes_stay_on_its_own_object() {
        let material = |object, overlay, shield| {
            let mut material = RenderModelPreviewMaterial {
                object,
                ..Default::default()
            };
            material.ce.overlay = overlay;
            material.h2.shield = shield;
            material
        };
        let materials = [
            material(0, false, None),
            material(0, true, None),
            material(0, false, Some(Dx9Layer::ShieldDamaged)),
            material(1, false, None),
        ];
        let mut textures = vec![MaterialTextures::default(); materials.len()];
        let [(_, pass), ..] = dx9_shadow_passes();
        textures[1].h2_layers[Dx9Layer::Transparent as usize] = vec![pass.clone()];
        textures[2].h2_layers[Dx9Layer::Transparent as usize] = vec![pass];
        share_object_layers(&materials, &mut textures);
        let has =
            |index: usize, layer: Dx9Layer| !textures[index].h2_layers[layer as usize].is_empty();
        for layer in [Dx9Layer::Hologram, Dx9Layer::ShieldDamaged] {
            assert!(
                has(0, layer) && has(1, layer) && has(2, layer),
                "{layer:?} on the model"
            );
            assert!(!has(3, layer), "{layer:?} on the weapon");
        }
    }
}
