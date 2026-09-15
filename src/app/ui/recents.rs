//! The recent-folders menu, shared by the File menu and the kit tab bar.
//! It owns presentation and choice collection; opening and forgetting belong to the controller.

use super::shell::recent_folder_menu_label;
use super::*;

/// What the user picked from a recents menu.
pub(super) enum RecentAction {
    Open(PathBuf),
    /// Forget one entry without opening it.
    Forget(PathBuf),
    ForgetAll,
}

fn recent_folder_path_button(ui: &mut Ui, label: &str, width: f32) -> egui::Response {
    let size = Vec2::new(width, ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect.expand(visuals.expansion),
            visuals.rounding,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
        );
        ui.painter().text(
            egui::pos2(rect.left() + ui.spacing().button_padding.x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Button.resolve(ui.style()),
            visuals.text_color(),
        );
    }
    response
}

/// Draw the recent-folders list as menu items, each with a button to forget it.
///
/// Returns the choice rather than acting on it: this is rendered inside a menu
/// closure that already holds a borrow of the app, and every action needs a
/// mutable one.
pub(super) fn draw_recent_folders_menu(ui: &mut Ui, recents: &[PathBuf]) -> Option<RecentAction> {
    if recents.is_empty() {
        ui.add_enabled(false, egui::Button::new("No recent folders"));
        return None;
    }
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let longest_label = recents
        .iter()
        .map(|path| {
            ui.painter()
                .layout_no_wrap(recent_folder_menu_label(path), font_id.clone(), text_dark())
                .size()
                .x
        })
        .fold(0.0_f32, f32::max);
    let clear_width = 18.0;
    let gap = 4.0;
    let row_height = ui.spacing().interact_size.y;
    let path_padding = ui.spacing().button_padding.x * 2.0;
    let menu_width = (longest_label + path_padding + gap + clear_width).max(240.0);
    let path_width = menu_width - clear_width - gap;
    // Keep this exact and derive every row from it. Basing the button width on
    // `available_width()` feeds the popup's size from the previous frame back
    // into its next layout pass, causing the menu to grow continuously.
    ui.set_width(menu_width);
    let mut action = None;
    for path in recents {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let full = path.display().to_string();
            let label = recent_folder_menu_label(path);
            if recent_folder_path_button(ui, &label, path_width)
                .on_hover_text(&full)
                .clicked()
            {
                action = Some(RecentAction::Open(path.clone()));
                ui.close_menu();
            }
            if ui
                .add_sized([clear_width, row_height], egui::Button::new("×"))
                .on_hover_text("Remove from recent folders")
                .clicked()
            {
                // Deliberately does not close the menu: removing several
                // entries in a row is the common case.
                action = Some(RecentAction::Forget(path.clone()));
            }
        });
    }
    ui.separator();
    if icon_text_button(ui, ButtonIcon::Clear, "Clear Recent Folders", true).clicked() {
        action = Some(RecentAction::ForgetAll);
        ui.close_menu();
    }
    action
}

impl Baboon {
    pub(super) fn apply_recent_action(&mut self, action: RecentAction, ctx: &egui::Context) {
        match action {
            RecentAction::Open(path) => self.load_recent_folder(path, ctx.clone()),
            RecentAction::Forget(path) => {
                self.remove_recent_folder(&path);
                self.status = format!("Removed {} from recent folders", path.display());
            }
            RecentAction::ForgetAll => {
                self.recent_folders.clear();
                self.status = "Cleared recent folders".to_owned();
            }
        }
    }
}
