//! The caches of icons recoloured for the theme, emptied when a theme changes
//! colours.

use super::{button_icons, close, selection_art, tag_icons};
use eframe::egui;

std::thread_local! {
    /// Image URIs a theme change left behind, for the next frame to forget.
    static FORGOTTEN_ICON_URIS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Empties every recoloured-icon cache, for a theme that changed colours:
/// each would otherwise keep one copy of its icons per colour it was ever
/// drawn in. The URIs they registered are forgotten on the next frame
/// ([`forget_icon_images`]), which has the egui context.
pub(in crate::app) fn forget_recoloured_icons() {
    let mut uris = close::forget_close_svgs();
    uris.extend(selection_art::forget_selection_svgs());
    tag_icons::forget_tag_icon_svgs();
    button_icons::forget_button_icon_svgs();
    FORGOTTEN_ICON_URIS.with(|forgotten| forgotten.borrow_mut().extend(uris));
}

/// Forgets the images a theme change left behind. Called once a frame.
pub(in crate::app) fn forget_icon_images(ctx: &egui::Context) {
    for uri in FORGOTTEN_ICON_URIS.with(|forgotten| std::mem::take(&mut *forgotten.borrow_mut())) {
        ctx.forget_image(&uri);
    }
}
