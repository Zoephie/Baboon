//! The Content Explorer window: one tag's referrers and references, walked
//! one click at a time.

use super::*;
use crate::app::search::result_windows::fixed_height_row;
use crate::app::shell::frame::explorer_entry_row;

/// The Content Explorer window, while it is open.
pub(in crate::app) fn draw_content_explorer_window(cx: &Ctx, references: &mut ReferencesFeature) {
    if references.content_explorer.is_none() {
        return;
    }
    let ctx = cx.egui;
    let mut open = true;
    let mut act: Option<ExplorerAct> = None;
    let explorer_kit = references
        .content_explorer
        .as_ref()
        .map(|explorer| explorer.kit)
        .expect("checked above");
    let explorer_kit_index = cx.model.resolve_kit(explorer_kit).unwrap_or(cx.model.active);
    let mut filter = references
        .content_explorer
        .as_ref()
        .map(|explorer| explorer.filter.clone())
        .unwrap_or_default();
    {
        let explorer = references.content_explorer.as_ref().expect("checked above");
        egui::Window::new("Content Explorer")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("content_explorer"))
            .open(&mut open)
            .default_width(window_width(ctx, 720.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!explorer.back.is_empty(), egui::Button::new("← Back"))
                        .clicked()
                    {
                        act = Some(ExplorerAct::Back);
                    }
                    if ui
                        .add_enabled(
                            !explorer.forward.is_empty(),
                            egui::Button::new("Forward →"),
                        )
                        .clicked()
                    {
                        act = Some(ExplorerAct::Forward);
                    }
                    ui.separator();
                    if ui.button("Open in editor").clicked() {
                        act = Some(ExplorerAct::Open(explorer.focus.key.clone()));
                    }
                    if ui.button("Reveal in browser").clicked() {
                        act = Some(ExplorerAct::Reveal(explorer.focus.key.clone()));
                    }
                    ui.separator();
                    ui.add(
                        egui::TextEdit::singleline(&mut filter)
                            .hint_text(placeholder_text("filter"))
                            .desired_width(140.0),
                    );
                });
                ui.separator();
                ui.label(
                    RichText::new(explorer.focus.display_path.replace('\\', "/"))
                        .strong()
                        .color(text_dark()),
                );
                if explorer.index_unavailable {
                    let note = if cx.model.kits[cx.model.active].index_jobs.building_references
                        || cx.model.kits[explorer_kit_index].scanning_entries
                    {
                        "Reference index is building — reopen this in a moment."
                    } else {
                        "Reference index unavailable — run Tools → Build Reference Index."
                    };
                    ui.label(RichText::new(note).color(subtle_dark()));
                }
                ui.separator();
                let query = filter.trim();
                let matches =
                    |entry: &TagEntry| contains_ignore_ascii_case(&entry.display_path, query);
                let parents: Vec<&TagEntry> =
                    explorer.parents.iter().filter(|e| matches(e)).collect();
                let children: Vec<&TagEntry> =
                    explorer.children.iter().filter(|e| matches(e)).collect();
                let count_label = |shown: usize, total: usize| {
                    if shown == total {
                        format!("({total})")
                    } else {
                        format!("({shown}/{total})")
                    }
                };
                ui.columns(2, |cols| {
                    cols[0].label(
                        RichText::new(format!(
                            "Referenced by {}",
                            count_label(parents.len(), explorer.parents.len())
                        ))
                        .strong()
                        .color(text_dark()),
                    );
                    if parents.is_empty() {
                        cols[0].label(RichText::new("(none)").color(subtle_dark()));
                    }
                    // A widely used tag has thousands of referrers: only
                    // the rows in view are drawn.
                    let row_height = cols[0].spacing().interact_size.y;
                    egui::ScrollArea::vertical()
                        .id_salt("ce_parents")
                        .max_height(380.0)
                        .show_rows(&mut cols[0], row_height, parents.len(), |ui, rows| {
                            for entry in &parents[rows] {
                                if fixed_height_row(ui, row_height, |ui| {
                                    explorer_entry_row(ui, entry)
                                }) {
                                    act = Some(ExplorerAct::Navigate((*entry).clone()));
                                }
                            }
                        });
                    cols[1].label(
                        RichText::new(format!(
                            "References {}",
                            count_label(children.len(), explorer.children.len())
                        ))
                        .strong()
                        .color(text_dark()),
                    );
                    if children.is_empty() {
                        cols[1].label(RichText::new("(none)").color(subtle_dark()));
                    }
                    // A widely used tag has thousands of referrers: only
                    // the rows in view are drawn.
                    let row_height = cols[1].spacing().interact_size.y;
                    egui::ScrollArea::vertical()
                        .id_salt("ce_children")
                        .max_height(380.0)
                        .show_rows(&mut cols[1], row_height, children.len(), |ui, rows| {
                            for entry in &children[rows] {
                                if fixed_height_row(ui, row_height, |ui| {
                                    explorer_entry_row(ui, entry)
                                }) {
                                    act = Some(ExplorerAct::Navigate((*entry).clone()));
                                }
                            }
                        });
                });
            });
    }
    if let Some(explorer) = references.content_explorer.as_mut() {
        explorer.filter = filter;
    }
    if let Some(act) = act {
        cx.send(ReferencesCommand::Explorer {
            kit: explorer_kit,
            act,
        });
    }
    if !open {
        references.content_explorer = None;
    }
}

impl Baboon {
    /// Carry out what the Content Explorer over `kit` asked for. The graph
    /// belongs to one kit; go back to it before acting, and close the window
    /// if that kit has gone.
    pub(in crate::app) fn apply_explorer_act(&mut self, kit: KitId, act: ExplorerAct, ctx: &egui::Context) {
        if !self.focus_navigation_kit(kit) {
            self.references.content_explorer = None;
            self.model.status = "That workspace has been closed".to_owned();
            return;
        }
        match act {
            ExplorerAct::Navigate(entry) => self.content_explorer_navigate(entry),
            ExplorerAct::Back => self.content_explorer_back(),
            ExplorerAct::Forward => self.content_explorer_forward(),
            ExplorerAct::Open(key) => self.select_entry(key, ctx.clone()),
            ExplorerAct::Reveal(key) => self.reveal_in_browser(&key),
        }
    }
}

/// What the Content Explorer can ask for.
pub(in crate::app) enum ExplorerAct {
    Navigate(TagEntry),
    Back,
    Forward,
    Open(String),
    Reveal(String),
}
