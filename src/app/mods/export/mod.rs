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

    /// The mounted mod currently serving this tag, if the mount resolved it to
    /// one rather than to the game's own pack.
    pub(in crate::app) fn mod_serving_tag(&self, kit: usize, identity: &str) -> Option<String> {
        let source = self.model.kits.get(kit)?.source.as_ref()?;
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
            let Some(entry) = self.campaign_entry_for_identity(exporting, &overlay.identity) else {
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
                        || self.find_container_template_in(exporting, *group_tag),
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
                    self.mods.exported_mod = Some(ExportedMod {
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
mod mod_output_tests;

#[cfg(test)]
mod mod_override_tests;

#[cfg(test)]
mod priority_suffix_tests;

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
}
