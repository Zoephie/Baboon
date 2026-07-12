//! Embedded button-icon lookup and display-scale selection.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ButtonIcon {
    Open,
    Import,
    /// Embedded for the staged export-button treatment and covered by asset tests.
    #[allow(dead_code)]
    Export,
    Clear,
    /// Embedded for staged search controls and covered by asset tests.
    #[allow(dead_code)]
    Search,
    Function,
    /// Embedded for staged grouped actions and covered by asset tests.
    #[allow(dead_code)]
    Group,
    FolderClosed,
    FolderOpen,
}

pub(super) fn button_icon_svg(icon: ButtonIcon) -> &'static str {
    match icon {
        ButtonIcon::Open => include_str!("../../assets/Button Icons/Open.svg"),
        ButtonIcon::Import => include_str!("../../assets/Button Icons/Import.svg"),
        ButtonIcon::Export => include_str!("../../assets/Button Icons/Export.svg"),
        ButtonIcon::Clear => include_str!("../../assets/Button Icons/Clear.svg"),
        ButtonIcon::Search => include_str!("../../assets/Button Icons/search.svg"),
        ButtonIcon::Function => include_str!("../../assets/Button Icons/Function.svg"),
        ButtonIcon::Group => include_str!("../../assets/Button Icons/Group.svg"),
        ButtonIcon::FolderClosed => include_str!("../../assets/Button Icons/Folder - closed.svg"),
        ButtonIcon::FolderOpen => include_str!("../../assets/Button Icons/Folder - open.svg"),
    }
}

pub(super) fn paint_button_icon_at(ui: &Ui, icon: ButtonIcon, rect: egui::Rect, color: Color32) {
    let svg = colorized_icon_svg(icon, color);
    let uri = button_icon_uri(ui.ctx(), icon, color);
    egui::Image::from_bytes(uri, svg.into_bytes())
        .fit_to_exact_size(rect.size())
        .tint(Color32::WHITE)
        .paint_at(ui, rect);
}

pub(super) fn icon_button(
    ui: &mut Ui,
    icon: ButtonIcon,
    tooltip: &str,
    enabled: bool,
    size: Vec2,
    color: Color32,
) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new("").min_size(size));
    let icon_color = if enabled {
        icon_color(icon, color)
    } else {
        ui.visuals().widgets.noninteractive.fg_stroke.color
    };
    let icon_size = size.min_elem().min(16.0);
    let icon_rect = egui::Rect::from_center_size(response.rect.center(), Vec2::splat(icon_size));
    paint_button_icon_at(ui, icon, icon_rect, icon_color);
    response.on_hover_text(tooltip)
}

pub(super) fn icon_for_foundation_button(label: &str) -> Option<ButtonIcon> {
    match label {
        "Open" => Some(ButtonIcon::Open),
        "Import" => Some(ButtonIcon::Import),
        "Clear" => Some(ButtonIcon::Clear),
        "f()" => Some(ButtonIcon::Function),
        _ => None,
    }
}

fn icon_color(icon: ButtonIcon, fallback: Color32) -> Color32 {
    match icon {
        ButtonIcon::Clear => material_delete_text(),
        _ => fallback,
    }
}

fn colorized_icon_svg(icon: ButtonIcon, color: Color32) -> String {
    button_icon_svg(icon).replace("currentColor", &svg_color(color))
}

fn button_icon_uri(ctx: &egui::Context, icon: ButtonIcon, color: Color32) -> String {
    button_icon_uri_for_pixels_per_point(icon, color, ctx.pixels_per_point())
}

fn button_icon_uri_for_pixels_per_point(
    icon: ButtonIcon,
    color: Color32,
    pixels_per_point: f32,
) -> String {
    let dpi = icon_dpi_bucket(pixels_per_point);
    format!(
        "bytes://baboon_button_icons/{:?}-{:02x}{:02x}{:02x}{:02x}-dpi{dpi}.svg",
        icon,
        color.r(),
        color.g(),
        color.b(),
        color.a()
    )
}

fn svg_color(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

fn icon_dpi_bucket(pixels_per_point: f32) -> u32 {
    (pixels_per_point * 100.0).round().max(1.0) as u32
}

#[cfg(test)]
#[path = "tests/button_icons.rs"]
mod tests;
