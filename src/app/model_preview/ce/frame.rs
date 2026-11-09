//! What a frame holds of Halo CE's: each object's state, its point lights and
//! the panel's plays of its event effects.

use super::super::ModelPreviewData;
use super::super::frame::ModelGpuFrame;

/// What a frame holds of Halo CE's.
#[derive(Clone, Default)]
pub(in crate::app::model_preview) struct CeFrame {
    /// Each object's `A out` to `D out` and change colors this frame, the
    /// opened one first. Empty with none.
    pub(in crate::app::model_preview) objects: Vec<CeObjectState>,
    /// Each object's point lights this frame, in step with `objects`.
    pub(in crate::app::model_preview) points: Vec<[[f32; 4]; super::lights::CE_POINT_LIGHT_ROWS]>,
    /// The panel's plays of the event effects.
    pub(in crate::app::model_preview) plays: super::effects::CeEventPlays,
}

impl CeFrame {
    /// What `panel` shows of `data` at `time`, without the point lights,
    /// which [`prepare`] lights once the rest of the frame is set.
    pub(in crate::app::model_preview) fn of(
        data: &ModelPreviewData,
        panel: &super::panel::CePanelState,
        time: f32,
    ) -> Self {
        Self {
            objects: object_states(data, &panel.function_inputs, &panel.shots, time),
            points: Vec::new(),
            plays: event_plays(data, &panel.event_counts, panel.event_scales),
        }
    }

    /// `object`'s `A out` to `D out` this frame; none for one it has not.
    pub(in crate::app::model_preview) fn outputs_of(&self, object: u8) -> Option<[f32; 4]> {
        self.objects.get(object as usize).map(|state| state.outputs)
    }

    /// `object`'s change colors this frame; none for one it has not.
    pub(in crate::app::model_preview) fn change_colors_of(
        &self,
        object: u8,
    ) -> Option<[[f32; 3]; 4]> {
        self.objects
            .get(object as usize)
            .map(|state| state.change_colors)
    }
}

/// The frame's point lights, once the rest of it is set.
pub(in crate::app::model_preview) fn prepare(frame: &mut ModelGpuFrame) {
    frame.ce.points = frame.ce_point_lights();
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
pub(in crate::app::model_preview) fn object_functions(
    data: &ModelPreviewData,
) -> impl Iterator<Item = &super::functions::CeObjectFunctions> {
    data.ce.functions.iter().chain(
        data.ce
            .attached
            .iter()
            .filter_map(|object| object.functions.as_ref()),
    )
}

pub(in crate::app::model_preview) fn object_states(
    data: &ModelPreviewData,
    set: &super::functions::CeFunctionInputs,
    shots: &super::firing::CeShots,
    time: f32,
) -> Vec<CeObjectState> {
    let state = |functions: Option<&super::functions::CeObjectFunctions>, object, colors| {
        let (outputs, active) = functions.map_or(([0.0; 4], [false; 4]), |functions| {
            let fired = super::firing::fired_input(
                &data.preview.ce.firing,
                shots,
                data.geometry_id,
                object,
                time,
            );
            let inputs =
                functions.with_fired(functions.inputs_for(set, (data.geometry_id, object)), fired);
            functions.outputs_active(&inputs, super::functions::object_time(time, object))
        });
        CeObjectState {
            outputs,
            active,
            change_colors: super::functions::ce_change_colors(colors),
        }
    };
    let mut states = vec![state(data.ce.functions.as_ref(), 0, &data.change_colors)];
    for (index, object) in data.ce.attached.iter().enumerate() {
        states.push(state(
            object.functions.as_ref(),
            index as u8 + 1,
            &object.change_colors,
        ));
    }
    states
}

impl ModelGpuFrame {
    /// `lights_prepare_for_object_dynamic`, for the opened object and what it
    /// holds, as one sphere around the model.
    pub(in crate::app::model_preview) fn ce_point_lights(
        &self,
    ) -> Vec<[[f32; 4]; super::lights::CE_POINT_LIGHT_ROWS]> {
        self.ce_point_lights_with(&[])
    }

    /// The same, with lights the object's effects have made besides.
    pub(in crate::app::model_preview) fn ce_point_lights_with(
        &self,
        made: &[super::lights::CeLight],
    ) -> Vec<[[f32; 4]; super::lights::CE_POINT_LIGHT_ROWS]> {
        if self.preview.ce.lights.is_empty() && made.is_empty() {
            return Vec::new();
        }
        let lights: std::borrow::Cow<'_, [super::lights::CeLight]> = if made.is_empty() {
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
            let state = self.ce.objects.get(usize::from(object));
            (
                state.map_or([0.0; 4], |state| state.outputs),
                state.map_or([[1.0; 3]; 4], |state| state.change_colors),
            )
        };
        (0..self.ce.objects.len().max(1) as u8)
            .map(|object| {
                super::lights::point_light_rows(&lights, object, center, reach, state, bones)
            })
            .collect()
    }
}

/// The panel's plays of `data`'s event effects, by index among its effects.
pub(in crate::app::model_preview) fn event_plays(
    data: &ModelPreviewData,
    counts: &super::functions::CeEventCounts,
    scales: [f32; 2],
) -> super::effects::CeEventPlays {
    let effects = &data.preview.ce.effects;
    let count = |index| counts.get(&(data.geometry_id, index)).copied().unwrap_or(0);
    super::effects::CeEventPlays {
        counts: (0..effects.len()).map(count).collect(),
        scales,
    }
}
