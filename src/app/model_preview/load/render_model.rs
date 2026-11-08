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
