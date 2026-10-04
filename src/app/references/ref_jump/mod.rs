//! Jumping from a reference to the tag and field it names, and from a tag to
//! every place that references it.

use super::*;

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
            && self.kit_index(nav.kit).is_none()
        {
            self.references.field_nav = None;
        }
        if let Some(hit) = self.search.pending_find_jump.clone() {
            if self.kits[self.active].selected_key.as_deref() == Some(hit.tag_key.as_str())
                && self.kits[self.active]
                    .parsed_tags
                    .contains_key(&hit.tag_key)
            {
                self.activate_find_occurrence(ctx, hit);
            }
        }
        let Some(jump) = self.references.pending_ref_jump.clone() else {
            return;
        };
        // The jump belongs to the kit it was queued from; if that kit closed
        // while the referrer was loading, drop it.
        let Some(kit) = self.kit_index(jump.kit) else {
            self.references.pending_ref_jump = None;
            return;
        };
        // Wait until the referrer is the focused tab and finished loading.
        if self.kits[kit].selected_key.as_deref() != Some(jump.tag_key.as_str()) {
            return;
        }
        let Some(doc) = self.kits[kit].parsed_tags.get(&jump.tag_key) else {
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
                self.status = format!(
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
        // Scroll the exact leaf field into view next frame, plus the enclosing
        // block header as a fallback for non-value leaves.
        ctx.data_mut(|data| data.insert_temp(field_jump_target_id(), field_path.to_owned()));
        if let Some(block) = parent_block_path(field_path) {
            ctx.data_mut(|data| data.insert_temp(jump_target_id(), block));
        }
        self.references.field_nav = Some(FieldNav {
            kit: self.active_kit_id(),
            tag_key: tag_key.to_owned(),
            field_path: field_path.to_owned(),
            block_indices: ancestor_block_indices(field_path),
            glow_until: ctx.input(|input| input.time) + 2.5,
        });
        ctx.request_repaint();
    }

    /// Populate `ref_jump_occurrences` for any expanded, uncached referrer row in
    /// the current "References to X" popup. Parsed referrers are walked in place;
    /// unparsed ones trigger a background load and stay uncached ("loading…").
    pub(in crate::app) fn refresh_ref_jump_occurrences(&mut self, ctx: &egui::Context) {
        let Some((group_tag, rel_path)) = self
            .search.query_results
            .as_ref()
            .and_then(|results| results.ref_target.clone())
        else {
            return;
        };
        // Snapshot (row, key) for expanded-but-uncached rows before borrowing
        // `parsed_tags` / triggering loads.
        let pending: Vec<(usize, String)> = self
            .search.query_results
            .as_ref()
            .map(|results| {
                self.references.ref_jump_expanded
                    .iter()
                    .filter(|index| !self.references.ref_jump_occurrences.contains_key(index))
                    .filter_map(|&index| {
                        results
                            .entries
                            .get(index)
                            .map(|entry| (index, entry.key.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let target = normalize_ref(&rel_path);
        for (index, key) in pending {
            if let Some(doc) = self.kits[self.active].parsed_tags.get(&key) {
                let occurrences = ref_occurrences_in(&doc.tag, group_tag, &target);
                self.references.ref_jump_occurrences.insert(index, occurrences);
                continue;
            }
            // Not open: read and walk it on a worker. This used to go through
            // the tab loader, which drops results for tags without a tab, so
            // the row asked again as soon as each load finished — forever.
            if !self.references.ref_jump_loading.insert(index) {
                continue;
            }
            let Some(entry) = self.entry_for_key(&key).cloned() else {
                self.references.ref_jump_loading.remove(&index);
                self.references.ref_jump_occurrences.insert(index, Vec::new());
                continue;
            };
            let Some(source_kind) = self.source().map(|source| source.source.clone()) else {
                self.references.ref_jump_loading.remove(&index);
                continue;
            };
            let kit = self.active_kit_id();
            // The popup's own target, as `handle_ref_jump_occurrences` compares
            // it; the walk matches against the normalized form.
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

    /// Applies `WorkerMessage::RefJumpOccurrences`. Dropped unless the popup
    /// still shows the same target with the same tag in that row.
    pub(in crate::app) fn handle_ref_jump_occurrences(
        &mut self,
        kit: KitId,
        index: usize,
        key: String,
        target: (u32, String),
        result: Result<Vec<RefOccurrence>, String>,
    ) -> bool {
        self.references.ref_jump_loading.remove(&index);
        let current = kit == self.active_kit_id()
            && self.search.query_results.as_ref().is_some_and(|results| {
                results.ref_target.as_ref() == Some(&target)
                    && results
                        .entries
                        .get(index)
                        .is_some_and(|entry| entry.key == key)
            });
        if !current {
            return true;
        }
        let occurrences = match result {
            Ok(occurrences) => occurrences,
            Err(error) => {
                self.status = format!("Could not read the referring tag: {error}");
                Vec::new()
            }
        };
        self.references.ref_jump_occurrences.insert(index, occurrences);
        false
    }

    /// Resolve a pending "Open referenced tag" request against its active
    /// source. Loose folders resolve a file path; Campaign Evolved resolves the
    /// existing stable entry from its mounted container catalog.
    pub(in crate::app) fn process_pending_open(&mut self, ctx: &egui::Context) {
        let Some(req) = self.references.pending_open.take() else {
            return;
        };
        let container_key = self.source().and_then(|source| {
            matches!(&source.source, TagSource::IoStoreContainerSet { .. }).then(|| {
                container_entry_for_reference(
                    &source.entries,
                    req.group_tag,
                    &req.rel_path,
                    self.names(),
                )
                .map(|entry| entry.key.clone())
            })
        });
        if let Some(container_key) = container_key {
            let Some(key) = container_key else {
                self.status = format!(
                    "Referenced Campaign Evolved tag not found: {} (group {})",
                    req.rel_path.replace('\\', "/"),
                    blam_tags::format_group_tag(req.group_tag)
                );
                return;
            };
            self.select_entry(key.clone(), ctx.clone());
            if req.float {
                self.kits[self.active].open_tag_pane_beside(&key);
            }
            return;
        }

        let root = match self.source().map(|s| &s.source) {
            Some(TagSource::LooseFolder { root, .. }) => root.clone(),
            _ => {
                self.status = "Open requires a loose-folder source".to_owned();
                return;
            }
        };
        // Resolve the file extension from the definitions name index first
        // (covers every group, e.g. collision_model/physics_model), falling
        // back to the built-in table.
        let ext = self
            .names()
            .name_for(req.group_tag)
            .or_else(|| blam_tags::paths::group_tag_to_extension(req.group_tag))
            .unwrap_or("");
        // Normalize: tolerate forward slashes and a path that already carries
        // its extension (e.g. a shader bitmap ref), so we don't double-append.
        let mut rel = req.rel_path.replace('/', "\\");
        if !ext.is_empty() {
            if let Some(stripped) = rel
                .strip_suffix(&format!(".{ext}"))
                .or_else(|| rel.strip_suffix(&format!(".{}", ext.to_ascii_uppercase())))
            {
                rel = stripped.to_owned();
            }
        }
        let abs = blam_tags::paths::resolve_tag_path(&root, &rel, ext);
        if !abs.exists() {
            self.status = format!(
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
        if self.entry_for_key(&key).is_none() {
            let names = self
                .source()
                .map(|source| source.names.clone())
                .unwrap_or_default();
            if let Ok(Some(entry)) = loose_file_entry(&root, &abs, &names) {
                let folder_seeds = self.kits[self.active].folder_seeds();
                if let Some(source) = self.source_mut() {
                    source.upsert_entry(entry, &folder_seeds);
                }
                self.kits[self.active].generation =
                    self.kits[self.active].generation.wrapping_add(1);
            }
        }
        self.select_entry(key.clone(), ctx.clone());
        // Alt-click asks for the tag beside the current one rather than as
        // another tab in the same group.
        if req.float {
            self.kits[self.active].open_tag_pane_beside(&key);
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
mod ref_jump_tests;
