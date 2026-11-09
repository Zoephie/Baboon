//! What one paint of a preview draws: the frame snapshot the panel builds
//! from its state, the visibility it derives and the function values.

use super::dx9::passes::{
    Dx9Layer, change_colors_for, h2_object_light_direction, h2_prt_object_lights,
};
use super::{
    MaterialTextures, ModelGpuCamera, ModelPreviewData, ModelPreviewState, ModelRenderMode,
    PreviewCamera, RenderModelPreview, animation_skinning_rows, camera_light, ce, common, dx9,
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
    let shield_batches = visible_batches.clone();
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
        shield_batches,
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
        h2_prt_clusters: None,
        object_states: object_states(data, &state.ce_function_inputs, &state.ce_shots, time),
        object_moves: object_moves(data, &state.function_inputs, time, function_values.as_ref()),
        function_values,
        attached_values: attached_values(data, time),
        attached_colors: attached_colors(data),
        shown: common::features::FeatureShown::of(&data.preview, &state.region_selections),
        ce_points: Vec::new(),
        ce_plays: ce_event_plays(data, &state.ce_event_counts, state.ce_event_scales),
        change_colors: change_colors_for(
            &data.change_colors,
            state
                .selected_variant
                .and_then(|index| data.variants.get(index))
                .map_or("", |variant| variant.name.as_str()),
        ),
    };
    frame.h2_prt_clusters = frame.h2_object_prt_clusters();
    frame.ce_points = frame.ce_point_lights();
    frame
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
    /// `visible_batches` with the parts only a shield draws.
    pub(in crate::app::model_preview) shield_batches: Vec<usize>,
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
    /// Halo CE: each object's `A out` to `D out` and change colors this frame,
    /// the opened one first. Empty with none.
    pub(in crate::app::model_preview) object_states: Vec<CeObjectState>,
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
    /// Each object's point lights this frame, in step with `object_states`.
    pub(in crate::app::model_preview) ce_points: Vec<[[f32; 4]; ce::lights::CE_POINT_LIGHT_ROWS]>,
    /// The panel's plays of Halo CE's event effects.
    pub(in crate::app::model_preview) ce_plays: ce::effects::CeEventPlays,
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
    /// A PRT model's constants on the object path: lit, then in shadow.
    pub(in crate::app::model_preview) h2_prt_clusters: Option<[Vec<[f32; 4]>; 2]>,
}

/// Wrapped to keep a frame time precise in the `f32` the shaders take.
const TIME_WRAP: f64 = 3600.0;

/// The time the preview's simulations and functions run by.
pub(in crate::app::model_preview) fn preview_clock(ui: &Ui) -> f32 {
    ui.input(|input| input.time % TIME_WRAP) as f32
}

/// What the parts an object owns draw with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) struct CeObjectState {
    pub(in crate::app::model_preview) outputs: [f32; 4],
    /// Which outputs are on, as `object_get_function_value` answers.
    pub(in crate::app::model_preview) active: [bool; 4],
    pub(in crate::app::model_preview) change_colors: [[f32; 3]; 4],
}

/// The opened object's functions, then those of what is attached to it.
fn object_functions(
    data: &ModelPreviewData,
) -> impl Iterator<Item = &ce::functions::CeObjectFunctions> {
    data.ce_object_functions.iter().chain(
        data.ce_attached
            .iter()
            .filter_map(|object| object.functions.as_ref()),
    )
}

pub(in crate::app::model_preview) fn object_states(
    data: &ModelPreviewData,
    set: &ce::functions::CeFunctionInputs,
    shots: &ce::firing::CeShots,
    time: f32,
) -> Vec<CeObjectState> {
    let state = |functions: Option<&ce::functions::CeObjectFunctions>, object, colors| {
        let (outputs, active) = functions.map_or(([0.0; 4], [false; 4]), |functions| {
            let fired = ce::firing::fired_input(
                &data.preview.ce.firing,
                shots,
                data.geometry_id,
                object,
                time,
            );
            let inputs =
                functions.with_fired(functions.inputs_for(set, (data.geometry_id, object)), fired);
            functions.outputs_active(&inputs, ce::functions::object_time(time, object))
        });
        CeObjectState {
            outputs,
            active,
            change_colors: dx9::passes::ce_change_colors(colors),
        }
    };
    let mut states = vec![state(
        data.ce_object_functions.as_ref(),
        0,
        &data.change_colors,
    )];
    for (index, object) in data.ce_attached.iter().enumerate() {
        states.push(state(
            object.functions.as_ref(),
            index as u8 + 1,
            &object.change_colors,
        ));
    }
    states
}

impl ModelGpuFrame {
    /// The outputs of the object that owns `material`.
    pub(in crate::app::model_preview) fn outputs_of(&self, material: usize) -> [f32; 4] {
        let object = self
            .preview
            .materials
            .get(material)
            .map_or(0, |material| material.object);
        self.object_states
            .get(object as usize)
            .map_or([0.0; 4], |state| state.outputs)
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
        self.object_states
            .get(object as usize)
            .map_or([[1.0; 3]; 4], |state| state.change_colors)
    }

    /// Transparent passes draw whether or not game shaders are on.
    pub(in crate::app::model_preview) fn h2_draws(&self, layer: Dx9Layer) -> bool {
        match layer {
            Dx9Layer::Transparent => true,
            Dx9Layer::ActiveCamo => self.active_camo,
            _ => self.game_shaders,
        }
    }

    pub(in crate::app::model_preview) fn h2_object_prt_clusters(
        &self,
    ) -> Option<[Vec<[f32; 4]>; 2]> {
        let prt = self
            .preview
            .h2
            .prt
            .as_ref()
            .filter(|_| self.prt && self.game_shaders && !self.default_lights)?;
        let (lights, ambient) = h2_prt_object_lights(self.h2_object_light());
        Some([
            prt.clusters_lit_by(&lights, ambient),
            prt.clusters_lit_by(&lights[..1], ambient),
        ])
    }

    /// `lights_prepare_for_object_dynamic`, for the opened object and what it
    /// holds, as one sphere around the model.
    pub(in crate::app::model_preview) fn ce_point_lights(
        &self,
    ) -> Vec<[[f32; 4]; ce::lights::CE_POINT_LIGHT_ROWS]> {
        self.ce_point_lights_with(&[])
    }

    /// The same, with lights the object's effects have made besides.
    pub(in crate::app::model_preview) fn ce_point_lights_with(
        &self,
        made: &[ce::lights::CeLight],
    ) -> Vec<[[f32; 4]; ce::lights::CE_POINT_LIGHT_ROWS]> {
        if self.preview.ce.lights.is_empty() && made.is_empty() {
            return Vec::new();
        }
        let lights: std::borrow::Cow<'_, [ce::lights::CeLight]> = if made.is_empty() {
            std::borrow::Cow::Borrowed(&self.preview.ce.lights)
        } else {
            std::borrow::Cow::Owned(self.preview.ce.lights.iter().chain(made).cloned().collect())
        };
        let (low, high) = (self.preview.bounds_min, self.preview.bounds_max);
        let center: [f32; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let reach = (0..3)
            .map(|axis| (high[axis] - center[axis]).powi(2))
            .sum::<f32>()
            .sqrt();
        let bones = self.bones.as_ref().map(|bones| bones.as_slice());
        let state = |object: u8| {
            let state = self.object_states.get(usize::from(object));
            (
                state.map_or([0.0; 4], |state| state.outputs),
                state.map_or([[1.0; 3]; 4], |state| state.change_colors),
            )
        };
        (0..self.object_states.len().max(1) as u8)
            .map(|object| {
                ce::lights::point_light_rows(&lights, object, center, reach, state, bones)
            })
            .collect()
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
        if self.object_moves && textures.iter().any(|texture| texture.h3.is_some()) {
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
                        .filter(|layer| self.h2_draws(**layer))
                        .flat_map(|layer| &material.h2_layers[*layer as usize])
                        .any(|pass| {
                            let outputs = self.outputs_of(batch.material_index as usize);
                            pass.moves()
                                || pass.moves_with(&outputs)
                                || (self.object_moves && pass.reads_outputs())
                        })
                })
        })
    }

    pub(in crate::app::model_preview) fn h2_lightmap_layer(&self) -> Dx9Layer {
        if self.default_lights {
            Dx9Layer::Lightmap
        } else {
            Dx9Layer::ObjectLightmap
        }
    }

    /// Which way the object path's direct light lies from a surface.
    pub(in crate::app::model_preview) fn h2_object_light(&self) -> [f32; 3] {
        if self.light_follows_camera {
            camera_light(self.camera.yaw, self.camera.pitch)
        } else {
            h2_object_light_direction()
        }
    }
}

/// The panel's plays of `data`'s event effects, by index among its effects.
pub(in crate::app::model_preview) fn ce_event_plays(
    data: &ModelPreviewData,
    counts: &ce::functions::CeEventCounts,
    scales: [f32; 2],
) -> ce::effects::CeEventPlays {
    let effects = &data.preview.ce.effects;
    let count = |index| counts.get(&(data.geometry_id, index)).copied().unwrap_or(0);
    ce::effects::CeEventPlays {
        counts: (0..effects.len()).map(count).collect(),
        scales,
    }
}
