//! Model tags: the render model a `.model` names, its variants and instance groups, and its overlay shaders.

use super::*;

/// A `.model`: the render model it names, the hologram and shield shaders it
/// lays over that, its variants' instances, and, when the `.model` itself is
/// open, what its variants attach.
pub(super) fn load_model(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    reader: Option<&TagReader>,
) -> Result<ModelPreviewData, String> {
    let (render_entry, normalized, render_tag) = read_model_render(model_tag, names, reader)?;
    let mut preview = build_render_preview(&render_tag)?;
    if preview.batches.is_empty() {
        return Err("Referenced render_model has no previewable draw batches.".to_owned());
    }
    add_overlay_shaders(model_tag, &mut preview);
    // The collision/physics overlays are not built here: walking the
    // collision BSP and tessellating the physics shapes is slow, and a
    // toggle would rebuild the merge each time. `maybe_request_model_overlays`
    // builds them once on a worker after this load lands, and the toggles are
    // draw-time filters from then on.
    // Halo 4 shows every instance for a NONE group as Reach does (the 360's
    // 0x8325C1B8).
    let reach = reader.is_some_and(|reader| matches!(reader.source.game(), Some(GameId::HaloReach | GameId::Halo4)));
    preview.instances.variants = read_variant_instances(model_tag, &preview.instances.names, reach);
    let mut data = model_preview_data(render_entry.key, normalized, preview, read_model_variants(model_tag));
    // The `.model` opened itself lists what its variants attach. An object
    // reaches here for its `.model` and attaches them once it has read its own.
    let opened = entry.group_tag == u32::from_be_bytes(*b"hlmt");
    let reader = reader.filter(|reader| {
        matches!(
            &reader.source,
            TagSource::LooseFolder {
                game: Some(GameId::Halo2 | GameId::Halo3 | GameId::Halo3Odst | GameId::HaloReach | GameId::Halo4),
                ..
            }
        )
    });
    if let Some(reader) = reader.filter(|_| opened) {
        let preview = std::sync::Arc::make_mut(&mut data.preview);
        let variants = &mut data.variants;
        let attach = |preview: &mut RenderModelPreview| {
            super::super::weapons::variants::attach_variant_objects(reader, model_tag, variants, preview)
        };
        data.attached.extend(feature_into(&mut data.notes, "the variants' children", preview, attach));
    }
    Ok(data)
}

/// The render model a `.model` names, found in the loose-folder kit: its
/// entry, its path without the extension, and the tag read.
pub(super) fn read_model_render(
    model_tag: &TagFile,
    names: &TagNameIndex,
    reader: Option<&TagReader>,
) -> Result<(TagEntry, String, TagFile), String> {
    let Some((group_tag, rel_path)) = model_tag.root().read_tag_ref_with_group("render model")
    else {
        return Err("This model tag has no render model reference.".to_owned());
    };
    if rel_path.trim().is_empty() {
        return Err("This model tag has an empty render model reference.".to_owned());
    }
    let Some(TagSource::LooseFolder { root, .. }) = reader.map(|reader| &reader.source) else {
        return Err("Render model preview requires a loaded loose-folder editing kit.".to_owned());
    };
    let extension = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
        .unwrap_or("render_model");
    let mut normalized = rel_path.to_owned();
    if let Some(stripped) = normalized.strip_suffix(&format!(".{extension}")) {
        normalized = stripped.to_owned();
    }
    let path = resolve_tag_path(root, &normalized, extension);
    if !path.exists() {
        return Err(format!(
            "Referenced render_model was not found: {}",
            path.display()
        ));
    }
    let render_entry = TagEntry {
        key: file_entry_key(&path),
        display_path: format!("{normalized}.{extension}"),
        group_tag,
        group_name: names.name_for(group_tag).map(str::to_owned),
        location: TagEntryLocation::LooseFile(path),
    };
    let render_tag =
        read_entry(&reader.unwrap().source, &render_entry).map_err(|error| error.to_string())?;
    Ok((render_entry, normalized, render_tag))
}

/// The shaders a `.model` lays over its render model: its hologram shader
/// and its shield shaders, each drawn again over every section.
pub(super) fn add_overlay_shaders(model_tag: &TagFile, preview: &mut RenderModelPreview) {
    // `sub_6C18F0`: while the model's `hologram control function` is above
    // nothing, `render_hologram_group` draws it again with its hologram shader
    // over every section. That shader's bloom passes join the frame's bloom.
    let shader_group = u32::from_be_bytes(*b"shad");
    let reference = model_tag.root().field("hologram shader").and_then(|field| field.value());
    if let Some(blam_tags::TagFieldData::TagReference(reference)) = reference
        && let Some((group, shader_path)) = reference.group_tag_and_name
        && group == shader_group
        && !shader_path.is_empty()
    {
        let control = model_tag.root().read_string_id("hologram control function").filter(|name| !name.is_empty());
        if let Some(control) = control {
            preview.h2.hologram_control = Some(control);
            preview.materials.push(RenderModelPreviewMaterial {
                shader_path,
                shader_group,
                overlay: false,
                shield: Some(H2Layer::HologramBloom),
                permutation: 0,
                object: 0,
            });
        }
    }
    // `sub_6C18F0` lays its shield shaders over it the same way.
    let damage = model_tag.root().field("new damage info").and_then(|field| field.as_block());
    if let Some(damage) = damage.and_then(|block| block.element(0)) {
        let shaders = [
            ("shield damaged shader", H2Layer::ShieldDamaged),
            ("overshield shader", H2Layer::Overshield),
        ];
        for (name, layer) in shaders {
            let Some(shader_path) = damage.read_tag_ref_path(name).filter(|path| !path.is_empty())
            else {
                continue;
            };
            preview.materials.push(RenderModelPreviewMaterial {
                shader_path,
                shader_group,
                overlay: false,
                shield: Some(layer),
                permutation: 0,
                object: 0,
            });
        }
    }
}
