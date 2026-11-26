//! What one paint of a preview draws: the frame snapshot the panel builds
//! from its state, the visibility it derives and the function values.

use super::dx9::passes::Dx9Layer;
use super::functions::change_colors_for;
use super::{
    MaterialTextures, ModelGpuCamera, ModelPreviewData, ModelPreviewLayer, ModelPreviewState,
    ModelRenderMode, PreviewCamera, RenderModelPreview, animation_skinning_rows, ce, common,
};
use eframe::egui;
use eframe::egui::Ui;
use std::sync::Arc;

mod visibility;
pub(in crate::app::model_preview) use visibility::*;
mod functions;
pub(in crate::app::model_preview) use functions::*;

/// The batches the current toggles and region selections draw.
///
/// The overlay toggles are draw-time filters: the collision and physics
/// layers sit merged in the geometry from the moment the worker delivers
/// them, and showing or hiding them costs one batch-list rebuild — not the
/// tag re-reads that froze the frame when the toggles re-merged the preview.
/// Gated on `overlays_loaded` so a standalone collision/physics tag — whose
/// MAIN content uses these region names — is never filtered.
///
/// An overlay region shares its selection with the render region of the same
/// name, so it follows the variant when its permutations are named like the
/// render model's. When they aren't — a Halo CE gbxmodel names its monitor
/// permutations `monitor`/`lightning-100` while the collision model's only
/// one is `__base` — the selected name matches no overlay batch, and the
/// overlay draws its region's first permutation instead of vanishing.
/// The frame `state` shows of `data` in `rect` at `time` over `background`,
/// with `forced` exports set over what the functions make. The app and the
/// headless harness both build their frames here.
pub(in crate::app::model_preview) fn build_frame(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
    rect: egui::Rect,
    time: f32,
    background: [f32; 3],
    forced: &[(String, f32)],
) -> ModelGpuFrame {
    let camera = PreviewCamera::new(data, state, rect);
    let preview = Arc::clone(&data.preview);
    let mut visible_batches = visible_batch_indices(&preview, state);
    // Collision and physics draw in a pass of their own, over the model.
    let overlay_batches = take_overlay_batches(&mut visible_batches, &preview);
    let shield_batches = visible_batches.clone();
    let highlight = highlighted_material_batches(data, state, &shield_batches);
    without_shield_parts(
        &mut visible_batches,
        &preview,
        data.textures.as_deref().map(Vec::as_slice),
    );
    let skinning_rows = animation_skinning_rows(data, state);
    let mut function_values = function_values_for(data, &state.function_inputs, time);
    // What a function exports, which its inputs cannot reach: a scripted
    // value, say. Only the harness forces any.
    if let Some(values) = function_values.as_mut() {
        for (name, value) in forced {
            values.insert(name.clone(), *value);
        }
    }
    let mut frame = ModelGpuFrame {
        preview,
        geometry_id: data.geometry_id,
        mesh_id: data.mesh_id,
        textures_id: data.textures_id,
        visible_batches,
        overlay_batches,
        shield_batches,
        highlight,
        camera: camera.gpu_uniforms(),
        render_mode: state.render_mode,
        background,
        show_backfaces: state.show_backfaces,
        show_grid: state.show_grid,
        textures: state
            .render_mode
            .uses_textures()
            .then(|| data.textures.clone())
            .flatten(),
        bones: skinning_rows.clone().map(Arc::new),
        game_shaders: state.game_shaders,
        default_lights: state.default_lights,
        active_camo: state.active_camo,
        bloom: state.bloom,
        self_shadow: state.self_shadow,
        prt: state.prt,
        light_follows_camera: state.light_follows_camera,
        time,
        h2: Default::default(),
        ce: Default::default(),
        object_moves: object_moves(data, &state.function_inputs, time, function_values.as_ref()),
        function_values,
        attached_values: attached_values(data, time),
        attached_colors: attached_colors(data),
        shown: common::features::FeatureShown::of(&data.preview, &state.region_selections),
        change_colors: change_colors_for(
            &data.change_colors,
            state
                .selected_variant
                .and_then(|index| data.variants.get(index))
                .map_or("", |variant| variant.name.as_str()),
        ),
    };
    super::renderer::prepare_game_frames(&mut frame, data, state, time);
    frame
}

/// The parts of `visible` the Material Editor's hovered material covers.
fn highlighted_material_batches(data: &ModelPreviewData, state: &ModelPreviewState, visible: &[usize]) -> Vec<usize> {
    let Some(hovered) = state.material_editor.as_ref().and_then(|editor| editor.hovered.as_ref()) else {
        return Vec::new();
    };
    let materials = super::material_editor::editor_materials(&data.preview);
    let Some(material) = materials.iter().find(|material| &material.key == hovered) else {
        return Vec::new();
    };
    super::material_editor::highlighted_batches(&data.preview, material)
        .into_iter()
        .filter(|batch| visible.contains(batch))
        .collect()
}

/// Moves `visible`'s collision and physics batches out, and returns them.
fn take_overlay_batches(visible: &mut Vec<usize>, preview: &RenderModelPreview) -> Vec<usize> {
    let is_overlay = |index: &usize| {
        preview.batches.get(*index).is_some_and(|batch| {
            matches!(batch.layer, ModelPreviewLayer::Collision | ModelPreviewLayer::Physics)
        })
    };
    let overlays = visible.iter().copied().filter(is_overlay).collect();
    visible.retain(|index| !is_overlay(index));
    overlays
}

#[derive(Clone)]
pub(in crate::app::model_preview) struct ModelGpuFrame {
    pub(in crate::app::model_preview) preview: Arc<RenderModelPreview>,
    pub(in crate::app::model_preview) geometry_id: u64,
    /// See `ModelPreviewData::mesh_id`.
    pub(in crate::app::model_preview) mesh_id: u64,
    /// What `textures` belongs to; see `ModelPreviewData::textures_id`.
    pub(in crate::app::model_preview) textures_id: u64,
    pub(in crate::app::model_preview) visible_batches: Vec<usize>,
    /// The visible collision and physics batches, left out of
    /// `visible_batches`: they draw see-through with solid edges, over the
    /// model, whatever the render mode.
    pub(in crate::app::model_preview) overlay_batches: Vec<usize>,
    /// `visible_batches` with the parts only a shield draws.
    pub(in crate::app::model_preview) shield_batches: Vec<usize>,
    /// The parts the Material Editor's hovered material covers, which draw
    /// lit over the finished picture.
    pub(in crate::app::model_preview) highlight: Vec<usize>,
    pub(in crate::app::model_preview) camera: ModelGpuCamera,
    pub(in crate::app::model_preview) render_mode: ModelRenderMode,
    pub(in crate::app::model_preview) show_backfaces: bool,
    /// What the frame clears to.
    pub(in crate::app::model_preview) background: [f32; 3],
    /// Draw the world-unit ground grid on the z = 0 plane.
    pub(in crate::app::model_preview) show_grid: bool,
    /// `None` until the worker has resolved them, or when shading is off.
    pub(in crate::app::model_preview) textures: Option<Arc<Vec<MaterialTextures>>>,
    /// Skinning matrices for this frame — three vec4 rows per bone, in
    /// `RenderModelPreview::nodes` order. `None` draws the bind pose.
    pub(in crate::app::model_preview) bones: Option<Arc<Vec<[f32; 4]>>>,
    /// The selected variant's change colors, for the game's passes.
    pub(in crate::app::model_preview) change_colors: [[f32; 3]; 4],
    /// Halo CE's: each object's state, point lights and the panel's plays.
    pub(in crate::app::model_preview) ce: ce::frame::CeFrame,
    /// Whether those change by themselves.
    pub(in crate::app::model_preview) object_moves: bool,
    /// What the game's shaders can read of the object's functions.
    pub(in crate::app::model_preview) function_values: Option<super::functions::FunctionValues>,
    /// What a held weapon's own functions read, by owner.
    pub(in crate::app::model_preview) attached_values: Vec<(u8, super::functions::FunctionValues)>,
    /// Each attached Halo 2 or Halo 3 object's own change colors, by owner.
    pub(in crate::app::model_preview) attached_colors: Vec<(u8, [[f32; 3]; 4])>,
    /// Which of each feature show under the chosen regions.
    pub(in crate::app::model_preview) shown: common::features::FeatureShown,
    /// Draw materials through the game's own passes.
    pub(in crate::app::model_preview) game_shaders: bool,
    pub(in crate::app::model_preview) default_lights: bool,
    pub(in crate::app::model_preview) active_camo: bool,
    pub(in crate::app::model_preview) bloom: bool,
    pub(in crate::app::model_preview) self_shadow: bool,
    pub(in crate::app::model_preview) prt: bool,
    pub(in crate::app::model_preview) light_follows_camera: bool,
    /// Seconds, for what the game's shaders animate.
    pub(in crate::app::model_preview) time: f32,
    /// Halo 2's: a PRT model's constants.
    pub(in crate::app::model_preview) h2: super::h2::frame::H2Frame,
}

/// Wrapped to keep a frame time precise in the `f32` the shaders take.
const TIME_WRAP: f64 = 3600.0;

/// The time the preview's simulations and functions run by.
pub(in crate::app::model_preview) fn preview_clock(ui: &Ui) -> f32 {
    ui.input(|input| input.time % TIME_WRAP) as f32
}

impl ModelGpuFrame {
    /// The outputs of the object that owns `material`.
    pub(in crate::app::model_preview) fn outputs_of(&self, material: usize) -> [f32; 4] {
        let object = self
            .preview
            .materials
            .get(material)
            .map_or(0, |material| material.object);
        self.ce.outputs_of(object).unwrap_or([0.0; 4])
    }

    /// The function values of the object that owns `material`.
    pub(in crate::app::model_preview) fn function_values_of(
        &self,
        material: usize,
    ) -> Option<&super::functions::FunctionValues> {
        let object = self
            .preview
            .materials
            .get(material)
            .map_or(0, |material| material.object);
        self.function_values_of_object(object)
    }

    pub(in crate::app::model_preview) fn function_values_of_object(
        &self,
        object: u8,
    ) -> Option<&super::functions::FunctionValues> {
        if object == 0 {
            return self.function_values.as_ref();
        }
        self.attached_values
            .iter()
            .find(|(owner, _)| *owner == object)
            .map(|(_, values)| values)
    }

    /// The change colors of the object that owns `material`.
    pub(in crate::app::model_preview) fn change_colors_of(&self, material: usize) -> [[f32; 3]; 4] {
        let object = self
            .preview
            .materials
            .get(material)
            .map_or(0, |material| material.object);
        self.change_colors_of_object(object)
    }

    /// The opened object's change colors by the selected variant; each
    /// attached object has its own, as `object_set_initial_change_colors`
    /// gives every object.
    pub(in crate::app::model_preview) fn change_colors_of_object(
        &self,
        object: u8,
    ) -> [[f32; 3]; 4] {
        if object == 0 {
            return self.change_colors;
        }
        if let Some((_, colors)) = self
            .attached_colors
            .iter()
            .find(|(owner, _)| *owner == object)
        {
            return *colors;
        }
        self.ce.change_colors_of(object).unwrap_or([[1.0; 3]; 4])
    }

    /// Whether anything in the frame changes over time, so the view keeps
    /// redrawing: an effect or simulation that runs, or a pass that animates.
    pub(in crate::app::model_preview) fn moves(&self) -> bool {
        // What draws, lights or simulates outside the passes runs whatever
        // the passes do.
        if self.object_moves && has_effects(&self.preview) {
            return true;
        }
        let Some(textures) = self.textures.as_ref() else {
            return false;
        };
        // Halo 3 has no layers to ask: what moves it is all in `object_moves`.
        if self.object_moves && textures.iter().any(|texture| texture.gen3.is_some()) {
            return true;
        }
        self.visible_batches.iter().any(|&index| {
            let Some(batch) = self.preview.batches.get(index) else {
                return false;
            };
            textures
                .get(batch.material_index as usize)
                .is_some_and(|material| {
                    Dx9Layer::ALL
                        .iter()
                        .filter(|layer| self.dx9_draws(**layer))
                        .flat_map(|layer| &material.dx9_layers[*layer as usize])
                        .any(|pass| {
                            let outputs = self.outputs_of(batch.material_index as usize);
                            pass.moves()
                                || pass.moves_with(&outputs)
                                || (self.object_moves && pass.reads_outputs())
                        })
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::material_editor::MaterialKey;
    use super::*;

    fn frame_of(state: &ModelPreviewState) -> ModelGpuFrame {
        let Some(Ok(data)) = state.data.as_ref() else { unreachable!() };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
        build_frame(data, state, rect, 0.0, [0.0; 3], &[])
    }

    /// The Material Editor's hovered material lights the parts it draws,
    /// shown ones only; with nothing hovered, nothing is lit.
    #[test]
    fn the_hovered_material_lights_its_shown_parts() {
        let mut state = ModelPreviewState::loaded_with_shaders_for_test(
            "objects\\smoke.render_model",
            &[(b"rmsh", "shaders\\body"), (b"rmsh", "shaders\\visor")],
        );
        state.material_editor = Some(Default::default());
        // As a load picks it.
        state.region_selections.insert(
            "body".to_owned(),
            super::super::ModelRegionSelection { enabled: true, permutation: "base".to_owned() },
        );
        assert_eq!(frame_of(&state).visible_batches, [0, 1]);
        assert!(frame_of(&state).highlight.is_empty(), "nothing hovered");

        state.material_editor.as_mut().unwrap().hovered =
            Some(MaterialKey::new(u32::from_be_bytes(*b"rmsh"), "shaders\\visor"));
        assert_eq!(frame_of(&state).highlight, [1]);

        // Its region hidden, it has no part to light.
        state.region_selections.insert(
            "body".to_owned(),
            super::super::ModelRegionSelection { enabled: false, permutation: "base".to_owned() },
        );
        assert!(frame_of(&state).highlight.is_empty(), "hidden parts stay dark");
    }
}
