//! Draft history for the function popup and release-time inline commits.
use super::*;

#[derive(Clone)]
pub(super) struct FunctionHistory {
    settled: FunctionView,
    undo: Vec<FunctionView>,
    redo: Vec<FunctionView>,
}

impl FunctionHistory {
    pub(super) fn new(view: &FunctionView) -> Self {
        Self {
            settled: view.clone(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub(super) fn observe(&mut self, view: &FunctionView, dragging: bool) {
        if dragging
            || FunctionSnapshot::from_view(&self.settled) == FunctionSnapshot::from_view(view)
        {
            return;
        }
        self.undo.push(self.settled.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.settled = view.clone();
    }

    pub(super) fn can_step(&self, redo: bool) -> bool {
        if redo {
            !self.redo.is_empty()
        } else {
            !self.undo.is_empty()
        }
    }

    pub(super) fn step(&mut self, view: &mut FunctionView, redo: bool) -> bool {
        self.observe(view, false);
        let (source, target) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let Some(restored) = source.pop() else {
            return false;
        };
        target.push(view.clone());
        *view = restored;
        self.settled = view.clone();
        true
    }
}

#[derive(Clone)]
pub(in crate::app) struct InlineFunctionGesture {
    original: FunctionSnapshot,
    pub(in crate::app) view: FunctionView,
}

impl InlineFunctionGesture {
    pub(in crate::app) fn new(view: &FunctionView) -> Self {
        Self {
            original: FunctionSnapshot::from_view(view),
            view: view.clone(),
        }
    }

    pub(in crate::app) fn matches_document(&self, view: &FunctionView) -> bool {
        self.original == FunctionSnapshot::from_view(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> FunctionView {
        FunctionView::from_function(
            h2_tag_function(&h2_constant_scalar_function_data(1.0, None)).unwrap(),
        )
    }

    #[test]
    fn drag_is_one_step_on_release_and_redo_restores_the_final_position() {
        let mut view = view();
        let original = FunctionSnapshot::from_view(&view);
        let mut history = FunctionHistory::new(&view);
        for value in [1.0, 2.0, 3.0] {
            view.time_period_in_seconds = value;
            history.observe(&view, true);
            assert!(!history.can_step(false));
        }
        history.observe(&view, false);
        assert_eq!(history.undo.len(), 1);
        assert!(history.step(&mut view, false));
        assert!(FunctionSnapshot::from_view(&view) == original);
        assert!(!history.can_step(false));
        assert!(history.step(&mut view, true));
        assert_eq!(view.time_period_in_seconds, 3.0);
        history.observe(&view, false);
        assert_eq!(history.undo.len(), 1);
    }

    #[test]
    fn undo_restores_all_fields_and_new_changes_clear_redo() {
        let mut view = view();
        let mut history = FunctionHistory::new(&view);
        let original = FunctionSnapshot::from_view(&view);
        let mut editor = TagFunctionEditor::from_function(view.function.clone());
        editor.set_function_type(FunctionType::Spline).unwrap();
        view.function = editor.into_function();
        view.input_name = "time".to_owned();
        view.range_name = "random".to_owned();
        view.output_index = Some(2);
        history.observe(&view, false);
        history.step(&mut view, false);
        assert!(FunctionSnapshot::from_view(&view) == original);
        view.input_name = "frame".to_owned();
        history.observe(&view, false);
        assert!(!history.can_step(true));
        history.step(&mut view, false);
        assert!(FunctionSnapshot::from_view(&view) == original);
    }

    #[test]
    fn no_op_gestures_and_external_document_changes() {
        let mut view = view();
        let mut history = FunctionHistory::new(&view);
        let gesture = InlineFunctionGesture::new(&view);
        view.time_period_in_seconds = 2.0;
        history.observe(&view, true);
        view.time_period_in_seconds = 0.0;
        history.observe(&view, false);
        assert!(!history.can_step(false));
        assert!(gesture.matches_document(&view));
        view.input_name = "external edit".to_owned();
        assert!(!gesture.matches_document(&view));
    }

    #[test]
    fn application_undo_and_redo_target_the_open_popup_draft() {
        let mut app = Baboon::for_test();
        let mut popup = FunctionPopup::new("tag".to_owned(), "function".to_owned(), view(), true);
        popup.view.input_name = "time".to_owned();
        popup.history.observe(&popup.view, false);
        let kit = app.model.kits[0].id;
        app.dialogs.open(FunctionPopupWindow {
            popup: Some(popup),
            kit,
            opened_at: None,
        });
        assert!(app.can_undo_current());
        app.undo_current_tag();
        assert!(
            app.dialogs
                .get::<FunctionPopupWindow>()
                .unwrap()
                .popup
                .as_ref()
                .unwrap()
                .view
                .input_name
                .is_empty()
        );
        assert!(app.can_redo_current());
        app.redo_current_tag();
        assert_eq!(
            app.dialogs
                .get::<FunctionPopupWindow>()
                .unwrap()
                .popup
                .as_ref()
                .unwrap()
                .view
                .input_name,
            "time"
        );
        assert!(!app.can_redo_current());
    }
}
