//! Value dropdowns: ordinary compact lists acquire search when their contents
//! exceed the available scrolling height. Callers keep their original values.
use super::*;
use std::ops::{Deref, DerefMut};

#[derive(Clone, Default)]
struct SearchState {
    last_pass: Option<u64>,
    full_height: f32,
    full_width: f32,
    searchable: bool,
    query: String,
}

/// Value-selection dropdowns with a stationary search field and one scrolling
/// results pane. Wrapping a native combo would add a competing scroll area.
pub(in crate::app) struct ValueComboBox {
    salt: egui::IdSalt,
    width: Option<f32>,
    text: egui::WidgetText,
    truncate: bool,
    height: Option<f32>,
}

impl ValueComboBox {
    pub(in crate::app) fn from_id_salt(salt: impl egui::AsIdSalt) -> Self {
        Self {
            salt: egui::IdSalt::new(salt),
            width: None,
            text: "".into(),
            truncate: false,
            height: None,
        }
    }

    pub(in crate::app) fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub(in crate::app) fn selected_text(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.text = text.into();
        self
    }

    pub(in crate::app) fn truncate(mut self) -> Self {
        self.truncate = true;
        self
    }

    pub(in crate::app) fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    pub(in crate::app) fn show_ui<R>(
        self,
        ui: &mut Ui,
        contents: impl FnOnce(&mut ValueComboUi<'_>) -> R,
    ) -> egui::InnerResponse<Option<R>> {
        let limit = self
            .height
            .unwrap_or(ui.spacing().combo_height)
            .min((ui.ctx().content_rect().height() - 80.0).max(32.0));
        let label = self.text.text().to_owned();
        let width = self.width.unwrap_or(ui.spacing().combo_width);
        let background = ui.painter().add(egui::Shape::Noop);
        let id = ui.make_persistent_id(self.salt);
        let response = ui.scope_builder(egui::UiBuilder::new().id(id), |ui| {
            // ButtonStyle subtracts each state's border from its padding before
            // the per-button override. Our separately painted border must not
            // affect layout, including hover expansion and pressed states.
            let widgets = &mut ui.visuals_mut().widgets;
            for visuals in [&mut widgets.noninteractive, &mut widgets.inactive,
                &mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
                visuals.bg_stroke = Stroke::NONE;
                visuals.expansion = 0.0;
            }
            // Keep egui's frame margins; Baboon paints the visible frame below.
            ui.add_sized(Vec2::new(width, ui.spacing().interact_size.y), egui::Button::new(self.text).right_text("")
                .frame(true).fill(Color32::TRANSPARENT).stroke(Stroke::NONE)
                .wrap_mode(if self.truncate { egui::TextWrapMode::Truncate } else { egui::TextWrapMode::Extend })
                .min_size(Vec2::new(width, ui.spacing().interact_size.y)))
        }).inner;
        let popup_id = egui::Popup::default_response_id(&response);
        let visuals = if egui::Popup::is_id_open(ui.ctx(), popup_id) {
            &ui.visuals().widgets.open
        } else { ui.style().interact(&response) };
        ui.painter().set(background, egui::Shape::rect_filled(response.rect.expand(visuals.expansion), visuals.corner_radius, visuals.bg_fill));
        ui.painter().rect_stroke(response.rect.expand(visuals.expansion), visuals.corner_radius, visuals.bg_stroke, egui::StrokeKind::Inside);
        let center = egui::pos2(response.rect.right() - ui.spacing().button_padding.x - 7.0, response.rect.center().y);
        ui.painter().add(egui::Shape::convex_polygon(vec![
            center + egui::vec2(-4.0, -2.5), center + egui::vec2(4.0, -2.5), center + egui::vec2(0.0, 2.5),
        ], visuals.fg_stroke.color, Stroke::NONE));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), label.as_str()));
        let inner = egui::Popup::menu(&response)
            .width(response.rect.width())
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| value_dropdown_contents(ui, limit, contents))
            .map(|popup| popup.inner);
        egui::InnerResponse { inner, response }
    }
}

pub(in crate::app) fn value_dropdown_contents<R>(
    ui: &mut Ui,
    max_height: f32,
    contents: impl FnOnce(&mut ValueComboUi<'_>) -> R,
) -> R {
    let limit = max_height.min((ui.ctx().content_rect().height() - 80.0).max(32.0));

    let state_id = ui.id().with("value_dropdown_search");
    let pass = ui.ctx().cumulative_pass_nr();
    let mut state = ui
        .data(|data| data.get_temp::<SearchState>(state_id))
        .unwrap_or_default();
    let reopened = !state.last_pass.is_some_and(|previous| previous + 1 >= pass);
    if reopened {
        state.query.clear();
        state.searchable = state.full_height > limit + 0.5;
    }
    let was_searchable = state.searchable;
    state.searchable |= state.full_height > limit + 0.5;
    let show_search = state.searchable;
    let needs_focus = show_search && (reopened || !was_searchable);
    // Restore room to grow after filtering a previous popup.
    ui.set_max_height(ui.ctx().content_rect().height());
    let max_width = (ui.ctx().content_rect().width() - 32.0).max(1.0);
    ui.set_min_width(state.full_width.min(max_width));
    let mut search_height = 0.0;
    let mut changed = false;
    if show_search {
        ui.set_min_width(240.0_f32.min(max_width));
        let start = ui.cursor().min.y;
        let response = ui.add(
            singleline_text_edit(&mut state.query)
                .id(state_id.with("query"))
                .hint_text(placeholder_text("Search…"))
                .desired_width(ui.available_width())
                .min_size(Vec2::new(0.0, BUTTON_HEIGHT)),
        );
        if needs_focus {
            response.request_focus();
        }
        changed = response.changed();
        ui.separator();
        search_height = ui.cursor().min.y - start;
    }
    let query = state.query.trim().to_lowercase();
    let mut scroll = crate::app::ScrollArea::vertical()
        .id_salt("value_dropdown_results")
        .max_height((limit - search_height).max(1.0))
        .auto_shrink([false, true]);
    if changed {
        scroll = scroll.vertical_scroll_offset(0.0);
    }
    let output = scroll.show(ui, |ui| {
        style_list_menu(ui);
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let mut rows = ValueComboUi {
                ui,
                query: &query,
                options: 0,
                shown: 0,
                width: 0.0,
            };
            let result = contents(&mut rows);
            if rows.options > 0 && rows.shown == 0 {
                rows.ui
                    .label(RichText::new("No matches.").color(subtle_dark()));
            }
            (result, rows.width)
        })
        .inner
    });
    if query.is_empty() {
        state.full_height = output.content_size.y;
        state.full_width = output.inner.1;
        if !show_search && state.full_height > limit + 0.5 {
            // A measurement pass adds search on the next repaint;
            // don't invoke the caller twice or replay its actions.
            ui.ctx().request_repaint();
        }
    }
    state.last_pass = Some(pass);
    ui.data_mut(|data| data.insert_temp(state_id, state));
    output.inner.0
}

/// Selection rows filter their display text, keeping caller-owned identities
/// and callbacks intact. Auxiliary labels and buttons remain ordinary UI.
pub(in crate::app) struct ValueComboUi<'a> {
    ui: &'a mut Ui,
    query: &'a str,
    options: usize,
    shown: usize,
    width: f32,
}

impl ValueComboUi<'_> {
    /// Keep filtering, measurement and selection behavior through enabled scopes.
    pub(in crate::app) fn add_enabled_ui<R>(
        &mut self, enabled: bool, contents: impl FnOnce(&mut ValueComboUi<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let response = self.ui.add_enabled_ui(enabled, |ui| {
            let mut rows = ValueComboUi { ui, query: self.query, options: 0, shown: 0, width: 0.0 };
            let result = contents(&mut rows);
            (result, rows.options, rows.shown, rows.width)
        });
        let (inner, options, shown, width) = response.inner;
        self.options += options;
        self.shown += shown;
        self.width = self.width.max(width);
        egui::InnerResponse { inner, response: response.response }
    }

    pub(in crate::app) fn baboon_checkbox(
        &mut self,
        checked: &mut bool,
        text: &str,
    ) -> egui::Response {
        self.checkbox_enabled(true, checked, text)
    }

    pub(in crate::app) fn checkbox_enabled(
        &mut self,
        enabled: bool,
        checked: &mut bool,
        text: &str,
    ) -> egui::Response {
        self.options += 1;
        if !self.query.is_empty() && !text.to_lowercase().contains(self.query) {
            let id = self.ui.next_auto_id();
            self.ui.skip_ahead_auto_ids(1);
            return self.ui.interact(egui::Rect::NOTHING, id, Sense::hover());
        }
        self.shown += 1;
        let response = self
            .ui
            .add_enabled(enabled, ThemedCheckbox::new(checked, text));
        self.width = self
            .width
            .max(response.intrinsic_size().unwrap_or(response.rect.size()).x);
        if response.clicked() {
            close_menu(self.ui);
        }
        response
    }

    pub(in crate::app) fn selectable_label(
        &mut self,
        selected: bool,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        self.selectable_label_enabled(true, selected, text)
    }

    pub(in crate::app) fn selectable_label_enabled(
        &mut self,
        enabled: bool,
        selected: bool,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        self.selectable_label_search(enabled, selected, text.into(), None)
    }

    pub(in crate::app) fn selectable_label_with_search(
        &mut self, selected: bool, text: impl Into<egui::WidgetText>, search: &str,
    ) -> egui::Response {
        self.selectable_label_search(true, selected, text.into(), Some(search))
    }

    fn selectable_label_search(
        &mut self, enabled: bool, selected: bool, text: egui::WidgetText, search: Option<&str>,
    ) -> egui::Response {
        self.options += 1;
        let label = text.text().to_owned();
        if !self.query.is_empty() && !search.unwrap_or(&label).to_lowercase().contains(self.query) {
            let id = self.ui.next_auto_id();
            self.ui.skip_ahead_auto_ids(1);
            return self.ui.interact(egui::Rect::NOTHING, id, Sense::hover());
        }
        self.shown += 1;
        let available_width = self.ui.available_width();
        let response = self
            .ui
            .add_enabled(enabled, egui::Button::selectable(selected, text));
        let intrinsic = response.intrinsic_size().unwrap_or(response.rect.size());
        self.width = self.width.max(intrinsic.x);
        let response = if intrinsic.x > available_width + 0.5 {
            response.on_hover_text(label)
        } else {
            response
        };
        if response.clicked() {
            close_menu(self.ui);
        }
        response
    }

    pub(in crate::app) fn selectable_value<T: PartialEq>(
        &mut self,
        current: &mut T,
        candidate: T,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        let mut response = self.selectable_label(*current == candidate, text);
        if response.clicked() && *current != candidate {
            *current = candidate;
            response.mark_changed();
        }
        response
    }

    pub(in crate::app) fn button(&mut self, text: impl Into<egui::WidgetText>) -> egui::Response {
        let response = self.ui.button(text);
        if response.clicked() {
            close_menu(self.ui);
        }
        response
    }
}

impl Deref for ValueComboUi<'_> {
    type Target = Ui;
    fn deref(&self) -> &Ui {
        self.ui
    }
}
impl DerefMut for ValueComboUi<'_> {
    fn deref_mut(&mut self) -> &mut Ui {
        self.ui
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Demo {
        ctx: egui::Context,
        options: Vec<String>,
        selected: usize,
        height: f32,
        screen: Vec2,
        time: f64,
        trigger: egui::Rect,
        rows: Vec<(usize, egui::Rect)>,
        search: Option<egui::Pos2>,
        open: bool,
        texts: Vec<String>,
        option_font_size: Option<f32>,
        nested: bool,
        shortened: bool,
        scroll_bars: Vec<egui::Rect>,
        trigger_y: f32,
        tag_field: bool,
        trigger_text_pos: Option<egui::Pos2>,
    }

    impl Demo {
        fn new(options: Vec<String>, height: f32) -> Self {
            let ctx = egui::Context::default();
            ctx.set_global_style(foundation_style());
            Self {
                ctx,
                options,
                selected: 0,
                height,
                screen: Vec2::new(800.0, 600.0),
                time: 0.0,
                trigger: egui::Rect::NOTHING,
                rows: Vec::new(),
                search: None,
                open: false,
                texts: Vec::new(),
                option_font_size: None,
                nested: false,
                shortened: false,
                scroll_bars: Vec::new(),
                trigger_y: 0.0,
                tag_field: false,
                trigger_text_pos: None,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.1;
            self.rows.clear();
            let ctx = self.ctx.clone();
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.screen)),
                    time: Some(self.time),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.add_space(self.trigger_y);
                    let combo = ValueComboBox::from_id_salt("value_dropdown_test")
                        .width(180.0)
                        .height(self.height)
                        .selected_text(&self.options[self.selected]);
                    let tag_field = self.tag_field;
                    let contents = |ui: &mut ValueComboUi<'_>| {
                            for (index, label) in self.options.iter().enumerate() {
                                let text: egui::WidgetText = self.option_font_size.map_or_else(
                                    || label.into(),
                                    |size| RichText::new(label).size(size).into(),
                                );
                                let row = if self.shortened {
                                    ui.selectable_label_with_search(self.selected == index, "Short display…", label)
                                } else if self.nested {
                                    ui.add_enabled_ui(true, |ui| ui.selectable_value(&mut self.selected, index, text)).inner
                                } else {
                                    ui.selectable_value(&mut self.selected, index, text)
                                };
                                if row.rect.is_positive() {
                                    self.rows.push((index, row.rect));
                                }
                            }
                        };
                    let response = if tag_field {
                        ui.horizontal(|ui| crate::app::editor::fields::combo_box_with_scroll(ui, combo, contents).0).inner
                    } else {
                        combo.show_ui(ui, contents)
                    };
                    self.trigger = response.response.rect;
                    self.open = response.inner.is_some();
                },
            );
            self.search = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Search…" => {
                    Some(text.pos + egui::vec2(3.0, 3.0))
                }
                _ => None,
            });
            self.trigger_text_pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == self.options[self.selected] => Some(text.pos),
                _ => None,
            });
            self.scroll_bars = output.shapes.iter().filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.rect.width() >= 1.0 && rect.rect.width() <= 16.0
                    && rect.rect.height() >= 8.0 && shape.clip_rect.intersects(rect.rect) => Some(rect.rect),
                _ => None,
            }).collect();
            self.texts = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }

        fn settle(&mut self) {
            for _ in 0..4 {
                self.frame(vec![]);
            }
        }

        fn click(&mut self, at: egui::Pos2) {
            self.frame(vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                self.frame(vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }]);
            }
            self.settle();
        }

        fn open(&mut self) {
            self.settle();
            self.click(self.trigger.center());
            assert!(self.open, "the dropdown opens");
        }

        fn search(&mut self, query: &str) {
            self.click(self.search.expect("overflowing values have search"));
            self.frame(vec![egui::Event::Text(query.to_owned())]);
            self.settle();
        }
    }

    #[test]
    fn dropdown_trigger_text_stays_fixed_across_interaction_states() {
        for tag_field in [false, true] {
            let mut demo = Demo::new(vec!["constant".into()], 180.0);
            demo.tag_field = tag_field;
            demo.settle();
            let idle = (demo.trigger, demo.trigger_text_pos.unwrap());
            let at = demo.trigger.center();
            demo.frame(vec![egui::Event::PointerMoved(at)]);
            demo.settle();
            assert_eq!((demo.trigger, demo.trigger_text_pos.unwrap()), idle, "hover must not shift text or bounds");
            demo.frame(vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
            demo.frame(vec![]);
            assert_eq!((demo.trigger, demo.trigger_text_pos.unwrap()), idle, "press must not shift text or bounds");
            demo.frame(vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
            demo.settle();
            assert!(demo.open);
            assert_eq!((demo.trigger, demo.trigger_text_pos.unwrap()), idle, "opening must not shift text or bounds");
        }
    }

    #[test]
    fn dropdown_triggers_keep_the_standard_left_padding() {
        for tag_field in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_global_style(foundation_style());
            let mut rect = egui::Rect::NOTHING;
            let mut padding = 0.0;
            let output = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
                padding = ui.spacing().button_padding.x;
                let combo = ValueComboBox::from_id_salt("padded_trigger").width(180.0).selected_text("constant");
                rect = if tag_field {
                    ui.horizontal(|ui| crate::app::editor::fields::combo_box_with_scroll(ui, combo, |_| {}).0.response.rect).inner
                } else { combo.show_ui(ui, |_| {}).response.rect };
            });
            let text_x = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "constant" => Some(text.pos.x),
                _ => None,
            }).unwrap();
            assert!((text_x - rect.left() - padding).abs() < 0.5,
                "tag_field={tag_field}: expected {padding}px left padding, got {}", text_x - rect.left());
        }
    }

    #[test]
    fn halo2_hud_dropdown_scrollbar_drag_moves_the_results() {
        let schema: serde_json::Value = serde_json::from_str(include_root_str!(
            "definitions/halo2_mcc/new_hud_definition.json"
        )).unwrap();
        let options: Vec<String> = schema["enums_flags"]["input 1"]["options"].as_array().unwrap()
            .iter().map(|value| value.as_str().unwrap().to_owned()).collect();
        for (tag_field, scale, top) in [
            (false, 1.0, 0.0), (true, 1.0, 0.0), (true, 1.25, 200.0),
            (true, 1.5, 200.0), (true, 1.75, 0.0), (true, 1.75, 350.0), (true, 2.0, 200.0),
        ] {
            let mut demo = Demo::new(options.clone(), 420.0);
            demo.ctx.set_pixels_per_point(scale);
            demo.trigger_y = top;
            demo.tag_field = tag_field;
            demo.open();
            let search = demo.search;
            let before = demo.rows[0].1.top();
            let thumb = *demo.scroll_bars.last().expect("the popup paints a scrollbar");
            let start = thumb.center();
            demo.frame(vec![egui::Event::PointerMoved(start)]);
            demo.frame(vec![egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
            let end = start + egui::vec2(0.0, 100.0);
            demo.frame(vec![egui::Event::PointerMoved(end)]);
            demo.frame(vec![egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
            demo.settle();
            assert!(demo.open, "dragging the scrollbar must keep the picker open");
            let distance = before - demo.rows[0].1.top();
            assert!(distance > 100.0, "tag_field={tag_field}, scale={scale}, top={top}: a 100px thumb drag moved results only {distance}px; bars {:?}", demo.scroll_bars);
            assert_eq!(demo.search, search, "search stays stationary while results scroll");
            let before_wheel = demo.rows[0].1.top();
            let over_results = demo.scroll_bars[0].center() - egui::vec2(40.0, 0.0);
            demo.frame(vec![egui::Event::PointerMoved(over_results), egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -150.0), modifiers: Default::default(),
                phase: egui::TouchPhase::Move,
            }]);
            demo.settle();
            assert!(before_wheel - demo.rows[0].1.top() > 50.0, "the wheel still scrolls the results");
            assert_eq!(demo.selected, 0, "scrolling does not select an input");
        }
    }

    #[test]
    fn nested_options_filter_and_close_like_direct_options() {
        for nested in [false, true] {
            let mut demo = Demo::new((0..20).map(|i| format!("Choice {i}")).collect(), 180.0);
            demo.nested = nested;
            demo.open();
            demo.search("Choice 19");
            assert_eq!(demo.rows.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [19]);
            demo.click(demo.rows[0].1.center());
            assert_eq!(demo.selected, 19);
            assert!(!demo.open, "nested={nested}: selection closes the popup");
        }
    }

    #[test]
    fn shortened_rows_search_the_full_label() {
        let mut demo = Demo::new((0..20).map(|i| format!("Long path to tag {i}")).collect(), 180.0);
        demo.shortened = true;
        demo.open();
        demo.search("tag 19");
        assert_eq!(demo.rows.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [19]);
        demo.click(demo.rows[0].1.center());
        assert!(!demo.open);
    }

    #[test]
    fn halo2_hud_inputs_gain_search_and_filter_without_renumbering_values() {
        let schema: serde_json::Value = serde_json::from_str(include_root_str!(
            "definitions/halo2_mcc/new_hud_definition.json"
        ))
        .unwrap();
        let options = schema["enums_flags"]["input 1"]["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let target = options
            .iter()
            .position(|label| label == "WEAPON heat")
            .unwrap();
        let count = options.len();
        let mut demo = Demo::new(options, 180.0);
        demo.open();
        assert!(
            demo.search.is_some(),
            "the actual HUD input list overflows and gains search"
        );
        assert_eq!(demo.selected, 0);
        demo.search("wEaPoN HeAt");
        assert!(demo.open, "using the search box keeps the popup open");
        assert_eq!(demo.selected, 0, "filtering does not edit the tag");
        assert_eq!(
            demo.rows
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            [target]
        );
        let row = demo.rows[0].1;
        demo.click(egui::pos2(row.right() - 4.0, row.center().y));
        assert_eq!(
            demo.selected, target,
            "the choice retains its original enum value"
        );
        assert!(!demo.open, "choosing a value closes the menu");
        demo.open();
        assert_eq!(
            demo.rows.len(),
            count,
            "reopening clears the previous filter"
        );
        demo.search("definitely-no-such-hud-input");
        assert!(demo.rows.is_empty());
        assert!(demo.texts.iter().any(|text| text == "No matches."));
        assert_eq!(
            demo.selected, target,
            "an empty search preserves the current value"
        );
        demo.click(egui::pos2(750.0, 500.0));
        assert!(!demo.open, "a click outside closes the menu");
    }

    #[test]
    fn search_depends_on_rendered_overflow_and_available_height() {
        let options = (0..4)
            .map(|index| format!("Choice {index}"))
            .collect::<Vec<_>>();
        let mut compact = Demo::new(options.clone(), 180.0);
        compact.open();
        assert!(compact.search.is_none(), "a list that fits stays compact");
        let mut constrained = Demo::new(options.clone(), 70.0);
        constrained.open();
        assert!(
            constrained.search.is_some(),
            "the same list becomes searchable at a shorter height"
        );
        let mut large_text = Demo::new(options.clone(), 180.0);
        large_text.option_font_size = Some(40.0);
        large_text.open();
        assert!(
            large_text.search.is_some(),
            "larger text can make the same number of entries overflow"
        );
        let mut narrow_window = Demo::new(options, 420.0);
        narrow_window.screen.y = 160.0;
        narrow_window.open();
        assert!(
            narrow_window.search.is_some(),
            "the popup budget follows the window height"
        );
    }
}
