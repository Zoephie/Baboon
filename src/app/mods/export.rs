//! Export Mod: writing the modified tags as a mod container in the game's mods
//! folder, named to load after what it overrides.

use super::*;
use crate::app::tag_ops::new_tag::new_container_template_bytes;
use crate::app::tag_ops::duplicate::resolve_source_uasset;

impl Baboon {
    /// Bundle every modified Campaign Evolved project tag into one portable `_P`
    /// overlay and write its `.baboon` recovery project beside the triplet.
    /// Open the review of what Export Mod would write.
    ///
    /// Nothing is written and no destination is chosen here. The snapshot is
    /// captured now and kept, so what the user reviews and what is written
    /// cannot drift apart between the two steps.
    pub(in crate::app) fn export_mod(&mut self) {
        self.open_mod_review(false);
    }









    /// Write the reviewed mod. `included` are the identities the user kept.
    pub(in crate::app) fn write_reviewed_mod(
        &mut self,
        snapshot: &CampaignProjectSnapshot,
        included: &HashSet<String>,
        output: PathBuf,
        ctx: &egui::Context,
    ) {
        let exporting = self.model.active;
        if let Err(error) = ensure_export_directory(&output) {
            self.model.status = error;
            return;
        }
        let Some(source) = self.model.source() else {
            self.model.status = "No source loaded".to_owned();
            return;
        };
        let TagSource::IoStoreContainerSet {
            containers,
            shipped,
            packages,
            ..
        } = &source.source
        else {
            self.model.status = "Export Mod is only for Campaign Evolved containers".to_owned();
            return;
        };
        // Tag bytes ride along as the `Arc` the overlay already holds rather
        // than as a copy. The writer only borrows slices from these, and a
        // batch of animation graphs is measured in gigabytes -- copying each
        // one to hand it over is a second full set nobody reads.
        let mut overrides: Vec<(
            std::sync::Arc<blam_tags::iostore::IoStoreArchive>,
            String,
            std::sync::Arc<Vec<u8>>,
        )> = Vec::new();
        // The origin rides along because the two ways a tag gets here want
        // opposite treatment of the wrapper: a copy keeps the bindings of what
        // it was copied from, an authored tag must not inherit its donor's.
        let mut new_pkgs: Vec<(
            Vec<u8>,
            std::sync::Arc<Vec<u8>>,
            String,
            blam_tags::iostore::writer::WrapperOrigin,
        )> = Vec::new();
        let mut skipped = 0usize;
        for overlay in snapshot.overlays.values() {
            if !included.contains(&overlay.identity) {
                continue;
            }
            let Some(entry) = self.model.campaign_entry_for_identity(exporting, &overlay.identity) else {
                skipped += 1;
                continue;
            };
            match &entry.location {
                // A copy Baboon made lives in a container like any other tag,
                // but the game ships no package for it — so an override chunk
                // would patch a package that exists only inside whichever mod
                // it was copied into, and the exported mod would be broken
                // anywhere else. It goes out as a new package, seeded with its
                // own wrapper.
                TagEntryLocation::Container {
                    container,
                    rel_path,
                } if overlay.kind == CampaignProjectTagKind::New => {
                    let Some(package) = overlay.package.clone() else {
                        skipped += 1;
                        continue;
                    };
                    let Ok(resolved) =
                        resolve_source_uasset(containers, packages, *container, rel_path)
                    else {
                        skipped += 1;
                        continue;
                    };
                    let Ok(template) =
                        containers
                            .get(resolved.container)
                            .ok_or(())
                            .and_then(|mounted| {
                                mounted.archive.read(&resolved.rel_path).map_err(|_| ())
                            })
                    else {
                        skipped += 1;
                        continue;
                    };
                    // A copy Baboon made: the template resolved just above is
                    // the very tag it was copied from, so its wrapper is this
                    // tag's wrapper. Stripping it would drop the Blueprint the
                    // copy presents as, and for a model would refuse the export
                    // outright over the region table it still names.
                    let Some(origin) = wrapper_origin_for(&entry.location) else {
                        skipped += 1;
                        continue;
                    };
                    new_pkgs.push((template, overlay.bytes.clone(), package, origin));
                }
                TagEntryLocation::Container {
                    container,
                    rel_path,
                } => {
                    // The base an override is built against is the game's own
                    // pack, not whatever the mount resolved this tag to. With a
                    // mod installed under `Paks`, the latter is that mod — so the
                    // export read its chunk layout out of the very file it was
                    // about to replace.
                    let base = shipped.container_for(rel_path).unwrap_or(*container);
                    let Some(m) = containers.get(base) else {
                        skipped += 1;
                        continue;
                    };
                    overrides.push((m.archive.clone(), rel_path.clone(), overlay.bytes.clone()));
                }
                TagEntryLocation::NewContainer {
                    template,
                    package,
                    group_tag,
                } => {
                    // A recorded donor is re-resolved against the kit being
                    // exported, not the active one: they differ, and the wrong
                    // kit's containers would drop the tag out of the export.
                    let Ok(template) = new_container_template_bytes(
                        template,
                        containers,
                        package,
                        overlay.bytes.len() as u64,
                        || self.model.find_container_template_in(exporting, *group_tag),
                    ) else {
                        skipped += 1;
                        continue;
                    };
                    // Authored from a recorded donor, which is some other tag:
                    // its bindings say nothing true about this one.
                    let Some(origin) = wrapper_origin_for(&entry.location) else {
                        skipped += 1;
                        continue;
                    };
                    new_pkgs.push((template, overlay.bytes.clone(), package.clone(), origin));
                }
                _ => skipped += 1,
            }
        }
        let count = overrides.len() + new_pkgs.len();
        if count == 0 {
            self.model.status = "Nothing selected to export".to_owned();
            return;
        }
        // Taken before anything is built, so a second export to the same files
        // is refused rather than interleaved, and so the Unreal package
        // workspace — which holds its own mapping of every `.ucas` and an open
        // handle on every `.pak` under `Paks` — lets go before the swap.
        let mut lease =
            match self.acquire_container_write_lease(&output, ContainerWriteMode::Replace) {
                Ok(lease) => lease,
                Err(failure) => {
                    self.model.status = failure.to_string();
                    return;
                }
            };
        // Built at a staging path first, with every container still mapped.
        // The writer reads each override's base container *while* it writes,
        // and for a tag only a mod carries that base is the container being
        // replaced — so "unmap, then write" cannot work here. Write, unmap,
        // then swap.
        let staging = staging_utoc_for(&output);
        if let Some(directory) = staging.parent()
            && let Err(error) = fs::create_dir_all(directory)
        {
            self.model.status =
                ContainerWriteFailure::at(LeasePhase::Write, directory, error).to_string();
            self.release_container_write_lease(lease, ContainerWriteOutcome::Unchanged, ctx);
            return;
        }
        let override_refs: Vec<(&blam_tags::iostore::IoStoreArchive, &str, &[u8])> = overrides
            .iter()
            .map(|(a, p, b)| (a.as_ref(), p.as_str(), b.as_slice()))
            .collect();
        let new_refs: Vec<blam_tags::iostore::writer::NewPackage> = new_pkgs
            .iter()
            .map(
                |(template, bytes, package, origin)| blam_tags::iostore::writer::NewPackage {
                    template_uasset: template.as_slice(),
                    tag_bytes: bytes.as_slice(),
                    new_package_path: package.as_str(),
                    redirect_from: None,
                    // Only ever set deliberately, and nothing here chooses one:
                    // an authored tag gets none, and a copy keeps whatever its
                    // original had rather than being handed a new one.
                    asset_reference: None,
                    origin: *origin,
                },
            )
            .collect();
        let built =
            blam_tags::iostore::writer::write_mod_container_ex(&override_refs, &new_refs, &staging);
        // Every borrow of a mounted archive has to be gone before the unmap:
        // one surviving clone is the difference between replacing the mod and
        // being told the workspace is reading it.
        drop(override_refs);
        drop(new_refs);
        drop(overrides);
        if let Err(error) = built {
            // Nothing of the original was touched — the whole point of building
            // at a staging path — so say so rather than leaving the user
            // wondering what state their installed mod is in.
            discard_staging(&staging);
            self.release_container_write_lease(lease, ContainerWriteOutcome::Unchanged, ctx);
            self.model.status = format!(
                "Export Mod failed: {}. Nothing was replaced",
                ContainerWriteFailure::at(LeasePhase::Write, &staging, error)
            );
            return;
        }
        if let Err(failure) = self.unmap_leased_containers(&mut lease) {
            discard_staging(&staging);
            self.release_container_write_lease(lease, ContainerWriteOutcome::Unchanged, ctx);
            self.model.status = failure.to_string();
            return;
        }
        // Each existing file is moved aside before any of the new ones land, so
        // a failure part-way through puts back what was there rather than
        // leaving a container built from two mods.
        // `swap_container_triplet` puts back whatever it moved aside, and says
        // so in the phase it reports: `Swap` when the original was restored,
        // `Rollback` when restoring it is what failed.
        let written = swap_container_triplet(&staging, &output);
        discard_staging(&staging);
        let replaced_a_mount = lease.unmapped_any();
        let report = self.release_container_write_lease(
            lease,
            if written.is_ok() {
                ContainerWriteOutcome::Committed
            } else {
                ContainerWriteOutcome::Unchanged
            },
            ctx,
        );
        let reopen_failures = report.reopen_failures;
        match written {
            Ok(()) => {
                let stem = output
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("mod")
                    .to_owned();
                let sidecar = output.with_extension("baboon");
                // A sidecar written next to an exported mod may be replacing an
                // older one, and nothing here knows what is in it.
                if let Err(error) =
                    save_campaign_project(&sidecar, snapshot, None, ProjectScope::ModSidecar)
                {
                    self.model.status = format!(
                        "Exported {count} tag(s), but the .baboon sidecar failed: {}",
                        ContainerWriteFailure::at(LeasePhase::Commit, &sidecar, error)
                    );
                }
                // The sidecar travels with the mod; the workspace keeps its own
                // project, checkpointed here so it carries what the export did.
                let _ = self.checkpoint_campaign_project(exporting, 0.0);
                let directory = output.parent().map(Path::to_path_buf).unwrap_or_default();
                // Anywhere inside the game's own Paks tree, not just its root:
                // a mod written to `Paks/~mods/` is already where the game will
                // find it, so there is nothing to copy there either.
                let in_place = self
                    .model.source()
                    .map(|source| directory.starts_with(source.source.root_path()))
                    .unwrap_or(false);
                self.model.status = if !reopen_failures.is_empty() {
                    // The mod was written; what failed is picking it back up.
                    format!(
                        "Exported {count} tag(s) as {stem}, but {} — reload the source",
                        ContainerWriteFailure::at(
                            LeasePhase::Remount,
                            &output,
                            reopen_failures.join("; ")
                        )
                    )
                } else if !replaced_a_mount {
                    format!("Exported {count} tag(s) as {stem}")
                } else {
                    // The container it replaced is mounted, so the browser is now
                    // showing the mod it just wrote. Anything that mod used to
                    // carry and no longer does still has an entry pointing at it.
                    format!(
                        "Exported {count} tag(s) as {stem}, replacing the mounted copy — reload \
                         the source if its tag list changed"
                    )
                };
                // A mod replacing one that was already mounted came back
                // through the lease's reopen. One written under a name nothing
                // was mounted under is not in the set at all, and until now the
                // only way to see it was a reload — which rebuilds the
                // workspace and costs every open tab and the stash with it.
                if in_place && !replaced_a_mount {
                    let folder_seeds = self.model.kits[exporting].folder_seeds();
                    let mounted = self.model.kits[exporting].source.as_mut().map(|source| {
                        crate::core::source::mount_additional_container(source, &output, &folder_seeds)
                    });
                    match mounted {
                        Some(Ok(count)) if count > 0 => {
                            self.model.kits[exporting].generation =
                                self.model.kits[exporting].generation.wrapping_add(1);
                            self.model.kits[exporting].field_index.invalidate();
                            self.model.status
                                .push_str(&format!(" — mounted, {count} tag(s) now served by it"));
                        }
                        Some(Err(error)) => self.model.status.push_str(&format!(
                            " — but {}; reload the source to see it",
                            ContainerWriteFailure::at(LeasePhase::Remount, &output, error)
                        )),
                        _ => {}
                    }
                }
                // Written straight into the game's own folder: there is nothing
                // to copy, so the instructions would only be noise.
                if !in_place {
                    self.dialogs.open(ExportedMod {
                        stem,
                        directory,
                        count,
                        skipped,
                    });
                }
            }
            Err(error) => self.model.status = format!("Export Mod failed: {error}"),
        }
    }
}

/// Move a mod's output into a folder of its own under `~mods`.
///
/// A mod is a triplet plus a sidecar, and a `Paks` directory that collects them
/// loose becomes impossible to tell apart from the game's own containers.
/// Grouping each mod under `~mods/<name>/` keeps them separable and replaceable,
/// and `~mods` is where the loader already expects mods to be, so a mod written
/// into the game's own `Paks` still mounts from where it lands.
///
/// The folder is named after the mod without the `_P` priority suffix, which is
/// a property of the container rather than part of what the user called it. A
/// path already inside a `~mods` folder is left where it is.
pub(in crate::app) fn mod_output_path(output: PathBuf) -> PathBuf {
    let Some(file_name) = output.file_name().map(|name| name.to_os_string()) else {
        return output;
    };
    let Some(parent) = output.parent() else {
        return output;
    };
    if parent
        .components()
        .any(|part| part.as_os_str().eq_ignore_ascii_case(MODS_DIR))
    {
        return output;
    }
    let stem = output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("mod");
    let folder = stem
        .strip_suffix("_P")
        .or_else(|| stem.strip_suffix("_p"))
        .unwrap_or(stem);
    parent.join(MODS_DIR).join(folder).join(file_name)
}

/// The folder Export Mod offers by default: the game's own `~mods`, inside the
/// `Paks` directory the source was mounted from.
///
/// `~mods` is where the engine's loader already looks — `FPakPlatformFile`
/// walks the pak folder recursively — so a mod written there is installed
/// where it lands, with nothing to copy afterwards.
pub(in crate::app) fn default_mod_export_folder(paks_root: &Path) -> PathBuf {
    paks_root.join(MODS_DIR)
}

/// Create the folder a mod is about to be written into.
pub(in crate::app) fn ensure_mod_output_dir(output: &Path) -> Result<(), String> {
    let Some(directory) = output.parent() else {
        return Ok(());
    };
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))
}

/// Force the `_P` suffix onto a mod's file name.
///
/// It is what gives an override container priority over the game's own
/// containers; without it the mod mounts alongside them and the base tag wins,
/// so the mod builds correctly and does nothing. That is not a naming
/// preference to be respected -- a mod without it is simply broken -- and it is
/// exactly what a user renaming the default to something meaningful drops.
///
/// Confirmed against the game's own mount path: it compares the last six
/// characters to `_P.pak` case-insensitively and adds `100 × version` to the
/// pak order, where the version defaults to 1 and only rises if the name
/// carries `_<digits>_` before the suffix. A base pak scores 4, so any `_P`
/// mod at 104 outranks it, and `_p` is accepted just as readily.
pub(in crate::app) fn ensure_priority_suffix(path: PathBuf) -> PathBuf {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return path;
    };
    if stem.len() >= 2 && stem[stem.len() - 2..].eq_ignore_ascii_case("_p") {
        return path;
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("utoc");
    path.with_file_name(format!("{stem}_P.{extension}"))
}

#[cfg(test)]
mod mod_output_tests {
    use super::*;
    use crate::app::mods::review::classify_overlay;
    use crate::app::mods::export::default_mod_export_folder;
    use crate::app::mods::in_place::ContainerSaveRoute;
    use crate::app::mods::in_place::container_save_route;
    use crate::app::mods::export::mod_output_path;

    #[test]
    fn a_mod_is_written_into_a_folder_of_its_own_under_mods() {
        // The name the user chose becomes the folder; `_P` is a property of the
        // container, not part of what they called it.
        assert_eq!(
            mod_output_path(PathBuf::from("D:/Game/Paks/coolmod_P.utoc")),
            PathBuf::from("D:/Game/Paks/~mods/coolmod/coolmod_P.utoc")
        );
        assert_eq!(
            mod_output_path(PathBuf::from("D:/Game/Paks/coolmod_p.utoc")),
            PathBuf::from("D:/Game/Paks/~mods/coolmod/coolmod_p.utoc")
        );
        // A name with no suffix keeps its whole stem as the folder.
        assert_eq!(
            mod_output_path(PathBuf::from("D:/Game/Paks/plain.utoc")),
            PathBuf::from("D:/Game/Paks/~mods/plain/plain.utoc")
        );
    }

    #[test]
    fn a_path_already_under_mods_is_left_alone() {
        // Browsing into the mods folder, or into a mod's own folder, must not
        // bury the output another level down each time.
        for path in [
            "D:/Game/Paks/~mods/coolmod_P.utoc",
            "D:/Game/Paks/~mods/coolmod/coolmod_P.utoc",
            "D:/Game/Paks/~MODS/coolmod/coolmod_P.utoc",
        ] {
            assert_eq!(mod_output_path(PathBuf::from(path)), PathBuf::from(path));
        }
    }

    #[test]
    fn saving_a_container_tag_never_touches_the_game_without_expert_mode() {
        // The confirmation preference is irrelevant outside expert mode: there
        // is nothing destructive left for it to guard. A user who once ticked
        // "don't ask again" must not silently get the in-place write back.
        for confirm in [true, false] {
            assert_eq!(
                container_save_route(false, confirm),
                ContainerSaveRoute::ExportReview,
                "confirm = {confirm}"
            );
        }
    }

    #[test]
    fn expert_mode_keeps_both_in_place_routes() {
        assert_eq!(
            container_save_route(true, true),
            ContainerSaveRoute::ConfirmOverwriteInPlace
        );
        assert_eq!(
            container_save_route(true, false),
            ContainerSaveRoute::OverwriteInPlace
        );
    }

    #[test]
    fn export_mod_defaults_into_the_games_own_mods_folder() {
        assert_eq!(
            default_mod_export_folder(Path::new("D:/Game/Meteorite/Content/Paks")),
            PathBuf::from("D:/Game/Meteorite/Content/Paks/~mods")
        );
    }

    #[test]
    fn the_default_export_creates_mods_when_it_is_missing() {
        // The one behaviour that must survive the destination change: a first
        // export into a `Paks` folder that has never had a mod in it makes
        // `~mods` rather than failing.
        let paks = std::env::temp_dir().join(format!(
            "baboon-export-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&paks).expect("a Paks folder to export into");
        let mods = default_mod_export_folder(&paks);
        assert!(!mods.exists(), "the fixture starts without a ~mods folder");

        ensure_export_directory(&mods.join("mymod_P.utoc")).expect("created");

        assert!(mods.is_dir(), "~mods was created for the export");
        // And the files land directly in it — no folder named after the mod.
        assert_eq!(mods.join("mymod_P.utoc").parent(), Some(mods.as_path()));
        let _ = fs::remove_dir_all(&paks);
    }

    #[test]
    fn a_copy_baboon_authored_exports_as_a_new_package() {
        // A duplicate mounts as an ordinary container tag, so without the
        // ledger's word for it the export would build a field override against
        // a package that exists only inside the mod it was copied into.
        let entry = TagEntry {
            key: "ublock:mymod_P:Tags/objects/copy-biped.ubulk".to_owned(),
            display_path: "objects/copy.biped".to_owned(),
            group_tag: parse_group_tag("bipd").unwrap(),
            group_name: Some("biped".to_owned()),
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: "Tags/objects/copy-biped.ubulk".to_owned(),
            },
        };
        let package = "/Game/Tags/objects/copy-biped".to_owned();

        let (_, _, authored_kind, authored_package) =
            crate::app::mods::campaign_entry_project_parts_with(&entry, Some(package.clone()))
                .expect("a container entry has project parts");
        assert_eq!(authored_kind, CampaignProjectTagKind::New);
        assert_eq!(authored_package.as_deref(), Some(package.as_str()));
        // `New` never reaches the "identical to the game's copy" branch: there
        // is no shipped copy for it to be identical to.
        assert_eq!(
            classify_overlay(true, authored_kind, true),
            ModExportChange::New
        );

        // The same entry with nothing in the ledger is still what it looks
        // like: an edit to a tag the game ships.
        let (_, _, shipped_kind, shipped_package) =
            crate::app::mods::campaign_entry_project_parts_with(&entry, None).expect("project parts");
        assert_eq!(shipped_kind, CampaignProjectTagKind::Existing);
        assert_eq!(shipped_package, None);
        assert_eq!(
            classify_overlay(true, shipped_kind, false),
            ModExportChange::Modified
        );
    }
}

#[cfg(test)]
mod mod_override_tests {
    //! Separating the game's own packs from mods installed into the same tree.
    //!
    //! Baboon mounts the pak folder recursively, exactly as the game does, so a mod
    //! in `Paks/~mods` -- or loose in `Paks` -- is mounted and wins every collision.
    //! That is correct for reading, and it silently redefined "as the game ships it"
    //! to mean "as this install currently loads it": every comparison against a
    //! modded tag came back empty, including a mod compared against an earlier export
    //! of itself.

    use super::*;
    use crate::app::mods::review::wrapper_origin_for;

    static PAKS: std::sync::LazyLock<&'static str> =
        std::sync::LazyLock::new(|| crate::core::test_kits::leak(crate::core::test_kits::ce_paks()));

    /// The install these fixtures run against, or `None` when there isn't one.
    ///
    /// `BABOON_CE_PAKS` overrides the default so the fixtures are runnable wherever
    /// the game happens to be installed. Without it a machine with a perfectly good
    /// install still skips every test here, and a skip reads exactly like a pass.
    fn paks() -> Option<std::path::PathBuf> {
        let path = std::env::var_os("BABOON_CE_PAKS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(*PAKS));
        if !path.exists() {
            eprintln!(
                "skipping: Campaign Evolved not present at {}",
                path.display()
            );
            return None;
        }
        Some(path)
    }

    fn mount() -> Option<crate::core::source::LoadedSourceData> {
        let paks = paks()?;
        let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = crate::core::format::TagNameIndex::load_from_definitions(&defs);
        Some(
            crate::core::source::load_iostore_container_set(paks, &names, &defs)
                .expect("mount container set"),
        )
    }

    /// The game's own packs must not be mistaken for mods, or every tag would be
    /// compared against nothing and the whole shipped layer would be empty.
    #[test]
    fn the_games_own_packs_are_not_mods() {
        let Some(loaded) = mount() else { return };
        let TagSource::IoStoreContainerSet {
            containers,
            shipped,
            ..
        } = &loaded.source
        else {
            panic!("not a container set");
        };
        let pakchunks = containers
            .iter()
            .filter(|container| container.chunk_label.starts_with("pakchunk"))
            .collect::<Vec<_>>();
        assert!(!pakchunks.is_empty(), "the install has pakchunk containers");
        for container in pakchunks {
            assert!(
                !container.is_mod,
                "{} is one of the game's own packs",
                container.chunk_label
            );
        }
        assert!(
            !shipped.is_empty(),
            "the shipped layer holds the game's own payloads"
        );
    }

    /// Every tag the browser serves from a mod must still resolve to the game's own
    /// copy, or "what does this change about the game?" answers itself with
    /// "nothing" -- which is exactly what a user reported after installing an
    /// earlier export of their own mod.
    #[test]
    fn a_modded_tag_still_resolves_to_the_shipped_copy() {
        let Some(loaded) = mount() else { return };
        let TagSource::IoStoreContainerSet {
            containers,
            shipped,
            ..
        } = &loaded.source
        else {
            panic!("not a container set");
        };
        let mods = containers
            .iter()
            .enumerate()
            .filter(|(_, container)| container.is_mod)
            .map(|(index, container)| (index, container.chunk_label.clone()))
            .collect::<Vec<_>>();
        if mods.is_empty() {
            eprintln!("skipping: no mod is installed in this Paks folder");
            return;
        }
        eprintln!(
            "{} mod container(s): {}",
            mods.len(),
            mods.iter()
                .map(|(_, label)| label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mod_indices = mods.iter().map(|(index, _)| *index).collect::<Vec<_>>();
        let mut checked = 0usize;
        for entry in &loaded.entries {
            let TagEntryLocation::Container {
                container,
                rel_path,
            } = &entry.location
            else {
                continue;
            };
            if !mod_indices.contains(container) {
                continue;
            }
            checked += 1;
            // The mod is what the mount resolved the tag to -- that part is correct,
            // it is what the game loads.
            let mounted = crate::core::source::read_entry(&loaded.source, entry)
                .unwrap_or_else(|error| panic!("read {} as mounted: {error}", entry.display_path));
            // ...and the game's own copy has to remain reachable beside it.
            let base = crate::core::source::read_shipped_entry(&loaded.source, entry)
                .unwrap_or_else(|error| panic!("read {} as shipped: {error}", entry.display_path));
            let Some(base) = base else {
                eprintln!(
                    "{} exists only in the mod, nothing shipped to compare",
                    entry.display_path
                );
                continue;
            };
            let shipped_container = shipped
                .container_for(rel_path)
                .expect("a shipped copy was just read");
            assert!(
                !mod_indices.contains(&shipped_container),
                "{} resolved its shipped copy to a mod",
                entry.display_path
            );
            assert_ne!(
                shipped_container, *container,
                "{} must read its baseline from a different container than the mod \
             serving it, or the comparison is the mod against itself",
                entry.display_path
            );
            // Whether the mod actually changed anything is up to the mod; what
            // matters is that both sides are now readable and distinct.
            let (base_bytes, mounted_bytes) = (
                base.write_to_bytes().expect("serialize shipped"),
                mounted.write_to_bytes().expect("serialize mounted"),
            );
            eprintln!(
                "{}: shipped {} bytes from container {shipped_container}, mounted {} bytes from \
             container {container} ({})",
                entry.display_path,
                base_bytes.len(),
                mounted_bytes.len(),
                if base_bytes == mounted_bytes {
                    "identical"
                } else {
                    "differs"
                }
            );
        }
        assert!(checked > 0, "at least one tag is served from a mod");
    }

    /// Bulk extraction writes the *shipped* payload to a mirror of the browser
    /// tree.
    ///
    /// Run over a slice of the install rather than all of it: the layout, the
    /// choice of payload, and the skip accounting are what can be wrong, and none of
    /// them need forty thousand tags to show it.
    #[test]
    fn extracting_container_tags_mirrors_the_tree_with_shipped_bytes() {
        let Some(loaded) = mount() else { return };
        let sample = loaded
            .entries
            .iter()
            .filter(|entry| matches!(entry.location, TagEntryLocation::Container { .. }))
            // Deliberately small. What can be wrong here is the layout, the choice
            // of payload, and the skip accounting, and none of them need volume to
            // show it — while a fixture that writes hundreds of files makes the
            // whole suite's disk busy, and the index tests share one SQLite file.
            .take(24)
            .cloned()
            .collect::<Vec<_>>();
        assert!(!sample.is_empty(), "the mount enumerated container tags");

        let output = std::env::temp_dir().join(format!("baboon-dump-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&output);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let report = crate::app::export::container_dump::dump_shipped_container_tags(
            &loaded.source,
            &sample,
            &output,
            &cancel,
            &|_, _| {},
        )
        .expect("extract sample");

        assert_eq!(
            report.written + report.skipped + report.failed,
            sample.len(),
            "every sampled tag is accounted for"
        );
        assert!(report.written > 0, "the sample produced files");
        assert!(!report.cancelled);
        // Failures are named rather than swallowed. Not asserted to be zero: a run
        // over the whole install turns up a handful of payloads the Oodle decoder
        // rejects, and the contract is that those are reported and the rest are
        // still written — not that the game decompresses perfectly.
        assert_eq!(
            report.failures.len(),
            report.failed.min(20),
            "every failure up to the report cap is named"
        );

        for entry in &sample {
            let path = output.join(&entry.display_path);
            let shipped = match crate::core::source::read_shipped_entry_bytes(&loaded.source, entry) {
                Ok(shipped) => shipped,
                // Unreadable here means unreadable for the extraction too, and it is
                // already counted in `failed`.
                Err(_) => continue,
            };
            let Some(shipped) = shipped else {
                // Mod-only: counted as skipped, and nothing written for it.
                assert!(!path.exists(), "{} is mod-only", entry.display_path);
                continue;
            };
            // `display_path` is the browser's own path, so joining it onto the
            // output root is the whole layout claim.
            let written =
                std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            assert_eq!(
                written, shipped,
                "{} was extracted verbatim from the shipped pack",
                entry.display_path
            );
        }
        let _ = std::fs::remove_dir_all(&output);
    }

    /// Cancelling stops the run and still reports what it managed, rather than
    /// failing outright — a partial extraction has files on disk worth naming.
    #[test]
    fn a_cancelled_extraction_reports_what_it_wrote() {
        let Some(loaded) = mount() else { return };
        let sample = loaded
            .entries
            .iter()
            .filter(|entry| matches!(entry.location, TagEntryLocation::Container { .. }))
            .take(24)
            .cloned()
            .collect::<Vec<_>>();
        assert!(!sample.is_empty());
        let output = std::env::temp_dir().join(format!("baboon-dump-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&output);
        // Already cancelled: every worker breaks on its first check, so nothing is
        // read and the run is still a reported outcome rather than an error.
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let report = crate::app::export::container_dump::dump_shipped_container_tags(
            &loaded.source,
            &sample,
            &output,
            &cancel,
            &|_, _| {},
        )
        .expect("a cancelled run is not a failure");
        assert!(report.cancelled);
        assert_eq!(report.written, 0);
        assert_eq!(report.failed, 0);
        let _ = std::fs::remove_dir_all(&output);
    }

    /// Exporting a mod over an installed copy of itself: the whole sequence the
    /// export performs, on a copy of a real mod container so nothing in the install
    /// is touched.
    ///
    /// The mount holds the only reference to each archive — which is what lets the
    /// mapping be released at all — and the container has to come back readable
    /// afterwards, since the browser reads through it.
    #[test]
    fn a_mounted_container_can_be_released_replaced_and_remounted() {
        let Some(paks) = paks() else { return };
        // A real mod container, copied out of the install. Any non-pakchunk container
        // will do; without one there is nothing index-less to exercise.
        let Some(donor) = std::fs::read_dir(&paks)
            .expect("read Paks")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| {
                path.extension().is_some_and(|ext| ext == "utoc")
                    && !path
                        .file_stem()
                        .is_some_and(|stem| stem.to_string_lossy().starts_with("pakchunk"))
                    && path.file_name().is_some_and(|name| name != "global.utoc")
            })
        else {
            eprintln!("skipping: no mod container in this Paks folder to copy");
            return;
        };
        let scratch = std::env::temp_dir().join(format!("baboon-remount-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        let target = scratch.join("copied_mod_P.utoc");
        for extension in ["utoc", "ucas", "pak"] {
            let from = donor.with_extension(extension);
            if from.exists() {
                std::fs::copy(&from, target.with_extension(extension)).expect("copy container");
            }
        }

        let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = crate::core::format::TagNameIndex::load_from_definitions(&defs);
        // Mounted against the real Paks, as the app does: an override container ships
        // no directory index, so only the containers it overrides can name its chunks.
        let mut loaded =
            crate::core::source::load_iostore_container(target.clone(), Some(paks.clone()), &names, &defs)
                .expect("mount the copied mod");

        assert_eq!(
            crate::core::source::mounted_containers_at(&loaded.source, &target),
            vec![0],
            "the export target is recognised as a mounted container"
        );
        let entry = loaded
            .entries
            .first()
            .cloned()
            .expect("the mod carries a tag");
        crate::core::source::read_entry(&loaded.source, &entry).expect("read while mapped");

        let root = match &loaded.source {
            TagSource::IoStoreContainerSet { root, .. } => root.clone(),
            _ => panic!("not a container set"),
        };
        {
            let TagSource::IoStoreContainerSet { containers, .. } = &mut loaded.source else {
                panic!("not a container set");
            };
            assert_eq!(
                std::sync::Arc::strong_count(&containers[0].archive),
                1,
                "the mount holds the only reference, so the mapping can be released"
            );
            std::sync::Arc::get_mut(&mut containers[0].archive)
                .expect("uniquely held archive")
                .release_partition();
            assert!(!containers[0].archive.is_partition_mapped());
        }
        assert!(
            crate::core::source::read_entry(&loaded.source, &entry).is_err(),
            "a read through a released partition is refused rather than served stale"
        );

        // What the release is for: the container is rewritten where it stands.
        let mut writer = blam_tags::iostore::writer::OverrideContainerWriter::new("../../../");
        let mut id = [0u8; 12];
        id[..8].copy_from_slice(&0x0bad_f00d_dead_beefu64.to_le_bytes());
        id[11] = blam_tags::iostore::CHUNK_TYPE_BULK_DATA;
        writer.add_chunk(blam_tags::iostore::FIoChunkId(id), vec![7u8; 2048]);
        writer
            .write(&target)
            .expect("replace the mounted container");

        let containers_snapshot = match &loaded.source {
            TagSource::IoStoreContainerSet { containers, .. } => containers.clone(),
            _ => panic!("not a container set"),
        };
        let reopened = crate::core::source::reopen_container_archive(&root, &containers_snapshot, 0)
            .expect("reopen the replaced container");
        assert!(
            reopened.is_partition_mapped(),
            "the remounted container is readable again"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Every tag the install mounts must be able to name the `.uasset` wrapper it
    /// belongs to.
    ///
    /// The Marine biped could not: swapping `.ubulk` for `.uasset` on the indexed
    /// payload path produced `objects/characters/Marine/marine-biped.uasset`, and
    /// the container spells that folder `marine`. A directory index is matched byte
    /// for byte, so the copy failed with "path not found in container" against a
    /// path nothing had ever held. This is that failure, asked of every tag at once
    /// rather than of the one that happened to be reported.
    #[test]
    fn every_container_tag_can_name_its_wrapper() {
        let Some(loaded) = mount() else { return };
        let TagSource::IoStoreContainerSet {
            containers,
            packages,
            ..
        } = &loaded.source
        else {
            panic!("not a container set");
        };
        let mut unresolved = Vec::new();
        let mut assembled_would_have_failed = 0usize;
        for entry in &loaded.entries {
            let TagEntryLocation::Container {
                container,
                rel_path,
            } = &entry.location
            else {
                continue;
            };
            match resolve_source_uasset(containers, packages, *container, rel_path) {
                Ok(resolved) => {
                    if resolved.how != "same container" {
                        assembled_would_have_failed += 1;
                    }
                }
                Err(_) if unresolved.len() < 20 => {
                    unresolved.push(format!("{} ({rel_path})", entry.display_path))
                }
                Err(_) => {}
            }
        }
        eprintln!(
            "{assembled_would_have_failed} of {} tags need more than an extension swap to find their \
         wrapper",
            loaded.entries.len()
        );
        assert!(
            unresolved.is_empty(),
            "{} tag(s) have no resolvable wrapper, e.g. {}",
            unresolved.len(),
            unresolved.join(", ")
        );
    }

    /// A mod exported under a name nothing was mounted under is folded into the
    /// mount that is already there, rather than needing a reload — which rebuilds
    /// the workspace and costs every open tab and the Mod Stash with it.
    #[test]
    fn a_freshly_written_mod_can_be_mounted_without_reloading() {
        let Some(paks) = paks() else { return };
        let Some(mut loaded) = mount() else { return };
        let before = loaded.entries.len();
        let containers_before = match &loaded.source {
            TagSource::IoStoreContainerSet { containers, .. } => containers.len(),
            _ => panic!("not a container set"),
        };
        let Some(donor) = std::fs::read_dir(&paks)
            .expect("read Paks")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| {
                path.extension().is_some_and(|ext| ext == "utoc")
                    && !path
                        .file_stem()
                        .is_some_and(|stem| stem.to_string_lossy().starts_with("pakchunk"))
                    && path.file_name().is_some_and(|name| name != "global.utoc")
            })
        else {
            eprintln!("skipping: no mod container in this Paks folder to copy");
            return;
        };
        // Inside the real Paks tree, because an index-less container's chunks are
        // named from the containers it overrides and those are found from the root.
        let scratch = paks.join(format!(".baboon-mount-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        let target = scratch.join("freshly_written_P.utoc");
        for extension in ["utoc", "ucas", "pak"] {
            let from = donor.with_extension(extension);
            if from.exists() {
                std::fs::copy(&from, target.with_extension(extension)).expect("copy container");
            }
        }

        // What every open tab, parsed document and undo stack is filed under.
        let keys_before: Vec<String> = loaded
            .entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect();

        let contributed = crate::core::source::mount_additional_container(&mut loaded, &target, &[])
            .expect("mount the freshly written mod");

        // Mounting a mod over a tag changes where it is read from, never what it
        // is. Anything else orphans the tabs of the tags that were just exported.
        for key in &keys_before {
            assert!(
                loaded.entries.iter().any(|entry| entry.key == *key),
                "{key} lost its identity when the mod was mounted over it"
            );
        }

        let TagSource::IoStoreContainerSet { containers, .. } = &loaded.source else {
            panic!("not a container set");
        };
        assert_eq!(
            containers.len(),
            containers_before + 1,
            "the new container joined the set"
        );
        let mounted = containers.last().expect("the new container");
        assert!(
            mounted.is_mod,
            "a container outside the game's own packs is a mod"
        );
        assert_eq!(mounted.utoc_path, target);
        if contributed > 0 {
            // Its tags are reachable through the mount, in sorted position, and
            // readable — which is the whole point of not needing the reload.
            assert!(loaded.entries.len() >= before);
            let entry = loaded
                .entries
                .iter()
                .find(|entry| matches!(
                    &entry.location,
                    TagEntryLocation::Container { container, .. } if *container == containers.len() - 1
                ))
                .cloned()
                .expect("the new container's tags are in the entry list");
            crate::core::source::read_entry(&loaded.source, &entry).expect("read through the new mount");
            assert!(
                loaded
                    .entries
                    .windows(2)
                    .all(|pair| crate::core::source::natural_key(&pair[0].display_path)
                        <= crate::core::source::natural_key(&pair[1].display_path)),
                "entries stay in the order the browser draws"
            );
        }
        // Mounting the same file twice is a no-op rather than a second container.
        assert_eq!(
            crate::core::source::mount_additional_container(&mut loaded, &target, &[]).expect("idempotent"),
            0
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The export target has to be recognised as a mounted container before the
    /// writer runs, since that is what decides whether a mapping must be released.
    #[test]
    fn an_export_over_a_mounted_container_is_detected() {
        let Some(loaded) = mount() else { return };
        let TagSource::IoStoreContainerSet { containers, .. } = &loaded.source else {
            panic!("not a container set");
        };
        let target = containers
            .first()
            .expect("the install mounts containers")
            .utoc_path
            .clone();
        assert_eq!(
            crate::core::source::mounted_containers_at(&loaded.source, &target),
            vec![0],
            "exporting over a mounted container is detected"
        );
        let free = target.with_file_name("a-name-nothing-has-taken_P.utoc");
        assert!(
            crate::core::source::mounted_containers_at(&loaded.source, &free).is_empty(),
            "an unused name is free to write"
        );
    }

    /// A tag stashed as modified whose bytes turn out to be exactly what the game
    /// ships is not an unexported change.
    ///
    /// From a user report: the export review listed seven modified tags, one of
    /// which had byte-identical content to the shipped copy and produced an empty
    /// diff. Listing it asserts the user made a change they did not make, and it
    /// would have been written into the mod as a redundant override. Reaching that
    /// state needs no deliberate undo — a value nudged and put back, or an edit that
    /// re-encodes identically, leaves the document flagged with nothing to show.
    #[test]
    fn an_overlay_identical_to_the_shipped_tag_is_not_a_change() {
        use crate::app::mods::review::classify_overlay;

        assert_eq!(
            classify_overlay(true, CampaignProjectTagKind::Existing, true),
            ModExportChange::Unchanged
        );
        assert_eq!(
            classify_overlay(true, CampaignProjectTagKind::Existing, false),
            ModExportChange::Modified
        );
        // A tag this workspace created has no shipped counterpart, so it can never
        // be "identical to the game's copy" -- there is no copy.
        assert_eq!(
            classify_overlay(true, CampaignProjectTagKind::New, true),
            ModExportChange::New
        );
        // Unresolvable wins over everything: it cannot be written at all.
        assert_eq!(
            classify_overlay(false, CampaignProjectTagKind::Existing, true),
            ModExportChange::Unresolved
        );
    }

    /// A copy and an authored tag are handed different wrappers, and the location
    /// is what decides.
    ///
    /// Both reach the exporter as `CampaignProjectTagKind::New` and are written
    /// whole, so the kind cannot tell them apart — the location can. A copy Baboon
    /// made lives in a container, and the `.uasset` resolved for it is the tag it
    /// was copied from; a tag from New Tag lives in a `NewContainer` with a donor
    /// recorded against it.
    ///
    /// This is gated because getting it backwards fails quietly in both directions.
    /// A copy stripped of its wrapper exports fine and presents as nothing in game —
    /// which is what shipped, and what users hit as an export that could never
    /// succeed once the tag was a `model`, since a model's wrapper names a region
    /// string table that stripping orphans.
    #[test]
    fn a_copy_keeps_its_wrapper_and_an_authored_tag_does_not() {
        use blam_tags::iostore::writer::WrapperOrigin;

        // A copy: it sits in a container like any other tag.
        assert_eq!(
            wrapper_origin_for(&TagEntryLocation::Container {
                container: 0,
                rel_path: "Meteorite/Content/Tags/objects/x/y-model.ubulk".to_owned(),
            }),
            Some(WrapperOrigin::Copy),
            "a copy must keep the bindings of the tag it was copied from"
        );

        // Authored through New Tag, both ways a wrapper is obtained: donated from
        // an unrelated same-group tag, or derived from the group's own rules.
        // Neither carries bindings that belong to the tag being created.
        for template in [
            NewContainerTemplate::Donor {
                container: 0,
                rel_path: "Meteorite/Content/Tags/objects/other/donor-model.ubulk".to_owned(),
            },
            NewContainerTemplate::Derived {
                group: "model".to_owned(),
            },
        ] {
            assert_eq!(
                wrapper_origin_for(&TagEntryLocation::NewContainer {
                    template,
                    package: "/Game/Tags/objects/x/y-model".to_owned(),
                    group_tag: u32::from_be_bytes(*b"mode"),
                }),
                Some(WrapperOrigin::Template),
                "an authored tag must not inherit its donor's bindings"
            );
        }
    }
}

#[cfg(test)]
mod priority_suffix_tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn a_mod_always_gets_the_priority_suffix() {
        assert_eq!(
            ensure_priority_suffix(PathBuf::from("/mods/h2a_magnum.utoc")),
            PathBuf::from("/mods/h2a_magnum_P.utoc")
        );
        // Already correct, including the platform suffix the game itself uses.
        assert_eq!(
            ensure_priority_suffix(PathBuf::from("/mods/mymod-WinGDK_P.utoc")),
            PathBuf::from("/mods/mymod-WinGDK_P.utoc")
        );
        // The loader folds case before comparing, so a lowercase suffix
        // already has priority and must not collect a second one.
        assert_eq!(
            ensure_priority_suffix(PathBuf::from("/mods/thing_p.utoc")),
            PathBuf::from("/mods/thing_p.utoc")
        );
        // A version before the suffix raises priority further; it is still a
        // suffixed name and must be left alone.
        assert_eq!(
            ensure_priority_suffix(PathBuf::from("/mods/thing_2_P.utoc")),
            PathBuf::from("/mods/thing_2_P.utoc")
        );
    }
}

impl Model {
    /// The container a tag would be written into, and whether it is a mod.
    pub(in crate::app) fn container_label_for_tag(&self, kit: usize, key: &str) -> Option<(String, bool)> {
        let source = self.kits.get(kit)?.source.as_ref()?;
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return None;
        };
        let TagEntryLocation::Container { container, .. } =
            &self.entry_for_key_in(kit, key)?.location
        else {
            return None;
        };
        containers
            .get(*container)
            .map(|container| (container.chunk_label.clone(), container.is_mod))
    }

    /// Every mod this workspace has mounted, by container label.
    pub(in crate::app) fn mounted_mod_labels(&self, kit: usize) -> Vec<String> {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return Vec::new();
        };
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return Vec::new();
        };
        containers
            .iter()
            .filter(|container| container.is_mod)
            .map(|container| container.chunk_label.clone())
            .collect()
    }

    /// The mounted containers an export to `output` would replace, by label.
    ///
    /// A mod installed under `Paks` is mounted like any other container, and
    /// mounting memory-maps its `.ucas`. Replacing that file means releasing the
    /// mapping first — Windows refuses to truncate a file with a mapped section
    /// open — so this is what the review dialog says out loud and what the export
    /// releases before it writes.
    pub(in crate::app) fn export_replaces_mounted(&self, kit: usize, output: &Path) -> Vec<String> {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return Vec::new();
        };
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return Vec::new();
        };
        crate::core::source::mounted_containers_at(&source.source, output)
            .into_iter()
            .filter_map(|index| containers.get(index))
            .map(|container| container.chunk_label.clone())
            .collect()
    }

    /// The mounted mod currently serving this tag, if the mount resolved it to
    /// one rather than to the game's own pack.
    pub(in crate::app) fn mod_serving_tag(&self, kit: usize, identity: &str) -> Option<String> {
        let source = self.kits.get(kit)?.source.as_ref()?;
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            return None;
        };
        let entry = self.campaign_entry_for_identity(kit, identity)?;
        let TagEntryLocation::Container { container, .. } = &entry.location else {
            return None;
        };
        containers
            .get(*container)
            .filter(|container| container.is_mod)
            .map(|container| container.chunk_label.clone())
    }
}
