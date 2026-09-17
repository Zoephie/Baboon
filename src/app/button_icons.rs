//! Embedded button-icon lookup and display-scale selection.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ButtonIcon {
    Add,
    About,
    Browse,
    Cache,
    Open,
    Edit,
    Import,
    Export,
    Clear,
    Closed,
    CopyPath,
    Copy,
    Container,
    DefaultTag,
    Doc,
    Duplicate,
    Favourite,
    FavouriteFilled,
    FileExplorer,
    Filter,
    Find,
    Search,
    SearchBar,
    Function,
    Garbage,
    GitHub,
    Group,
    HaloMods,
    InsertRow,
    Json,
    JumpTo,
    JumpUp,
    Left,
    ListDropdownLeft,
    ListDropdownRight,
    Move,
    Opened,
    Other,
    Remove,
    RenderModel,
    Rename,
    Right,
    Save,
    Settings,
    Sort,
    Tag,
    Bitmap,
    WindowMode,
    FolderClosed,
    FolderOpen,
}

pub(super) fn button_icon_svg(icon: ButtonIcon) -> &'static str {
    match icon {
        ButtonIcon::Add => include_str!("../../assets/Button Icons/Add.svg"),
        ButtonIcon::About => include_str!("../../assets/Button Icons/About.svg"),
        ButtonIcon::Browse => include_str!("../../assets/Button Icons/Browse.svg"),
        ButtonIcon::Cache => include_str!("../../assets/Button Icons/Cache.svg"),
        ButtonIcon::Open => include_str!("../../assets/Button Icons/Open.svg"),
        ButtonIcon::Edit => include_str!("../../assets/Button Icons/Edit.svg"),
        ButtonIcon::Import => include_str!("../../assets/Button Icons/Import.svg"),
        ButtonIcon::Export => include_str!("../../assets/Button Icons/Export.svg"),
        ButtonIcon::Clear => include_str!("../../assets/Button Icons/Clear.svg"),
        ButtonIcon::Closed => include_str!("../../assets/Button Icons/Closed.svg"),
        ButtonIcon::CopyPath => include_str!("../../assets/Button Icons/Copy Path.svg"),
        ButtonIcon::Copy => include_str!("../../assets/Button Icons/Copy.svg"),
        ButtonIcon::Container => include_str!("../../assets/Button Icons/Container.svg"),
        ButtonIcon::DefaultTag => include_str!("../../assets/icons/default_tag.svg"),
        ButtonIcon::Doc => include_str!("../../assets/Button Icons/Doc.svg"),
        ButtonIcon::Duplicate => include_str!("../../assets/Button Icons/Duplicate.svg"),
        ButtonIcon::Favourite => include_str!("../../assets/Button Icons/Favourite.svg"),
        ButtonIcon::FavouriteFilled => {
            include_str!("../../assets/Button Icons/Favourite Filled.svg")
        }
        ButtonIcon::FileExplorer => include_str!("../../assets/Button Icons/File Explorer.svg"),
        ButtonIcon::Filter => include_str!("../../assets/Button Icons/Filter.svg"),
        ButtonIcon::Find => include_str!("../../assets/Button Icons/Find.svg"),
        ButtonIcon::Search => include_str!("../../assets/Button Icons/search.svg"),
        ButtonIcon::SearchBar => include_str!("../../assets/Button Icons/Search Bar Icon.svg"),
        ButtonIcon::Function => include_str!("../../assets/Button Icons/Function.svg"),
        ButtonIcon::Garbage => include_str!("../../assets/Button Icons/Garbage.svg"),
        ButtonIcon::GitHub => include_str!("../../assets/Button Icons/GitHub.svg"),
        ButtonIcon::Group => include_str!("../../assets/Button Icons/Group.svg"),
        ButtonIcon::HaloMods => include_str!("../../assets/Button Icons/Halo Mods.svg"),
        ButtonIcon::InsertRow => include_str!("../../assets/Button Icons/Insert Row.svg"),
        ButtonIcon::Json => include_str!("../../assets/Button Icons/JSON.svg"),
        ButtonIcon::JumpTo => include_str!("../../assets/Button Icons/Jump To.svg"),
        ButtonIcon::JumpUp => include_str!("../../assets/Button Icons/Jump Up.svg"),
        ButtonIcon::Left => include_str!("../../assets/Button Icons/Left.svg"),
        ButtonIcon::ListDropdownLeft => {
            include_str!("../../assets/Button Icons/List Dropdown - Left.svg")
        }
        ButtonIcon::ListDropdownRight => {
            include_str!("../../assets/Button Icons/List Dropdown - Right.svg")
        }
        ButtonIcon::Move => include_str!("../../assets/Button Icons/Move.svg"),
        ButtonIcon::Opened => include_str!("../../assets/Button Icons/Opened.svg"),
        ButtonIcon::Other => include_str!("../../assets/Button Icons/Other.svg"),
        ButtonIcon::Remove => include_str!("../../assets/Button Icons/Remove.svg"),
        ButtonIcon::RenderModel => include_str!("../../assets/icons/render_model.svg"),
        ButtonIcon::Rename => include_str!("../../assets/Button Icons/Rename.svg"),
        ButtonIcon::Right => include_str!("../../assets/Button Icons/Right.svg"),
        ButtonIcon::Save => include_str!("../../assets/Button Icons/Save.svg"),
        ButtonIcon::Settings => include_str!("../../assets/Button Icons/Settings.svg"),
        ButtonIcon::Sort => include_str!("../../assets/Button Icons/Sort.svg"),
        ButtonIcon::Tag => include_str!("../../assets/Button Icons/Tag.svg"),
        ButtonIcon::Bitmap => {
            if is_dark_mode() {
                include_str!("../../assets/icons/bitmap.svg")
            } else {
                include_str!("../../assets/icons/bitmap_lightmode.svg")
            }
        }
        ButtonIcon::WindowMode => include_str!("../../assets/Button Icons/Window Mode.svg"),
        ButtonIcon::FolderClosed => include_str!("../../assets/Button Icons/Folder - closed.svg"),
        ButtonIcon::FolderOpen => include_str!("../../assets/Button Icons/Folder - open.svg"),
    }
}

pub(super) fn paint_button_icon_at(ui: &Ui, icon: ButtonIcon, rect: egui::Rect, color: Color32) {
    let svg = colorized_icon_svg(icon, color);
    let uri = button_icon_uri(ui.ctx(), icon, color, rect.width());
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
    let uri = button_icon_uri(ui.ctx(), icon, color, size);
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
    let image = button_icon_image(ui, icon, text_dark(), BUTTON_ICON_SIZE);
    ui.add_enabled(
        enabled,
        egui::Button::image_and_text(image, label).min_size(Vec2::new(0.0, BUTTON_HEIGHT)),
    )
}

pub(super) fn icon_button(
    ui: &mut Ui,
    icon: ButtonIcon,
    tooltip: &str,
    enabled: bool,
    color: Color32,
) -> egui::Response {
    let size = ICON_BUTTON_SIZE;
    let response = ui.add_enabled(enabled, egui::Button::new("").min_size(size));
    let icon_color = if enabled {
        icon_color(icon, color)
    } else {
        ui.visuals().widgets.noninteractive.fg_stroke.color
    };
    let icon_size = BUTTON_ICON_SIZE;
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
    let color = svg_color(color);
    button_icon_svg(icon)
        .replace("currentColor", &color)
        .replace("#A3C0C2", &color)
        .replace("#5CCC33", &color)
        .replace("white", &color)
        .replace("black", &color)
}

fn button_icon_uri(ctx: &egui::Context, icon: ButtonIcon, color: Color32, size: f32) -> String {
    button_icon_uri_for_pixels_per_point_and_size(icon, color, ctx.pixels_per_point(), size)
}

pub(super) fn selectable_icon_text_button(
    ui: &mut Ui,
    icon: ButtonIcon,
    label: impl Into<egui::WidgetText>,
    selected: bool,
) -> egui::Response {
    ui.scope(|ui| {
        if selected {
            let selection = ui.visuals().selection;
            let hover_color = if is_dark_mode() {
                Color32::WHITE
            } else {
                Color32::BLACK
            };
            let widgets = &mut ui.visuals_mut().widgets;

            // egui's built-in `Button::selected` paints with square corners.
            // Apply the selected palette through the normal button states so
            // toggles retain their usual rounding and hover expansion.
            widgets.inactive.weak_bg_fill = selection.bg_fill;
            widgets.inactive.bg_stroke = selection.stroke;
            widgets.hovered.weak_bg_fill = selection.bg_fill;
            widgets.hovered.bg_stroke = Stroke::new(selection.stroke.width, hover_color);
            widgets.hovered.expansion = widgets.hovered.expansion.max(1.0);
            widgets.active.weak_bg_fill = selection.bg_fill;
            widgets.active.bg_stroke = Stroke::new(selection.stroke.width, hover_color);
            widgets.active.expansion = widgets.active.expansion.max(1.0);
        }

        let image = button_icon_image(ui, icon, text_dark(), BUTTON_ICON_SIZE);
        ui.add(egui::Button::image_and_text(image, label).min_size(Vec2::new(0.0, BUTTON_HEIGHT)))
    })
    .inner
}

/// egui offsets a popup frame to align its first row with the trigger. Shift
/// only the positioning response by that inset so the frame edge aligns with
/// the real button while retaining the frame's visible content padding.
fn aligned_menu_custom_button<R>(
    ui: &mut Ui,
    button: egui::Button<'_>,
    right_aligned_width: Option<f32>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let bar_id = ui.id();
    let mut bar_state = egui::menu::BarState::load(ui.ctx(), bar_id);
    let button_response = ui.add(button);
    let mut positioning_response = button_response.clone();
    let frame_left = Frame::menu(&ui.ctx().style()).total_margin().left;
    positioning_response.rect.min.x = right_aligned_width
        .map_or(positioning_response.rect.min.x + frame_left, |width| {
            button_response.rect.right() - width + frame_left
        });
    let inner = bar_state.bar_menu(&positioning_response, add_contents);
    bar_state.store(ui.ctx(), bar_id);
    egui::InnerResponse::new(inner.map(|response| response.inner), button_response)
}

pub(super) fn aligned_menu_button<R>(
    ui: &mut Ui,
    title: impl Into<egui::WidgetText>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let previous_padding = ui.spacing().button_padding.x;
    ui.spacing_mut().button_padding.x = 8.0;
    let menu = aligned_menu_custom_button(ui, egui::Button::new(title), None, add_contents);
    ui.spacing_mut().button_padding.x = previous_padding;
    menu
}

pub(super) fn right_aligned_menu_button<R>(
    ui: &mut Ui,
    title: impl Into<egui::WidgetText>,
    popup_width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let previous_padding = ui.spacing().button_padding.x;
    ui.spacing_mut().button_padding.x = 8.0;
    let menu = aligned_menu_custom_button(
        ui,
        egui::Button::new(title),
        Some(popup_width),
        add_contents,
    );
    ui.spacing_mut().button_padding.x = previous_padding;
    menu
}

/// A menu trigger with the same fixed square geometry as the app's other
/// icon-only buttons. `Ui::menu_image_button` derives its size from theme
/// padding, which allowed these controls to drift away from 24×24.
pub(super) fn icon_menu_button<R>(
    ui: &mut Ui,
    icon: ButtonIcon,
    tooltip: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::Response {
    let menu = aligned_menu_custom_button(
        ui,
        egui::Button::new("").min_size(ICON_BUTTON_SIZE),
        None,
        add_contents,
    );
    let icon_rect =
        egui::Rect::from_center_size(menu.response.rect.center(), Vec2::splat(BUTTON_ICON_SIZE));
    paint_button_icon_at(ui, icon, icon_rect, text_dark());
    menu.response.on_hover_text(tooltip)
}

/// Header action menus sit against the right edge of their pane. Align their
/// popup's outer right edge to the trigger instead of using the usual left
/// edge anchor.
pub(super) fn right_aligned_icon_menu_button<R>(
    ui: &mut Ui,
    icon: ButtonIcon,
    tooltip: &str,
    popup_width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::Response {
    let menu = aligned_menu_custom_button(
        ui,
        egui::Button::new("").min_size(ICON_BUTTON_SIZE),
        Some(popup_width),
        add_contents,
    );
    let icon_rect =
        egui::Rect::from_center_size(menu.response.rect.center(), Vec2::splat(BUTTON_ICON_SIZE));
    paint_button_icon_at(ui, icon, icon_rect, text_dark());
    menu.response.on_hover_text(tooltip)
}

pub(super) fn paint_submenu_icon(ui: &Ui, response: &egui::Response, opens_left: bool) {
    let color = if ui.is_enabled() {
        text_dark()
    } else {
        ui.visuals().widgets.noninteractive.fg_stroke.color
    };
    let rect = egui::Rect::from_center_size(
        egui::pos2(response.rect.right() - 12.0, response.rect.center().y),
        Vec2::splat(16.0),
    );
    let icon = if opens_left {
        ButtonIcon::ListDropdownLeft
    } else {
        ButtonIcon::ListDropdownRight
    };
    let image = button_icon_image(ui, icon, color, 16.0);
    image.paint_at(ui, rect);
}

pub(super) fn right_opening_menu_button<R>(
    ui: &mut Ui,
    label: impl Into<egui::WidgetText>,
    popup_width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    nested_menu_button(ui, label.into().text(), popup_width, false, add_contents)
}

pub(super) fn left_opening_menu_button<R>(
    ui: &mut Ui,
    label: &str,
    popup_width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    nested_menu_button(ui, label, popup_width, true, add_contents).inner
}

/// egui's own nested menu keeps child clicks in the parent menu hierarchy.
/// The earlier detached Area looked right but lost clicks when the parent
/// treated them as outside clicks.
fn nested_menu_button<R>(
    ui: &mut Ui,
    label: &str,
    popup_width: f32,
    opens_left: bool,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let original_visuals = ui.visuals().clone();
    let original_menu_spacing = ui.spacing().menu_spacing;
    if opens_left {
        let margin = Frame::menu(ui.style()).total_margin();
        let parent_width = ui.max_rect().width() + margin.left + margin.right;
        let child_width = popup_width + margin.left + margin.right;
        ui.spacing_mut().menu_spacing = -(parent_width + child_width + original_menu_spacing);
    }
    {
        let visuals = ui.visuals_mut();
        visuals.override_text_color = Some(Color32::TRANSPARENT);
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
            &mut visuals.widgets.noninteractive,
        ] {
            widget.fg_stroke.color = Color32::TRANSPARENT;
        }
    }
    let menu = ui.menu_button(
        egui::RichText::new(label).color(Color32::TRANSPARENT),
        |ui| {
            ui.set_min_width(popup_width);
            add_contents(ui)
        },
    );
    *ui.visuals_mut() = original_visuals;
    ui.spacing_mut().menu_spacing = original_menu_spacing;
    let color = if ui.is_enabled() {
        text_dark()
    } else {
        ui.visuals().widgets.noninteractive.fg_stroke.color
    };
    ui.painter().text(
        egui::pos2(
            menu.response.rect.left() + 8.0,
            menu.response.rect.center().y,
        ),
        Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        color,
    );
    paint_submenu_icon(ui, &menu.response, opens_left);
    menu
}

fn button_icon_uri_for_pixels_per_point(
    icon: ButtonIcon,
    color: Color32,
    pixels_per_point: f32,
) -> String {
    button_icon_uri_for_pixels_per_point_and_size(icon, color, pixels_per_point, BUTTON_ICON_SIZE)
}

fn button_icon_uri_for_pixels_per_point_and_size(
    icon: ButtonIcon,
    color: Color32,
    pixels_per_point: f32,
    size: f32,
) -> String {
    let dpi = icon_dpi_bucket(pixels_per_point);
    let pixels = (size * pixels_per_point).round().max(1.0) as u32;
    format!(
        "bytes://baboon_button_icons/{:?}-{:02x}{:02x}{:02x}{:02x}-dpi{dpi}-{pixels}px.svg",
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
