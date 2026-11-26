//! Baboon colors, typography, spacing, and reusable egui style helpers.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

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
            // A direct with_layout(right_to_left(...)) otherwise takes all
            // remaining popup height. Give action layouts a content-sized row;
            // taller controls still expand it naturally.
            ui.horizontal(contents).inner
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

/// egui's visuals for the installed theme, built once per theme change.
pub(in crate::app) fn foundation_visuals() -> egui::Visuals {
    #[cfg(test)]
    if themes::has_test_colors() {
        return build_foundation_visuals();
    }
    static BUILT: std::sync::Mutex<Option<(u64, egui::Visuals)>> = std::sync::Mutex::new(None);
    let generation = themes::generation();
    let mut built = BUILT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, visuals)) = built.as_ref()
        && *at == generation
    {
        return visuals.clone();
    }
    let visuals = build_foundation_visuals();
    *built = Some((generation, visuals.clone()));
    visuals
}

fn build_foundation_visuals() -> egui::Visuals {
    use themes::Token as T;
    let c = themes::color;
    let mut visuals = if is_dark_mode() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(c(T::Text));
    // egui 0.29's: a hovered or pressed widget grows a point on each side,
    // and a slider's handle is round.
    visuals.widgets.hovered.expansion = 1.0;
    visuals.widgets.active.expansion = 1.0;
    visuals.handle_shape = egui::style::HandleShape::Circle;
    visuals.panel_fill = c(T::Background);
    visuals.window_fill = c(T::Background);
    visuals.faint_bg_color = c(T::RowBackground);
    visuals.extreme_bg_color = c(T::InputBackground);
    visuals.code_bg_color = c(T::CodeInlineBackground);
    visuals.hyperlink_color = c(T::LinkText);
    visuals.selection.bg_fill = c(T::Accent);
    visuals.selection.stroke = Stroke::new(1.0_f32, c(T::AccentBorder));
    visuals.widgets.noninteractive.bg_fill = c(T::RowBackground);
    visuals.widgets.noninteractive.bg_stroke.color = c(T::Separator);
    // Buttons and combo boxes paint `weak_bg_fill`; sliders and checkboxes
    // `bg_fill`, which a light theme keeps opaque so a slider's rail does not
    // show through its handle. An outline with no colour has no width.
    let outline = |stroke: &mut Stroke, token: T| {
        stroke.color = c(token);
        if stroke.color.a() != 0 {
            stroke.width = 1.0;
        }
    };
    let widgets = &mut visuals.widgets;
    for (widget, fill, weak, border) in [
        (
            &mut widgets.inactive,
            T::ControlBackground,
            T::ButtonBackground,
            T::ButtonBorder,
        ),
        (
            &mut widgets.hovered,
            T::ControlHover,
            T::ButtonHover,
            T::ButtonBorderHover,
        ),
        (
            &mut widgets.active,
            T::ControlActive,
            T::ButtonActive,
            T::ButtonBorderActive,
        ),
    ] {
        widget.bg_fill = c(fill);
        widget.weak_bg_fill = c(weak);
        outline(&mut widget.bg_stroke, border);
    }
    // An open combo box or menu button looks hovered.
    widgets.open.weak_bg_fill = c(T::ButtonHover);
    outline(&mut widgets.open.bg_stroke, T::ButtonBorderOpen);
    // Warnings and errors take the amber and red a field uses for an
    // unapplied or invalid value.
    visuals.warn_fg_color = c(T::StatusWarning);
    visuals.error_fg_color = c(T::StatusError);
    visuals.menu_corner_radius = egui::CornerRadius::same(5);
    visuals.window_stroke = Stroke::new(1.0_f32, c(T::GroupBorder));
    visuals
}

/// Match translucent button colors without showing the slider rail through
/// its handle. Resolve against the surface the controls are drawn on.
pub(in crate::app) fn opaque_widget_fills(visuals: &mut egui::Visuals, background: Color32) {
    visuals.widgets.inactive.bg_fill = background.blend(visuals.widgets.inactive.weak_bg_fill);
    visuals.widgets.hovered.bg_fill = background.blend(visuals.widgets.hovered.weak_bg_fill);
    visuals.widgets.active.bg_fill = background.blend(visuals.widgets.active.weak_bg_fill);
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
    egui::Id::unique("baboon_window_work_area")
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

/// Whether the installed theme is dark.
pub(in crate::app) fn is_dark_mode() -> bool {
    themes::appearance().is_dark()
}

/// Applies the theme `id`, the preferences' choice for an appearance whose
/// built-in is `slot`, before anything draws. The frame calls this every
/// pass; it does work only when the choice or one of the theme's files
/// changed. Gives back a status line to show, when the theme could not be
/// applied or reloaded.
pub(in crate::app) fn apply_theme(id: &str, slot: themes::BuiltIn) -> Option<String> {
    themes::state::with_state(|state| {
        state.apply(id, slot, std::time::Instant::now());
        state.take_status()
    })
}

/// Runs `f` with this thread drawing in the built-in theme `built_in`,
/// whatever the app's frames on other threads install meanwhile.
#[cfg(test)]
pub(in crate::app) fn with_built_in<R>(built_in: themes::BuiltIn, f: impl FnOnce() -> R) -> R {
    themes::with_colors(&built_in.colors(), f)
}

/// What light mode draws icons over, for caches of icons darkened to read on
/// it: the darkest of [`icon_backdrops`], so an icon reads on all of them.
/// `None` in dark mode, which leaves icons as drawn.
pub(in crate::app) fn icon_backdrop() -> Option<Color32> {
    if is_dark_mode() {
        return None;
    }
    let luminance = |colour: Color32| {
        let linear = egui::Rgba::from(colour);
        0.2126 * linear.r() + 0.7152 * linear.g() + 0.0722 * linear.b()
    };
    icon_backdrops()
        .into_iter()
        .min_by(|one, other| luminance(*one).total_cmp(&luminance(*other)))
}

/// The light-mode surfaces tag and button icons sit on: the tag tree and
/// tab bar, open pages and active tabs, the input wells of reference fields
/// on either, and the welcome screen's tinted column.
pub(in crate::app) fn icon_backdrops() -> [Color32; 5] {
    [
        left_panel(),
        editor_bg(),
        left_panel().blend(foundation_input()),
        editor_bg().blend(foundation_input()),
        left_panel().blend(welcome_side_panel()),
    ]
}

pub(in crate::app) fn menu_bar() -> Color32 {
    themes::color(themes::Token::MenuBarBackground)
}

/// Fill for the selected tab in every tiled tab rack. In dark mode this must
/// sit visibly above the surrounding 31-55 gray fills; the old menu-bar gray
/// was actually darker than document tabs, which hid the active selection.
/// The lines between the window's frame and the workspace: under the menu
/// bar and over the status bar. Light mode draws them in the focused
/// tab's border colour; dark mode keeps its divider.
pub(in crate::app) fn chrome_edge() -> Color32 {
    themes::color(themes::Token::ChromeBorder)
}

/// The window's frame: the menu bar and the workspace tab bar. Light mode
/// shares the status bar's peach, so the top and bottom read as one frame
/// around the warmer workspace.
pub(in crate::app) fn chrome_bar() -> Color32 {
    themes::color(themes::Token::ChromeBackground)
}

pub(in crate::app) fn active_tab() -> Color32 {
    themes::color(themes::Token::TabActiveBackground)
}

pub(in crate::app) fn foundation_blue() -> Color32 {
    themes::color(themes::Token::AccentText)
}

pub(in crate::app) fn left_panel() -> Color32 {
    themes::color(themes::Token::PanelBackground)
}

pub(in crate::app) fn editor_bg() -> Color32 {
    themes::color(themes::Token::Background)
}

pub(in crate::app) fn row_type() -> Color32 {
    themes::color(themes::Token::RowBackground)
}

pub(in crate::app) fn grid_line() -> Color32 {
    themes::color(themes::Token::Border)
}

/// Neutral emphasis for the tab owning the currently focused pane.
pub(in crate::app) fn focused_tab_border() -> Color32 {
    themes::color(themes::Token::FocusBorder)
}

pub(in crate::app) fn foundation_group_bg() -> Color32 {
    themes::color(themes::Token::GroupBackground)
}

pub(in crate::app) fn foundation_documentation_bg() -> Color32 {
    themes::color(themes::Token::CardBackground)
}

pub(in crate::app) fn foundation_group_edge() -> Color32 {
    themes::color(themes::Token::GroupBorder)
}

pub(in crate::app) fn foundation_section_bar() -> Color32 {
    themes::color(themes::Token::SectionHeaderBackground)
}

pub(in crate::app) fn foundation_block_bar() -> Color32 {
    themes::color(themes::Token::BlockHeaderBackground)
}

pub(in crate::app) fn foundation_block_text() -> Color32 {
    themes::color(themes::Token::BlockHeaderText)
}

/// A sound player's playhead: the theme's highlight, blue in dark mode and
/// Baboon Orange in light, as the waveform's played part is.
pub(in crate::app) fn playhead() -> Color32 {
    themes::color(themes::Token::AccentText)
}

/// High-visibility navigation accent used by the filtered block jump. Block
/// headers are mid-gray in both themes, so one bright cyan works cleanly on
/// each without colliding with the green structural-edit controls.
pub(in crate::app) fn foundation_jump_cyan() -> Color32 {
    themes::color(themes::Token::BlockJumpIcon)
}


pub(in crate::app) fn foundation_input() -> Color32 {
    themes::color(themes::Token::InputBackground)
}

/// Read-only inputs share the palette and fade as a whole in read_only_input_scope.
pub(in crate::app) fn foundation_input_read_only() -> Color32 {
    themes::color(themes::Token::InputBackground)
}

/// Keep page content equally inset from its side and bottom borders.
pub(in crate::app) fn pane_content_margin() -> egui::Margin {
    egui::Margin { left: 10, right: 10, top: 8, bottom: 10 }
}

pub(in crate::app) fn foundation_input_edge() -> Color32 {
    themes::color(themes::Token::InputBorder)
}

pub(in crate::app) fn text_dark() -> Color32 {
    themes::color(themes::Token::Text)
}

pub(in crate::app) fn subtle_dark() -> Color32 {
    themes::color(themes::Token::TextMuted)
}

/// Green for good news worth noticing in passing — an available update sitting
/// in the status bar. Darkened for the light theme so it stays legible against
/// a pale background.
pub(in crate::app) fn good_news() -> Color32 {
    themes::color(themes::Token::StatusSuccess)
}

/// Shared error fill for inputs: a subtle red tint in either theme.
pub(in crate::app) fn invalid_value_fill() -> Color32 {
    themes::color(themes::Token::StatusErrorBackground)
}


/// Text marking the part of a value that is wrong, legible on
/// [`invalid_value_fill`] and on an ordinary value box.
pub(in crate::app) fn invalid_value_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// Keep an erroneous input's border red even when the input has focus.
pub(in crate::app) fn invalid_value_stroke() -> Stroke {
    Stroke::new(1.0_f32, themes::color(themes::Token::StatusErrorBorder))
}

/// A rejected draft is visible for correction but is not part of the document.
pub(in crate::app) fn unapplied_value_fill() -> Color32 {
    themes::color(themes::Token::StatusWarningBackground)
}

pub(in crate::app) fn unapplied_value_text() -> Color32 {
    themes::color(themes::Token::StatusWarning)
}

/// Formatting tags in string list text: `<color …>`, `<br/>`.
pub(in crate::app) fn markup_tag_text() -> Color32 {
    themes::color(themes::Token::SyntaxMarkupTag)
}

/// The underline under markup the game won't read the way it looks.
pub(in crate::app) fn markup_warning_text() -> Color32 {
    themes::color(themes::Token::SyntaxMarkupWarning)
}

pub(in crate::app) fn function_plot_bg() -> Color32 {
    themes::color(themes::Token::GraphBackground)
}

pub(in crate::app) fn function_grid_line() -> Color32 {
    themes::color(themes::Token::GraphGrid)
}

pub(in crate::app) fn foundation_flag_hover() -> Color32 {
    themes::color(themes::Token::FlagsHover)
}

/// The material/shader pane's backing fill. A near-white one bled through
/// every 1-2px gap between the dark grid's rows and cells in dark mode,
/// outlining the whole editor; dark mode gets a soft grey in the same family
/// as the grid lines instead.
pub(in crate::app) fn material_panel() -> Color32 {
    themes::color(themes::Token::ShaderPanelBackground)
}

pub(in crate::app) fn material_panel_edge() -> Color32 {
    themes::color(themes::Token::ShaderPanelBorder)
}
/// A data row in the material editor, which is light in every theme, as Guerilla's is.
pub(in crate::app) fn material_editor_data_row() -> Color32 {
    themes::color(themes::Token::MaterialRowDataBackground)
}

/// A function row in the material editor.
pub(in crate::app) fn material_editor_function_row() -> Color32 {
    themes::color(themes::Token::MaterialRowFunctionBackground)
}

/// A value box at its default in the material editor.
pub(in crate::app) fn material_editor_box() -> Color32 {
    themes::color(themes::Token::MaterialBoxBackground)
}

/// The material editor's grid lines.
pub(in crate::app) fn material_editor_grid() -> Color32 {
    themes::color(themes::Token::MaterialGrid)
}

/// Outline of an input in the material editor and colour picker.
pub(in crate::app) fn material_editor_input_edge() -> Color32 {
    themes::color(themes::Token::MaterialInputBorder)
}

/// Text in the material editor.
pub(in crate::app) fn material_editor_text() -> Color32 {
    themes::color(themes::Token::MaterialText)
}

/// Secondary text in the material editor.
pub(in crate::app) fn material_editor_muted_text() -> Color32 {
    themes::color(themes::Token::MaterialTextMuted)
}


pub(in crate::app) fn disclosure_triangle_green() -> Color32 {
    themes::color(themes::Token::TreeFolderOpenIcon)
}

pub(in crate::app) fn foundation_block_edge() -> Color32 {
    themes::color(themes::Token::BlockBorder)
}

pub(in crate::app) fn browser_search_bg() -> Color32 {
    themes::color(themes::Token::BrowserSearchBackground)
}

pub(in crate::app) fn browser_toolbar_bg() -> Color32 {
    themes::color(themes::Token::BrowserToolbarBackground)
}

pub(in crate::app) fn browser_toolbar_active() -> Color32 {
    themes::color(themes::Token::BrowserToolbarActiveBackground)
}

pub(in crate::app) fn context_menu_hover() -> Color32 {
    themes::color(themes::Token::MenuHover)
}

pub(in crate::app) fn disclosure_triangle_blue() -> Color32 {
    themes::color(themes::Token::TreeFolderClosedIcon)
}

pub(in crate::app) fn material_ref_row() -> Color32 {
    themes::color(themes::Token::ShaderRowReferenceBackground)
}

pub(in crate::app) fn material_numeric_row() -> Color32 {
    themes::color(themes::Token::ShaderRowNumericBackground)
}

pub(in crate::app) fn material_data_row() -> Color32 {
    themes::color(themes::Token::ShaderRowDataBackground)
}

pub(in crate::app) fn material_grid_light() -> Color32 {
    themes::color(themes::Token::ShaderGrid)
}

pub(in crate::app) fn material_input_edge() -> Color32 {
    themes::color(themes::Token::ShaderInputBorder)
}

pub(in crate::app) fn material_text() -> Color32 {
    themes::color(themes::Token::ShaderText)
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
    themes::color(themes::Token::ShaderTextMuted)
}

pub(in crate::app) fn material_function_row() -> Color32 {
    themes::color(themes::Token::ShaderRowFunctionBackground)
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

// ---------------------------------------------------------------------------
// Named colours that used to be written inline where they were drawn. Each
// returns exactly what its call sites painted before; the names say what the
// colour is for, so a later change to one role does not drag another along.
// ---------------------------------------------------------------------------

/// The strongest text colour: white on dark, black on light. For a hovered
/// splitter grip and the clear button's cross.
pub(in crate::app) fn text_strong() -> Color32 {
    themes::color(themes::Token::TextStrong)
}

/// Hover wash behind a row of a custom menu.
pub(in crate::app) fn menu_row_hover() -> Color32 {
    themes::color(themes::Token::MenuBarItemHover)
}

/// Dims the window behind a modal panel.
pub(in crate::app) fn modal_scrim() -> Color32 {
    themes::color(themes::Token::ModalBackdrop)
}


/// The egui theme matching the app's.
pub(in crate::app) fn egui_theme() -> egui::Theme {
    if is_dark_mode() { egui::Theme::Dark } else { egui::Theme::Light }
}

/// [`focused_tab_border`] for the visuals `ui` paints with.
pub(in crate::app) fn focused_tab_border_in(_ui: &Ui) -> Color32 {
    focused_tab_border()
}

// Diffs and edited tags.

/// Colour for a tag that this workspace created, with no counterpart in the
/// game. Paired with a `+` marker wherever it is used, so the meaning does not
/// rest on colour alone.
pub(in crate::app) fn added_text() -> Color32 {
    themes::color(themes::Token::DiffAdded)
}

/// Background wash behind the shipped side of a diff, and behind the edited
/// side. Kept dark: the editor draws its own widgets on top, and a strong fill
/// would fight them rather than frame them.
pub(in crate::app) fn removed_wash() -> Color32 {
    themes::color(themes::Token::DiffRemovedBackground)
}

pub(in crate::app) fn added_wash() -> Color32 {
    themes::color(themes::Token::DiffAddedBackground)
}

/// Colour for a value or element that is going away. Paired with a `-` marker,
/// so red and green never carry the meaning by themselves.
pub(in crate::app) fn removed_text() -> Color32 {
    themes::color(themes::Token::DiffRemoved)
}

/// Colour for a tag or folder holding edits that are not written into the game.
/// The same goldenrod the workspace tab is tinted with.
pub(in crate::app) fn modified_text() -> Color32 {
    themes::color(themes::Token::DiffModified)
}

/// A mod-export row with nothing to write.
pub(in crate::app) fn unchanged_text() -> Color32 {
    themes::color(themes::Token::DiffUnchanged)
}

/// The goldenrod a Chimp document tab is tinted toward while it has unsaved
/// edits.
pub(in crate::app) fn dirty_tab_tint() -> Color32 {
    themes::color(themes::Token::TabModifiedTint)
}

/// Accent on a shader parameter edited away from its template's value.
pub(in crate::app) fn shader_modified_accent() -> Color32 {
    themes::color(themes::Token::ShaderRowModifiedMarker)
}

/// A value lower than the other side's in a tag comparison.
pub(in crate::app) fn value_decreased() -> Color32 {
    themes::color(themes::Token::ActionDestructive)
}

/// A tick for two things that match: an identical tag, a finished step.
pub(in crate::app) fn match_tick() -> Color32 {
    themes::color(themes::Token::StatusSuccess)
}

/// An open folder's icon in the tag tree.
pub(in crate::app) fn open_folder_icon() -> Color32 {
    themes::color(themes::Token::TreeFolderOpenIcon)
}

/// Icons for actions that remove something: Clear, Delete, Remove.
pub(in crate::app) fn destructive_icon() -> Color32 {
    themes::color(themes::Token::ActionDestructive)
}

/// A shader parameter the template no longer uses.
pub(in crate::app) fn unused_value_text() -> Color32 {
    themes::color(themes::Token::ActionDestructive)
}

// Badges, highlights and markers.

/// The four-character group badge beside a tag in the tree.
// A tag group's badge in Groups view: slate in dark mode, a pale slate in
// light mode.
pub(in crate::app) fn group_badge_fill() -> Color32 {
    themes::color(themes::Token::BadgeBackground)
}

pub(in crate::app) fn group_badge_edge() -> Color32 {
    themes::color(themes::Token::BadgeBorder)
}

pub(in crate::app) fn group_badge_text() -> Color32 {
    themes::color(themes::Token::BadgeText)
}

/// The welcome screen's left column, over the page. Dark mode shades it;
/// light mode lays a pale tint over it instead, so it stays light: sand in
/// the warm theme, the same lightness in grey in the classic one.
pub(in crate::app) fn welcome_side_panel() -> Color32 {
    themes::color(themes::Token::WelcomePanelBackground)
}

/// A navigation section's title (Favorites, Shortcuts, Recents) beside its
/// `accent` icon. Light mode writes it in the text colour, since a pale
/// accent such as the favourite star's yellow reads poorly on the light
/// panel.
pub(in crate::app) fn navigation_title_text(accent: Color32) -> Color32 {
    if is_dark_mode() { accent } else { text_dark() }
}

/// A favourite's filled star.
pub(in crate::app) fn favourite_star() -> Color32 {
    themes::color(themes::Token::FavouriteIcon)
}

/// Marks string rows waiting to be pasted: cut rows, then copied rows.
pub(in crate::app) fn clipboard_cut_mark() -> Color32 {
    themes::color(themes::Token::ClipboardCutBorder)
}

pub(in crate::app) fn clipboard_copy_mark() -> Color32 {
    themes::color(themes::Token::ClipboardCopyBorder)
}

/// Background behind text matching a find, and behind the match the find
/// is on. Text over either is [`search_match_text`].
pub(in crate::app) fn search_match_bg() -> Color32 {
    themes::color(themes::Token::SearchMatchBackground)
}

pub(in crate::app) fn search_active_match_bg() -> Color32 {
    themes::color(themes::Token::SearchMatchActiveBackground)
}

pub(in crate::app) fn search_match_text() -> Color32 {
    themes::color(themes::Token::SearchMatchText)
}

/// The glow behind a block a reference jump landed on.
pub(in crate::app) fn jump_glow() -> Color32 {
    themes::color(themes::Token::BlockJumpHighlight)
}

/// The blue "?" beside a field name with documentation.
pub(in crate::app) fn help_cue() -> Color32 {
    themes::color(themes::Token::FieldHelpText)
}

/// A tag saved with a different layout than its definitions (the triangle,
/// and the "!" painted on it), and the header note on a tag whose edits have
/// nowhere to be written. Warning-coloured rather than error-coloured: nothing
/// is wrong, and the tag is fully editable.
pub(in crate::app) fn read_only_badge() -> Color32 {
    themes::color(themes::Token::BadgeReadOnlyBackground)
}

pub(in crate::app) fn read_only_badge_mark() -> Color32 {
    themes::color(themes::Token::BadgeReadOnlyIcon)
}

/// The filled Import button on the Blam! surface: dark green with white text
/// in both themes.
pub(in crate::app) fn import_button_fill() -> Color32 {
    themes::color(themes::Token::ButtonPrimaryBackground)
}

// Function graphs.

/// The two curves of a function graph: the first, then the second.
pub(in crate::app) fn graph_curve(graph: usize) -> Color32 {
    themes::color(if graph == 0 {
        themes::Token::GraphCurvePrimary
    } else {
        themes::Token::GraphCurveSecondary
    })
}

/// The wash under a function graph's curve.
pub(in crate::app) fn graph_curve_fill(graph: usize) -> Color32 {
    themes::color(if graph == 0 {
        themes::Token::GraphCurvePrimaryFill
    } else {
        themes::Token::GraphCurveSecondaryFill
    })
}

/// The box a function graph sits in.
pub(in crate::app) fn graph_container_fill() -> Color32 {
    themes::color(themes::Token::GraphContainerBackground)
}

/// The band a function's exclusion range covers, and its edges.
pub(in crate::app) fn graph_band_fill() -> Color32 {
    themes::color(themes::Token::GraphBandBackground)
}

pub(in crate::app) fn graph_band_edge() -> Color32 {
    themes::color(themes::Token::GraphBandBorder)
}

// Terminal, progress and help.

/// The terminal pane's background, dark in both themes.
pub(in crate::app) fn terminal_bg() -> Color32 {
    themes::color(themes::Token::TerminalBackground)
}

/// Terminal text: ordinary lines, then warnings, errors and successes.
pub(in crate::app) fn terminal_text() -> Color32 {
    themes::color(themes::Token::TerminalText)
}

pub(in crate::app) fn terminal_warning_text() -> Color32 {
    themes::color(themes::Token::TerminalWarning)
}

pub(in crate::app) fn terminal_error_text() -> Color32 {
    themes::color(themes::Token::TerminalError)
}

pub(in crate::app) fn terminal_success_text() -> Color32 {
    themes::color(themes::Token::TerminalSuccess)
}

/// A dark-mode slider's rail: a see-through shade, so it sinks into
/// whatever it is drawn on, a card's header bar or its body.
pub(in crate::app) fn slider_rail_fill() -> Color32 {
    themes::color(themes::Token::SliderRailBackground)
}

/// The outline of a slider's rail; none in dark themes.
pub(in crate::app) fn slider_rail_edge() -> Color32 {
    themes::color(themes::Token::SliderRailBorder)
}

/// The loading animation's brightest strokes.
pub(in crate::app) fn loading_primary() -> Color32 {
    themes::color(themes::Token::LoadingPrimary)
}

/// The loading animation's middle strokes.
pub(in crate::app) fn loading_secondary() -> Color32 {
    themes::color(themes::Token::LoadingSecondary)
}

/// The loading animation's faint lines.
pub(in crate::app) fn loading_lines() -> Color32 {
    themes::color(themes::Token::LoadingLines)
}

/// The well inside a checked checkbox or radio button.
pub(in crate::app) fn control_well() -> Color32 {
    themes::color(themes::Token::ControlWell)
}

/// A destructive button's accent: the red of the Garbage and Close Tab icons.
pub(in crate::app) fn button_destructive_accent() -> Color32 {
    themes::color(themes::Token::ButtonDestructive)
}

/// A progress bar's track and its fill.
pub(in crate::app) fn progress_track() -> Color32 {
    themes::color(themes::Token::ProgressBackground)
}

pub(in crate::app) fn progress_fill() -> Color32 {
    themes::color(themes::Token::ProgressBar)
}

/// Behind a block of script in the help window.
pub(in crate::app) fn help_code_bg() -> Color32 {
    themes::color(themes::Token::CodeBackground)
}

// Settings cards.

/// A settings card, and one holding a setting that does not work.
pub(in crate::app) fn settings_card_fill() -> Color32 {
    themes::color(themes::Token::SettingsCardBackground)
}

pub(in crate::app) fn settings_card_invalid_fill() -> Color32 {
    themes::color(themes::Token::SettingsCardErrorBackground)
}

/// Widgets inside a light settings card get opaque fills over the card's
/// tint, so they read as they do elsewhere; dark cards keep the default.
pub(in crate::app) fn settings_card_widget_fills(ui: &mut Ui) {
    if !ui.visuals().dark_mode {
        let background = editor_bg().blend(settings_card_fill());
        opaque_widget_fills(ui.visuals_mut(), background);
    }
}

// Chimp's JSON view.

/// Syntax colours for JSON text in Chimp's documents.
pub(in crate::app) struct JsonSyntaxColors {
    pub(in crate::app) plain: Color32,
    pub(in crate::app) key: Color32,
    pub(in crate::app) string: Color32,
    pub(in crate::app) path: Color32,
    pub(in crate::app) number: Color32,
    pub(in crate::app) literal: Color32,
    pub(in crate::app) punctuation: Color32,
}

pub(in crate::app) fn json_syntax_colors() -> JsonSyntaxColors {
    use themes::Token as T;
    JsonSyntaxColors {
        plain: themes::color(T::SyntaxPlain),
        key: themes::color(T::SyntaxKey),
        string: themes::color(T::SyntaxString),
        path: themes::color(T::SyntaxPath),
        number: themes::color(T::SyntaxNumber),
        literal: themes::color(T::SyntaxLiteral),
        punctuation: themes::color(T::SyntaxPunctuation),
    }
}

// Bitmap channel buttons.

/// A bitmap channel toggle when on: its 40% fill and its icon colour. The
/// premultiplied values are deliberate: `from_rgba_unmultiplied` converts
/// saturated colours through linear space, which looks much brighter than a
/// restrained UI fill.
pub(in crate::app) fn bitmap_channel_colors(icon: ButtonIcon) -> (Color32, Color32) {
    match icon {
        ButtonIcon::ChannelRed => (
            Color32::from_rgba_premultiplied(102, 0, 0, 102),
            Color32::from_rgb(255, 0, 0),
        ),
        ButtonIcon::ChannelGreen => (
            Color32::from_rgba_premultiplied(0, 102, 0, 102),
            Color32::from_rgb(0, 255, 0),
        ),
        ButtonIcon::ChannelBlue => (
            Color32::from_rgba_premultiplied(0, 0, 102, 102),
            Color32::from_rgb(0, 0, 255),
        ),
        ButtonIcon::ChannelAlpha if is_dark_mode() => (
            Color32::from_rgba_premultiplied(102, 102, 102, 102),
            Color32::WHITE,
        ),
        ButtonIcon::ChannelAlpha => (
            Color32::from_rgba_premultiplied(0, 0, 0, 102),
            Color32::BLACK,
        ),
        _ => (Color32::TRANSPARENT, Color32::TRANSPARENT),
    }
}

// Statuses. Every error, warning and success message shares one of three
// colours, invalid_value_text, unapplied_value_text and good_news, each
// legible in both themes; these names say where each is used.
// Warnings that something will be lost or overwritten were red before the
// colours were shared, so they keep the error red rather than amber.

/// Warnings in the import, compatibility and cache windows.
pub(in crate::app) fn import_warning_text() -> Color32 {
    themes::color(themes::Token::StatusWarning)
}

/// Notes that an action will replace or lose something: mods, Chimp,
/// closing documents, container dumps.
pub(in crate::app) fn replace_warning_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// Errors in Chimp's property editor and save dialog.
pub(in crate::app) fn chimp_error_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// A save or close that failed, and a missing source folder.
pub(in crate::app) fn save_error_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// Saving over the installed game's containers.
pub(in crate::app) fn installed_overwrite_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// Errors in Chimp's document view and the Model Preview's panels.
pub(in crate::app) fn panel_error_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// A bitmap that could not be shown.
pub(in crate::app) fn bitmap_error_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// The banner across a tag whose file changed on disk while it had unsaved
/// edits: an amber wash behind the pane's own text colour.
pub(in crate::app) fn disk_banner_fill() -> Color32 {
    themes::color(themes::Token::BannerWarningBackground)
}

/// The changed-on-disk banner's border.
pub(in crate::app) fn disk_banner_edge() -> Color32 {
    themes::color(themes::Token::BannerWarningBorder)
}

/// The changed-on-disk banner's warning icon.
pub(in crate::app) fn disk_banner_icon() -> Color32 {
    themes::color(themes::Token::BannerWarningIcon)
}

/// The banner across a tag whose file was deleted: a red wash behind the
/// pane's own text colour.
pub(in crate::app) fn disk_missing_fill() -> Color32 {
    themes::color(themes::Token::BannerErrorBackground)
}

/// The deleted-file banner's border.
pub(in crate::app) fn disk_missing_edge() -> Color32 {
    themes::color(themes::Token::BannerErrorBorder)
}

/// The deleted-file banner's warning icon, and the dot on its tag's tab.
pub(in crate::app) fn disk_missing_icon() -> Color32 {
    themes::color(themes::Token::BannerErrorIcon)
}

/// Read-only and missing notes in Chimp's headers and the New Tag window.
pub(in crate::app) fn header_notice_text() -> Color32 {
    themes::color(themes::Token::StatusWarning)
}

/// A browser filter that cannot match as typed.
pub(in crate::app) fn filter_warning_text() -> Color32 {
    themes::color(themes::Token::StatusWarning)
}

/// Settings that need attention.
pub(in crate::app) fn settings_warning_text() -> Color32 {
    themes::color(themes::Token::StatusWarning)
}

/// Expert mode's note in the reference picker.
pub(in crate::app) fn expert_mode_text() -> Color32 {
    themes::color(themes::Token::StatusWarning)
}

/// A drop target that will take what is dragged over it, and a search's
/// result count.
pub(in crate::app) fn accept_text() -> Color32 {
    themes::color(themes::Token::StatusSuccess)
}

/// The Add icon on a shortcut row.
pub(in crate::app) fn add_icon() -> Color32 {
    themes::color(themes::Token::StatusSuccess)
}

/// Errors and archive names in Chimp's browser.
pub(in crate::app) fn chimp_browser_error_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

pub(in crate::app) fn chimp_archive_text() -> Color32 {
    themes::color(themes::Token::ChimpArchiveText)
}

/// Untyped bytes Chimp keeps read-only.
pub(in crate::app) fn chimp_raw_value_text() -> Color32 {
    themes::color(themes::Token::StatusWarning)
}

/// Errors in the git review window.
pub(in crate::app) fn review_error_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// Errors in the first-run window.
pub(in crate::app) fn first_run_error_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// A reference to a tag that is not in the kit.
pub(in crate::app) fn reference_missing_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// An error message in a window.
pub(in crate::app) fn error_text() -> Color32 {
    themes::color(themes::Token::StatusError)
}

/// How long a status message stays on the status line before it clears.
/// Without this the last thing that happened sits there indefinitely, which
/// reads as current state long after it stopped being true.
pub(in crate::app) const STATUS_LINGER_SECS: f64 = 5.0;

pub(in crate::app) const FOUNDATION_LABEL_WIDTH: f32 = 280.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// Rename and Import pass a right-aligned row directly to the footer;
    /// its height must follow the buttons, even in a tall popup.
    #[test]
    fn popup_footer_action_rows_do_not_fill_remaining_window_height() {
        for height in [240.0, 480.0, 720.0] {
            let ctx = egui::Context::default();
            crate::app::Baboon::configure_context(&ctx);
            let mut geometry = None;
            for _ in 0..3 {
                let _ = crate::app::run_ui_test(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(1200.0, 1000.0),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::Window::new("Footer regression")
                            .title_bar(false)
                            .default_size(Vec2::new(480.0, height))
                            .show(root.ctx(), |ui| {
                                ui.label("Popup body");
                                let mut button = egui::Rect::NOTHING;
                                let footer = popup_footer(ui, |ui| {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            button = ui.button("Cancel").rect;
                                            let _ = ui.button("Rename");
                                        },
                                    );
                                });
                                geometry = Some((footer.response.rect, button));
                            });
                    },
                );
            }
            let (footer, button) = geometry.expect("popup rendered");
            assert!(
                footer.height() <= button.height() + 16.0,
                "window height {height}: footer height {} for a {}-point button",
                footer.height(),
                button.height()
            );
        }
    }

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
        with_built_in(themes::BuiltIn::Dark, || {
            let widgets = foundation_visuals().widgets;
            assert_eq!(widgets.inactive.weak_bg_fill.to_array(), [26, 26, 26, 26]);
            assert_eq!(widgets.hovered.weak_bg_fill.to_array(), [51, 51, 51, 51]);
        });
        // The classic light theme: black at the same opacity.
        with_built_in(themes::BuiltIn::ClassicLight, || {
            assert_eq!(
                foundation_visuals().widgets.inactive.weak_bg_fill,
                Color32::from_black_alpha(26)
            );
        });
        // The warm light theme's menus hover in a warm brown, not black.
        with_built_in(themes::BuiltIn::Light, || {
            let hover = context_menu_hover();
            assert_eq!(hover.a(), 26);
            assert!(hover.r() > hover.g() && hover.g() > hover.b(), "{hover:?}");
        });
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

    const THEMES: [themes::BuiltIn; 3] = themes::BuiltIn::ALL;

    /// The classic light theme is main's grey look: its panels, pages and
    /// navy accent are the ones Baboon had before the warm theme.
    #[test]
    fn the_classic_light_theme_keeps_the_grey_look() {
        with_built_in(themes::BuiltIn::ClassicLight, || {
            assert_eq!(editor_bg(), Color32::from_rgb(224, 224, 220));
            assert_eq!(left_panel(), Color32::from_rgb(238, 238, 234));
            assert_eq!(row_type(), Color32::from_rgb(219, 219, 216));
            assert_eq!(menu_bar(), row_type(), "the status bar matches the top bar");
            assert_eq!(chrome_bar(), row_type());
            assert_eq!(foundation_blue(), Color32::from_rgb(15, 43, 64));
            assert_eq!(
                foundation_visuals().widgets.inactive.weak_bg_fill,
                Color32::from_black_alpha(26)
            );
            // Blue highlights rather than orange.
            let border = focused_tab_border();
            assert!(border.b() > border.r(), "{border:?}");
        });
        with_built_in(themes::BuiltIn::Light, || {
            assert_eq!(editor_bg(), Color32::from_rgb(242, 241, 240));
            assert_eq!(chrome_bar(), menu_bar());
        });
    }

    /// Selected text reads on the selection in every theme. The classic
    /// theme's old #4B7D9B selection held dark text at about 4:1.
    #[test]
    fn selected_text_reads_on_the_selection_in_every_theme() {
        for built_in in THEMES {
            with_built_in(built_in, || {
                let ratio = contrast(text_dark(), foundation_visuals().selection.bg_fill);
                assert!(ratio >= 4.5, "{built_in:?}: {ratio:.2}");
            });
        }
        let classic_text = with_built_in(themes::BuiltIn::ClassicLight, text_dark);
        assert!(contrast(classic_text, Color32::from_rgb(75, 125, 155)) < 4.5);
    }

    /// Error, warning and success messages read at 4.5:1 on the pages,
    /// panels and status bar they sit on, in every theme. The warm status
    /// bar held the success green at 3.8:1.
    #[test]
    fn status_colours_read_on_every_theme_background() {
        for built_in in THEMES {
            with_built_in(built_in, || {
                for (name, colour) in [
                    ("error", invalid_value_text()),
                    ("warning", unapplied_value_text()),
                    ("success", good_news()),
                ] {
                    for (place, backdrop) in [
                        ("page", editor_bg()),
                        ("panel", left_panel()),
                        ("group", foundation_group_bg()),
                        ("status bar", menu_bar()),
                    ] {
                        let ratio = contrast(colour, backdrop);
                        assert!(
                            ratio >= 4.5,
                            "{name} on the {place}, {built_in:?}: {ratio:.2}"
                        );
                    }
                }
            });
        }
    }

    /// An invalid value's box keeps its text readable in either theme, and
    /// the part marked wrong stands out on it. Foundation's pink under the
    /// dark theme's light text read at about 1.2:1.
    #[test]
    fn invalid_values_stay_readable_in_both_themes() {
        for built_in in THEMES {
            with_built_in(built_in, || {
                let fill = invalid_value_fill();
                assert!(
                    contrast(text_dark(), fill) >= 7.0,
                    "text on the fill, {built_in:?}"
                );
                assert!(
                    contrast(invalid_value_text(), fill) >= 4.5,
                    "marked text on the fill, {built_in:?}"
                );
            });
        }
        let dark_text = with_built_in(themes::BuiltIn::Dark, text_dark);
        assert!(contrast(dark_text, Color32::from_rgb(0xFF, 0xC0, 0xCB)) < 1.5);
        let light_fill = with_built_in(themes::BuiltIn::Light, invalid_value_fill);
        assert_eq!(
            light_fill,
            Color32::from_rgb(255, 240, 240),
            "subtle red tint"
        );
    }

    #[test]
    fn selection_colors_are_lighter_with_a_dark_border_in_light_mode() {
        with_built_in(themes::BuiltIn::Dark, || {
            assert_eq!(
                foundation_visuals().selection.bg_fill,
                Color32::from_rgb(64, 108, 134)
            );
        });
        with_built_in(themes::BuiltIn::Light, || {
            let selection = foundation_visuals().selection;
            assert_eq!(selection.bg_fill, Color32::from_rgb(146, 192, 226));
            assert_eq!(selection.stroke.color, Color32::from_rgb(38, 63, 78));
        });
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
        let active = with_built_in(themes::BuiltIn::Dark, active_tab);
        assert_eq!(active, Color32::from_rgb(72, 72, 72));
        assert!(active.r() > 55, "document/Chimp inactive gray");
        assert!(active.r() > 31, "workspace inactive gray");
    }

    #[test]
    fn light_active_tab_keeps_the_existing_menu_bar_color() {
        with_built_in(themes::BuiltIn::Light, || {
            assert_eq!(active_tab(), Color32::from_rgb(229, 204, 178));
        });
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

/// A keyword pill's colours, from its Manage Keywords colour.
pub(in crate::app) struct KeywordPillColours {
    pub(in crate::app) fill: Color32,
    pub(in crate::app) border: Option<Color32>,
    pub(in crate::app) dot: Option<Color32>,
    pub(in crate::app) text: Color32,
}

/// A keyword pill's colours: its colour tints the fill and border and
/// colours the dot, a darker version of each colour for dark mode. No colour
/// is grey, so every pill has the same dot and border.
pub(in crate::app) fn keyword_pill_colours(
    colour: Option<crate::core::keyword_catalog::KeywordColour>,
) -> KeywordPillColours {
    use crate::core::keyword_catalog::KeywordColour as K;
    let mix = |base: Color32, over: Color32, amount: f32| {
        let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
        Color32::from_rgb(
            channel(base.r(), over.r()),
            channel(base.g(), over.g()),
            channel(base.b(), over.b()),
        )
    };
    let background = editor_bg();
    let colour = colour.unwrap_or(K::Grey);
    let dark = is_dark_mode();
    let hue = match (colour, dark) {
        (K::Red, false) => Color32::from_rgb(238, 120, 120),
        (K::Orange, false) => Color32::from_rgb(238, 170, 112),
        (K::Yellow, false) => Color32::from_rgb(230, 208, 104),
        (K::Green, false) => Color32::from_rgb(112, 212, 112),
        (K::Teal, false) => Color32::from_rgb(104, 214, 186),
        (K::Blue, false) => Color32::from_rgb(120, 170, 238),
        (K::Purple, false) => Color32::from_rgb(170, 136, 238),
        (K::Pink, false) => Color32::from_rgb(236, 128, 200),
        (K::Grey, false) => Color32::from_rgb(172, 172, 172),
        (K::Red, true) => Color32::from_rgb(204, 72, 72),
        (K::Orange, true) => Color32::from_rgb(204, 128, 64),
        (K::Yellow, true) => Color32::from_rgb(192, 170, 64),
        (K::Green, true) => Color32::from_rgb(72, 172, 72),
        (K::Teal, true) => Color32::from_rgb(64, 172, 140),
        (K::Blue, true) => Color32::from_rgb(72, 132, 204),
        (K::Purple, true) => Color32::from_rgb(132, 98, 204),
        (K::Pink, true) => Color32::from_rgb(192, 72, 142),
        (K::Grey, true) => Color32::from_rgb(128, 128, 128),
    };
    if dark {
        KeywordPillColours {
            fill: mix(background, hue, 0.32),
            border: Some(mix(background, hue, 0.58)),
            dot: Some(hue),
            text: mix(Color32::from_rgb(240, 240, 240), hue, 0.18),
        }
    } else {
        KeywordPillColours {
            fill: mix(background, hue, 0.38),
            border: Some(mix(hue, Color32::from_rgb(96, 88, 84), 0.45)),
            dot: Some(hue),
            text: mix(Color32::from_rgb(44, 40, 38), hue, 0.22),
        }
    }
}
