//! One frame and validation palette for ordinary inputs and tag value cells.
use super::*;
use std::sync::Arc;

pub(in crate::app) const INPUT_CORNER_RADIUS: egui::CornerRadius = egui::CornerRadius::same(2);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum InputState {
    #[default]
    Normal,
    Invalid,
    Warning,
}

pub(in crate::app) struct InputAppearance {
    pub fill: Color32,
    pub stroke: Stroke,
    pub corners: egui::CornerRadius,
}

pub(in crate::app) fn input_appearance(
    ui: &Ui,
    state: InputState,
    read_only: bool,
    focused: bool,
) -> InputAppearance {
    let (fill, stroke) = match state {
        InputState::Invalid => (invalid_value_fill(), invalid_value_stroke()),
        InputState::Warning => (
            unapplied_value_fill(),
            Stroke::new(1.0, unapplied_value_text()),
        ),
        InputState::Normal => (
            if read_only {
                foundation_input_read_only()
            } else {
                foundation_input()
            },
            if focused {
                ui.visuals().selection.stroke
            } else {
                Stroke::new(1.0, foundation_input_edge())
            },
        ),
    };
    InputAppearance {
        fill,
        stroke,
        corners: INPUT_CORNER_RADIUS,
    }
}

/// Native text editing and accessibility with Baboon's shared input frame.
/// Explicit frames opt out for search pills and embedded tag-cell editors.
pub(in crate::app) struct SinglelineTextEdit<'a> {
    edit: egui::TextEdit<'a>,
    state: InputState,
    read_only: bool,
    custom_frame: bool,
    margin: egui::Margin,
    background: Option<Color32>,
}

pub(in crate::app) fn singleline_text_edit(
    text: &mut dyn egui::TextBuffer,
) -> SinglelineTextEdit<'_> {
    let read_only = !text.is_mutable();
    SinglelineTextEdit {
        edit: egui::TextEdit::singleline(text).vertical_align(egui::Align::Center),
        state: InputState::Normal,
        read_only,
        custom_frame: false,
        margin: egui::Margin::symmetric(4, 2),
        background: None,
    }
}

pub(in crate::app) fn singleline_text_edit_with_error<'a>(
    _ui: &Ui,
    text: &'a mut dyn egui::TextBuffer,
    invalid: bool,
) -> SinglelineTextEdit<'a> {
    singleline_text_edit(text).validation_state(if invalid {
        InputState::Invalid
    } else {
        InputState::Normal
    })
}

macro_rules! builder {
    ($name:ident, $arg:ident: $ty:ty) => {
        pub(in crate::app) fn $name(mut self, $arg: $ty) -> Self {
            self.edit = self.edit.$name($arg);
            self
        }
    };
}

impl<'a> SinglelineTextEdit<'a> {
    builder!(desired_width, width: f32);
    builder!(min_size, size: Vec2);
    builder!(id, id: egui::Id);
    builder!(id_salt, salt: impl egui::AsIdSalt);
    builder!(font, font: impl Into<egui::FontSelection>);
    builder!(hint_text, text: impl Into<egui::WidgetText>);
    builder!(text_color, color: Color32);
    builder!(vertical_align, align: egui::Align);
    builder!(clip_text, clip: bool);
    builder!(prefix, prefix: impl egui::IntoAtoms<'static>);

    pub(in crate::app) fn layouter(
        mut self,
        layouter: &'a mut dyn FnMut(&Ui, &dyn egui::TextBuffer, f32) -> Arc<egui::Galley>,
    ) -> Self {
        self.edit = self.edit.layouter(layouter);
        self
    }

    pub(in crate::app) fn frame(mut self, frame: egui::Frame) -> Self {
        self.edit = self.edit.frame(frame);
        self.custom_frame = true;
        self
    }

    pub(in crate::app) fn margin(mut self, margin: impl Into<egui::Margin>) -> Self {
        self.margin = margin.into();
        self.edit = self.edit.margin(self.margin);
        self
    }

    pub(in crate::app) fn background_color(mut self, color: Color32) -> Self {
        self.background = Some(color);
        self.edit = self.edit.background_color(color);
        self
    }

    pub(in crate::app) fn interactive(mut self, interactive: bool) -> Self {
        self.read_only |= !interactive;
        self.edit = self.edit.interactive(interactive);
        self
    }

    pub(in crate::app) fn validation_state(mut self, state: InputState) -> Self {
        self.state = state;
        self.edit = match state {
            InputState::Normal => self.edit,
            InputState::Invalid => self.edit.text_color(invalid_value_text()),
            InputState::Warning => self.edit.text_color(unapplied_value_text()),
        };
        self
    }

    pub(in crate::app) fn show(mut self, ui: &mut Ui) -> egui::text_edit::TextEditOutput {
        if self.custom_frame {
            return self.edit.show(ui);
        }
        let background = ui.painter().add(egui::Shape::Noop);
        self.edit = self.edit.frame(egui::Frame::NONE.inner_margin(self.margin));
        let output = self.edit.show(ui);
        let style = input_appearance(ui, self.state, self.read_only, output.response.has_focus());
        let fill = if self.state == InputState::Normal {
            self.background.unwrap_or(style.fill)
        } else {
            style.fill
        };
        ui.painter().set(
            background,
            egui::Shape::rect_filled(output.response.rect, style.corners, fill),
        );
        ui.painter().rect_stroke(
            output.response.rect,
            style.corners,
            style.stroke,
            egui::StrokeKind::Inside,
        );
        output
    }
}

impl egui::Widget for SinglelineTextEdit<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        self.show(ui).response.response
    }
}
