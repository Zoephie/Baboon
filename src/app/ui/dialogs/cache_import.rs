//! The Import Cache Folder window: converting a monolithic cache's tags into an editing kit.
//! It owns presentation and action collection; the conversion run and its report belong to the controller.

use super::*;

/// What the Import Cache Folder window asks for, collected during the render
/// pass and applied after it.
///
/// Same reason the Import Tags dialog does it: every handler wants `&mut self`
/// while the dialog it was clicked in is still borrowed.
enum CacheImportAction {
    Start,
    /// Run again over the ticked folders of what the last run reached for.
    ImportOutside,
    /// Find out which of the tags being imported the kit already has, so the
    /// user can say which of those to replace.
    ScanConflicts,
    Cancel,
    Close,
}

/// One folder of the outside-reference tree, and everything under it.
///
/// A folder's tick is whether *all* of its subtree is ticked, and setting it
/// sets the subtree; the count beside it is what says the answer is partial,
/// which a two-state box on its own cannot. Folders that hold exactly one thing
/// still get a row, because collapsing them would hide where a tag lives, and
/// where it lives is most of what the answer is being given about.
fn draw_outside_folder(
    ui: &mut egui::Ui,
    tree: &OutsideTree,
    picked: &mut std::collections::BTreeMap<String, bool>,
    folder: &str,
    depth: usize,
) {
    let (chosen, total) = tree.tally(folder, picked);
    let leaf = folder.rsplit('/').next().unwrap_or(folder);
    let mut all = chosen == total && total > 0;
    let id = ui.make_persistent_id(("cache_import_outside_folder", folder));
    // Open at the top so the first level is readable without a click, closed
    // below it so a folder of two thousand tags does not arrive expanded.
    let state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth == 0);
    state
        .show_header(ui, |ui| {
            if ui.checkbox(&mut all, "").changed() {
                for key in tree.keys_under(folder) {
                    picked.insert(key, all);
                }
            }
            let label = if chosen == total {
                format!("{leaf}  ({total})")
            } else {
                format!("{leaf}  ({chosen} of {total})")
            };
            let text = RichText::new(label);
            ui.label(if chosen == 0 {
                text.color(subtle_dark())
            } else {
                text
            });
        })
        .body(|ui| {
            if let Some(children) = tree.folders.get(folder) {
                for child in children {
                    let path = if folder.is_empty() {
                        child.clone()
                    } else {
                        format!("{folder}/{child}")
                    };
                    draw_outside_folder(ui, tree, picked, &path, depth + 1);
                }
            }
            if let Some(tags) = tree.tags.get(folder) {
                for (key, name) in tags {
                    let mut wanted = picked.get(key).copied().unwrap_or(false);
                    if ui
                        .checkbox(&mut wanted, RichText::new(name).monospace())
                        .changed()
                    {
                        picked.insert(key.clone(), wanted);
                    }
                }
            }
        });
}

/// Everything inside the Import Cache Folder window.
///
/// Split out from the window itself so it can be rendered against a dialog on
/// its own. A window body is where an egui id collision or a borrow that only
/// fails at runtime shows up, and neither is visible to a compile.
fn draw_cache_import_body(
    ui: &mut Ui,
    ctx: &egui::Context,
    dialog: &mut CacheImportDialog,
) -> Option<CacheImportAction> {
    let mut action = None;
    ui.label(RichText::new("From").strong());
    ui.label(
        RichText::new(match dialog.single.as_ref() {
            Some(single) => single.display_path.clone(),
            None if dialog.prefix.is_empty() => {
                format!("the whole cache — {} tag(s)", dialog.selected)
            }
            None => format!("{} — {} tag(s)", dialog.prefix, dialog.selected),
        })
        .monospace(),
    );

    ui.add_space(8.0);
    ui.label(RichText::new("Into").strong());
    let selected_label = dialog
        .target()
        .map(|target| format!("{} ({})", target.label, target.game))
        .unwrap_or_else(|| "no kit loaded".to_owned());
    let target_before = dialog.target_index;
    ui.add_enabled_ui(!dialog.running, |ui| {
        egui::ComboBox::from_id_salt("cache_import_target")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for (index, target) in dialog.targets.iter().enumerate() {
                    ui.selectable_value(
                        &mut dialog.target_index,
                        index,
                        format!("{} ({})", target.label, target.game),
                    );
                }
            });
    });
    // Another kit holds another set of tags, so what the last scan found is an
    // answer about somewhere else.
    if dialog.target_index != target_before {
        dialog.conflicts_stale = true;
    }
    // Where it lands. Their own paths by default, for both a folder and a
    // single tag: every reference names the path the build gave a tag, so a tag
    // written anywhere else is one nothing points at. Somewhere else is a real
    // answer to a real question -- trying a build's version of a level beside
    // the kit's own -- and the window says what it costs rather than refusing.
    let tags_root = dialog.target().map(|target| target.tags_root.clone());
    if let Some(tags_root) = tags_root {
        ui.add_space(6.0);
        ui.label(RichText::new("Where").strong());
        let one = dialog.single.is_some();
        ui.add_enabled_ui(!dialog.running, |ui| {
            let own = format!(
                "{} own path{} under {}",
                if one { "Its" } else { "Their" },
                if one { "" } else { "s" },
                tags_root.display()
            );
            let mut chosen = dialog.destination.is_some();
            if ui.radio_value(&mut chosen, false, own).clicked() {
                dialog.destination = None;
                dialog.conflicts_stale = true;
            }
            ui.horizontal(|ui| {
                ui.radio_value(&mut chosen, true, "A folder I choose");
                if ui.button("Choose folder...").clicked()
                    && let Some(picked) = rfd::FileDialog::new()
                        .set_directory(&tags_root)
                        .pick_folder()
                {
                    // Inside the kit or not at all: a tag written outside the
                    // tags root is not in the kit, whatever the path says.
                    dialog.destination =
                        picked.strip_prefix(&tags_root).ok().map(Path::to_path_buf);
                    dialog.conflicts_stale = true;
                }
            });
            // Ticking the radio without picking a folder yet would otherwise
            // leave the two disagreeing about what was chosen.
            if chosen && dialog.destination.is_none() {
                dialog.destination = Some(PathBuf::new());
                dialog.conflicts_stale = true;
            }
        });
        if let Some(folder) = dialog.destination.clone() {
            let name = match dialog.single.as_ref() {
                Some(single) => single.display_path.clone(),
                None => format!("{}/...", dialog.prefix.replace('\\', "/")),
            };
            let strip = match dialog.single.as_ref() {
                Some(single) => single.parent.clone(),
                None => dialog.prefix.clone(),
            };
            let leaf = name
                .replace('\\', "/")
                .strip_prefix(&strip.replace('\\', "/"))
                .map(|rest| rest.trim_start_matches('/').to_owned())
                .filter(|rest| !rest.is_empty())
                .unwrap_or_else(|| name.rsplit(['\\', '/']).next().unwrap_or(&name).to_owned());
            ui.label(
                RichText::new(tags_root.join(&folder).join(leaf).display().to_string())
                    .monospace()
                    .small(),
            );
            ui.label(
                RichText::new(
                    "Off their own paths, so the tags that reference these will not find \
                     them: a reference names the path the build gave it, and nothing here \
                     rewrites those.",
                )
                .small()
                .color(Color32::from_rgb(242, 196, 48)),
            );
        }

        // What to do about tags the kit already has. Replacing was the only
        // behaviour, which is right for a kit being filled from a build and
        // wrong for one that has been worked in.
        ui.add_space(8.0);
        ui.label(RichText::new("Tags the kit already has").strong());
        ui.add_enabled_ui(!dialog.running, |ui| {
            let before = dialog.replace;
            ui.radio_value(&mut dialog.replace, ReplaceChoice::Always, "Replace them");
            ui.radio_value(&mut dialog.replace, ReplaceChoice::Never, "Keep them");
            ui.radio_value(&mut dialog.replace, ReplaceChoice::Chosen, "Let me pick");
            if dialog.replace != before && dialog.replace == ReplaceChoice::Chosen {
                dialog.conflicts_stale = true;
            }
        });
        if dialog.replace == ReplaceChoice::Chosen {
            if dialog.scanning {
                ui.label(
                    RichText::new("Looking for what the kit already has...")
                        .small()
                        .color(subtle_dark()),
                );
            } else if dialog.conflicts_stale {
                action = Some(CacheImportAction::ScanConflicts);
            } else {
                let total = dialog.conflicts.totals.values().copied().max().unwrap_or(0);
                if total == 0 {
                    ui.label(
                        RichText::new("The kit has none of these yet — nothing to replace.")
                            .small()
                            .color(subtle_dark()),
                    );
                } else {
                    let chosen = dialog
                        .conflict_picked
                        .values()
                        .filter(|wanted| **wanted)
                        .count();
                    ui.label(
                        RichText::new(format!(
                            "Ticked tags are replaced; the rest are left as they are \
                             ({chosen} of {total})."
                        ))
                        .small()
                        .color(subtle_dark()),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("cache_import_conflicts")
                        .max_height(220.0)
                        .show(ui, |ui| {
                            let roots = dialog.conflicts.roots.clone();
                            for root in &roots {
                                draw_outside_folder(
                                    ui,
                                    &dialog.conflicts,
                                    &mut dialog.conflict_picked,
                                    root,
                                    0,
                                );
                            }
                            if dialog.conflicts.tags.contains_key("") {
                                draw_outside_folder(
                                    ui,
                                    &dialog.conflicts,
                                    &mut dialog.conflict_picked,
                                    "",
                                    0,
                                );
                            }
                        });
                }
            }
        }
    }

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if dialog.running {
            if ui.button("Cancel").clicked() {
                action = Some(CacheImportAction::Cancel);
            }
            ui.label(RichText::new("Importing...").color(subtle_dark()));
        } else {
            if ui
                .add_enabled(dialog.target().is_some(), egui::Button::new("Import"))
                .clicked()
            {
                action = Some(CacheImportAction::Start);
            }
            if dialog.report.is_some() && ui.button("Close").clicked() {
                action = Some(CacheImportAction::Close);
            }
        }
    });
    ui.label(
        RichText::new(
            "Existing tags at the same paths are replaced. Nothing is written for a \
             tag whose data cannot come across; the report names those.",
        )
        .small()
        .color(subtle_dark()),
    );

    if let Some(error) = dialog.error.as_ref() {
        ui.add_space(8.0);
        ui.label(RichText::new(error).color(material_delete_text()));
    }

    if let Some(progress) = dialog.progress.as_ref() {
        ui.add_space(8.0);
        let fraction = if progress.total == 0 {
            0.0
        } else {
            progress.processed as f32 / progress.total as f32
        };
        ui.label(RichText::new(&progress.phase).strong());
        ui.add(
            egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                .animate(progress.total == 0)
                .text(format!(
                    "{} / {} — {} imported, {} failed",
                    progress.processed, progress.total, progress.converted, progress.failed
                )),
        );
        if !progress.current.is_empty() {
            ui.label(
                RichText::new(&progress.current)
                    .monospace()
                    .small()
                    .color(subtle_dark()),
            );
        }
        // The total grows as references are followed, so the bar can
        // move backwards. Said once here rather than left to be
        // discovered.
        ui.label(
            RichText::new(
                "The total climbs as referenced tags are found, so the bar can slip \
                 back.",
            )
            .small()
            .color(subtle_dark()),
        );
        ctx.request_repaint();
    }

    if let Some(report) = dialog.report.as_ref() {
        ui.add_space(8.0);
        if report.cancelled {
            ui.label(
                RichText::new("Stopped early. Everything below was written.")
                    .color(Color32::from_rgb(242, 196, 48)),
            );
        }
        if !report.outside_references.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "{} tag(s) outside this folder are referenced by what just \
                     landed. Import them too?",
                    report.outside_references.len()
                ))
                .strong(),
            );
            ui.label(
                RichText::new(
                    "Open a folder to answer inside it. Anything left unticked stays                      out, and the tags that point at it keep a reference to a tag the                      kit does not have.",
                )
                .small()
                .color(subtle_dark()),
            );
            ui.horizontal(|ui| {
                if ui.button("All").clicked() {
                    for wanted in dialog.outside_picked.values_mut() {
                        *wanted = true;
                    }
                }
                if ui.button("None").clicked() {
                    for wanted in dialog.outside_picked.values_mut() {
                        *wanted = false;
                    }
                }
                let chosen = dialog
                    .outside_picked
                    .values()
                    .filter(|wanted| **wanted)
                    .count();
                ui.label(
                    RichText::new(format!(
                        "{chosen} of {} tag(s)",
                        dialog.outside_picked.len()
                    ))
                    .small()
                    .color(subtle_dark()),
                );
            });
            // Borrowed field by field: the report is held open around this, and
            // reaching for the whole dialog inside the closure would collide
            // with it.
            let outside_tree = &dialog.outside_tree;
            let outside_picked = &mut dialog.outside_picked;
            egui::ScrollArea::vertical()
                .id_salt("cache_import_outside")
                .max_height(260.0)
                .show(ui, |ui| {
                    for root in &outside_tree.roots {
                        draw_outside_folder(ui, outside_tree, outside_picked, root, 0);
                    }
                    // Tags with no folder of their own, if a build has any.
                    let loose: Vec<(String, String)> =
                        outside_tree.tags.get("").cloned().unwrap_or_default();
                    for (key, name) in loose {
                        let mut wanted = outside_picked.get(&key).copied().unwrap_or(false);
                        if ui
                            .checkbox(&mut wanted, RichText::new(&name).monospace())
                            .changed()
                        {
                            outside_picked.insert(key, wanted);
                        }
                    }
                });
            let picked = dialog
                .outside_picked
                .values()
                .filter(|wanted| **wanted)
                .count();
            if ui
                .add_enabled(
                    picked > 0 && !dialog.running,
                    egui::Button::new(format!("Import those too ({picked})")),
                )
                .clicked()
            {
                action = Some(CacheImportAction::ImportOutside);
            }
        }
        // Ahead of the rest, because it is the difference between a level that
        // runs and one that cannot, and the engine's own error for it names the
        // bsp and never mentions lighting.
        if !report.levels_without_lighting.is_empty() {
            ui.label(
                RichText::new(format!(
                    "{} level(s) came across without their baked lighting",
                    report.levels_without_lighting.len()
                ))
                .strong()
                .color(Color32::from_rgb(242, 196, 48)),
            );
            ui.label(
                RichText::new(
                    "This build kept no lightmap data for them — nothing here could                      bring it. Sapien will not open a level whose lighting is missing:                      it reports the bsp as having failed to load, which is what a bsp                      with no lightmap looks like from the inside. Bake lightmaps for                      these, or work with a level this build did keep them for.",
                )
                .small()
                .color(subtle_dark()),
            );
            egui::ScrollArea::vertical()
                .id_salt("cache_import_no_lighting")
                .max_height(110.0)
                .show(ui, |ui| {
                    for level in &report.levels_without_lighting {
                        ui.label(RichText::new(level).monospace().small());
                    }
                });
        }
        if !report.unresolved_references.is_empty() {
            ui.collapsing(
                RichText::new(format!(
                    "{} reference(s) name a tag the build does not hold",
                    report.unresolved_references.len()
                ))
                .color(subtle_dark()),
                |ui| {
                    ui.label(
                        RichText::new(
                            "Already broken in the source: these tags were gone \
                             before the build was made, so nothing here could bring \
                             them across.",
                        )
                        .small()
                        .color(subtle_dark()),
                    );
                    for missing in &report.unresolved_references {
                        ui.label(RichText::new(missing).monospace().small());
                    }
                },
            );
        }
        if !report.held_back.is_empty() {
            ui.label(
                RichText::new(format!(
                    "{} tag(s) were not written, because their data has no way \
                     across:",
                    report.held_back.len()
                ))
                .color(Color32::from_rgb(242, 196, 48)),
            );
            egui::ScrollArea::vertical()
                .id_salt("cache_import_held_back")
                .max_height(140.0)
                .show(ui, |ui| {
                    for entry in &report.held_back {
                        ui.label(
                            RichText::new(format!(
                                "{} — {}",
                                entry.source,
                                entry.losses.join("; ")
                            ))
                            .monospace()
                            .small(),
                        );
                    }
                });
        }
        draw_folder_import_report(ui, report);
    }
    action
}

impl Baboon {
    /// Import Cache Folder: convert a monolithic cache's tags into an editing
    /// kit.
    ///
    /// The window stays up for the whole run and keeps its report afterwards.
    /// A run of this can reach thousands of tags — following references out of
    /// a folder is the point — so the outcome is a document to read, not a
    /// status-bar line to catch.
    pub(in crate::app::ui) fn draw_cache_import_window(&mut self, ctx: &egui::Context) {
        if self.cache_import_dialog.is_none() {
            return;
        }
        let mut open = true;
        let mut action = None;
        egui::Window::new("Import Cache Folder")
            .id(egui::Id::new("cache_import"))
            .open(&mut open)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                if let Some(dialog) = self.cache_import_dialog.as_mut() {
                    action = draw_cache_import_body(ui, ctx, dialog);
                }
            });

        match action {
            Some(CacheImportAction::Start) => self.start_cache_import(ctx.clone(), None),
            Some(CacheImportAction::ImportOutside) => {
                let picked = self
                    .cache_import_dialog
                    .as_ref()
                    .map(|dialog| {
                        dialog
                            .report
                            .as_ref()
                            .map(|report| {
                                report
                                    .outside_references
                                    .iter()
                                    .filter(|reference| {
                                        dialog
                                            .outside_picked
                                            .get(&reference.key)
                                            .copied()
                                            .unwrap_or(false)
                                    })
                                    .map(|reference| reference.key.clone())
                                    .collect::<HashSet<String>>()
                            })
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                if !picked.is_empty() {
                    self.start_cache_import(ctx.clone(), Some(picked));
                }
            }
            Some(CacheImportAction::ScanConflicts) => self.scan_cache_import_conflicts(ctx.clone()),
            Some(CacheImportAction::Cancel) => {
                if let Some(dialog) = self.cache_import_dialog.as_ref() {
                    dialog.cancel.store(true, Ordering::Relaxed);
                }
                self.status = "Stopping the cache import".to_owned();
            }
            Some(CacheImportAction::Close) => self.cache_import_dialog = None,
            None => {}
        }
        // A run owns its window: closing it would leave a worker writing into a
        // kit with nothing left to report to.
        let running = self
            .cache_import_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.running);
        if !open && !running {
            self.cache_import_dialog = None;
        }
    }
}

#[cfg(test)]
mod cache_import_window_tests {
    use super::*;

    fn dialog(report: Option<FolderConversionReport>) -> CacheImportDialog {
        CacheImportDialog {
            kit: KitId(0),
            prefix: r"objects\weapons\rifle".to_owned(),
            selected: 12,
            targets: vec![CacheImportTarget {
                kit: KitId(1),
                label: "HREK".to_owned(),
                game: "haloreach_mcc".to_owned(),
                tags_root: PathBuf::from("D:/HREK/tags"),
            }],
            target_index: 0,
            outside_tree: OutsideTree::default(),
            outside_picked: std::collections::BTreeMap::new(),
            single: None,
            destination: None,
            replace: ReplaceChoice::Always,
            conflicts: OutsideTree::default(),
            conflict_picked: std::collections::BTreeMap::new(),
            conflicts_stale: true,
            scanning: false,
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: None,
            report,
            error: None,
        }
    }

    fn render(dialog: &mut CacheImportDialog) {
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(700.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    draw_cache_import_body(ui, ctx, dialog);
                });
            },
        );
    }

    /// Every state the window can be in draws.
    ///
    /// Worth a test on its own because none of what breaks here is visible to a
    /// compile: an egui id used twice, a scroll area nested where it cannot be,
    /// a borrow that only fails once the closure actually runs. The window has
    /// five shapes — asking for a folder, asking for one tag, running, failed,
    /// done — and the done one carries every list it can show at once,
    /// including a reference tree several folders deep.
    #[test]
    fn the_cache_import_window_draws_in_every_state() {
        render(&mut dialog(None));

        let mut running = dialog(None);
        running.running = true;
        running.progress = Some(FolderConversionProgress {
            phase: "Converting tags".to_owned(),
            current: r"objects\weapons\rifle\assault_rifle".to_owned(),
            processed: 40,
            total: 210,
            converted: 38,
            failed: 2,
        });
        render(&mut running);

        let mut failed = dialog(None);
        failed.error = Some("the destination kit went away".to_owned());
        render(&mut failed);

        // One tag, at its own path and at one the user picked: the second draws
        // a warning the first does not.
        let mut single = dialog(None);
        single.single = Some(SingleTagImport {
            key: r"cache:bitm:objects\weapons\rifle\bitmaps\ar_diffuse".to_owned(),
            display_path: r"objects\weapons\rifle\bitmaps\ar_diffuse.bitmap".to_owned(),
            parent: r"objects\weapons\rifle\bitmaps".to_owned(),
        });
        render(&mut single);
        single.destination = Some(PathBuf::from("scratch/imported"));
        render(&mut single);

        // The folder case now offers the same choice, and keeps its
        // shape under the folder that was picked rather than flattening.
        let mut moved = dialog(None);
        moved.destination = Some(PathBuf::from("scratch"));
        render(&mut moved);

        // Picking what to replace asks for a scan before it can draw
        // anything, and draws the answer once it has one.
        moved.replace = ReplaceChoice::Chosen;
        render(&mut moved);
        moved.conflicts_stale = false;
        render(&mut moved);
        moved.conflicts = OutsideTree::build(&[OutsideReference {
            key: r"cache:bitm:objects\weapons\rifle\bitmaps\ar_diffuse".to_owned(),
            display_path: "scratch/bitmaps/ar_diffuse.bitmap".to_owned(),
        }]);
        render(&mut moved);

        let mut done = dialog(Some(FolderConversionReport {
            source_root: PathBuf::from(r"objects\weapons\rifle"),
            source_game: "haloreach_mcc".to_owned(),
            target_game: "haloreach_mcc".to_owned(),
            destination_root: PathBuf::from("D:/HREK/tags"),
            files: vec![FolderConversionFileResult {
                source: "objects/weapons/rifle/assault_rifle.weapon".to_owned(),
                output: Some(PathBuf::from(
                    "D:/HREK/tags/objects/weapons/rifle/assault_rifle.weapon",
                )),
                status: FolderConversionFileStatus::GeneratedLayout,
                overwritten: true,
                detail: "Built from the target profile's own definitions".to_owned(),
            }],
            ignored_files: Vec::new(),
            held_back: vec![FolderConversionHeldBack {
                source: "objects/weapons/rifle/fp_assault_rifle.model_animation_graph".to_owned(),
                key: r"cache:jmad:objects\weapons\rifle\fp_assault_rifle".to_owned(),
                losses: vec!["the animation payload has no way across".to_owned()],
            }],
            outside_references: vec![
                OutsideReference {
                    key: r"cache:bitm:fx\decals\_bitmaps\scorch".to_owned(),
                    display_path: "fx/decals/_bitmaps/scorch.bitmap".to_owned(),
                },
                OutsideReference {
                    key: r"cache:rmt2:shaders\shader_templates\_0_0".to_owned(),
                    display_path: "shaders/shader_templates/_0_0.render_method_template".to_owned(),
                },
            ],
            unresolved_references: ["fx/decals/_bitmaps/gone.bitmap".to_owned()]
                .into_iter()
                .collect(),
            levels_without_lighting: [r"levels\multirchive8_boneyard_v2".to_owned()]
                .into_iter()
                .collect(),
            cancelled: true,
        }));
        // The tree the window draws is built when a run reports, so a fixture
        // that skips that step would exercise an empty one.
        if let Some(report) = done.report.as_ref() {
            done.outside_tree = OutsideTree::build(&report.outside_references);
            done.outside_picked = report
                .outside_references
                .iter()
                .map(|reference| (reference.key.clone(), true))
                .collect();
        }
        render(&mut done);
    }
}
