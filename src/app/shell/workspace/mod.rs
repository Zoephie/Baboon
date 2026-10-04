//! Main application shell: menus, toolbar, sidebar, tabs, terminal, and status areas.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;
use crate::app::shell::frame::terminal_line_is_strong;
use crate::app::shell::frame::terminal_line_color;
use crate::app::kits::terminal::open_terminal_log;
use crate::app::shell::frame::draw_index_progress_bar;

/// How often a progress bar is redrawn while its job runs.
const PROGRESS_REPAINT: std::time::Duration = std::time::Duration::from_millis(200);

impl Baboon {
    pub(in crate::app) fn draw_root_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        if self.shell.first_run_wizard.is_some() {
            ctx.set_zoom_factor(self.model.prefs.ui_scale);
            set_dark_mode(self.model.prefs.dark_mode);
            ctx.set_visuals(foundation_visuals());
            egui::CentralPanel::default().show(ui, |_ui| {});
            draw_first_run_wizard(&cx!(self, ctx), &mut self.shell, &mut self.kit_tools);
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
        draw_entry_index_wait_notice(&cx!(self, ctx), &mut self.kit_tools);
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
        draw_color_popup_window(&cx!(self, ctx), &mut self.editor);
        draw_function_popup_window(&cx!(self, ctx), &mut self.editor);
        self.process_frame_requests(ctx);
        self.apply_commands(ctx);
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
        draw_block_confirm(&cx!(self, ctx), &mut self.editor);
        draw_save_changes_prompt(&cx!(self, ctx), &mut self.documents);
        draw_last_opened_windows_prompt(&cx!(self, ctx), &mut self.shell);
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
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            self.search.find.open = true;
            self.search.find.focus_query = true;
        }
        self.refresh_find(ctx);
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.defer_file_action(DeferredFileAction::SaveCurrentTag, ctx);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::P)) {
            self.defer_file_action(DeferredFileAction::PokeCurrentTag, ctx);
        }
        // Deferred like the File menu's Close Current Tag: the close runs after
        // the editor renders, so an edit still focused in a field is committed
        // before the dirty check decides whether to prompt.
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::W)) {
            self.defer_file_action(DeferredFileAction::CloseCurrentTab, ctx);
        }
        // Undo: Ctrl+Z. Redo: Ctrl+Shift+Z or Ctrl+Y.
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Z)) {
            self.undo_current_tag();
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
        }) || ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Y))
        {
            self.redo_current_tag();
        }
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
        draw_tag_reference_picker_window(&cx!(self, ctx), &mut self.editor);
        draw_settings_window(
            &cx!(self, ctx),
            &mut self.shell,
            &mut self.kit_tools,
            &mut self.chimp.chimp_usmap_path_input,
        );
        draw_tool_commands_window(&cx!(self, ctx), &mut self.kit_tools);
        draw_operation_notice_window(&cx!(self, ctx), &mut self.shell);
        self.diff_expanded_mod_export_rows();
        draw_poke_window(&cx!(self, ctx), &mut self.poke);
        // Walk any expanded rows whose fields are not known yet before the
        // window reads them.
        self.refresh_ref_jump_occurrences(ctx);
        draw_query_results_window(&cx!(self, ctx), &mut self.search, &mut self.references);
        draw_tag_diff_window(
            &cx!(self, ctx),
            &mut self.compare,
            &self.kit_tools.editing_kit_validation,
        );
        draw_content_explorer_window(&cx!(self, ctx), &mut self.references);
        draw_keyword_chooser_window(&cx!(self, ctx), &mut self.browser);
        draw_field_value_search_window(&cx!(self, ctx), &mut self.search);
        draw_find_window(&cx!(self, ctx), &mut self.search);
        draw_tsv_paste_window(&cx!(self, ctx), &mut self.editor);
        self.dialogs.draw(&cx!(self, ctx));
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
                        terminal_output_tests::LINES_BUILT.with(|built| built.set(built.get() + 1));
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

#[cfg(test)]
pub(in crate::app) mod terminal_output_tests;

#[cfg(test)]
mod folder_refactor_lock_tests;

#[cfg(test)]
mod popup_kit_stamp_tests;

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
pub(in crate::app) fn draw_entry_index_wait_notice(cx: &Ctx, kit_tools: &mut KitsFeature) {
    let ctx = cx.egui;
    if kit_tools.show_entry_index_wait_notice
        && (cx.model.kits[cx.model.active].scanning_entries
            || cx.model.kits[cx.model.active].index_jobs.references_for_entry_index)
    {
        let mut open = kit_tools.show_entry_index_wait_notice;
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
                        draw_index_progress_bar(ui, 330.0, Some(fraction), &text);
                    }
                } else if cx.model.kits[cx.model.active].index_jobs.references_for_entry_index {
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
        kit_tools.show_entry_index_wait_notice = open && !hide_notice;
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
