//! geometry export operations.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;

use blam_tags::game::Game;
use blam_tags::jms_generation::Halo1Permutation;

/// Write `jms` the way `target`'s tools import it.
///
/// Halo 2 and later take one file, `<dir>/<file_name>`, at the target's JMS
/// version. Halo CE takes a permutation from a file's name, so an H2/H3-form
/// JMS is split into one `<halo1_dir>/<permutation>.jms` per permutation; one
/// that is already Halo CE's (a collision JMS built from a Halo CE tag) is
/// written as it is. Whatever the target cannot hold is added to `notes`.
fn write_jms_for_target(
    jms: &JmsFile,
    source: Game,
    target: Game,
    dir: &Path,
    file_name: &str,
    halo1_dir: &Path,
    notes: &mut Vec<String>,
) -> anyhow::Result<Vec<PathBuf>> {
    if target == Game::Halo1 && source != Game::Halo1 {
        let (permutations, warnings) = jms.split_for_halo1();
        notes.extend(warnings);
        return write_halo1_permutations(&permutations, halo1_dir);
    }
    fs::create_dir_all(dir)?;
    let path = dir.join(file_name);
    let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
    jms.write(&mut file, target.jms_version())?;
    Ok(vec![path])
}

/// One Halo CE JMS per permutation, named for it, in `dir`.
fn write_halo1_permutations(
    permutations: &[Halo1Permutation],
    dir: &Path,
) -> anyhow::Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)?;
    let mut written = Vec::with_capacity(permutations.len());
    for permutation in permutations {
        let path = dir.join(permutation.file_name());
        let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
        permutation
            .jms
            .write(&mut file, Game::Halo1.jms_version())?;
        written.push(path);
    }
    Ok(written)
}

/// A render model's geometry for `target`. A Halo CE gbxmodel is read one
/// permutation at a time — the form Halo CE tool.exe imports — and, for a
/// later target, merged into one file whose material lines carry each
/// triangle's permutation and region.
fn write_render_geometry(
    tag: &TagFile,
    target: Game,
    dir: &Path,
    stem: &str,
    halo1_dir: &Path,
    notes: &mut Vec<String>,
) -> anyhow::Result<Vec<PathBuf>> {
    let source = Game::of(tag);
    if source != Game::Halo1 {
        let jms = render_jms_for_game(tag)?;
        return write_jms_for_target(
            &jms,
            source,
            target,
            dir,
            &format!("{stem}.render.jms"),
            halo1_dir,
            notes,
        );
    }
    let permutations = JmsFile::gbxmodel_permutation_names(tag)
        .into_iter()
        .map(|name| {
            let jms = JmsFile::from_gbxmodel_permutation(tag, Some(&name))?;
            Ok(Halo1Permutation { name, jms })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if target == Game::Halo1 {
        return write_halo1_permutations(&permutations, halo1_dir);
    }
    let (jms, warnings) = JmsFile::merge_halo1_permutations(&permutations);
    notes.extend(warnings);
    write_jms_for_target(
        &jms,
        target,
        target,
        dir,
        &format!("{stem}.render.jms"),
        halo1_dir,
        notes,
    )
}

/// The ASS version `target` reads, or — Halo CE has no ASS — the source's,
/// with a note saying so.
fn ass_version_for(source: Game, target: Game, notes: &mut Vec<String>) -> u32 {
    match target.ass_version() {
        Some(version) => version as u32,
        None => {
            notes.push(
                "Halo CE has no ASS format, so ASS geometry keeps this game's version".to_owned(),
            );
            source.ass_version().unwrap_or(7) as u32
        }
    }
}

/// `message`, with `notes` after it.
fn with_notes(mut message: String, notes: &[String]) -> String {
    if !notes.is_empty() {
        message.push_str(&format!(" — {}", notes.join("; ")));
    }
    message
}

/// The file paths written, for a status line.
fn display_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Extract `entry`'s geometry for `target`'s tools: the JMS and ASS versions
/// that game imports, and for Halo CE one JMS per permutation (see
/// [`write_jms_for_target`]). Level and particle geometry keep the source
/// game's form.
pub(in crate::app) fn extract_geometry_for_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
    target: Game,
) -> anyhow::Result<String> {
    match &entry.group_tag.to_be_bytes() {
        b"hlmt" => extract_model_geometry(source, entry, output, target),
        b"scnr" => extract_scenario_geometry(source, entry, output, target),
        b"sbsp" => {
            let tag = read_entry(source, entry)?;
            let mut notes = Vec::new();
            let version = ass_version_for(Game::of(&tag), target, &mut notes);
            let ass = AssFile::from_scenario_structure_bsp(&tag)?;
            fs::create_dir_all(output)?;
            let path = output.join(format!("{}.ASS", tag_file_stem(entry)));
            let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
            ass.write_version(&mut file, version)?;
            Ok(with_notes(
                format!("Extracted BSP geometry {}", path.display()),
                &notes,
            ))
        }
        b"mode" | b"mod2" => {
            let tag = read_entry(source, entry)?;
            let stem = tag_file_stem(entry);
            let mut notes = Vec::new();
            let written = write_render_geometry(
                &tag,
                target,
                output,
                &stem,
                &output.join("models"),
                &mut notes,
            )?;
            Ok(with_notes(
                format!(
                    "Extracted render_model geometry {}",
                    display_paths(&written)
                ),
                &notes,
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
            let source_game = Game::of(&tag);
            let mut notes = Vec::new();
            if skeleton.is_none() {
                notes.push(
                    "no owning model found, so every bone is at the origin — extract the \
                     .model that references this tag instead"
                        .to_owned(),
                );
            }
            if !collision && target == Game::Halo1 && source_game != Game::Halo1 {
                anyhow::bail!("Halo CE has no physics model, so there is nothing to write for it");
            }
            let written = write_jms_for_target(
                &jms,
                source_game,
                target,
                output,
                &format!("{stem}.{kind}.jms"),
                &output.join("physics"),
                &mut notes,
            )?;
            Ok(with_notes(
                format!(
                    "Extracted {group_name} geometry {}",
                    display_paths(&written)
                ),
                &notes,
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
            let objects = summary
                .emitted
                .iter()
                .filter(|e| e.object.is_some())
                .count();
            let manifest = summary
                .emitted
                .first()
                .map(|e| e.path.display().to_string())
                .unwrap_or_default();
            Ok(format!(
                "Extracted particle geometry {manifest} ({objects} object{}){}{}",
                if objects == 1 { "" } else { "s" },
                if summary.names_are_authentic {
                    ""
                } else {
                    " — this engine stores no object names, so they are \
                     numbered from the tag name"
                },
                if target == Game::of(&tag) {
                    ""
                } else {
                    " — particle geometry is written in this game's own form"
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
pub(in crate::app) struct ModelSkeleton {
    nodes: Vec<blam_tags::JmsNode>,
    campaign_evolved: Option<TagFile>,
}

impl ModelSkeleton {
    pub(in crate::app) fn nodes(&self) -> &[blam_tags::JmsNode] {
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
pub(in crate::app) fn owning_model_skeleton(
    source: &TagSource,
    entry: &TagEntry,
) -> Option<ModelSkeleton> {
    let reference = entry_reference_path(source, entry)?;
    if let Ok(model) = load_referenced_tag_from_source(source, &reference, "model", b"hlmt") {
        if let Some(skeleton) = model_skeleton(source, &model) {
            return Some(skeleton);
        }
    }
    // No `.model` beside the tag (or it named no usable skeleton) — a render
    // tag at the same path is the next-best owner. Halo CE has no hlmt wrapper
    // and calls that tag `gbxmodel`; later engines call it `render_model`.
    let render = load_referenced_tag_from_source(source, &reference, "render_model", b"mode")
        .or_else(|_| load_referenced_tag_from_source(source, &reference, "gbxmodel", b"mod2"))
        .ok()?;
    Some(ModelSkeleton {
        nodes: render_model_skeleton(&render).ok()?,
        campaign_evolved: None,
    })
}

/// The rest pose a `.model` supplies to its collision and physics geometry:
/// the render_model's bind pose, or the `skeleton model` on Campaign Evolved,
/// which ships no render_model at all.
pub(in crate::app) fn model_skeleton(source: &TagSource, model: &TagFile) -> Option<ModelSkeleton> {
    let root = model.root();
    if let Some(reference) = tag_ref_path(&root, "render model") {
        if let Ok(render) =
            load_referenced_tag_from_source(source, &reference, "render_model", b"mode")
        {
            if let Ok(nodes) = render_model_skeleton(&render) {
                return Some(ModelSkeleton {
                    nodes,
                    campaign_evolved: None,
                });
            }
        }
    }
    let reference = tag_ref_path(&root, "skeleton model")?;
    let skeleton =
        load_referenced_tag_from_source(source, &reference, "skeleton_model", b"skel").ok()?;
    // Deliberately the raw rest pose — see the note in `extract_model_geometry`
    // on why the reorientation is applied after the geometry is placed.
    let nodes = JmsFile::skeleton_rest_pose(&skeleton).ok()?;
    Some(ModelSkeleton {
        nodes,
        campaign_evolved: Some(skeleton),
    })
}

/// Read the bind pose through the same [`RenderModel`] conversion as the live
/// preview, then compose it to world space in JMS centimetres. This matters on
/// Halo CE: gbxmodel stores the inverse of each parent-relative bind rotation,
/// and `RenderModel::from_tag` corrects that historical on-disk convention.
pub(in crate::app) fn render_model_skeleton(
    tag: &TagFile,
) -> anyhow::Result<Vec<blam_tags::JmsNode>> {
    let model = RenderModel::from_tag(tag)?;
    Ok(render_model_skeleton_nodes(&model))
}

fn render_model_skeleton_nodes(model: &RenderModel) -> Vec<blam_tags::JmsNode> {
    let mut world: Vec<blam_tags::JmsNode> = Vec::with_capacity(model.nodes.len());
    for node in &model.nodes {
        let local_rotation = node.default_rotation.normalized();
        let local_translation = node.default_translation;
        let (rotation, translation) = if node.parent_node >= 0
            && let Some(parent) = world.get(node.parent_node as usize)
        {
            (
                (parent.rotation * local_rotation).normalized(),
                parent.translation
                    + (parent.rotation
                        * blam_tags::math::RealVector3d {
                            i: local_translation.x * 100.0,
                            j: local_translation.y * 100.0,
                            k: local_translation.z * 100.0,
                        }),
            )
        } else {
            (
                local_rotation,
                blam_tags::math::RealPoint3d {
                    x: local_translation.x * 100.0,
                    y: local_translation.y * 100.0,
                    z: local_translation.z * 100.0,
                },
            )
        };
        world.push(blam_tags::JmsNode {
            name: node.name.clone(),
            parent: node.parent_node,
            rotation,
            translation,
        });
    }
    world
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
pub(in crate::app) fn collision_jms_for_game(
    tag: &TagFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) -> anyhow::Result<JmsFile> {
    Ok(match blam_tags::game::Game::of(tag) {
        // CE stores each BSP under a node and indexes it by that node's
        // region/permutation. Keep that hierarchy instead of turning the node
        // name into a JMS material label, and compose the node-local points
        // against the same-path gbxmodel skeleton when one is available.
        blam_tags::game::Game::Halo1 => halo1_collision_jms(tag, skeleton)?,
        _ => match skeleton {
            Some(skeleton) => JmsFile::from_collision_model_with_skeleton(tag, skeleton)?,
            None => JmsFile::from_collision_model(tag)?,
        },
    })
}

#[derive(Clone, Copy)]
struct CeCollisionEdge {
    start_vertex: i32,
    end_vertex: i32,
    forward_edge: i32,
    reverse_edge: i32,
    left_surface: i32,
    right_surface: i32,
}

/// Reconstruct a CE collision JMS while retaining the tag's actual
/// region/permutation cells. `blam-tags`' generic CE exporter labels cells by
/// node name, which is useful for a flat geometry export but is not enough for
/// the preview: Model Setup then mistakes `bip01 pelvis` for a permutation and
/// region, and multiple BSPs on one node lose their permutation identity.
fn halo1_collision_jms(
    tag: &TagFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) -> anyhow::Result<JmsFile> {
    let root = tag.root();
    let regions = root
        .field_path("regions")
        .and_then(|field| field.as_block())
        .ok_or_else(|| anyhow::anyhow!("model_collision_geometry has no regions block"))?;
    let nodes = root
        .field_path("nodes")
        .and_then(|field| field.as_block())
        .ok_or_else(|| anyhow::anyhow!("model_collision_geometry has no nodes block"))?;
    let collision_materials = root
        .field_path("materials")
        .and_then(|field| field.as_block())
        .ok_or_else(|| anyhow::anyhow!("model_collision_geometry has no materials block"))?;

    // Reuse the shared reader for node names/parents. CE collision tags do not
    // carry bind transforms, so the returned identity transforms are exactly
    // what an unposed standalone export needs.
    let mut out = JmsFile::from_model_collision_geometry(tag)?;
    out.materials.clear();
    out.vertices.clear();
    out.triangles.clear();
    out.regions.clear();
    for region_index in 0..regions.len() {
        let name = regions
            .element(region_index)
            .and_then(|region| region.read_string("name"))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("region {region_index}"));
        out.regions.push(name);
    }

    let skeleton_map = skeleton.map(|target| {
        out.nodes
            .iter()
            .map(|source| target.iter().position(|node| node.name == source.name))
            .collect::<Vec<_>>()
    });
    if let (Some(skeleton), Some(map)) = (skeleton, skeleton_map.as_ref()) {
        for (node, target_index) in out.nodes.iter_mut().zip(map) {
            let Some(target) = target_index.and_then(|index| skeleton.get(index)) else {
                continue;
            };
            node.rotation = target.rotation;
            node.translation = target.translation;
        }
    }

    for node_index in 0..nodes.len() {
        let Some(node) = nodes.element(node_index) else {
            continue;
        };
        let region_index = node.read_int_any("region").unwrap_or(-1) as i32;
        let region = (region_index >= 0)
            .then(|| regions.element(region_index as usize))
            .flatten();
        let region_name = region
            .as_ref()
            .and_then(|region| region.read_string("name"))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "default".to_owned());
        let permutations = region
            .as_ref()
            .and_then(|region| region.field("permutations"))
            .and_then(|field| field.as_block());
        let Some(bsps) = node.field("bsps").and_then(|field| field.as_block()) else {
            continue;
        };

        for bsp_index in 0..bsps.len() {
            let permutation_name = permutations
                .as_ref()
                .and_then(|permutations| permutations.element(bsp_index))
                .and_then(|permutation| permutation.read_string("name"))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "default".to_owned());
            let Some(bsp) = bsps.element(bsp_index) else {
                continue;
            };
            append_halo1_collision_bsp(
                &bsp,
                node_index,
                region_index,
                &region_name,
                &permutation_name,
                &collision_materials,
                skeleton,
                skeleton_map.as_deref(),
                &mut out,
            );
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn append_halo1_collision_bsp(
    bsp: &TagStruct<'_>,
    node_index: usize,
    region_index: i32,
    region_name: &str,
    permutation_name: &str,
    collision_materials: &TagBlock<'_>,
    skeleton: Option<&[blam_tags::JmsNode]>,
    skeleton_map: Option<&[Option<usize>]>,
    out: &mut JmsFile,
) {
    let Some(surfaces) = bsp.field("surfaces").and_then(|field| field.as_block()) else {
        return;
    };
    let Some(edges) = bsp.field("edges").and_then(|field| field.as_block()) else {
        return;
    };
    let Some(vertices) = bsp.field("vertices").and_then(|field| field.as_block()) else {
        return;
    };

    let edge_rows = (0..edges.len())
        .filter_map(|index| edges.element(index))
        .map(|edge| CeCollisionEdge {
            start_vertex: edge.read_int_any("start vertex").unwrap_or(-1) as i32,
            end_vertex: edge.read_int_any("end vertex").unwrap_or(-1) as i32,
            forward_edge: edge.read_int_any("forward edge").unwrap_or(-1) as i32,
            reverse_edge: edge.read_int_any("reverse edge").unwrap_or(-1) as i32,
            left_surface: edge.read_int_any("left surface").unwrap_or(-1) as i32,
            right_surface: edge.read_int_any("right surface").unwrap_or(-1) as i32,
        })
        .collect::<Vec<_>>();
    let points = (0..vertices.len())
        .filter_map(|index| vertices.element(index))
        .map(
            |vertex| match vertex.field("point").and_then(|field| field.value()) {
                Some(TagFieldData::RealPoint3d(point)) => point,
                Some(TagFieldData::RealVector3d(vector)) => blam_tags::math::RealPoint3d {
                    x: vector.i,
                    y: vector.j,
                    z: vector.k,
                },
                _ => blam_tags::math::RealPoint3d::ZERO,
            },
        )
        .map(|point| {
            let local = point * 100.0;
            let target = skeleton_map
                .and_then(|map| map.get(node_index))
                .copied()
                .flatten();
            match target.and_then(|index| skeleton.and_then(|nodes| nodes.get(index))) {
                Some(node) => node.translation + node.rotation.rotate(local.as_vector()),
                None => local,
            }
        })
        .collect::<Vec<_>>();
    let target_node = skeleton_map
        .and_then(|map| map.get(node_index))
        .copied()
        .flatten()
        .unwrap_or(node_index) as i16;

    for surface_index in 0..surfaces.len() {
        let Some(surface) = surfaces.element(surface_index) else {
            continue;
        };
        let first_edge = surface.read_int_any("first edge").unwrap_or(-1) as i32;
        let polygon = walk_halo1_collision_surface(surface_index as i32, first_edge, &edge_rows);
        if polygon.len() < 3 {
            continue;
        }
        let source_material = surface.read_int_any("material").unwrap_or(-1) as i32;
        let shader_name = (source_material >= 0)
            .then(|| collision_materials.element(source_material as usize))
            .flatten()
            .and_then(|material| material.read_string("name"))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "default".to_owned());
        let label = format!("{permutation_name} {region_name}");
        let material_index = out
            .materials
            .iter()
            .position(|material| material.name == shader_name && material.material_name == label)
            .unwrap_or_else(|| {
                out.materials.push(blam_tags::JmsMaterial {
                    name: shader_name,
                    material_name: label,
                });
                out.materials.len() - 1
            }) as i32;

        for corner in 1..polygon.len() - 1 {
            let triangle = [polygon[0], polygon[corner], polygon[corner + 1]];
            let base = out.vertices.len() as u32;
            for point_index in triangle {
                let position = points
                    .get(point_index as usize)
                    .copied()
                    .unwrap_or(blam_tags::math::RealPoint3d::ZERO);
                out.vertices.push(blam_tags::JmsVertex {
                    position,
                    normal: blam_tags::math::RealVector3d {
                        i: 0.0,
                        j: 0.0,
                        k: 1.0,
                    },
                    tangent: None,
                    binormal: None,
                    node_sets: vec![(target_node, 1.0)],
                    uvs: vec![blam_tags::math::RealPoint2d::ZERO],
                    color: None,
                });
            }
            out.triangles.push(blam_tags::JmsTriangle {
                material: material_index,
                v: [base, base + 1, base + 2],
                region: region_index.max(0),
            });
        }
    }
}

fn walk_halo1_collision_surface(
    surface: i32,
    first_edge: i32,
    edges: &[CeCollisionEdge],
) -> Vec<i32> {
    if first_edge < 0 {
        return Vec::new();
    }
    let mut polygon = Vec::new();
    let mut edge_index = first_edge;
    for _ in 0..=edges.len() {
        let Some(edge) = edges.get(edge_index as usize) else {
            break;
        };
        let next = if edge.left_surface == surface {
            polygon.push(edge.start_vertex);
            edge.forward_edge
        } else if edge.right_surface == surface {
            polygon.push(edge.end_vertex);
            edge.reverse_edge
        } else {
            break;
        };
        if next == first_edge {
            break;
        }
        if next < 0 || next == edge_index {
            return Vec::new();
        }
        edge_index = next;
    }
    polygon
}

/// Halo 2 physics models store their shapes flat; Halo 3 and later nest them
/// behind rigid-body shape references.
pub(in crate::app) fn physics_jms_for_game(
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
    target: Game,
) -> anyhow::Result<String> {
    let mut notes = Vec::new();
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
                        // Campaign Evolved runs on the Halo Reach engine, whose
                        // toolset uses the Halo 3-era JMS form — N-influence
                        // vertices with region/permutation in the material slot
                        // names — so that is what this JMS is built in.
                        let written = write_jms_for_target(
                            &jms,
                            Game::Halo3,
                            target,
                            &output.join("render"),
                            &format!("{stem}.render.jms"),
                            &output.join("models"),
                            &mut notes,
                        )?;
                        emitted.push(format!("render {}", display_paths(&written)));
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
        let game = Game::of(tag);
        if matches!(game, Game::Halo3) && render_model_prefers_ass(tag) {
            let version = ass_version_for(game, target, &mut notes);
            let ass = AssFile::from_render_model(tag)?;
            fs::create_dir_all(&render_dir)?;
            let path = render_dir.join(format!("{stem}.render.ASS"));
            let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
            ass.write_version(&mut file, version)?;
            emitted.push(format!("render {}", path.display()));
        } else if let Some(jms) = render_jms_for_skeleton.as_ref() {
            let written = write_jms_for_target(
                jms,
                game,
                target,
                &render_dir,
                &format!("{stem}.render.jms"),
                &output.join("models"),
                &mut notes,
            )?;
            emitted.push(format!("render {}", display_paths(&written)));
        }
    }

    match collision_ref.as_deref() {
        Some(reference) => {
            match load_referenced_tag_from_source(source, reference, "collision_model", b"coll") {
                Ok(tag) => {
                    let mut jms = collision_jms_for_game(&tag, skeleton)?;
                    if let Some(skel) = campaign_evolved_skeleton.as_ref() {
                        jms.reorient_for_campaign_evolved(skel);
                    }
                    let written = write_jms_for_target(
                        &jms,
                        Game::of(&tag),
                        target,
                        &output.join("collision"),
                        &format!("{stem}.collision.jms"),
                        &output.join("physics"),
                        &mut notes,
                    )?;
                    emitted.push(format!("collision {}", display_paths(&written)));
                }
                Err(error) => skipped.push(format!("collision: {error}")),
            }
        }
        None => skipped.push("collision: no collision_model reference".to_owned()),
    }

    match physics_ref.as_deref() {
        Some(reference) => {
            match load_referenced_tag_from_source(source, reference, "physics_model", b"phmo") {
                Ok(_) if target == Game::Halo1 => {
                    skipped.push("physics: Halo CE has no physics model".to_owned());
                }
                Ok(tag) => {
                    let physics_dir = output.join("physics");
                    fs::create_dir_all(&physics_dir)?;
                    let mut jms = physics_jms_for_game(&tag, skeleton)?;
                    if let Some(skel) = campaign_evolved_skeleton.as_ref() {
                        jms.reorient_for_campaign_evolved(skel);
                    }
                    let path = physics_dir.join(format!("{stem}.physics.jms"));
                    let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
                    jms.write(&mut file, target.jms_version())?;
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
    Ok(with_notes(message, &notes))
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
                key: file_entry_key(&path),
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
fn animation_graph_owner(source: &TagSource, entry: &TagEntry, jmad: &TagFile) -> Option<TagFile> {
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
    target: Game,
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
        blam_tags::extract::animation::animations_to_dir(input, &resolver, output, &stem, target)?;
    let mut message = format!(
        "Extracted {} animation(s) from {} into {} (JMA {})",
        summary.written,
        entry.display_path,
        output.display(),
        target.jma_version(),
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
    target: Game,
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
    if target != Game::of(&tag) {
        message.push_str(" — level geometry is written in this game's own form");
    }
    Ok(message)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod extract_targets_tests;
#[cfg(test)]
mod particle_model_extract_menu_tests;
