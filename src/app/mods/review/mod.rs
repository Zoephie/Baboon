//! Reviewing a mod before it is written: which tags it changes, how each
//! differs from what ships, and saving a diagnostic.

use super::*;

impl Baboon {
    /// Review what this workspace is carrying, without exporting anything.
    pub(in crate::app) fn review_changes(&mut self) {
        self.open_mod_review(true);
    }

    /// Re-derive an open review's rows from the workspace as it stands now.
    ///
    /// A duplicate that lands while the review is open adds a tag to the stash
    /// the review is describing, and a list that quietly does not include it is
    /// worse than no list — the point of the window is that what is reviewed
    /// and what is written cannot disagree.
    pub(in crate::app) fn refresh_open_mod_review(&mut self, kit: usize) {
        let Some(open) = self.mod_export.as_ref() else {
            return;
        };
        if self.resolve_kit(open.kit) != Some(kit) {
            return;
        }
        let Some((snapshot, rows)) = self.capture_mod_export_rows(kit) else {
            return;
        };
        let Some(dialog) = self.mod_export.as_mut() else {
            return;
        };
        // Whatever the user had already unticked stays unticked.
        let excluded: HashSet<String> = dialog
            .rows
            .iter()
            .filter(|row| !row.include)
            .map(|row| row.identity.clone())
            .collect();
        dialog.snapshot = snapshot;
        dialog.rows = rows
            .into_iter()
            .map(|mut row| {
                if excluded.contains(&row.identity) {
                    row.include = false;
                }
                row
            })
            .collect();
        dialog.diffs.clear();
    }

    /// Capture the stash and describe every tag in it, for the review window.
    pub(in crate::app) fn capture_mod_export_rows(
        &mut self,
        exporting: usize,
    ) -> Option<(CampaignProjectSnapshot, Vec<ModExportRow>)> {
        let snapshot = match self.capture_campaign_project(exporting, 0.0) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.status = "Export Mod is only for Campaign Evolved containers".to_owned();
                return None;
            }
            Err(error) => {
                self.status = format!("Could not checkpoint project for export: {error}");
                return None;
            }
        };
        let mut rows: Vec<ModExportRow> = snapshot
            .overlays
            .values()
            .map(|overlay| {
                // An overlay whose tag no longer resolves cannot be written.
                // That was previously counted into a status line and dropped;
                // it is a row here, so it is at least visible.
                let resolvable = self
                    .campaign_entry_for_identity(exporting, &overlay.identity)
                    .is_some();
                let kind = classify_overlay(
                    resolvable,
                    overlay.kind,
                    // Only asked where the answer can matter, so a review does
                    // not read shipped payloads it has no use for.
                    resolvable
                        && overlay.kind == CampaignProjectTagKind::Existing
                        && self.overlay_matches_shipped(exporting, overlay),
                );
                let overridden_by = self.mod_serving_tag(exporting, &overlay.identity);
                ModExportRow {
                    identity: overlay.identity.clone(),
                    display_path: overlay.logical_path.clone(),
                    group_tag: overlay.group_tag,
                    kind,
                    include: !matches!(
                        kind,
                        ModExportChange::Unresolved | ModExportChange::Unchanged
                    ),
                    bytes: overlay.bytes.len(),
                    reason: match kind {
                        ModExportChange::Unresolved => Some("not in this source".to_owned()),
                        ModExportChange::Unchanged => {
                            Some("identical to the game's copy".to_owned())
                        }
                        _ => None,
                    },
                    overridden_by,
                }
            })
            .collect();
        rows.sort_by(|a, b| a.display_path.cmp(&b.display_path));
        Some((snapshot, rows))
    }

    pub(in crate::app) fn open_mod_review(&mut self, review_only: bool) {
        let exporting = self.active;
        let Some((snapshot, rows)) = self.capture_mod_export_rows(exporting) else {
            return;
        };
        // The container source's root is already the game's `Paks` directory,
        // so a mod exported into its `~mods` needs no copying at all. The
        // folder is created here as well as at write time: it is what the
        // "Browse..." picker opens into and what the preview claims, and
        // neither can name a directory that does not exist yet. A failure is
        // ignored — the write path reports it properly, with the error.
        let folder = self.kits[exporting]
            .source
            .as_ref()
            .map(|source| default_mod_export_folder(source.source.root_path()))
            .unwrap_or_default();
        // Not for a review: looking at what is stashed should not leave a
        // directory behind in the game's install.
        if !review_only && !folder.as_os_str().is_empty() {
            let _ = fs::create_dir_all(&folder);
        }
        self.mod_export = Some(ModExportDialog {
            kit: self.active_kit_id(),
            review_only,
            snapshot,
            rows,
            // Whatever this session last exported, so a second export replaces
            // that mod instead of quietly making a new one beside it.
            name: self
                .last_mod_export_name
                .clone()
                .unwrap_or_else(|| "mymod".to_owned()),
            folder,
            overwrite_acknowledged: false,
            expanded: HashSet::new(),
            diffs: HashMap::new(),
            controls_height: 0.0,
        });
    }

    /// Whether a stashed overlay is byte-for-byte what the game already ships.
    ///
    /// Answered on bytes rather than by diffing parsed tags: a diff can come
    /// back empty for two tags that are not identical (a field the differ does
    /// not reach), and "nothing to export" has to mean *nothing*, not "nothing
    /// I looked at".
    ///
    /// A tag only a mod provides has no shipped counterpart, so it is never
    /// unchanged -- there is nothing for it to be identical to.
    pub(in crate::app) fn overlay_matches_shipped(&self, kit: usize, overlay: &CampaignProjectOverlay) -> bool {
        let Some(entry) = self.campaign_entry_for_identity(kit, &overlay.identity) else {
            return false;
        };
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return false;
        };
        matches!(
            crate::core::source::read_shipped_entry_bytes(&source.source, &entry),
            Ok(Some(bytes)) if bytes == *overlay.bytes
        )
    }

    /// Compute the field differences for one reviewed tag, against the tag as
    /// the game ships it.
    ///
    /// The baseline comes from the *shipped* containers, not from whatever the
    /// mount resolved the tag to -- which is the comparison the reviewer actually
    /// wants: what this mod changes about the game, not what changed since the
    /// last autosave, and not "nothing" because an earlier export of this very
    /// mod is installed under `Paks` and now serves the tag.
    pub(in crate::app) fn diff_reviewed_tag(&self, kit: usize, identity: &str) -> ModRowDiff {
        const LIMIT: usize = 5000;
        let failed = |error: String| ModRowDiff {
            rows: Vec::new(),
            base: None,
            edited: None,
            truncated: false,
            error: Some(error),
            view: Default::default(),
        };
        let Some(dialog) = self.mod_export.as_ref() else {
            return failed("The review is no longer open".to_owned());
        };
        let Some(overlay) = dialog.snapshot.overlays.get(identity) else {
            return failed("This tag is no longer in the export".to_owned());
        };
        let Some(entry) = self.campaign_entry_for_identity(kit, identity) else {
            return failed("This tag is no longer in the source".to_owned());
        };
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return failed("No source loaded".to_owned());
        };
        let edited_tag = match TagFile::read_from_bytes(&overlay.bytes) {
            Ok(edited) => edited,
            Err(error) => return failed(format!("Could not read the edited tag: {error}")),
        };
        // A tag this workspace created has no shipped counterpart to compare
        // against, so the whole tag is described instead.
        let describe_whole = |edited_tag: TagFile, names: &TagNameIndex| {
            let (rows, truncated) = describe_tag(&edited_tag, names, LIMIT);
            ModRowDiff {
                rows,
                base: None,
                edited: Some(edited_tag),
                truncated,
                error: None,
                view: Default::default(),
            }
        };
        if overlay.kind == CampaignProjectTagKind::New {
            return describe_whole(edited_tag, &self.kits[kit].names);
        }
        let base = match crate::core::source::read_shipped_entry(&source.source, &entry) {
            Ok(Some(base)) => base,
            // Only a mod carries this tag, so there is no shipped version to
            // difference against — describing it whole is the honest answer, and
            // it is what the reviewer needs to see either way.
            Ok(None) => return describe_whole(edited_tag, &self.kits[kit].names),
            Err(error) => return failed(format!("Could not read the shipped tag: {error}")),
        };
        let (rows, truncated) = diff_tags(&base, &edited_tag, &self.kits[kit].names, LIMIT);
        ModRowDiff {
            rows,
            base: Some(base),
            edited: Some(edited_tag),
            truncated,
            error: None,
            view: Default::default(),
        }
    }

    /// Dump everything the review is working from into `folder`, so a diff
    /// that looks wrong can be reproduced away from the UI.
    ///
    /// Writes the computed rows as JSON, and for every tag both sides as raw
    /// bytes: the tag as the game ships it and the tag as this workspace has
    /// it. Those two files are enough to re-run the comparison exactly.
    pub(in crate::app) fn save_review_diagnostic(&mut self, folder: PathBuf) -> Result<usize, String> {
        let Some(kit) = self
            .mod_export
            .as_ref()
            .map(|dialog| dialog.kit)
            .and_then(|kit| self.resolve_kit(kit))
        else {
            return Err("The review is no longer open".to_owned());
        };
        let identities: Vec<String> = self
            .mod_export
            .as_ref()
            .map(|dialog| dialog.rows.iter().map(|row| row.identity.clone()).collect())
            .unwrap_or_default();

        let mut tags = Vec::new();
        for identity in identities {
            // Computed on demand, so a diagnostic does not depend on which rows
            // the user happened to expand.
            if !self
                .mod_export
                .as_ref()
                .is_some_and(|dialog| dialog.diffs.contains_key(&identity))
            {
                let diff = self.diff_reviewed_tag(kit, &identity);
                if let Some(dialog) = self.mod_export.as_mut() {
                    dialog.diffs.insert(identity.clone(), diff);
                }
            }
            let Some(dialog) = self.mod_export.as_ref() else {
                break;
            };
            let Some(diff) = dialog.diffs.get(&identity) else {
                continue;
            };
            let Some(row) = dialog.rows.iter().find(|row| row.identity == identity) else {
                continue;
            };
            let stem: String = identity
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            for (suffix, tag) in [
                ("base", diff.base.as_ref()),
                ("edited", diff.edited.as_ref()),
            ] {
                let Some(tag) = tag else { continue };
                let bytes = tag
                    .write_to_bytes()
                    .map_err(|error| format!("Could not serialize {identity}: {error}"))?;
                std::fs::write(folder.join(format!("{stem}.{suffix}.tag")), bytes)
                    .map_err(|error| format!("Could not write {stem}.{suffix}.tag: {error}"))?;
            }
            tags.push(serde_json::json!({
                "identity": identity,
                "path": row.display_path,
                "kind": match row.kind {
                    ModExportChange::New => "new",
                    ModExportChange::Modified => "modified",
                    ModExportChange::Unresolved => "unresolved",
                    ModExportChange::Unchanged => "unchanged",
                },
                "bytes": row.bytes,
                "error": diff.error,
                "truncated": diff.truncated,
                "rows": diff
                    .rows
                    .iter()
                    .map(|row| serde_json::json!({
                        "path": row.path,
                        "base_path": row.base_path,
                        "before": row.a,
                        "after": row.b,
                    }))
                    .collect::<Vec<_>>(),
            }));
        }
        let count = tags.len();
        let document = serde_json::json!({ "tags": tags });
        let text = serde_json::to_string_pretty(&document)
            .map_err(|error| format!("Could not encode the diagnostic: {error}"))?;
        std::fs::write(folder.join("review-diagnostic.json"), text)
            .map_err(|error| format!("Could not write review-diagnostic.json: {error}"))?;
        Ok(count)
    }
}

/// What a stashed overlay is, for the export review.
///
/// Pure so the rule can be tested without a mounted game, which is what
/// reproducing the report needed: a tag stashed as modified whose bytes turn
/// out to equal the game's own copy.
pub(in crate::app) fn classify_overlay(
    resolvable: bool,
    kind: CampaignProjectTagKind,
    matches_shipped: bool,
) -> ModExportChange {
    match (resolvable, kind) {
        (false, _) => ModExportChange::Unresolved,
        (true, CampaignProjectTagKind::New) => ModExportChange::New,
        (true, CampaignProjectTagKind::Existing) if matches_shipped => ModExportChange::Unchanged,
        (true, CampaignProjectTagKind::Existing) => ModExportChange::Modified,
    }
}

/// Which wrapper the exporter hands the writer for a tag it writes whole.
///
/// The two ways a tag reaches that path want opposite treatment, and the
/// difference is already in the location. A copy Baboon made sits in a
/// container under `Container`, and the `.uasset` resolved for it is the tag it
/// was copied from — so its bindings are this tag's bindings. A tag authored
/// through New Tag sits under `NewContainer` with a *donor* recorded, some
/// unrelated tag that supplies structure only.
///
/// Getting this backwards is not a loud failure: an authored tag that kept its
/// donor's bindings presents as the donor, and a copy that lost its own
/// presents as nothing at all.
pub(in crate::app) fn wrapper_origin_for(
    location: &TagEntryLocation,
) -> Option<blam_tags::iostore::writer::WrapperOrigin> {
    use blam_tags::iostore::writer::WrapperOrigin;
    match location {
        TagEntryLocation::Container { .. } => Some(WrapperOrigin::Copy),
        TagEntryLocation::NewContainer { .. } => Some(WrapperOrigin::Template),
        _ => None,
    }
}
