//! The Rename Tag window.
//! It owns presentation and the path entered; the rename is
//! [`TagOpsCommand::Rename`].

use super::*;
use crate::app::editor::{TagPathCompletion, add_completing};
use crate::core::tag_path_completion::Wanted;

impl Dialog for RenameTagState {
    fn show(&mut self, cx: &Ctx, reads: &AppReads) -> bool {
        // Rename keeps its folder: the shared Rename layout. Move, Duplicate
        // and Save As choose destinations and keep their own.
        if self.operation == TagNameOperation::Rename && !self.whole_path_editable {
            return show_rename(self, cx);
        }
        show_name_operation(self, cx, reads)
    }
}

/// Rename a tag within its folder, styled like Bulk Rename.
fn show_rename(state: &mut RenameTagState, cx: &Ctx) -> bool {
    let ctx = cx.egui;
    let mut open = true;
    let mut response = Default::default();
    let source = cx.model.kit_index(state.kit).and_then(|index| cx.model.kits[index].source.as_ref());
    let game = source.and_then(|source| source.source.game());
    let group = source.and_then(|source| source.names.group_tag_for(&state.extension));
    let current = tag_path::name(state.old_display.rsplit_once('.').map_or(state.old_display.as_str(), |(stem, _)| stem)).to_owned();
    // A Campaign Evolved container is copied or rewritten rather than having
    // its referrers fixed: say what happens instead of listing references.
    let notes = if state.is_new_container {
        Some(("This tag has not been saved yet, so it simply moves to the new name; nothing is written.".to_owned(),
            Some("It is written when you Save it or Export Mod.".to_owned())))
    } else if let Some(pak) = &state.in_place_pak {
        Some((format!("Moves this tag inside {pak}, the pack that already holds it. No new container is written."),
            Some("That pack is backed up first. Only tags Baboon created can be moved this way; the game's own are copied into an overlay instead, because moving one would break every reference to it.".to_owned())))
    } else if state.is_container {
        Some(("This is one of the game's own tags, so it is copied to the new name in an overlay container rather than moved. Moving it would break every reference to it, and a pak cannot forward them.".to_owned(),
            Some("A higher-priority overlay container is written; base game files are never modified.".to_owned())))
    } else {
        None
    };
    let references = (!state.is_container).then(|| {
        let rows = source.map(|source| crate::app::browser::bulk_rename::referrer_rows(&state.referrers, &source.names)).unwrap_or_default();
        let notes = state.referrers_unavailable.then(|| "Reference index unavailable. References are still found and fixed when renaming.".to_owned());
        (rows, notes.into_iter().collect())
    });
    // Scripts name the tag by path: listed with the references.
    let scripts = if state.is_container {
        Default::default()
    } else {
        let old = state.old_display.clone();
        super::script_fixes::ScriptCheck::rows_for(&mut state.scripts, cx, state.kit, |_| {
            crate::core::script_paths::ScriptChanges::renaming([(old.as_str(), None)])
        })
    };
    egui::Window::new("Rename Tag")
        .constrain_to(window_work_area(ctx))
        .id(egui::Id::unique("rename_tag"))
        .title_bar(false)
        .collapsible(false)
        .frame(egui::Frame::window(&ctx.global_style()))
        .default_width(window_width(ctx, 480.0))
        .resizable(false)
        .show(ctx, |ui| {
            response = super::single_rename::draw_single_rename(cx, ui, &mut open, super::single_rename::SingleRename {
                input: &mut state.new_path_input,
                focus: &mut state.focus_input,
                field_id: egui::Id::unique("rename_tag_name"),
                current: &current,
                location: state.old_display.clone(),
                extension: Some(&state.extension),
                icon: crate::app::browser::bulk_rename::RenameIcon::Tag(group),
                game,
                noun: "Tag names",
                error: None,
                notes,
                references,
                scripts,
                apply_hover: if state.is_new_container {
                    "Rename this unsaved tag (nothing is written yet)"
                } else {
                    "Rename the file and fix every reference"
                },
            });
        });
    // Left open for the apply, which closes it once it succeeds.
    if response.apply {
        cx.send(TagOpsCommand::Rename);
    }
    open && !response.cancel
}

/// Move, Duplicate and Save As: choosing a destination.
fn show_name_operation(this: &mut RenameTagState, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut do_apply = false;
        let mut cancel = false;
        {
            let state = &mut *this;
            let title = match state.operation {
                TagNameOperation::Duplicate => "Duplicate Tag",
                TagNameOperation::Rename if state.whole_path_editable && state.is_new_container => {
                    "Move New Tag"
                }
                TagNameOperation::Rename if state.whole_path_editable => "Move Tag",
                TagNameOperation::Rename if state.is_new_container => "Rename New Tag",
                TagNameOperation::SaveAsOverlay if state.is_new_container => "Copy New Tag",
                TagNameOperation::SaveAsOverlay if state.is_container => "Save Tag As (New Copy)",
                _ => "Rename Tag",
            };
            let icon = match state.operation {
                TagNameOperation::Duplicate => ButtonIcon::Duplicate,
                TagNameOperation::SaveAsOverlay => ButtonIcon::Copy,
                TagNameOperation::Rename if state.whole_path_editable => ButtonIcon::Move,
                TagNameOperation::Rename => ButtonIcon::Rename,
            };
            egui::Window::new(title)
                .constrain_to(window_work_area(ctx))
                .id(egui::Id::unique("rename_tag"))
                .title_bar(false)
                .collapsible(false)
                .frame(egui::Frame::window(&ctx.global_style()))
                .default_width(window_width(ctx, 560.0))
                .show(ctx, |ui| {
                    crate::app::search::draw_icon_window_header(ui, title, icon, &mut open);
                    ui.add_space(12.0);
                    ui.label(RichText::new("Current path").color(subtle_dark()).small());
                    ui.label(
                        RichText::new(&state.old_display)
                            .color(text_dark())
                            .monospace(),
                    );
                    ui.add_space(6.0);
                    if state.operation == TagNameOperation::Duplicate {
                        ui.label(
                            RichText::new("Destination leaf (parent and extension are fixed)")
                                .color(subtle_dark())
                                .small(),
                        );
                        ui.horizontal(|ui| {
                            let parent = if state.fixed_parent.is_empty() {
                                "(root)/".to_owned()
                            } else {
                                format!("{}/", state.fixed_parent)
                            };
                            ui.label(RichText::new(parent).color(subtle_dark()).monospace());
                            let response = ui.add(
                                crate::app::ui_kit::singleline_text_edit(&mut state.new_path_input)
                                    .id(egui::Id::unique("duplicate_tag_name"))
                                    .desired_width(330.0)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if state.focus_input {
                                response.request_focus();
                                if let Some(mut text_state) =
                                    egui::TextEdit::load_state(ctx, response.id)
                                {
                                    text_state.cursor.set_char_range(Some(
                                        egui::text::CCursorRange::two(
                                            egui::text::CCursor::new(0),
                                            egui::text::CCursor::new(
                                                state.new_path_input.chars().count(),
                                            ),
                                        ),
                                    ));
                                    text_state.store(ctx, response.id);
                                }
                                state.focus_input = false;
                            }
                            ui.label(
                                RichText::new(format!(".{}", state.extension)).color(subtle_dark()),
                            );
                        });
                    } else {
                        ui.label(
                            RichText::new(if state.whole_path_editable {
                                "New path (folders allowed; extension is fixed)"
                            } else {
                                "New name (extension is fixed)"
                            })
                            .color(subtle_dark())
                            .small(),
                        );
                        ui.horizontal(|ui| {
                            // A whole path lists the kit's folders as it is
                            // typed; a name alone has none to list.
                            let completion = state
                                .whole_path_editable
                                .then(|| {
                                    TagPathCompletion::for_model_kit(
                                        cx.model,
                                        cx.model.active,
                                        Wanted::Folders,
                                    )
                                })
                                .flatten();
                            let before = state.new_path_input.clone();
                            add_completing(
                                ui,
                                crate::app::ui_kit::singleline_text_edit(&mut state.new_path_input)
                                    .desired_width(430.0)
                                    .font(egui::TextStyle::Monospace),
                                egui::Id::unique("rename_tag_path"),
                                completion.as_ref(),
                                &before,
                            );
                            ui.label(
                                RichText::new(format!(".{}", state.extension)).color(subtle_dark()),
                            );
                        });
                    }
                    let name_error = if state.operation == TagNameOperation::Rename
                        && !state.whole_path_editable
                        && !state.new_path_input.trim().is_empty()
                    {
                        duplicate::validate_leaf_characters(
                            &state.new_path_input,
                            "Tag names",
                            "Enter a name",
                        )
                        .err()
                    } else {
                        None
                    };
                    if let Some(error) = &name_error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    if state.operation != TagNameOperation::Rename || state.whole_path_editable {
                        // When the whole path is editable it keeps no parent from
                        // the old one — what is typed IS the destination.
                        let preview_parent = if state.operation == TagNameOperation::Duplicate {
                            state.fixed_parent.as_str()
                        } else if state.whole_path_editable {
                            ""
                        } else {
                            tag_path::parent(&state.old_display).unwrap_or("")
                        };
                        let preview_name = state.new_path_input.trim();
                        let preview = if preview_name.is_empty() {
                            "(enter a new name)".to_owned()
                        } else {
                            tag_path::join(
                                preview_parent,
                                &format!("{preview_name}.{}", state.extension),
                            )
                        };
                        ui.add_space(3.0);
                        ui.label(RichText::new("Preview").color(subtle_dark()).small());
                        ui.label(
                            RichText::new(preview)
                                .color(text_dark())
                                .monospace()
                                .small(),
                        );
                        ui.add_space(8.0);
                    }
                    if state.operation == TagNameOperation::Duplicate {
                        ui.label(
                            RichText::new(
                                "Creates an independent clean duplicate; existing references are \
                                 unchanged.",
                            )
                            .color(text_dark()),
                        );
                        ui.label(
                            RichText::new(if state.is_container {
                                "The exact current Campaign Evolved container will be backed up \
                                 and updated in place after a separate confirmation."
                            } else {
                                "Current unsaved edits are copied when the source is dirty; clean \
                                 sources are copied byte-for-byte."
                            })
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if state.is_new_container {
                        ui.label(
                            RichText::new(if state.operation == TagNameOperation::Rename {
                                "This tag has not been saved yet, so it simply moves to the new \
                                 path — nothing is written."
                            } else {
                                "Creates a second unsaved tag with a copy of this one's contents."
                            })
                            .color(text_dark()),
                        );
                        ui.label(
                            RichText::new(if state.operation == TagNameOperation::Rename {
                                "It is written when you Save it or Export Mod."
                            } else {
                                "Both tags are written only when you Save them or Export Mod."
                            })
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if let Some(pak) = state.in_place_pak.clone() {
                        ui.label(
                            RichText::new(format!(
                                "Moves this tag inside {pak}, the pack that already holds it. No \
                                 new container is written."
                            ))
                            .color(text_dark()),
                        );
                        ui.label(
                            RichText::new(
                                "That pack is backed up first. Only tags Baboon created can be \
                                 moved this way — the game's own are copied into an overlay \
                                 instead, because moving one would break every reference to it.",
                            )
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if state.is_container {
                        if state.operation == TagNameOperation::Rename {
                            ui.label(
                                RichText::new(
                                    "This is one of the game's own tags, so it is copied to the \
                                     new path in an overlay container rather than moved. Moving it \
                                     would break every reference to it, and a pak cannot forward \
                                     them.",
                                )
                                .color(text_dark()),
                            );
                        } else {
                            ui.label(
                                RichText::new(
                                    "Creates an unsaved copy at the new path and switches to it; \
                                     existing references are unchanged.",
                                )
                                .color(text_dark()),
                            );
                        }
                        ui.label(
                            RichText::new(if state.operation == TagNameOperation::Rename {
                                "A higher-priority overlay container is written; base game files \
                                 are never modified."
                            } else {
                                "It is written when you Save it or Export Mod, keeping this tag's \
                                 Unreal bindings; base game files are never modified."
                            })
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if state.referrers_unavailable {
                        ui.label(
                            RichText::new(
                                "Reference index unavailable — references are still rewritten on \
                                 apply, but can't be previewed here.",
                            )
                            .color(subtle_dark()),
                        );
                    } else if state.referrers.is_empty() {
                        ui.label(
                            RichText::new("No other tags reference this tag.").color(subtle_dark()),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "{} referring tag(s) will be updated:",
                                state.referrers.len()
                            ))
                            .color(text_dark()),
                        );
                        crate::app::ScrollArea::vertical()
                            .id_salt("rename_referrers")
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for referrer in &state.referrers {
                                    ui.label(RichText::new(referrer).color(subtle_dark()).small());
                                }
                            });
                    }
                    ui.add_space(12.0);
                    popup_footer(ui, |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked() {
                                cancel = true;
                            }
                            if icon_text_button(
                                ui,
                                icon,
                                match state.operation {
                                    TagNameOperation::Duplicate => "Duplicate",
                                    TagNameOperation::SaveAsOverlay => "Save As",
                                    TagNameOperation::Rename if state.whole_path_editable => "Move",
                                    TagNameOperation::Rename => "Rename",
                                },
                                !state.new_path_input.trim().is_empty() && name_error.is_none(),
                            )
                            .on_hover_text(match state.operation {
                                TagNameOperation::Duplicate => {
                                    "Create an independent duplicate without rewriting references"
                                }
                                TagNameOperation::Rename if state.is_new_container && state.whole_path_editable => {
                                    "Move this unsaved tag to the new path (nothing is written yet)"
                                }
                                TagNameOperation::Rename if state.is_new_container => {
                                    "Rename this unsaved tag (nothing is written yet)"
                                }
                                TagNameOperation::SaveAsOverlay if state.is_new_container => {
                                    "Copy this unsaved tag to the new path (nothing is written yet)"
                                }
                                TagNameOperation::SaveAsOverlay if state.is_container => {
                                    "Copy this tag to the new path and edit the copy (nothing is \
                                         written yet)"
                                }
                                TagNameOperation::Rename if state.whole_path_editable => {
                                    "Move this tag inside the pak that already holds it"
                                }
                                _ => "Rename the file and rewrite all references",
                            })
                            .clicked()
                            {
                                do_apply = true;
                            }
                        });
                    });
                });
        }
        // Left open for the apply, which closes it once it succeeds.
        if do_apply {
            cx.send(TagOpsCommand::Rename);
        }
        open && !cancel
}
