//! Chimp package decoding and rebuilding, with no UI.
//! It owns reading packages into exports, previews, type indexes and JSON text; drawing and saving belong elsewhere.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChimpMeshKind {
    Skeletal,
    Static,
}

pub(in crate::app) struct ChimpTypeIndex {
    pub(super) package_types: Vec<Option<String>>,
    pub(super) type_counts: BTreeMap<String, usize>,
    pub(super) failures: usize,
}

pub(super) struct ChimpTexturePreview {
    pub(super) export_index: usize,
    pub(super) preview: BitmapPreviewState,
    /// Every layer and mip the package stores, kept in its cooked pixel format.
    /// The viewer expands one of these at a time; export writes them untouched.
    pub(super) surfaces: Result<Texture2dSurfaces, String>,
}

pub(super) struct ChimpExport {
    pub(super) object: String,
    pub(super) class: Option<String>,
    pub(super) decoded: Result<Export, String>,
}

pub(super) fn load_chimp_document(world: &World, package: &str) -> Result<ChimpDocument, String> {
    let record = world
        .package(package)
        .ok_or_else(|| format!("{package} is not mounted"))?;
    let provider = record
        .active_provider()
        .cloned()
        .ok_or_else(|| format!("{package} has no active provider"))?;
    let bytes = world
        .read_provider(&provider)
        .map_err(|error| error.to_string())?;
    decode_chimp_document(world, provider, bytes)
}

/// One package, decoded as far as its exports and no further.
///
/// This is all a reader needs, and it is a small fraction of what loading a
/// *document* costs: [`decode_chimp_document`] goes on to decode previews and
/// render the editor's text panes, which for a World Partition cell is 329ms of
/// work against the 1ms the exports themselves take. Reading a level is 2,334
/// cells, so the difference is a quarter of an hour against seconds.
pub(super) struct ChimpPackage {
    pub(super) provider: PackageProvider,
    pub(super) bytes: Vec<u8>,
    pub(super) header: FZenPackageHeader,
    pub(super) exports: Vec<ChimpExport>,
}

/// Read and decode a package's exports, skipping everything the editor's
/// document pane needs and a reader does not.
pub(super) fn load_chimp_package(world: &World, package: &str) -> Result<ChimpPackage, String> {
    let record = world
        .package(package)
        .ok_or_else(|| format!("{package} is not mounted"))?;
    let provider = record
        .active_provider()
        .cloned()
        .ok_or_else(|| format!("{package} has no active provider"))?;
    let bytes = world
        .read_provider(&provider)
        .map_err(|error| error.to_string())?;
    let (header, _, exports) = decode_chimp_exports(world, &provider, &bytes)?;
    Ok(ChimpPackage {
        provider,
        bytes,
        header,
        exports,
    })
}

/// A package's Texture2D exports, decoded to surfaces, and nothing else.
///
/// What a texture export needs. A full document also decodes any mesh — a
/// Nanite decode for a static mesh — and renders both text panes, all of which
/// an export throws away.
pub(super) fn load_chimp_texture_previews(
    world: &World,
    package: &str,
) -> Result<Vec<ChimpTexturePreview>, String> {
    let record = world
        .package(package)
        .ok_or_else(|| format!("{package} is not mounted"))?;
    let provider = record
        .active_provider()
        .cloned()
        .ok_or_else(|| format!("{package} has no active provider"))?;
    let bytes = world
        .read_provider(&provider)
        .map_err(|error| error.to_string())?;
    let (header, payloads, exports) = decode_chimp_exports(world, &provider, &bytes)?;
    Ok(chimp_texture_previews_for(
        world, &provider, &bytes, &header, &payloads, &exports,
    ))
}

/// Decode a package's Texture2D exports, given its decoded front half.
fn chimp_texture_previews_for(
    world: &World,
    provider: &PackageProvider,
    bytes: &[u8],
    header: &FZenPackageHeader,
    payloads: &[Vec<u8>],
    exports: &[ChimpExport],
) -> Vec<ChimpTexturePreview> {
    if !exports
        .iter()
        .any(|export| export.class.as_deref() == Some("Texture2D"))
    {
        return Vec::new();
    }
    let names = header.name_map.copy_raw_names();
    let resolver = world.resolver(header, bytes, &names);
    let bulk: Vec<(i64, i64)> = header
        .bulk_data
        .iter()
        .map(|entry| (entry.serial_offset, entry.serial_size))
        .collect();
    decode_chimp_texture_previews(
        world, provider, header, payloads, &names, &resolver, &bulk, exports,
    )
}

/// Which kind of mesh a package holds, if any, by its exports' classes.
pub(super) fn chimp_mesh_kind(exports: &[ChimpExport]) -> Option<ChimpMeshKind> {
    let has = |class: &str| {
        exports
            .iter()
            .any(|export| export.class.as_deref() == Some(class))
    };
    if has("SkeletalMesh") {
        Some(ChimpMeshKind::Skeletal)
    } else if has("StaticMesh") {
        Some(ChimpMeshKind::Static)
    } else {
        None
    }
}

/// The shared front half of loading a package: header, payloads, exports.
fn decode_chimp_exports(
    world: &World,
    provider: &PackageProvider,
    bytes: &[u8],
) -> Result<(FZenPackageHeader, Vec<Vec<u8>>, Vec<ChimpExport>), String> {
    let header = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .map_err(|error| format!("Could not parse {}: {error:#}", provider.entry_path))?;
    let payloads = read_payloads(&header, bytes)
        .map_err(|error| format!("Could not split {}: {error:#}", provider.entry_path))?;
    let names = header.name_map.copy_raw_names();
    let resolver = world.resolver(&header, bytes, &names);
    let bulk: Vec<(i64, i64)> = header
        .bulk_data
        .iter()
        .map(|entry| (entry.serial_offset, entry.serial_size))
        .collect();
    let context = ExportContext {
        bulk_data: &bulk,
        resolver: Some(&resolver),
    };
    let exports: Vec<ChimpExport> = header
        .export_map
        .iter()
        .zip(&payloads)
        .map(|(entry, payload)| {
            let object = header.name_map.get(entry.object_name).to_string();
            let class = world.class_key(&header, entry.class_index);
            let decoded = class
                .as_deref()
                .ok_or_else(|| "class could not be resolved".to_owned())
                .and_then(|class| {
                    read_export_in(
                        payload,
                        &names,
                        world.usmap(),
                        class,
                        entry.object_flags,
                        &context,
                    )
                    .map_err(|error| error.to_string())
                });
            ChimpExport {
                object,
                class,
                decoded,
            }
        })
        .collect();
    drop(resolver);
    Ok((header, payloads, exports))
}

pub(super) fn decode_chimp_document(
    world: &World,
    provider: PackageProvider,
    bytes: Vec<u8>,
) -> Result<ChimpDocument, String> {
    let (header, payloads, exports) = decode_chimp_exports(world, &provider, &bytes)?;
    let texture_previews =
        chimp_texture_previews_for(world, &provider, &bytes, &header, &payloads, &exports);
    let (mesh_kind, mesh_preview, mesh_preview_state) =
        decode_chimp_mesh_preview(world, &provider, &bytes, &header, &exports);
    let initial_view = if !texture_previews.is_empty() {
        ChimpDocumentView::Texture
    } else if mesh_kind.is_some() {
        ChimpDocumentView::Mesh
    } else {
        ChimpDocumentView::default()
    };
    let mut document = ChimpDocument {
        package: header.package_name(),
        provider,
        original: bytes,
        header,
        payloads,
        exports,
        texture_previews,
        mesh_kind,
        mesh_preview,
        mesh_preview_state,
        selected_export: 0,
        dirty: false,
        view: initial_view,
        document_text: String::new(),
        document_lines: ChimpJsonLines::default(),
        document_text_dirty: true,
        metadata_text: String::new(),
        metadata_lines: ChimpJsonLines::default(),
        metadata_text_dirty: true,
        header_usage: None,
        header_name_filter: String::new(),
        header_name_edit: None,
        header_import_edit: None,
        header_export_edit: None,
        header_identity_edit: None,
        header_error: None,
        referrers: ChimpReferrerState::Idle,
        orphaned: false,
        checkpoint_due: None,
        edits: 0,
    };
    refresh_chimp_document_text(&mut document);
    refresh_chimp_metadata_text(&mut document, world);
    Ok(document)
}

/// Whether an imported package is one of the mesh's materials.
///
/// Recognised by the `M_`/`MI_` prefix the game's own content uses. A mesh's
/// import list carries far more than its materials, and this is the same rule
/// the material list written into an exported mesh already relies on — so the
/// textures exported alongside a mesh belong to the materials named in it.
pub(super) fn is_chimp_material_package(path: &str) -> bool {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    leaf.starts_with("MI_") || leaf.starts_with("M_")
}

pub(super) fn chimp_material_names(header: &FZenPackageHeader) -> Vec<String> {
    header
        .imported_package_names
        .iter()
        .filter(|path| is_chimp_material_package(path))
        .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
        .collect()
}

fn decode_chimp_mesh_preview(
    world: &World,
    provider: &PackageProvider,
    bytes: &[u8],
    header: &FZenPackageHeader,
    exports: &[ChimpExport],
) -> (
    Option<ChimpMeshKind>,
    Option<Result<ModelPreviewData, String>>,
    ModelPreviewState,
) {
    let kind = chimp_mesh_kind(exports);
    let preview = kind.map(|kind| {
        let header_size = header.summary.header_size as usize;
        let preview = match kind {
            ChimpMeshKind::Skeletal => {
                SkeletalMesh::from_package(bytes, &header.name_map.copy_raw_names(), header_size)
                    .map(chimp_skeletal_mesh_preview)
            }
            ChimpMeshKind::Static => {
                let bulk = world
                    .archives()
                    .get(provider.container)
                    .and_then(|archive| {
                        let chunk = archive.chunk_index_for(&provider.entry_path).ok()?;
                        archive.read_bulk_for(chunk, 0).ok()
                    });
                StaticMesh::from_package_preferring_nanite(bytes, header_size, bulk.as_deref())
                    .map(chimp_static_mesh_preview)
            }
        }
        .map_err(|error| format!("Could not decode mesh geometry: {error:#}"))?;
        Ok(model_preview::standalone_mesh_preview(
            header.package_name(),
            preview,
        ))
    });
    let mut state = ModelPreviewState::default();
    if kind.is_some() {
        state.show_backfaces = true;
        state.region_selections.insert(
            "mesh".to_owned(),
            ModelRegionSelection {
                enabled: true,
                permutation: "default".to_owned(),
            },
        );
    }
    (kind, preview, state)
}

fn chimp_skeletal_mesh_preview(mesh: SkeletalMesh) -> RenderModelPreview {
    let mut preview = chimp_mesh_preview_base(
        mesh.vertices
            .iter()
            .map(|vertex| (vertex.position, vertex.normal)),
        mesh.indices,
    );
    for section in mesh.sections {
        let index_start = section.base_index.min(preview.indices.len() as u32);
        let index_count =
            (section.num_triangles * 3).min(preview.indices.len() as u32 - index_start);
        if index_count > 0 {
            preview.batches.push(RenderModelPreviewBatch {
                region_name: "mesh".to_owned(),
                permutation_name: "default".to_owned(),
                material_index: section.material_index,
                index_start,
                index_count,
                flat_color: None,
                layer: ModelPreviewLayer::Render,
            });
        }
    }
    if preview.batches.is_empty() && !preview.indices.is_empty() {
        preview.batches.push(RenderModelPreviewBatch {
            region_name: "mesh".to_owned(),
            permutation_name: "default".to_owned(),
            material_index: 0,
            index_start: 0,
            index_count: preview.indices.len() as u32,
            flat_color: None,
            layer: ModelPreviewLayer::Render,
        });
    }
    preview
}

fn chimp_static_mesh_preview(mesh: StaticMesh) -> RenderModelPreview {
    let mut preview = chimp_mesh_preview_base(
        mesh.vertices
            .iter()
            .map(|vertex| (vertex.position, vertex.normal)),
        mesh.indices,
    );
    if !preview.indices.is_empty() {
        preview.batches.push(RenderModelPreviewBatch {
            region_name: "mesh".to_owned(),
            permutation_name: "default".to_owned(),
            material_index: 0,
            index_start: 0,
            index_count: preview.indices.len() as u32,
            flat_color: None,
            layer: ModelPreviewLayer::Render,
        });
    }
    preview
}

fn chimp_mesh_preview_base(
    vertices: impl IntoIterator<Item = ([f32; 3], [f32; 3])>,
    indices: Vec<u32>,
) -> RenderModelPreview {
    let mut preview = RenderModelPreview {
        regions: vec![RenderModelPreviewRegion {
            name: "mesh".to_owned(),
            permutations: vec!["default".to_owned()],
        }],
        indices,
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };
    for (position, normal) in vertices {
        for axis in 0..3 {
            preview.bounds_min[axis] = preview.bounds_min[axis].min(position[axis]);
            preview.bounds_max[axis] = preview.bounds_max[axis].max(position[axis]);
        }
        preview.vertices.push(RenderModelPreviewVertex {
            position,
            normal,
            // UE meshes reach the preview without a resolved tangent frame;
            // they render with the untextured path.
            ..Default::default()
        });
    }
    if preview.vertices.is_empty() {
        preview.bounds_min = [-1.0; 3];
        preview.bounds_max = [1.0; 3];
    }
    preview
}

fn decode_chimp_texture_previews(
    world: &World,
    provider: &PackageProvider,
    header: &FZenPackageHeader,
    payloads: &[Vec<u8>],
    names: &[String],
    resolver: &dyn blam_tags::iostore::object::archive::PackageResolver,
    bulk: &[(i64, i64)],
    exports: &[ChimpExport],
) -> Vec<ChimpTexturePreview> {
    let archive = &world.archives()[provider.container];
    let package_chunk = archive.chunk_index_for(&provider.entry_path);
    exports
        .iter()
        .enumerate()
        .filter(|(_, export)| export.class.as_deref() == Some("Texture2D"))
        .map(|(export_index, export)| {
            let decoded = (|| {
                let package_entry = header
                    .export_map
                    .get(export_index)
                    .ok_or_else(|| "Texture export is missing from the package map".to_owned())?;
                let payload = payloads
                    .get(export_index)
                    .ok_or_else(|| "Texture export payload is missing".to_owned())?;
                let export = export.decoded.as_ref().map_err(Clone::clone)?;
                let context = TailContext {
                    bulk_data: bulk,
                    origin: payload.len().saturating_sub(export.tail.len()),
                    usmap: world.usmap(),
                    resolver: Some(resolver),
                    object_flags: package_entry.object_flags,
                };
                let texture = parse_texture_chain_tail(&export.tail, names, context, true)
                    .map_err(|error| error.to_string())?;
                let package_chunk = package_chunk.as_ref().map_err(|error| error.to_string())?;
                decode_texture2d_surfaces(&texture, |bulk_index| {
                    let entry = header
                        .bulk_data
                        .get(bulk_index.max(0) as usize)
                        .ok_or_else(|| {
                            anyhow::anyhow!("bulk-data index {bulk_index} is out of range")
                        })?;
                    let chunk = archive.read_bulk_for(*package_chunk, entry.cooked_index as u16)?;
                    let start = usize::try_from(entry.serial_offset)
                        .map_err(|_| anyhow::anyhow!("negative bulk-data offset"))?;
                    let size = usize::try_from(entry.serial_size)
                        .map_err(|_| anyhow::anyhow!("negative bulk-data size"))?;
                    chunk
                        .get(start..start.saturating_add(size))
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| {
                            anyhow::anyhow!("bulk-data entry lies outside its sibling chunk")
                        })
                })
                .map_err(|error| error.to_string())
            })();
            let mut preview = BitmapPreviewState::default();
            // Start on the largest mip that a GPU texture can actually hold.
            // Mip 0 of a virtual texture is routinely wider than the driver's
            // limit, and an oversized upload renders as noise rather than
            // failing, so choosing here is the difference between a preview and
            // an apparently corrupt one.
            if let Ok(surfaces) = &decoded {
                preview.mip_index = first_displayable_mip(surfaces, 0);
            }
            preview.decoded = Some(decoded.as_ref().map_err(Clone::clone).and_then(|surfaces| {
                chimp_texture_mip_data(surfaces, preview.image_index, preview.mip_index)
            }));
            ChimpTexturePreview {
                export_index,
                preview,
                surfaces: decoded,
            }
        })
        .collect()
}

/// Largest mip index that is safe to upload as a GPU texture.
///
/// Cooked virtual textures reach 14336 px on their base mip, well past the
/// 8192-16384 limit typical hardware reports. Uploading past that limit does not
/// error — the texture is simply never defined, and the viewer shows garbage —
/// so the base mip is skipped rather than shown wrong. `max_side` of 0 means the
/// limit is not known yet, in which case a conservative cap is applied.
pub(super) fn first_displayable_mip(surfaces: &Texture2dSurfaces, max_side: usize) -> usize {
    let limit = if max_side == 0 { 8192 } else { max_side } as u32;
    surfaces
        .layers
        .first()
        .and_then(|layer| {
            layer
                .mips
                .iter()
                .position(|mip| mip.width <= limit && mip.height <= limit)
        })
        .unwrap_or(0)
}

/// Expand one layer/mip of a decoded texture into the shared bitmap viewer's
/// model. Layer and mip counts come from the surfaces, so the viewer's existing
/// steppers work without Chimp growing a second viewer.
pub(super) fn chimp_texture_mip_data(
    surfaces: &Texture2dSurfaces,
    layer_index: usize,
    mip_index: usize,
) -> Result<BitmapPreviewData, String> {
    let layer_count = surfaces.layers.len().max(1);
    let layer_index = layer_index.min(layer_count - 1);
    let layer = surfaces
        .layers
        .get(layer_index)
        .ok_or_else(|| "texture has no layers".to_owned())?;
    let mip_count = layer.mips.len().max(1);
    let mip_index = mip_index.min(mip_count - 1);
    let surface = layer
        .mips
        .get(mip_index)
        .ok_or_else(|| format!("texture layer has no mip {mip_index}"))?;
    // Not `to_rgba8`: a mixed-resolution UDIM set has no tiles at this level for
    // its lower-resolution blocks, and those regions must come from the coarser
    // level rather than show as holes.
    let rgba = surfaces
        .display_rgba8(layer_index, mip_index)
        .map_err(|error| format!("{error:#}"))?;

    let mut type_name = String::from("Texture2D");
    if surfaces.is_virtual {
        type_name.push_str(" • virtual");
    }
    if surfaces.is_udim() {
        type_name.push_str(&format!(
            " • {}x{} UDIM",
            surfaces.width_in_blocks, surfaces.height_in_blocks
        ));
    }
    if layer_count > 1 {
        type_name.push_str(&format!(" • layer {layer_index}"));
    }
    if !surface.missing_tiles.is_empty() {
        // Say so rather than let a magnified region read as a decode fault.
        type_name.push_str(" • some blocks magnified (authored lower-resolution)");
    }
    Ok(BitmapPreviewData {
        width: surface.width,
        height: surface.height,
        image_count: layer_count,
        mip_count,
        format_name: surface.pixel_format.clone(),
        type_name,
        rgba,
    })
}

fn chimp_package_type(world: &World, header: &FZenPackageHeader) -> Option<String> {
    let package_name = header.package_name();
    let package_leaf = package_name.rsplit('/').next().unwrap_or_default();
    let classify = |index: usize| {
        let entry = header.export_map.get(index)?;
        let class = world.class_key(header, entry.class_index)?;
        if class.contains('/') || class.contains('#') {
            return None;
        }
        Some(if class.contains("Blueprint") {
            "Blueprint".to_owned()
        } else {
            class
        })
    };
    let primary = header
        .export_map
        .iter()
        .enumerate()
        .find(|(_, export)| {
            export.outer_index.is_null() && header.name_map.get(export.object_name) == package_leaf
        })
        .and_then(|(index, _)| classify(index));
    primary.or_else(|| {
        let roots: Vec<usize> = header
            .export_map
            .iter()
            .enumerate()
            .filter(|(_, export)| export.outer_index.is_null())
            .map(|(index, _)| index)
            .collect();
        roots
            .iter()
            .filter_map(|index| classify(*index))
            .find(|class| class == "Blueprint")
            .or_else(|| roots.into_iter().find_map(classify))
    })
}

pub(super) fn index_chimp_package_types(world: &World) -> ChimpTypeIndex {
    // Most Zen package headers fit in one 64 KiB IoStore compression block.
    // Only retry with the former 1 MiB window (and finally the whole package)
    // for the uncommon large header. This avoids decompressing sixteen blocks
    // for every package while preserving the existing fallback behavior.
    const HEADER_PREFIXES: [usize; 2] = [64 * 1024, 1024 * 1024];
    index_chimp_package_types_with_prefixes(world, &HEADER_PREFIXES)
}

fn index_chimp_package_types_with_prefixes(
    world: &World,
    header_prefixes: &[usize],
) -> ChimpTypeIndex {
    let mut package_types = Vec::with_capacity(world.packages().len());
    let mut type_counts = BTreeMap::new();
    let mut failures = 0usize;
    for package in world.packages() {
        let result = (|| {
            let provider = package.active_provider()?;
            let archive = world.archives().get(provider.container)?;
            let mut header = None;
            for &max_bytes in header_prefixes {
                let prefix = archive.read_prefix(&provider.entry_path, max_bytes).ok()?;
                if let Ok(decoded) = FZenPackageHeader::deserialize(
                    &mut Cursor::new(&prefix),
                    None,
                    CE_TOC_VERSION,
                    CE_HEADER_VERSION,
                    None,
                ) {
                    header = Some(decoded);
                    break;
                }
                if prefix.len() < max_bytes {
                    break;
                }
            }
            let header = header.or_else(|| {
                world.read_provider(provider).ok().and_then(|bytes| {
                    FZenPackageHeader::deserialize(
                        &mut Cursor::new(bytes),
                        None,
                        CE_TOC_VERSION,
                        CE_HEADER_VERSION,
                        None,
                    )
                    .ok()
                })
            })?;
            chimp_package_type(world, &header)
        })();
        if let Some(class) = &result {
            *type_counts.entry(class.clone()).or_insert(0) += 1;
        } else {
            failures += 1;
        }
        package_types.push(result);
    }
    ChimpTypeIndex {
        package_types,
        type_counts,
        failures,
    }
}

/// What a sweep for "who imports this package" found.
#[derive(Clone, Debug, Default)]
pub(in crate::app) struct ChimpReferrerScan {
    /// Packages whose import map names the target, in mount order.
    pub(super) referrers: Vec<String>,
    /// How many packages were examined.
    pub(super) scanned: usize,
    /// How many could not be read, and so could not be ruled out.
    ///
    /// Reported rather than swallowed: "nothing imports this" and "nothing I
    /// could read imports this" are different answers, and only one of them
    /// makes renaming safe.
    pub(super) unreadable: usize,
}

/// Find every mounted package whose import map names `target`.
///
/// A package's imports are the only record of the dependency — there is no
/// reverse index in the paks — so answering this at all means reading every
/// package header. That is the same sweep the type index already performs at
/// mount, and it uses the same prefix ladder: most Zen headers fit in one
/// 64 KiB compression block, so the larger reads are the exception.
pub(super) fn scan_chimp_referrers(world: &World, target: &str) -> ChimpReferrerScan {
    const HEADER_PREFIXES: [usize; 2] = [64 * 1024, 1024 * 1024];
    let mut scan = ChimpReferrerScan::default();
    for package in world.packages() {
        if package.name.eq_ignore_ascii_case(target) {
            continue;
        }
        scan.scanned += 1;
        let header = (|| {
            let provider = package.active_provider()?;
            let archive = world.archives().get(provider.container)?;
            for &max_bytes in &HEADER_PREFIXES {
                let prefix = archive.read_prefix(&provider.entry_path, max_bytes).ok()?;
                if let Ok(decoded) = FZenPackageHeader::deserialize(
                    &mut Cursor::new(&prefix),
                    None,
                    CE_TOC_VERSION,
                    CE_HEADER_VERSION,
                    None,
                ) {
                    return Some(decoded);
                }
                if prefix.len() < max_bytes {
                    break;
                }
            }
            world.read_provider(provider).ok().and_then(|bytes| {
                FZenPackageHeader::deserialize(
                    &mut Cursor::new(bytes),
                    None,
                    CE_TOC_VERSION,
                    CE_HEADER_VERSION,
                    None,
                )
                .ok()
            })
        })();
        let Some(header) = header else {
            scan.unreadable += 1;
            continue;
        };
        if header
            .imported_package_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(target))
        {
            scan.referrers.push(package.name.clone());
        }
    }
    scan
}

pub(super) fn rebuild_chimp_document(
    world: &World,
    document: &ChimpDocument,
) -> Result<(Vec<u8>, blam_tags::iostore::container::header::StoreEntry), String> {
    // A remount can leave a document whose package is no longer provided by any
    // mounted container. Its `provider` addresses a container positionally, so
    // writing through it would land the bytes somewhere they never came from.
    if document.orphaned {
        return Err(format!(
            "{} is no longer in the mounted containers, so it cannot be written back. Its bytes \
             are still here and can be extracted.",
            document.package
        ));
    }
    // Before anything is serialized: a header whose references point outside the
    // tables they name would otherwise be caught by a panic in the name map, or
    // not at all.
    validate_chimp_header(document)?;
    let names = document.header.name_map.copy_raw_names();
    let resolver = world.resolver(&document.header, &document.original, &names);
    let mut payloads = document.payloads.clone();
    for (index, export) in document.exports.iter().enumerate() {
        let (Some(class), Ok(decoded)) = (export.class.as_deref(), &export.decoded) else {
            continue;
        };
        if let ExportBlock::Reflected(block) = &decoded.block {
            validate_chimp_property_block(class, block, world.usmap()).map_err(|error| {
                format!(
                    "Could not validate {} export {}: {error}",
                    document.package, export.object
                )
            })?;
        }
        payloads[index] =
            write_export_in(class, decoded, world.usmap(), Some(&resolver)).map_err(|error| {
                format!(
                    "Could not serialize {} export {}: {error:#}",
                    document.package, export.object
                )
            })?;
    }
    write_package(&document.header, &payloads, CE_HEADER_VERSION)
        .map_err(|error| format!("Could not rebuild {}: {error:#}", document.package))
}

pub(super) fn load_chimp_usmap(path: Option<&Path>) -> Result<Usmap, String> {
    let Some(path) = path else {
        return Usmap::meteorite()
            .map_err(|error| format!("Could not parse bundled Campaign Evolved USMAP: {error:#}"));
    };
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not read USMAP {}: {error}", path.display()))?;
    Usmap::parse(&bytes)
        .map_err(|error| format!("Could not parse USMAP {}: {error:#}", path.display()))
}

fn chimp_class_display_name(class: Option<&str>) -> &str {
    class
        .and_then(|class| {
            class
                .rsplit(|character| character == '.' || character == '/')
                .find(|segment| !segment.is_empty())
        })
        .unwrap_or("Unknown")
}

fn chimp_object_index_json(
    document: &ChimpDocument,
    world: &World,
    index: FPackageObjectIndex,
) -> Value {
    let object_path = match index.kind() {
        FPackageObjectIndexType::ScriptImport => world
            .class_path(index.raw_index())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("script import #{:016X}", index.raw_index())),
        FPackageObjectIndexType::PackageImport => index
            .package_import()
            .and_then(|reference| {
                let package = document
                    .header
                    .imported_package_names
                    .get(reference.imported_package_index as usize)?;
                let hash = document
                    .header
                    .imported_public_export_hashes
                    .get(reference.imported_public_export_hash_index as usize)?;
                Some(format!("{package}#{hash:016X}"))
            })
            .unwrap_or_else(|| format!("package import #{:016X}", index.raw_index())),
        FPackageObjectIndexType::Export => document
            .header
            .export_map
            .get(index.raw_index() as usize)
            .map(|export| {
                format!(
                    "{}.{}",
                    document.package,
                    document.header.name_map.get(export.object_name)
                )
            })
            .unwrap_or_else(|| format!("export #{}", index.raw_index())),
        FPackageObjectIndexType::Null => "None".to_owned(),
    };
    json!({
        "Kind": format!("{:?}", index.kind()),
        "RawIndex": format!("0x{:016X}", index.raw_index()),
        "ObjectPath": object_path,
    })
}

pub(super) fn chimp_document_json(document: &ChimpDocument) -> Value {
    json!({
        "Package": document.package,
        "Source": document.provider.entry_path,
        "Summary": {
            "Exports": document.header.export_map.len(),
            "Imports": document.header.import_map.len(),
            "Names": document.header.name_map.copy_raw_names().len(),
            "OriginalSize": document.original.len(),
        },
        "Imports": document.header.imported_package_names,
        "ExternalDependencies": document.header.external_package_dependencies
            .iter()
            .map(|dependency| format!("{dependency:?}"))
            .collect::<Vec<_>>(),
        "Exports": document.exports.iter().map(|export| {
            json!({
                "Type": chimp_class_display_name(export.class.as_deref()),
                "Name": export.object,
                "Class": export.class,
                "Properties": export.decoded.as_ref().ok()
                    .and_then(Export::properties)
                    .map(chimp_block_json),
                "DecodeError": export.decoded.as_ref().err(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn chimp_metadata_json(document: &ChimpDocument, world: &World) -> Value {
    let header = &document.header;
    let summary = &header.summary;
    let record = world.package(&document.package);
    json!({
        "Summary": {
            "Package": document.package,
            "SourcePackage": header.source_package_name(),
            "PackageFlags": format!("0x{:08X}", summary.package_flags),
            "HeaderSize": summary.header_size,
            "CookedHeaderSize": summary.cooked_header_size,
            "OriginalSize": document.original.len(),
            "HasVersioningInfo": summary.has_versioning_info != 0,
            "IsUnversioned": header.is_unversioned,
            "ContainerHeaderVersion": format!("{:?}", header.container_header_version),
            "ExportCount": header.export_map.len(),
            "ImportCount": header.import_map.len(),
            "NameCount": header.name_map.copy_raw_names().len(),
            "BulkDataCount": header.bulk_data.len(),
            "ImportedPackageCount": header.imported_package_names.len(),
            "ExternalDependencyCount": header.external_package_dependencies.len(),
        },
        "Versioning": {
            "ZenVersion": format!("{:?}", header.versioning_info.zen_version),
            "FileVersionUE4": header.versioning_info.package_file_version.file_version_ue4,
            "FileVersionUE5": header.versioning_info.package_file_version.file_version_ue5,
            "LicenseeVersion": header.versioning_info.licensee_version,
            "CustomVersions": header.versioning_info.custom_versions.iter().map(|version| {
                json!({
                    "Key": format!("{:?}", version.key),
                    "Version": version.version,
                })
            }).collect::<Vec<_>>(),
        },
        "SectionOffsets": summary.section_offsets().iter().map(|(section, offset)| {
            json!({
                "Section": section,
                "Offset": offset,
            })
        }).collect::<Vec<_>>(),
        "NameMap": header.name_map.copy_raw_names(),
        "ImportedPackageNames": header.imported_package_names,
        "ImportedPublicExportHashes": header.imported_public_export_hashes.iter()
            .map(|hash| format!("0x{hash:016X}"))
            .collect::<Vec<_>>(),
        "ImportMap": header.import_map.iter().enumerate().map(|(index, object)| {
            json!({
                "Index": index,
                "Object": chimp_object_index_json(document, world, *object),
            })
        }).collect::<Vec<_>>(),
        "ExportMap": header.export_map.iter().enumerate().map(|(index, export)| {
            json!({
                "Index": index,
                "ObjectName": header.name_map.get(export.object_name).to_string(),
                "Class": world.class_key(header, export.class_index),
                "Outer": chimp_object_index_json(document, world, export.outer_index),
                "Super": chimp_object_index_json(document, world, export.super_index),
                "Template": chimp_object_index_json(document, world, export.template_index),
                "SerialOffset": export.cooked_serial_offset,
                "SerialSize": export.cooked_serial_size,
                "PublicExportHash": format!("0x{:016X}", export.public_export_hash),
                "ObjectFlags": format!("0x{:08X}", export.object_flags),
                "FilterFlags": format!("{:?}", export.filter_flags),
            })
        }).collect::<Vec<_>>(),
        "BulkDataMap": header.bulk_data.iter().enumerate().map(|(index, entry)| {
            json!({
                "Index": index,
                "SerialOffset": entry.serial_offset,
                "DuplicateSerialOffset": entry.duplicate_serial_offset,
                "SerialSize": entry.serial_size,
                "Flags": format!("0x{:08X}", entry.flags),
                "CookedIndex": entry.cooked_index,
            })
        }).collect::<Vec<_>>(),
        "ExternalPackageDependencies": header.external_package_dependencies.iter()
            .map(|dependency| {
                json!({
                    "FromPackageId": format!("0x{:016X}", dependency.from_package_id.0),
                    "ExternalArcs": dependency.external_dependency_arcs.iter().map(|arc| {
                        json!({
                            "FromImportIndex": arc.from_import_index,
                            "FromCommandType": format!("{:?}", arc.from_command_type),
                            "ToExportBundleIndex": arc.to_export_bundle_index,
                        })
                    }).collect::<Vec<_>>(),
                    "LegacyArcs": dependency.legacy_dependency_arcs.iter().map(|arc| {
                        json!({
                            "FromExportBundleIndex": arc.from_export_bundle_index,
                            "ToExportBundleIndex": arc.to_export_bundle_index,
                        })
                    }).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        "ShaderMapHashes": header.shader_map_hashes.iter()
            .map(|hash| format!("{hash:?}"))
            .collect::<Vec<_>>(),
        "PhysicalProviders": record.map(|record| {
            record.providers.iter().rev().map(|provider| {
                let container = &world.containers()[provider.container];
                json!({
                    "Active": record.active_provider() == Some(provider),
                    "Container": container.path.display().to_string(),
                    "EntryPath": provider.entry_path,
                    "ReadOrder": provider.read_order,
                    "RecoveredDirectoryIndex": container.recovered_directory_index,
                })
            }).collect::<Vec<_>>()
        }).unwrap_or_default(),
    })
}

pub(super) fn refresh_chimp_document_text(document: &mut ChimpDocument) {
    document.document_text = serde_json::to_string_pretty(&chimp_document_json(document))
        .unwrap_or_else(|error| format!("Could not render package document: {error}"));
    document.document_lines = ChimpJsonLines::default();
    document.document_text_dirty = false;
}

pub(super) fn refresh_chimp_metadata_text(document: &mut ChimpDocument, world: &World) {
    document.metadata_text = serde_json::to_string_pretty(&chimp_metadata_json(document, world))
        .unwrap_or_else(|error| format!("Could not render package metadata: {error}"));
    document.metadata_lines = ChimpJsonLines::default();
    document.metadata_text_dirty = false;
}

fn chimp_block_json(block: &PropertyBlock) -> Value {
    Value::Object(
        block
            .iter()
            .map(|(name, value)| (name.to_owned(), chimp_value_json(value)))
            .collect(),
    )
}

fn chimp_value_json(value: &PropValue) -> Value {
    match value {
        PropValue::Bool(value) => json!(value),
        PropValue::Int(value) => json!(value),
        PropValue::Float(value) => json!(value),
        PropValue::Name(value) => json!(value.to_string()),
        PropValue::Str(value) => json!(value.to_string()),
        PropValue::Object(value) => json!({"object_index": value}),
        PropValue::SoftObject(value) => json!({
            "package": value.package.to_string(),
            "asset": value.asset.to_string(),
            "sub_path": value.sub_path.to_string(),
        }),
        PropValue::Array(values) | PropValue::Set(values) => {
            Value::Array(values.iter().map(chimp_value_json).collect())
        }
        PropValue::Map(values) => Value::Array(
            values
                .iter()
                .map(|(key, value)| {
                    json!({
                        "key": chimp_value_json(key),
                        "value": chimp_value_json(value),
                    })
                })
                .collect(),
        ),
        PropValue::Struct(block) => chimp_block_json(block),
        PropValue::Raw(bytes) => json!({"unknown_bytes": bytes.len()}),
        other => json!({"type": format!("{other:?}")}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exporters read the mesh kind straight off the exports now, not off
    /// a document's preview. A skeletal mesh package also carries static
    /// pieces, and is still skeletal.
    #[test]
    fn a_packages_mesh_kind_is_read_from_its_export_classes() {
        let export = |class: &str| ChimpExport {
            object: class.to_owned(),
            class: Some(class.to_owned()),
            decoded: Err(String::new()),
        };
        assert_eq!(chimp_mesh_kind(&[export("Texture2D")]), None);
        assert_eq!(
            chimp_mesh_kind(&[export("Material"), export("StaticMesh")]),
            Some(ChimpMeshKind::Static)
        );
        assert_eq!(
            chimp_mesh_kind(&[export("StaticMesh"), export("SkeletalMesh")]),
            Some(ChimpMeshKind::Skeletal)
        );
    }

    /// "Nothing imports this" and "nothing I could read imports this" are
    /// different answers, and a rename is only safe under the first. A scan
    /// that swallowed unreadable packages would report the safe one.
    #[test]
    fn a_referrer_scan_keeps_what_it_could_not_rule_out_separate_from_what_it_cleared() {
        let clean = ChimpReferrerScan {
            referrers: Vec::new(),
            scanned: 100,
            unreadable: 0,
        };
        let partial = ChimpReferrerScan {
            referrers: Vec::new(),
            scanned: 100,
            unreadable: 3,
        };
        // Both found nothing; only one of them looked everywhere.
        assert!(clean.referrers.is_empty() && partial.referrers.is_empty());
        assert_eq!(clean.unreadable, 0);
        assert_ne!(partial.unreadable, 0);
    }

    #[test]
    fn a_mesh_import_is_a_material_by_the_prefix_the_game_uses() {
        assert!(is_chimp_material_package(
            "/Game/Art/Materials/MI_Brute_Body"
        ));
        assert!(is_chimp_material_package("/Game/Art/Materials/M_Master"));
        // Everything else a mesh imports - skeletons, physics, engine content,
        // and the textures themselves - is not a material.
        assert!(!is_chimp_material_package("/Game/Art/Textures/T_Brute_D"));
        assert!(!is_chimp_material_package("/Game/Art/SK_Brute"));
        assert!(!is_chimp_material_package("/Script/Engine"));
        // The prefix is on the leaf, not anywhere in the path.
        assert!(!is_chimp_material_package("/Game/M_Things/SK_Brute"));
    }

    #[test]
    fn bundled_chimp_usmap_loads_and_invalid_custom_file_is_rejected() {
        assert!(load_chimp_usmap(None).is_ok());
        let path =
            std::env::temp_dir().join(format!("baboon-invalid-{}.usmap", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"not a usmap").unwrap();
        let error = load_chimp_usmap(Some(&path)).err().expect("invalid USMAP");
        assert!(error.contains("Could not parse USMAP"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install plus CE_PAKS and CE_USMAP"]
    fn real_custom_usmap_mounts_and_decodes_a_package() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let path = PathBuf::from(std::env::var_os("CE_USMAP").expect("set CE_USMAP"));
        let world = World::open(root, load_chimp_usmap(Some(&path)).unwrap()).unwrap();
        let package = world
            .packages()
            .iter()
            .find(|package| {
                package
                    .name
                    .to_ascii_lowercase()
                    .contains("sm_spiritdropship_body")
            })
            .unwrap_or_else(|| panic!("SM_SpiritDropShip_Body was not found"));
        let document = load_chimp_document(&world, &package.name).unwrap();
        assert!(!document.exports.is_empty());
        assert_eq!(document.mesh_kind, Some(ChimpMeshKind::Static));
    }

    #[test]
    fn raw_values_are_identified_without_embedding_binary_in_json() {
        assert_eq!(
            chimp_value_json(&PropValue::Raw(vec![1, 2, 3])),
            json!({"unknown_bytes": 3})
        );
    }

    #[test]
    fn readable_documents_are_the_default_view() {
        assert_eq!(ChimpDocumentView::default(), ChimpDocumentView::Document);
        assert_eq!(
            chimp_class_display_name(Some("/Script/Engine.BlueprintGeneratedClass")),
            "BlueprintGeneratedClass"
        );
        assert_eq!(chimp_class_display_name(None), "Unknown");
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_fast_type_index_matches_legacy_window() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let fast = index_chimp_package_types_with_prefixes(&world, &[64 * 1024, 1024 * 1024]);
        let legacy = index_chimp_package_types_with_prefixes(&world, &[1024 * 1024]);
        assert_eq!(fast.package_types, legacy.package_types);
        assert_eq!(fast.type_counts, legacy.type_counts);
        assert_eq!(fast.failures, legacy.failures);
    }
}
