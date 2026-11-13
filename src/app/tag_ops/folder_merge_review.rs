//! Operation-local review and cancellation shared with the folder worker.
use super::*;
use crate::core::folder_merge::{Choice, Conflict};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

#[derive(Default)]
pub(in crate::app) struct MergeControl {
    state: AtomicU8,
}
impl MergeControl {
    pub fn cancelled(&self) -> bool {
        self.state.load(Ordering::SeqCst) == 1
    }
    pub fn is_applying(&self) -> bool {
        self.state.load(Ordering::SeqCst) == 2
    }
    pub fn cancel(&self) {
        let _ = self
            .state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
    }
    pub fn begin_apply(&self) -> Result<(), String> {
        self.state
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| "Folder operation cancelled; no files changed".into())
    }
}
pub(in crate::app) struct MergeUi {
    pub control: Arc<MergeControl>,
    pub review: Option<MergeReview>,
    /// A bulk delete or move whose review is still being checked: the review
    /// window opens at once with the selected items, and fills in the rest
    /// when the check sends its [`ReviewKind::Delete`] or [`ReviewKind::Move`].
    pub items_preview: Option<ItemsPreview>,
}

/// Which selection review: a bulk delete, or a bulk move into `destination`
/// (a display path).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) enum ItemsAction {
    Delete,
    Move { destination: String },
}

/// What a bulk delete or move review shows before its check finishes.
pub(in crate::app) struct ItemsPreview {
    pub action: ItemsAction,
    pub selected: Vec<(String, crate::app::browser::bulk_rename::RenameIcon)>,
    pub game: Option<GameId>,
}

/// What the check adds to the review: the referring tags (left with missing
/// references by a delete, fixed by a move), what a delete removes (tags,
/// other files, folders) and how many moved items clash with the destination.
struct ItemsCheck<'a> {
    referrers: &'a [(String, u32)],
    /// The scripts with paths to the moved or deleted tags.
    scripts: &'a crate::app::browser::bulk_rename::ScriptRows,
    counts: (usize, usize, usize),
    conflicts: usize,
}
pub(in crate::app) struct MergeReview {
    pub kind: ReviewKind,
    pub answer: Sender<Option<MergeAnswer>>,
    pub apply_remaining: bool,
}
pub(in crate::app) enum ReviewKind {
    BatchSummary { sources: Vec<PathBuf>, destination: PathBuf, files: usize, conflicts: usize },
    /// A bulk delete: the selected items as icon rows, how much they hold,
    /// and every tag outside the selection that will lose a reference.
    Delete {
        selected: Vec<(String, crate::app::browser::bulk_rename::RenameIcon)>,
        referrers: Vec<(String, u32)>,
        /// The scripts that will have missing paths.
        scripts: crate::app::browser::bulk_rename::ScriptRows,
        game: Option<GameId>,
        tags: usize,
        files: usize,
        folders: usize,
    },
    /// A bulk move: the selected items, the folder they move to, every tag
    /// whose references the move fixes, and how many items clash with what
    /// is already there.
    Move {
        selected: Vec<(String, crate::app::browser::bulk_rename::RenameIcon)>,
        referrers: Vec<(String, u32)>,
        /// The scripts whose paths the move fixes.
        scripts: crate::app::browser::bulk_rename::ScriptRows,
        game: Option<GameId>,
        destination: String,
        conflicts: usize,
    },
    Summary {
        source: PathBuf,
        destination: PathBuf,
        files: usize,
        conflicts: usize,
    },
    Conflict {
        conflict: Conflict,
        incoming_references: Option<usize>,
        existing_references: Option<usize>,
    },
}
/// The bulk delete review's title, also shown while the review is prepared:
/// nothing is deleted until the user answers it.
pub(in crate::app) const DELETE_REVIEW_TITLE: &str = "Delete Selected Items?";
/// The bulk move review's title.
pub(in crate::app) const MOVE_REVIEW_TITLE: &str = "Move Selected Items?";

pub(in crate::app) struct MergeAnswer {
    pub choice: Option<Choice>,
    pub apply_remaining: bool,
}

pub(in crate::app) fn ask(
    tx: &Sender<WorkerMessage>,
    control: &MergeControl,
    kind: ReviewKind,
) -> Result<MergeAnswer, String> {
    let (answer, rx) = mpsc::channel();
    tx.send(WorkerMessage::FolderMergeReview(MergeReview {
        kind,
        answer,
        apply_remaining: false,
    }))
    .map_err(|_| "Folder review closed".to_owned())?;
    loop {
        if control.cancelled() {
            return Err("Folder operation cancelled; no files changed".into());
        }
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(Some(answer)) => return Ok(answer),
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Folder operation cancelled; no files changed".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn file_details(
    ui: &mut Ui,
    path: &Path,
    info: &crate::core::folder_merge::Fingerprint,
    references: Option<usize>,
) {
    ui.add(egui::Label::new(RichText::new(path.display().to_string()).monospace()).wrap());
    ui.label(if info.directory {
        "Folder".into()
    } else {
        format!("{} bytes", info.size)
    });
    if let Some(time) = info.modified {
        // Match the folder browser's local date format.
        if let Ok(duration) = time.duration_since(std::time::UNIX_EPOCH) {
            ui.label(format!(
                "Modified: {}",
                crate::app::tag_ops::folder_merge_review::date_label(duration.as_secs())
            ));
        }
    }
    ui.label(
        references
            .map(|n| format!("Referenced by {n} other tags"))
            .unwrap_or_else(|| "References: Not applicable".into()),
    );
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// What a bulk delete removes, as a sentence: "3 tags and 1 other file, in
/// 2 folders".
fn delete_contents(tags: usize, files: usize, folders: usize) -> String {
    let mut parts = vec![plural(tags, "tag", "tags")];
    if files > 0 {
        parts.push(plural(files, "other file", "other files"));
    }
    let mut text = parts.join(" and ");
    if folders > 0 {
        text.push_str(&format!(", in {}", plural(folders, "folder", "folders")));
    }
    text
}

/// The bulk delete and move reviews, in the Rename and Delete windows'
/// style. Until the check is in (`check` is `None`) the references box shows
/// a spinner with the check's `phase`, and the action is unavailable. Returns
/// `Some(true)` to go ahead and `Some(false)` for Cancel.
fn draw_items_review(
    ui: &mut Ui,
    ctx: &egui::Context,
    action: &ItemsAction,
    selected: &[(String, crate::app::browser::bulk_rename::RenameIcon)],
    check: Option<ItemsCheck>,
    game: Option<GameId>,
    phase: Option<&str>,
) -> Option<bool> {
    use crate::app::browser::bulk_rename::{draw_folder_band, draw_items_box, draw_references_box, missing_references_title, references_box_height, references_to_fix_title, rename_box};
    let moving = matches!(action, ItemsAction::Move { .. });
    let (title, icon) = if moving { (MOVE_REVIEW_TITLE, ButtonIcon::Move) } else { (DELETE_REVIEW_TITLE, ButtonIcon::Garbage) };
    crate::app::search::draw_icon_window_header_without_close(ui, title, icon);
    if let ItemsAction::Move { destination } = action {
        draw_folder_band(ui, destination);
    }
    ui.add_space(12.0);
    crate::app::browser::bulk_rename::padded_body(ui, |ui| {
        if moving {
            ui.label("Moves the selected items into this folder.");
            ui.label(RichText::new("Tags that reference them are updated to their new paths.").color(subtle_dark()));
            // Always one line, so the window keeps its size when the check
            // fills in.
            match check.as_ref().map(|check| check.conflicts) {
                None => ui.label(RichText::new("Checking for name conflicts…").color(subtle_dark())),
                Some(0) => ui.label(RichText::new("Nothing in this folder has the same name.").color(subtle_dark())),
                Some(conflicts) => ui.label(RichText::new(format!(
                    "{} the same name as something already here. You'll choose to keep both, skip or replace each one before anything moves.",
                    if conflicts == 1 { "1 item has".to_owned() } else { format!("{conflicts} items have") }
                )).color(ui.visuals().warn_fg_color)),
            };
        } else {
            let contents = match &check {
                Some(ItemsCheck { counts: (tags, files, folders), .. }) => delete_contents(*tags, *files, *folders),
                None => "the selected items".to_owned(),
            };
            ui.label(format!("This deletes {contents}, including subfolders and empty folders."));
            // Muted, like the single Delete windows' note about recovery.
            ui.label(
                RichText::new("Everything is moved together into Baboon's deleted-tags folder so it can be recovered by hand.")
                    .color(subtle_dark()),
            );
        }
        ui.add_space(14.0);
        // Side by side like Bulk Rename: the selected items on the left, the
        // referring tags on the right. Both show 8 rows and scroll past that,
        // so the window keeps its size when the check fills in.
        let notes: [String; 0] = [];
        let fits = (window_work_area(ctx).height() - 300.0).max(120.0);
        let height = references_box_height(8, 0, fits);
        let gap = 20.0;
        let column_width = ((ui.available_width() - gap) / 2.0).floor();
        let title = if moving {
            plural(selected.len(), "Item to Move", "Items to Move")
        } else {
            plural(selected.len(), "Item to Delete", "Items to Delete")
        };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(Vec2::new(column_width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(column_width);
                draw_items_box(ui, "bulk_review_items", &title, selected, &[], game, height);
            });
            ui.allocate_ui_with_layout(Vec2::new(column_width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(column_width);
                match &check {
                    Some(check) => {
                        let count = check.scripts.total(check.referrers.len());
                        let title = if moving { references_to_fix_title(count) } else { missing_references_title(count) };
                        draw_references_box(ui, &title, check.referrers, check.scripts, &notes, game, height);
                    }
                    None => rename_box(ui, height, |ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new(if moving { "Checking References to Fix…" } else { "Checking for Missing References…" }).strong());
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(phase.unwrap_or("Checking references")).color(subtle_dark()));
                        });
                    }),
                }
            });
        });
    });
    ui.add_space(12.0);
    let mut choice = None;
    popup_footer(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked() {
                choice = Some(false);
            }
            let go = if moving {
                icon_text_button(ui, ButtonIcon::Move, "Move", check.is_some())
            } else {
                accent_icon_text_button(ui, ButtonAccent::Destructive, ButtonIcon::Garbage, "Delete", check.is_some())
            };
            if go.on_disabled_hover_text("Available once the reference check finishes.").clicked() {
                choice = Some(true);
            }
        });
    });
    choice
}

fn date_label(seconds: u64) -> String {
    // Use the folder browser's platform-aware local date formatter.
    crate::app::browser::local_date(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
        .unwrap_or_else(|| "Unavailable".into())
}

/// The open review, or a bulk delete review still being checked. `phase` is
/// the check's progress, shown while it runs.
pub(in crate::app) fn draw(ctx: &egui::Context, state: &mut MergeUi, phase: Option<&str>) {
    let Some(review) = state.review.as_mut() else {
        let Some(preview) = &state.items_preview else {
            return;
        };
        // The same window as the finished review (same id), so it fills in
        // place when the check arrives.
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::unique("folder_merge_review"))
            .frame(egui::Frame::window(&ctx.global_style()))
            .show(ctx, |ui| {
                ui.set_width(window_width(ctx, 880.0));
                cancel = draw_items_review(ui, ctx, &preview.action, &preview.selected, None, preview.game, phase) == Some(false);
            });
        if cancel || modal.should_close() {
            state.control.cancel();
            state.items_preview = None;
        }
        return;
    };
    let mut response = None;
    let modal = egui::Modal::new(egui::Id::unique("folder_merge_review"))
        .frame(egui::Frame::window(&ctx.global_style()))
        .show(ctx, |ui| {
            let items = match &review.kind {
                ReviewKind::Delete { selected, referrers, scripts, game, tags, files, folders } => Some((
                    ItemsAction::Delete, selected, *game,
                    ItemsCheck { referrers, scripts, counts: (*tags, *files, *folders), conflicts: 0 },
                )),
                ReviewKind::Move { selected, referrers, scripts, game, destination, conflicts } => Some((
                    ItemsAction::Move { destination: destination.clone() }, selected, *game,
                    ItemsCheck { referrers, scripts, counts: (0, 0, 0), conflicts: *conflicts },
                )),
                _ => None,
            };
            if let Some((action, selected, game, check)) = items {
                ui.set_width(window_width(ctx, 880.0));
                response = draw_items_review(ui, ctx, &action, selected, Some(check), game, None)
                    .map(|go| go.then_some(MergeAnswer { choice: None, apply_remaining: false }));
                return;
            }
            ui.set_width(window_width(ctx, 560.0));
            crate::app::search::draw_icon_window_header_without_close(ui, match &review.kind { ReviewKind::BatchSummary {..} => "Review Selected Items", _ => "Merge Folders" }, ButtonIcon::FolderClosed);
            ui.add_space(12.0);
            ScrollArea::vertical().id_salt("folder_merge_review_scroll")
                .max_height((window_work_area(ctx).height() - 180.0).max(100.0))
                .show(ui, |ui| {
                    match &review.kind {
                        ReviewKind::Delete { .. } | ReviewKind::Move { .. } => {}
                        ReviewKind::BatchSummary { sources, destination, files, conflicts } => {
                            ui.label(format!("{files} incoming files, {conflicts} conflicts"));
                            ui.label("From:"); for source in sources { ui.label(RichText::new(source.display().to_string()).monospace()); }
                            ui.label("To:"); ui.label(RichText::new(destination.display().to_string()).monospace());
                        }
                        ReviewKind::Summary { source, destination, files, conflicts } => {
                            ui.label("Review this operation before changing files.");
                            ui.add_space(8.0);
                            ui.label(RichText::new("From").small().color(subtle_dark()));
                            ui.add(egui::Label::new(RichText::new(source.display().to_string()).monospace()).wrap());
                            ui.add_space(8.0);
                            ui.label(RichText::new("To").small().color(subtle_dark()));
                            ui.add(egui::Label::new(RichText::new(destination.display().to_string()).monospace()).wrap());
                            ui.add_space(8.0);
                            ui.label(format!("{files} incoming file(s), {conflicts} conflict(s)"));
                            ui.add_space(8.0);
                            ui.label(RichText::new("Matching subfolders will merge. Existing destination contents stay in place unless you choose Replace. Replaced files are preserved in Deleted Tags.").color(subtle_dark()));
                        }
                        ReviewKind::Conflict { conflict, incoming_references, existing_references } => {
                            ui.columns(2, |columns| {
                                columns[0].strong("Incoming");
                                file_details(&mut columns[0], &conflict.incoming, &conflict.incoming_info, *incoming_references);
                                columns[1].strong("Existing");
                                file_details(&mut columns[1], &conflict.existing, &conflict.existing_info, *existing_references);
                            });
                            ui.add_space(8.0);
                            if !conflict.replace_allowed() { ui.label("A file and a folder have the same name. Replace is unavailable."); }
                            ui.baboon_checkbox(&mut review.apply_remaining, "Apply this choice to all remaining conflicts");
                        }
                    }
                });
            ui.add_space(12.0);
            popup_footer(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked() { response = Some(None); }
                    match &review.kind {
                        ReviewKind::Summary { .. } | ReviewKind::BatchSummary { .. } | ReviewKind::Delete { .. } | ReviewKind::Move { .. } => {
                            if icon_text_button(ui, ButtonIcon::Confirm, "Continue", true).clicked() {
                                response = Some(Some(MergeAnswer { choice: None, apply_remaining: false }));
                            }
                        }
                        ReviewKind::Conflict { conflict, .. } => {
                            for (choice, label, icon) in [(Choice::KeepBoth, "Keep Both", ButtonIcon::Duplicate), (Choice::Skip, "Skip", ButtonIcon::Closed), (Choice::Replace, "Replace", ButtonIcon::Replace)] {
                                if icon_text_button(ui, icon, label, choice != Choice::Replace || conflict.replace_allowed()).clicked() {
                                    response = Some(Some(MergeAnswer { choice: Some(choice), apply_remaining: review.apply_remaining }));
                                }
                            }
                        }
                    }
                });
            });
        });
    if modal.should_close() {
        response = Some(None);
    }
    if let Some(answer) = response {
        if answer.is_none() {
            state.control.cancel();
        }
        let review = state.review.take().unwrap();
        let _ = review.answer.send(answer);
        // Answered: from here the delete runs, or stops, under the lock.
        state.items_preview = None;
    }
}

#[derive(Clone)]
pub(in crate::app) struct SaveBeforeFolderOperation {
    pub kit: KitStamp,
    pub rel_path: PathBuf,
    pub label: String,
    pub moving: bool,
    pub error: Option<String>,
}
impl Dialog for SaveBeforeFolderOperation {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let mut open = true;
        egui::Window::new("Save Before Moving or Copying")
            .id(egui::Id::unique("save_before_folder_operation")).open(&mut open).resizable(false)
            .show(cx.egui, |ui| {
                ui.label("Save all unsaved tags in this workspace before checking references and moving or copying folders.");
                if let Some(error) = &self.error { ui.colored_label(material_delete_text(), error); }
                if ui.button("Save All and Continue").clicked() { cx.send(TagOpsCommand::SaveBeforeFolderOperation); }
                if ui.button("Cancel").clicked() { cx.send(TagOpsCommand::CancelFolderOperationSave); }
            });
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_apply_have_one_atomic_boundary() {
        let cancelled = MergeControl::default();
        cancelled.cancel();
        assert!(cancelled.cancelled());
        assert!(cancelled.begin_apply().is_err());
        let applying = MergeControl::default();
        applying.begin_apply().unwrap();
        applying.cancel();
        assert!(applying.is_applying());
        assert!(!applying.cancelled());
    }

    #[test]
    fn conflict_review_shows_both_reference_counts_and_sends_keep_both_choice() {
        let root =
            std::env::temp_dir().join(format!("baboon-merge-review-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let incoming = root.join("incoming.weapon");
        let existing = root.join("existing.weapon");
        fs::write(&incoming, b"incoming bytes").unwrap();
        fs::write(&existing, b"existing bytes").unwrap();
        let (answer, rx) = mpsc::channel();
        let mut state = MergeUi {
            items_preview: None,
            control: Arc::new(MergeControl::default()),
            review: Some(MergeReview {
                kind: ReviewKind::Conflict {
                    conflict: Conflict {
                        incoming_info: crate::core::folder_merge::fingerprint(&incoming).unwrap(),
                        existing_info: crate::core::folder_merge::fingerprint(&existing).unwrap(),
                        incoming,
                        existing,
                    },
                    incoming_references: Some(2),
                    existing_references: Some(3),
                },
                answer,
                apply_remaining: true,
            }),
        };
        let ctx = egui::Context::default();
        ctx.set_global_style(foundation_style());
        let mut time = 0.0;
        let mut frame = |state: &mut MergeUi, events| {
            time += 0.1;
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 800.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |_| draw(&ctx, state, None),
            );
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        frame(&mut state, vec![]);
        let painted = frame(&mut state, vec![]);
        for label in [
            "Incoming",
            "Existing",
            "Referenced by 2 other tags",
            "Referenced by 3 other tags",
            "Apply this choice to all remaining conflicts",
        ] {
            assert!(
                painted.iter().any(|(text, _)| text == label),
                "missing {label}"
            );
        }
        let position = painted
            .iter()
            .find(|(text, _)| text == "Keep Both")
            .unwrap()
            .1
            .center();
        frame(&mut state, vec![egui::Event::PointerMoved(position)]);
        for pressed in [true, false] {
            frame(
                &mut state,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        let answer = rx.try_recv().unwrap().unwrap();
        assert_eq!(answer.choice, Some(Choice::KeepBoth));
        assert!(answer.apply_remaining);
        assert!(state.review.is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
