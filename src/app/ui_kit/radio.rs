//! Native radio behavior with the same menu inset as checkbox and icon rows.
use super::*;

pub(in crate::app) trait RadioUi {
    fn baboon_radio_value<'a, Value: PartialEq>(
        &mut self,
        current: &mut Value,
        alternative: Value,
        label: impl egui::IntoAtoms<'a>,
    ) -> egui::Response;
}

impl RadioUi for Ui {
    fn baboon_radio_value<'a, Value: PartialEq>(
        &mut self,
        current: &mut Value,
        alternative: Value,
        label: impl egui::IntoAtoms<'a>,
    ) -> egui::Response {
        if egui::containers::menu::is_in_menu(self) {
            Frame::NONE
                .inner_margin(menu_control_margin(self))
                .show(self, |ui| {
                    ui.spacing_mut().icon_width = 16.0;
                    ui.radio_value(current, alternative, label)
                })
                .inner
        } else {
            self.radio_value(current, alternative, label)
        }
    }
}
