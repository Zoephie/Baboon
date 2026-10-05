//! Main application shell: menus, toolbar, sidebar, tabs, terminal, and status areas.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;
use crate::app::shell::actions::pressed_shortcuts;
use crate::app::editor::DraftFlush;
use crate::app::shell::frame::terminal_line_is_strong;
use crate::app::shell::frame::terminal_line_color;
use crate::app::kits::terminal::open_terminal_log;
use crate::app::shell::frame::draw_index_progress_bar;

/// How often a progress bar is redrawn while its job runs.
const PROGRESS_REPAINT: std::time::Duration = std::time::Duration::from_millis(200);

impl Baboon {
    pub(in crate::app) fn draw_root_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        if self.dialogs.get::<FirstRunWizardState>().is_some() {
            // Setup draws alone, over an empty window.
            ctx.set_zoom_factor(self.model.prefs.ui_scale);
            set_dark_mode(self.model.prefs.dark_mode);
            ctx.set_visuals(foundation_visuals());
            egui::CentralPanel::default().show(ui, |_ui| {});
            self.dialogs.draw(&cx!(self, ctx), &app_reads!(self));
            self.apply_commands(ctx);
            return;
        }
        self.prepare_root_frame(ctx);

        let menu = self.menu_state(ctx);
        let active = self.model.kits[self.model.active].id;
        draw_menu_bar(&cx!(self, ctx), ui, &menu, &mut self.views[active]);
        draw_status_bar(
            &cx!(self, ctx),
            ui,
            self.export.container_dump_job.as_ref(),
            self.tag_ops.folder_refactor.as_ref(),
            self.shell.available_update.as_ref(),
        );
        // Terminal panel — rendered AFTER status so it sits above it.
        let active = self.model.kits[self.model.active].id;
        draw_terminal_panel(&cx!(self, ctx), ui, &mut self.kit_tools, &mut self.views[active]);
        set_window_work_area(ctx, ui.available_rect_before_wrap());

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(editor_bg()))
            .show(ui, |ui| {
                self.draw_workspace_tiles(ui, ctx);
            });
        self.draw_auxiliary_windows(ctx);
        // Every kit, not just the active one: a background kit's sidecar can be
        // dirty from edits made before the user switched away.
        let mut keyword_notice = None;
        for kit in &mut self.model.kits {
            kit.keywords.save_if_dirty();
            if let Some(notice) = kit.keywords.take_notice() {
                keyword_notice = Some(notice);
            }
        }
        if let Some(notice) = keyword_notice {
            self.model.status = notice;
        }
        // The draws' commands land first: some of them queue the very requests
        // processed next (the tag pane sends its sound player's plays this way),
        // and the next frame's draws have to see those requests settled, or
        // they ask again.
        self.apply_commands(ctx);
        self.process_frame_requests(ctx);
        self.apply_commands(ctx);
        // A field typed into and then not drawn on this pass -- its section
        // collapsed, its pane switched to another sub-tab or block element,
        // its tab closed or hidden -- never sees its box lose focus, so it
        // commits here instead of keeping the edit to itself.
        if self.commit_drafts(DraftFlush::NotDrawnOn(ctx.cumulative_pass_nr())) {
            ctx.request_repaint();
        }
    }

    /// The kit a confirmed popup applies to: the one it was opened from, or
    /// none if that kit has closed since, so the edit is dropped rather than
    /// landing in another kit's tag that happens to share its key. A popup
    /// with no recorded kit applies to the active one.
    pub(in crate::app) fn popup_target_kit(&mut self, opened_from: Option<KitId>) -> Option<usize> {
        match opened_from {
            Some(kit) => {
                let index = self.model.resolve_kit(kit);
                if index.is_none() {
                    self.model.status =
                        "The editing kit this was opened from has closed; the edit was dropped."
                            .to_owned();
                }
                index
            }
            None => Some(self.model.active),
        }
    }

    /// Settle what this frame queued after every window has drawn: prompts,
    /// pending opens and field navigation, and the sound drains.
    fn process_frame_requests(&mut self, ctx: &egui::Context) {
        self.process_pending_open(ctx);
        self.apply_field_nav(ctx);
        // A referenced sound on a container source resolves to its own Wwise
        // binding first, and queues an ordinary play/extract from there — so it
        // must run before both drains below, not after.
        self.process_ce_sound_ref();
        // Drain queued sound-player actions: resolve the permutation against the
        // FMOD banks, decode (cached), and play/stop. Runs every frame so voices
        // are reaped even when idle; the tags root is only cloned when acting.
        // Playback follows its tab: paused once another tab (or another kit)
        // has focus, disposed of once its tab is closed, whichever way that
        // happened. Checked against the open tabs rather than hooked into each
        // close path, so a close confirmed after the save prompt counts and a
        // cancelled one does not.
        let focus = self.model.kits.get(self.model.active).and_then(|kit| {
            kit.selected_key
                .clone()
                .map(|key| crate::app::audio::SoundOwner { kit: kit.id, key })
        });
        let kits = &self.model.kits;
        self.audio.follow_tabs(focus.as_ref(), |owner| {
            kits.iter()
                .find(|kit| kit.id == owner.kit)
                .is_some_and(|kit| kit.open_tabs.contains(&owner.key))
        });
        // And the players forget what they kept for a tab that is gone.
        crate::app::editor::forget_closed_players(ctx, |tag_key| {
            kits.iter()
                .any(|kit| kit.open_tabs.iter().any(|key| key == tag_key))
        });
        let sound_root = if !self.audio.pending.is_empty() {
            self.model.source_tags_root().map(std::path::Path::to_path_buf)
        } else {
            None
        };
        self.audio.process(sound_root.as_deref(), ctx);
        // A single interaction can queue a transport update before playback
        // (most notably H3's language fallback followed by Play). Draining only
        // one item left Play waiting for a repaint that might never arrive,
        // making the click silently do nothing. Preserve queue order, but
        // settle the whole interaction in this frame.
        while !self.audio.pending.is_empty() {
            self.audio.process(sound_root.as_deref(), ctx);
        }
        // Drain a queued sound extraction (decode + write files off the render
        // hot loop) and a reimport hand-off (opens the tool runner pre-filled).
        if let Some(request) = self.export.pending_sound_extract.take() {
            self.audio.run_extract(request, ctx);
            if let Some(status) = self.audio.status.clone() {
                self.model.status = status;
            }
        }
        self.process_pending_tool_import(ctx);
    }

    /// Clear the status line once its message has been up for a while.
    ///
    /// Runs after the worker drain so a message set this frame is timed from
    /// this frame. Progress states are rendered from their own fields rather
    /// than from `status`, so expiring it never blanks a running scan.
    pub(in crate::app) fn expire_status(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        if self.model.status != self.status_shown {
            self.status_shown = self.model.status.clone();
            self.status_changed_at = now;
        }
        if self.model.status.is_empty() {
            return;
        }
        let elapsed = now - self.status_changed_at;
        if elapsed >= STATUS_LINGER_SECS {
            self.model.status.clear();
            self.status_shown.clear();
        } else {
            // Nothing else may be animating, so ask for the frame that will
            // do the clearing rather than waiting for the next interaction.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                STATUS_LINGER_SECS - elapsed,
            ));
        }
    }

    pub(in crate::app) fn run_deferred_file_action(&mut self, ctx: &egui::Context) {
        // A save, poke or close reads the tags as they stand, so every field
        // still holding a typed change commits first. Usually the pass that
        // queued the action already committed it; but a window that is
        // minimized runs no UI pass at all, and the close is decided here.
        if self.editor.deferred_file_action.is_some() {
            self.commit_drafts(DraftFlush::All);
        }
        match self.editor.deferred_file_action.take() {
            Some(DeferredFileAction::SaveCurrentTag)
                if self.model.prefs.enable_chimp
                    && self.views[self.model.kits[self.model.active].id].surface == KitSurface::Chimp =>
            {
                self.open_chimp_save_dialog(self.model.active)
            }
            Some(DeferredFileAction::SaveCurrentTag) => self.save_current_tag(ctx),
            Some(DeferredFileAction::SaveProject) => {
                let (kit, now) = (self.model.active, ctx.input(|input| input.time));
                self.save_campaign_project_file(kit, now);
            }
            Some(DeferredFileAction::SaveProjectAs) => {
                let (kit, now) = (self.model.active, ctx.input(|input| input.time));
                self.save_campaign_project_file_as(kit, now);
            }
            Some(DeferredFileAction::ExportMod) => self.export_mod(),
            Some(DeferredFileAction::ExtractAllContainerTags) => {
                self.begin_extract_all_container_tags(ctx.clone())
            }
            Some(DeferredFileAction::PokeCurrentTag) => self.begin_poke_current_tag(ctx.clone()),
            Some(DeferredFileAction::Close(action)) => self.request_close_action(action, ctx),
            Some(DeferredFileAction::CloseCurrentTab)
                if self.model.prefs.enable_chimp
                    && self.views[self.model.kits[self.model.active].id].surface == KitSurface::Chimp =>
            {
                if let Some(package) = self.model.kits[self.model.active].chimp.selected_package.clone() {
                    let kit = self.model.active;
                    if !self.close_chimp_package(kit, &package) {
                        self.model.status =
                            "Save or discard modified Chimp packages before closing them."
                                .to_owned();
                    }
                }
            }
            Some(DeferredFileAction::CloseCurrentTab) => {
                if let Some(key) = self.model.kits[self.model.active].selected_key.clone() {
                    self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
                }
            }
            None => {}
        }
    }

    pub(in crate::app) fn defer_file_action(
        &mut self,
        action: DeferredFileAction,
        ctx: &egui::Context,
    ) {
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
        self.editor.deferred_file_action = Some(action);
        // It runs at the start of the next frame, which a window with
        // nothing else to do would otherwise never draw.
        ctx.request_repaint();
    }

    fn prepare_root_frame(&mut self, ctx: &egui::Context) {
        ctx.set_zoom_factor(self.model.prefs.ui_scale);
        self.handle_pixels_per_point_change(ctx);
        self.maybe_refresh_entry_index(ctx.clone());
        set_dark_mode(self.model.prefs.dark_mode);
        // Pushed the same way and for the same reason as the theme: the two
        // halves of the angle conversion are free functions on opposite sides
        // of the frame, and neither can reach `Baboon`.
        crate::core::format::set_angles_in_degrees(self.model.prefs.angles_in_degrees);
        ctx.set_visuals(foundation_visuals());
        set_combo_scroll_cycle_enabled(ctx, self.model.prefs.scroll_to_cycle_dropdowns);
        apply_scroll_speed(ctx, self.model.prefs.scroll_speed);
        set_zoom_speed(ctx, self.model.prefs.zoom_speed);
        // Opened before any pane draws and settled after the last one, so a
        // dropdown can only claim a gesture on the frame it began.
        begin_wheel_gesture(ctx);
        // A folder move or rename is rewriting tags on disk. Nothing may edit,
        // save or open them until it lands, so no shortcut or dropped file is
        // taken, and no text field keeps the keyboard.
        if self.tag_ops.folder_refactor.is_some() {
            ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
            return;
        }
        let shortcuts = pressed_shortcuts(ctx);
        // Save, close and poke act on the tags as they stand. Giving up the
        // focused field now, before any pane draws, lets it see the loss and
        // commit on this pass; given up when the action is applied, after
        // the draw, the field committed a frame after the save had run.
        if shortcuts.iter().any(|action| matches!(action, AppAction::Defer(_))) {
            ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
        }
        for action in shortcuts {
            self.commands.send(action);
        }
        self.refresh_find(ctx);
        let dropped_paths = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if !dropped_paths.is_empty() {
            self.open_dropped_files(dropped_paths, ctx.clone());
        }
        // The other direction: a browser drag that ends on Sapien's window.
        self.track_kit_tool_drop(ctx);
    }

    fn draw_auxiliary_windows(&mut self, ctx: &egui::Context) {
        self.diff_expanded_mod_export_rows();
        // Walk any expanded rows whose fields are not known yet before the
        // window reads them.
        self.refresh_ref_jump_occurrences(ctx);
        self.dialogs.draw(&cx!(self, ctx), &app_reads!(self));
        draw_folder_refactor_lock(ctx, self.tag_ops.folder_refactor.as_ref());
        end_wheel_gesture(ctx);
    }

}

pub(in crate::app) fn recent_folder_menu_label(path: &Path) -> String {
    const MAX_CHARS: usize = 54;
    let text = path.display().to_string();
    let count = text.chars().count();
    if count <= MAX_CHARS {
        return text;
    }
    let keep = MAX_CHARS.saturating_sub(3);
    let tail = text
        .chars()
        .rev()
        .take(keep)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("...{tail}")
}

fn terminal_line_text(line: &TerminalLineEntry) -> RichText {
    let text = RichText::new(&line.text).color(terminal_line_color(line.severity));
    if terminal_line_is_strong(line.severity) {
        text.font(bold_font(13.0)).strong()
    } else {
        text.monospace().font(FontId::monospace(13.0))
    }
}

/// The terminal's output lines, scrolled.
///
/// Only the lines in view are laid out and drawn. The rest are placed by
/// their wrapped heights, which each line keeps for the width it was last
/// wrapped at: a width change re-measures once, an appended line measures
/// itself. Drawing every line as a label, up to the 20,000 kept, cost 4.5 ms
/// a frame in a release build.
pub(in crate::app) fn draw_terminal_output(
    ui: &mut Ui,
    lines: &[TerminalLineEntry],
    want_scroll_bottom: bool,
) {
    let gap = ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt("terminal_output")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            ui.visuals_mut().override_text_color = None;
            ui.set_min_width(ui.available_width());
            let width = ui.available_width();
            let height_of = |line: &TerminalLineEntry| match line.wrapped.get() {
                Some((at, height)) if at == width => height,
                _ => {
                    let height = egui::WidgetText::from(terminal_line_text(line))
                        .into_galley(ui, Some(egui::TextWrapMode::Wrap), width, TextStyle::Body)
                        .size()
                        .y;
                    line.wrapped.set(Some((width, height)));
                    height
                }
            };
            // Where each line starts, from the top of the content.
            let mut top = 0.0;
            let mut first = None;
            let mut first_top = 0.0;
            let mut last = 0;
            for (index, line) in lines.iter().enumerate() {
                let bottom = top + height_of(line);
                if first.is_none() && bottom >= viewport.min.y {
                    first = Some(index);
                    first_top = top;
                }
                if top <= viewport.max.y {
                    last = index + 1;
                }
                top = bottom + gap;
            }
            let total = (top - gap).max(0.0);
            ui.set_height(total);
            let origin = ui.max_rect().top();
            if let Some(first) = first {
                let rect = egui::Rect::from_x_y_ranges(
                    ui.max_rect().x_range(),
                    origin + first_top..=origin + total,
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.skip_ahead_auto_ids(first);
                    for line in &lines[first..last.max(first)] {
                        #[cfg(test)]
                        tests::LINES_BUILT.with(|built| built.set(built.get() + 1));
                        ui.add(egui::Label::new(terminal_line_text(line)).wrap());
                    }
                });
            }
            if want_scroll_bottom {
                let bottom = egui::Rect::from_x_y_ranges(
                    ui.max_rect().x_range(),
                    origin + total..=origin + total,
                );
                ui.scroll_to_rect(bottom, Some(egui::Align::BOTTOM));
            }
        });
}

/// The status bar: the status line, index and job progress, the update
/// link and the workspace's project.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_status_bar(
    cx: &Ctx,
    ui: &mut egui::Ui,
    container_dump: Option<&ContainerDumpJob>,
    folder_refactor: Option<&FolderRefactorUiState>,
    available_update: Option<&UpdateCheckResult>,
) {
    let ctx = &ui.ctx().clone();
    egui::Panel::bottom("status")
        .frame(Frame::NONE.fill(menu_bar()).inner_margin(egui::Margin {
            left: 6,
            right: 6,
            top: 2,
            bottom: 2,
        }))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Status").strong());
                ui.separator();
                if cx.model.kits[cx.model.active].scanning_entries {
                    let progress = cx.model.kits[cx.model.active].index_jobs.entry_progress.as_ref();
                    let label = progress
                        .map(|progress| progress.label.as_str())
                        .unwrap_or("Indexing tags...");
                    ui.label(RichText::new(label).strong());
                    if let Some(progress) = progress {
                        let fraction = if progress.total == 0 {
                            0.0
                        } else {
                            progress.processed as f32 / progress.total as f32
                        };
                        let text = if progress.total == 0 {
                            "Discovering files...".to_owned()
                        } else {
                            format!(
                                "{} / {} files, {} tags",
                                progress.processed, progress.total, progress.matched
                            )
                        };
                        draw_index_progress_bar(ui, 260.0, Some(fraction), &text);
                    }
                } else if cx.model.kits[cx.model.active].index_jobs.building_references {
                    let progress = cx.model.kits[cx.model.active]
                        .index_jobs
                        .reference_progress
                        .as_ref();
                    let label = progress
                        .map(|progress| progress.label.as_str())
                        .unwrap_or("Building reference index...");
                    ui.label(RichText::new(label).strong());
                    if let Some(progress) = progress {
                        let fraction = if progress.total == 0 {
                            0.0
                        } else {
                            progress.processed as f32 / progress.total as f32
                        };
                        let text = format!("{} / {} tags", progress.processed, progress.total);
                        draw_index_progress_bar(ui, 260.0, Some(fraction), &text);
                    }
                } else {
                    ui.label(&cx.model.status);
                }
                // Additive rather than part of the chain above: the
                // extraction outlives whatever the user does next, and its
                // bar is the only place a cancel is reachable from.
                if let Some(job) = container_dump {
                    let (fraction, done, total) = (job.fraction(), job.done, job.total);
                    let remaining = job.remaining();
                    ui.separator();
                    ui.label(RichText::new("Extracting tags").strong())
                        // Where it is writing. The folder was chosen minutes
                        // ago in a native dialog and is nowhere else on
                        // screen once the confirm has closed.
                        .on_hover_text(format!("Writing to {}", job.output.display()));
                    draw_index_progress_bar(
                        ui,
                        220.0,
                        Some(fraction),
                        &format!("{done} / {total} tags"),
                    );
                    if let Some(remaining) = remaining {
                        ui.label(
                            RichText::new(format!("{} left", format_remaining(remaining)))
                                .color(subtle_dark())
                                .small(),
                        );
                    }
                    if ui.small_button("Cancel").clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    // A few times a second moves the bar and the estimate;
                    // every frame kept the app at full frame rate for the
                    // whole of a multi-minute extraction.
                    ctx.request_repaint_after(PROGRESS_REPAINT);
                }
                if let Some(progress) = folder_refactor {
                    ui.separator();
                    ui.label(RichText::new(&progress.label).strong());
                    let mut bar = if let Some(value) = progress.progress {
                        egui::ProgressBar::new(value.clamp(0.0, 1.0))
                    } else {
                        egui::ProgressBar::new(0.0).animate(true)
                    };
                    bar = bar
                        .desired_width(180.0)
                        .text(RichText::new(&progress.phase).color(text_dark()));
                    ui.add(bar);
                    // An indeterminate bar asks for its own frames while
                    // it animates.
                    ctx.request_repaint_after(PROGRESS_REPAINT);
                }
                // Anchored to the right edge, out of the way of the status
                // text and the progress bars that share this row. The
                // status line expires on a timer, so an update found by the
                // silent startup check would otherwise scroll past unread;
                // this link stays until the next check clears it.
                let update = available_update.cloned();
                // Which `.baboon` this workspace's changes belong to, and
                // where they are actually being kept. Autosave and Save write
                // different files, and a workspace that has never been saved
                // writes only the recovery file — none of which was visible
                // anywhere before.
                let project = cx
                    .model.current_source_is_campaign_project_capable(cx.model.active)
                    .then(|| cx.model.kits[cx.model.active].project.active.as_ref())
                    .flatten()
                    // A workspace with neither a project file nor a stash has
                    // nothing to say here, and saying it anyway on every
                    // Campaign Evolved kit would just be furniture.
                    .filter(|project| {
                        project.project_path.is_some() || !project.overlays.is_empty()
                    })
                    .map(|project| {
                        let mut hover = match project.project_path.as_deref() {
                            Some(path) => format!("Baboon project: {}", path.display()),
                            None => "This workspace has no saved Baboon project yet — use \
                                     File > Save Baboon Project"
                                .to_owned(),
                        };
                        hover.push_str(&format!(
                            "\nAutosaved to {}",
                            project.recovery_path.display()
                        ));
                        (format!("Project: {}", project.label()), hover)
                    });
                if update.is_some() || project.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(update) = update {
                            let label = format!("Update available: {}", update.short_name());
                            let hover = match update.channel {
                                UpdateChannel::Stable => "Open the release page on GitHub",
                                UpdateChannel::Development => {
                                    "Open the latest development build on GitHub"
                                }
                            };
                            // An explicit colour beats the app-wide
                            // `override_text_color`, which would otherwise
                            // flatten both this and the link colour to
                            // ordinary body text. `strong()` only brightens;
                            // the weight comes from the bold family, at the
                            // body size of the row it sits in.
                            ui.hyperlink_to(
                                RichText::new(label)
                                    .font(bold_font(12.0))
                                    .color(good_news()),
                                &update.release_url,
                            )
                            .on_hover_text(hover);
                        }
                        if let Some((label, hover)) = project {
                            ui.label(RichText::new(label).small().color(subtle_dark()))
                                .on_hover_text(hover);
                        }
                    });
                }
            });
        });
}

/// The "please wait" window shown while the active kit is still indexing.
/// "Please wait" while the active workspace indexes its tags or their
/// references. Opened when an index job starts and closed when it ends; between
/// jobs of its own it stays open and draws nothing.
pub(in crate::app) struct IndexingNotice;

impl Dialog for IndexingNotice {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        if cx.model.kits[cx.model.active].scanning_entries
            || cx.model.kits[cx.model.active]
                .index_jobs
                .references_for_entry_index
        {
            let mut open = true;
            let mut hide_notice = false;
            egui::Window::new("Indexing")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.set_min_width(360.0);
                    ui.label("Please wait until indexing is completed for best compatibility.");
                    ui.add_space(8.0);
                    if cx.model.kits[cx.model.active].scanning_entries {
                        let progress = cx.model.kits[cx.model.active]
                            .index_jobs
                            .entry_progress
                            .as_ref();
                        let label = progress
                            .map(|progress| progress.label.as_str())
                            .unwrap_or("Indexing tags...");
                        ui.label(RichText::new(label).strong());
                        if let Some(progress) = progress {
                            let fraction = if progress.total == 0 {
                                0.0
                            } else {
                                progress.processed as f32 / progress.total as f32
                            };
                            let text = if progress.total == 0 {
                                "Discovering files...".to_owned()
                            } else {
                                format!(
                                    "{} / {} files, {} tags",
                                    progress.processed, progress.total, progress.matched
                                )
                            };
                            draw_index_progress_bar(ui, 330.0, Some(fraction), &text);
                        }
                    } else if cx.model.kits[cx.model.active]
                        .index_jobs
                        .references_for_entry_index
                    {
                        ui.label(RichText::new("Building reference index...").strong());
                        if let Some(progress) = cx.model.kits[cx.model.active]
                            .index_jobs
                            .reference_progress
                            .as_ref()
                        {
                            let fraction = if progress.total == 0 {
                                0.0
                            } else {
                                progress.processed as f32 / progress.total as f32
                            };
                            let text = format!("{} / {} tags", progress.processed, progress.total);
                            draw_index_progress_bar(ui, 330.0, Some(fraction), &text);
                        } else {
                            draw_index_progress_bar(
                                ui,
                                330.0,
                                None,
                                "Scanning tag dependencies...",
                            );
                        }
                    }
                    ui.add_space(8.0);
                    if ui.button("Hide").clicked() {
                        hide_notice = true;
                    }
                });
            return open && !hide_notice;
        }
        true
    }
}

/// The terminal panel, when the active kit has it open.
pub(in crate::app) fn draw_terminal_panel(
    cx: &Ctx,
    ui: &mut egui::Ui,
    kit_tools: &mut KitsFeature,
    view: &mut KitView,
) {
    if view.terminal.open {
        let work_dir_label = view
            .terminal.work_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        egui::Panel::bottom("terminal")
            .resizable(true)
            .default_size(180.0)
            .size_range(90.0..=600.0)
            .frame(
                Frame::NONE
                    .fill(foundation_group_bg())
                    .inner_margin(egui::Margin {
                        left: 6,
                        right: 6,
                        top: 4,
                        bottom: 4,
                    }),
            )
            .show(ui, |ui| {
                // Header pinned to the top of the panel.
                egui::Panel::top("terminal_header")
                    .frame(Frame::NONE)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(RichText::new("Terminal").color(text_dark()));
                            ui.small(
                                RichText::new(&work_dir_label)
                                    .color(subtle_dark())
                                    .monospace(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button("×")
                                        .on_hover_text("Close terminal")
                                        .clicked()
                                    {
                                        cx.send(KitsCommand::CloseTerminal);
                                    }
                                    if icon_button(
                                        ui,
                                        ButtonIcon::Clear,
                                        "Clear terminal",
                                        true,
                                        text_dark(),
                                    )
                                    .clicked()
                                    {
                                        kit_tools.terminal.lines.clear();
                                    }
                                    let open_log_enabled =
                                        kit_tools.terminal.last_log_path.is_some();
                                    let mut open_log_button = ui.add_enabled(
                                        open_log_enabled,
                                        egui::Button::new(
                                            RichText::new("Open full log").small(),
                                        ),
                                    );
                                    if let Some(path) = kit_tools.terminal.last_log_path.as_ref() {
                                        open_log_button = open_log_button
                                            .on_hover_text(path.display().to_string());
                                    }
                                    if open_log_button.clicked()
                                        && let Some(path) = kit_tools.terminal.last_log_path.clone()
                                        && let Err(error) = open_terminal_log(&path)
                                    {
                                        cx.set_status(error);
                                    }
                                    if kit_tools.terminal.running {
                                        if kit_tools.terminal.process.is_some()
                                            && ui.small_button("Stop").clicked()
                                        {
                                            cx.send(KitsCommand::StopTerminal);
                                        }
                                        let running_label = kit_tools
                                            .terminal
                                            .running_command
                                            .as_deref()
                                            .unwrap_or("running...");
                                        ui.small(
                                            RichText::new(running_label)
                                                .color(subtle_dark())
                                                .monospace(),
                                        );
                                    }
                                },
                            );
                        });
                        ui.add_space(2.0);
                    });

                // Input row pinned to the bottom of the panel.
                egui::Panel::bottom("terminal_input")
                    .frame(Frame::NONE)
                    .show(ui, |ui| {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(">").monospace().color(subtle_dark()));
                            // Reserve a fixed width for the Run button on
                            // the right; the TextEdit fills the rest. (Do
                            // NOT wrap the button in a right_to_left layout
                            // — that consumes all remaining width and leaves
                            // nothing for the input field.)
                            let button_w = 52.0;
                            let text_w = (ui.available_width() - button_w - 8.0).max(40.0);
                            let resp = ui.add_enabled(
                                !kit_tools.terminal.running,
                                egui::TextEdit::singleline(&mut kit_tools.terminal.input)
                                    .desired_width(text_w)
                                    .font(egui::TextStyle::Monospace)
                                    .hint_text(placeholder_text("tool <command> …")),
                            );
                            if kit_tools.terminal.refocus_input && !kit_tools.terminal.running {
                                resp.request_focus();
                                kit_tools.terminal.refocus_input = false;
                            }
                            let run_clicked = ui
                                .add_enabled(!kit_tools.terminal.running, egui::Button::new("Run"))
                                .clicked();
                            let enter = lost_focus_once(&resp)
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if resp.has_focus() && !kit_tools.terminal.running {
                                let recall = ui.input(|i| {
                                    if i.key_pressed(egui::Key::ArrowUp) {
                                        -1
                                    } else if i.key_pressed(egui::Key::ArrowDown) {
                                        1
                                    } else {
                                        0
                                    }
                                });
                                if recall != 0 {
                                    kit_tools.terminal.recall_history(recall);
                                    resp.request_focus();
                                }
                            }
                            if run_clicked || enter {
                                cx.send(KitsCommand::RunTerminalInput);
                                // Refocus the input so the user can keep typing.
                                resp.request_focus();
                            }
                        });
                    });

                // Output fills the remaining center space. The CentralPanel
                // bounds the scroll area exactly, so there's no available_height
                // feedback to fight the resize handle.
                egui::CentralPanel::default()
                    .frame(
                        Frame::NONE
                            .fill(Color32::from_rgb(24, 24, 23))
                            .inner_margin(egui::Margin {
                                left: 6,
                                right: 6,
                                top: 4,
                                bottom: 4,
                            }),
                    )
                    .show(ui, |ui| {
                        let want_scroll_bottom = kit_tools.terminal.scroll_to_bottom;
                        kit_tools.terminal.scroll_to_bottom = false;
                        draw_terminal_output(ui, &kit_tools.terminal.lines, want_scroll_bottom);
                    });
            });
    }
}

/// While a folder move or rename runs, cover the whole window with a layer
/// that takes every click, drag and scroll, and show its progress on it.
///
/// The job rewrites tags on disk from a snapshot taken when it started; an
/// edit, save or second refactor in the meantime would be overwritten or
/// would race it. Drawn last and in the foreground so no window or panel
/// sits above it.
pub(in crate::app) fn draw_folder_refactor_lock(ctx: &egui::Context, folder_refactor: Option<&FolderRefactorUiState>) {
    let Some(progress) = folder_refactor else {
        return;
    };
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("folder_refactor_lock"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, _) =
                ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_black_alpha(140));
            let panel = egui::Rect::from_center_size(rect.center(), egui::vec2(360.0, 96.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(panel.width());
                    ui.label(RichText::new(&progress.label).strong().color(text_dark()));
                    ui.add_space(4.0);
                    let bar = match progress.progress {
                        Some(value) => egui::ProgressBar::new(value.clamp(0.0, 1.0)),
                        None => egui::ProgressBar::new(0.0).animate(true),
                    };
                    ui.add(bar.text(RichText::new(&progress.phase).color(text_dark())));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Baboon is locked until references are updated.")
                            .color(subtle_dark())
                            .small(),
                    );
                });
            });
        });
    ctx.request_repaint_after(PROGRESS_REPAINT);
}

#[cfg(test)]
pub(in crate::app) mod tests {
    use super::*;
    use crate::app::editor::{ColorPopupWindow, MaterialColorPopup};

    thread_local! {
        /// Output lines laid out. egui skips painting offscreen labels by
        /// itself, so the painted text alone cannot show that the pane lays
        /// out only what is in view.
        pub(in crate::app) static LINES_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    fn lines(count: usize) -> Vec<TerminalLineEntry> {
        (0..count)
            .map(|index| {
                TerminalLineEntry::new(format!(
                    "{index}: tool.exe: importing C:\\Halo\\tags\\objects\\weapons\\rifle_{index}\\\
                     render\\rifle_{index}.render_model from data\\objects\\weapons ... done"
                ))
            })
            .collect()
    }

    fn frame(
        ctx: &egui::Context,
        lines: &[TerminalLineEntry],
        bottom: bool,
    ) -> std::time::Duration {
        let started = std::time::Instant::now();
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    draw_terminal_output(ui, lines, bottom);
                });
            },
        );
        started.elapsed()
    }

    /// The text of every line painted in a frame.
    fn painted(ctx: &egui::Context, lines: &[TerminalLineEntry], bottom: bool) -> Vec<String> {
        LINES_BUILT.with(|built| built.set(0));
        let output = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    draw_terminal_output(ui, lines, bottom);
                });
            },
        );
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    /// Only the lines in view are drawn, and they are the right ones: the
    /// top of the output when opened, the end of it after scrolling there.
    #[test]
    fn the_terminal_draws_the_lines_in_view() {
        let lines = lines(20_000);
        let starts =
            |painted: &[String], prefix: &str| painted.iter().any(|text| text.starts_with(prefix));

        let top = painted(&egui::Context::default(), &lines, false);
        assert!(starts(&top, "0: ") && !starts(&top, "19999: "));
        let built = LINES_BUILT.with(std::cell::Cell::get);
        assert!(built < 100, "laid out {built} of 20,000 lines");

        let ctx = egui::Context::default();
        // Scrolling animates over frames; land in one.
        ctx.global_style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
        painted(&ctx, &lines, true);
        let bottom = (0..3).map(|_| painted(&ctx, &lines, false)).last().unwrap();
        assert!(starts(&bottom, "19999: "), "the last line is in view");
        assert!(!starts(&bottom, "0: "));
        let built = LINES_BUILT.with(std::cell::Cell::get);
        assert!(built < 100, "laid out {built} of 20,000 lines");
    }

    /// Frame time with a full terminal. Run with `--release --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn bench_terminal_frame() {
        let lines = lines(20_000);
        let ctx = egui::Context::default();
        for index in 0..6 {
            eprintln!("frame {index}: {:?}", frame(&ctx, &lines, index == 0));
        }
    }

    // While a folder move or rename rewrites tags on disk, nothing under the lock
    // takes a click and no shortcut runs. Each check has an unlocked control that
    // must disagree, so a lock that blocked nothing would fail here.

    fn app(locked: bool) -> Baboon {
        let mut app = Baboon::assemble(
            &egui::Context::default(),
            crate::app::shell::window_state::WindowStateTracker::for_test(),
            GuiPrefs::default(),
            HashSet::new(),
            None,
            TagNameIndex::default(),
            None,
        );
        if locked {
            app.tag_ops.folder_refactor = Some(FolderRefactorUiState {
                label: "Renaming creep to shadow".to_owned(),
                phase: "Moving files".to_owned(),
                progress: Some(0.5),
            });
        }
        app
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        }
    }

    /// Click a button drawn beneath the lock layer; whether it saw the click.
    fn button_sees_click(locked: bool) -> bool {
        let mut app = app(locked);
        let ctx = egui::Context::default();
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let clicked = std::cell::Cell::new(false);
        let frame = |events: Vec<egui::Event>, app: &mut Baboon| {
            let _ = crate::app::run_ui_test(&ctx, input(events), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let response = ui.button("Save");
                    rect.set(response.rect);
                    clicked.set(clicked.get() | response.clicked());
                });
                draw_folder_refactor_lock(&ctx, app.tag_ops.folder_refactor.as_ref());
            });
        };
        frame(Vec::new(), &mut app);
        let pos = rect.get().center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pos)], &mut app);
        frame(vec![press(true)], &mut app);
        frame(vec![press(false)], &mut app);
        frame(Vec::new(), &mut app);
        clicked.get()
    }

    #[test]
    fn the_lock_swallows_clicks_meant_for_the_app_beneath() {
        assert!(
            button_sees_click(false),
            "control: the click lands unlocked"
        );
        assert!(!button_sees_click(true), "the lock must take the click");
    }

    /// Press Ctrl+S; whether a save was queued.
    fn ctrl_s_queues_save(locked: bool) -> bool {
        let mut app = app(locked);
        let ctx = egui::Context::default();
        let _ = crate::app::run_ui_test(
            &ctx,
            input(vec![egui::Event::Key {
                key: egui::Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                // As Windows reports Ctrl: `command` set with it.
                modifiers: egui::Modifiers::CTRL.plus(egui::Modifiers::COMMAND),
            }]),
            |_| {
                app.prepare_root_frame(&ctx);
                // A shortcut's action is applied with the frame's commands.
                app.apply_commands(&ctx);
            },
        );
        app.editor.deferred_file_action.is_some()
    }

    #[test]
    fn shortcuts_do_not_run_while_locked() {
        assert!(
            ctrl_s_queues_save(false),
            "control: Ctrl+S queues a save unlocked"
        );
        assert!(
            !ctrl_s_queues_save(true),
            "Ctrl+S must not run under the lock"
        );
    }

    // A confirmed colour or function popup edits the kit it was opened from.
    //
    // The shader and material grids used to write the shared popup directly,
    // with no record of their kit, so the stamp left by whichever popup was
    // opened before decided where the edit went: dropped, or applied to another
    // kit's tag with the same key.

    fn two_kits() -> (Baboon, KitId, KitId) {
        let mut app = Baboon::for_test();
        let a = app.model.kits[0].id;
        let b = KitId(a.0 + 1);
        app.push_kit(Kit::empty(b, TagNameIndex::default()));
        (app, a, b)
    }

    /// A colour popup opened from `kit`, as a tag pane opens one.
    fn open_popup(app: &mut Baboon, kit: KitId) {
        app.dialogs.open(ColorPopupWindow {
            popup: Some(MaterialColorPopup::new("color", 1.0, 0.5, 0.25, 1.0)),
            kit,
            opened_at: None,
        });
    }

    /// The kit the open colour popup applies to.
    fn popup_kit(app: &mut Baboon) -> Option<usize> {
        let opened_from = app
            .dialogs
            .get::<ColorPopupWindow>()
            .map(|window| window.kit);
        app.popup_target_kit(opened_from)
    }

    #[test]
    fn a_grid_popup_opened_after_another_kits_popup_edits_its_own_kit() {
        let (mut app, a, b) = two_kits();
        // A normal swatch in B opens the picker; it is stamped with B.
        open_popup(&mut app, b);
        assert_eq!(popup_kit(&mut app), Some(1));

        // Then the shader grid in A opens one, B still active.
        app.model.active = 1;
        open_popup(&mut app, a);
        assert_eq!(
            popup_kit(&mut app),
            Some(0),
            "the edit lands in A, where the popup was opened"
        );
    }

    #[test]
    fn a_popup_from_a_closed_kit_is_dropped_not_redirected() {
        let (mut app, _, b) = two_kits();
        open_popup(&mut app, b);
        app.model.kits.pop();
        assert_eq!(popup_kit(&mut app), None);
        // A popup with no recorded kit still applies to the active one.
        assert_eq!(app.popup_target_kit(None), Some(app.model.active));
    }

    /// What the tag pane asks of the sound player during a frame is taken up
    /// in that same frame. The pane sends it as a command, and when commands
    /// were applied after the audio queue had been processed, each draw saw
    /// the player as it was before its own last requests: it asked again for
    /// a preview, which, processed after Play, replaced it. Nothing played.
    #[test]
    fn a_sound_request_sent_during_a_frame_is_taken_up_that_frame() {
        use crate::app::audio::{AudioCommand, SoundAction, SoundRequest};
        let mut app = Baboon::for_test();
        let ctx = egui::Context::default();
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| app.draw_root_ui(ui));

        app.commands.send(AudioCommand::Queue(std::collections::VecDeque::from([
            SoundRequest::from(SoundAction::Stop),
        ])));
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| app.draw_root_ui(ui));

        assert!(app.audio.pending.is_empty(), "the request is still waiting for another frame");
    }
}
