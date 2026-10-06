//! Jumping from a reference to the tag and field it names, and from a tag to
//! every place that references it.

use super::*;

/// `rel` without a trailing `.ext`, in any case: a reference that already
/// carries its extension would otherwise get a second one appended.
fn without_extension<'a>(rel: &'a str, ext: &str) -> &'a str {
    if ext.is_empty() {
        return rel;
    }
    let Some(dot) = rel.len().checked_sub(ext.len() + 1) else {
        return rel;
    };
    match rel.get(dot..) {
        Some(tail) if tail.starts_with('.') && tail[1..].eq_ignore_ascii_case(ext) => &rel[..dot],
        _ => rel,
    }
}

impl Baboon {
    /// Once-per-frame driver for reference-jumps. Expires a finished glow, and —
    /// when a pending jump's referrer tag has become the focused, parsed tab —
    /// walks it for the exact field referencing the target and navigates there.
    pub(in crate::app) fn apply_field_nav(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        if let Some(nav) = &self.references.field_nav {
            if now >= nav.glow_until {
                self.references.field_nav = None;
            } else {
                // Keep frames coming so the glow expires on time even when idle.
                ctx.request_repaint();
            }
        }
        // A glow belongs to the kit whose tag it is; drop it once that kit is
        // gone rather than glowing a field in another game.
        if let Some(nav) = &self.references.field_nav
            && self.model.kit_index(nav.kit).is_none()
        {
            self.references.field_nav = None;
        }
        if let Some((kit, hit)) = self.search.pending_find_jump.clone() {
            // Fires in the kit it was found in, once that tag has loaded there.
            // Another kit with a tag under the same key is a different tag.
            match self.model.kit_index(kit) {
                None => self.search.pending_find_jump = None,
                Some(index)
                    if index == self.model.active
                        && self.model.kits[index].selected_key.as_deref() == Some(hit.tag_key.as_str())
                        && self.model.kits[index].parsed_tags.contains_key(&hit.tag_key) =>
                {
                    self.activate_find_occurrence(ctx, hit);
                }
                Some(_) => {}
            }
        }
        let Some(jump) = self.references.pending_ref_jump.clone() else {
            return;
        };
        // The jump belongs to the kit it was queued from; if that kit closed
        // while the referrer was loading, drop it.
        let Some(kit) = self.model.kit_index(jump.kit) else {
            self.references.pending_ref_jump = None;
            return;
        };
        // Wait until the referrer is the focused tab and finished loading.
        if self.model.kits[kit].selected_key.as_deref() != Some(jump.tag_key.as_str()) {
            return;
        }
        let Some(doc) = self.model.kits[kit].parsed_tags.get(&jump.tag_key) else {
            return; // still loading — retry next frame
        };
        let mut refs = Vec::new();
        collect_tag_references(doc.tag.root(), "", &mut refs);
        let target = normalize_ref(&jump.rel_path);
        let hit = refs.into_iter().find(|reference| {
            reference.group_tag == jump.group_tag && normalize_ref(&reference.rel_path) == target
        });
        self.references.pending_ref_jump = None;
        match hit {
            Some(reference) => self.navigate_to_field(ctx, &jump.tag_key, &reference.field_path),
            None => {
                self.model.status = format!(
                    "Could not locate the referencing field in {}",
                    jump.tag_key.replace('\\', "/")
                );
            }
        }
    }

    /// Drive the editor to reveal `field_path` in the tag `tag_key`: select the
    /// element index at every ancestor block, scroll the exact leaf into view,
    /// and glow it briefly. Scroll targets are written once via egui temp-data;
    /// element selection, the glow and force-open persist via `self.field_nav`.
    pub(in crate::app) fn navigate_to_field(
        &mut self,
        ctx: &egui::Context,
        tag_key: &str,
        field_path: &str,
    ) {
        let kit = self.model.active_kit_id();
        self.navigate_to_field_in(ctx, kit, tag_key, field_path);
    }

    /// [`Self::navigate_to_field`] in `kit`'s copy of the tag, for a jump
    /// started from a pane of a workspace that need not be the active one.
    pub(in crate::app) fn navigate_to_field_in(
        &mut self,
        ctx: &egui::Context,
        kit: KitId,
        tag_key: &str,
        field_path: &str,
    ) {
        // Scroll the exact leaf field into view next frame, plus the enclosing
        // block header as a fallback for non-value leaves.
        ctx.data_mut(|data| data.insert_temp(field_jump_target_id(), field_path.to_owned()));
        if let Some(block) = parent_block_path(field_path) {
            ctx.data_mut(|data| data.insert_temp(jump_target_id(), block));
        }
        self.references.field_nav = Some(FieldNav {
            kit,
            tag_key: tag_key.to_owned(),
            field_path: field_path.to_owned(),
            block_indices: ancestor_block_indices(field_path),
            glow_until: ctx.input(|input| input.time) + 2.5,
        });
        ctx.request_repaint();
    }

    /// Walk the occurrences of any expanded, unwalked row of the open
    /// "References to X" results. Open referrers are walked in place; the rest
    /// are read and walked on a worker, and show as loading until it answers.
    pub(in crate::app) fn refresh_ref_jump_occurrences(&mut self, ctx: &egui::Context) {
        let Some(window) = self.dialogs.get::<QueryResultsWindow>() else {
            return;
        };
        let Some((group_tag, rel_path)) = window.results.ref_target.clone() else {
            return;
        };
        // The results belong to the kit they were found in. The window stays
        // open across a switch to another game, whose source cannot answer
        // for these keys.
        let kit = window.results.kit;
        let Some(kit_index) = self.model.kit_index(kit) else {
            return;
        };
        let pending: Vec<(usize, String)> = window
            .expanded
            .iter()
            .filter(|index| !window.occurrences.contains_key(index))
            .filter_map(|&index| {
                window
                    .results
                    .entries
                    .get(index)
                    .map(|entry| (index, entry.key.clone()))
            })
            .collect();

        let target = normalize_ref(&rel_path);
        for (index, key) in pending {
            let Some(window) = self.dialogs.get_mut::<QueryResultsWindow>() else {
                return;
            };
            if let Some(doc) = self.model.kits[kit_index].parsed_tags.get(&key) {
                let occurrences = ref_occurrences_in(&doc.tag, group_tag, &target);
                window.occurrences.insert(index, occurrences);
                continue;
            }
            // Not open: read and walk it on a worker. This used to go through
            // the tab loader, which drops results for tags without a tab, so
            // the row asked again as soon as each load finished — forever.
            if !window.loading.insert(index) {
                continue;
            }
            let Some(entry) = self.model.entry_for_key_in(kit_index, &key).cloned() else {
                window.loading.remove(&index);
                window.occurrences.insert(index, Vec::new());
                continue;
            };
            let Some(source_kind) = self.model.kits[kit_index].source.as_ref().map(|source| source.source.clone()) else {
                window.loading.remove(&index);
                continue;
            };
            // The results' own target, as `handle_ref_jump_occurrences`
            // compares it; the walk matches against the normalized form.
            let query_target = (group_tag, rel_path.clone());
            let normalized = target.clone();
            let (panic_key, panic_target) = (key.clone(), query_target.clone());
            spawn_worker(
                &self.tx,
                ctx,
                move || {
                    let target = query_target;
                    let result = read_entry(&source_kind, &entry)
                        .map(|tag| ref_occurrences_in(&tag, target.0, &normalized))
                        .map_err(|error| format!("{error:#}"));
                    WorkerMessage::RefJumpOccurrences {
                        kit,
                        index,
                        key,
                        target,
                        result,
                    }
                },
                move |error| WorkerMessage::RefJumpOccurrences {
                    kit,
                    index,
                    key: panic_key,
                    target: panic_target,
                    result: Err(error),
                },
            );
        }
    }

    /// Applies `WorkerMessage::RefJumpOccurrences`. Dropped unless the results
    /// still show the same target with the same tag in that row.
    pub(in crate::app) fn handle_ref_jump_occurrences(
        &mut self,
        kit: KitId,
        index: usize,
        key: String,
        target: (u32, String),
        result: Result<Vec<RefOccurrence>, String>,
    ) -> bool {
        let Some(window) = self.dialogs.get_mut::<QueryResultsWindow>() else {
            return true;
        };
        window.loading.remove(&index);
        let current = kit == window.results.kit
            && window.results.ref_target.as_ref() == Some(&target)
            && window
                .results
                .entries
                .get(index)
                .is_some_and(|entry| entry.key == key);
        if !current {
            return true;
        }
        let occurrences = match result {
            Ok(occurrences) => occurrences,
            Err(error) => {
                self.model.status = format!("Could not read the referring tag: {error}");
                Vec::new()
            }
        };
        window.occurrences.insert(index, occurrences);
        false
    }

    /// Resolve a pending "Open referenced tag" request against its active
    /// source. Loose folders resolve a file path; Campaign Evolved resolves the
    /// existing stable entry from its mounted container catalog.
    pub(in crate::app) fn process_pending_open(&mut self, ctx: &egui::Context) {
        let Some(req) = self.references.pending_open.take() else {
            return;
        };
        let container_key = self.model.source().and_then(|source| {
            matches!(&source.source, TagSource::IoStoreContainerSet { .. }).then(|| {
                container_entry_for_reference(
                    &source.entries,
                    req.group_tag,
                    &req.rel_path,
                    self.model.names(),
                )
                .map(|entry| entry.key.clone())
            })
        });
        if let Some(container_key) = container_key {
            let Some(key) = container_key else {
                self.model.status = format!(
                    "Referenced Campaign Evolved tag not found: {} (group {})",
                    req.rel_path.replace('\\', "/"),
                    blam_tags::format_group_tag(req.group_tag)
                );
                return;
            };
            self.select_entry(key.clone(), ctx.clone());
            if req.float {
                self.kit_and_view(self.model.active).open_tag_pane_beside(&key);
            }
            return;
        }

        let root = match self.model.source().map(|s| &s.source) {
            Some(TagSource::LooseFolder { root, .. }) => root.clone(),
            _ => {
                self.model.status = "Open requires a loose-folder source".to_owned();
                return;
            }
        };
        // Resolve the file extension from the definitions name index first
        // (covers every group, e.g. collision_model/physics_model), falling
        // back to the built-in table.
        let ext = self
            .model.names()
            .name_for(req.group_tag)
            .or_else(|| blam_tags::paths::group_tag_to_extension(req.group_tag))
            .unwrap_or("");
        // Normalize: tolerate forward slashes and a path that already carries
        // its extension (e.g. a shader bitmap ref), so we don't double-append.
        let rel = without_extension(&req.rel_path.replace('/', "\\"), ext).to_owned();
        let abs = blam_tags::paths::resolve_tag_path(&root, &rel, ext);
        if !abs.exists() {
            self.model.status = format!(
                "Referenced tag not found: {} (group {})",
                abs.display(),
                blam_tags::format_group_tag(req.group_tag)
            );
            return;
        }
        let key = file_entry_key(&abs);
        // Ensure an entry exists so ensure_tag_loading can resolve it. Built by
        // the scanner's own constructor: this used to derive the display path
        // from the unstripped reference, which could double the extension.
        if self.model.entry_for_key(&key).is_none() {
            let names = self
                .model.source()
                .map(|source| source.names.clone())
                .unwrap_or_default();
            if let Ok(Some(entry)) = loose_file_entry(&root, &abs, &names) {
                let folder_seeds = self.model.kits[self.model.active].folder_seeds();
                if let Some(source) = self.source_mut() {
                    source.upsert_entry(entry, &folder_seeds);
                }
                self.model.kits[self.model.active].generation =
                    self.model.kits[self.model.active].generation.wrapping_add(1);
            }
        }
        self.select_entry(key.clone(), ctx.clone());
        // Alt-click asks for the tag beside the current one rather than as
        // another tab in the same group.
        if req.float {
            self.kit_and_view(self.model.active).open_tag_pane_beside(&key);
        }
    }
}

/// Where `tag` points at the reference `(group_tag, target)`, one row per
/// referencing field. `target` is already normalized.
pub(in crate::app) fn ref_occurrences_in(tag: &TagFile, group_tag: u32, target: &str) -> Vec<RefOccurrence> {
    let mut refs = Vec::new();
    collect_tag_references(tag.root(), "", &mut refs);
    refs.into_iter()
        .filter(|reference| {
            reference.group_tag == group_tag && normalize_ref(&reference.rel_path) == target
        })
        .map(|reference| RefOccurrence {
            label: occurrence_label(&reference.field_path),
            field_path: reference.field_path,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! The "References to X" popup's per-row occurrence walk.

    use super::*;
    use std::time::Duration;

    /// A reference that carries its own extension is stripped of it in any
    /// case; a mixed-case `.Bitmap` was left on and a second one appended.
    #[test]
    fn a_carried_extension_is_stripped_in_any_case() {
        for rel in [r"a\b.bitmap", r"a\b.BITMAP", r"a\b.Bitmap", r"a\b"] {
            assert_eq!(without_extension(rel, "bitmap"), r"a\b", "{rel}");
        }
        assert_eq!(without_extension(r"a\bbitmap", "bitmap"), r"a\bbitmap");
        assert_eq!(without_extension("map", "bitmap"), "map");
    }

    fn scratch_root(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("baboon-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("objects")).unwrap();
        root
    }

    /// A 64-byte header the folder probe recognises as a tag, and nothing after it.
    fn write_header_only_tag(path: &Path, group: &[u8; 4]) {
        let mut bytes = [0u8; 64];
        bytes[48..52].copy_from_slice(&u32::from_be_bytes(*group).to_le_bytes());
        bytes[60..64].copy_from_slice(b"MALB");
        std::fs::write(path, bytes).unwrap();
    }

    /// A Find hit waiting for its tag to load belongs to the kit it was found
    /// in. It used to fire in whichever kit next selected and loaded a tag
    /// under the same key, which is a different tag.
    #[test]
    fn a_waiting_find_jump_does_not_fire_in_another_kit() {
        let key = "file:/tags/objects/marine.biped".to_owned();
        let mut app = Baboon::for_test();
        let found_in = app.model.kits[0].id;
        app.add_kit();
        let definition = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions/haloce_evolved/biped.json");
        app.model.kits[1]
            .parsed_tags
            .insert(key.clone(), TagDocument::clean(TagFile::new(definition).unwrap()));
        app.kit_and_view(1).open_tag_pane(&key);
        let hit = crate::app::search::FindOccurrence {
            tag_key: key.clone(),
            field_path: "model".to_owned(),
            kind: crate::app::search::FindTargetKind::Label,
            text: "model".to_owned(),
            range: 0..5,
        };
        app.search.pending_find_jump = Some((found_in, hit));
        let ctx = egui::Context::default();

        app.apply_field_nav(&ctx);
        assert!(app.search.pending_find_jump.is_some(), "still waiting for its own kit");
        assert!(app.references.field_nav.is_none(), "nothing navigated in the other kit");

        app.remove_kit(found_in);
        app.apply_field_nav(&ctx);
        assert!(app.search.pending_find_jump.is_none(), "dropped with its kit");
    }

    /// An app with one loose kit holding a referrer tag, and the "References
    /// to" window open on it with its row expanded.
    fn referrer_in_open_results(name: &str) -> (Baboon, PathBuf) {
        let root = scratch_root(name);
        let path = root.join("objects/referrer.model");
        write_header_only_tag(&path, b"hlmt");
        let names = TagNameIndex::default();
        let entry = loose_file_entry(&root, &path, &names).unwrap().unwrap();

        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: root.clone(),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names,
            game: None,
            entries: vec![entry.clone()],
            tree: crate::core::source::build_folder_directory_tree(&root).unwrap(),
            group_tree: crate::core::source::build_group_tree(std::slice::from_ref(&entry)),
            all_entries: vec![entry.clone()],
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });
        app.dialogs.open(QueryResultsWindow::new(TagQueryResults {
            kit: app.model.active_kit_id(),
            title: "References to bitmaps/target".to_owned(),
            entries: vec![entry],
            annotations: Vec::new(),
            note: None,
            ref_target: Some((u32::from_be_bytes(*b"bitm"), "bitmaps/target".to_owned())),
        }));
        app.dialogs
            .get_mut::<QueryResultsWindow>()
            .unwrap()
            .expanded
            .insert(0);
        (app, root)
    }

    /// Three frames of the popup, each delivering whatever the last one started
    /// through the app's own message pump. Installing the source starts
    /// unrelated background work too, so only reads of the referrer — through
    /// either path — are counted.
    fn reads_over_three_frames(app: &mut Baboon) -> usize {
        let ctx = egui::Context::default();
        let mut loads = 0;
        for frame in 0..3 {
            app.refresh_ref_jump_occurrences(&ctx);
            let mut wait = if frame == 0 {
                Duration::from_secs(10)
            } else {
                Duration::from_millis(300)
            };
            let mut received = Vec::new();
            while let Ok(message) = app.rx.recv_timeout(wait) {
                if matches!(
                    message,
                    WorkerMessage::RefJumpOccurrences { .. } | WorkerMessage::TagLoaded { .. }
                ) {
                    loads += 1;
                }
                received.push(message);
                wait = Duration::from_millis(300);
            }
            for message in received {
                app.tx.send(message).unwrap();
            }
            app.process_worker_messages(&ctx);
        }
        loads
    }

    fn row_settled(app: &Baboon) -> bool {
        app.dialogs
            .get::<QueryResultsWindow>()
            .is_some_and(|window| window.occurrences.contains_key(&0))
    }

    /// An expanded row for a referrer that is not open must be read once. It used
    /// to go through the tab loader, which drops results for tags without a tab,
    /// so the row asked again as soon as each load finished — for as long as the
    /// popup stayed open.
    #[test]
    fn an_unopened_referrer_is_read_once_not_reloaded_forever() {
        let (mut app, root) = referrer_in_open_results("ref-jump");
        let loads = reads_over_three_frames(&mut app);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(loads, 1, "the referrer must be read exactly once");
        assert!(
            row_settled(&app),
            "the row must settle (here with no occurrences: the tag has no body)"
        );
    }

    /// The window stays open when the user moves to another game. Its rows
    /// are still read from the kit the results came from; they used to be
    /// looked up in the focused kit, which has no such tag, and settled empty
    /// without being read.
    #[test]
    fn a_row_expanded_from_another_game_reads_the_results_own_kit() {
        let (mut app, root) = referrer_in_open_results("ref-jump-other-kit");
        app.add_kit();
        assert_eq!(app.model.active, 1, "another game has focus");
        let loads = reads_over_three_frames(&mut app);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(loads, 1, "the referrer is read from its own kit");
        assert!(row_settled(&app));
    }
}
