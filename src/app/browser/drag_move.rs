//! Dragging Flat view items onto a folder to move them, or with Ctrl held, to
//! copy them: a folder row, another folder tab of the same kit, or another
//! folder pane open beside it. A drop never changes files itself: it starts
//! the same Move or Copy as the selection menu, with the folder already
//! chosen, so Move Selected Items? (or Copy's review) still asks first.
//!
//! A drag of one tag still fills tag reference fields, Sapien's palettes and
//! Guerilla, which take [`DraggedTagRef`]; see [`dragged_tag`].
use super::*;
use std::sync::Arc;

/// What a Flat view drag carries: the items, which pane and kit they come
/// from, and the tag itself when the drag is one tag.
#[derive(Clone)]
pub(in crate::app) struct DraggedItems {
    pub kit: KitId,
    pub pane: String,
    pub items: Vec<DraggedItem>,
    /// The tag, when it is the only item; reference fields take only this.
    pub tag: Option<DraggedTagRef>,
}

/// One dragged item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct DraggedItem {
    /// Its browser key: a tag's, or [`folder_pane_key`] for a folder.
    pub key: String,
    /// Its path in the tags folder with `\` separators: a tag's with its
    /// extension, a folder's.
    pub path: String,
    pub folder: bool,
}

/// What a browser drag offers a tag reference target.
pub(in crate::app) enum TagDrag {
    /// One tag.
    Tag(Arc<DraggedTagRef>),
    /// Folders, or several items: no reference takes them.
    Refused,
}

impl TagDrag {
    fn of(tag: Option<Arc<DraggedTagRef>>, items: Option<Arc<DraggedItems>>) -> Option<Self> {
        match (tag, items) {
            (Some(tag), _) => Some(Self::Tag(tag)),
            (None, Some(items)) => Some(match &items.tag {
                Some(tag) => Self::Tag(Arc::new(tag.clone())),
                None => Self::Refused,
            }),
            (None, None) => None,
        }
    }
}

/// The browser drag hovering `response`, for a tag reference target.
pub(in crate::app) fn hovered_tag(response: &egui::Response) -> Option<TagDrag> {
    TagDrag::of(
        response.dnd_hover_payload::<DraggedTagRef>(),
        response.dnd_hover_payload::<DraggedItems>(),
    )
}

/// The tag dropped on `response`, if a single tag was. A refused drop is
/// taken too, so it lands nowhere else.
pub(in crate::app) fn released_tag(response: &egui::Response) -> Option<Arc<DraggedTagRef>> {
    match TagDrag::of(
        response.dnd_release_payload::<DraggedTagRef>(),
        response.dnd_release_payload::<DraggedItems>(),
    )? {
        TagDrag::Tag(tag) => Some(tag),
        TagDrag::Refused => None,
    }
}

/// The single tag being dragged anywhere, as a tool outside Baboon takes it.
pub(in crate::app) fn dragged_tag(ctx: &egui::Context) -> Option<Arc<DraggedTagRef>> {
    egui::DragAndDrop::payload::<DraggedTagRef>(ctx).or_else(|| {
        egui::DragAndDrop::payload::<DraggedItems>(ctx)
            .and_then(|items| items.tag.clone().map(Arc::new))
    })
}

/// Start a drag from the Flat view row at `key`, carrying the selection, or
/// just this item when it is not selected (it becomes the selection). `tag`
/// is the row's tag. Returns `false` outside Flat view, where the row starts
/// its own drag.
pub(super) fn start(
    ui: &Ui,
    response: &egui::Response,
    key: &str,
    tag: Option<DraggedTagRef>,
) -> bool {
    let Some((kit, pane, keys)) = drag_selection(ui, key) else {
        return false;
    };
    let entries = operation_entries(ui);
    // One pass over the kit's tags for the whole selection, not one per item.
    let wanted: HashSet<&str> = keys
        .iter()
        .map(String::as_str)
        .filter(|key| !key.starts_with(FOLDER_PANE_PREFIX))
        .collect();
    let found: HashMap<&str, &TagEntry> = entries
        .as_deref()
        .into_iter()
        .flatten()
        .filter(|entry| wanted.contains(entry.key.as_str()))
        .map(|entry| (entry.key.as_str(), entry))
        .collect();
    let items: Vec<DraggedItem> = keys
        .iter()
        .cloned()
        .filter_map(|key| {
            if let Some(path) = key.strip_prefix(FOLDER_PANE_PREFIX) {
                let path = path.replace('/', "\\");
                return Some(DraggedItem {
                    key,
                    path,
                    folder: true,
                });
            }
            let entry = *found.get(key.as_str())?;
            Some(DraggedItem {
                path: entry.display_path.replace('/', "\\"),
                key,
                folder: false,
            })
        })
        .collect();
    if items.is_empty() {
        return false;
    }
    let tag = tag.filter(|_| items.len() == 1 && !items[0].folder);
    response.dnd_set_drag_payload(DraggedItems {
        kit,
        pane,
        items,
        tag,
    });
    true
}

/// The `\`-separated form of a folder relative to the tags folder; empty for
/// the tags folder itself.
fn folder_path(folder: &Path) -> String {
    tag_path::relative_tag_path(folder)
}

fn is_copy(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.modifiers.ctrl || input.modifiers.command)
}

/// Why a plain drop onto the folder the items are already in does nothing.
/// Not shown: every drag starts there, so the folder simply isn't a target.
const ALREADY_HERE: &str = "Already in this folder";

/// What dropping `drag` into `folder` (relative, `\`-separated) of `kit`
/// would do: the label beside the pointer, or why it can't.
pub(super) fn verdict(
    drag: &DraggedItems,
    kit: KitId,
    folder: &str,
    writable: bool,
    copy: bool,
) -> Result<String, String> {
    if drag.kit != kit {
        return Err("Drop onto a folder in the same kit".into());
    }
    if !writable {
        return Err("This kit can't be changed".into());
    }
    let lower = folder.to_ascii_lowercase();
    let inside = |item: &DraggedItem| {
        let path = item.path.to_ascii_lowercase();
        lower == path || lower.starts_with(&format!("{path}\\"))
    };
    if drag.items.iter().any(|item| item.folder && inside(item)) {
        return Err("A folder can't go inside itself".into());
    }
    let parent = |item: &DraggedItem| {
        tag_path::parent(&item.path)
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    if !copy && drag.items.iter().all(|item| parent(item) == lower) {
        return Err(ALREADY_HERE.into());
    }
    let target = if folder.is_empty() {
        "the tags folder".to_owned()
    } else {
        tag_path::name(folder).to_owned()
    };
    Ok(format!(
        "{} to {target}",
        if copy { "Copy" } else { "Move" }
    ))
}

/// The gap between the drag's two tooltips (what is dragged, and where it
/// would go), centred on the pointer.
const DRAG_TOOLTIP_GAP: f32 = 4.0;

/// The drag tooltips' background, so they read over anything.
fn drag_tooltip_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(foundation_block_bar())
        .stroke(Stroke::new(1.0, foundation_block_edge()))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(8, 4))
}

/// What is being dragged, as the text above the pointer says it: the tag or
/// folder's name, or how many items.
fn dragged_name(drag: &DraggedItems) -> String {
    match drag.items.as_slice() {
        [item] => tag_path::name(&item.path).to_owned(),
        items => format!("{} items", items.len()),
    }
}

/// Shown beside the pointer this frame by [`draw_drag_label`]: the target's
/// verdict, set by the first target the pointer is over.
#[derive(Clone)]
struct Hint {
    frame: u64,
    /// `None` over the items' own folder: claimed, but no target, so the
    /// label stays neutral.
    verdict: Option<Result<String, String>>,
}

fn hint_id() -> egui::Id {
    egui::Id::unique("browser_drop_hint")
}

/// Make `rect` a drop target for Flat view drags: into `folder` of `kit`.
/// `over` says whether the pointer is over it (`Ui::rect_contains_pointer`,
/// or a tab's own response), so no extra widget sits over the rows.
/// Outlines it in blue when the drop is allowed, says why not when it isn't,
/// and on release returns the Move or Copy to start. The first target the
/// pointer is over in a frame wins, so a folder row drawn before its pane
/// takes the drop.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn drop_target(
    ctx: &egui::Context,
    painter: &egui::Painter,
    over: bool,
    rect: egui::Rect,
    kit: KitId,
    folder: &Path,
    writable: bool,
) -> Option<BrowserAction> {
    let frame = ctx.cumulative_frame_nr();
    let claimed = ctx
        .data(|data| data.get_temp::<Hint>(hint_id()))
        .is_some_and(|hint| hint.frame == frame);
    if claimed || !over {
        return None;
    }
    let drag = egui::DragAndDrop::payload::<DraggedItems>(ctx)?;
    let copy = is_copy(ctx);
    let folder_text = folder_path(folder);
    let verdict = verdict(&drag, kit, &folder_text, writable, copy);
    // The items' own folder is no target at all: no outline, no reason, and
    // a release there does nothing. It still claims the frame, so the pane
    // behind its row (showing the folder above) doesn't take the drop and
    // move the items up a level.
    if verdict.as_ref().is_err_and(|reason| reason == ALREADY_HERE) {
        ctx.data_mut(|data| {
            data.insert_temp(
                hint_id(),
                Hint {
                    frame,
                    verdict: None,
                },
            )
        });
        return None;
    }
    if verdict.is_ok() {
        painter.rect_stroke(
            rect.shrink(1.0),
            3.0,
            Stroke::new(1.5, foundation_blue()),
            egui::StrokeKind::Inside,
        );
    }
    let allowed = verdict.is_ok();
    ctx.data_mut(|data| {
        data.insert_temp(
            hint_id(),
            Hint {
                frame,
                verdict: Some(verdict),
            },
        )
    });
    if !ctx.input(|input| input.pointer.any_released()) {
        return None;
    }
    // Taken either way: a refused drop lands nowhere else.
    let drag = egui::DragAndDrop::take_payload::<DraggedItems>(ctx)?;
    if !allowed {
        return None;
    }
    Some(BrowserAction::DropItems {
        pane: drag.pane.clone(),
        keys: drag.items.iter().map(|item| item.key.clone()).collect(),
        copy,
        folder: folder.to_path_buf(),
    })
}

/// Why the navigation pane and Asset Browser refuse a drop.
pub(in crate::app) const DROP_IN_FLAT_VIEW: &str = "Drop onto a folder in Flat view";
/// Why a folder tab in Groups view refuses a drop: groups are tag types, not
/// folders.
pub(in crate::app) const SWITCH_TO_FLAT_VIEW: &str =
    "Switch this tab to Flat or Tree view to drop here";
/// Why a Tree view tab refuses a drop anywhere but a folder row: a tag row
/// inside an expanded folder would leave it unclear which folder was meant.
pub(in crate::app) const DROP_ONTO_A_FOLDER: &str = "Drop onto a folder";

/// Why the navigation pane in Groups view refuses a drop: it has no folder
/// rows.
pub(in crate::app) const SWITCH_NAVIGATION_TO_FOLDERS: &str = "Switch to Folders view to drop here";

fn tree_drop_id() -> egui::Id {
    egui::Id::unique("browser_tree_drop")
}

fn navigation_kit_id() -> egui::Id {
    egui::Id::unique("browser_navigation_drop_kit")
}

/// While kit `kit`'s navigation pane draws, its folder rows take drops.
/// Undone by [`end_navigation_drops`], so other trees (pickers and the like)
/// never do.
pub(in crate::app) fn begin_navigation_drops(ui: &Ui, kit: KitId) {
    ui.data_mut(|data| data.insert_temp(navigation_kit_id(), kit));
}

pub(in crate::app) fn end_navigation_drops(ui: &Ui) {
    ui.data_mut(|data| data.remove::<KitId>(navigation_kit_id()));
}

/// A Tree view folder row, in a folder tab or the navigation pane: drop
/// Flat view items into `folder`. The Move or Copy waits for the tab or pane
/// to collect it with [`take_tree_drop`] once it has drawn.
pub(super) fn tree_folder_target(ui: &Ui, rect: egui::Rect, folder: &Path) {
    let kit = if browser_is_folder_pane(ui) {
        ui.data(|data| {
            data.get_temp::<(KitId, String)>(browser_selection_owner_id().with("drawing"))
        })
        .map(|(kit, _)| kit)
    } else {
        ui.data(|data| data.get_temp::<KitId>(navigation_kit_id()))
    };
    let Some(kit) = kit else {
        return;
    };
    let writable = browser_loose_source(ui) && !operation_read_only(ui);
    if let Some(action) = drop_target(
        ui.ctx(),
        ui.painter(),
        ui.rect_contains_pointer(rect),
        rect,
        kit,
        folder,
        writable,
    ) {
        let parked = Arc::new(std::sync::Mutex::new(Some((kit, action))));
        ui.data_mut(|data| data.insert_temp(tree_drop_id(), parked));
    }
}

type ParkedDrop = Arc<std::sync::Mutex<Option<(KitId, BrowserAction)>>>;

/// The Move or Copy a Tree view folder row of `kit` was dropped on this
/// frame. Only that kit's tab or navigation pane collects it; another kit's
/// leaves it alone.
pub(in crate::app) fn take_tree_drop(ctx: &egui::Context, kit: KitId) -> Option<BrowserAction> {
    let parked = ctx.data(|data| data.get_temp::<ParkedDrop>(tree_drop_id()))?;
    let mut parked = parked
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if parked.as_ref().is_some_and(|(owner, _)| *owner == kit) {
        ctx.data_mut(|data| data.remove::<ParkedDrop>(tree_drop_id()));
        return parked.take().map(|(_, action)| action);
    }
    None
}

/// Somewhere a Flat view drag can't land (the navigation pane, a folder tab
/// in Tree or Groups view, an Asset Browser): while the pointer is over it,
/// the label says `reason` in red with the not-allowed cursor, rather than
/// the drag looking as if it would work. Letting go there does nothing.
pub(in crate::app) fn refuse_drop(ctx: &egui::Context, over: bool, reason: &str) {
    let frame = ctx.cumulative_frame_nr();
    let claimed = ctx
        .data(|data| data.get_temp::<Hint>(hint_id()))
        .is_some_and(|hint| hint.frame == frame);
    if claimed || !over || !egui::DragAndDrop::has_payload_of_type::<DraggedItems>(ctx) {
        return;
    }
    ctx.data_mut(|data| {
        data.insert_temp(
            hint_id(),
            Hint {
                frame,
                verdict: Some(Err(reason.to_owned())),
            },
        )
    });
}

/// Once a frame, after the panes and tabs: while a Flat view drag is under
/// way, a label beside the pointer saying what dropping here would do, and
/// the matching cursor.
pub(in crate::app) fn draw_drag_label(ctx: &egui::Context) {
    let Some(drag) = egui::DragAndDrop::payload::<DraggedItems>(ctx) else {
        return;
    };
    let Some(pointer) = ctx.pointer_latest_pos() else {
        return;
    };
    let frame = ctx.cumulative_frame_nr();
    let hint = ctx
        .data(|data| data.get_temp::<Hint>(hint_id()))
        .filter(|hint| hint.frame == frame);
    let copy = is_copy(ctx);
    let verdict = hint.and_then(|hint| hint.verdict);
    ctx.set_cursor_icon(match &verdict {
        Some(Err(_)) => egui::CursorIcon::NotAllowed,
        Some(Ok(_)) if copy => egui::CursorIcon::Copy,
        _ => egui::CursorIcon::Grabbing,
    });
    // What is dragged just above the pointer's centre, and what dropping here
    // would do just below it, the gap between them centred on the pointer:
    // apart, so neither can cover the other, whichever is drawn last.
    egui::Area::new(egui::Id::unique("browser_drag_name"))
        .order(egui::Order::Tooltip)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(pointer + egui::vec2(16.0, -DRAG_TOOLTIP_GAP / 2.0))
        .interactable(false)
        .show(ctx, |ui| {
            drag_tooltip_frame().show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(dragged_name(&drag)).color(text_dark()))
                        .extend(),
                );
            });
        });
    // Nothing to say away from a target.
    let Some(verdict) = verdict else {
        return;
    };
    let (text, color) = match verdict {
        Ok(text) => (text, text_dark()),
        Err(reason) => (reason, ctx.global_style().visuals.error_fg_color),
    };
    // The action's icon first: Move, or Copy while Ctrl is held. Red with
    // the reason when the drop isn't allowed.
    let icon = if copy {
        ButtonIcon::Copy
    } else {
        ButtonIcon::Move
    };
    egui::Area::new(egui::Id::unique("browser_drag_label"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer + egui::vec2(16.0, DRAG_TOOLTIP_GAP / 2.0))
        .interactable(false)
        .show(ctx, |ui| {
            drag_tooltip_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    paint_button_icon_at(ui, icon, rect, color);
                    ui.add(egui::Label::new(RichText::new(text).color(color)).extend());
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(items: &[(&str, bool)]) -> DraggedItems {
        DraggedItems {
            kit: KitId(1),
            pane: "pane".into(),
            items: items
                .iter()
                .map(|(path, folder)| DraggedItem {
                    key: (*path).into(),
                    path: (*path).into(),
                    folder: *folder,
                })
                .collect(),
            tag: None,
        }
    }

    /// Above the pointer, what is dragged; below it, boxed, the action's icon
    /// and where the drop goes. Ctrl turns Move into Copy.
    #[test]
    fn the_drag_label_leads_with_the_action_icon() {
        let ctx = egui::Context::default();
        // As the app does: icons are rasterized by its SVG loader.
        crate::app::Baboon::configure_context(&ctx);
        let frame = |ctrl: bool, over_target: bool| {
            let modifiers = if ctrl {
                egui::Modifiers::CTRL
            } else {
                egui::Modifiers::NONE
            };
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(600.0, 400.0),
                    )),
                    events: vec![
                        egui::Event::ModifiersChanged(modifiers),
                        egui::Event::PointerMoved(egui::pos2(100.0, 100.0)),
                    ],
                    ..Default::default()
                },
                |ui| {
                    egui::DragAndDrop::set_payload(
                        ui.ctx(),
                        drag(&[("a.model", false), ("b.model", false)]),
                    );
                    if over_target {
                        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(300.0));
                        let _ = drop_target(
                            ui.ctx(),
                            ui.painter(),
                            true,
                            rect,
                            KitId(1),
                            Path::new("objects"),
                            true,
                        );
                    }
                    draw_drag_label(ui.ctx());
                },
            );
            let texts: Vec<String> = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
            let icon = output.shapes.iter().any(|clipped| {
                matches!(&clipped.shape, egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default())
            });
            (texts, icon)
        };
        // An egui area measures itself on its first pass and paints from
        // the next.
        frame(false, true);
        let (texts, icon) = frame(false, true);
        assert!(
            texts.contains(&"2 items".to_owned()),
            "what is dragged, above: {texts:?}"
        );
        assert!(texts.contains(&"Move to objects".to_owned()), "{texts:?}");
        assert!(icon, "no icon painted beside the label");
        frame(true, true);
        let (texts, _) = frame(true, true);
        assert!(texts.contains(&"Copy to objects".to_owned()), "{texts:?}");
        // Away from any target, only what is dragged.
        let (texts, icon) = frame(false, false);
        assert_eq!(texts, ["2 items"]);
        assert!(!icon);
    }

    /// Every drag starts over the items' own folder; that is no target, so
    /// the label stays neutral there instead of saying why it can't drop. Its
    /// row in a pane showing the folder above still claims the pointer, so
    /// that pane doesn't take the drop and move the items up a level.
    #[test]
    fn the_items_own_folder_is_quietly_no_target() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(50.0));
        let mut dropped = Vec::new();
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::DragAndDrop::set_payload(ui.ctx(), drag(&[(r"objects\weapons\a.model", false)]));
            // The `weapons` row, then the `objects` pane behind it.
            for folder in [r"objects\weapons", "objects"] {
                dropped.push(
                    drop_target(
                        ui.ctx(),
                        ui.painter(),
                        true,
                        rect,
                        KitId(1),
                        Path::new(folder),
                        true,
                    )
                    .is_some(),
                );
            }
        });
        assert_eq!(dropped, [false, false]);
        let hint = ctx
            .data(|data| data.get_temp::<Hint>(hint_id()))
            .expect("the row claims the pointer");
        assert!(
            hint.verdict.is_none(),
            "a neutral label, not {ALREADY_HERE} or the pane's Move"
        );
    }

    /// The navigation pane, Tree and Groups tabs and Asset Browsers say why
    /// they take no drop, but never talk over a folder that does.
    #[test]
    fn places_without_drops_say_why_unless_a_target_already_spoke() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(50.0));
        let reason = || {
            ctx.data(|data| data.get_temp::<Hint>(hint_id()))
                .and_then(|hint| hint.verdict)
        };
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::DragAndDrop::set_payload(ui.ctx(), drag(&[(r"objects\a.model", false)]));
            refuse_drop(ui.ctx(), false, DROP_IN_FLAT_VIEW);
            refuse_drop(ui.ctx(), true, SWITCH_TO_FLAT_VIEW);
        });
        assert_eq!(
            reason(),
            Some(Err(SWITCH_TO_FLAT_VIEW.to_owned())),
            "only where the pointer is"
        );
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::DragAndDrop::set_payload(ui.ctx(), drag(&[(r"objects\a.model", false)]));
            let _ = drop_target(
                ui.ctx(),
                ui.painter(),
                true,
                rect,
                KitId(1),
                Path::new("other"),
                true,
            );
            refuse_drop(ui.ctx(), true, DROP_IN_FLAT_VIEW);
        });
        assert_eq!(
            reason(),
            Some(Ok("Move to other".to_owned())),
            "the folder under the pointer wins"
        );
    }

    #[test]
    fn reference_fields_take_one_dragged_tag_and_refuse_the_rest() {
        let tag = DraggedTagRef {
            group_tag: 1,
            input: "x".into(),
            rel_path: "x".into(),
            file_path: None,
        };
        let one = DraggedItems {
            tag: Some(tag.clone()),
            ..drag(&[("a.model", false)])
        };
        assert!(matches!(
            TagDrag::of(None, Some(Arc::new(one))),
            Some(TagDrag::Tag(_))
        ));
        for refused in [
            drag(&[("a.model", false), ("b.model", false)]),
            drag(&[("folder", true)]),
        ] {
            assert!(matches!(
                TagDrag::of(None, Some(Arc::new(refused))),
                Some(TagDrag::Refused)
            ));
        }
        // A tree view's tag drag, as before.
        assert!(matches!(
            TagDrag::of(Some(Arc::new(tag)), None),
            Some(TagDrag::Tag(_))
        ));
        assert!(TagDrag::of(None, None).is_none());
    }

    #[test]
    fn a_drop_says_what_it_will_do_or_why_it_cannot() {
        let one = drag(&[(r"objects\characters\grunt\grunt.biped", false)]);
        let some = drag(&[
            (r"objects\characters\grunt", true),
            (r"objects\characters\elite", true),
        ]);
        let covenant = r"objects\characters\covenant";
        assert_eq!(
            verdict(&one, KitId(1), covenant, true, false).unwrap(),
            "Move to covenant"
        );
        assert_eq!(
            verdict(&some, KitId(1), covenant, true, true).unwrap(),
            "Copy to covenant"
        );
        assert_eq!(
            verdict(&some, KitId(1), "", true, false).unwrap(),
            "Move to the tags folder"
        );
        assert_eq!(
            verdict(&one, KitId(2), covenant, true, false).unwrap_err(),
            "Drop onto a folder in the same kit"
        );
        assert_eq!(
            verdict(&one, KitId(1), covenant, false, false).unwrap_err(),
            "This kit can't be changed"
        );
        for inside in [r"objects\characters\grunt", r"OBJECTS\characters\grunt\ai"] {
            assert_eq!(
                verdict(&some, KitId(1), inside, true, false).unwrap_err(),
                "A folder can't go inside itself"
            );
        }
        // A folder whose name merely starts the same is somewhere else.
        assert!(verdict(&some, KitId(1), r"objects\characters\grunt_v2", true, false).is_ok());
        let here = r"objects\characters";
        assert_eq!(
            verdict(&some, KitId(1), here, true, false).unwrap_err(),
            ALREADY_HERE
        );
        assert_eq!(
            verdict(&some, KitId(1), here, true, true).unwrap(),
            "Copy to characters",
            "copying in place duplicates"
        );
    }
}
