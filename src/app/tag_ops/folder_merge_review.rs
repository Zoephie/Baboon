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
}
pub(in crate::app) struct MergeReview {
    pub kind: ReviewKind,
    pub answer: Sender<Option<MergeAnswer>>,
    pub apply_remaining: bool,
}
pub(in crate::app) enum ReviewKind {
    BatchSummary { sources: Vec<PathBuf>, destination: PathBuf, files: usize, conflicts: usize },
    Delete { items: Vec<(PathBuf,Option<usize>)>, folders: Vec<PathBuf>, tags: usize },
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

fn date_label(seconds: u64) -> String {
    // Use the folder browser's platform-aware local date formatter.
    crate::app::browser::local_date(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
        .unwrap_or_else(|| "Unavailable".into())
}

pub(in crate::app) fn draw(ctx: &egui::Context, state: &mut MergeUi) {
    let Some(review) = state.review.as_mut() else {
        return;
    };
    let mut response = None;
    let modal = egui::Modal::new(egui::Id::unique("folder_merge_review"))
        .frame(egui::Frame::window(&ctx.global_style()))
        .show(ctx, |ui| {
            ui.set_width(window_width(ctx, 560.0));
            crate::app::search::draw_icon_window_header_without_close(ui, match &review.kind { ReviewKind::Delete {..} => "Delete Selected Items", ReviewKind::BatchSummary {..} => "Review Selected Items", _ => "Merge Folders" }, ButtonIcon::FolderClosed);
            ui.add_space(12.0);
            ScrollArea::vertical().id_salt("folder_merge_review_scroll")
                .max_height((window_work_area(ctx).height() - 180.0).max(100.0))
                .show(ui, |ui| {
                    match &review.kind {
                        ReviewKind::Delete { items, folders, tags } => {
                            ui.label(format!("Delete {tags} tags and {} other files?",items.len()-tags));
                            ui.label(format!("{} folder(s), including their contents and empty subfolders", folders.len()));
                            ui.label("Files will be preserved together in Deleted Tags. Reference counts below exclude tags in this deletion.");
                            for folder in folders { ui.label(RichText::new(format!("{}{}", folder.display(), std::path::MAIN_SEPARATOR)).monospace()); }
                            for (path,references) in items { ui.label(RichText::new(path.display().to_string()).monospace()); if let Some(count)=references { if *count>0 {ui.label(format!("Referenced by {count} tags outside this selection"));} } }
                        }
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
                        ReviewKind::Summary { .. } | ReviewKind::BatchSummary { .. } | ReviewKind::Delete { .. } => {
                            if icon_text_button(ui, ButtonIcon::Confirm, if matches!(&review.kind,ReviewKind::Delete{..}) {"Delete"} else {"Continue"}, true).clicked() {
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
                |_| draw(&ctx, state),
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
