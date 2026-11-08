//! Structure BSPs as preview geometry: the native decode or the JMS/ASS scene, its portals and collision.

use super::*;

/// A structure BSP's geometry, per engine: H3-family and H2 through the ASS
/// scene builders, H1 through its JMS render/collision extractors.
/// `render_only` drops the collision/portal/weather layers — the scenario
/// composite wants just the visible world.
pub(in crate::app::model_preview) fn build_sbsp_preview(
    tag: &TagFile,
    render_only: bool,
) -> Result<RenderModelPreview, String> {
    let mut preview = match blam_tags::game::Game::of(tag) {
        blam_tags::game::Game::Halo1 => {
            let mut preview = empty_preview();
            match JmsFile::from_scenario_structure_bsp_ce(tag) {
                Ok(jms) => append_jms_triangles(&mut preview, &jms, "render", None, true),
                Err(error) => return Err(error.to_string()),
            }
            if !render_only && let Ok(jms) = JmsFile::from_scenario_structure_bsp_ce_collision(tag)
            {
                append_jms_triangles(
                    &mut preview,
                    &jms,
                    COLLISION_REGION,
                    Some(COLLISION_COLOR),
                    false,
                );
            }
            preview
        }
        blam_tags::game::Game::Halo2 => ass_to_preview(
            &AssFile::from_scenario_structure_bsp_h2(tag).map_err(|error| error.to_string())?,
            render_only,
        ),
        blam_tags::game::Game::Halo3 => {
            // Campaign Evolved's Blam/Unreal hybrid BSPs ship no render
            // geometry at all (`per mesh temporary` is empty; Unreal owns
            // everything rendered) — the ASS builder already knows to source
            // their content from collision, so those stay on that path.
            let has_render_geometry = tag
                .root()
                .field_path("render geometry/per mesh temporary")
                .and_then(|field| field.as_block())
                .is_some_and(|block| !block.is_empty());
            if has_render_geometry {
                build_sbsp_preview_h3(tag, render_only)?
            } else {
                ass_to_preview(
                    &AssFile::from_scenario_structure_bsp(tag)
                        .map_err(|error| error.to_string())?,
                    render_only,
                )
            }
        }
    };
    append_model_errors(tag, &mut preview, ModelPreviewLayer::Render);
    finish_preview(preview, "structure BSP")
}

/// The H3-family structure BSP, decoded natively so the render layer keeps
/// what the ASS text format throws away: real UVs, the authored tangent
/// frames, and the `materials` block's shader references. That is what lets
/// the existing shader→texture pipeline shade a BSP exactly like a model —
/// diffuse, normal maps, and alpha-test cutouts included.
fn build_sbsp_preview_h3(tag: &TagFile, render_only: bool) -> Result<RenderModelPreview, String> {
    let root = tag.root();
    let mut preview = empty_preview();
    preview.materials = sbsp_preview_materials(&root);
    if preview.materials.is_empty() {
        preview
            .materials
            .push(RenderModelPreviewMaterial::default());
    }

    let clusters = root
        .field_path("clusters")
        .and_then(|field| field.as_block())
        .ok_or("This structure BSP has no clusters block.")?;
    let defs = root
        .field_path(
            "resource interface/raw_resources[0]/raw_items/instanced geometries definitions",
        )
        .and_then(|field| field.as_block());
    let instances = root
        .field_path("instanced geometry instances")
        .and_then(|field| field.as_block());

    // Which compression bounds decode which mesh. Cluster meshes are stored
    // in world units (identity); each instanced-geometry definition names its
    // own `compression index`. An odd number of negative-span axes flips the
    // unpacker's winding, which the append below undoes per triangle.
    let mut mesh_compression: HashMap<usize, usize> = HashMap::new();
    if let Some(defs) = &defs {
        for def in defs.iter() {
            let mesh_index = def.read_int_any("mesh index").unwrap_or(-1);
            let compression = def.read_int_any("compression index").unwrap_or(0).max(0) as usize;
            if mesh_index >= 0 {
                mesh_compression.insert(mesh_index as usize, compression);
            }
        }
    }
    let meshes = extract_sbsp_render_geometry_meshes(&root, |mesh_index| {
        match mesh_compression.get(&mesh_index) {
            Some(&compression) => read_compression_bounds_at(&root, compression),
            None => CompressionBounds::identity(),
        }
    })
    .map_err(|error| error.to_string())?;

    // Everything lands in one `render` region, batched per material at the
    // end: a level is thousands of cluster/instance parts, and one draw call
    // per part would be the slow way to spend a frame.
    let mut render_indices: Vec<Vec<u32>> = vec![Vec::new(); preview.materials.len()];
    let identity = InstancePlacement::identity();
    for cluster in clusters.iter() {
        let mesh_index = cluster.read_int_any("mesh index").unwrap_or(-1);
        let Some(mesh) = usize::try_from(mesh_index).ok().and_then(|i| meshes.get(i)) else {
            continue;
        };
        append_sbsp_mesh(&mut preview, &mut render_indices, mesh, &identity, false);
    }
    if let (Some(defs), Some(instances)) = (&defs, &instances) {
        for instance in instances.iter() {
            let def_index = instance.read_int_any("instance definition").unwrap_or(-1);
            let Some(def) = usize::try_from(def_index)
                .ok()
                .and_then(|i| defs.element(i))
            else {
                continue;
            };
            let mesh_index = def.read_int_any("mesh index").unwrap_or(-1);
            let Some(mesh) = usize::try_from(mesh_index).ok().and_then(|i| meshes.get(i)) else {
                continue;
            };
            let flip = mesh_compression
                .get(&(mesh_index as usize))
                .map(|&compression| {
                    bounds_axis_flip(&read_compression_bounds_at(&root, compression))
                })
                .unwrap_or(false);
            let placement = InstancePlacement {
                forward: vector(&instance.read_vec3("forward")),
                left: vector(&instance.read_vec3("left")),
                up: vector(&instance.read_vec3("up")),
                position: point(&instance.read_point3d("position")),
                scale: instance.read_real("scale").unwrap_or(1.0),
            };
            append_sbsp_mesh(&mut preview, &mut render_indices, mesh, &placement, flip);
        }
    }
    for (material_index, indices) in render_indices.into_iter().enumerate() {
        if indices.is_empty() {
            continue;
        }
        let index_start = preview.indices.len() as u32;
        let index_count = indices.len() as u32;
        preview.indices.extend(indices);
        preview.batches.push(RenderModelPreviewBatch {
            region_name: "render".to_owned(),
            permutation_name: "default".to_owned(),
            material_index: material_index.min(u16::MAX as usize) as u16,
            index_start,
            index_count,
            flat_color: None,
            layer: ModelPreviewLayer::Render,
            sort_position: None,
        });
    }
    if !preview.batches.is_empty() {
        ensure_preview_region(&mut preview, "render");
    }

    if !render_only {
        append_sbsp_portals(&root, &mut preview);
        append_sbsp_collision(&root, &mut preview);
    }
    Ok(preview)
}

/// The sbsp `materials` block's shader references, index-aligned with the
/// mesh parts' `material_index` — the same contract the render_model preview
/// keeps, so `resolve_model_textures` needs nothing new.
fn sbsp_preview_materials(root: &TagStruct<'_>) -> Vec<RenderModelPreviewMaterial> {
    let Some(block) = root
        .field_path("materials")
        .and_then(|field| field.as_block())
    else {
        return Vec::new();
    };
    (0..block.len())
        .map(|index| {
            block
                .element(index)
                .and_then(|element| element.read_tag_ref_with_group("render method"))
                .filter(|(_, path)| !path.trim().is_empty())
                .map(|(shader_group, path)| RenderModelPreviewMaterial {
                    shader_path: path,
                    shader_group,
                    overlay: false, shield: None,
                    permutation: 0,
                    object: 0,
                })
                .unwrap_or_default()
        })
        .collect()
}

/// One instanced-geometry placement: basis columns, position, uniform scale.
pub(super) struct InstancePlacement {
    pub(super) forward: [f32; 3],
    pub(super) left: [f32; 3],
    pub(super) up: [f32; 3],
    pub(super) position: [f32; 3],
    pub(super) scale: f32,
}

impl InstancePlacement {
    pub(super) fn identity() -> Self {
        Self {
            forward: [1.0, 0.0, 0.0],
            left: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            position: [0.0; 3],
            scale: 1.0,
        }
    }

    pub(super) fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let s = if self.scale.is_finite() && self.scale != 0.0 {
            self.scale
        } else {
            1.0
        };
        [
            self.position[0]
                + s * (self.forward[0] * p[0] + self.left[0] * p[1] + self.up[0] * p[2]),
            self.position[1]
                + s * (self.forward[1] * p[0] + self.left[1] * p[1] + self.up[1] * p[2]),
            self.position[2]
                + s * (self.forward[2] * p[0] + self.left[2] * p[1] + self.up[2] * p[2]),
        ]
    }

    pub(super) fn rotate(&self, v: [f32; 3]) -> [f32; 3] {
        [
            self.forward[0] * v[0] + self.left[0] * v[1] + self.up[0] * v[2],
            self.forward[1] * v[0] + self.left[1] * v[1] + self.up[1] * v[2],
            self.forward[2] * v[0] + self.left[2] * v[1] + self.up[2] * v[2],
        ]
    }
}

fn vector(v: &RealVector3d) -> [f32; 3] {
    [v.i, v.j, v.k]
}

/// Whether decompressing through these bounds mirrors an odd number of axes,
/// inverting triangle winding against the stored normals.
pub(super) fn bounds_axis_flip(bounds: &CompressionBounds) -> bool {
    if !bounds.pos_compressed {
        return false;
    }
    [
        bounds.px_max < bounds.px_min,
        bounds.py_max < bounds.py_min,
        bounds.pz_max < bounds.pz_min,
    ]
    .into_iter()
    .filter(|flipped| *flipped)
    .count()
        % 2
        == 1
}

/// Append one decoded render mesh under `placement`, keeping UVs and rotating
/// the full tangent frame, and file its triangles into the per-material index
/// lists. `flip` swaps winding for meshes whose compression bounds mirror.
fn append_sbsp_mesh(
    preview: &mut RenderModelPreview,
    render_indices: &mut [Vec<u32>],
    mesh: &blam_tags::render_model::RenderMesh,
    placement: &InstancePlacement,
    flip: bool,
) {
    let Some(vertex_base) = VertexBase::of(preview.vertices.len(), mesh.vertices.len(), Overflow::RefusesAppend) else {
        return;
    };
    preview.vertices.reserve(mesh.vertices.len());
    for vertex in &mesh.vertices {
        let position = placement.apply(point(&vertex.position));
        expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
        preview.vertices.push(RenderModelPreviewVertex {
            position,
            normal: placement.rotate(vector(&vertex.normal)),
            texcoord: [vertex.texcoord.x, vertex.texcoord.y],
            tangent: placement.rotate(vector(&vertex.tangent)),
            binormal: placement.rotate(vector(&vertex.binormal)),
            // BSPs are static; zero weights keep the skinning path inert.
            ..Default::default()
        });
    }
    for part in &mesh.parts {
        let start = part.index_start as usize;
        let end = start
            .saturating_add(part.index_count as usize)
            .min(mesh.indices.len());
        if start >= end {
            continue;
        }
        let slot = (part.material_index as usize).min(render_indices.len().saturating_sub(1));
        let Some(indices) = render_indices.get_mut(slot) else {
            continue;
        };
        for triangle in mesh.indices[start..end].chunks_exact(3) {
            if triangle.iter().any(|&i| i as usize >= mesh.vertices.len()) {
                continue;
            }
            let (b, c) = if flip { (2, 1) } else { (1, 2) };
            indices.push(vertex_base.offset(triangle[0]));
            indices.push(vertex_base.offset(triangle[b]));
            indices.push(vertex_base.offset(triangle[c]));
        }
    }
}

/// Cluster portals as a fan-triangulated `portals` layer. Portal points are
/// stored in world units, so no rescale.
fn append_sbsp_portals(root: &TagStruct<'_>, preview: &mut RenderModelPreview) {
    let Some(portals) = root
        .field_path("cluster portals")
        .and_then(|field| field.as_block())
    else {
        return;
    };
    let mut triples: Vec<([f32; 3], [f32; 3])> = Vec::new();
    for portal in portals.iter() {
        let Some(vertices) = portal.field("vertices").and_then(|field| field.as_block()) else {
            continue;
        };
        let ring: Vec<[f32; 3]> = (0..vertices.len())
            .filter_map(|vi| vertices.element(vi))
            .map(|element| point(&element.read_point3d("point")))
            .collect();
        for k in 1..ring.len().saturating_sub(1) {
            let normal = face_normal(ring[0], ring[k], ring[k + 1]);
            triples.push((ring[0], normal));
            triples.push((ring[k], normal));
            triples.push((ring[k + 1], normal));
        }
    }
    push_derived_batch(preview, "portals", Some(PORTAL_COLOR), &triples);
}

/// The sealed-world collision BSP as a `collision` layer, walked straight off
/// the winged-edge blocks. H3/ODST keep it in `collision bsp`; Reach-era
/// tags moved it to `large collision bsp` — read whichever are present.
fn append_sbsp_collision(root: &TagStruct<'_>, preview: &mut RenderModelPreview) {
    let mut triples: Vec<([f32; 3], [f32; 3])> = Vec::new();
    for name in ["collision bsp", "large collision bsp"] {
        let Some(block) = root
            .field_path(&format!(
                "resource interface/raw_resources[0]/raw_items/{name}"
            ))
            .and_then(|field| field.as_block())
        else {
            continue;
        };
        for bsp in block.iter() {
            append_collision_bsp_triangles(&bsp, &mut triples);
        }
    }
    push_derived_batch(preview, COLLISION_REGION, Some(COLLISION_COLOR), &triples);
}

/// Walk one collision BSP's surfaces: each surface rings its edges (an edge
/// belongs to two surfaces; which side it is on decides start-vs-end vertex
/// and forward-vs-reverse continuation), then fan-triangulates the ring.
pub(super) fn append_collision_bsp_triangles(bsp: &TagStruct<'_>, triples: &mut Vec<([f32; 3], [f32; 3])>) {
    let (Some(surfaces), Some(edges), Some(vertices)) = (
        bsp.field_path("surfaces")
            .and_then(|field| field.as_block()),
        bsp.field_path("edges").and_then(|field| field.as_block()),
        bsp.field_path("vertices")
            .and_then(|field| field.as_block()),
    ) else {
        return;
    };
    // The later games keep these in 16 bits that index past 32767 (Reach's
    // edges run to `UNSIGNED_SHORT_MAX`); only 0xFFFF is none.
    let index = |entry: &TagStruct<'_>, name: &str| -> i128 {
        match entry.field(name).and_then(|field| field.value()) {
            Some(TagFieldData::ShortInteger(-1)) | None => -1,
            Some(TagFieldData::ShortInteger(value)) => i128::from(value as u16),
            _ => entry.read_int_any(name).unwrap_or(-1),
        }
    };
    type EdgeRow = (i128, i128, i128, i128, i128, i128);
    let read_edge = |at: i128| -> Option<EdgeRow> {
        let edge = edges.element(usize::try_from(at).ok()?)?;
        Some((
            index(&edge, "start vertex"),
            index(&edge, "end vertex"),
            index(&edge, "forward edge"),
            index(&edge, "reverse edge"),
            index(&edge, "left surface"),
            index(&edge, "right surface"),
        ))
    };
    for (surface_index, surface) in surfaces.iter().enumerate() {
        let first_edge = index(&surface, "first edge");
        if first_edge < 0 {
            continue;
        }
        let si = surface_index as i128;
        let mut ring: Vec<[f32; 3]> = Vec::new();
        let mut edge_index = first_edge;
        // Malformed rings never terminate; the step bound is the bail-out.
        let max_steps = edges.len() * 2 + 8;
        for _ in 0..max_steps {
            let Some((start, end, forward, reverse, left, right)) = read_edge(edge_index) else {
                ring.clear();
                break;
            };
            let (vertex_index, next) = if left == si {
                (start, forward)
            } else if right == si {
                (end, reverse)
            } else {
                ring.clear();
                break;
            };
            let Some(vertex) = usize::try_from(vertex_index)
                .ok()
                .and_then(|vi| vertices.element(vi))
            else {
                ring.clear();
                break;
            };
            ring.push(point(&vertex.read_point3d("point")));
            if next == first_edge {
                break;
            }
            edge_index = next;
        }
        for k in 1..ring.len().saturating_sub(1) {
            let normal = face_normal(ring[0], ring[k], ring[k + 1]);
            triples.push((ring[0], normal));
            triples.push((ring[k], normal));
            triples.push((ring[k + 1], normal));
        }
    }
}
