//! The block table window: a block's entries as rows to reorder, rename, add,
//! duplicate and delete. Changes stay staged until Confirm Changes succeeds.

use super::*;
use crate::app::search::draw_icon_window_header_without_close;

#[derive(Clone)]
struct BlockTableDrag(u64);

fn block_table_move_order(len: usize, from: usize, before: usize) -> Vec<usize> {
    let mut order: Vec<_> = (0..len).collect();
    let moved = order.remove(from);
    let destination = if before > from { before - 1 } else { before };
    order.insert(destination.min(order.len()), moved);
    order
}

enum BlockTableAction {
    None,
    Cancel,
    Save,
    Change(BlockOpKind),
}

fn draw_block_table(ctx: &egui::Context, table: &mut BlockTableState) -> BlockTableAction {
    let mut save = false;
    let mut cancel = false;
    let mut operation = None;
    let mut drop_move = None;
    let cap = table
        .tag
        .root()
        .field_path(&table.request.path)
        .and_then(|field| field.as_block())
        .map(|block| block.definition().max_count() as usize)
        .unwrap_or(0);
    let can_grow = cap == 0 || table.rows.len() < cap;
    // egui draws a corner grip even for horizontal-only resizing. Hide its
    // paint locally while keeping the side resize interactions available.
    let original_style = ctx.global_style();
    let mut window_style = (*original_style).clone();
    window_style.visuals.resize_corner_size = 0.0;
    ctx.set_global_style(window_style);
    let title = format!("Block Entry Table - {}", table.request.label);
    egui::Window::new(&title)
        .id(egui::Id::new("block_table"))
        .title_bar(false)
        .collapsible(false)
        .resizable([true, false])
        .default_width(720.0)
        .min_width(660.0)
        .show(ctx, |ui| {
            draw_icon_window_header_without_close(ui, &title, ButtonIcon::TableView);
            let count = table.rows.len();
            let painter = ui.painter().clone();
            let margin = ui.spacing().window_margin;
            // Out to the window's edge: egui 0.36 counts its stroke as part
            // of the frame's margin, beside the padding.
            let edge = egui::Frame::window(ui.style()).total_margin();
            let band_left = ui.min_rect().left() - edge.left;
            let band_right = ui.max_rect().right() + edge.right;
            let band_gap = 0.5 * ui.spacing().item_spacing.y;
            let band_fill = ui.visuals().faint_bg_color;
            let drop_stroke = Stroke::new(2.0_f32, ui.visuals().selection.stroke.color);
            // Keep the list budget independent of the previous window height.
            // Short blocks hug their rows; long blocks scroll within this cap.
            let list_height = (ctx.content_rect().height() - 230.0).clamp(100.0, 560.0);
            let list_rect = egui::Rect::from_min_size(ui.cursor().min,
                Vec2::new(ui.available_width(), list_height + 26.0 + ui.spacing().item_spacing.y));
            ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
            egui_extras::TableBuilder::new(ui)
                .id_salt("block_entry_table")
                .striped(false)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(egui_extras::Column::exact(20.0))
                .column(egui_extras::Column::exact(50.0))
                .column(egui_extras::Column::exact(16.0))
                .column(egui_extras::Column::remainder().at_least(160.0).clip(true))
                .column(egui_extras::Column::exact(270.0))
                .min_scrolled_height(32.0)
                .max_scroll_height(list_height)
                .auto_shrink([false, true])
                .header(26.0, |mut header| {
                    header.col(|_| {});
                    header.col(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.strong("Index");
                        });
                    });
                    header.col(|_| {});
                    header.col(|ui| { ui.strong("Name"); });
                    header.col(|ui| { ui.strong("Actions"); });
                    let mut border_painter = painter.clone();
                    let mut clip = border_painter.clip_rect();
                    clip.min.x = band_left;
                    clip.max.x = band_right;
                    border_painter.set_clip_rect(clip.intersect(ctx.content_rect()));
                    border_painter.hline(band_left..=band_right,
                        header.response().rect.bottom(), Stroke::new(1.0_f32, grid_line()));
                })
                .body(|mut body| {
                    for (index, row) in table.rows.iter_mut().enumerate() {
                        let background = painter.add(egui::Shape::Noop);
                        body.row(36.0, |mut cells| {
                            let mut band_painter = painter.clone();
                            cells.col(|ui| {
                                let clip = ui.clip_rect();
                                band_painter.set_clip_rect(egui::Rect::from_min_max(
                                    egui::pos2(band_left, clip.top()),
                                    egui::pos2(band_right, clip.bottom())).intersect(ctx.content_rect()));
                                let (rect, handle) = ui.allocate_exact_size(Vec2::new(18.0, 30.0), Sense::drag());
                                let handle = handle.on_hover_text("Drag to reorder entry").on_hover_cursor(egui::CursorIcon::Grab);
                                handle.dnd_set_drag_payload(BlockTableDrag(row.id));
                                if handle.dragged() { ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing); }
                                for x in [-2.5, 2.5] {
                                    for y in [-6.0, 0.0, 6.0] {
                                        ui.painter().circle_filled(rect.center() + Vec2::new(x, y), 1.2, subtle_dark());
                                    }
                                }
                            });
                            cells.col(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(format!("{index}.")).monospace());
                                });
                            });
                            cells.col(|_| {});
                            cells.col(|ui| {
                                // Draw at the current column width, but report
                                // only the minimum width to the table. Otherwise
                                // the filled width becomes next frame's minimum.
                                let mut editor = ui.new_child(egui::UiBuilder::new()
                                    .max_rect(ui.available_rect_before_wrap())
                                    .layout(*ui.layout()));
                                if row.name_field.is_some() {
                                    let font = egui::TextStyle::Monospace.resolve(editor.style());
                                    let row_height = editor.fonts_mut(|fonts| fonts.row_height(&font));
                                    editor.add_sized(Vec2::new(editor.available_width(), 24.0),
                                        egui::TextEdit::singleline(&mut row.name)
                                        .id(egui::Id::new(("block_table_name", row.id)))
                                        .margin(egui::Margin::symmetric(4, ((24.0 - row_height) * 0.5).max(0.0) as i8))
                                        .desired_width(editor.available_width()).font(egui::TextStyle::Monospace));
                                } else {
                                    editor.add(egui::Label::new(&row.name).truncate())
                                        .on_hover_text("This entry has no editable name field");
                                }
                                ui.allocate_space(Vec2::new(160.0, editor.min_size().y));
                            });
                            cells.col(|ui| {
                                if icon_text_button(ui, ButtonIcon::InsertRow, "Insert", can_grow).clicked() {
                                    operation = Some(BlockOpKind::Insert(index));
                                }
                                if icon_text_button(ui, ButtonIcon::Duplicate, "Duplicate", can_grow).clicked() {
                                    operation = Some(BlockOpKind::Duplicate(index));
                                }
                                if icon_text_button(ui, ButtonIcon::Remove, "Delete", true).clicked() {
                                    operation = Some(BlockOpKind::Delete(index));
                                }
                            });
                            let response = cells.response();
                            if index % 2 == 1 {
                                let mut band = response.rect.expand2(Vec2::new(0.0, band_gap));
                                band.min.x = band_left;
                                band.max.x = band_right;
                                band_painter.set(background, egui::epaint::RectShape::filled(
                                    band, 0.0, band_fill));
                            }
                            if response.dnd_hover_payload::<BlockTableDrag>().is_some() {
                                let below = ctx.pointer_hover_pos().is_some_and(|pos| pos.y > response.rect.center().y);
                                let y = if below { response.rect.bottom() } else { response.rect.top() };
                                painter.hline(response.rect.x_range(), y, drop_stroke);
                            }
                            if let Some(source) = response.dnd_release_payload::<BlockTableDrag>() {
                                let below = ctx.pointer_hover_pos().is_some_and(|pos| pos.y > response.rect.center().y);
                                drop_move = Some((source.0, index + usize::from(below)));
                            }
                        });
                    }
                });
            });
            if count == 0 { ui.label(RichText::new("Empty block").color(subtle_dark())); }
            if let Some(status) = &table.status { ui.label(RichText::new(status).color(REFERENCE_MISSING_COLOR)); }
            let mut footer_painter = painter.clone();
            footer_painter.set_clip_rect(ui.clip_rect().expand(margin.sum().max_elem()).intersect(ctx.content_rect()));
            let footer_background = footer_painter.add(egui::Shape::Noop);
            let footer = Frame::NONE.inner_margin(egui::Margin {
                left: 0, right: 0, top: 6 + margin.bottom, bottom: 6,
            }).show(ui, |ui| {
            ui.horizontal(|ui| {
                if icon_text_button(ui, ButtonIcon::Add, "Add Entry", can_grow).clicked() {
                    operation = Some(BlockOpKind::Add);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    cancel = icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked();
                    save = icon_text_button(ui, ButtonIcon::Confirm, "Confirm Changes", table.has_changes()).clicked();
                });
            });
            });
            let mut footer_rect = footer.response.rect;
            footer_rect.min.x = band_left;
            footer_rect.max.x = band_right;
            footer_rect.max.y += edge.bottom;
            let mut rounding = ui.visuals().window_corner_radius;
            rounding.nw = 0;
            rounding.ne = 0;
            footer_painter.set(footer_background, egui::epaint::RectShape::filled(
                footer_rect, rounding, foundation_block_bar()));
        });
    ctx.set_global_style(original_style);
    if let Some((id, before)) = drop_move
        && let Some(from) = table.rows.iter().position(|row| row.id == id)
    {
        operation = Some(BlockOpKind::Reorder {
            order: block_table_move_order(table.rows.len(), from, before),
        });
    }
    if cancel {
        BlockTableAction::Cancel
    } else if save {
        BlockTableAction::Save
    } else if let Some(operation) = operation {
        BlockTableAction::Change(operation)
    } else {
        BlockTableAction::None
    }
}

impl Dialog for BlockTableState {
    /// The table edits its own copy of the tag; Confirm Changes asks for it to
    /// be committed, and the window stays open until that succeeds.
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        match draw_block_table(cx.egui, self) {
            BlockTableAction::Cancel => return false,
            BlockTableAction::Save => cx.send(EditorCommand::SaveBlockTable),
            BlockTableAction::None => {}
            BlockTableAction::Change(operation) => {
                if let Some(kit) = cx.model.kit(self.kit) {
                    if let Err(error) = self.stage(operation, &kit.names) {
                        self.status = Some(error);
                    }
                    cx.egui.request_repaint();
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_drag_emits_reorder_and_footer_cancel_discards_the_stage() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        let mut tag = TagFile::new(crate::app::test_definition_path(
            "haloreach_mcc/test_tag.json",
        ))
        .unwrap();
        for _ in 0..3 {
            apply_one_block_op(
                &mut tag,
                &BlockOp {
                    path: "basic block".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
        }
        let baseline_bytes = tag.write_to_bytes().unwrap();
        let mut table = BlockTableState {
            kit: KitId(1),
            tag_key: "test".to_owned(),
            request: BlockTableRequest {
                path: "basic block".to_owned(),
                label: "Basic Block".to_owned(),
                view_scope: "test".to_owned(),
                selected: 0,
            },
            stamp: (1, 0),
            game: None,
            definitions_root: None,
            tag,
            baseline_bytes,
            rows: (0..3)
                .map(|index| BlockTableRow {
                    id: index as u64,
                    original_index: Some(index),
                    name_field: Some("name".to_owned()),
                    name: format!("entry_{index}"),
                    stored_name: format!("entry_{index}"),
                })
                .collect(),
            next_id: 3,
            status: None,
            changed: false,
        };
        let frame = |table: &mut BlockTableState, events| {
            let mut action = BlockTableAction::None;
            let output = crate::app::run_ui_test(&ctx, 
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1100.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |_| {
                    action = draw_block_table(&ctx, table);
                },
            );
            (action, output)
        };
        for _ in 0..5 {
            frame(&mut table, Vec::new());
        }
        let window_rect = || ctx.memory(|memory| memory.area_rect(egui::Id::new("block_table")).unwrap());
        let resize = |table: &mut BlockTableState, delta: f32| {
            let edge = window_rect().right_center() - Vec2::new(1.0, 0.0);
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(table, vec![egui::Event::PointerMoved(edge)]);
            frame(table, vec![button(edge, true)]);
            frame(table, vec![egui::Event::PointerMoved(edge + Vec2::new(delta, 0.0))]);
            frame(table, vec![button(edge + Vec2::new(delta, 0.0), false)]);
            for _ in 0..5 { frame(table, Vec::new()); }
        };
        let initial_width = window_rect().width();
        resize(&mut table, 200.0);
        let expanded_width = window_rect().width();
        assert!(expanded_width > initial_width + 150.0);
        resize(&mut table, -200.0);
        assert!(window_rect().width() < expanded_width - 150.0, "window must shrink after expansion");
        let short_height = window_rect().height();
        for index in 3..30 {
            let mut row = table.rows[0].clone();
            row.id = index as u64;
            row.name = format!("entry_{index}");
            table.rows.push(row);
        }
        for _ in 0..5 { frame(&mut table, Vec::new()); }
        let capped_height = window_rect().height();
        assert!(capped_height > short_height + 200.0);
        assert!(capped_height < 800.0, "long blocks leave room for the footer");
        for index in 30..60 {
            let mut row = table.rows[0].clone();
            row.id = index as u64;
            table.rows.push(row);
        }
        for _ in 0..5 { frame(&mut table, Vec::new()); }
        assert!((window_rect().height() - capped_height).abs() < 1.0, "additional rows scroll instead of growing the window");
        table.rows.truncate(3);
        for _ in 0..5 { frame(&mut table, Vec::new()); }
        assert!((window_rect().height() - short_height).abs() < 1.0, "short blocks hug their rows again");
        let (_, output) = frame(&mut table, Vec::new());
        let full_width = window_rect().width() - 3.0;
        let full_width_fills = |fill| output.shapes.iter().filter(|clipped| {
            matches!(&clipped.shape, egui::Shape::Rect(rect)
                if rect.fill == fill && rect.rect.width() >= full_width)
        }).count();
        assert!(full_width_fills(foundation_block_bar()) >= 2, "header and footer fills reach the edges");
        assert!(full_width_fills(ctx.global_style().visuals.faint_bg_color) >= 1, "row bands reach the edges");
        assert!(ctx.global_style().visuals.resize_corner_size > 0.0, "grip override stays local to this window");
        let handle = output
            .shapes
            .iter()
            .find_map(|clipped| match clipped.shape {
                egui::Shape::Circle(circle) if (circle.radius - 1.2).abs() < 0.01 => {
                    Some(circle.center + Vec2::new(2.5, 6.0))
                }
                _ => None,
            })
            .expect("drag handle");
        let text_pos = |output: &egui::FullOutput, text: &str| {
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(shape) if shape.galley.text() == text => {
                        Some(shape.pos + shape.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("table text")
        };
        let target = text_pos(&output, "entry_2") + Vec2::new(0.0, 10.0);
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &mut table,
            vec![egui::Event::PointerMoved(handle), pointer(handle, true)],
        );
        frame(
            &mut table,
            vec![egui::Event::PointerMoved(handle + Vec2::new(0.0, 12.0))],
        );
        frame(&mut table, vec![egui::Event::PointerMoved(target)]);
        let (action, _) = frame(&mut table, vec![pointer(target, false)]);
        match action {
            BlockTableAction::Change(BlockOpKind::Reorder { order }) => {
                assert_eq!(order, vec![1, 2, 0])
            }
            _ => panic!("dropping the handle must request a reordered block"),
        }
        let (_, output) = frame(&mut table, Vec::new());
        let cancel = text_pos(&output, "Cancel");
        frame(
            &mut table,
            vec![egui::Event::PointerMoved(cancel), pointer(cancel, true)],
        );
        assert!(matches!(
            frame(&mut table, vec![pointer(cancel, false)]).0,
            BlockTableAction::Cancel
        ));
        assert!(!table.changed);
    }

    #[test]
    fn drag_before_or_after_rows_previews_the_final_indices() {
        assert_eq!(block_table_move_order(4, 0, 4), vec![1, 2, 3, 0]);
        assert_eq!(block_table_move_order(4, 3, 0), vec![3, 0, 1, 2]);
        assert_eq!(block_table_move_order(4, 1, 3), vec![0, 2, 1, 3]);
        assert_eq!(block_table_move_order(4, 1, 2), vec![0, 1, 2, 3]);
    }
}
