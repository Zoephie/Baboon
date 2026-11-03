//! Shared close artwork for tabs and custom window headers.
use super::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

fn close_svg(color: Color32, hovered: bool) -> String {
    let source = include_root_str!("assets/Button Icons/Close.svg");
    let source = if hovered {
        source.to_owned()
    } else {
        source
            .replace(
                "id=\"hover-background\" opacity=\"0.25\"",
                "id=\"hover-background\" opacity=\"0\"",
            )
            .replace(
                "id=\"close-mark\" opacity=\"1\"",
                "id=\"close-mark\" opacity=\"0.5\"",
            )
    };
    source.replace(
        "currentColor",
        &format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b()),
    )
}

/// Paint within the existing hitbox, preserving its input and accessibility.
pub(in crate::app) fn paint_close_control(ui: &Ui, response: &egui::Response) {
    thread_local! {
        static SVGS: RefCell<HashMap<(Color32, bool), (String, Arc<[u8]>)>> = RefCell::new(HashMap::new());
    }
    let color = if ui.visuals().dark_mode {
        Color32::WHITE
    } else {
        Color32::BLACK
    };
    let hovered = response.enabled()
        && (response.hovered() || response.has_focus() || response.is_pointer_button_down_on());
    let (uri, bytes) = SVGS.with(|cache| {
        cache
            .borrow_mut()
            .entry((color, hovered))
            .or_insert_with(|| {
                (
                    format!(
                        "bytes://baboon_close/{:08x}-{hovered}.svg",
                        u32::from_le_bytes(color.to_array())
                    ),
                    close_svg(color, hovered).into_bytes().into(),
                )
            })
            .clone()
    });
    let size = 16.0_f32
        .min(response.rect.width())
        .min(response.rect.height());
    let rect = egui::Rect::from_center_size(response.rect.center(), Vec2::splat(size));
    egui::Image::from_bytes(uri, egui::load::Bytes::Shared(bytes))
        .fit_to_exact_size(rect.size())
        .tint(if response.enabled() {
            Color32::WHITE
        } else {
            Color32::from_white_alpha(128)
        })
        .paint_at(ui, rect);
}

pub(in crate::app) fn close_button(ui: &mut Ui, tooltip: &str) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
    let response = ui
        .interact(rect, ui.auto_id_with("close"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::Default)
        .on_hover_text(tooltip);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    paint_close_control(ui, &response);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_art_preserves_the_supplied_geometry_and_state_opacities_in_both_themes() {
        for color in [Color32::WHITE, Color32::BLACK] {
            let raster = |hovered| {
                egui_extras::image::load_svg_bytes(
                    close_svg(color, hovered).as_bytes(),
                    &Default::default(),
                )
                .unwrap()
            };
            let idle = raster(false);
            let hover = raster(true);
            assert_eq!(idle.size, [16, 16]);
            assert_eq!(hover.size, [16, 16]);
            // Inside the circle, away from the cross and antialiased edges.
            assert_eq!(idle[(8, 2)].a(), 0);
            assert!((60..=66).contains(&hover[(8, 2)].a()));
            // The narrow diagonal edges are antialiased at the asset's
            // native resolution; no idle pixel may exceed half opacity.
            let idle_peak = idle.pixels.iter().map(|pixel| pixel.a()).max().unwrap();
            let hover_peak = hover.pixels.iter().map(|pixel| pixel.a()).max().unwrap();
            assert!((100..=128).contains(&idle_peak));
            assert!(hover_peak > 220);
        }
    }
}
