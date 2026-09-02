//! Ending a browser drag on Sapien's or Guerilla's window: the gate on what may go, the feedback while hovering, and the hand-over itself.
//! It owns application actions and workflow coordination; the Win32 delivery belongs to `kit_tool_drop` and the palette table to `scenario_palettes`.

use super::*;

/// egui temp-data key under which a frame leaves the cursor it wants shown
/// while a drag hovers a kit tool. Read by the end-of-pass hook `Baboon::new`
/// installs, which runs after egui's own drag-and-drop hook has forced the
/// grabbing hand, so it has the last word on the cursor.
pub(in crate::app) const KIT_TOOL_DROP_CURSOR: &str = "kit_tool_drop_cursor";

/// What a drop on a kit tool does, once every objection is out of the way.
struct KitToolDropPlan {
    file: PathBuf,
    /// The scenario palette the tag lands in, when Baboon can tell: only for
    /// Sapien, and only when the tag comes from a kit whose game is known.
    palette: Option<String>,
}

impl Baboon {
    /// Once per frame: follow a browser drag that has left Baboon's window,
    /// and when it ends over a kit tool's window, hand that tool the tag file
    /// the way Explorer would. Sapien then adds the tag to its scenario's
    /// matching palette; Guerilla opens it.
    pub(in crate::app) fn track_kit_tool_drop(&mut self, ctx: &egui::Context) {
        let payload = egui::DragAndDrop::payload::<DraggedTagRef>(ctx);
        // The tool under the cursor is only worth asking Windows about while
        // something is being dragged. egui keeps the payload through the frame
        // the pointer is released in, which is the frame the drop happens.
        let target = payload
            .as_ref()
            .and_then(|_| kit_tool_under_cursor(&mut self.kit_tool_drag.executables));
        let cursor_id = egui::Id::new(KIT_TOOL_DROP_CURSOR);
        let (Some(payload), Some(target)) = (payload, target) else {
            ctx.data_mut(|data| data.remove_temp::<egui::CursorIcon>(cursor_id));
            if !egui::DragAndDrop::has_any_payload(ctx) {
                self.kit_tool_drag.executables.clear();
            }
            self.end_kit_tool_hover(ctx);
            return;
        };
        let plan = self.plan_kit_tool_drop(&target, &payload);
        // egui-winit drops a button event that arrives while it thinks the
        // pointer has left the window, so the button's own state is asked as
        // well: a drag over another program's window with no button held is
        // a release by any name.
        let released = ctx.input(|input| input.pointer.any_released()) || mouse_buttons_are_up();
        if released {
            // From here the drop is this window's, not that of whatever
            // reference cell may sit under the same spot in Baboon's window.
            egui::DragAndDrop::clear_payload(ctx);
            ctx.data_mut(|data| data.remove_temp::<egui::CursorIcon>(cursor_id));
            self.end_kit_tool_hover(ctx);
            self.status = match plan {
                Ok(plan) => self.drop_tag_on_kit_tool(&target, &plan),
                Err(objection) => objection,
            };
            return;
        }

        let cursor = match &plan {
            Ok(_) => egui::CursorIcon::Copy,
            Err(_) => egui::CursorIcon::NotAllowed,
        };
        ctx.data_mut(|data| data.insert_temp(cursor_id, cursor));
        let message = match &plan {
            Ok(plan) => hover_message(target.tool, plan),
            Err(objection) => objection.clone(),
        };
        if self.kit_tool_drag.hover.is_none() {
            // The first frame over a tool: keep the status the drag interrupted
            // so it can come back if the drag leaves without dropping.
            let interrupted = std::mem::take(&mut self.status);
            self.kit_tool_drag.saved_status = Some((interrupted, self.status_changed_at));
        }
        self.kit_tool_drag.hover = Some(target);
        self.status = message;
    }

    /// Undo the hover feedback: the drag left the tool's window, ended
    /// somewhere else, or is about to drop. The interrupted status comes back
    /// only if it would still be showing had nothing interrupted it.
    fn end_kit_tool_hover(&mut self, ctx: &egui::Context) {
        let left_a_tool = self.kit_tool_drag.hover.take().is_some();
        if let Some((saved, shown_at)) = self.kit_tool_drag.saved_status.take()
            && left_a_tool
            && ctx.input(|input| input.time) - shown_at < STATUS_LINGER_SECS
        {
            self.status = saved;
        }
    }

    /// Every reason the drop cannot or should not happen, or the plan for it.
    ///
    /// The objections are the ones Sapien would otherwise answer with a modal
    /// error or silence: a tag with no file, a window that takes no drops, a
    /// tag from another kit, and a group the scenario has no palette for.
    /// Guerilla opens anything, so only the first three apply to it.
    fn plan_kit_tool_drop(
        &mut self,
        target: &KitToolDropTarget,
        payload: &DraggedTagRef,
    ) -> Result<KitToolDropPlan, String> {
        let tool = target.tool.label();
        let Some(file) = payload.file_path.clone() else {
            return Err(format!(
                "Only a tag on disk can be dropped into {tool}; this one lives in a cache or container"
            ));
        };
        let leaf = file_leaf(&file);
        if !target.accepts_files {
            // Sapien's Hierarchy, Properties and Output windows are separate
            // top-level windows that take no drops; the main window does.
            // A tool with no such window anywhere is another matter.
            return Err(if target.tool_accepts_files {
                format!(
                    "This {tool} window does not take dropped files; drop {leaf} on {tool}'s main window instead"
                )
            } else {
                match target.tool {
                    KitTool::Sapien => format!(
                        "This Sapien does not take dropped files (Halo CE's and Halo 2's do not); add {leaf} to the palette from Edit Types in Sapien instead"
                    ),
                    KitTool::Guerilla => format!(
                        "This Guerilla does not take dropped files; open {leaf} from Guerilla's File menu instead"
                    ),
                }
            });
        }
        if !tag_within_kit(&file, &target.kit_root) {
            return Err(format!(
                "{leaf} is not inside {tool}'s editing kit ({}), so {tool} could not use it; open the kit in Baboon by the same path {tool} runs from",
                target.kit_root.join("tags").display()
            ));
        }
        if target.tool != KitTool::Sapien {
            return Ok(KitToolDropPlan {
                file,
                palette: None,
            });
        }
        // Which palette is the scenario definition's call, and the definition
        // is the source kit's game. A kit Baboon has not loaded, or whose
        // definitions it cannot read, gets no gate: Sapien decides.
        let Some(game) = self.game_of_loaded_kit_containing(&file) else {
            return Ok(KitToolDropPlan {
                file,
                palette: None,
            });
        };
        let Some(palettes) = self.scenario_palettes_for_game(&game) else {
            return Ok(KitToolDropPlan {
                file,
                palette: None,
            });
        };
        let palette = palettes_for_group(palettes, payload.group_tag)
            .first()
            .map(|palette| palette.name.clone());
        if palette.is_none() {
            let extension = file
                .extension()
                .map(|extension| extension.to_string_lossy().into_owned())
                .unwrap_or_else(|| "this".to_owned());
            return Err(format!(
                "Sapien has no scenario palette for .{extension} tags; it takes objects such as weapons, vehicles, bipeds, crates and scenery"
            ));
        }
        Ok(KitToolDropPlan { file, palette })
    }

    /// The game of the loaded editing kit whose tags folder holds `file`.
    fn game_of_loaded_kit_containing(&self, file: &Path) -> Option<String> {
        (0..self.kits.len()).find_map(|kit_index| {
            let kit_root = self.editing_kit_root_for(kit_index)?;
            if !tag_within_kit(file, &kit_root) {
                return None;
            }
            self.kits[kit_index].source.as_ref()?.game.clone()
        })
    }

    /// The scenario palette table for `game`, once a worker has read it from
    /// the definitions. The first ask starts that read and answers `None`, as
    /// does a definition that could not be read: the UI thread does not wait
    /// on disk, least of all with a drag in hand.
    fn scenario_palettes_for_game(&mut self, game: &str) -> Option<&[ScenarioPalette]> {
        if !self.kit_tool_drag.palettes.contains_key(game) {
            self.kit_tool_drag
                .palettes
                .insert(game.to_owned(), PaletteTable::Loading);
            let tx = self.tx.clone();
            let game = game.to_owned();
            thread::spawn(move || {
                let palettes = scenario_palettes(&locate_definitions_root(), &game).ok();
                let _ = tx.send(WorkerMessage::ScenarioPalettesRead { game, palettes });
            });
        }
        match self.kit_tool_drag.palettes.get(game) {
            Some(PaletteTable::Ready(palettes)) => Some(palettes.as_slice()),
            _ => None,
        }
    }

    fn drop_tag_on_kit_tool(
        &mut self,
        target: &KitToolDropTarget,
        plan: &KitToolDropPlan,
    ) -> String {
        let leaf = file_leaf(&plan.file);
        match deliver_file_drop(target, &plan.file) {
            Ok(()) => match (target.tool, &plan.palette) {
                (KitTool::Sapien, Some(palette)) => {
                    format!("Handed {leaf} to Sapien for its {palette}")
                }
                (KitTool::Sapien, None) => format!("Handed {leaf} to Sapien"),
                (KitTool::Guerilla, _) => format!("Handed {leaf} to Guerilla to open"),
            },
            Err(error) => error,
        }
    }
}

fn hover_message(tool: KitTool, plan: &KitToolDropPlan) -> String {
    let leaf = file_leaf(&plan.file);
    match (tool, &plan.palette) {
        (KitTool::Sapien, Some(palette)) => {
            format!("Release to add {leaf} to Sapien's {palette}")
        }
        (KitTool::Sapien, None) => format!("Release to hand {leaf} to Sapien"),
        (KitTool::Guerilla, _) => format!("Release to open {leaf} in Guerilla"),
    }
}

fn file_leaf(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string())
}
