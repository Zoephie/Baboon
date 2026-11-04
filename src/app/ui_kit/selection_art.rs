//! Shared SVG artwork, theme palette and native-glyph suppression for selection controls.
use super::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) enum SelectionState {
    Unchecked,
    Checked,
    Indeterminate,
    RadioUnchecked,
    RadioChecked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Palette {
    state: SelectionState,
    dark: bool,
    hovered: bool,
    foreground: Color32,
    accent: Color32,
}

fn svg(palette: Palette) -> String {
    let source = match palette.state {
        SelectionState::Unchecked => {
            include_root_str!("assets/Button Icons/Checkbox-Unchecked.svg")
        }
        SelectionState::Checked => include_root_str!("assets/Button Icons/Checkbox-Checked.svg"),
        SelectionState::Indeterminate => {
            include_root_str!("assets/Button Icons/Checkbox-Indeterminate.svg")
        }
        SelectionState::RadioUnchecked => {
            include_root_str!("assets/Button Icons/Radio-Unfilled.svg")
        }
        SelectionState::RadioChecked => include_root_str!("assets/Button Icons/Radio-Filled.svg"),
    };
    // Match the checkbox's selected well while retaining the supplied radio geometry.
    let source = if palette.state == SelectionState::RadioChecked {
        source.replacen(
            "fill=\"currentColor\" fill-opacity=\"0.5\"",
            "fill=\"black\" fill-opacity=\"0.5\"",
            1,
        )
    } else {
        source.to_owned()
    };
    let colour = |c: Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
    let mut source = source
        .replace("currentColor", &colour(palette.foreground))
        .replace("#47AEEB", &colour(palette.accent));
    if !palette.dark {
        source = source.replace("fill=\"black\"", "fill=\"white\"");
    }
    if palette.hovered {
        source = source
            .replace(
                "opacity=\"0.2\"",
                if matches!(
                    palette.state,
                    SelectionState::Unchecked | SelectionState::RadioUnchecked
                ) {
                    "opacity=\"0.75\""
                } else {
                    "opacity=\"0.5\""
                },
            )
            .replace("opacity=\"0.4\"", "opacity=\"0.7\"")
            .replace("opacity=\"0.8\"", "opacity=\"1\"");
    } else if matches!(
        palette.state,
        SelectionState::Checked | SelectionState::RadioChecked
    ) {
        // Only the outer border softens; the checkmark stays fully legible.
        source = source.replacen("<path ", "<path opacity=\"0.5\" ", 1);
    }
    source
}

fn image(palette: Palette) -> egui::Image<'static> {
    thread_local! {
        static SVGS: RefCell<HashMap<Palette, (String, Arc<[u8]>)>> = RefCell::new(HashMap::new());
    }
    let (uri, bytes) = SVGS.with(|cache| {
        cache
            .borrow_mut()
            .entry(palette)
            .or_insert_with(|| {
                (
                    format!(
                        "bytes://baboon_selection_controls/{:?}-{}-{}-{:08x}-{:08x}.svg",
                        palette.state,
                        palette.dark,
                        palette.hovered,
                        u32::from_le_bytes(palette.foreground.to_array()),
                        u32::from_le_bytes(palette.accent.to_array())
                    ),
                    svg(palette).into_bytes().into(),
                )
            })
            .clone()
    });
    egui::Image::from_bytes(uri, egui::load::Bytes::Shared(bytes))
        .fit_to_exact_size(Vec2::splat(16.0))
}

/// Shared artwork for native-layout widgets and custom allocated checkbox rows.
pub(in crate::app) fn paint_selection_control(
    ui: &Ui,
    rect: egui::Rect,
    state: SelectionState,
    hovered: bool,
) {
    image(Palette {
        state,
        dark: ui.visuals().dark_mode,
        hovered,
        foreground: ui.ctx().global_style().visuals.text_color(),
        accent: ui.ctx().global_style().visuals.selection.bg_fill,
    })
    .paint_at(ui, rect);
}

/// Preserve native input, text and accessibility while suppressing only its glyph paint.
pub(in crate::app) fn native_selection_response(
    ui: &mut Ui,
    add_native: impl FnOnce(&mut Ui) -> egui::Response,
) -> egui::Response {
    let original = ui.style().clone();
    // Leave the native glyph's space in the layout and suppress its paint.
    // Preserve text colour explicitly while hiding only the glyph strokes.
    {
        let style = ui.style_mut();
        style.spacing.icon_width = 16.0;
        style.visuals.override_text_color = Some(original.visuals.text_color());
        for widget in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            widget.bg_fill = Color32::TRANSPARENT;
            widget.bg_stroke = Stroke::NONE;
            widget.fg_stroke = Stroke::NONE;
        }
    }
    let response = add_native(ui);
    ui.set_style(original);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(state: SelectionState, dark: bool, hovered: bool) -> Palette {
        Palette {
            state,
            dark,
            hovered,
            foreground: if dark { Color32::WHITE } else { Color32::BLACK },
            accent: Color32::from_rgb(64, 108, 134),
        }
    }
    fn raster(palette: Palette) -> egui::ColorImage {
        egui_extras::image::load_svg_bytes(svg(palette).as_bytes(), &Default::default()).unwrap()
    }

    #[test]
    fn selection_art_rasterizes_all_states_and_hover_strengthens_the_border() {
        for dark in [false, true] {
            for state in [
                SelectionState::Unchecked,
                SelectionState::Checked,
                SelectionState::Indeterminate,
                SelectionState::RadioUnchecked,
                SelectionState::RadioChecked,
            ] {
                let normal = raster(palette(state, dark, false));
                let hover = raster(palette(state, dark, true));
                assert_eq!(normal.size, [16, 16]);
                assert!(normal.pixels.iter().any(|pixel| pixel.a() > 0));
                assert!(
                    hover[(0, 8)].a() > normal[(0, 8)].a(),
                    "hover border: {state:?}, dark={dark}"
                );
            }
        }
        let light = raster(palette(SelectionState::Checked, false, false));
        let dark = raster(palette(SelectionState::Checked, true, false));
        assert!(
            light[(1, 8)].r() > dark[(1, 8)].r(),
            "the translucent well reverses for light mode"
        );
    }

    #[test]
    fn selection_images_load_through_the_apps_svg_loader() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        for dark in [false, true] {
            for hovered in [false, true] {
                for state in [
                    SelectionState::Unchecked,
                    SelectionState::Checked,
                    SelectionState::Indeterminate,
                    SelectionState::RadioUnchecked,
                    SelectionState::RadioChecked,
                ] {
                    let image = image(palette(state, dark, hovered));
                    let loaded = image.load_for_size(&ctx, Vec2::splat(16.0));
                    assert!(
                        matches!(loaded, Ok(egui::load::TexturePoll::Ready { .. })),
                        "checkbox image must load in the app: {:?}, URI={:?}",
                        loaded.as_ref().err(),
                        image.uri()
                    );
                }
            }
        }
    }

    #[test]
    fn unselected_hover_uses_the_same_stronger_border_without_changing_fill() {
        for state in [SelectionState::Unchecked, SelectionState::RadioUnchecked] {
            let svg = svg(palette(state, true, true));
            assert!(svg.contains("opacity=\"0.75\""));
            assert!(svg.contains("opacity=\"0.1\""));
        }
    }
}
