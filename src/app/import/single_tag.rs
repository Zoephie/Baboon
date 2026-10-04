//! Importing one tag: checking it against each profile it could be, then
//! writing it in or over an existing tag.

use super::*;
use crate::app::controller::saving::load_new_tag_groups;
use crate::app::tag_ops::new_tag::normalize_container_tag_rel;

impl Baboon {
    /// Open the "Import tag" dialog: pick a self-describing MCC/Reach tag file,
    /// parse it, validate its schema against our JSON, and seed the dialog.
    /// `folder_rel` pre-fills the destination folder (from a right-clicked node).
    pub(in crate::app) fn begin_import_tag(&mut self, folder_rel: Option<String>) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        if !self.current_source_is_container() {
            self.status = "Import tag is only for Campaign Evolved containers".to_owned();
            return;
        }
        let Some(picked) = rfd::FileDialog::new().set_title("Import Tag").pick_file() else {
            return;
        };
        let bytes = match fs::read(&picked) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.status = format!("Could not read {}: {error}", picked.display());
                return;
            }
        };
        let tag = match TagFile::read_from_bytes(&bytes) {
            Ok(tag) => tag,
            Err(error) => {
                self.status = format!("Not a valid MCC tag file: {error}");
                return;
            }
        };
        if tag.classic_engine().is_some() || tag.endian != Endian::Le {
            self.status = "Only little-endian MCC tags can be imported".to_owned();
            return;
        }
        let group_tag = tag.header.group_tag;
        let group_name = self
            .source()
            .and_then(|s| s.names.name_for(group_tag))
            .map(str::to_owned)
            .or_else(|| group_tag_to_extension(group_tag).map(str::to_owned))
            .unwrap_or_else(|| format_group_tag(group_tag));
        let extension = group_tag_to_extension(group_tag)
            .unwrap_or(group_name.as_str())
            .to_owned();
        let (profile_verdicts, mode) = self.classify_import_source(group_tag, &tag);
        let name = picked
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported")
            .to_owned();
        self.import_tag_dialog = Some(ImportTagDialog {
            kit: self.active_kit_id(),
            source_path: picked,
            folder_rel: folder_rel.unwrap_or_default(),
            name,
            group_tag,
            group_name,
            extension,
            tag: Some(tag),
            mode,
            profile_verdicts,
            error: None,
        });
    }

    /// Convert the picked file for the dialog's chosen source profile and hold
    /// the draft. Nothing is registered until the user confirms, so this is
    /// safe to re-run as they change the profile.
    pub(in crate::app) fn analyze_import_conversion(&mut self) {
        let Some(dialog) = self.import_tag_dialog.as_mut() else {
            return;
        };
        let ImportMode::Convert { source_game, draft } = &mut dialog.mode else {
            return;
        };
        let Some(source) = dialog.tag.as_ref() else {
            dialog.error = Some("No tag to convert".to_owned());
            return;
        };
        let source_game = source_game.clone();
        match analyze_conversion(
            source,
            &source_game,
            GameId::CampaignEvolved.as_str(),
            &locate_definitions_root(),
            None,
        ) {
            Ok(analyzed) => {
                *draft = Some(analyzed);
                dialog.error = None;
            }
            Err(error) => {
                *draft = None;
                dialog.error = Some(error);
            }
        }
    }

    /// Work out how a picked file has to be landed, against the active source's
    /// game as the destination.
    pub(in crate::app) fn classify_import_source(
        &self,
        group_tag: u32,
        imported: &TagFile,
    ) -> (Vec<(String, ProfileFit)>, ImportMode) {
        let target_game = self
            .source()
            .and_then(|s| s.game)
            .unwrap_or(GameId::CampaignEvolved);
        classify_import_source_for(target_game.as_str(), group_tag, imported)
    }

    /// Apply the pending import: validate the schema gate, resolve the target
    /// path against existing tags, and either overwrite an existing tag's
    /// document (dirty, with a discard prompt if it has unsaved edits) or add a
    /// brand-new container tag.
    pub(in crate::app) fn confirm_import_tag(&mut self) {
        // The import is resolved and registered against the active kit's
        // source, so return to the workspace the dialog was opened for.
        let Some(kit) = self.import_tag_dialog.as_ref().map(|dialog| dialog.kit) else {
            return;
        };
        if !self.focus_navigation_kit(kit) {
            self.import_tag_dialog = None;
            self.status = "The workspace this import came from is closed".to_owned();
            return;
        }
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(dialog) = self.import_tag_dialog.as_mut() else {
            return;
        };
        // Schema gate. A file authored for another game has to be converted;
        // one that merely drifted from our Campaign Evolved definition can be
        // waved through. Splitting those two is the whole point of `ImportMode`
        // — the old gate saw only "not a match" and offered the same override
        // for both, which let a Halo Reach tag through on a tick box.
        match &dialog.mode {
            ImportMode::Convert { source_game, draft } => {
                if draft.is_none() {
                    dialog.error = Some(format!(
                        "This is a {source_game} tag. Analyze the conversion first — its bytes \
                         cannot be copied as they are."
                    ));
                    return;
                }
            }
            ImportMode::Native {
                comparison,
                import_anyway,
            } => {
                if let Some(cmp) = comparison {
                    if !cmp.group_match || !cmp.version_match || !cmp.root_size_match {
                        dialog.error = Some(
                            "Schema is incompatible (group, version, or size differs) — this \
                             tag doesn't match the base game's definition."
                                .to_owned(),
                        );
                        return;
                    }
                    if cmp.severity != blam_tags::LayoutSeverity::Match && !*import_anyway {
                        dialog.error = Some(
                            "Schema differs in field metadata. Tick \"Import anyway\" to \
                             proceed."
                                .to_owned(),
                        );
                        return;
                    }
                }
            }
        }
        let folder = normalize_container_tag_rel(&dialog.folder_rel);
        let leaf = normalize_container_tag_rel(&dialog.name);
        if leaf.is_empty() {
            dialog.error = Some("Enter a tag name".to_owned());
            return;
        }
        let logical = if folder.is_empty() {
            leaf
        } else {
            format!("{folder}/{leaf}")
        };
        let group_tag = dialog.group_tag;
        let group_name = dialog.group_name.clone();
        let extension = dialog.extension.clone();
        // In Convert mode the converted draft is what lands, not the file that
        // was picked — the picked bytes are the wrong shape, which is the whole
        // reason the mode exists.
        let converted = match &mut dialog.mode {
            ImportMode::Convert { draft, .. } => draft.take().map(|draft| draft.tag),
            ImportMode::Native { .. } => None,
        };
        let Some(mut tag) = converted.or_else(|| dialog.tag.take()) else {
            dialog.error = Some("No tag to import".to_owned());
            return;
        };
        // The generation belongs to the destination, not to the file that was
        // picked. Import only ever targets a Campaign Evolved container, and the
        // schema gate above compares layout rather than the file header — so a
        // tag authored for another kit, or by a Baboon old enough to leave the
        // header zeroed, would otherwise land in the paks claiming a generation
        // the simulation never ships.
        if let Err(error) = apply_editing_kit_mcc_header(&mut tag, GameId::CampaignEvolved.as_str()) {
            dialog.error = Some(error);
            return;
        }

        // Does a base-game tag already exist at this path+group?
        let existing = self.source().and_then(|s| match &s.source {
            TagSource::IoStoreContainerSet { index, .. } => index
                .lookup(group_tag, &logical)
                .map(|(c, r)| (c, r.to_owned())),
            _ => None,
        });
        if let Some((container, rel_path)) = existing {
            let key = self.source().and_then(|s| {
                s.entries
                    .iter()
                    .find(|e| {
                        matches!(&e.location, TagEntryLocation::Container { container: c, rel_path: rp }
                            if *c == container && rp == &rel_path)
                    })
                    .map(|e| e.key.clone())
            });
            let Some(key) = key else {
                self.import_tag_dialog = None;
                self.status = "Could not resolve the existing tag to overwrite".to_owned();
                return;
            };
            // Already open with unsaved edits → confirm discard first.
            if self.kits[self.active]
                .parsed_tags
                .get(&key)
                .map(|d| d.dirty.is_set())
                .unwrap_or(false)
            {
                self.import_discard_confirm = Some(PendingImport {
                    kit: self.active_kit_id(),
                    tag,
                    target_key: key,
                });
                self.import_tag_dialog = None;
                return;
            }
            self.apply_import_over_existing(&key, tag);
            self.import_tag_dialog = None;
        } else {
            match self.add_new_container_tag(&logical, group_tag, &group_name, &extension, tag) {
                Ok(()) => {
                    self.import_tag_dialog = None;
                    self.status = format!("Imported {logical}.{extension} (unsaved)");
                }
                Err(error) => {
                    if let Some(dialog) = self.import_tag_dialog.as_mut() {
                        dialog.error = Some(error);
                    }
                }
            }
        }
    }

    /// Replace an existing container tag's document with imported bytes, marked
    /// dirty (no pak write). Opens/selects the tab.
    pub(in crate::app) fn apply_import_over_existing(&mut self, key: &str, tag: TagFile) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        self.kits[self.active].open_tag_pane(key);
        self.kits[self.active].selected_key = Some(key.to_owned());
        self.kits[self.active]
            .parsed_tags
            .insert(key.to_owned(), TagDocument::modified(tag));
        let label = self.tag_path_label(key);
        self.status = format!("Imported over {label} (unsaved)");
    }

    /// If an import at `folder_rel`/`name` (group `group_tag`) would land on an
    /// existing base-game tag, return that tag's logical path; else `None` (a new
    /// tag). Used by the Import dialog's overwrite-vs-new banner.
    pub(in crate::app) fn import_overwrite_target(
        &self,
        folder_rel: &str,
        name: &str,
        group_tag: u32,
    ) -> Option<String> {
        let folder = normalize_container_tag_rel(folder_rel);
        let leaf = normalize_container_tag_rel(name);
        if leaf.is_empty() {
            return None;
        }
        let logical = if folder.is_empty() {
            leaf
        } else {
            format!("{folder}/{leaf}")
        };
        match &self.source()?.source {
            TagSource::IoStoreContainerSet { index, .. } => {
                index.lookup(group_tag, &logical).map(|_| logical)
            }
            _ => None,
        }
    }

    /// Resolve the pending "discard unsaved edits?" import confirmation.
    pub(in crate::app) fn apply_import_discard(&mut self) {
        let Some(pending) = self.import_discard_confirm.take() else {
            return;
        };
        if !self.focus_navigation_kit(pending.kit) {
            self.status = "The workspace this import came from is closed".to_owned();
            return;
        }
        self.apply_import_over_existing(&pending.target_key, pending.tag);
    }
}

/// Compare an imported tag's embedded layout against one profile's shipped JSON
/// definition for its group. `None` if that profile ships no schema for the
/// group, or the schema cannot be built.
pub(in crate::app) fn compare_import_against_profile(
    game: &str,
    group_tag: u32,
    imported: &TagFile,
) -> Option<blam_tags::LayoutComparison> {
    let group = load_new_tag_groups(game)
        .ok()?
        .into_iter()
        .find(|g| g.group_tag == group_tag)?;
    let expected = TagFile::new(&group.schema_path).ok()?;
    Some(blam_tags::compare_root_layout(&expected, imported))
}

/// How an imported tag's own layout fits one profile's definition of its group.
///
/// Uses the recursive comparison rather than `compare_root_layout`, because the
/// root is exactly where the interesting cases agree: a Halo Reach
/// `model_animation_graph` and a Campaign Evolved one declare identical root
/// structs and diverge four structs down.
pub(in crate::app) fn profile_fit(game: &str, group_tag: u32, imported: &TagFile) -> Option<ProfileFit> {
    let group = load_new_tag_groups(game)
        .ok()?
        .into_iter()
        .find(|g| g.group_tag == group_tag)?;
    let expected = TagFile::new(&group.schema_path).ok()?;
    if expected.header.group_tag != imported.header.group_tag {
        return Some(ProfileFit::WrongGroup);
    }
    Some(
        match blam_tags::struct_trees_are_wire_identical(
            expected.definitions().root_struct(),
            imported.definitions().root_struct(),
        ) {
            Ok(_) => ProfileFit::Identical,
            Err(mismatch) => ProfileFit::Diverges(mismatch.to_string()),
        },
    )
}

/// Work out how a picked file has to be landed in `target_game`, by comparing it
/// against every profile that defines its group.
///
/// Import only ever targets a Campaign Evolved container, so the question is not
/// "which game is this?" in the abstract — it is "can these bytes be copied, or
/// do they have to be converted first?". A file whose layout is wire-identical
/// to the destination's is copied. A file wire-identical to some *other*
/// profile is that game's tag, whatever its extension claims, and its bytes
/// cannot be copied.
///
/// Root-level comparison cannot make this call. Reach and Campaign Evolved
/// declare the `model_animation_graph` root field-for-field identically, so
/// `compare_root_layout` reports a clean match for a Reach animation graph —
/// no warning, no override, straight into the paks with
/// `shared_model_animation_block` 12 bytes too long. Closing that is why this
/// walks the whole struct graph.
///
/// The returned verdict list is evidence, not a decision. It seeds the mode and
/// stays on the dialog so the user can see why, and correct an unusual file.
pub(in crate::app) fn classify_import_source_for(
    target_game: &str,
    group_tag: u32,
    imported: &TagFile,
) -> (Vec<(String, ProfileFit)>, ImportMode) {
    let mut verdicts = CONVERSION_PROFILES
        .iter()
        .filter_map(|profile| {
            profile_fit(profile, group_tag, imported).map(|fit| ((*profile).to_owned(), fit))
        })
        .collect::<Vec<_>>();
    verdicts.sort_by(|a, b| a.0.cmp(&b.0));

    let native = || ImportMode::Native {
        comparison: compare_import_against_profile(target_game, group_tag, imported),
        import_anyway: false,
    };

    // The destination fitting settles it: the bytes are already the right
    // shape, and no other profile's opinion can change that.
    if verdicts
        .iter()
        .any(|(game, fit)| game == target_game && fit.is_identical())
    {
        return (verdicts, native());
    }

    // Otherwise a profile that *does* fit names the game this tag was authored
    // for. Prefer Reach when several do, since it is the profile Campaign
    // Evolved's own schemas descend from.
    let foreign = verdicts
        .iter()
        .filter(|(game, fit)| fit.is_identical() && game != target_game)
        .map(|(game, _)| game.clone())
        .min_by_key(|game| (game != GameId::HaloReach.as_str(), game.clone()));

    match foreign {
        Some(source_game) => (
            verdicts,
            ImportMode::Convert {
                source_game,
                draft: None,
            },
        ),
        // Nothing claims it. This is the dev-era layout drift the gate was
        // originally calibrated for, so keep the override available.
        None => (verdicts, native()),
    }
}
