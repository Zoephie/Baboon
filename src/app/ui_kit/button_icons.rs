//! Embedded button-icon lookup and display-scale selection.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::app) enum ButtonIcon {
    Add,
    About,
    AssetBrowser,
    Browse,
    Cache,
    ChangeAdded,
    ChangeModified,
    ChangeRemoved,
    ChangeSame,
    ChannelAlpha,
    ChannelBlue,
    ChannelGreen,
    ChannelRed,
    ColorPicker,
    Confirm,
    Open,
    Edit,
    Import,
    Export,
    Clear,
    Closed,
    CopyPath,
    Copy,
    Compare,
    Swap,
    Container,
    DefaultTag,
    Doc,
    Down,
    Duplicate,
    Errors,
    Favourite,
    FavouriteFilled,
    FileExplorer,
    Filter,
    Find,
    Search,
    SearchBar,
    Function,
    Garbage,
    Git,
    GitHub,
    Group,
    HaloMods,
    InsertRow,
    Json,
    Loop,
    Markers,
    JumpTo,
    JumpUp,
    Left,
    ListDropdownLeft,
    ListDropdownRight,
    Move,
    Opened,
    Other,
    Pause,
    Pin,
    Play,
    Remove,
    Refresh,
    RenderModel,
    Rename,
    Right,
    Save,
    Settings,
    Sort,
    Stop,
    Tag,
    TableView,
    Bitmap,
    View,
    WindowMode,
    FolderClosed,
    FolderOpen,
}

pub(in crate::app) fn button_icon_svg(icon: ButtonIcon) -> &'static str {
    match icon {
        ButtonIcon::Add => include_root_str!("assets/Button Icons/Add.svg"),
        ButtonIcon::About => include_root_str!("assets/Button Icons/About.svg"),
        ButtonIcon::AssetBrowser => include_root_str!("assets/Button Icons/Asset Browser.svg"),
        ButtonIcon::Browse => include_root_str!("assets/Button Icons/Browse.svg"),
        ButtonIcon::Cache => include_root_str!("assets/Button Icons/Cache.svg"),
        ButtonIcon::ChangeAdded => include_root_str!("assets/Button Icons/change-added.svg"),
        ButtonIcon::ChangeModified => {
            include_root_str!("assets/Button Icons/change-modified.svg")
        }
        ButtonIcon::ChangeRemoved => include_root_str!("assets/Button Icons/change-removed.svg"),
        ButtonIcon::ChangeSame => include_root_str!("assets/Button Icons/change-same.svg"),
        ButtonIcon::ChannelAlpha => include_root_str!("assets/Button Icons/channel-alpha.svg"),
        ButtonIcon::ChannelBlue => include_root_str!("assets/Button Icons/channel-blue.svg"),
        ButtonIcon::ChannelGreen => include_root_str!("assets/Button Icons/channel-green.svg"),
        ButtonIcon::ChannelRed => include_root_str!("assets/Button Icons/channel-red.svg"),
        ButtonIcon::ColorPicker => include_root_str!("assets/Button Icons/Color Picker.svg"),
        ButtonIcon::Confirm => include_root_str!("assets/Button Icons/Confirm.svg"),
        ButtonIcon::Open => include_root_str!("assets/Button Icons/Open.svg"),
        ButtonIcon::Edit => include_root_str!("assets/Button Icons/Edit.svg"),
        ButtonIcon::Import => include_root_str!("assets/Button Icons/Import.svg"),
        ButtonIcon::Export => include_root_str!("assets/Button Icons/Export.svg"),
        ButtonIcon::Clear => include_root_str!("assets/Button Icons/Clear.svg"),
        ButtonIcon::Closed => include_root_str!("assets/Button Icons/Closed.svg"),
        ButtonIcon::CopyPath => include_root_str!("assets/Button Icons/Copy Path.svg"),
        ButtonIcon::Copy => include_root_str!("assets/Button Icons/Copy.svg"),
        ButtonIcon::Compare => include_root_str!("assets/Button Icons/Compare.svg"),
        ButtonIcon::Swap => include_root_str!("assets/Button Icons/Swap.svg"),
        ButtonIcon::Container => include_root_str!("assets/Button Icons/Container.svg"),
        ButtonIcon::DefaultTag => include_root_str!("assets/icons/default_tag.svg"),
        ButtonIcon::Doc => include_root_str!("assets/Button Icons/Doc.svg"),
        ButtonIcon::Down => include_root_str!("assets/Button Icons/Down.svg"),
        ButtonIcon::Duplicate => include_root_str!("assets/Button Icons/Duplicate.svg"),
        ButtonIcon::Errors => include_root_str!("assets/icons/errors.svg"),
        ButtonIcon::Favourite => include_root_str!("assets/Button Icons/Favourite.svg"),
        ButtonIcon::FavouriteFilled => {
            include_root_str!("assets/Button Icons/Favourite Filled.svg")
        }
        ButtonIcon::FileExplorer => include_root_str!("assets/Button Icons/File Explorer.svg"),
        ButtonIcon::Filter => include_root_str!("assets/Button Icons/Filter.svg"),
        ButtonIcon::Find => include_root_str!("assets/Button Icons/Find.svg"),
        ButtonIcon::Search => include_root_str!("assets/Button Icons/search.svg"),
        ButtonIcon::SearchBar => include_root_str!("assets/Button Icons/Search Bar Icon.svg"),
        ButtonIcon::Function => include_root_str!("assets/Button Icons/Function.svg"),
        ButtonIcon::Garbage => include_root_str!("assets/Button Icons/Garbage.svg"),
        ButtonIcon::Git => include_root_str!("assets/Button Icons/Git.svg"),
        ButtonIcon::GitHub => include_root_str!("assets/Button Icons/GitHub.svg"),
        ButtonIcon::Group => include_root_str!("assets/Button Icons/Group.svg"),
        ButtonIcon::HaloMods => include_root_str!("assets/Button Icons/Halo Mods.svg"),
        ButtonIcon::InsertRow => include_root_str!("assets/Button Icons/Insert Row.svg"),
        ButtonIcon::Json => include_root_str!("assets/Button Icons/JSON.svg"),
        ButtonIcon::Loop => include_root_str!("assets/Button Icons/Loop.svg"),
        ButtonIcon::Markers => include_root_str!("assets/icons/markers.svg"),
        ButtonIcon::JumpTo => include_root_str!("assets/Button Icons/Jump To.svg"),
        ButtonIcon::JumpUp => include_root_str!("assets/Button Icons/Jump Up.svg"),
        ButtonIcon::Left => include_root_str!("assets/Button Icons/Left.svg"),
        ButtonIcon::ListDropdownLeft => {
            include_root_str!("assets/Button Icons/List Dropdown - Left.svg")
        }
        ButtonIcon::ListDropdownRight => {
            include_root_str!("assets/Button Icons/List Dropdown - Right.svg")
        }
        ButtonIcon::Move => include_root_str!("assets/Button Icons/Move.svg"),
        ButtonIcon::Opened => include_root_str!("assets/Button Icons/Opened.svg"),
        ButtonIcon::Other => include_root_str!("assets/Button Icons/Other.svg"),
        ButtonIcon::Pause => include_root_str!("assets/Button Icons/Pause.svg"),
        ButtonIcon::Pin => include_root_str!("assets/Button Icons/Pin.svg"),
        ButtonIcon::Play => include_root_str!("assets/Button Icons/Play.svg"),
        ButtonIcon::Remove => include_root_str!("assets/Button Icons/Remove.svg"),
        ButtonIcon::Refresh => include_root_str!("assets/Button Icons/Refresh.svg"),
        ButtonIcon::RenderModel => include_root_str!("assets/icons/render_model.svg"),
        ButtonIcon::Rename => include_root_str!("assets/Button Icons/Rename.svg"),
        ButtonIcon::Right => include_root_str!("assets/Button Icons/Right.svg"),
        ButtonIcon::Save => include_root_str!("assets/Button Icons/Save.svg"),
        ButtonIcon::Settings => include_root_str!("assets/Button Icons/Settings.svg"),
        ButtonIcon::Sort => include_root_str!("assets/Button Icons/Sort.svg"),
        ButtonIcon::Stop => include_root_str!("assets/Button Icons/Stop.svg"),
        ButtonIcon::Tag => include_root_str!("assets/Button Icons/Tag.svg"),
        ButtonIcon::TableView => include_root_str!("assets/Button Icons/Table View.svg"),
        ButtonIcon::View => include_root_str!("assets/Button Icons/View.svg"),
        ButtonIcon::Bitmap => {
            if is_dark_mode() {
                include_root_str!("assets/icons/bitmap.svg")
            } else {
                include_root_str!("assets/icons/bitmap_lightmode.svg")
            }
        }
        ButtonIcon::WindowMode => include_root_str!("assets/Button Icons/Window Mode.svg"),
        ButtonIcon::FolderClosed => include_root_str!("assets/Button Icons/Folder - closed.svg"),
        ButtonIcon::FolderOpen => include_root_str!("assets/Button Icons/Folder - open.svg"),
    }
}

pub(in crate::app) fn paint_button_icon_at(ui: &Ui, icon: ButtonIcon, rect: egui::Rect, color: Color32) {
    paint_icon_tinted(ui, icon, rect, color, Color32::WHITE);
}

/// Paint `icon` recolored to `color` into `rect`, multiplied by `tint`, as
/// `egui::Image::paint_at` would: the rect rounded to pixels and the SVG
/// rasterized at exactly that pixel size.
fn paint_icon_tinted(ui: &Ui, icon: ButtonIcon, rect: egui::Rect, color: Color32, tint: Color32) {
    use egui::emath::GuiRounding as _;
    let pixels_per_point = ui.pixels_per_point();
    let rect = rect.round_to_pixels(pixels_per_point);
    let pixels = (pixels_per_point * rect.size()).round();
    if let Some(texture) = icon_texture(ui.ctx(), icon, color, rect.width(), pixels) {
        let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
        ui.painter().image(texture, rect, uv, tint);
        return;
    }
    egui::Image::from_bytes(button_icon_uri(ui.ctx(), icon, color, rect.width()), colorized_icon_bytes(icon, color))
        .fit_to_exact_size(rect.size())
        .tint(tint)
        .paint_at(ui, rect);
}

/// The textures of recolored icons, by icon, color and pixel size, with the
/// pass each was last painted in. Kept in the egui context whose textures
/// they are.
///
/// The cache owns the handles, so a texture lives as long as its entry.
/// Keeping only the id of a texture egui's loader made let it be freed under
/// us: the loader drops an SVG's sizes that were not asked for in the last
/// pass once a URI has two, and a cached id never asks again. The icon then
/// painted nothing for the rest of the session.
#[derive(Clone, Default)]
struct IconTextures(HashMap<(ButtonIcon, Color32, u32, u32), (egui::TextureHandle, u64)>);

/// Passes an icon texture may go unpainted before a miss sweeps it out,
/// so sizes left behind by a zoom or a monitor change do not pile up.
const ICON_TEXTURE_UNUSED_PASSES: u64 = 600;

/// The texture of `icon` recolored to `color` at `pixels`, once egui has
/// rasterized it. Icons are painted on every frame, a dozen to a block
/// header, and each used to format its URI and have egui's loaders hash it
/// to find the texture; this is a lookup of a small key instead.
fn icon_texture(
    ctx: &egui::Context,
    icon: ButtonIcon,
    color: Color32,
    size: f32,
    pixels: Vec2,
) -> Option<egui::TextureId> {
    let key = (icon, color, pixels.x as u32, pixels.y as u32);
    let id = egui::Id::new("baboon_icon_textures");
    let pass = ctx.cumulative_pass_nr();
    let cached = ctx.data_mut(|data| {
        let (texture, last_used) = data.get_temp_mut_or_default::<IconTextures>(id).0.get_mut(&key)?;
        *last_used = pass;
        Some(texture.id())
    });
    if cached.is_some() {
        return cached;
    }
    let uri = button_icon_uri(ctx, icon, color, size);
    ctx.include_bytes(uri.clone(), colorized_icon_bytes(icon, color));
    let size_hint = egui::load::SizeHint::Size {
        width: key.2,
        height: key.3,
        maintain_aspect_ratio: false,
    };
    let Ok(egui::load::ImagePoll::Ready { image }) = ctx.try_load_image(&uri, size_hint) else {
        return None;
    };
    let texture = ctx.load_texture(uri, image, egui::TextureOptions::default());
    let texture_id = texture.id();
    ctx.data_mut(|data| {
        let textures = &mut data.get_temp_mut_or_default::<IconTextures>(id).0;
        textures.retain(|_, (_, last_used)| pass <= *last_used + ICON_TEXTURE_UNUSED_PASSES);
        textures.insert(key, (texture, pass));
    });
    Some(texture_id)
}

pub(in crate::app) fn button_icon_image(
    ui: &Ui,
    icon: ButtonIcon,
    color: Color32,
    size: f32,
) -> egui::Image<'static> {
    let color = icon_color(icon, color);
    let svg = colorized_icon_bytes(icon, color);
    let uri = button_icon_uri(ui.ctx(), icon, color, size);
    egui::Image::from_bytes(uri, svg)
        .fit_to_exact_size(Vec2::splat(size))
        .tint(Color32::WHITE)
}

pub(in crate::app) fn icon_text_button(
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

pub(in crate::app) fn icon_button(
    ui: &mut Ui,
    icon: ButtonIcon,
    tooltip: &str,
    enabled: bool,
    color: Color32,
) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new("").min_size(ICON_BUTTON_SIZE));
    let icon_rect =
        egui::Rect::from_center_size(response.rect.center(), Vec2::splat(BUTTON_ICON_SIZE));
    // A disabled button fades the whole SVG, accent colors embedded in the
    // asset included, as egui fades a disabled widget. It used to do that by
    // drawing the button in a child `Ui` of its own, which with a dozen of
    // these on every block header was a real share of each frame.
    let tint = if enabled {
        Color32::WHITE
    } else {
        Color32::WHITE.gamma_multiply(ui.visuals().disabled_alpha())
    };
    paint_icon_tinted(ui, icon, icon_rect, icon_color(icon, color), tint);
    response.on_hover_text(tooltip)
}

/// Repaint a native checkbox with its hovered visuals when an adjacent icon
/// or label owns the pointer. Icon checkbox rows are assembled from multiple
/// widgets, while a normal text checkbox has one response spanning the row.
pub(in crate::app) fn paint_checkbox_row_hover(ui: &Ui, checkbox_rect: egui::Rect, checked: bool) {
    if !ui.is_enabled() {
        return;
    }
    let visuals = &ui.visuals().widgets.hovered;
    let (small_icon_rect, big_icon_rect) = ui.spacing().icon_rectangles(checkbox_rect);
    ui.painter().add(egui::epaint::RectShape::new(
        big_icon_rect.expand(visuals.expansion),
        visuals.corner_radius,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Middle,
    ));
    if checked {
        ui.painter().add(egui::Shape::line(
            vec![
                egui::pos2(small_icon_rect.left(), small_icon_rect.center().y),
                egui::pos2(small_icon_rect.center().x, small_icon_rect.bottom()),
                egui::pos2(small_icon_rect.right(), small_icon_rect.top()),
            ],
            visuals.fg_stroke,
        ));
    }
}

pub(in crate::app) fn icon_for_foundation_button(label: &str) -> Option<ButtonIcon> {
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

/// [`colorized_icon_svg`], made once per icon and color. Icons are painted
/// every frame and egui drops the bytes it is handed once it has the texture,
/// so rebuilding the SVG each time (five passes over its text) was all waste:
/// a sixth of a frame with one weapon tag open.
fn colorized_icon_bytes(icon: ButtonIcon, color: Color32) -> egui::load::Bytes {
    thread_local! {
        static COLORIZED: std::cell::RefCell<HashMap<(ButtonIcon, Color32), Arc<[u8]>>> =
            std::cell::RefCell::new(HashMap::new());
    }
    let bytes = COLORIZED.with(|colorized| {
        colorized
            .borrow_mut()
            .entry((icon, color))
            .or_insert_with(|| colorized_icon_svg(icon, color).into_bytes().into())
            .clone()
    });
    egui::load::Bytes::Shared(bytes)
}

fn colorized_icon_svg(icon: ButtonIcon, color: Color32) -> String {
    if matches!(
        icon,
        ButtonIcon::ChangeAdded
            | ButtonIcon::ChangeModified
            | ButtonIcon::ChangeRemoved
            | ButtonIcon::ChangeSame
    ) {
        // These state badges carry their own semantic colors and outline.
        return button_icon_svg(icon).to_owned();
    }
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

pub(in crate::app) fn selectable_icon_text_button(
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

pub(in crate::app) fn selectable_icon_button(
    ui: &mut Ui,
    icon: ButtonIcon,
    tooltip: &str,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    ui.scope(|ui| {
        if selected {
            let selection = ui.visuals().selection;
            let widgets = &mut ui.visuals_mut().widgets;
            widgets.inactive.weak_bg_fill = selection.bg_fill;
            widgets.inactive.bg_stroke = selection.stroke;
            widgets.hovered.weak_bg_fill = selection.bg_fill;
            widgets.active.weak_bg_fill = selection.bg_fill;
        }
        icon_button(ui, icon, tooltip, enabled, text_dark())
    })
    .inner
}

pub(in crate::app) fn selectable_text_button(
    ui: &mut Ui,
    label: impl Into<egui::WidgetText>,
    selected: bool,
) -> egui::Response {
    // Add directly to the wrapping parent. A scope is measured after its
    // contents, too late for the parent to move a whole button to the next row.
    let response = ui.add(
        egui::Button::new(label)
            .selected(selected)
            .wrap_mode(egui::TextWrapMode::Extend)
            .min_size(Vec2::new(0.0, BUTTON_HEIGHT)),
    );
    if selected && response.hovered() {
        let hover_color = if is_dark_mode() {
            Color32::WHITE
        } else {
            Color32::BLACK
        };
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            ui.visuals().widgets.hovered.corner_radius,
            Stroke::new(ui.visuals().selection.stroke.width, hover_color),
            egui::StrokeKind::Middle,
        );
    }
    response
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
    let button_response = ui.add(button);
    let mut positioning_response = button_response.clone();
    let frame_left = Frame::menu(ui.style()).total_margin().left;
    positioning_response.rect.min.x = right_aligned_width
        .map_or(positioning_response.rect.min.x + frame_left, |width| {
            button_response.rect.right() - width + frame_left
        });
    // As `egui::containers::menu::MenuButton` opens its menu, in the menu
    // bar's style, but placed by the shifted response.
    let config = menu_config().style(egui::containers::menu::MenuConfig::find(ui).style);
    let inner = egui::Popup::menu(&positioning_response)
        .close_behavior(config.close_behavior)
        .style(config.style.clone())
        .info(
            egui::UiStackInfo::new(egui::UiKind::Menu)
                .with_tag_value(egui::containers::menu::MenuConfig::MENU_CONFIG_TAG, config),
        )
        .show(add_contents);
    egui::InnerResponse::new(inner.map(|response| response.inner), button_response)
}

/// How every Baboon menu closes: when an item asks it to (`ui.close()`), or
/// on a click outside it, as egui 0.29 closed menus. egui 0.36 closes a menu
/// on any click inside it by default, so ticking a checkbox in one shut it.
/// Submenus inherit it from the menu they open in.
pub(in crate::app) fn menu_config() -> egui::containers::menu::MenuConfig {
    egui::containers::menu::MenuConfig::new()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
}

/// Close the menu `ui` is in, as egui 0.29's `Ui::close_menu` did; outside a
/// menu it does nothing. egui 0.36's `Ui::close` instead closes the nearest
/// closable container, which outside a menu is a collapsing header or the
/// window the code sits in.
pub(in crate::app) fn close_menu(ui: &Ui) {
    if egui::containers::menu::is_in_menu(ui) {
        ui.close_kind(egui::UiKind::Menu);
    }
}

/// `response`'s right-click menu, closing as [`menu_config`] describes.
pub(in crate::app) fn context_menu(
    response: &egui::Response,
    add_contents: impl FnOnce(&mut Ui),
) -> Option<egui::InnerResponse<()>> {
    egui::Popup::context_menu(response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(add_contents)
}

pub(in crate::app) fn aligned_menu_button<R>(
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

pub(in crate::app) fn right_aligned_menu_button<R>(
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
pub(in crate::app) fn icon_menu_button<R>(
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

pub(in crate::app) fn icon_text_dropdown_button<R>(
    ui: &mut Ui,
    icon: ButtonIcon,
    label: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let (response, inner) = egui::containers::menu::MenuButton::new(format!("     {label}      "))
        .config(menu_config())
        .ui(ui, add_contents);
    let menu = egui::InnerResponse::new(inner.map(|inner| inner.inner), response);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(
            menu.response.rect.left() + 13.0,
            menu.response.rect.center().y,
        ),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    let chevron_rect = egui::Rect::from_center_size(
        egui::pos2(
            menu.response.rect.right() - 12.0,
            menu.response.rect.center().y,
        ),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    paint_button_icon_at(ui, icon, icon_rect, text_dark());
    paint_button_icon_at(ui, ButtonIcon::Down, chevron_rect, text_dark());
    menu
}

/// An icon-and-text dropdown whose popup is anchored to the trigger's right
/// edge. Preview headers use this when their actions sit against the pane edge.
pub(in crate::app) fn right_aligned_icon_text_dropdown_button<R>(
    ui: &mut Ui,
    icon: ButtonIcon,
    label: &str,
    popup_width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    // `popup_width` describes the content area set by the caller. Include the
    // menu frame margins when positioning so the visible outer edge, not just
    // the content edge, lands on the trigger's right edge.
    let menu_margin = Frame::menu(ui.style()).total_margin();
    let popup_outer_width = popup_width + menu_margin.left + menu_margin.right;
    let menu = aligned_menu_custom_button(
        ui,
        egui::Button::new(format!("     {label}      ")),
        Some(popup_outer_width),
        add_contents,
    );
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(
            menu.response.rect.left() + 13.0,
            menu.response.rect.center().y,
        ),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    let chevron_rect = egui::Rect::from_center_size(
        egui::pos2(
            menu.response.rect.right() - 12.0,
            menu.response.rect.center().y,
        ),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    paint_button_icon_at(ui, icon, icon_rect, text_dark());
    paint_button_icon_at(ui, ButtonIcon::Down, chevron_rect, text_dark());
    menu
}

/// Header action menus sit against the right edge of their pane. Align their
/// popup's outer right edge to the trigger instead of using the usual left
/// edge anchor.
pub(in crate::app) fn right_aligned_icon_menu_button<R>(
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

pub(in crate::app) fn paint_submenu_icon(ui: &Ui, response: &egui::Response, opens_left: bool) {
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

pub(in crate::app) fn right_opening_menu_button<R>(
    ui: &mut Ui,
    label: impl Into<egui::WidgetText>,
    popup_width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    nested_menu_button(ui, label.into().text(), popup_width, false, add_contents)
}

pub(in crate::app) fn left_opening_menu_button<R>(
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
    let text = egui::RichText::new(label).color(Color32::TRANSPARENT);
    let contents = |ui: &mut Ui| {
        ui.set_min_width(popup_width);
        add_contents(ui)
    };
    // Inside a menu this is a submenu, which takes that menu's config. On its
    // own it is a menu of its own and needs Baboon's: with egui 0.36's
    // default it closed on any click inside, its scrollbar included, so only
    // one item could be picked per opening.
    let menu = if egui::containers::menu::is_in_menu(ui) {
        ui.menu_button(text, contents)
    } else {
        let (response, inner) = egui::containers::menu::MenuButton::new(text)
            .config(menu_config())
            .ui(ui, contents);
        egui::InnerResponse::new(inner.map(|inner| inner.inner), response)
    };
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

#[cfg(test)]
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
mod tests {
    //! Unit tests for embedded button icons.
    //! It owns test-only characterization and does not participate in runtime application behavior.

    use super::*;

    #[test]
    fn button_icon_lookup_uses_expected_assets() {
        let icons = [
            ButtonIcon::Add,
            ButtonIcon::About,
            ButtonIcon::AssetBrowser,
            ButtonIcon::Browse,
            ButtonIcon::Cache,
            ButtonIcon::ChannelAlpha,
            ButtonIcon::ChannelBlue,
            ButtonIcon::ChannelGreen,
            ButtonIcon::ChannelRed,
            ButtonIcon::ColorPicker,
            ButtonIcon::Confirm,
            ButtonIcon::Clear,
            ButtonIcon::Closed,
            ButtonIcon::CopyPath,
            ButtonIcon::Copy,
            ButtonIcon::Compare,
            ButtonIcon::Swap,
            ButtonIcon::Container,
            ButtonIcon::Doc,
            ButtonIcon::Down,
            ButtonIcon::Duplicate,
            ButtonIcon::Errors,
            ButtonIcon::Export,
            ButtonIcon::Favourite,
            ButtonIcon::FileExplorer,
            ButtonIcon::Filter,
            ButtonIcon::Find,
            ButtonIcon::FolderClosed,
            ButtonIcon::FolderOpen,
            ButtonIcon::Function,
            ButtonIcon::Garbage,
            ButtonIcon::GitHub,
            ButtonIcon::Group,
            ButtonIcon::HaloMods,
            ButtonIcon::Import,
            ButtonIcon::InsertRow,
            ButtonIcon::Json,
            ButtonIcon::JumpTo,
            ButtonIcon::JumpUp,
            ButtonIcon::Left,
            ButtonIcon::Loop,
            ButtonIcon::Markers,
            ButtonIcon::ListDropdownLeft,
            ButtonIcon::ListDropdownRight,
            ButtonIcon::Move,
            ButtonIcon::Open,
            ButtonIcon::Edit,
            ButtonIcon::Opened,
            ButtonIcon::Other,
            ButtonIcon::Pause,
            ButtonIcon::Play,
            ButtonIcon::Remove,
            ButtonIcon::Refresh,
            ButtonIcon::Rename,
            ButtonIcon::Right,
            ButtonIcon::Save,
            ButtonIcon::SearchBar,
            ButtonIcon::Search,
            ButtonIcon::Settings,
            ButtonIcon::Sort,
            ButtonIcon::Stop,
            ButtonIcon::Tag,
            ButtonIcon::TableView,
            ButtonIcon::Pin,
            ButtonIcon::View,
            ButtonIcon::WindowMode,
        ];
        for icon in icons {
            assert!(button_icon_svg(icon).contains("<svg"), "missing {icon:?}");
        }
    }

    #[test]
    fn colorized_icon_replaces_current_color() {
        let svg = colorized_icon_svg(ButtonIcon::Open, Color32::from_rgb(1, 2, 3));
        assert!(svg.contains("#010203"));
        assert!(!svg.contains("currentColor"));
    }

    /// Painting an icon again hands egui the bytes made the first time, not
    /// a fresh recoloring: icons are painted on every frame.
    #[test]
    fn a_colorized_icon_is_made_once_per_color() {
        let shared = |color| match colorized_icon_bytes(ButtonIcon::Open, color) {
            egui::load::Bytes::Shared(bytes) => bytes,
            egui::load::Bytes::Static(_) => panic!("a recolored icon is not static"),
        };
        let first = shared(Color32::from_rgb(1, 2, 3));
        assert_eq!(
            &*first,
            colorized_icon_svg(ButtonIcon::Open, Color32::from_rgb(1, 2, 3)).as_bytes()
        );
        assert!(Arc::ptr_eq(&first, &shared(Color32::from_rgb(1, 2, 3))));
        let other = shared(Color32::from_rgb(4, 5, 6));
        assert!(!Arc::ptr_eq(&first, &other));
        assert!(std::str::from_utf8(&other).unwrap().contains("#040506"));
    }

    #[test]
    fn submenu_directions_use_distinct_assets() {
        assert_ne!(
            button_icon_svg(ButtonIcon::ListDropdownLeft),
            button_icon_svg(ButtonIcon::ListDropdownRight)
        );
    }

    #[test]
    fn button_icon_uri_changes_with_pixels_per_point() {
        let low = button_icon_uri_for_pixels_per_point(ButtonIcon::Open, Color32::WHITE, 1.0);
        let high = button_icon_uri_for_pixels_per_point(ButtonIcon::Open, Color32::WHITE, 2.0);
        assert_ne!(low, high);
        assert!(low.contains("dpi100"));
        assert!(high.contains("dpi200"));
    }

    #[test]
    fn button_icon_uri_changes_with_rendered_size() {
        let small = button_icon_uri_for_pixels_per_point_and_size(
            ButtonIcon::FolderOpen,
            Color32::WHITE,
            1.0,
            16.0,
        );
        let large = button_icon_uri_for_pixels_per_point_and_size(
            ButtonIcon::FolderOpen,
            Color32::WHITE,
            1.0,
            32.0,
        );
        assert_ne!(small, large);
        assert!(large.contains("32px"));
    }

    /// A menu opened from a button of its own stays open when an item in it
    /// is clicked, as every Baboon menu does, so several can be picked in one
    /// opening. With egui 0.36's default it closed on any click inside.
    #[test]
    fn a_standalone_menu_stays_open_after_a_pick() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let mut picks = 0;
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>, picks: &mut i32| -> Vec<(String, egui::Rect)> {
            time += 0.1;
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        right_opening_menu_button(ui, "Add", 200.0, |ui| {
                            for item in ["first item", "second item"] {
                                if ui.button(item).clicked() {
                                    *picks += 1;
                                }
                            }
                        });
                    });
                },
            );
            output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2()))),
                    _ => None,
                })
                .collect()
        };
        fn click(
            frame: &mut impl FnMut(Vec<egui::Event>, &mut i32) -> Vec<(String, egui::Rect)>,
            at: egui::Pos2,
            picks: &mut i32,
        ) -> Vec<(String, egui::Rect)> {
            for step in 1..=3 {
                frame(vec![egui::Event::PointerMoved(at - egui::vec2(0.0, 3.0 - step as f32))], picks);
            }
            for pressed in [true, false] {
                frame(
                    vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }],
                    picks,
                );
            }
            frame(Vec::new(), picks)
        }
        let find = |shown: &[(String, egui::Rect)], text: &str| shown.iter().find(|(shown, _)| shown == text).map(|(_, rect)| rect.center());
        frame(Vec::new(), &mut picks);
        let shown = frame(Vec::new(), &mut picks);
        let button = find(&shown, "Add").expect("the menu button");
        let shown = click(&mut frame, button, &mut picks);
        let first = find(&shown, "first item").expect("the menu opened");
        let shown = click(&mut frame, first, &mut picks);
        assert_eq!(picks, 1);
        assert!(find(&shown, "second item").is_some(), "the menu is still open");
    }

    /// A cached icon keeps its texture after a text button showing the same
    /// icon comes and goes, as the shader editor's Clear button does when a
    /// color picker's "Cancel" closes. The button sizes its image at a second
    /// size hint of the same URI, and once it is gone egui's loader frees
    /// every size of that URI nobody asked for in the last pass; with only an
    /// id cached, the icon painted nothing from then on.
    #[test]
    fn a_cached_icon_survives_a_text_button_with_the_same_icon_closing() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut painted = Vec::new();
        let mut freed = Vec::new();
        for pass in 0..6 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 300.0))),
                    time: Some(pass as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), Vec2::splat(BUTTON_ICON_SIZE));
                        paint_button_icon_at(ui, ButtonIcon::Clear, rect, material_delete_text());
                        if (1..3).contains(&pass) {
                            ui.add_space(40.0);
                            icon_text_button(ui, ButtonIcon::Clear, "Cancel", true);
                        }
                    });
                },
            );
            freed.extend(output.textures_delta.free.iter().copied());
            output.textures_delta.clear();
            let icon_textures = ctx.data(|data| {
                data.get_temp::<IconTextures>(egui::Id::new("baboon_icon_textures")).unwrap_or_default()
            });
            painted.extend(icon_textures.0.values().map(|(texture, _)| texture.id()));
        }
        painted.dedup();
        assert_eq!(painted.len(), 1, "one texture for the one cached icon: {painted:?}");
        assert!(!freed.contains(&painted[0]), "the cached icon's texture was freed: {freed:?}");
        assert!(ctx.tex_manager().read().meta(painted[0]).is_some());
    }
}
