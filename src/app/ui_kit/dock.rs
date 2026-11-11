//! A page split into a main card and a sidebar along its right edge, the
//! sidebar as wide as its left edge was last dragged, as Blender docks its
//! editors. The model preview and the bitmap preview both lay out this way.

use eframe::egui;
use eframe::egui::{Rect, Sense, Ui};

use super::style::foundation_group_edge;

/// How wide a sidebar may be, and what it leaves the main card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct SidebarLimits {
    /// The narrowest the sidebar is dragged to.
    pub(in crate::app) min: f32,
    /// The widest it is dragged to.
    pub(in crate::app) max: f32,
    /// The narrowest the main card is squeezed to.
    pub(in crate::app) min_main: f32,
    /// The gap between the two, which is the handle that drags the width.
    pub(in crate::app) gap: f32,
}

/// Where the main card, the handle and the sidebar go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct SidebarSplit {
    pub(in crate::app) main: Rect,
    pub(in crate::app) handle: Rect,
    pub(in crate::app) sidebar: Rect,
}

/// The sidebar's width for a page `page_width` wide: as asked, but never
/// narrower than its least, nor so wide the main card goes below its own.
pub(in crate::app) fn clamp_sidebar_width(width: f32, page_width: f32, limits: SidebarLimits) -> f32 {
    let widest = (page_width - limits.min_main - limits.gap).min(limits.max);
    let width = if width.is_finite() { width } else { limits.min };
    width.min(widest).max(limits.min)
}

/// Splits `area` into the main card, the handle and a sidebar `width` wide.
pub(in crate::app) fn split_sidebar(area: Rect, width: f32, limits: SidebarLimits) -> SidebarSplit {
    let width = clamp_sidebar_width(width, area.width(), limits);
    let sidebar = Rect::from_min_max(egui::pos2(area.right() - width, area.top()), area.max);
    let handle = Rect::from_min_max(
        egui::pos2(sidebar.left() - limits.gap, area.top()),
        egui::pos2(sidebar.left(), area.bottom()),
    );
    let main = Rect::from_min_max(area.min, egui::pos2(handle.left(), area.bottom()));
    SidebarSplit {
        main,
        handle,
        sidebar,
    }
}

/// The sidebar's left edge: dragging it widens or narrows the sidebar, and
/// the main card takes what is left.
pub(in crate::app) fn draw_sidebar_resize_handle(
    ui: &mut Ui,
    split: &SidebarSplit,
    page_width: f32,
    width: &mut f32,
    limits: SidebarLimits,
    id_salt: &str,
) {
    let response = ui.interact(split.handle, ui.scope_id().with(id_salt), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        let x = split.handle.center().x;
        ui.painter().line_segment(
            [
                egui::pos2(x, split.handle.top() + 4.0),
                egui::pos2(x, split.handle.bottom() - 4.0),
            ],
            egui::Stroke::new(2.0, foundation_group_edge()),
        );
    }
    if response.dragged() {
        // From the width the sidebar is drawn at, so dragging back from a
        // clamp moves it at once.
        let shown = split.sidebar.width();
        *width = clamp_sidebar_width(shown - response.drag_delta().x, page_width, limits);
    }
}

/// Draws `contents` inside `rect`, which the page has already allocated.
pub(in crate::app) fn in_rect<R>(
    ui: &mut Ui,
    rect: Rect,
    salt: &str,
    contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    contents(&mut child)
}

/// The room a sidebar's cards leave at their right: the scroll bar at its
/// widest, and its margin from the cards.
pub(in crate::app) fn sidebar_scroll_gutter(ui: &Ui) -> f32 {
    let scroll = ui.spacing().scroll;
    (scroll.bar_width + scroll.bar_inner_margin).ceil()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: SidebarLimits = SidebarLimits {
        min: 240.0,
        max: 1200.0,
        min_main: 320.0,
        gap: 8.0,
    };

    #[test]
    fn the_sidebar_keeps_its_width_between_its_limits() {
        assert_eq!(clamp_sidebar_width(400.0, 1600.0, LIMITS), 400.0);
        assert_eq!(clamp_sidebar_width(100.0, 1600.0, LIMITS), 240.0);
        assert_eq!(clamp_sidebar_width(f32::NAN, 1600.0, LIMITS), 240.0);
        // The main card keeps its 320 and the gap: 1000 - 328.
        assert_eq!(clamp_sidebar_width(900.0, 1000.0, LIMITS), 672.0);
        assert_eq!(clamp_sidebar_width(5000.0, 4000.0, LIMITS), 1200.0);
    }

    #[test]
    fn the_split_lays_main_handle_and_sidebar_side_by_side() {
        let area = Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(1010.0, 520.0));
        let split = split_sidebar(area, 300.0, LIMITS);
        assert_eq!(split.sidebar, Rect::from_min_max(egui::pos2(710.0, 20.0), egui::pos2(1010.0, 520.0)));
        assert_eq!(split.handle, Rect::from_min_max(egui::pos2(702.0, 20.0), egui::pos2(710.0, 520.0)));
        assert_eq!(split.main, Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(702.0, 520.0)));
    }
}
