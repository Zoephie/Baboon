//! Shared focus outlines for floating windows and the tiled workspace below them.
use super::*;

#[derive(Clone, Default)]
struct WindowFocus {
    windows: Vec<egui::LayerId>,
    focused: Option<egui::LayerId>,
    keyboard: Option<egui::Id>,
    return_focus: HashMap<egui::LayerId, Option<egui::LayerId>>,
}

fn state_id() -> egui::Id { egui::Id::unique("baboon_window_focus") }

pub(in crate::app) fn workspace_has_focus(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<WindowFocus>(state_id()))
        .is_none_or(|state| state.focused.is_none())
}

pub(in crate::app) fn install_focus_outlines(ctx: &egui::Context) {
    ctx.on_end_pass("baboon_focus_outlines", Arc::new(|ui| paint_window_focus(ui.ctx())));
}

/// Locate the area's own frame, including the Vec containing its shadow.
/// Matching its outer bounds excludes controls and nested content frames.
fn window_frame(shape: &egui::Shape, outer: egui::Rect) -> Option<(egui::Rect, egui::CornerRadius)> {
    match shape {
        egui::Shape::Rect(rect) if rect.stroke.width > 0.0
            && rect.rect.min.distance(outer.min) < 1.0
            && rect.rect.max.distance(outer.max) < 1.0 => Some((rect.rect, rect.corner_radius)),
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| window_frame(shape, outer)),
        _ => None,
    }
}

fn paint_window_focus(ctx: &egui::Context) {
    let (layers, modal, keyboard) = ctx.memory(|memory| (
        memory.layer_ids().filter(|layer| memory.areas().is_visible(layer)).collect::<Vec<_>>(),
        memory.top_modal_layer(), memory.focused(),
    ));
    let windows: Vec<_> = layers.into_iter()
        .filter(|layer| layer.order == egui::Order::Middle || Some(*layer) == modal)
        .filter_map(|layer| {
            let outer = ctx.memory(|memory| memory.area_rect(layer.id))?;
            ctx.graphics(|graphics| graphics.get(layer)?.all_entries()
                .find_map(|entry| window_frame(&entry.shape, outer)))
                .map(|(rect, corners)| (layer, rect, corners))
        }).collect();
    let mut state = ctx.data(|data| data.get_temp::<WindowFocus>(state_id())).unwrap_or_default();
    let previous = state.focused;
    let contains = |layer| windows.iter().any(|(id, _, _)| *id == layer);
    if let Some((layer, _, _)) = windows.iter().rev().find(|(layer, _, _)| !state.windows.contains(layer)) {
        state.return_focus.insert(*layer, state.focused);
        state.focused = Some(*layer);
    } else if state.focused.is_some_and(|layer| !contains(layer)) {
        state.focused = state.return_focus.remove(&state.focused.unwrap()).flatten();
        while let Some(layer) = state.focused.filter(|layer| !contains(*layer)) {
            state.focused = state.return_focus.remove(&layer).flatten();
        }
    }
    // read_response also processes pointer interaction: reading a newly
    // focused search field on the popup-opening click surrenders its focus
    // because that click was outside the field. Defer this observation until
    // the next pass; the pointer below already identifies the clicked surface.
    let defer_keyboard = keyboard != state.keyboard
        && ctx.input(|input| input.pointer.any_pressed() || input.pointer.any_click());
    if keyboard != state.keyboard && !defer_keyboard
        && let Some(response) = keyboard.and_then(|id| ctx.read_response(id))
    {
        if contains(response.layer_id) { state.focused = Some(response.layer_id); }
        else if response.layer_id.order == egui::Order::Background { state.focused = None; }
    }
    if let Some(position) = ctx.input(|input| input.pointer.any_pressed().then(|| input.pointer.interact_pos()).flatten())
        && let Some(layer) = ctx.layer_id_at(position)
    {
        if contains(layer) { state.focused = Some(layer); }
        // A menu belongs to the surface which opened it; opening or clicking
        // one must not transfer focus to the workspace behind it.
        else if layer.order == egui::Order::Background { state.focused = None; }
    }
    if modal.is_some_and(contains) { state.focused = modal; }
    if let Some((layer, rect, corners)) = windows.iter().find(|(layer, _, _)| Some(*layer) == state.focused) {
        ctx.layer_painter(*layer).rect_stroke(*rect, *corners,
            Stroke::new(1.0, focused_tab_border(ctx.global_style().visuals.dark_mode)), egui::StrokeKind::Inside);
    }
    state.windows = windows.iter().map(|(layer, _, _)| *layer).collect();
    state.return_focus.retain(|layer, _| contains(*layer));
    if !defer_keyboard { state.keyboard = keyboard; }
    if previous != state.focused || defer_keyboard { ctx.request_repaint(); }
    ctx.data_mut(|data| data.insert_temp(state_id(), state));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, modal: bool) -> egui::FullOutput {
        crate::app::run_ui_test(ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 700.0))),
            focused: true, events, ..Default::default()
        }, |ui| {
            for (title, x) in [("Find", 20.0), ("Compare Tags", 400.0)] {
                egui::Window::new(title).id(egui::Id::unique(title))
                    .title_bar(false).resizable(false).movable(false).fade_in(false)
                    .fixed_pos(egui::pos2(x, 20.0)).fixed_size(Vec2::new(250.0, 180.0))
                    .show(ui.ctx(), |ui| { ui.label(title); });
            }
            if modal {
                egui::Modal::new(egui::Id::unique("Confirm"))
                    .frame(egui::Frame::window(ui.style()))
                    .show(ui.ctx(), |ui| { ui.set_width(200.0); ui.label("Confirm"); });
            }
        })
    }

    fn focused(ctx: &egui::Context) -> Option<egui::Id> {
        ctx.data(|data| data.get_temp::<WindowFocus>(state_id())).unwrap().focused.map(|layer| layer.id)
    }

    #[test]
    fn floating_window_focus_follows_clicks_and_modals_in_both_themes() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = egui::Context::default();
            Baboon::configure_context(&ctx);
            ctx.set_theme(theme);
            let mut output = frame(&ctx, vec![], false);
            for _ in 0..3 { output = frame(&ctx, vec![], false); }
            assert_eq!(focused(&ctx), Some(egui::Id::unique("Compare Tags")));
            let border = focused_tab_border(theme == egui::Theme::Dark);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.stroke.color == border)), "the focused window has an emphasized border");
            let find_center = ctx.read_response(egui::Id::unique("Find")).unwrap().rect.center();
            frame(&ctx, vec![egui::Event::PointerMoved(find_center)], false);
            assert_eq!(focused(&ctx), Some(egui::Id::unique("Compare Tags")), "hovering never changes focus");
            for (position, expected) in [
                (find_center, Some(egui::Id::unique("Find"))),
                (egui::pos2(900.0, 600.0), None),
            ] {
                frame(&ctx, vec![egui::Event::PointerMoved(position)], false);
                for pressed in [true, false] {
                    frame(&ctx, vec![egui::Event::PointerButton {
                        pos: position, button: egui::PointerButton::Primary, pressed,
                        modifiers: Default::default(),
                    }], false);
                }
                assert_eq!(focused(&ctx), expected);
                assert_eq!(workspace_has_focus(&ctx), expected.is_none());
            }
            for _ in 0..3 { frame(&ctx, vec![], true); }
            assert_eq!(focused(&ctx), Some(egui::Id::unique("Confirm")), "a modal owns the focus outline");
            for _ in 0..3 { frame(&ctx, vec![], false); }
            assert!(workspace_has_focus(&ctx), "closing the modal returns focus to its workspace caller");
        }
    }
}
