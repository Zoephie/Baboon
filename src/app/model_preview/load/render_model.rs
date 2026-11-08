//! Render models and particle models as preview geometry.

use super::*;

/// `particle_model` carries its geometry inline with no wrapper, but
/// it is not a render_model: no regions, no permutations, no
/// materials, no nodes. Its objects are the entries of the JMI the
/// artist imported, so each becomes a preview region and gets its own
/// toggle in the region list.
pub(super) fn load_particle_model(model_tag: &TagFile, entry: &TagEntry) -> Result<ModelPreviewData, String> {
    let stem = preview_tag_stem(entry);
    let preview = build_particle_model_preview(model_tag, &stem)?;
    if preview.batches.is_empty() {
        return Err("This particle_model has no previewable geometry.".to_owned());
    }
    Ok(model_preview_data(
        entry.key.clone(),
        entry.display_path.clone(),
        preview,
        Vec::new(),
    ))
}

/// Halo CE `gbxmodel` (mod2) and a bare `render_model` (mode) ARE the
/// render geometry — there is no `.model` (hlmt) wrapper carrying a
/// "render model" reference, so preview the tag itself.
pub(super) fn load_render_tag(model_tag: &TagFile, entry: &TagEntry) -> Result<ModelPreviewData, String> {
    let preview = build_render_preview(model_tag)?;
    if preview.batches.is_empty() {
        return Err("This render tag has no previewable draw batches.".to_owned());
    }
    Ok(model_preview_data(
        entry.key.clone(),
        entry.display_path.clone(),
        preview,
        Vec::new(),
    ))
}

/// The `.model` an object stands for. `None` for any other tag, which is its
/// own model, and for a Halo CE object, which has no `.model` to name.
pub(in crate::app::model_preview) fn preview_model_tag(source: &TagSource, tag: &TagFile) -> Result<Option<TagFile>, String> {
    if !is_object_family_group(tag.header.group_tag, source.game())
        || blam_tags::game::Game::of(tag) == blam_tags::game::Game::Halo1
    {
        return Ok(None);
    }
    let reference = object_reference(tag, "model").ok_or("This object references no model.")?;
    load_referenced_tag_from_source(source, &reference, "model", b"hlmt")
        .map(Some)
        .map_err(|error| format!("Could not load {reference}.model: {error}"))
}

/// Tag basename, used to name gen3 particle objects (`pmdf` stores no
/// object names — see `blam_tags::particle_model`). Falls back to the
/// entry key when the display path has no usable leaf.
pub(super) fn preview_tag_stem(entry: &TagEntry) -> String {
    Some(tag_path::name(&entry.display_path))
        .map(|leaf| leaf.split('.').next().unwrap_or(leaf))
        .filter(|stem| !stem.is_empty())
        .unwrap_or("particle_model")
        .to_owned()
}

/// Build preview geometry from a `particle_model` (`pmdf` or Halo 2
/// `PRTM`).
///
/// blam-tags does the decode — splitting the merged strip at the
/// `m_gpu_data/m_variants` boundaries (gen3) or the `models[]` ranges
/// (Halo 2), decompressing positions through the compression bounds, and
/// compacting each object to its own vertex set. Each object lands as a
/// one-permutation region so the existing region list doubles as an
/// object toggle, which is the closest thing the tag has to structure.
pub(in crate::app::model_preview) fn build_particle_model_preview(
    tag: &TagFile,
    stem: &str,
) -> Result<RenderModelPreview, String> {
    let meshes = blam_tags::particle_model_meshes(tag, stem).map_err(|e| e.to_string())?;

    let mut preview = RenderModelPreview {
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };

    for mesh in &meshes {
        let Some(vertex_base) =
            VertexBase::of(preview.vertices.len(), mesh.vertices.len(), Overflow::RefusesAppend)
        else {
            continue;
        };

        preview.vertices.reserve(mesh.vertices.len());
        for vertex in &mesh.vertices {
            let position = [vertex.position.x, vertex.position.y, vertex.position.z];
            expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
            preview.vertices.push(RenderModelPreviewVertex {
                position,
                normal: [vertex.normal.i, vertex.normal.j, vertex.normal.k],
                // Particle models carry no tangent frame, and this path has no
                // materials to texture with — the defaults leave it unshaded.
                ..Default::default()
            });
        }

        let Ok(index_start) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        preview
            .indices
            .extend(mesh.indices.iter().map(|&index| vertex_base.offset(index)));
        let Ok(index_end) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        let index_count = index_end - index_start;
        if index_count == 0 {
            continue;
        }

        preview.regions.push(RenderModelPreviewRegion {
            name: mesh.name.clone(),
            permutations: vec![PARTICLE_OBJECT_PERMUTATION.to_owned()],
        });
        preview.batches.push(RenderModelPreviewBatch {
            region_name: mesh.name.clone(),
            permutation_name: PARTICLE_OBJECT_PERMUTATION.to_owned(),
            material_index: 0,
            index_start,
            index_count,
            flat_color: None,
            layer: ModelPreviewLayer::Render,
            sort_position: None,
        });
    }

    if preview.vertices.is_empty() {
        preview.bounds_min = [0.0; 3];
        preview.bounds_max = [0.0; 3];
    }
    Ok(preview)
}

/// A particle object has no permutations; the region list still needs a
/// selectable entry per region, so every object gets this single one.
pub(super) const PARTICLE_OBJECT_PERMUTATION: &str = "default";

/// Build preview geometry from a render-geometry tag — a `render_model`
/// (`mode`), a Halo CE `gbxmodel` (`mod2`), or a Halo 2 `render_model`.
///
/// One tag→geometry path for every engine: blam-tags' `RenderModel::from_tag`
/// / `derive_render_meshes` game-dispatch (H3 reads `render geometry`, H2 the
/// `sections`, Halo CE the gbxmodel `geometries`), so batches carry the render
/// model's own region/permutation names and stay in sync with the variant
/// selection. JMS is export-only — never used for rendering.
pub(in crate::app) fn build_render_preview(
    render_tag: &TagFile,
) -> Result<RenderModelPreview, String> {
    let mut render_model = RenderModel::from_tag(render_tag).map_err(|error| error.to_string())?;
    let render_meshes =
        RenderModel::derive_render_meshes(render_tag).map_err(|error| error.to_string())?;
    // `matrix4x3_rotation_from_quaternion` builds a CE marker from the conjugate
    // of what the tag stores, as it does a node, and `RenderModel` only turns
    // the nodes.
    if blam_tags::game::Game::of(render_tag) == blam_tags::game::Game::Halo1 {
        for marker in render_model.marker_groups.iter_mut().flat_map(|group| &mut group.markers) {
            marker.rotation = marker.rotation.conjugate();
        }
    }
    let mut preview = render_model_to_preview(&render_model, &render_meshes);
    // Only Halo CE's transparents sort by where their parts are; the later
    // games' sorting positions belong to a renderer the preview does not run.
    if blam_tags::game::Game::of(render_tag) != blam_tags::game::Game::Halo1 {
        preview.batches.iter_mut().for_each(|batch| batch.sort_position = None);
    }
    // A gbxmodel's `shaders` run parallel to its materials.
    if let Some(shaders) = render_tag.root().field("shaders").and_then(|field| field.as_block()) {
        for (index, material) in preview.materials.iter_mut().enumerate() {
            let permutation = shaders.element(index).and_then(|shader| shader.read_int_any("permutation"));
            material.permutation = permutation.unwrap_or(0).max(0) as usize;
        }
    }
    preview.h2.prt = read_h2_prt(render_tag, &render_model, &render_meshes, &h2_prt_lights())
        .filter(|prt| prt.vertices.len() == preview.vertices.len());
    // Instances draw `static_lighting_sh`, with no transfer of their own.
    let instance_vertices = preview.instances.vertex_count;
    preview.gen3.prt = super::super::gen3::prt::read_h3_prt(render_tag, &render_model, &render_meshes)
        .map(|mut prt| {
            prt.pad(instance_vertices);
            prt
        })
        .filter(|prt| prt.vertices.len() == preview.vertices.len());
    append_model_errors(render_tag, &mut preview, ModelPreviewLayer::Render);
    Ok(preview)
}

/// Build flat preview geometry with draw batches grouped by region and
/// permutation. Ported from blam-tags so the GUI owns its preview type; the
/// render meshes are derived separately via `RenderModel::derive_render_meshes`.
pub(in crate::app::model_preview) fn render_model_to_preview(
    model: &RenderModel,
    render_meshes: &[RenderMesh],
) -> RenderModelPreview {
    let node_world = preview_node_world_transforms(&model.nodes);
    let mut preview = RenderModelPreview {
        regions: model
            .regions
            .iter()
            .map(|region| RenderModelPreviewRegion {
                name: region.name.clone(),
                permutations: region
                    .permutations
                    .iter()
                    .map(|permutation| permutation.name.clone())
                    .collect(),
            })
            .collect(),
        // Index-aligned with `RenderMeshPart::material_index`, which is what
        // the batches carry, so a batch's shader is `materials[index]`.
        materials: model
            .materials
            .iter()
            .map(|material| RenderModelPreviewMaterial {
                shader_path: material.render_method.clone(),
                shader_group: material.render_method_group,
                overlay: false, shield: None,
                permutation: 0,
                object: 0,
            })
            .collect(),
        nodes: preview_skeleton_nodes(&model.nodes),
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };

    for (region, permutation, mesh_index) in preview_mesh_order(model, render_meshes.len()) {
        let (region, permutation) = (&model.regions[region], &model.regions[region].permutations[permutation]);
        append_render_mesh_to_preview(&mut preview, &render_meshes[mesh_index], &region.name, &permutation.name);
    }

    append_instances_to_preview(&mut preview, model, render_meshes);

    if preview.vertices.is_empty() {
        preview.bounds_min = [0.0; 3];
        preview.bounds_max = [0.0; 3];
    }

    for group in &model.marker_groups {
        for marker in &group.markers {
            preview.markers.push(RenderModelPreviewMarker {
                name: group.name.clone(),
                node_index: marker.node_index as i16,
                position: transform_preview_marker_position(marker, &node_world),
                scale: marker.scale,
                axes: transform_preview_marker_axes(marker, &node_world),
                only_under: marker_permutation(model, marker),
            });
        }
    }

    preview
}

fn marker_permutation(model: &RenderModel, marker: &blam_tags::render_model::Marker) -> Option<(String, String)> {
    let region = model.regions.get(usize::try_from(marker.region_index).ok()?)?;
    let permutation = region.permutations.get(usize::try_from(marker.permutation_index).ok()?)?;
    Some((region.name.clone(), permutation.name.clone()))
}

/// The meshes a preview holds, in the order its vertices lie: by region and
/// permutation, which is not the tag's. Anything parallel to the vertices has
/// to walk this too. Each is a region, permutation and mesh index.
pub(in crate::app::model_preview) fn preview_mesh_order(model: &RenderModel, mesh_count: usize) -> Vec<(usize, usize, usize)> {
    let mut order = Vec::new();
    for (region_index, region) in model.regions.iter().enumerate() {
        for (permutation_index, permutation) in region.permutations.iter().enumerate() {
            // NONE, whatever its count says: a permutation with no geometry.
            let Ok(first_mesh) = usize::try_from(permutation.mesh_index) else {
                continue;
            };
            let count = permutation.mesh_count.max(0) as usize;
            for mesh_index in first_mesh..first_mesh.saturating_add(count) {
                if mesh_index < mesh_count {
                    order.push((region_index, permutation_index, mesh_index));
                }
            }
        }
    }
    order
}

/// `render_object_context_instances`: each placement draws its subpart of the
/// instance mesh, carried from there into the model by its own transform and
/// riding its node from then on. A mirrored one winds backward.
fn append_instances_to_preview(
    preview: &mut RenderModelPreview,
    model: &RenderModel,
    render_meshes: &[RenderMesh],
) {
    let instances = &mut preview.instances;
    instances.empty_regions = model
        .regions
        .iter()
        .filter(|region| region.permutations.iter().all(|permutation| permutation.mesh_index < 0))
        .map(|region| region.name.clone())
        .collect();
    for region in &model.regions {
        for permutation in &region.permutations {
            let mask = permutation.instance_mask.iter().rev().fold(0u128, |mask, word| mask << 32 | *word as u128);
            instances.masks.insert((region.name.clone(), permutation.name.clone()), mask);
        }
    }
    instances.names = model.instance_placements.iter().map(|placement| placement.name.clone()).collect();
    let Some(mesh) = model.instance_mesh_index.and_then(|mesh| render_meshes.get(mesh)) else {
        return;
    };
    let first_vertex = preview.vertices.len();
    for (instance, (placement, subpart)) in model.instance_placements.iter().zip(&mesh.subparts).enumerate() {
        let (Some(part), Ok(node)) = (mesh.parts.get(subpart.part_index as usize), usize::try_from(placement.node_index)) else {
            continue;
        };
        let start = subpart.index_start as usize;
        let Some(source) = mesh.indices.get(start..start + subpart.index_count as usize) else {
            continue;
        };
        let scale = placement.scale;
        let axes = [placement.forward, placement.left, placement.up].map(vector3_to_array);
        let turned = |[x, y, z]: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|axis| axes[0][axis] * x + axes[1][axis] * y + axes[2][axis] * z)
        };
        let direction = |vector: [f32; 3]| turned(vector).map(|value| value * scale.signum());
        let origin = point3_to_array(placement.position);
        // A triangle naming a vertex the mesh lacks is dropped whole, so the
        // triangles after it keep their corners.
        let triangles: Vec<&[u32]> = source
            .chunks_exact(3)
            .filter(|corners| corners.iter().all(|&index| (index as usize) < mesh.vertices.len()))
            .collect();
        // Each vertex the triangles name is added once.
        let adding = triangles.iter().copied().flatten().collect::<std::collections::HashSet<_>>().len();
        let Some(vertex_base) =
            common::geometry::VertexBase::of(preview.vertices.len(), adding, common::geometry::Overflow::RefusesAppend)
        else {
            continue;
        };
        let mut remapped = HashMap::new();
        let mut indices = Vec::with_capacity(source.len());
        for &index in triangles.into_iter().flatten() {
            let next = vertex_base.offset(remapped.len() as u32);
            let at = *remapped.entry(index).or_insert(next);
            if at == next {
                let vertex = &mesh.vertices[index as usize];
                let placed = turned(point3_to_array(vertex.position));
                let position = std::array::from_fn(|axis| placed[axis] * scale + origin[axis]);
                expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
                preview.vertices.push(RenderModelPreviewVertex {
                    position,
                    normal: direction(vector3_to_array(vertex.normal)),
                    texcoord: [vertex.texcoord.x, vertex.texcoord.y],
                    tangent: direction(vector3_to_array(vertex.tangent)),
                    binormal: direction(vector3_to_array(vertex.binormal)),
                    node_indices: [node as f32, 0.0, 0.0, 0.0],
                    node_weights: [1.0, 0.0, 0.0, 0.0],
                });
            }
            indices.push(at);
        }
        if scale < 0.0 {
            indices.chunks_exact_mut(3).for_each(|triangle| triangle.swap(1, 2));
        }
        let index_start = preview.indices.len() as u32;
        preview.indices.extend_from_slice(&indices);
        instances_batch(preview, instance, part.material_index, index_start, indices.len() as u32);
    }
    preview.instances.vertex_count = preview.vertices.len() - first_vertex;
}

fn instances_batch(
    preview: &mut RenderModelPreview,
    instance: usize,
    material_index: u16,
    index_start: u32,
    index_count: u32,
) {
    preview.instances.batches.insert(preview.batches.len(), instance);
    preview.batches.push(RenderModelPreviewBatch {
        region_name: String::new(),
        permutation_name: String::new(),
        material_index,
        index_start,
        index_count,
        flat_color: None,
        layer: ModelPreviewLayer::Render,
        sort_position: None,
    });
}

/// Append one render mesh without flattening every triangle into three unique
/// vertices. The GPU renderer consumes indexed `u32` geometry directly, so
/// retaining the source topology is both faithful and dramatically smaller for
/// dense Campaign Evolved Nanite previews.
fn append_render_mesh_to_preview(
    preview: &mut RenderModelPreview,
    mesh: &RenderMesh,
    region_name: &str,
    permutation_name: &str,
) {
    let Some(vertex_base) =
        common::geometry::VertexBase::of(preview.vertices.len(), mesh.vertices.len(), common::geometry::Overflow::DropsTriangle)
    else {
        return;
    };

    preview.vertices.reserve(mesh.vertices.len());
    for vertex in &mesh.vertices {
        let position = point3_to_array(vertex.position);
        expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
        preview.vertices.push(RenderModelPreviewVertex {
            position,
            normal: vector3_to_array(vertex.normal),
            // Already decompressed: `derive_render_meshes` applies the mesh's
            // compression bounds, so these are real UVs rather than the packed
            // [0,1] the buffer stores for the UShort2N formats.
            texcoord: [vertex.texcoord.x, vertex.texcoord.y],
            tangent: vector3_to_array(vertex.tangent),
            binormal: vector3_to_array(vertex.binormal),
            // Global skeleton indices — blam-tags resolves the per-mesh
            // palettes, and rigid meshes arrive as a single full-weight
            // influence, so animation needs no rigid special case.
            node_indices: vertex.node_indices.map(|index| index as f32),
            node_weights: vertex.node_weights,
        });
    }

    for part in &mesh.parts {
        let source_start = part.index_start as usize;
        let source_end = source_start
            .saturating_add(part.index_count as usize)
            .min(mesh.indices.len());
        let Some(source_indices) = mesh.indices.get(source_start..source_end) else {
            continue;
        };
        let Ok(index_start) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        for triangle in source_indices.chunks_exact(3) {
            let convert = |index: u32| {
                ((index as usize) < mesh.vertices.len())
                    .then(|| vertex_base.checked_offset(index))
                    .flatten()
            };
            if let (Some(a), Some(b), Some(c)) = (
                convert(triangle[0]),
                convert(triangle[1]),
                convert(triangle[2]),
            ) {
                preview.indices.extend_from_slice(&[a, b, c]);
            }
        }
        let Ok(index_end) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        let index_count = index_end - index_start;
        if index_count > 0 {
            let sort_position =
                part.sort_position.map(|at| (point3_to_array(at.position), usize::from(at.node_indices[0])));
            preview.batches.push(RenderModelPreviewBatch {
                region_name: region_name.to_owned(),
                permutation_name: permutation_name.to_owned(),
                material_index: part.material_index,
                index_start,
                index_count,
                flat_color: None,
                layer: ModelPreviewLayer::Render,
                sort_position,
            });
        }
    }
}

/// The preview's copy of the skeleton: parent-local bind pose plus the
/// inverse of the accumulated bind world transform, ready for skinning.
pub(in crate::app::model_preview) fn preview_skeleton_nodes(nodes: &[Node]) -> Vec<RenderModelPreviewNode> {
    let world = preview_node_world_transforms(nodes);
    nodes
        .iter()
        .zip(&world)
        .map(|(node, (world_rot, world_trans))| {
            let bind_world =
                blam_tags::math::Matrix4::from_loc_rot_scale(*world_trans, *world_rot, 1.0);
            let inverse = bind_world.inverse();
            RenderModelPreviewNode {
                name: node.name.clone(),
                parent: node.parent_node,
                bind_rotation: node.default_rotation.normalized().to_array(),
                bind_translation: point3_to_array(node.default_translation),
                inverse_bind: [inverse.m[0], inverse.m[1], inverse.m[2]],
            }
        })
        .collect()
}

pub(in crate::app::model_preview) fn preview_node_world_transforms(nodes: &[Node]) -> Vec<(RealQuaternion, RealPoint3d)> {
    let mut world: Vec<(RealQuaternion, RealPoint3d)> = Vec::with_capacity(nodes.len());
    for node in nodes {
        let local_rot = node.default_rotation.normalized();
        let local_trans = node.default_translation;
        if node.parent_node >= 0
            && let Some((parent_rot, parent_trans)) = world.get(node.parent_node as usize).copied()
        {
            let rot = (parent_rot * local_rot).normalized();
            let trans = parent_trans + (parent_rot * local_trans.as_vector());
            world.push((rot, trans));
            continue;
        }
        world.push((local_rot, local_trans));
    }
    world
}

pub(in crate::app::model_preview) fn transform_preview_marker_position(
    marker: &Marker,
    node_world: &[(RealQuaternion, RealPoint3d)],
) -> [f32; 3] {
    let local = marker.translation;
    let world = if marker.node_index >= 0 {
        node_world
            .get(marker.node_index as usize)
            .map(|(rot, trans)| *trans + (*rot * local.as_vector()))
            .unwrap_or(local)
    } else {
        local
    };
    point3_to_array(world)
}

pub(in crate::app::model_preview) fn transform_preview_marker_axes(
    marker: &Marker,
    node_world: &[(RealQuaternion, RealPoint3d)],
) -> [[f32; 3]; 3] {
    let local_rot = marker.rotation.normalized();
    let world_rot = if marker.node_index >= 0 {
        node_world
            .get(marker.node_index as usize)
            .map(|(rot, _)| (*rot * local_rot).normalized())
            .unwrap_or(local_rot)
    } else {
        local_rot
    };
    [
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 1.0,
                    j: 0.0,
                    k: 0.0,
                },
        ),
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 0.0,
                    j: 1.0,
                    k: 0.0,
                },
        ),
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 0.0,
                    j: 0.0,
                    k: 1.0,
                },
        ),
    ]
}

pub(in crate::app::model_preview) fn point3_to_array(p: RealPoint3d) -> [f32; 3] {
    [p.x, p.y, p.z]
}

pub(in crate::app::model_preview) fn vector3_to_array(v: RealVector3d) -> [f32; 3] {
    [v.i, v.j, v.k]
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::math::{RealPoint2d, RealPoint3d, RealVector3d};
    use blam_tags::render_model::{GeometryPartType, RenderMeshPart, RenderVertex};

    #[test]
    fn indexed_preview_keeps_shared_vertices_and_part_batches() {
        let vertex = |x, y| RenderVertex {
            position: RealPoint3d { x, y, z: 0.0 },
            texcoord: RealPoint2d { x, y },
            normal: RealVector3d {
                i: 0.0,
                j: 0.0,
                k: 1.0,
            },
            tangent: RealVector3d::ZERO,
            binormal: RealVector3d::ZERO,
            node_indices: [0; 4],
            node_weights: [0.0; 4],
            lightmap_texcoord: RealPoint2d { x: 0.0, y: 0.0 },
            vert_color: RealVector3d::ZERO,
        };
        let part = |index_start| RenderMeshPart {
            material_index: index_start as u16 / 3,
            index_start,
            index_count: 3,
            part_type: GeometryPartType::OpaqueNonShadowing,
            transparent_sorting_index: -1,
            sort_position: None,
        };
        let mesh = RenderMesh {
            vertices: vec![
                vertex(0.0, 0.0),
                vertex(1.0, 0.0),
                vertex(1.0, 1.0),
                vertex(0.0, 1.0),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            parts: vec![part(0), part(3)],
            subparts: Vec::new(),
            rigid_node_index: None,
            water_data: None,
            prt_vertex_type: Default::default(),
            has_prt_vertex_stream: false,
            prt_ambient_stream: Vec::new(),
            has_vertex_color: false,
            use_region_index_for_sorting: false,
            has_lightmap_uvs: false,
        };
        let mut preview = RenderModelPreview {
            bounds_min: [f32::INFINITY; 3],
            bounds_max: [f32::NEG_INFINITY; 3],
            ..Default::default()
        };

        append_render_mesh_to_preview(&mut preview, &mesh, "body", "default");

        assert_eq!(preview.vertices.len(), 4);
        assert_eq!(preview.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(preview.batches.len(), 2);
        assert_eq!(
            (
                preview.batches[0].index_start,
                preview.batches[0].index_count
            ),
            (0, 3)
        );
        assert_eq!(
            (
                preview.batches[1].index_start,
                preview.batches[1].index_count
            ),
            (3, 3)
        );
    }

    #[test]
    fn a_permutation_of_mesh_none_draws_nothing_whatever_its_count() {
        let permutation = |mesh_index, mesh_count| blam_tags::Permutation {
            name: String::new(),
            mesh_index,
            mesh_count,
            instance_mask: [0; 4],
        };
        let region = |permutations| blam_tags::Region { name: String::new(), permutations };
        let model = RenderModel {
            regions: vec![
                region(vec![permutation(0, 1)]),
                region(vec![permutation(-1, 1)]),
                region(vec![permutation(1, 2)]),
            ],
            ..Default::default()
        };
        assert_eq!(preview_mesh_order(&model, 3), [(0, 0, 0), (2, 0, 1), (2, 0, 2)]);
    }
}
