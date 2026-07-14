//! Embedded button-icon lookup and display-scale selection.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ButtonIcon {
    Add,
    Browse,
    Open,
    Import,
    Export,
    Clear,
    Closed,
    CopyPath,
    Copy,
    Doc,
    Duplicate,
    Favourite,
    FileExplorer,
    Filter,
    Find,
    Search,
    SearchBar,
    Function,
    Garbage,
    Group,
    InsertRow,
    Json,
    JumpTo,
    JumpUp,
    Left,
    Move,
    Opened,
    Other,
    Remove,
    Rename,
    Right,
    Save,
    Settings,
    Sort,
    WindowMode,
    FolderClosed,
    FolderOpen,
}

pub(super) fn button_icon_svg(icon: ButtonIcon) -> &'static str {
    match icon {
        ButtonIcon::Add => include_str!("../../assets/Button Icons/Add.svg"),
        ButtonIcon::Browse => include_str!("../../assets/Button Icons/Browse.svg"),
        ButtonIcon::Open => include_str!("../../assets/Button Icons/Open.svg"),
        ButtonIcon::Import => include_str!("../../assets/Button Icons/Import.svg"),
        ButtonIcon::Export => include_str!("../../assets/Button Icons/Export.svg"),
        ButtonIcon::Clear => include_str!("../../assets/Button Icons/Clear.svg"),
        ButtonIcon::Closed => include_str!("../../assets/Button Icons/Closed.svg"),
        ButtonIcon::CopyPath => include_str!("../../assets/Button Icons/Copy Path.svg"),
        ButtonIcon::Copy => include_str!("../../assets/Button Icons/Copy.svg"),
        ButtonIcon::Doc => include_str!("../../assets/Button Icons/Doc.svg"),
        ButtonIcon::Duplicate => include_str!("../../assets/Button Icons/Duplicate.svg"),
        ButtonIcon::Favourite => include_str!("../../assets/Button Icons/Favourite.svg"),
        ButtonIcon::FileExplorer => include_str!("../../assets/Button Icons/File Explorer.svg"),
        ButtonIcon::Filter => include_str!("../../assets/Button Icons/Filter.svg"),
        ButtonIcon::Find => include_str!("../../assets/Button Icons/Find.svg"),
        ButtonIcon::Search => include_str!("../../assets/Button Icons/search.svg"),
        ButtonIcon::SearchBar => include_str!("../../assets/Button Icons/Search Bar Icon.svg"),
        ButtonIcon::Function => include_str!("../../assets/Button Icons/Function.svg"),
        ButtonIcon::Garbage => include_str!("../../assets/Button Icons/Garbage.svg"),
        ButtonIcon::Group => include_str!("../../assets/Button Icons/Group.svg"),
        ButtonIcon::InsertRow => include_str!("../../assets/Button Icons/Insert Row.svg"),
        ButtonIcon::Json => include_str!("../../assets/Button Icons/JSON.svg"),
        ButtonIcon::JumpTo => include_str!("../../assets/Button Icons/Jump To.svg"),
        ButtonIcon::JumpUp => include_str!("../../assets/Button Icons/Jump Up.svg"),
        ButtonIcon::Left => include_str!("../../assets/Button Icons/Left.svg"),
        ButtonIcon::Move => include_str!("../../assets/Button Icons/Move.svg"),
        ButtonIcon::Opened => include_str!("../../assets/Button Icons/Opened.svg"),
        ButtonIcon::Other => include_str!("../../assets/Button Icons/Other.svg"),
        ButtonIcon::Remove => include_str!("../../assets/Button Icons/Remove.svg"),
        ButtonIcon::Rename => include_str!("../../assets/Button Icons/Rename.svg"),
        ButtonIcon::Right => include_str!("../../assets/Button Icons/Right.svg"),
        ButtonIcon::Save => include_str!("../../assets/Button Icons/Save.svg"),
        ButtonIcon::Settings => include_str!("../../assets/Button Icons/Settings.svg"),
        ButtonIcon::Sort => include_str!("../../assets/Button Icons/Sort.svg"),
        ButtonIcon::WindowMode => include_str!("../../assets/Button Icons/Window Mode.svg"),
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

pub(super) fn button_icon_image(
    ui: &Ui,
    icon: ButtonIcon,
    color: Color32,
    size: f32,
) -> egui::Image<'static> {
    let color = icon_color(icon, color);
    let svg = colorized_icon_svg(icon, color);
    let uri = button_icon_uri(ui.ctx(), icon, color);
    egui::Image::from_bytes(uri, svg.into_bytes())
        .fit_to_exact_size(Vec2::splat(size))
        .tint(Color32::WHITE)
}

pub(super) fn icon_text_button(
    ui: &mut Ui,
    icon: ButtonIcon,
    label: impl Into<egui::WidgetText>,
    enabled: bool,
) -> egui::Response {
    let image = button_icon_image(ui, icon, text_dark(), 16.0);
    ui.add_enabled(enabled, egui::Button::image_and_text(image, label))
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
        "Add" => Some(ButtonIcon::Add),
        "..." => Some(ButtonIcon::Browse),
        "Open" => Some(ButtonIcon::Open),
        "Import" => Some(ButtonIcon::Import),
        "Clear" => Some(ButtonIcon::Clear),
        "f()" => Some(ButtonIcon::Function),
        "Insert" => Some(ButtonIcon::InsertRow),
        "Duplicate" => Some(ButtonIcon::Duplicate),
        "Delete" => Some(ButtonIcon::Remove),
        "Delete all" => Some(ButtonIcon::Garbage),
        _ => None,
    }
}

fn icon_color(icon: ButtonIcon, fallback: Color32) -> Color32 {
    match icon {
        ButtonIcon::Clear | ButtonIcon::Garbage | ButtonIcon::Remove => material_delete_text(),
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
