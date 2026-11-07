//! Baboon colors, typography, spacing, and reusable egui style helpers.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Match the content inset of neighboring menu buttons without shifting tables.
pub(in crate::app) fn menu_control_margin(ui: &Ui) -> egui::Margin {
    let inset = if egui::containers::menu::is_in_menu(ui) {
        ui.spacing().button_padding.x.round() as i8
    } else {
        0
    };
    egui::Margin::symmetric(inset, 0)
}

/// Shared dimensions for ordinary, single-line application buttons.
pub(in crate::app) const BUTTON_HEIGHT: f32 = 24.0;
pub(in crate::app) const BUTTON_ICON_SIZE: f32 = 16.0;
pub(in crate::app) const ICON_BUTTON_SIZE: Vec2 = Vec2::new(24.0, 24.0);
/// Horizontal inset for text-only buttons. Icon-only buttons use their fixed
/// 24×24 frame, leaving four points around the 16×16 glyph.
pub(in crate::app) const BUTTON_TEXT_PADDING_X: f32 = 8.0;

/// Shared full-width menu row styling for action menus and value pickers.
pub(in crate::app) fn style_list_menu(ui: &mut Ui) {
    ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
    ui.spacing_mut().button_padding = Vec2::new(8.0, 4.0);
    ui.spacing_mut().interact_size.y = 28.0;
    ui.visuals_mut().override_text_color = Some(text_dark());
    ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
    ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    ui.visuals_mut().widgets.hovered.bg_fill = context_menu_hover();
    ui.visuals_mut().widgets.hovered.weak_bg_fill = context_menu_hover();
    ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
    ui.visuals_mut().widgets.active.bg_fill = context_menu_hover();
    ui.visuals_mut().widgets.active.weak_bg_fill = context_menu_hover();
    ui.visuals_mut().widgets.active.bg_stroke = Stroke::NONE;
}
/// Visual separation between an icon and its label: two points of spacing
/// plus the label's four-point inner inset from the design specification.
pub(in crate::app) const BUTTON_ICON_TEXT_GAP: f32 = 6.0;
pub(in crate::app) const ALPHA_CHECKER_TILE_SIZE: usize = 4;
pub(in crate::app) const ALPHA_CHECKER_LIGHT_RGBA: [u8; 4] = [255, 255, 255, 13];
pub(in crate::app) const ALPHA_CHECKER_DARK_RGBA: [u8; 4] = [0, 0, 0, 13];

/// Join a header/footer fill to its enclosing frame without separately
/// antialiasing an inset curve against the frame's border. Paint fill and
/// border together at the outer radius, behind the band's controls.
pub(in crate::app) fn paint_frame_band(
    ui: &Ui,
    background: egui::layers::ShapeIdx,
    inner: egui::Rect,
    mut radius: egui::CornerRadius,
    stroke: Stroke,
    fill: Color32,
    header: bool,
) {
    let mut outer = inner.expand2(Vec2::new(stroke.width, 0.0));
    if header {
        outer.min.y -= stroke.width;
        radius.sw = 0;
        radius.se = 0;
    } else {
        outer.max.y += stroke.width;
        radius.nw = 0;
        radius.ne = 0;
    }
    // Move the joined edge beyond the clip, so it adds no horizontal border
    // between the band and the body.
    let mut shape_rect = outer;
    if header { shape_rect.max.y += stroke.width + 1.0; }
    else { shape_rect.min.y -= stroke.width + 1.0; }
    let clip = outer.intersect(ui.clip_rect().expand(stroke.width)).intersect(ui.ctx().content_rect());
    let shape = egui::epaint::RectShape::new(shape_rect, radius, fill, stroke, egui::StrokeKind::Inside);
    ui.ctx().graphics_mut(|graphics| {
        graphics.entry(ui.layer_id()).mutate_shape(background, |paint| {
            paint.clip_rect = clip;
            paint.shape = shape.into();
        });
    });
}

/// A popup's bottom action band, matching its header and reaching the window
/// frame while retaining padded, normally laid out controls.
pub(in crate::app) fn popup_footer<R>(
    ui: &mut Ui,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let margin = ui.spacing().window_margin;
    // The band includes the window's side and bottom margins. Count those
    // toward the padding rather than adding them a second time below buttons.
    let padding = 8.max(margin.left).max(margin.right).max(margin.bottom);
    let inset = egui::Margin {
        left: padding - margin.left,
        right: padding - margin.right,
        top: padding,
        bottom: padding - margin.bottom,
    };
    let width = ui.available_width().max(ui.min_rect().right() - ui.next_widget_position().x);
    let fill = ui.painter().add(egui::Shape::Noop);
    let footer = egui::Frame::NONE
        .inner_margin(inset)
        .show(ui, |ui| {
            ui.set_min_width((width - inset.sum().x).max(0.0));
            contents(ui)
        });
    let mut band = footer.response.rect;
    band.min.x -= f32::from(margin.left);
    band.max.x += f32::from(margin.right);
    band.max.y += f32::from(margin.bottom);
    // The content clip normally excludes the window's padding. Only the
    // background painter reaches into it, leaving control clipping intact.
    let mut background_ui = ui.new_child(egui::UiBuilder::new().max_rect(footer.response.rect));
    let extra = f32::from(margin.left.max(margin.right).max(margin.bottom))
        + ui.visuals().window_stroke.width;
    background_ui.set_clip_rect(ui.clip_rect().expand(extra).intersect(ui.ctx().content_rect()));
    paint_frame_band(&background_ui, fill, band, ui.visuals().window_corner_radius,
        ui.visuals().window_stroke, foundation_block_bar(), false);
    footer
}

pub(in crate::app) fn paint_alpha_checkerboard(painter: &egui::Painter, rect: egui::Rect) {
    let tile = ALPHA_CHECKER_TILE_SIZE as f32;
    painter.rect_filled(rect, 0.0, editor_bg());
    let clipped = painter.with_clip_rect(rect);
    let columns = (rect.width() / tile).ceil() as usize;
    let rows = (rect.height() / tile).ceil() as usize;
    for row in 0..rows {
        for column in 0..columns {
            let rgba = if (column + row) % 2 == 0 {
                ALPHA_CHECKER_LIGHT_RGBA
            } else {
                ALPHA_CHECKER_DARK_RGBA
            };
            clipped.rect_filled(
                egui::Rect::from_min_size(
                    rect.min + Vec2::new(column as f32 * tile, row as f32 * tile),
                    Vec2::splat(tile),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]),
            );
        }
    }
}

pub(in crate::app) fn foundation_visuals() -> egui::Visuals {
    let mut visuals = if is_dark_mode() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(text_dark());
    // egui 0.29's: a hovered or pressed widget grows a point on each side,
    // and a slider's handle is round.
    visuals.widgets.hovered.expansion = 1.0;
    visuals.widgets.active.expansion = 1.0;
    visuals.handle_shape = egui::style::HandleShape::Circle;
    visuals.panel_fill = editor_bg();
    visuals.window_fill = editor_bg();
    visuals.faint_bg_color = row_type();
    visuals.extreme_bg_color = foundation_input();
    visuals.selection.bg_fill = selection_fill_for(is_dark_mode());
    visuals.selection.stroke = Stroke::new(1.0_f32, selection_stroke_for(is_dark_mode()));
    visuals.widgets.noninteractive.bg_fill = row_type();
    visuals.widgets.inactive.bg_fill = if is_dark_mode() {
        Color32::from_rgb(68, 68, 68)
    } else {
        Color32::from_rgb(218, 218, 214)
    };
    // Buttons and combo boxes paint `weak_bg_fill`. Use premultiplied sRGB
    // values for a visually literal 10% neutral overlay; `from_white_alpha`
    // gamma-converts the color channels and reads much brighter on dark bars.
    visuals.widgets.inactive.weak_bg_fill = neutral_button_fill(is_dark_mode(), 26);
    visuals.widgets.hovered.weak_bg_fill = neutral_button_fill(is_dark_mode(), 51);
    visuals.widgets.active.weak_bg_fill = neutral_button_fill(is_dark_mode(), 64);
    visuals.widgets.open.weak_bg_fill = neutral_button_fill(is_dark_mode(), 51);
    visuals.widgets.hovered.bg_fill = if is_dark_mode() {
        Color32::from_rgb(82, 82, 82)
    } else {
        Color32::from_rgb(201, 215, 221)
    };
    visuals.widgets.active.bg_fill = if is_dark_mode() {
        Color32::from_rgb(92, 92, 92)
    } else {
        Color32::from_rgb(188, 207, 216)
    };
    if !is_dark_mode() {
        // Sliders paint both their rail and handle with bg_fill, while
        // buttons use weak_bg_fill. Share the button palette so the light
        // slider no longer disappears into its surrounding panel.
        opaque_widget_fills(&mut visuals, editor_bg());
    }
    visuals.menu_corner_radius = egui::CornerRadius::same(5);
    visuals.window_stroke = Stroke::new(1.0_f32, foundation_group_edge());
    visuals
}

/// Match translucent button colors without showing the slider rail through
/// its handle. Resolve against the surface the controls are drawn on.
pub(in crate::app) fn opaque_widget_fills(visuals: &mut egui::Visuals, background: Color32) {
    visuals.widgets.inactive.bg_fill = background.blend(visuals.widgets.inactive.weak_bg_fill);
    visuals.widgets.hovered.bg_fill = background.blend(visuals.widgets.hovered.weak_bg_fill);
    visuals.widgets.active.bg_fill = background.blend(visuals.widgets.active.weak_bg_fill);
}

fn neutral_button_fill(dark_mode: bool, alpha: u8) -> Color32 {
    if dark_mode {
        Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha)
    } else {
        Color32::from_black_alpha(alpha)
    }
}

fn selection_fill_for(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(64, 108, 134) // #406C86
    } else {
        Color32::from_rgb(75, 125, 155) // #4B7D9B
    }
}

fn selection_stroke_for(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(120, 170, 198)
    } else {
        Color32::from_rgb(38, 63, 78) // #263F4E
    }
}

/// Consistent styling for empty text-input prompts without changing egui's
/// general weak-text color, which is also used by unrelated disabled UI.
pub(in crate::app) fn placeholder_text(text: impl Into<String>) -> RichText {
    RichText::new(text).color(text_dark().gamma_multiply(0.5))
}

/// Named family used for bold headers (egui has no font-weight API — bold is a
/// separate font). Falls back to the regular family when no bold font is found.
pub(in crate::app) const FOUNDATION_BOLD: &str = "foundation_bold";

/// Name of the last-resort face appended to every family for glyph coverage.
const GLYPH_FALLBACK: &str = "glyph_fallback";

/// System CJK faces are fallbacks, shared between contexts rather than reading
/// the large font collections again whenever a preview creates its own fonts.
fn cjk_font_fallbacks() -> &'static [(String, std::sync::Arc<FontData>)] {
    static FONTS: std::sync::OnceLock<Vec<(String, std::sync::Arc<FontData>)>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        [
            ("cjk_japanese", &[
                r"C:\Windows\Fonts\YuGothR.ttc",
                r"C:\Windows\Fonts\msgothic.ttc",
                "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            ][..]),
            ("cjk_chinese_simplified", &[
                r"C:\Windows\Fonts\msyh.ttc",
                r"C:\Windows\Fonts\simsun.ttc",
                "/System/Library/Fonts/PingFang.ttc",
                "/System/Library/Fonts/Supplemental/Songti.ttc",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            ][..]),
            ("cjk_chinese_traditional", &[
                r"C:\Windows\Fonts\msjh.ttc",
                r"C:\Windows\Fonts\mingliu.ttc",
                "/System/Library/Fonts/PingFang.ttc",
                "/System/Library/Fonts/Supplemental/Songti.ttc",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            ][..]),
            ("cjk_korean", &[
                r"C:\Windows\Fonts\malgun.ttf",
                "/System/Library/Fonts/AppleSDGothicNeo.ttc",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            ][..]),
        ].into_iter().filter_map(|(name, paths)| {
            paths.iter().find_map(|path| std::fs::read(path).ok().map(|bytes| (*path, bytes))).map(|(path, bytes)| {
                let mut data = FontData::from_owned(bytes);
                if path.ends_with("YuGothR.ttc") {
                    // Yu Gothic's native metrics place fallback glyphs above
                    // the UI baseline, including the Chinese ideographs it
                    // supplies. Correct this face without moving Malgun or
                    // the Chinese fonts whose baseline already lines up.
                    data.tweak.y_offset_factor = 0.375;
                }
                (name.to_owned(), std::sync::Arc::new(data))
            })
        }).collect()
    })
}

pub(in crate::app) fn foundation_fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for path in [
        r"C:\Windows\Fonts\micross.ttf",
        r"C:\Windows\Fonts\tahoma.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "foundation_ui".to_owned(),
                std::sync::Arc::new(FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "foundation_ui".to_owned());
            break;
        }
    }

    // Glyph fallback, appended *last* so it only supplies characters the fonts
    // above lack — the UI keeps the face it already had.
    //
    // Without it the default family on macOS and Linux is Ubuntu-Light plus the
    // two emoji fonts, and none of the three carry the arrows and geometric
    // shapes the UI uses: the modified-tag dot rendered as a tofu box. Windows
    // never showed it, because Segoe/Tahoma above is inserted at index 0 and
    // has them all.
    for path in [
        // Verified to carry every glyph the UI uses that the defaults miss.
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        r"C:\Windows\Fonts\seguisym.ttf",
    ] {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        fonts.font_data.insert(
            GLYPH_FALLBACK.to_owned(),
            std::sync::Arc::new(FontData::from_owned(bytes)),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push(GLYPH_FALLBACK.to_owned());
        }
        break;
    }

    // The default Latin and symbol faces do not cover Japanese kana, Chinese
    // ideographs or Korean Hangul. Include fallbacks for editable text as well
    // as labels; the bold family below inherits the same coverage.
    for (name, data) in cjk_font_fallbacks() {
        fonts.font_data.insert(name.clone(), data.clone());
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }

    // Bold face for headers (Foundation uses FontWeight=Bold). Try common
    // system bold fonts across platforms; gracefully degrade to the regular
    // family if none are present so the named family is always valid.
    let bold_loaded = [
        r"C:\Windows\Fonts\segoeuib.ttf",
        r"C:\Windows\Fonts\tahomabd.ttf",
        r"C:\Windows\Fonts\arialbd.ttf",
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/System/Library/Fonts/Supplemental/Tahoma Bold.ttf",
        "/Library/Fonts/Arial Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    ]
    .iter()
    .any(|path| match std::fs::read(path) {
        Ok(bytes) => {
            fonts.font_data.insert(
                FOUNDATION_BOLD.to_owned(),
                std::sync::Arc::new(FontData::from_owned(bytes)),
            );
            true
        }
        Err(_) => false,
    });

    let regular = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut bold_family = Vec::new();
    if bold_loaded {
        bold_family.push(FOUNDATION_BOLD.to_owned());
    }
    bold_family.extend(regular); // glyph fallback (and the whole family if no bold)
    fonts
        .families
        .insert(FontFamily::Name(FOUNDATION_BOLD.into()), bold_family);

    fonts
}

/// A bold [`FontId`] at `size`, for headers. Renders bold where a system bold
/// font was found, otherwise regular weight.
pub(in crate::app) fn bold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FOUNDATION_BOLD.into()))
}

pub(in crate::app) fn foundation_style() -> egui::Style {
    let mut style = egui::Style::default();
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(17.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(10.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::proportional(12.0));
    style.spacing.item_spacing = Vec2::new(4.0, 3.0);
    style.spacing.button_padding = Vec2::new(BUTTON_TEXT_PADDING_X, 2.0);
    style.spacing.icon_spacing = BUTTON_ICON_TEXT_GAP;
    // egui uses `interact_size` as the floor for ordinary controls. Keeping
    // the vertical floor here makes text, icon, and image+text buttons align;
    // dense custom-painted editor cells continue to use their explicit sizes.
    style.spacing.interact_size.y = BUTTON_HEIGHT;
    // egui 0.36 changed these defaults; Baboon keeps egui 0.29's: animations
    // (collapsing, fades, scroll) a twelfth of a second rather than a fifth,
    // scroll areas without a fade at their edges, and window edges that grab
    // for a resize five points out rather than three.
    style.animation_time = 1.0 / 12.0;
    style.spacing.scroll.fade.strength = 0.0;
    style.interaction.resize_grab_radius_side = 5.0;
    style
}

/// What a window adds around its content: frame margins, stroke and, with a
/// title bar, the title bar. egui 0.29 sized a window by its content; egui
/// 0.36 sizes it by everything it covers. Every window size in Baboon was
/// picked for the content, so [`window_size`] and friends add this back.
fn window_chrome(ctx: &egui::Context, title_bar: bool) -> Vec2 {
    let style = ctx.global_style();
    let frame = Frame::window(&style);
    let mut chrome = frame.total_margin().sum();
    if title_bar {
        // The title row: a heading-font line inside the window margin, then
        // the title frame's own bottom edge.
        let heading =
            ctx.fonts_mut(|fonts| fonts.row_height(&TextStyle::Heading.resolve(&style)));
        let title_edge = frame.inner_margin(0).total_margin().bottom;
        chrome.y += heading + frame.inner_margin.sum().y + title_edge;
    }
    chrome.ceil()
}

/// The outer size egui 0.36 wants for a window whose content is `content`.
pub(in crate::app) fn window_size(ctx: &egui::Context, content: Vec2, title_bar: bool) -> Vec2 {
    content + window_chrome(ctx, title_bar)
}

/// The outer width for a window whose content is `content` wide.
pub(in crate::app) fn window_width(ctx: &egui::Context, content: f32) -> f32 {
    content + window_chrome(ctx, false).x
}

/// The outer height for a window whose content is `content` tall.
pub(in crate::app) fn window_height(ctx: &egui::Context, content: f32, title_bar: bool) -> f32 {
    content + window_chrome(ctx, title_bar).y
}

/// Where the root frame's windows may sit: the screen less the menu bar, the
/// status bar and the terminal. Recorded by the root frame each pass.
fn window_work_area_id() -> egui::Id {
    egui::Id::new("baboon_window_work_area")
}

pub(in crate::app) fn set_window_work_area(ctx: &egui::Context, rect: egui::Rect) {
    ctx.data_mut(|data| data.insert_temp(window_work_area_id(), rect));
}

/// The rect a window with no position of its own is placed and kept in.
/// egui 0.29 placed such a window below the app's panels; egui 0.36 places it
/// at the top of its constrain rect, which is the whole screen unless set,
/// so it opened over the menu bar.
pub(in crate::app) fn window_work_area(ctx: &egui::Context) -> egui::Rect {
    ctx.data(|data| data.get_temp(window_work_area_id()))
        .unwrap_or_else(|| ctx.content_rect())
}

static DARK_MODE_ENABLED: AtomicBool = AtomicBool::new(false);

pub(in crate::app) fn set_dark_mode(enabled: bool) {
    DARK_MODE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub(in crate::app) fn is_dark_mode() -> bool {
    DARK_MODE_ENABLED.load(Ordering::Relaxed)
}

pub(in crate::app) fn menu_bar() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(50, 50, 50)
    } else {
        Color32::from_rgb(161, 161, 157)
    }
}

/// Fill for the selected tab in every tiled tab rack. In dark mode this must
/// sit visibly above the surrounding 31-55 gray fills; the old menu-bar gray
/// was actually darker than document tabs, which hid the active selection.
pub(in crate::app) fn active_tab() -> Color32 {
    active_tab_for(is_dark_mode())
}

fn active_tab_for(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(72, 72, 72)
    } else {
        Color32::from_rgb(161, 161, 157)
    }
}

pub(in crate::app) fn foundation_blue() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(134, 184, 213)
    } else {
        Color32::from_rgb(15, 43, 64)
    }
}

pub(in crate::app) fn left_panel() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(31, 31, 31)
    } else {
        Color32::from_rgb(238, 238, 234)
    }
}

pub(in crate::app) fn editor_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(40, 40, 40)
    } else {
        Color32::from_rgb(224, 224, 220)
    }
}

pub(in crate::app) fn row_type() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(55, 55, 55)
    } else {
        Color32::from_rgb(219, 219, 216)
    }
}

pub(in crate::app) fn grid_line() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(82, 82, 82)
    } else {
        Color32::from_rgb(180, 180, 174)
    }
}

pub(in crate::app) fn foundation_group_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(43, 43, 43)
    } else {
        Color32::from_rgb(236, 236, 234)
    }
}

pub(in crate::app) fn foundation_documentation_bg() -> Color32 {
    if is_dark_mode() {
        // Use an explicitly premultiplied sRGB overlay. `from_white_alpha`
        // gamma-expands this value, which makes a nominal 5% white look much
        // brighter than the equivalent design-tool/CSS overlay.
        Color32::from_rgba_premultiplied(13, 13, 13, 13)
    } else {
        Color32::from_black_alpha(13)
    }
}

pub(in crate::app) fn foundation_group_edge() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(82, 82, 82)
    } else {
        Color32::from_rgb(152, 152, 148)
    }
}

pub(in crate::app) fn foundation_section_bar() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(68, 68, 68)
    } else {
        Color32::from_rgb(214, 214, 210)
    }
}

pub(in crate::app) fn foundation_block_bar() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(72, 72, 72)
    } else {
        // Keep block headers distinct from the section bar without carrying
        // the dark-theme slab into an otherwise light editor.
        Color32::from_rgb(204, 204, 200)
    }
}

pub(in crate::app) fn foundation_block_text() -> Color32 {
    text_dark()
}

/// High-visibility navigation accent used by the filtered block jump. Block
/// headers are mid-gray in both themes, so one bright cyan works cleanly on
/// each without colliding with the green structural-edit controls.
pub(in crate::app) fn foundation_jump_cyan() -> Color32 {
    Color32::from_rgb(77, 208, 225)
}

fn input_fill_for(dark_mode: bool) -> Color32 {
    // Match the literal sRGB overlay convention used by buttons: black in
    // dark mode, white in light mode, both at approximately 30% alpha.
    neutral_button_fill(!dark_mode, 77)
}

pub(in crate::app) fn foundation_input() -> Color32 {
    input_fill_for(is_dark_mode())
}

/// Read-only inputs share the standard fill; their text/interaction conveys state.
pub(in crate::app) fn foundation_input_read_only() -> Color32 {
    foundation_input()
}

pub(in crate::app) fn foundation_input_edge() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(84, 84, 84)
    } else {
        Color32::from_rgb(112, 112, 108)
    }
}

pub(in crate::app) fn text_dark() -> Color32 {
    text_dark_for(is_dark_mode())
}

fn text_dark_for(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(242, 242, 242)
    } else {
        Color32::from_rgb(25, 25, 24)
    }
}

pub(in crate::app) fn subtle_dark() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(164, 164, 164)
    } else {
        Color32::from_rgb(82, 82, 78)
    }
}

/// Green for good news worth noticing in passing — an available update sitting
/// in the status bar. Darkened for the light theme so it stays legible against
/// a pale background.
pub(in crate::app) fn good_news() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(126, 205, 133)
    } else {
        Color32::from_rgb(22, 116, 51)
    }
}

/// Shared error fill for inputs: a subtle red tint in either theme.
pub(in crate::app) fn invalid_value_fill() -> Color32 {
    invalid_value_fill_for(is_dark_mode())
}

fn invalid_value_fill_for(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(48, 30, 32)
    } else {
        Color32::from_rgb(255, 240, 240)
    }
}

/// Text marking the part of a value that is wrong, legible on
/// [`invalid_value_fill`] and on an ordinary value box.
pub(in crate::app) fn invalid_value_text() -> Color32 {
    invalid_value_text_for(is_dark_mode())
}

fn invalid_value_text_for(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(255, 124, 124)
    } else {
        Color32::from_rgb(166, 35, 45)
    }
}

/// Keep an erroneous input's border red even when the input has focus.
pub(in crate::app) fn invalid_value_stroke() -> Stroke {
    Stroke::new(1.0_f32, if is_dark_mode() {
        Color32::from_rgb(171, 70, 76)
    } else {
        Color32::from_rgb(192, 75, 84)
    })
}

/// A rejected draft is visible for correction but is not part of the document.
pub(in crate::app) fn unapplied_value_fill() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(54, 43, 24)
    } else {
        Color32::from_rgb(255, 247, 220)
    }
}

pub(in crate::app) fn unapplied_value_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(240, 190, 90)
    } else {
        Color32::from_rgb(122, 80, 0)
    }
}

/// Formatting tags in string list text: `<color …>`, `<br/>`.
pub(in crate::app) fn markup_tag_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(197, 146, 230)
    } else {
        Color32::from_rgb(121, 46, 156)
    }
}

/// The underline under markup the game won't read the way it looks.
pub(in crate::app) fn markup_warning_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(240, 180, 70)
    } else {
        Color32::from_rgb(176, 104, 0)
    }
}

pub(in crate::app) fn function_plot_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(64, 64, 62)
    } else {
        Color32::from_rgb(205, 205, 205)
    }
}

pub(in crate::app) fn function_grid_line() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(132, 132, 126)
    } else {
        Color32::from_rgb(92, 92, 88)
    }
}

pub(in crate::app) fn foundation_flag_hover() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(58, 58, 56)
    } else {
        Color32::from_rgb(232, 232, 228)
    }
}

pub(in crate::app) const MATERIAL_PANEL: Color32 = Color32::from_rgb(238, 238, 235);
pub(in crate::app) const MATERIAL_PANEL_EDGE: Color32 = Color32::from_rgb(168, 168, 162);

/// The material/shader pane's backing fill. The light constant above bled
/// through every 1-2px gap between the dark grid's rows and cells in dark
/// mode, outlining the whole editor in near-white; dark mode gets a soft grey
/// in the same family as the grid lines instead.
pub(in crate::app) fn material_panel() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(58, 60, 56)
    } else {
        MATERIAL_PANEL
    }
}

pub(in crate::app) fn material_panel_edge() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(72, 74, 70)
    } else {
        MATERIAL_PANEL_EDGE
    }
}
pub(in crate::app) const MATERIAL_REF_ROW: Color32 = Color32::from_rgb(166, 205, 166);
pub(in crate::app) const MATERIAL_NUMERIC_ROW: Color32 = Color32::from_rgb(232, 191, 171);
pub(in crate::app) const MATERIAL_DATA_ROW: Color32 = Color32::from_rgb(216, 216, 216);
pub(in crate::app) const MATERIAL_GRID: Color32 = Color32::from_rgb(92, 92, 92);
pub(in crate::app) const MATERIAL_GRID_LIGHT: Color32 = Color32::from_rgb(198, 198, 192);
pub(in crate::app) const MATERIAL_INPUT_EDGE: Color32 = Color32::from_rgb(112, 112, 112);
pub(in crate::app) const MATERIAL_DEFAULT_BOX: Color32 = Color32::from_rgb(224, 224, 224);
pub(in crate::app) const MATERIAL_TEXT: Color32 = Color32::from_rgb(20, 20, 20);
pub(in crate::app) const MATERIAL_MUTED_TEXT: Color32 = Color32::from_rgb(96, 96, 96);
pub(in crate::app) const MATERIAL_FUNCTION_ROW: Color32 = Color32::from_rgb(239, 205, 137);

pub(in crate::app) fn disclosure_triangle_green() -> Color32 {
    Color32::from_rgb(34, 205, 84)
}

pub(in crate::app) fn foundation_block_edge() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(94, 94, 94)
    } else {
        Color32::from_rgb(154, 154, 149)
    }
}

pub(in crate::app) fn browser_search_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_black_alpha(102)
    } else {
        Color32::from_rgba_premultiplied(102, 102, 102, 102)
    }
}

pub(in crate::app) fn browser_toolbar_bg() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(68, 68, 68)
    } else {
        Color32::from_rgb(226, 226, 222)
    }
}

pub(in crate::app) fn browser_toolbar_active() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(73, 112, 136)
    } else {
        Color32::from_rgb(93, 137, 158)
    }
}

pub(in crate::app) fn context_menu_hover() -> Color32 {
    neutral_button_fill(is_dark_mode(), 26)
}

pub(in crate::app) fn disclosure_triangle_blue() -> Color32 {
    Color32::from_rgb(24, 111, 205)
}

pub(in crate::app) fn material_ref_row() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(30, 58, 40)
    } else {
        MATERIAL_REF_ROW
    }
}

pub(in crate::app) fn material_numeric_row() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(62, 45, 39)
    } else {
        MATERIAL_NUMERIC_ROW
    }
}

pub(in crate::app) fn material_data_row() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(42, 43, 41)
    } else {
        MATERIAL_DATA_ROW
    }
}

pub(in crate::app) fn material_grid_light() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(54, 56, 52)
    } else {
        MATERIAL_GRID_LIGHT
    }
}

pub(in crate::app) fn material_input_edge() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(82, 86, 78)
    } else {
        MATERIAL_INPUT_EDGE
    }
}

pub(in crate::app) fn material_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(231, 232, 226)
    } else {
        MATERIAL_TEXT
    }
}

#[cfg(test)]
pub(in crate::app) fn material_text_for_bg(bg: Color32) -> Color32 {
    let luminance = 0.2126 * bg.r() as f32 + 0.7152 * bg.g() as f32 + 0.0722 * bg.b() as f32;
    if luminance < 128.0 {
        Color32::from_gray(232)
    } else {
        Color32::from_gray(20)
    }
}

pub(in crate::app) fn material_muted_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(155, 158, 150)
    } else {
        MATERIAL_MUTED_TEXT
    }
}

pub(in crate::app) fn material_function_row() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(58, 47, 32)
    } else {
        MATERIAL_FUNCTION_ROW
    }
}

pub(in crate::app) fn material_delete_text() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(226, 92, 92)
    } else {
        Color32::DARK_RED
    }
}

/// Shared framed color preview for shader parameters and ordinary tag fields.
pub(in crate::app) fn paint_color_preview(ui: &Ui, rect: egui::Rect, color: Color32) {
    // Shader cells own their click responses. Resolve the same button palette
    // without creating a competing interaction or changing the cell's width.
    let hovered = ui.is_enabled() && ui.rect_contains_pointer(rect);
    let visuals = if hovered && ui.input(|input| input.pointer.primary_down()) {
        &ui.visuals().widgets.active
    } else if hovered {
        &ui.visuals().widgets.hovered
    } else {
        &ui.visuals().widgets.inactive
    };
    let frame = rect.expand(visuals.expansion);
    ui.painter().rect_filled(frame, visuals.corner_radius, visuals.weak_bg_fill);
    ui.painter().rect_stroke(frame, visuals.corner_radius, visuals.bg_stroke, egui::StrokeKind::Inside);
    paint_color_preview_fill(ui, rect, color);
}

fn paint_color_preview_fill(ui: &Ui, rect: egui::Rect, color: Color32) {
    let inner = rect.shrink(3.0);
    ui.painter().rect_filled(inner, 0.0, Color32::from_rgb(color.r(), color.g(), color.b()));
    ui.painter().rect_stroke(inner, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
}

/// A color swatch with the standard button frame, including themed hover,
/// focus and pressed borders and corner radii.
pub(in crate::app) fn color_preview_button(ui: &mut Ui, size: Vec2, color: Color32) -> egui::Response {
    let response = ui.add_sized(size, egui::Button::new(""));
    paint_color_preview_fill(ui, response.rect, color);
    response
}

pub(in crate::app) fn material_color_swatch_edge(color: Color32) -> Color32 {
    let luminance =
        0.2126 * color.r() as f32 + 0.7152 * color.g() as f32 + 0.0722 * color.b() as f32;
    if luminance < 80.0 {
        Color32::from_rgb(238, 238, 232)
    } else if luminance > 188.0 {
        Color32::from_rgb(24, 24, 22)
    } else {
        material_input_edge()
    }
}

pub(in crate::app) const MATERIAL_PARAMETER_SECTIONS: &[&str] = &[
    "ALBEDO",
    "BUMP_MAPPING",
    "MATERIAL_MODEL",
    "ENVIRONMENT_MAPPING",
    "SELF_ILLUMINATION",
    "ATMOSPHERE PROPERTIES",
    "MISC",
];
/// How long a status message stays on the status line before it clears.
/// Without this the last thing that happened sits there indefinitely, which
/// reads as current state long after it stopped being true.
pub(in crate::app) const STATUS_LINGER_SECS: f64 = 5.0;

pub(in crate::app) const FOUNDATION_LABEL_WIDTH: f32 = 280.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singleline_text_is_centered_in_taller_fields() {
        for scale in [1.0, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let mut text = "App name and path".to_owned();
            let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let output = singleline_text_edit(&mut text)
                        .min_size(Vec2::new(200.0, 40.0))
                        .show(ui);
                    let center = output.galley_pos.y + output.galley.size().y * 0.5;
                    assert!((center - output.response.rect.center().y).abs() <= 1.0,
                        "text should be vertically centered at scale {scale}");
                });
            });
        }
    }

    #[test]
    #[cfg(windows)]
    fn installed_cjk_text_is_vertically_centered_with_latin_text() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        let _ = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
            ui.fonts_mut(|fonts| {
                for size in [12.0, 16.0] {
                    let font = FontId::proportional(size);
                    let samples = ["English", "日本語", "简体中文", "繁體中文", "한국어", "汉字體中文", "English 日本語 中文 한국어"];
                    let centers: Vec<_> = samples.iter().map(|sample| {
                        let galley = fonts.layout_no_wrap(sample.to_string(), font.clone(), Color32::WHITE);
                        (sample, galley.mesh_bounds.center().y - galley.size().y * 0.5)
                    }).collect();
                    for (_, center) in &centers[1..] {
                        assert!((center - centers[0].1).abs() <= 1.5, "font size {size}: rendered centers {centers:?}");
                    }
                }
            });
        });
    }

    #[test]
    fn installed_cjk_fallbacks_render_in_text_fields_labels_and_headers() {
        let definitions = foundation_fonts();
        let samples: Vec<_> = [
            ("cjk_japanese", "日本語ひらがなカタカナ"),
            ("cjk_chinese_simplified", "简体中文汉语"),
            ("cjk_chinese_traditional", "繁體中文漢語"),
            ("cjk_korean", "한국어한글"),
        ].into_iter().filter(|(name, _)| definitions.font_data.contains_key(*name)).collect();
        // Minimal CI systems need not have optional system language fonts.
        // Exercise actual glyph coverage for every installed fallback.
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions);
        let _ = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
            ui.fonts_mut(|fonts| {
                for family in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name(FOUNDATION_BOLD.into())] {
                    let font = FontId::new(12.0, family);
                    for (name, sample) in &samples {
                        assert!(fonts.has_glyphs(&font, sample), "{name} missing glyphs in {font:?}: {sample}");
                        let galley = fonts.layout_no_wrap(sample.to_string(), font.clone(), Color32::WHITE);
                        assert!(galley.size().x > 0.0);
                    }
                }
            });
        });
    }

    #[test]
    fn neutral_button_fills_use_literal_ten_and_twenty_percent_channels() {
        assert_eq!(neutral_button_fill(true, 26).to_array(), [26, 26, 26, 26]);
        assert_eq!(neutral_button_fill(true, 51).to_array(), [51, 51, 51, 51]);
        assert_eq!(neutral_button_fill(false, 26).to_array(), [0, 0, 0, 26]);
        assert_eq!(neutral_button_fill(false, 51).to_array(), [0, 0, 0, 51]);
    }

    /// WCAG contrast ratio between two opaque colors.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |c: Color32| {
            let channel = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        };
        let (x, y) = (luminance(a) + 0.05, luminance(b) + 0.05);
        x.max(y) / x.min(y)
    }

    /// An invalid value's box keeps its text readable in either theme, and
    /// the part marked wrong stands out on it. Foundation's pink under the
    /// dark theme's light text read at about 1.2:1.
    #[test]
    fn invalid_values_stay_readable_in_both_themes() {
        for dark in [false, true] {
            let fill = invalid_value_fill_for(dark);
            assert!(contrast(text_dark_for(dark), fill) >= 7.0, "text on the fill, dark {dark}");
            assert!(contrast(invalid_value_text_for(dark), fill) >= 4.5, "marked text on the fill, dark {dark}");
        }
        assert!(contrast(text_dark_for(true), Color32::from_rgb(0xFF, 0xC0, 0xCB)) < 1.5);
        assert_eq!(invalid_value_fill_for(false), Color32::from_rgb(255, 240, 240), "subtle red tint");
    }

    #[test]
    fn selection_colors_are_lighter_with_a_dark_border_in_light_mode() {
        assert_eq!(selection_fill_for(true), Color32::from_rgb(64, 108, 134));
        assert_eq!(selection_fill_for(false), Color32::from_rgb(75, 125, 155));
        assert_eq!(selection_stroke_for(false), Color32::from_rgb(38, 63, 78));
    }

    // Unit tests for shared visual-style helpers.
    // It owns test-only characterization and does not participate in runtime application behavior.

    #[test]
    fn material_text_for_bg_chooses_contrasting_foreground() {
        assert_eq!(
            material_text_for_bg(Color32::from_rgb(42, 43, 41)),
            Color32::from_gray(232)
        );
        assert_eq!(
            material_text_for_bg(Color32::from_rgb(232, 191, 171)),
            Color32::from_gray(20)
        );
    }

    #[test]
    fn dark_active_tab_is_lighter_than_every_tab_rack_background() {
        let active = active_tab_for(true);
        assert_eq!(active, Color32::from_rgb(72, 72, 72));
        assert!(active.r() > 55, "document/Chimp inactive gray");
        assert!(active.r() > 31, "workspace inactive gray");
    }

    #[test]
    fn light_active_tab_keeps_the_existing_menu_bar_color() {
        assert_eq!(active_tab_for(false), Color32::from_rgb(161, 161, 157));
    }

    #[test]
    fn filtered_block_jump_uses_the_cyan_navigation_accent() {
        assert_eq!(foundation_jump_cyan(), Color32::from_rgb(77, 208, 225));
    }

    /// A window sized through [`window_size`] gets the content area it asks for,
    /// with a title bar and without, resizable or not, at two heading sizes. The
    /// sizes in Baboon were picked for the content, which is what egui 0.29
    /// sized; egui 0.36 sizes the whole window.
    #[test]
    fn a_window_sized_for_its_content_gets_that_content_area() {
        let content = Vec2::new(400.0, 300.0);
        for heading in [17.0, 24.0] {
            let ctx = egui::Context::default();
            crate::app::Baboon::configure_context(&ctx);
            ctx.global_style_mut(|style| {
                style
                    .text_styles
                    .insert(TextStyle::Heading, FontId::proportional(heading));
            });
            let mut seen = Vec::new();
            for frame in 0..3 {
                seen.clear();
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1600.0, 1000.0),
                    )),
                    time: Some(f64::from(frame)),
                    ..Default::default()
                };
                let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                    let ctx = ui.ctx().clone();
                    for (name, title_bar, resizable) in
                        [("a", true, true), ("b", true, false), ("c", false, true)]
                    {
                        let shown = egui::Window::new(name)
                            .title_bar(title_bar)
                            .resizable(resizable)
                            .default_size(window_size(&ctx, content, title_bar))
                            .default_pos(egui::pos2(100.0, 100.0))
                            .show(&ctx, |ui| {
                                let available = ui.available_size();
                                ui.allocate_space(available);
                                available
                            });
                        seen.push((name, shown.and_then(|shown| shown.inner)));
                    }
                });
            }
            for (name, available) in &seen {
                let available = available.expect("the window drew");
                assert!(
                    (available - content).abs().max_elem() < 1.0,
                    "heading {heading}, window {name}: content {available:?}, asked for {content:?}"
                );
            }
        }
    }

    /// A window with no position of its own opens inside the work area the root
    /// frame records — below the menu bar — as egui 0.29 placed it.
    #[test]
    fn a_window_without_a_position_opens_inside_the_work_area() {
        let ctx = egui::Context::default();
        crate::app::Baboon::configure_context(&ctx);
        let work_area = egui::Rect::from_min_max(egui::pos2(0.0, 29.0), egui::pos2(1600.0, 970.0));
        let mut rect = egui::Rect::NOTHING;
        for frame in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1000.0),
                )),
                time: Some(f64::from(frame)),
                ..Default::default()
            };
            let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                set_window_work_area(ui.ctx(), work_area);
                let ctx = ui.ctx().clone();
                rect = egui::Window::new("Unplaced")
                    .constrain_to(window_work_area(&ctx))
                    .show(&ctx, |ui| ui.label("text"))
                    .expect("the window drew")
                    .response
                    .rect;
            });
        }
        assert!(work_area.contains_rect(rect), "{rect:?} is outside {work_area:?}");
        assert!(rect.top() > work_area.top(), "{rect:?}");
    }

    /// How far a drag from `grab` moves a window, with or without a title bar.
    fn window_moved_by_drag(title_bar: bool, grab: Vec2) -> Vec2 {
        let ctx = egui::Context::default();
        crate::app::Baboon::configure_context(&ctx);
        let start = egui::pos2(100.0, 100.0);
        let frame = |events: Vec<egui::Event>, time: f64| {
            let mut rect = egui::Rect::NOTHING;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1000.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                let ctx = ui.ctx().clone();
                rect = egui::Window::new("Dragged")
                    .title_bar(title_bar)
                    .resizable(false)
                    .default_pos(start)
                    .show(&ctx, |ui| {
                        ui.allocate_space(egui::vec2(300.0, 200.0));
                    })
                    .expect("the window drew")
                    .response
                    .rect;
            });
            rect
        };
        frame(Vec::new(), 0.0);
        let before = frame(Vec::new(), 0.1).min;
        let from = before + grab;
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(from)], 0.2);
        frame(vec![button(from, true)], 0.3);
        for step in 1..=5 {
            let at = from + egui::vec2(20.0, 10.0) * step as f32;
            frame(vec![egui::Event::PointerMoved(at)], 0.3 + 0.1 * f64::from(step));
        }
        let to = from + egui::vec2(100.0, 50.0);
        frame(vec![button(to, false)], 1.0);
        frame(Vec::new(), 1.1).min - before
    }

    /// egui 0.36 moves a window only by its title bar. A window without one —
    /// Find, Settings, Compare Tags, the color picker — still moves by a drag
    /// anywhere on it, as every window did in 0.29; one with a title bar moves
    /// by the title bar and no longer by its body.
    #[test]
    fn a_window_without_a_title_bar_moves_by_its_body() {
        let body = Vec2::new(150.0, 120.0);
        assert_eq!(window_moved_by_drag(false, body), Vec2::new(100.0, 50.0));
        assert_eq!(window_moved_by_drag(true, body), Vec2::ZERO);
        assert_eq!(window_moved_by_drag(true, Vec2::new(150.0, 10.0)), Vec2::new(100.0, 50.0));
    }

    #[test]
    fn theme_switches_preserve_font_sizes_spacing_and_widget_dimensions() {
        let ctx = egui::Context::default();
        crate::app::Baboon::configure_context(&ctx);
        ctx.set_zoom_factor(1.25);
        let expected = foundation_style();
        let mut previous = None;
        for theme in [egui::Theme::Light, egui::Theme::Dark, egui::Theme::Light] {
            ctx.set_theme(theme);
            let style = ctx.global_style();
            assert_eq!(style.text_styles, expected.text_styles);
            assert_eq!(style.spacing.item_spacing, expected.spacing.item_spacing);
            assert_eq!(style.spacing.button_padding, expected.spacing.button_padding);
            assert_eq!(style.spacing.interact_size, expected.spacing.interact_size);
            assert_eq!(style.visuals.dark_mode, theme == egui::Theme::Dark);
            let mut sizes = None;
            let _ = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    sizes = Some((ui.label("Stable text").rect.size(), ui.button("Stable button").rect.size()));
                });
            });
            if let Some(previous) = previous {
                assert_eq!(sizes, Some(previous));
            }
            previous = sizes;
        }
    }

    /// The egui defaults that changed between 0.29 and 0.36 and that Baboon sets
    /// back to 0.29's. egui 0.36's own defaults differ in each,
    /// which the first assertions check, so this cannot pass by accident.
    #[test]
    fn style_keeps_egui_0_29_defaults_that_0_36_changed() {
        let egui_default = egui::Style::default();
        assert_ne!(egui_default.animation_time, 1.0 / 12.0);
        assert_ne!(egui_default.spacing.scroll.fade.strength, 0.0);
        assert_ne!(egui_default.interaction.resize_grab_radius_side, 5.0);
        assert_ne!(egui::Visuals::light().widgets.hovered.expansion, 1.0);

        let style = foundation_style();
        assert_eq!(style.animation_time, 1.0 / 12.0);
        assert_eq!(style.spacing.scroll.fade.strength, 0.0);
        assert_eq!(style.interaction.resize_grab_radius_side, 5.0);
        // Set the same in either theme; the theme is a global other tests read,
        // so it is left as it is.
        let visuals = foundation_visuals();
        assert_eq!(visuals.widgets.hovered.expansion, 1.0);
        assert_eq!(visuals.widgets.active.expansion, 1.0);
        assert!(matches!(visuals.handle_shape, egui::style::HandleShape::Circle));
    }
}
