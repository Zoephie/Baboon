//! Middle-click autoscroll, shared by ordinary scroll areas and table bodies.
use super::*;

#[derive(Clone)]
struct Target {
    id: egui::Id,
    rect: egui::Rect,
    axes: egui::Vec2b,
    limit: Vec2,
    state: egui::scroll_area::State,
    hovered: bool,
}

#[derive(Clone)]
struct Session {
    owner: egui::Id,
    origin: egui::Pos2,
    moved: bool,
    time: f64,
}

fn targets_id() -> egui::Id { egui::Id::new("autoscroll_targets") }
fn session_id() -> egui::Id { egui::Id::new("autoscroll_session") }
fn cancelled_id() -> egui::Id { session_id().with("cancelled_pass") }

pub(in crate::app) fn register<R>(ui: &Ui, output: &egui::scroll_area::ScrollAreaOutput<R>, axes: egui::Vec2b) {
    if !ui.is_enabled() || !ui.is_visible() { return; }
    let rect = output.inner_rect.intersect(ui.clip_rect());
    let limit = (output.content_size - output.inner_rect.size()).max(Vec2::ZERO);
    let axes = egui::Vec2b::new(axes.x && limit.x > 0.5, axes.y && limit.y > 0.5);
    if !rect.is_positive() || !axes.any() { return; }
    let target = Target { id: output.id, rect, axes, limit, state: output.state,
        hovered: ui.rect_contains_pointer(rect) };
    ui.data_mut(|data| {
        let mut targets = data.get_temp::<Vec<Target>>(targets_id()).unwrap_or_default();
        targets.push(target);
        data.insert_temp(targets_id(), targets);
    });
}

pub(in crate::app) fn install(ctx: &egui::Context) {
    ctx.on_begin_pass("baboon_autoscroll_input", Arc::new(|ui| {
        let ctx = ui.ctx();
        ctx.data_mut(|data| data.remove::<Vec<Target>>(targets_id()));
        if ctx.data(|data| data.get_temp::<Session>(session_id())).is_none() { return; }
        let cancel = ctx.input(|input| !input.focused || input.pointer.hover_pos().is_none()
            || input.pointer.any_pressed() || input.key_pressed(egui::Key::Escape)
            || input.events.iter().any(|event| matches!(event,
                egui::Event::MouseWheel { .. } | egui::Event::Key { pressed: true, .. } | egui::Event::Text(_))));
        if cancel {
            let pass = ctx.cumulative_pass_nr();
            ctx.data_mut(|data| data.remove::<Session>(session_id()));
            ctx.data_mut(|data| data.insert_temp(cancelled_id(), pass));
            // A click which stops scrolling must not also activate a control.
            ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                input.events.retain(|event| !matches!(event, egui::Event::PointerButton { .. }));
            });
        }
    }));
    ctx.on_end_pass("baboon_autoscroll", Arc::new(|ui| update(ui.ctx())));
}

fn speed(distance: f32) -> f32 {
    let beyond = (distance.abs() - 10.0).max(0.0);
    distance.signum() * (beyond * 6.0 + beyond * beyond * 0.04).min(2400.0)
}

fn update(ctx: &egui::Context) {
    if ctx.data(|data| data.get_temp::<u64>(cancelled_id())) == Some(ctx.cumulative_pass_nr()) { return; }
    let targets = ctx.data(|data| data.get_temp::<Vec<Target>>(targets_id())).unwrap_or_default();
    let mut session = ctx.data(|data| data.get_temp::<Session>(session_id()));
    let (position, time, pressed, released, focused) = ctx.input(|input| (
        input.pointer.hover_pos(), input.time,
        input.pointer.button_pressed(egui::PointerButton::Middle),
        input.pointer.button_released(egui::PointerButton::Middle), input.focused,
    ));
    if session.is_none() && pressed && focused
        && let Some(origin) = position
        && let Some(target) = targets.iter().filter(|target| target.hovered)
            .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
    {
        session = Some(Session { owner: target.id, origin, moved: false, time });
    }
    let Some(mut session) = session else { return; };
    let target = targets.iter().find(|target| target.id == session.owner);
    let Some((target, position)) = target.zip(position).filter(|_| focused) else {
        ctx.data_mut(|data| data.remove::<Session>(session_id()));
        return;
    };
    let distance = position - session.origin;
    session.moved |= (target.axes.x && distance.x.abs() > 10.0)
        || (target.axes.y && distance.y.abs() > 10.0);
    if released && session.moved {
        ctx.data_mut(|data| data.remove::<Session>(session_id()));
        return;
    }
    let dt = ((time - session.time) as f32).clamp(0.0, 0.05);
    session.time = time;
    let velocity = Vec2::new(if target.axes.x { speed(distance.x) } else { 0.0 },
        if target.axes.y { speed(distance.y) } else { 0.0 });
    let mut state = target.state;
    state.offset = (state.offset + velocity * dt).clamp(Vec2::ZERO, target.limit);
    state.store(ctx, target.id);
    paint_anchor(ctx, session.origin, target.axes);
    ctx.set_cursor_icon(match (target.axes.x, target.axes.y) {
        (true, true) => egui::CursorIcon::AllScroll,
        (true, false) => egui::CursorIcon::ResizeHorizontal,
        _ => egui::CursorIcon::ResizeVertical,
    });
    ctx.data_mut(|data| data.insert_temp(session_id(), session));
    ctx.request_repaint();
}

/// Interactive widgets must be drawn outside egui's locked plugin callbacks.
/// Capture the stopping click instead of activating a control underneath.
pub(in crate::app) fn draw_blocker(ui: &mut Ui) {
    let ctx = ui.ctx();
    if ctx.data(|data| data.get_temp::<Session>(session_id())).is_none() { return; }
    egui::Area::new(session_id()).order(egui::Order::Tooltip)
        .fixed_pos(ctx.content_rect().min).fade_in(false).movable(false)
        .show(ctx, |ui| { ui.allocate_exact_size(ctx.content_rect().size(), Sense::click_and_drag()); });
}

fn paint_anchor(ctx: &egui::Context, origin: egui::Pos2, axes: egui::Vec2b) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, session_id()));
    let visuals = &ctx.global_style().visuals;
    let ink = visuals.text_color();
    painter.circle(origin, 13.0, visuals.window_fill(), Stroke::new(1.0, ink));
    painter.circle_filled(origin, 1.5, ink);
    for direction in [Vec2::UP, Vec2::DOWN, Vec2::LEFT, Vec2::RIGHT] {
        if (direction.x != 0.0 && !axes.x) || (direction.y != 0.0 && !axes.y) { continue; }
        let side = Vec2::new(-direction.y, direction.x);
        painter.add(egui::Shape::convex_polygon(vec![origin + direction * 10.0,
            origin + direction * 6.0 + side * 3.0,
            origin + direction * 6.0 - side * 3.0], ink, Stroke::NONE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Demo {
        ctx: egui::Context,
        time: f64,
        rects: Vec<egui::Rect>,
        offsets: Vec<Vec2>,
        axes: egui::Vec2b,
        nested: bool,
        table: bool,
        visible: bool,
        button: egui::Rect,
        clicks: usize,
    }

    impl Demo {
        fn new(axes: egui::Vec2b) -> Self {
            let ctx = egui::Context::default();
            install(&ctx);
            let mut demo = Self { ctx, time: 0.0, rects: Vec::new(), offsets: Vec::new(),
                axes, nested: false, table: false, visible: true,
                button: egui::Rect::NOTHING, clicks: 0 };
            demo.settle();
            demo
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 1.0 / 60.0;
            self.rects.clear();
            self.offsets.clear();
            let _ = crate::app::run_ui_test(&self.ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(700.0, 400.0))),
                time: Some(self.time), focused: true, events, ..Default::default()
            }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let button = ui.button("Do something");
                    self.button = button.rect;
                    self.clicks += usize::from(button.clicked());
                    if !self.visible { return; }
                    ui.columns(2, |columns| {
                        for (index, ui) in columns.iter_mut().enumerate() {
                            if self.table {
                                let output = egui_extras::TableBuilder::new(ui).id_salt(index)
                                    .max_scroll_height(250.0).column(egui_extras::Column::remainder())
                                    .header(24.0, |mut header| { header.col(|ui| { ui.label("Header"); }); })
                                    .body(|body| body.rows(24.0, 1000, |mut row| {
                                        let index = row.index();
                                        row.col(|ui| { ui.label(format!("Row {index}")); });
                                    }));
                                register(ui, &output, egui::Vec2b::new(false, true));
                                self.rects.push(output.inner_rect);
                                self.offsets.push(output.state.offset);
                                continue;
                            }
                            let area = if self.axes == egui::Vec2b::TRUE {
                                crate::app::ScrollArea::both()
                            } else if self.axes.x { crate::app::ScrollArea::horizontal() }
                            else { crate::app::ScrollArea::vertical() };
                            let output = area.id_salt(index).max_height(300.0).auto_shrink([false, false])
                                .show(ui, |ui| {
                                    if self.nested {
                                        let inner = crate::app::ScrollArea::vertical().id_salt(("inner", index)).max_height(120.0)
                                            .show_rows(ui, 20.0, 1000, |ui, rows| {
                                                for index in rows { ui.label(format!("Row {index}")); }
                                            });
                                        self.rects.push(inner.inner_rect);
                                        self.offsets.push(inner.state.offset);
                                    }
                                    ui.allocate_space(Vec2::new(if self.axes.x { 2000.0 } else { 100.0 }, 3000.0));
                                });
                            self.rects.push(output.inner_rect);
                            self.offsets.push(output.state.offset);
                        }
                    });
                });
                draw_blocker(ui);
            });
        }

        fn settle(&mut self) { for _ in 0..8 { self.frame(vec![]); } }
        fn pointer(&mut self, at: egui::Pos2, button: egui::PointerButton, pressed: bool) {
            self.frame(vec![egui::Event::PointerMoved(at), egui::Event::PointerButton {
                pos: at, button, pressed, modifiers: Default::default(),
            }]);
        }
        fn click(&mut self, at: egui::Pos2, button: egui::PointerButton) {
            self.pointer(at, button, true);
            self.pointer(at, button, false);
            self.settle();
        }
        fn active(&self) -> bool { self.ctx.data(|data| data.get_temp::<Session>(session_id())).is_some() }
    }

    #[test]
    fn middle_click_scrolls_only_its_pane_and_stopping_click_does_not_activate_a_button() {
        for axes in [egui::Vec2b::new(false, true), egui::Vec2b::new(true, false), egui::Vec2b::TRUE] {
            let mut demo = Demo::new(axes);
            let origin = demo.rects[1].center();
            demo.click(origin, egui::PointerButton::Middle);
            assert!(demo.active());
            assert_eq!(demo.offsets, vec![Vec2::ZERO; 2], "the anchor dead zone stays stationary");
            demo.frame(vec![egui::Event::PointerMoved(origin + Vec2::splat(70.0))]);
            demo.settle();
            assert_eq!(demo.offsets[0], Vec2::ZERO);
            assert!(if axes.x { demo.offsets[1].x > 20.0 } else { demo.offsets[1].y > 20.0 });
            if !axes.x { assert_eq!(demo.offsets[1].x, 0.0); }
            if !axes.y { assert_eq!(demo.offsets[1].y, 0.0); }
            demo.click(demo.button.center(), egui::PointerButton::Primary);
            assert!(!demo.active());
            assert_eq!(demo.clicks, 0, "stopping autoscroll must not click the underlying button");
            let stopped = demo.offsets.clone();
            demo.settle();
            assert_eq!(demo.offsets, stopped);
            demo.click(origin, egui::PointerButton::Middle);
            assert!(demo.active());
            demo.click(origin, egui::PointerButton::Middle);
            assert!(!demo.active(), "a second middle click stops rather than restarting autoscroll");
        }
    }

    #[test]
    fn innermost_virtualized_pane_owns_autoscroll_and_hold_release_stops_it() {
        let mut demo = Demo::new(egui::Vec2b::new(false, true));
        demo.nested = true;
        demo.settle();
        let origin = demo.rects[0].center();
        demo.pointer(origin, egui::PointerButton::Middle, true);
        demo.frame(vec![egui::Event::PointerMoved(origin + Vec2::new(0.0, 80.0))]);
        demo.settle();
        assert!(demo.offsets[0].y > 20.0);
        assert_eq!(&demo.offsets[1..], &[Vec2::ZERO; 3]);
        demo.pointer(origin + Vec2::new(0.0, 80.0), egui::PointerButton::Middle, false);
        assert!(!demo.active());
    }

    #[test]
    fn table_bodies_autoscroll_and_escape_or_disappearing_owner_cancels() {
        let mut demo = Demo::new(egui::Vec2b::new(false, true));
        demo.table = true;
        demo.settle();
        let origin = demo.rects[0].center();
        demo.click(origin, egui::PointerButton::Middle);
        demo.frame(vec![egui::Event::PointerMoved(origin + Vec2::new(0.0, 80.0))]);
        demo.settle();
        assert!(demo.offsets[0].y > 20.0);
        assert_eq!(demo.offsets[1], Vec2::ZERO);
        demo.frame(vec![egui::Event::PointerMoved(origin - Vec2::new(0.0, 10000.0))]);
        demo.settle();
        assert_eq!(demo.offsets[0], Vec2::ZERO, "scrolling upward clamps at the start of the table");
        demo.frame(vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None,
            pressed: true, repeat: false, modifiers: Default::default() }]);
        assert!(!demo.active());
        demo.click(origin, egui::PointerButton::Middle);
        assert!(demo.active());
        demo.visible = false;
        demo.frame(vec![]);
        assert!(!demo.active());
    }
}
