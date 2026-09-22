//! Function view state shared by every function editor, and the Halo 2 entry
//! point. Halo 2 functions are edited by the same editor as H3+ ones
//! (`draw_function_editor`); only where their bytes come from differs.

use super::*;

/// A Halo 2 `data` byte-block, read the way the H2 engine reads it.
pub(in crate::app) fn h2_tag_function(bytes: &[u8]) -> Option<TagFunction> {
    TagFunction::parse_encoded(FunctionEncoding::H2, bytes).ok()
}

#[cfg(test)]
#[path = "../tests/function_editor_h2.rs"]
mod tests;

impl FunctionView {
    pub(in crate::app) fn from_function(function: TagFunction) -> Self {
        Self {
            function,
            input_name: String::new(),
            range_name: String::new(),
            output_index: None,
            time_period_in_seconds: 0.0,
            edit: None,
            color_types: ColorTypeChoices::All,
        }
    }

    pub(in crate::app) fn from_animated(
        animated: &RenderMethodAnimatedParameter,
        function: TagFunction,
    ) -> Self {
        Self {
            function,
            input_name: animated.input_name.clone(),
            range_name: animated.range_name.clone(),
            output_index: animated.parameter_type.and_then(|kind| {
                OUTPUT_TYPE_OPTIONS
                    .iter()
                    .find(|(_, name)| name.eq_ignore_ascii_case(kind.name()))
                    .map(|(value, _)| *value)
            }),
            time_period_in_seconds: animated.time_period_in_seconds,
            edit: None,
            color_types: ColorTypeChoices::All,
        }
    }

    pub(in crate::app) fn with_edit(mut self, paths: FunctionEditPaths) -> Self {
        self.edit = Some(paths);
        self
    }

    pub(in crate::app) fn with_color_types(mut self, choices: ColorTypeChoices) -> Self {
        self.color_types = choices;
        self
    }

    pub(in crate::app) fn data_bytes(&self) -> Vec<u8> {
        self.function.to_bytes()
    }
}

#[derive(Clone)]
/// Cross-frame function editor state bound to one tag and one captured view.
/// The original write targets and last-applied snapshot prevent selection or
/// presentation-setting changes from redirecting an in-progress edit.
pub(in crate::app) struct FunctionPopup {
    /// The tag the function belongs to — edits target this tag's doc.
    pub(super) tag_key: String,
    pub(super) title: String,
    pub(super) view: FunctionView,
    /// Whether the owning tag is writable (LE loose file). Read-only
    /// tags still open the dialog but disable the controls.
    pub(super) editable: bool,
    /// Snapshot of the values last pushed as edits; lets us emit a
    /// `PendingFieldEdit` only when something actually changed.
    pub(super) last_applied: FunctionSnapshot,
    /// Currently selected LinearKey control point (drag/x-y target).
    pub(super) selected_point: usize,
    /// Selected graph slot in the Foundation H3+ editor (green=0, red=1).
    pub(super) selected_graph: usize,
}

impl FunctionPopup {
    pub(in crate::app) fn new(
        tag_key: String,
        title: String,
        view: FunctionView,
        editable: bool,
    ) -> Self {
        let last_applied = FunctionSnapshot::from_view(&view);
        Self {
            tag_key,
            title,
            view,
            editable,
            last_applied,
            selected_point: 0,
            selected_graph: 0,
        }
    }

    pub(in crate::app) fn apply_draft_color(
        &mut self,
        target: FunctionDraftColorTarget,
        argb: u32,
    ) {
        // The picker carries the swatch's original alpha, so `argb` is whole.
        let FunctionDraftColorTarget::Logical(index) = target;
        let mut editor = TagFunctionEditor::from_function(self.view.function.clone());
        if editor.set_color(index, argb).is_ok() {
            self.view.function = editor.into_function();
        }
    }
}

/// Values that map to writable tag fields. Compared frame-to-frame to
/// decide which `PendingFieldEdit`s to emit.
/// Raw function bytes are compared in full so changes never silently discard
/// unrecognized classic H2 data.
#[derive(Clone, PartialEq)]
pub(in crate::app) struct FunctionSnapshot {
    pub(super) data: Vec<u8>,
    pub(super) output_index: Option<i32>,
    pub(super) input_name: String,
    pub(super) range_name: String,
    pub(super) time_period: f32,
}

impl FunctionSnapshot {
    pub(in crate::app) fn from_view(view: &FunctionView) -> Self {
        Self {
            data: view.data_bytes(),
            output_index: view.output_index,
            input_name: view.input_name.clone(),
            range_name: view.range_name.clone(),
            time_period: view.time_period_in_seconds,
        }
    }
}

/// Edits produced by the function dialog this frame, plus the tag they
/// belong to.
pub(in crate::app) struct FunctionEditBatch {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) edits: Vec<PendingFieldEdit>,
    pub(in crate::app) data_ops: Vec<FunctionDataOp>,
}
