//! The recent-folders menu, shared by the File menu and the kit tab bar.
//! It owns presentation and choice collection; opening and forgetting belong to the controller.

use super::*;
use super::shell::recent_folder_menu_label;

/// What the user picked from a recents menu.
pub(super) enum RecentAction {
    Open(PathBuf),
    /// Forget one entry without opening it.
    Forget(PathBuf),
    ForgetAll,
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
    let mut action = None;
    for path in recents {
        ui.horizontal(|ui| {
            let full = path.display().to_string();
            if ui
                .button(recent_folder_menu_label(path))
                .on_hover_text(&full)
                .clicked()
            {
                action = Some(RecentAction::Open(path.clone()));
                ui.close_menu();
            }
            if ui
                .add(egui::Button::new("×").min_size(Vec2::splat(18.0)))
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
    if ui.button("Clear Recent Folders").clicked() {
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
