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
    TagDrag::of(response.dnd_hover_payload::<DraggedTagRef>(), response.dnd_hover_payload::<DraggedItems>())
}

/// The tag dropped on `response`, if a single tag was. A refused drop is
/// taken too, so it lands nowhere else.
pub(in crate::app) fn released_tag(response: &egui::Response) -> Option<Arc<DraggedTagRef>> {
    match TagDrag::of(response.dnd_release_payload::<DraggedTagRef>(), response.dnd_release_payload::<DraggedItems>())? {
        TagDrag::Tag(tag) => Some(tag),
        TagDrag::Refused => None,
    }
}

/// The single tag being dragged anywhere, as a tool outside Baboon takes it.
pub(in crate::app) fn dragged_tag(ctx: &egui::Context) -> Option<Arc<DraggedTagRef>> {
    egui::DragAndDrop::payload::<DraggedTagRef>(ctx).or_else(|| {
        egui::DragAndDrop::payload::<DraggedItems>(ctx).and_then(|items| items.tag.clone().map(Arc::new))
    })
}

/// Start a drag from the Flat view row at `key`, carrying the selection, or
/// just this item when it is not selected (it becomes the selection). `tag`
/// is the row's tag. Returns `false` outside Flat view, where the row starts
/// its own drag.
pub(super) fn start(ui: &Ui, response: &egui::Response, key: &str, tag: Option<DraggedTagRef>) -> bool {
    let Some((kit, pane, keys)) = drag_selection(ui, key) else {
        return false;
    };
    let entries = operation_entries(ui);
    let items: Vec<DraggedItem> = keys
        .into_iter()
        .filter_map(|key| {
            if let Some(path) = key.strip_prefix(FOLDER_PANE_PREFIX) {
                let path = path.replace('/', "\\");
                return Some(DraggedItem { key, path, folder: true });
            }
            let entry = entries.as_ref()?.iter().find(|entry| entry.key == key)?;
            Some(DraggedItem { path: entry.display_path.replace('/', "\\"), key, folder: false })
        })
        .collect();
    if items.is_empty() {
        return false;
    }
    let tag = tag.filter(|_| items.len() == 1 && !items[0].folder);
    response.dnd_set_drag_payload(DraggedItems { kit, pane, items, tag });
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

/// What dropping `drag` into `folder` (relative, `\`-separated) of `kit`
/// would do: the label beside the pointer, or why it can't.
pub(super) fn verdict(drag: &DraggedItems, kit: KitId, folder: &str, writable: bool, copy: bool) -> Result<String, String> {
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
    let parent = |item: &DraggedItem| tag_path::parent(&item.path).unwrap_or("").to_ascii_lowercase();
    if !copy && drag.items.iter().all(|item| parent(item) == lower) {
        return Err("Already in this folder".into());
    }
    let what = match drag.items.as_slice() {
        [item] => tag_path::name(&item.path).to_owned(),
        items => format!("{} items", items.len()),
    };
    let target = if folder.is_empty() { "the tags folder".to_owned() } else { tag_path::name(folder).to_owned() };
    Ok(format!("{} {what} to {target}", if copy { "Copy" } else { "Move" }))
}

/// Shown beside the pointer this frame by [`draw_drag_label`]: the target's
/// verdict, set by the first target the pointer is over.
#[derive(Clone)]
struct Hint {
    frame: u64,
    verdict: Result<String, String>,
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
    let claimed = ctx.data(|data| data.get_temp::<Hint>(hint_id())).is_some_and(|hint| hint.frame == frame);
    if claimed || !over {
        return None;
    }
    let drag = egui::DragAndDrop::payload::<DraggedItems>(ctx)?;
    let copy = is_copy(ctx);
    let folder_text = folder_path(folder);
    let verdict = verdict(&drag, kit, &folder_text, writable, copy);
    if verdict.is_ok() {
        painter.rect_stroke(rect.shrink(1.0), 3.0, Stroke::new(1.5, foundation_blue()), egui::StrokeKind::Inside);
    }
    let allowed = verdict.is_ok();
    ctx.data_mut(|data| data.insert_temp(hint_id(), Hint { frame, verdict }));
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

/// Once a frame, after the panes and tabs: while a Flat view drag is under
/// way, a label beside the pointer saying what dropping here would do, and
/// the matching cursor.
pub(in crate::app) fn draw_drag_label(ctx: &egui::Context) {
    let Some(drag) = egui::DragAndDrop::payload::<DraggedItems>(ctx) else {
        return;
    };
    let Some(pointer) = ctx.pointer_latest_pos() else { return };
    let frame = ctx.cumulative_frame_nr();
    let hint = ctx.data(|data| data.get_temp::<Hint>(hint_id())).filter(|hint| hint.frame == frame);
    let copy = is_copy(ctx);
    let (text, cursor, error) = match hint.map(|hint| hint.verdict) {
        Some(Ok(text)) => (text, if copy { egui::CursorIcon::Copy } else { egui::CursorIcon::Grabbing }, false),
        Some(Err(reason)) => (reason, egui::CursorIcon::NotAllowed, true),
        None => {
            let what = match drag.items.as_slice() {
                [item] => tag_path::name(&item.path).to_owned(),
                items => format!("{} items", items.len()),
            };
            (format!("{} {what}", if copy { "Copy" } else { "Move" }), egui::CursorIcon::Grabbing, false)
        }
    };
    ctx.set_cursor_icon(cursor);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("browser_drag_label")));
    let font = egui::TextStyle::Body.resolve(&ctx.global_style());
    let color = if error { ctx.global_style().visuals.error_fg_color } else { text_dark() };
    let galley = painter.layout_no_wrap(text, font, color);
    let at = pointer + egui::vec2(16.0, 18.0);
    let rect = egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(8.0, 4.0));
    painter.rect(rect, 4.0, foundation_block_bar(), Stroke::new(1.0, foundation_block_edge()), egui::StrokeKind::Inside);
    painter.galley(at, galley, color);
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
                .map(|(path, folder)| DraggedItem { key: (*path).into(), path: (*path).into(), folder: *folder })
                .collect(),
            tag: None,
        }
    }

    #[test]
    fn reference_fields_take_one_dragged_tag_and_refuse_the_rest() {
        let tag = DraggedTagRef { group_tag: 1, input: "x".into(), rel_path: "x".into(), file_path: None };
        let one = DraggedItems { tag: Some(tag.clone()), ..drag(&[("a.model", false)]) };
        assert!(matches!(TagDrag::of(None, Some(Arc::new(one))), Some(TagDrag::Tag(_))));
        for refused in [drag(&[("a.model", false), ("b.model", false)]), drag(&[("folder", true)])] {
            assert!(matches!(TagDrag::of(None, Some(Arc::new(refused))), Some(TagDrag::Refused)));
        }
        // A tree view's tag drag, as before.
        assert!(matches!(TagDrag::of(Some(Arc::new(tag)), None), Some(TagDrag::Tag(_))));
        assert!(TagDrag::of(None, None).is_none());
    }

    #[test]
    fn a_drop_says_what_it_will_do_or_why_it_cannot() {
        let one = drag(&[(r"objects\characters\grunt\grunt.biped", false)]);
        let some = drag(&[(r"objects\characters\grunt", true), (r"objects\characters\elite", true)]);
        let covenant = r"objects\characters\covenant";
        assert_eq!(verdict(&one, KitId(1), covenant, true, false).unwrap(), "Move grunt.biped to covenant");
        assert_eq!(verdict(&some, KitId(1), covenant, true, true).unwrap(), "Copy 2 items to covenant");
        assert_eq!(verdict(&some, KitId(1), "", true, false).unwrap(), "Move 2 items to the tags folder");
        assert_eq!(verdict(&one, KitId(2), covenant, true, false).unwrap_err(), "Drop onto a folder in the same kit");
        assert_eq!(verdict(&one, KitId(1), covenant, false, false).unwrap_err(), "This kit can't be changed");
        for inside in [r"objects\characters\grunt", r"OBJECTS\characters\grunt\ai"] {
            assert_eq!(verdict(&some, KitId(1), inside, true, false).unwrap_err(), "A folder can't go inside itself");
        }
        // A folder whose name merely starts the same is somewhere else.
        assert!(verdict(&some, KitId(1), r"objects\characters\grunt_v2", true, false).is_ok());
        let here = r"objects\characters";
        assert_eq!(verdict(&some, KitId(1), here, true, false).unwrap_err(), "Already in this folder");
        assert_eq!(verdict(&some, KitId(1), here, true, true).unwrap(), "Copy 2 items to characters", "copying in place duplicates");
    }
}
