//! Select single-line values on keyboard traversal, without changing click focus.
use super::*;

#[derive(Clone, Copy)]
struct TabNavigation { pass: u64, origin: Option<egui::Id> }

type Fields = HashMap<egui::Id, (usize, u64)>;
fn fields_id() -> egui::Id { egui::Id::new("singleline_focus_fields") }
fn navigation_id() -> egui::Id { egui::Id::new("singleline_tab_navigation") }

pub(in crate::app) fn install_text_focus(ctx: &egui::Context) {
    ctx.on_begin_pass("baboon_text_tab_focus", Arc::new(|ui| {
        let ctx = ui.ctx();
        let pass = ctx.cumulative_pass_nr();
        let focus = ctx.memory(|memory| memory.focused());
        let (tab, pointer) = ctx.input(|input| (
            input.events.iter().any(|event| matches!(event, egui::Event::Key {
                key: egui::Key::Tab, pressed: true, modifiers, ..
            } if !modifiers.alt && !modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd)),
            input.pointer.any_pressed(),
        ));
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<Fields>(fields_id()).retain(|_, (_, seen)| *seen + 1 >= pass);
            if pointer { data.remove::<TabNavigation>(navigation_id()); }
            else if tab { data.insert_temp(navigation_id(), TabNavigation { pass, origin: focus }); }
        });
        if pointer { return; }
        let pending = ctx.data(|data| data.get_temp::<TabNavigation>(navigation_id()));
        let Some(pending) = pending else { return; };
        if pass > pending.pass + 1 {
            ctx.data_mut(|data| data.remove::<TabNavigation>(navigation_id()));
            return;
        }
        // Shift+Tab assigns focus at the beginning of the following pass.
        // Select before TextEdit processes that pass's first typed character.
        if let Some(id) = focus.filter(|id| Some(*id) != pending.origin)
            && let Some((length, _)) = ctx.data_mut(|data| data.get_temp_mut_or_default::<Fields>(fields_id()).get(&id).copied())
            && let Some(mut state) = egui::TextEdit::load_state(ctx, id)
        {
            state.cursor.set_char_range(Some(all_text(length)));
            state.store(ctx, id);
            ctx.data_mut(|data| data.remove::<TabNavigation>(navigation_id()));
        }
    }));
}

fn all_text(length: usize) -> egui::text::CCursorRange {
    egui::text::CCursorRange::two(egui::text::CCursor::new(length), egui::text::CCursor::new(0))
}

pub(super) fn singleline_tab_focus(ui: &Ui, output: &mut egui::text_edit::TextEditOutput) {
    let id = output.response.id;
    let length = output.galley.text().chars().count();
    let pass = ui.ctx().cumulative_pass_nr();
    ui.data_mut(|data| data.get_temp_mut_or_default::<Fields>(fields_id()).insert(id, (length, pass)));
    let pending = ui.data(|data| data.get_temp::<TabNavigation>(navigation_id()));
    if output.response.gained_focus() && pending.is_some_and(|pending|
        pass <= pending.pass + 1 && pending.origin != Some(id))
    {
        let range = all_text(length);
        output.state.cursor.set_char_range(Some(range));
        output.state.clone().store(ui.ctx(), id);
        output.cursor_range = Some(range);
        ui.data_mut(|data| data.remove::<TabNavigation>(navigation_id()));
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FieldsDemo {
        ctx: egui::Context,
        values: [String; 3],
        rects: [egui::Rect; 3],
        custom_frame: bool,
    }

    impl FieldsDemo {
        fn new(custom_frame: bool) -> Self {
            let ctx = egui::Context::default();
            Baboon::configure_context(&ctx);
            let mut demo = Self { ctx, values: ["café 🐒".into(), "123.45".into(), "line one\nline two".into()],
                rects: [egui::Rect::NOTHING; 3], custom_frame };
            demo.frame(vec![]);
            demo.frame(vec![]);
            demo
        }
        fn id(index: usize) -> egui::Id { egui::Id::new(("tab_value", index)) }
        fn frame(&mut self, events: Vec<egui::Event>) {
            let _ = crate::app::run_ui_test(&self.ctx, egui::RawInput { events, focused: true, ..Default::default() }, |ui| {
                for index in 0..3 {
                    let id = Self::id(index);
                    self.rects[index] = if index == 2 {
                        ui.add(egui::TextEdit::multiline(&mut self.values[index]).id(id)).rect
                    } else {
                        let mut edit = singleline_text_edit(&mut self.values[index]).id(id);
                        if self.custom_frame { edit = edit.frame(egui::Frame::NONE); }
                        ui.add(edit).rect
                    };
                }
            });
        }
        fn focus(&mut self, index: usize) {
            self.ctx.memory_mut(|memory| memory.request_focus(Self::id(index)));
            self.frame(vec![]);
        }
        fn tab(&mut self, shift: bool) {
            self.frame(vec![egui::Event::Key { key: egui::Key::Tab, physical_key: None,
                pressed: true, repeat: false,
                modifiers: if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE } }]);
        }
        fn type_text(&mut self, text: &str) { self.frame(vec![egui::Event::Text(text.into())]); }
        fn selection(&self, index: usize) -> egui::text::CCursorRange {
            egui::TextEdit::load_state(&self.ctx, Self::id(index)).unwrap().cursor.char_range().unwrap()
        }
    }

    #[test]
    fn tab_and_shift_tab_replace_whole_singleline_values_on_the_first_typed_character() {
        for custom_frame in [false, true] {
            let mut demo = FieldsDemo::new(custom_frame);
            demo.focus(0);
            assert!(demo.selection(0).is_empty(), "programmatic focus preserves the caret");
            demo.tab(false);
            demo.type_text("7");
            assert_eq!(demo.values[1], "7", "Tab selects the entire numeric value");
            demo.tab(true);
            // No settling frame: backward traversal must select before this
            // very next input pass handles the first typed character.
            demo.type_text("Z");
            assert_eq!(demo.values[0], "Z", "Shift+Tab selects the complete Unicode value");
        }
    }

    #[test]
    fn tab_selection_remains_selected_until_typing_and_arrows_allow_partial_edits() {
        let mut demo = FieldsDemo::new(false);
        demo.focus(0);
        demo.tab(false);
        demo.frame(vec![]);
        let range = demo.selection(1).sorted_cursors();
        assert_eq!(range[0], egui::text::CCursor::new(0));
        assert_eq!(range[1], egui::text::CCursor::new(6));
        demo.frame(vec![egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None,
            pressed: true, repeat: false, modifiers: Default::default() }]);
        demo.type_text("7");
        assert_eq!(demo.values[1], "123.457");
    }

    #[test]
    fn mouse_clicks_position_the_caret_and_multiline_tab_focus_preserves_selection() {
        let mut demo = FieldsDemo::new(false);
        let at = demo.rects[0].left_center() + Vec2::new(25.0, 0.0);
        for pressed in [true, false] {
            demo.frame(vec![egui::Event::PointerMoved(at), egui::Event::PointerButton {
                pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default(),
            }]);
        }
        assert!(demo.selection(0).is_empty());
        let length = demo.values[0].chars().count();
        demo.type_text("Z");
        assert_eq!(demo.values[0].chars().count(), length + 1, "click focus inserts rather than replaces");
        let mut state = egui::TextEdit::load_state(&demo.ctx, FieldsDemo::id(2)).unwrap();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(3))));
        state.store(&demo.ctx, FieldsDemo::id(2));
        demo.focus(1);
        demo.tab(false);
        demo.type_text("Z");
        assert_eq!(demo.values[2], "linZe one\nline two");
    }
}
