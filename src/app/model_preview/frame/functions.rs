//! The function inputs and values a frame reads: the sliders, the attached objects' values and whether anything moves.

use super::super::dx9::passes::Dx9Layer;
use super::super::load::{own_shield_gates, own_state_moves};
use super::super::{MaterialTextures, ModelPreviewData, RenderModelPreview, gen3};

/// Where the names the panel's sliders offer come from, after the Halo 2
/// passes' own and before the shield's gate, in the order the panel lists them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app::model_preview) enum SliderSource {
    /// The preview feature at this path.
    Feature(&'static str),
    /// The gen3 and Halo 4 materials' animated parameters and submit inputs,
    /// a hologram's screen materials' with them.
    Gen3Materials,
}

pub(in crate::app::model_preview) const SLIDER_SOURCES: [SliderSource; 20] = [
    SliderSource::Feature("h2.flares"),
    SliderSource::Gen3Materials,
    SliderSource::Feature("h3.lights"),
    SliderSource::Feature("reach.lights"),
    SliderSource::Feature("reach.cheap_lights"),
    SliderSource::Feature("h4.lights"),
    SliderSource::Feature("h4.cheap_lights"),
    SliderSource::Feature("h4.flares"),
    SliderSource::Feature("gen3.flares"),
    SliderSource::Feature("gen3.light_volumes"),
    SliderSource::Feature("h4.tracers"),
    SliderSource::Feature("particles"),
    SliderSource::Feature("gen3.strips"),
    SliderSource::Feature("gen3.cheap_emitters"),
    SliderSource::Feature("gen3.screen_effects"),
    SliderSource::Feature("h2.point_lights"),
    SliderSource::Feature("h2.gel_lights"),
    SliderSource::Feature("h2.light_volumes"),
    SliderSource::Feature("h2.contrails"),
    SliderSource::Feature("h2.liquids"),
];

/// The names the shaders, lights and effects of `object` read, besides what
/// its functions import: 0 for the opened object, or an attached one's owner.
pub(in crate::app::model_preview) fn shader_input_names(
    data: &ModelPreviewData,
    object: u8,
) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let textures = data.textures.iter().flat_map(|textures| textures.iter());
    let owned: Vec<&MaterialTextures> = data
        .preview
        .materials
        .iter()
        .zip(textures)
        .filter(|(material, _)| material.object == object)
        .map(|(_, texture)| texture)
        .collect();
    let passes = owned
        .iter()
        .flat_map(|texture| texture.dx9_layers.iter().flatten());
    let gen3 = owned
        .iter()
        .filter_map(|texture| texture.gen3.as_ref())
        .flat_map(|material| &material.animations);
    let gen3 = gen3.flat_map(|animation| [animation.input.as_str(), animation.range.as_str()]);
    let extensions = owned
        .iter()
        .filter_map(|texture| texture.gen3.as_ref())
        .filter_map(|material| material.extension());
    // What draws after a material reads its object's functions too, as a
    // hologram's screen materials do.
    let after: Vec<&str> = extensions
        .clone()
        .flat_map(|extension| extension.after_inputs())
        .collect();
    let materials: Vec<&str> = gen3
        .chain(extensions.flat_map(|extension| extension.inputs()))
        .chain(after)
        .filter(|name| !name.is_empty())
        .collect();
    let mut sourced: Vec<String> = Vec::new();
    for source in SLIDER_SOURCES {
        match source {
            SliderSource::Feature(path) => {
                sourced.extend(data.preview.feature_inputs(path, object))
            }
            SliderSource::Gen3Materials => {
                sourced.extend(materials.iter().map(|name| (*name).to_owned()))
            }
        }
    }
    // `sub_6C18F0` gates the shield overlay on this itself.
    let gen3_shield = owned
        .iter()
        .filter_map(|texture| texture.gen3.as_ref())
        .any(|gen3| gen3.shield.is_some());
    let shield = data
        .preview
        .materials
        .iter()
        .any(|m| m.object == object && m.shield_layer() == Some(Dx9Layer::ShieldDamaged));
    let own_shield = object == 0 && own_shield_gates(data);
    let gate = (shield || gen3_shield || own_shield).then_some("current_shield_damage");
    let gate = gate
        .into_iter()
        .chain(gen3_shield.then_some(gen3::pipeline::OVERSHIELD));
    let sourced = sourced.iter().map(String::as_str);
    for name in passes
        .flat_map(|pass| pass.function_inputs())
        .chain(sourced)
        .chain(gate)
    {
        if !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    names
}

/// Every name an opened Halo 2 object answers to.
pub(in crate::app::model_preview) fn object_function_values(
    data: &ModelPreviewData,
    set: &super::super::functions::FunctionInputs,
    time: f32,
) -> super::super::functions::FunctionValues {
    let inputs = shader_input_names(data, 0);
    data.object_functions
        .as_ref()
        .map_or_else(Default::default, |functions| {
            functions.values(set, data.geometry_id, &inputs, time, 0)
        })
}

/// Weapons in hand run on their own functions, at rest: the sliders are the
/// opened object's.
pub(in crate::app::model_preview) fn attached_values(
    data: &ModelPreviewData,
    time: f32,
) -> Vec<(u8, super::super::functions::FunctionValues)> {
    if data.attached.is_empty() {
        return Vec::new();
    }
    let rest = super::super::functions::FunctionInputs::default();
    let values = data.attached.iter().map(|attached| {
        let inputs = shader_input_names(data, attached.owner);
        (
            attached.owner,
            attached
                .functions
                .values(&rest, data.geometry_id, &inputs, time, attached.owner),
        )
    });
    values.collect()
}

pub(in crate::app::model_preview) fn attached_colors(
    data: &ModelPreviewData,
) -> Vec<(u8, [[f32; 3]; 4])> {
    data.attached
        .iter()
        .map(|attached| (attached.owner, attached.change_colors))
        .collect()
}

pub(in crate::app::model_preview) fn function_values_for(
    data: &ModelPreviewData,
    set: &super::super::functions::FunctionInputs,
    time: f32,
) -> Option<super::super::functions::FunctionValues> {
    let features = data.preview.any_feature(|list| list.needs_values());
    let gen3 = data
        .textures
        .iter()
        .flat_map(|textures| textures.iter())
        .any(|texture| texture.gen3.is_some());
    (data.object_functions.is_some() || features || gen3)
        .then(|| object_function_values(data, set, time))
}

/// Whether anything on the object changes with time alone.
pub(in crate::app::model_preview) fn object_moves(
    data: &ModelPreviewData,
    set: &super::super::functions::FunctionInputs,
    time: f32,
    function_values: Option<&super::super::functions::FunctionValues>,
) -> bool {
    own_state_moves(data, function_values)
        || object_functions_move(data, set, time)
        || data.preview.any_feature(|list| list.moves())
        || data
            .textures
            .iter()
            .flat_map(|textures| textures.iter())
            .any(|texture| {
                texture.gen3.as_ref().is_some_and(|material| {
                    material
                        .animations
                        .iter()
                        .any(|animation| animation.moves())
                        || material
                            .extension()
                            .is_some_and(|extension| extension.moves())
                        || (material.shield.is_some()
                            && gen3::pipeline::gen3_shield_showing(function_values))
                })
            })
}

/// Whether anything draws, lights or simulates outside the materials' passes:
/// any of the preview's features.
pub(in crate::app::model_preview) fn has_effects(preview: &RenderModelPreview) -> bool {
    preview.any_feature(|list| !list.is_empty())
}

fn object_functions_move(
    data: &ModelPreviewData,
    set: &super::super::functions::FunctionInputs,
    time: f32,
) -> bool {
    let held = attached_values(data, time);
    let held_move = data
        .attached
        .iter()
        .zip(&held)
        .any(|(attached, (_, values))| attached.functions.move_at_rest(values));
    held_move
        || data.object_functions.as_ref().is_some_and(|functions| {
            functions.move_at_rest(&object_function_values(data, set, time))
        })
}
