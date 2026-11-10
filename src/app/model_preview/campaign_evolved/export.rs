//! A Campaign Evolved preview built from the game's exported meshes, with the tool's JMS conventions.

use super::super::super::{TagEntry, TagSource};
use super::super::{ModelPreviewData, TagReader, model_preview_data, render_model_to_preview};
use super::MountedContainer;
use super::meshes::{CevMeshes, CevModelMeshes, cev_mesh_parts, resolve_ce_model_meshes};
use blam_tags::iostore::container_header::EIoContainerHeaderVersion;
use blam_tags::iostore::ue_types::EIoStoreTocVersion;
use blam_tags::{RenderModel, TagFile};

pub(super) const CEV_CV: EIoStoreTocVersion = EIoStoreTocVersion::ReplaceIoChunkHashWithIoHash;

pub(super) const CEV_HV: EIoContainerHeaderVersion =
    EIoContainerHeaderVersion::SoftPackageReferences;

/// Halo: Campaign Evolved model preview. Returns `None` when this isn't a CE
/// model (caller falls through to the classic render_model path); `Some(_)`
/// once we've recognized a CE `.model` (hlmt with a `skeleton model` ref
/// inside an IoStore container set), whether or not resolution succeeds.
pub(in crate::app::model_preview) fn load_campaign_evolved_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: Option<&TagReader>,
    high_detail: bool,
) -> Option<Result<ModelPreviewData, String>> {
    let reader = reader?;
    let TagSource::IoStoreContainerSet { containers, .. } = &reader.source else {
        return None;
    };
    if &model_tag.header.group_tag.to_be_bytes() != b"hlmt" {
        return None;
    }
    // Classic hlmt references a `render model`; CE references a `skeleton model`.
    let (_group, skel_ref) = model_tag.root().read_tag_ref_with_group("skeleton model")?;
    if skel_ref.trim().is_empty() {
        return None;
    }
    Some(build_campaign_evolved_preview(
        model_tag,
        entry,
        reader,
        containers,
        &skel_ref,
        high_detail,
    ))
}

fn build_campaign_evolved_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: &TagReader,
    containers: &[MountedContainer],
    skel_ref: &str,
    high_detail: bool,
) -> Result<ModelPreviewData, String> {
    let CevModelMeshes {
        skel,
        variants,
        meshes,
        render_path,
    } = resolve_ce_model_meshes(model_tag, entry, reader, containers, skel_ref, high_detail)?;
    let (parts, static_parts, world_parts) = cev_mesh_parts(&meshes);

    // 5. Reconstruct the cross-game RenderModel and run the standard pipeline.
    let (render_model, render_meshes) =
        RenderModel::from_ue_meshes(&parts, &static_parts, &world_parts, &skel)
            .map_err(|e| e.to_string())?;
    let preview = render_model_to_preview(&render_model, &render_meshes);
    if preview.batches.is_empty() {
        return Err("Reconstructed CE model has no previewable geometry.".to_owned());
    }
    Ok(model_preview_data(
        entry.key.clone(),
        render_path,
        preview,
        variants,
    ))
}

/// Build a full-resolution JMS for a Campaign Evolved `hlmt` model by fusing
/// its Unreal render geometry (skeletal + **Nanite** static pieces) onto the
/// classic `skeleton_model` rig. Resolves the meshes as the preview does
/// ([`resolve_ce_model_meshes`]) but at full Nanite detail, and emits JMS —
/// the render-geometry half of model extraction (CE keeps render geometry in
/// Unreal, so there's no `render_model` tag to walk).
pub(in crate::app) fn campaign_evolved_render_jms(
    model_tag: &TagFile,
    entry: &TagEntry,
    source: &TagSource,
    skel_ref: &str,
) -> Result<blam_tags::jms::JmsFile, String> {
    // An extraction is a load of its own.
    let reader = &TagReader::new(source);
    let TagSource::IoStoreContainerSet { containers, .. } = source else {
        return Err("CE render extraction requires an IoStore container source.".to_owned());
    };
    let CevModelMeshes { skel, meshes, .. } =
        resolve_ce_model_meshes(model_tag, entry, reader, containers, skel_ref, true)?;
    let (parts, static_parts, world_parts) = cev_mesh_parts(&meshes);
    let mut jms =
        blam_tags::jms::JmsFile::from_ue_meshes(&parts, &static_parts, &world_parts, &skel)
            .map_err(|e| e.to_string())?;
    apply_chimp_jms_uv_conventions(&mut jms, &meshes)?;
    Ok(jms)
}

/// The composite CE exporter must retain its classic skeleton retargeting and
/// region/permutation material cells. UV orientation and wrapping are safe to
/// take directly from Chimp's standalone converters; positions, normals and
/// weights remain assembly-specific because they must be retargeted to the tag
/// skeleton. The composite builder emits vertices in the same part order used
/// here.
fn apply_chimp_jms_uv_conventions(
    target: &mut blam_tags::jms::JmsFile,
    meshes: &CevMeshes,
) -> Result<(), String> {
    let mut vertex_offset = 0usize;

    for (_, _, _, mesh, materials) in &meshes.skeletal {
        let source = crate::app::chimp::chimp_skeletal_mesh_to_jms(mesh, materials);
        copy_chimp_jms_uvs(target, vertex_offset, &source)?;
        vertex_offset = vertex_offset.saturating_add(source.vertices.len());
    }
    for (_, _, _, mesh, _, materials, _, _) in &meshes.statics {
        let source = crate::app::chimp::chimp_static_mesh_to_jms(mesh, materials);
        copy_chimp_jms_uvs(target, vertex_offset, &source)?;
        vertex_offset = vertex_offset.saturating_add(source.vertices.len());
    }
    for (_, _, _, mesh, _, materials, _) in &meshes.world {
        let source = crate::app::chimp::chimp_skeletal_mesh_to_jms(mesh, materials);
        copy_chimp_jms_uvs(target, vertex_offset, &source)?;
        vertex_offset = vertex_offset.saturating_add(source.vertices.len());
    }

    if vertex_offset != target.vertices.len() {
        return Err(format!(
            "Chimp JMS vertex layout mismatch: converted {vertex_offset}, composite has {}",
            target.vertices.len()
        ));
    }
    Ok(())
}

pub(super) fn copy_chimp_jms_uvs(
    target: &mut blam_tags::jms::JmsFile,
    vertex_offset: usize,
    source: &blam_tags::jms::JmsFile,
) -> Result<(), String> {
    let end = vertex_offset
        .checked_add(source.vertices.len())
        .ok_or_else(|| "Chimp JMS vertex range overflowed".to_owned())?;
    let target_vertices = target
        .vertices
        .get_mut(vertex_offset..end)
        .ok_or_else(|| "Chimp JMS vertex range does not match the composite model".to_owned())?;
    for (target, source) in target_vertices.iter_mut().zip(&source.vertices) {
        target.uvs.clone_from(&source.uvs);
    }
    Ok(())
}

/// A Campaign Evolved `.model`: one that names a skeleton model.
pub(in crate::app::model_preview) fn is_campaign_evolved_model(tag: &TagFile) -> bool {
    tag.header.group_tag.to_be_bytes() == *b"hlmt"
        && tag
            .root()
            .read_tag_ref_with_group("skeleton model")
            .map(|(_, reference)| !reference.trim().is_empty())
            .unwrap_or(false)
}
