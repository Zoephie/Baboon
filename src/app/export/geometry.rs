//! geometry export operations.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;

pub(in crate::app) fn extract_geometry_for_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    match &entry.group_tag.to_be_bytes() {
        b"hlmt" => extract_model_geometry(source, entry, output),
        b"scnr" => extract_scenario_geometry(source, entry, output),
        b"sbsp" => {
            let tag = read_entry(source, entry)?;
            let ass = AssFile::from_scenario_structure_bsp(&tag)?;
            fs::create_dir_all(output)?;
            let path = output.join(format!("{}.ASS", tag_file_stem(entry)));
            let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
            ass.write(&mut file)?;
            Ok(format!("Extracted BSP geometry {}", path.display()))
        }
        b"mode" | b"mod2" => {
            let tag = read_entry(source, entry)?;
            fs::create_dir_all(output)?;
            let stem = tag_file_stem(entry);
            let jms = render_jms_for_game(&tag)?;
            let path = output.join(format!("{stem}.render.jms"));
            let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
            jms.write(&mut file, blam_tags::game::Game::of(&tag).jms_version())?;
            Ok(format!(
                "Extracted render_model geometry {}",
                path.display()
            ))
        }
        // Neither of these tags stores its own bone transforms, so both need the
        // owning model's skeleton to come out anywhere but the origin.
        group @ (b"coll" | b"phmo") => {
            let collision = group == b"coll";
            let tag = read_entry(source, entry)?;
            fs::create_dir_all(output)?;
            let stem = tag_file_stem(entry);
            let skeleton = owning_model_skeleton(source, entry);
            let nodes = skeleton.as_ref().map(ModelSkeleton::nodes);
            let mut jms = if collision {
                collision_jms_for_game(&tag, nodes)?
            } else {
                physics_jms_for_game(&tag, nodes)?
            };
            if let Some(skel) = skeleton.as_ref().and_then(ModelSkeleton::campaign_evolved) {
                jms.reorient_for_campaign_evolved(skel);
            }
            let (group_name, kind) = if collision {
                ("collision_model", "collision")
            } else {
                ("physics_model", "physics")
            };
            let path = output.join(format!("{stem}.{kind}.jms"));
            let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
            jms.write(&mut file, blam_tags::game::Game::of(&tag).jms_version())?;
            // Halo 1 collision is node-local with no bind transform in the tag,
            // so there is no skeleton to miss there.
            let composes_a_skeleton =
                !(collision && blam_tags::game::Game::of(&tag) == blam_tags::game::Game::Halo1);
            Ok(format!(
                "Extracted {group_name} geometry {}{}",
                path.display(),
                if skeleton.is_none() && composes_a_skeleton {
                    " (no owning model found, so every bone is at the origin — \
                     extract the .model that references this tag instead)"
                } else {
                    ""
                }
            ))
        }
        // A particle_model is the merged geometry of every object in the
        // JMI it was imported from, so it comes back out as that manifest
        // plus one JMS per object — the layout `import particle model`
        // reads. `pmdf` is Halo 3 / Reach / Halo 4; `PRTM` is Halo 2's
        // unrelated tag of the same name, which additionally stores the
        // original object names.
        b"pmdf" | b"PRTM" => {
            let tag = read_entry(source, entry)?;
            let stem = tag_file_stem(entry);
            let summary =
                blam_tags::extract::particle_model::particle_model_to_dir(&tag, output, &stem)?;
            let objects = summary.emitted.iter().filter(|e| e.object.is_some()).count();
            let manifest = summary
                .emitted
                .first()
                .map(|e| e.path.display().to_string())
                .unwrap_or_default();
            Ok(format!(
                "Extracted particle geometry {manifest} ({objects} object{}){}",
                if objects == 1 { "" } else { "s" },
                if summary.names_are_authentic {
                    ""
                } else {
                    " — this engine stores no object names, so they are \
                     numbered from the tag name"
                },
            ))
        }
        _ => anyhow::bail!(
            "geometry extraction is not available for {}",
            format_group_tag(entry.group_tag)
        ),
    }
}

/// The rest pose a collision or physics JMS is composed against, plus — on
/// Campaign Evolved — the `skeleton model` whose Halo-style armature the
/// finished file is reoriented onto.
struct ModelSkeleton {
    nodes: Vec<blam_tags::JmsNode>,
    campaign_evolved: Option<TagFile>,
}

impl ModelSkeleton {
    fn nodes(&self) -> &[blam_tags::JmsNode] {
        &self.nodes
    }

    fn campaign_evolved(&self) -> Option<&TagFile> {
        self.campaign_evolved.as_ref()
    }
}

/// Collision and physics geometry is stored bone-local, and neither tag carries
/// its own bone transforms — those live in the `render_model` (or, on Campaign
/// Evolved, the `skeleton model`) that the owning `.model` names. Extracting one
/// of those tags on its own therefore had nothing to compose against: every bone
/// came out at the origin and every hull and shape piled up on top of it, which
/// is what a rigged model looks like collapsed to 0,0,0.
///
/// A standalone tag names no skeleton, so the owner is found by the tag's own
/// path: `objects/…/flood_tank.collision_model` belongs to
/// `objects/…/flood_tank.model`. Measured across the 2,486 collision and physics
/// tags in the Halo 3 tag set, that names a render_model one of the tag's real
/// owners uses in 2,479 cases; the 7 remaining have no `.model` beside them and
/// fall through to the render_model at the same path. Anything still unresolved
/// returns `None` and the export degrades to the old unposed output rather than
/// to a wrong pose.
fn owning_model_skeleton(source: &TagSource, entry: &TagEntry) -> Option<ModelSkeleton> {
    let reference = entry_reference_path(source, entry)?;
    if let Ok(model) = load_referenced_tag_from_source(source, &reference, "model", b"hlmt") {
        if let Some(skeleton) = model_skeleton(source, &model) {
            return Some(skeleton);
        }
    }
    // No `.model` beside the tag (or it named no usable skeleton) — a
    // render_model at the same path is the next-best owner.
    let render = load_referenced_tag_from_source(source, &reference, "render_model", b"mode").ok()?;
    Some(ModelSkeleton {
        nodes: render_jms_for_game(&render).ok()?.nodes,
        campaign_evolved: None,
    })
}

/// The rest pose a `.model` supplies to its collision and physics geometry:
/// the render_model's bind pose, or the `skeleton model` on Campaign Evolved,
/// which ships no render_model at all.
fn model_skeleton(source: &TagSource, model: &TagFile) -> Option<ModelSkeleton> {
    let root = model.root();
    if let Some(reference) = tag_ref_path(&root, "render model") {
        if let Ok(render) =
            load_referenced_tag_from_source(source, &reference, "render_model", b"mode")
        {
            if let Ok(jms) = render_jms_for_game(&render) {
                return Some(ModelSkeleton {
                    nodes: jms.nodes,
                    campaign_evolved: None,
                });
            }
        }
    }
    let reference = tag_ref_path(&root, "skeleton model")?;
    let skeleton = load_referenced_tag_from_source(source, &reference, "skeleton_model", b"skel")
        .ok()?;
    // Deliberately the raw rest pose — see the note in `extract_model_geometry`
    // on why the reorientation is applied after the geometry is placed.
    let nodes = JmsFile::skeleton_rest_pose(&skeleton).ok()?;
    Some(ModelSkeleton {
        nodes,
        campaign_evolved: Some(skeleton),
    })
}

/// A tag's own reference path, in the backslash form tag references use.
///
/// Taken from where the tag physically lives rather than from its display path
/// where possible: a monolithic cache stores the reference name verbatim, and a
/// loose file's is its path under the tags root. The display path is only a
/// fallback because building it replaces whatever follows the last dot with the
/// group's friendly extension, which truncates the handful of authoring names
/// that contain a literal dot.
fn entry_reference_path(source: &TagSource, entry: &TagEntry) -> Option<String> {
    let extension = entry
        .group_name
        .clone()
        .or_else(|| group_tag_to_extension(entry.group_tag).map(str::to_owned))?;
    let strip = |path: &str| -> Option<String> {
        let reference = path
            .strip_suffix(&format!(".{extension}"))
            .unwrap_or_else(|| path.rsplit_once('.').map_or(path, |(stem, _)| stem));
        (!reference.is_empty()).then(|| reference.replace('/', "\\"))
    };
    match &entry.location {
        TagEntryLocation::Monolithic { name, .. } if !name.is_empty() => Some(name.clone()),
        TagEntryLocation::LooseFile(path) => {
            let root = match source {
                TagSource::LooseFolder { root, .. } => Some(root.clone()),
                TagSource::SingleFile { path } => derive_tags_root(path),
                _ => None,
            }?;
            strip(&path.strip_prefix(&root).ok()?.to_string_lossy())
        }
        _ => strip(&entry.display_path),
    }
}

/// Halo 1 keeps collision geometry in `model_collision_geometry`, which stores
/// its BSPs per node with no region/permutation nesting; every later engine uses
/// `collision_model`. Reading a Halo 1 tag with the later walker found no `bsps`
/// under any permutation and wrote an empty file.
fn collision_jms_for_game(
    tag: &TagFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) -> anyhow::Result<JmsFile> {
    Ok(match blam_tags::game::Game::of(tag) {
        // Halo 1 collision vertices are already node-local and the tag holds no
        // bind transform, so there is nothing to compose a skeleton against.
        blam_tags::game::Game::Halo1 => JmsFile::from_model_collision_geometry(tag)?,
        _ => match skeleton {
            Some(skeleton) => JmsFile::from_collision_model_with_skeleton(tag, skeleton)?,
            None => JmsFile::from_collision_model(tag)?,
        },
    })
}

/// Halo 2 physics models store their shapes flat; Halo 3 and later nest them
/// behind rigid-body shape references.
fn physics_jms_for_game(
    tag: &TagFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) -> anyhow::Result<JmsFile> {
    Ok(match (blam_tags::game::Game::of(tag), skeleton) {
        (blam_tags::game::Game::Halo2, Some(skeleton)) => {
            JmsFile::from_physics_model_h2_with_skeleton(tag, skeleton)?
        }
        (blam_tags::game::Game::Halo2, None) => JmsFile::from_physics_model_h2(tag)?,
        (_, Some(skeleton)) => JmsFile::from_physics_model_with_skeleton(tag, skeleton)?,
        (_, None) => JmsFile::from_physics_model(tag)?,
    })
}

pub(in crate::app) fn extract_model_geometry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let model = read_entry(source, entry)?;
    let root = model.root();
    let render_ref = tag_ref_path(&root, "render model");
    let collision_ref = tag_ref_path(&root, "collision model");
    let physics_ref =
        tag_ref_path(&root, "physics_model").or_else(|| tag_ref_path(&root, "physics model"));
    let skeleton_ref = tag_ref_path(&root, "skeleton model");
    let stem = tag_file_stem(entry);

    let mut emitted = Vec::new();
    let mut skipped = Vec::new();

    let render_tag = match render_ref.as_deref() {
        Some(reference) => {
            match load_referenced_tag_from_source(source, reference, "render_model", b"mode") {
                Ok(tag) => Some(tag),
                Err(error) => {
                    skipped.push(format!("render: {error}"));
                    None
                }
            }
        }
        None => {
            // Campaign Evolved keeps render geometry in Unreal assets (it has a
            // `skeleton model` instead of a `render model`). Reconstruct the
            // high-resolution render JMS from the Unreal Nanite/skeletal meshes
            // fused onto the classic skeleton_model rig.
            if let Some(skel_ref) = skeleton_ref.as_deref() {
                match crate::app::model_preview::loading::campaign_evolved_render_jms(
                    &model, entry, source, skel_ref,
                ) {
                    Ok(jms) => {
                        let render_dir = output.join("render");
                        fs::create_dir_all(&render_dir)?;
                        let path = render_dir.join(format!("{stem}.render.jms"));
                        let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
                        // Campaign Evolved runs on the Halo Reach engine, whose
                        // toolset uses the Halo 3-era JMS format (8213) — N-influence
                        // vertices with region/permutation encoded in the material
                        // slot names — not the rigid Halo 1 8200 layout (2-influence
                        // vertices + a per-triangle region section this path never
                        // populates, which desyncs the importer).
                        jms.write(&mut file, 8213)?;
                        emitted.push(format!("render {}", path.display()));
                    }
                    Err(error) => skipped.push(format!("render (CE): {error}")),
                }
            } else {
                skipped.push("render: no render_model reference".to_owned());
            }
            None
        }
    };

    let render_jms_for_skeleton = match render_tag.as_ref() {
        Some(tag) => match render_jms_for_game(tag) {
            Ok(jms) => Some(jms),
            Err(error) => {
                skipped.push(format!("render skeleton: {error}"));
                None
            }
        },
        None => None,
    };
    let render_jms_version = render_tag
        .as_ref()
        .map(|tag| blam_tags::game::Game::of(tag).jms_version())
        .unwrap_or(8213);
    // Campaign Evolved has no render_model to take a skeleton from, so collision
    // hulls stayed in bone-local space (every limb stacked on the pelvis) and
    // physics shapes hung off bones that were all at the origin. The
    // `skeleton model` holds the same rest pose the render path uses.
    let campaign_evolved_skeleton = match (render_tag.as_ref(), skeleton_ref.as_deref()) {
        (None, Some(reference)) => {
            match load_referenced_tag_from_source(source, reference, "skeleton_model", b"skel") {
                Ok(tag) => Some(tag),
                Err(error) => {
                    skipped.push(format!("skeleton: {error}"));
                    None
                }
            }
        }
        _ => None,
    };
    // Deliberately the raw rest pose, not the armature the render JMS emits:
    // that one is reoriented, which preserves bone positions but changes almost
    // every rotation, and geometry composed against it would be twisted off its
    // bone. The reorientation is applied afterwards instead, once the geometry
    // is placed.
    let campaign_evolved_rest_pose = campaign_evolved_skeleton
        .as_ref()
        .and_then(|tag| JmsFile::skeleton_rest_pose(tag).ok());
    let skeleton = render_jms_for_skeleton
        .as_ref()
        .map(|jms| jms.nodes.as_slice())
        .or(campaign_evolved_rest_pose.as_deref());

    if let Some(tag) = render_tag.as_ref() {
        let render_dir = output.join("render");
        fs::create_dir_all(&render_dir)?;
        let game = blam_tags::game::Game::of(tag);
        if matches!(game, blam_tags::game::Game::Halo3) && render_model_prefers_ass(tag) {
            let ass = AssFile::from_render_model(tag)?;
            let path = render_dir.join(format!("{stem}.render.ASS"));
            let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
            ass.write(&mut file)?;
            emitted.push(format!("render {}", path.display()));
        } else if let Some(jms) = render_jms_for_skeleton.as_ref() {
            let path = render_dir.join(format!("{stem}.render.jms"));
            let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
            jms.write(&mut file, render_jms_version)?;
            emitted.push(format!("render {}", path.display()));
        }
    }

    match collision_ref.as_deref() {
        Some(reference) => {
            match load_referenced_tag_from_source(source, reference, "collision_model", b"coll") {
                Ok(tag) => {
                    let collision_dir = output.join("collision");
                    fs::create_dir_all(&collision_dir)?;
                    let mut jms = collision_jms_for_game(&tag, skeleton)?;
                    if let Some(skel) = campaign_evolved_skeleton.as_ref() {
                        jms.reorient_for_campaign_evolved(skel);
                    }
                    let path = collision_dir.join(format!("{stem}.collision.jms"));
                    let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
                    jms.write(&mut file, blam_tags::game::Game::of(&tag).jms_version())?;
                    emitted.push(format!("collision {}", path.display()));
                }
                Err(error) => skipped.push(format!("collision: {error}")),
            }
        }
        None => skipped.push("collision: no collision_model reference".to_owned()),
    }

    match physics_ref.as_deref() {
        Some(reference) => {
            match load_referenced_tag_from_source(source, reference, "physics_model", b"phmo") {
                Ok(tag) => {
                    let physics_dir = output.join("physics");
                    fs::create_dir_all(&physics_dir)?;
                    let mut jms = physics_jms_for_game(&tag, skeleton)?;
                    if let Some(skel) = campaign_evolved_skeleton.as_ref() {
                        jms.reorient_for_campaign_evolved(skel);
                    }
                    let path = physics_dir.join(format!("{stem}.physics.jms"));
                    let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
                    jms.write(&mut file, blam_tags::game::Game::of(&tag).jms_version())?;
                    emitted.push(format!("physics {}", path.display()));
                }
                Err(error) => skipped.push(format!("physics: {error}")),
            }
        }
        None => skipped.push("physics: no physics_model reference".to_owned()),
    }

    if emitted.is_empty() {
        anyhow::bail!(
            "model geometry extraction emitted nothing: {}",
            skipped.join("; ")
        );
    }
    let mut message = format!(
        "Extracted {} model geometry file(s) to {}",
        emitted.len(),
        output.display()
    );
    if !skipped.is_empty() {
        message.push_str(&format!("; skipped {}", skipped.join("; ")));
    }
    Ok(message)
}

pub(in crate::app) fn load_referenced_tag_from_source(
    source: &TagSource,
    reference: &str,
    extension: &str,
    group_tag: &[u8; 4],
) -> anyhow::Result<TagFile> {
    let group_tag = u32::from_be_bytes(*group_tag);
    match source {
        TagSource::LooseFolder { root, .. } => {
            let path = resolve_tag_path(root, reference, extension);
            let entry = TagEntry {
                key: format!("file:{}", path.display()),
                display_path: format!("{}.{}", reference.replace('\\', "/"), extension),
                group_tag,
                group_name: Some(extension.to_owned()),
                location: TagEntryLocation::LooseFile(path.clone()),
            };
            read_entry(source, &entry)
                .map_err(|error| anyhow::anyhow!("read {} failed: {error}", path.display()))
        }
        TagSource::SingleFile { path } => {
            let root = derive_tags_root(path)
                .or_else(|| path.parent().map(Path::to_path_buf))
                .ok_or_else(|| {
                    anyhow::anyhow!("could not derive a tag root for {}", path.display())
                })?;
            let resolved = resolve_tag_path(&root, reference, extension);
            TagFile::read(&resolved)
                .map_err(|error| anyhow::anyhow!("read {} failed: {error}", resolved.display()))
        }
        TagSource::MonolithicCache { cache, .. } => cache
            .read_tag_by_name(group_tag, reference)
            .map_err(|error| anyhow::anyhow!("read {reference}.{extension} failed: {error}")),
        TagSource::IoStoreContainerSet { .. } => source
            .read_container_tag_by_ref(group_tag, reference)
            .map_err(|error| anyhow::anyhow!("read {reference}.{extension} failed: {error}")),
    }
}

pub(in crate::app) fn render_jms_for_game(tag: &TagFile) -> anyhow::Result<JmsFile> {
    Ok(match blam_tags::game::Game::of(tag) {
        blam_tags::game::Game::Halo1 => JmsFile::from_gbxmodel(tag)?,
        blam_tags::game::Game::Halo2 => JmsFile::from_h2_render_model(tag)?,
        blam_tags::game::Game::Halo3 => JmsFile::from_render_model(tag)?,
    })
}

pub(in crate::app) fn render_model_prefers_ass(tag: &TagFile) -> bool {
    let root = tag.root();
    let instance_mesh_index = root
        .field("instance mesh index")
        .and_then(|field| field.value())
        .and_then(|value| match value {
            TagFieldData::LongBlockIndex(index) => Some(index as i64),
            TagFieldData::CustomLongBlockIndex(index) => Some(index as i64),
            TagFieldData::ShortBlockIndex(index) => Some(index as i64),
            TagFieldData::LongInteger(index) => Some(index as i64),
            _ => None,
        })
        .unwrap_or(-1);
    let placements_len = root
        .field("instance placements")
        .and_then(|field| field.as_block())
        .map(|block| block.len())
        .unwrap_or(0);
    instance_mesh_index >= 0 && placements_len > 0
}

/// Adapts a [`TagSource`] into a [`blam_tags::extract::TagResolver`] so the
/// shared extraction orchestration can resolve child tag references
/// (jmad → render_model, scenario → structure_bsp/stli) through Baboon's
/// cache- and classic-aware loader.
struct SourceResolver<'a> {
    source: &'a TagSource,
}

impl blam_tags::extract::TagResolver for SourceResolver<'_> {
    fn resolve(
        &self,
        reference: &str,
        group_ext: &str,
        group_tag: u32,
    ) -> Result<TagFile, blam_tags::extract::ExtractError> {
        load_referenced_tag_from_source(self.source, reference, group_ext, &group_tag.to_be_bytes())
            .map_err(|error| blam_tags::extract::ExtractError::resolve(error.to_string()))
    }
}

/// The `.model` that owns a selected `model_animation_graph`, when swapping it
/// in for the graph would give the export a rest pose it does not otherwise
/// have.
///
/// Applied only to graphs whose `additional node data` leaves at least one
/// skeleton bone without a rest pose, so the 2,515 Reach graphs that are
/// already complete export byte-for-byte as before. The owner is found by the
/// graph's own path and then **verified**: it counts only if that model's
/// `animation` reference points back at this graph. Measured over Halo Reach,
/// that improves 45 of the 79 short graphs (`magnum`, `plasma_pistol`, and the
/// cinematic object graphs among them); the other 34 have no model beside them
/// and keep today's behaviour. Halo 3's 50 short graphs have no sibling model
/// at all, so nothing there changes.
fn animation_graph_owner(
    source: &TagSource,
    entry: &TagEntry,
    jmad: &TagFile,
) -> Option<TagFile> {
    if entry.group_tag != u32::from_be_bytes(*b"jmad") {
        return None;
    }
    let skeleton = blam_tags::Skeleton::from_tag(jmad);
    if skeleton.is_empty() || animation_rest_pose_is_complete(jmad, &skeleton) {
        return None;
    }
    let reference = entry_reference_path(source, entry)?;
    let model = load_referenced_tag_from_source(source, &reference, "model", b"hlmt").ok()?;
    let names_this_graph = tag_ref_path(&model.root(), "animation")
        .is_some_and(|r| r.eq_ignore_ascii_case(&reference));
    names_this_graph.then_some(model)
}

/// Whether the jmad's own `additional node data` gives every skeleton bone a
/// rest pose.
fn animation_rest_pose_is_complete(jmad: &TagFile, skeleton: &blam_tags::Skeleton) -> bool {
    let named: std::collections::HashSet<String> = jmad
        .root()
        .field_path("additional node data")
        .and_then(|field| field.as_block())
        .map(|block| {
            (0..block.len())
                .filter_map(|i| block.element(i)?.read_string_id("node name"))
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    skeleton.nodes.iter().all(|node| named.contains(&node.name))
}

/// Extract every animation in `entry` (a jmad, `.model`, object tag, or
/// Halo CE `model_animations`) to JMA-family files under
/// `<output>/<stem>/animations/`, in-process via `blam_tags::extract`.
pub(in crate::app) fn extract_animations_for_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    let resolver = SourceResolver { source };
    let stem = tag_file_stem(entry);
    // A graph extracted on its own names no render_model, so its rest pose can
    // only come from `additional node data` — the denormalised copy inside the
    // jmad. 79 of Halo Reach's 2,594 graphs carry none at all, and those export
    // with every bone at identity. Hand the extractor the owning `.model`
    // instead, exactly as if the model had been the selected tag.
    let owner = animation_graph_owner(source, entry, &tag);
    let input = owner.as_ref().unwrap_or(&tag);
    let summary =
        blam_tags::extract::animation::animations_to_dir(input, &resolver, output, &stem)?;
    let mut message = format!(
        "Extracted {} animation(s) from {} into {}",
        summary.written,
        entry.display_path,
        output.display(),
    );
    if summary.skipped > 0 {
        message.push_str(&format!(" ({} skipped)", summary.skipped));
    }
    Ok(message)
}

/// Extract per-BSP scenario geometry — one ASS (Halo 2 / Halo 3) or render +
/// collision JMS (Halo CE) per structure BSP — under
/// `<output>/<stem>/structure/`, in-process via `blam_tags::extract`.
pub(in crate::app) fn extract_scenario_geometry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    let resolver = SourceResolver { source };
    let stem = tag_file_stem(entry);
    let summary =
        blam_tags::extract::geometry::scenario_geometry_to_dir(&tag, &resolver, output, &stem)?;
    let mut message = format!(
        "Extracted {} geometry file(s) from {} into {}",
        summary.emitted.len(),
        entry.display_path,
        output.display(),
    );
    if !summary.warnings.is_empty() {
        message.push_str(&format!(" ({} warning(s))", summary.warnings.len()));
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tags_own_reference_path_drops_only_the_group_extension() {
        let root = PathBuf::from("/kits/halo3/tags");
        let source = TagSource::LooseFolder {
            root: root.clone(),
            game: Some("halo3_mcc".to_owned()),
            definitions_root: PathBuf::from("/definitions"),
        };
        let loose = |relative: &str| TagEntry {
            key: relative.to_owned(),
            display_path: relative.to_owned(),
            group_tag: u32::from_be_bytes(*b"coll"),
            group_name: Some("collision_model".to_owned()),
            location: TagEntryLocation::LooseFile(root.join(relative)),
        };

        assert_eq!(
            entry_reference_path(
                &source,
                &loose("objects/characters/flood_tank/flood_tank.collision_model")
            )
            .as_deref(),
            Some("objects\\characters\\flood_tank\\flood_tank")
        );
        // Authoring names may contain a literal dot. The group extension is a
        // known suffix, so it comes off without taking the dotted name with it.
        assert_eq!(
            entry_reference_path(&source, &loose("objects/gear/crate_1.5.collision_model"))
                .as_deref(),
            Some("objects\\gear\\crate_1.5")
        );
        // A monolithic cache stores the reference name itself — no derivation.
        let cached = TagEntry {
            key: "cache:coll:x".to_owned(),
            display_path: "objects/gear/crate_1.collision_model".to_owned(),
            group_tag: u32::from_be_bytes(*b"coll"),
            group_name: Some("collision_model".to_owned()),
            location: TagEntryLocation::Monolithic {
                name: "objects\\gear\\crate_1.5".to_owned(),
                group_tag: u32::from_be_bytes(*b"coll"),
            },
        };
        assert_eq!(
            entry_reference_path(&source, &cached).as_deref(),
            Some("objects\\gear\\crate_1.5")
        );
    }

    /// A `collision_model` / `physics_model` extracted on its own used to come
    /// out collapsed — every bone at the origin and every hull and shape stacked
    /// there with it — because nothing supplied the rest pose those tags do not
    /// store. Guards that the owning `.model` is now found from the tag's own
    /// path and its skeleton composed in.
    ///
    /// Ignored by default — it needs a loose Halo 3 tag tree.
    ///
    /// Run with:
    ///   H3_TAGS=~/Halo/halo3_mcc/tags \
    ///     cargo test standalone_collision -- --ignored --nocapture
    #[test]
    #[ignore = "requires a loose Halo 3 tag tree; set H3_TAGS"]
    fn standalone_collision_and_physics_export_is_posed_by_the_owning_model() {
        let Ok(root) = std::env::var("H3_TAGS") else {
            eprintln!("skipping: set H3_TAGS to a loose Halo 3 tags directory");
            return;
        };
        let root = PathBuf::from(root);
        let source = TagSource::LooseFolder {
            root: root.clone(),
            game: Some("halo3_mcc".to_owned()),
            definitions_root: crate::app::locate_definitions_root(),
        };
        // A rigged character: its collision hulls and physics shapes are stored
        // per-bone, so an unposed export piles all of them on the origin.
        let stem = "objects/characters/flood_tank/flood_tank";
        let cases = [
            ("collision_model", *b"coll"),
            ("physics_model", *b"phmo"),
        ];
        for (extension, group_tag) in cases {
            let path = root.join(format!("{stem}.{extension}"));
            assert!(path.is_file(), "{} is not in this tag tree", path.display());
            let entry = TagEntry {
                key: format!("file:{}", path.display()),
                display_path: format!("{stem}.{extension}"),
                group_tag: u32::from_be_bytes(group_tag),
                group_name: Some(extension.to_owned()),
                location: TagEntryLocation::LooseFile(path),
            };

            let skeleton = owning_model_skeleton(&source, &entry)
                .unwrap_or_else(|| panic!("{extension}: no owning model resolved"));
            let posed = skeleton
                .nodes()
                .iter()
                .filter(|node| {
                    node.translation.x != 0.0
                        || node.translation.y != 0.0
                        || node.translation.z != 0.0
                })
                .count();
            assert!(
                posed > 1,
                "{extension}: the owning model's skeleton is itself unposed \
                 ({posed}/{} bones off the origin)",
                skeleton.nodes().len()
            );

            let tag = read_entry(&source, &entry).expect("read tag");
            let unposed = match extension {
                "collision_model" => collision_jms_for_game(&tag, None),
                _ => physics_jms_for_game(&tag, None),
            }
            .expect("build unposed jms");
            let jms = match extension {
                "collision_model" => collision_jms_for_game(&tag, Some(skeleton.nodes())),
                _ => physics_jms_for_game(&tag, Some(skeleton.nodes())),
            }
            .expect("build posed jms");

            // The armature the file emits, not just the skeleton handed in.
            let off_origin = jms
                .nodes
                .iter()
                .filter(|node| {
                    node.translation.x != 0.0
                        || node.translation.y != 0.0
                        || node.translation.z != 0.0
                })
                .count();
            assert!(
                off_origin > 1,
                "{extension}: {off_origin}/{} emitted bones are off the origin — \
                 the skeleton was not overlaid",
                jms.nodes.len()
            );
            assert_eq!(
                unposed
                    .nodes
                    .iter()
                    .filter(|node| node.translation.x != 0.0)
                    .count(),
                0,
                "{extension}: the no-skeleton control is supposed to be collapsed; \
                 if it is not, this test proves nothing"
            );
            // The armature also has to be a hierarchy, not a flat pile of roots
            // — a collision node spells its parent link `parent node`.
            assert!(
                jms.nodes.iter().filter(|node| node.parent >= 0).count() > 1,
                "{extension}: the emitted armature has no bone hierarchy"
            );

            // Collision vertices are absolute, so composing the skeleton in has
            // to move them; physics shapes stay node-local and are placed by the
            // bone at import time, so only the armature changes there.
            if extension == "collision_model" {
                assert_ne!(
                    unposed.vertices.len(),
                    0,
                    "collision_model exported no geometry"
                );
                let moved = jms
                    .vertices
                    .iter()
                    .zip(unposed.vertices.iter())
                    .filter(|(a, b)| a.position.x != b.position.x || a.position.z != b.position.z)
                    .count();
                assert!(
                    moved > jms.vertices.len() / 2,
                    "only {moved}/{} collision vertices moved when the skeleton was applied",
                    jms.vertices.len()
                );
            }

            // And the same through the entry point the browser's Extract
            // Geometry menu actually calls.
            let out = std::env::temp_dir().join("baboon_standalone_geometry_test");
            let _ = std::fs::remove_dir_all(&out);
            let message =
                extract_geometry_for_entry(&source, &entry, &out).expect("extract geometry");
            println!("{message}");
            assert!(
                !message.contains("no owning model"),
                "{extension}: the export path did not resolve a skeleton — {message}"
            );
        }
    }

    /// A `model_animation_graph` that stores no `additional node data` has no
    /// rest pose reachable from the graph alone, so extracting it on its own
    /// wrote every bone at identity. Guards that the owning `.model` is now
    /// found from the graph's own path and its render_model used instead.
    ///
    /// Ignored by default — it needs a loose Halo Reach tag tree.
    ///
    /// Run with:
    ///   REACH_TAGS=~/Halo/haloreach_mcc/tags \
    ///     cargo test animation_graph_without -- --ignored --nocapture
    #[test]
    #[ignore = "requires a loose Halo Reach tag tree; set REACH_TAGS"]
    fn animation_graph_without_its_own_rest_pose_borrows_the_owning_models() {
        let Ok(root) = std::env::var("REACH_TAGS") else {
            eprintln!("skipping: set REACH_TAGS to a loose Halo Reach tags directory");
            return;
        };
        let root = PathBuf::from(root);
        let source = TagSource::LooseFolder {
            root: root.clone(),
            game: Some("haloreach_mcc".to_owned()),
            definitions_root: crate::app::locate_definitions_root(),
        };
        // The magnum's own graph: five gun bones, and not one `additional node
        // data` entry to place them with.
        let stem = "objects/weapons/pistol/magnum/magnum";
        let path = root.join(format!("{stem}.model_animation_graph"));
        assert!(path.is_file(), "{} is not in this tag tree", path.display());
        let entry = TagEntry {
            key: format!("file:{}", path.display()),
            display_path: format!("{stem}.model_animation_graph"),
            group_tag: u32::from_be_bytes(*b"jmad"),
            group_name: Some("model_animation_graph".to_owned()),
            location: TagEntryLocation::LooseFile(path),
        };

        let jmad = read_entry(&source, &entry).expect("read the graph");
        let skeleton = blam_tags::Skeleton::from_tag(&jmad);
        assert!(
            !animation_rest_pose_is_complete(&jmad, &skeleton),
            "this graph carries its own rest pose, so it proves nothing here"
        );

        let owner = animation_graph_owner(&source, &entry, &jmad)
            .expect("the magnum's .model should be found and should name this graph");
        let resolved =
            blam_tags::extract::animation::resolve_animation_inputs(&owner, &SourceResolver {
                source: &source,
            })
            .expect("resolve through the owning model");
        let render = resolved
            .render_model
            .as_ref()
            .expect("the owning model names a render_model");

        let object_space = blam_tags::extract::animation::additional_node_data_is_object_space(
            &blam_tags::Animation::new(&jmad).expect("read animations"),
        );
        let without =
            blam_tags::extract::animation::build_defaults(&skeleton, &jmad, None, object_space);
        let with = blam_tags::extract::animation::build_defaults(
            &skeleton,
            &jmad,
            Some(render),
            object_space,
        );

        let posed = |set: &[blam_tags::NodeTransform]| {
            set.iter()
                .filter(|t| {
                    t.translation.x != 0.0 || t.translation.y != 0.0 || t.translation.z != 0.0
                })
                .count()
        };
        assert_eq!(
            posed(&without),
            0,
            "the graph-only control is supposed to be collapsed to identity; \
             if it is not, this test proves nothing"
        );
        assert!(
            posed(&with) > 1,
            "only {}/{} bones got a rest pose from the owning model",
            posed(&with),
            with.len()
        );
    }

    /// End-to-end against a real Campaign Evolved install: mount the pak set,
    /// find a structure BSP and the scenario that references it, and run both
    /// export paths the browser's Extract menu now offers.
    ///
    /// CE is a Blam/Unreal hybrid — the Blam tag owns collision, Unreal owns
    /// rendered geometry — so a CE BSP legitimately exports collision only.
    /// What this guards is that it exports *something substantial*: before the
    /// collision fallback in blam-tags, `c10/level_a` produced a single
    /// 198-vertex object out of 775 instanced definitions.
    ///
    /// Ignored by default — it needs the shipped game.
    ///
    /// Run with:
    ///   CE_ROOT="D:/SteamLibrary/steamapps/common/Halo Campaign Evolved" \
    ///     cargo test ce_structure_bsp -- --ignored --nocapture
    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_ROOT"]
    fn ce_structure_bsp_and_scenario_export_geometry() {
        let Ok(root) = std::env::var("CE_ROOT") else {
            eprintln!("skipping: set CE_ROOT to the Campaign Evolved install root");
            return;
        };
        let root = PathBuf::from(root);
        let paks = crate::source::find_paks_dir(&root)
            .unwrap_or_else(|| panic!("no Paks dir under {}", root.display()));

        let definitions = crate::app::locate_definitions_root();
        let names = crate::format::TagNameIndex::load_game(&definitions, "haloce_evolved")
            .expect("load haloce_evolved tag-name index");
        let loaded = crate::source::load_iostore_container_set(paks, &names, &definitions)
            .expect("mount CE container set");

        let find = |suffix: &str, group: &[u8; 4]| {
            let want = u32::from_be_bytes(*group);
            loaded
                .all_entries
                .iter()
                .chain(loaded.entries.iter())
                .find(|e| {
                    e.group_tag == want
                        && e.display_path.replace('\\', "/").to_ascii_lowercase().contains(suffix)
                })
                .cloned()
        };

        let out = std::env::temp_dir().join("baboon_ce_geometry_test");
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).expect("create out dir");

        // Single BSP → one ASS.
        let bsp = find("c10/_generated_/level_a", b"sbsp")
            .or_else(|| find("level_a", b"sbsp"))
            .expect("no c10 level_a scenario_structure_bsp mounted");
        let msg = extract_geometry_for_entry(&loaded.source, &bsp, &out).expect("export BSP");
        println!("{msg}");
        let ass = out.join(format!("{}.ASS", tag_file_stem(&bsp)));
        let len = std::fs::metadata(&ass).expect("BSP ASS written").len();
        assert!(
            len > 1_000_000,
            "{} is only {len} bytes — the collision fallback did not run (needs a \
             blam-tags with the collision-only BSP support)",
            ass.display()
        );

        // Scenario → one file per referenced BSP.
        let scnr = find("c10", b"scnr").expect("no c10 scenario mounted");
        let msg = extract_scenario_geometry(&loaded.source, &scnr, &out).expect("export scenario");
        println!("{msg}");
        let structure = out.join(tag_file_stem(&scnr)).join("structure");
        let emitted: Vec<_> = std::fs::read_dir(&structure)
            .unwrap_or_else(|e| panic!("read {}: {e}", structure.display()))
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("ass"))
            })
            .collect();
        assert!(
            emitted.len() > 1,
            "scenario emitted {} file(s) into {} — expected one per structure_bsp",
            emitted.len(),
            structure.display()
        );
        for e in &emitted {
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            println!("  {} — {len} bytes", e.path().display());
            assert!(len > 0, "{} is empty", e.path().display());
        }
    }
}

#[cfg(test)]
#[path = "../tests/particle_model_extract_menu.rs"]
mod particle_model_extract_menu;
