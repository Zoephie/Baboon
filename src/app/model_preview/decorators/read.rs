//! Reading a decorator set and the meshes its types draw.

use super::super::common::fields::{elements, has_flag};
use super::super::load::TagReader;
use super::lod::{self, Decimation, DetailLevels, LodRule};
use super::{DecoratorMesh, DecoratorSet, DecoratorShader, DecoratorType, Stipple, WindDefinition};
use blam_tags::{TagFieldData, TagFile, TagStruct};
use std::sync::Arc;

/// Reach's and Halo 4's four render model slots, nearest first; Halo 3's
/// one is `render model`.
const LEVEL_FIELDS: [&str; 4] = ["Base", "Lod2", "Lod3", "Lod4"];

/// `rasterizer_globals`' `default bitmaps` element the stipple's pattern is
/// (`rasterizer_stipple_initialize`): Reach's `dither_pattern4`, Halo 4's
/// `dither_pattern3`.
const STIPPLE_BITMAP: usize = 9;

/// The set `tag`, with each type's mesh read from its render model.
/// `notes` gains what the preview shows without, and why.
pub(in crate::app::model_preview) fn read_decorator_set(
    tag: &TagFile,
    reader: Option<&TagReader>,
    notes: &mut Vec<String>,
) -> Result<DecoratorSet, String> {
    let root = tag.root();
    let game = reader.and_then(|reader| reader.source.game());
    // An older layout has no `render shader`: the game's version upgrade
    // leaves it 0, its first option.
    let shader_name = match root.field("render shader") {
        Some(_) => root.read_enum_option("render shader").unwrap_or_default(),
        None => DecoratorShader::first_option(game).name().to_owned(),
    };
    let shader = DecoratorShader::from_option_name(&shader_name);
    if shader.is_none() {
        notes.push(format!(
            "Its render shader, \"{shader_name}\", is one this build does not know."
        ));
    }
    let instance_names: Vec<String> = elements(root, "render model instance names")
        .iter()
        .map(|element| element.read_string_id("name").unwrap_or_default())
        .collect();
    let types: Vec<DecoratorType> = elements(root, "decorator types")
        .iter()
        .map(|element| read_type(*element, &instance_names, LodRule::of(game).is_some()))
        .collect();
    if types.is_empty() {
        return Err("This decorator set has no decorator types.".to_owned());
    }
    let reader = reader
        .ok_or_else(|| "A decorator set's render model needs a loaded editing kit.".to_owned())?;
    let rule = LodRule::of(game);
    // Reach and Halo 4 name four LOD models, nearest first; each that loads
    // is a level, and the first of them is the one up close.
    let mut models = Vec::new();
    let mut paths = Vec::new();
    let mut valid = [false; 4];
    // Each type's subpart, by the nearest model's placement its mesh names.
    let mut subparts: Option<Vec<Option<usize>>> = None;
    let slots: &[&str] = if rule.is_some() { &LEVEL_FIELDS } else { &["render model"] };
    for (slot, name) in slots.iter().enumerate() {
        let path = root
            .read_tag_ref_path(name)
            .map(|path| path.trim().to_owned())
            .unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        match reader.load(&path, "render_model", b"mode") {
            Ok(model) => match read_meshes(&model, &types, &mut subparts, notes) {
                Ok(meshes) => {
                    valid[slot] = true;
                    models.push(meshes);
                    paths.push(path);
                }
                Err(error) if models.is_empty() => return Err(error),
                Err(error) => notes.push(format!("Its {name} model: {error}")),
            },
            Err(error) if slot == 0 && models.is_empty() => {
                return Err(format!("Could not load {path}.render_model: {error}"));
            }
            Err(error) => notes.push(format!("Its {name} model, {path}, did not load: {error}")),
        }
    }
    if models.is_empty() {
        return Err("This decorator set names no render model.".to_owned());
    }
    let meshes = models.remove(0);
    let detail = rule.and_then(|rule| read_detail_levels(root, reader, rule, valid, paths));
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    Ok(DecoratorSet {
        shader,
        two_sided: has_flag(root, "render flags", "render two sided"),
        translucency: [
            "translucency A",
            "translucency B",
            "translucency C",
            "translucency",
        ]
        .map(real),
        wave: [
            "wavelength X",
            "wavelength Y",
            "wave speed",
            "wave frequency",
        ]
        .map(real),
        shaded: ["shaded dark", "shaded bright"].map(real),
        fade_range: root
            .read_real("start fade")
            .zip(root.read_real("end fade"))
            .map(|(start, end)| [start, end]),
        wind: WindDefinition::default_for(reader).map(std::sync::Arc::new),
        types,
        meshes,
        levels: models,
        detail,
        stipple: rule.and_then(|_| read_stipple(reader)).map(Arc::new),
        layout: Default::default(),
        halo2: None,
        halo_ce: None,
        placements: Vec::new(),
    })
}

/// The set's levels as `s_decorator_set::postprocess` leaves them, `valid`
/// saying which slots' models loaded, and the throttle the game scales
/// their distances by.
fn read_detail_levels(
    root: TagStruct<'_>,
    reader: &TagReader,
    rule: LodRule,
    valid: [bool; 4],
    models: Vec<String>,
) -> Option<DetailLevels> {
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let ranges: [[f32; 2]; 4] =
        std::array::from_fn(|slot| [real(&format!("start point{slot}")), real(&format!("end point{slot}"))]);
    let (slots, last) = lod::postprocess(valid, ranges)?;
    let (scale, offset) = lod::decimation_terms(real("decimation start"), real("decimation end"));
    Some(DetailLevels {
        rule,
        slots,
        last,
        decimation: Decimation {
            on: has_flag(root, "render flags", "Use decimation method for random structured decorators"),
            scale,
            offset,
            speed: real("decimation speed"),
            start_distance: real("decimation start distance"),
            decimate_to: real("decimate to"),
        },
        early_cull: real("early cull"),
        block_size: real("cull block size").clamp(lod::BLOCK_SIZE[0], lod::BLOCK_SIZE[1]),
        throttle: decorator_fade_scale(reader).unwrap_or(1.0),
        models,
    })
}

/// The rasterizer globals' default performance throttles' first element's
/// `decorator fade dist scale`: the one-player throttles the game takes
/// outside a level that names its own (`update_current_performance_throttles`).
fn decorator_fade_scale(reader: &TagReader) -> Option<f32> {
    let globals = reader
        .load("globals\\rasterizer_globals", "rasterizer_globals", b"rasg")
        .ok()?;
    let path = globals.root().read_tag_ref_path("default performance throttles")?;
    let throttles = reader.load(path.trim(), "performance_throttles", b"perf").ok()?;
    elements(throttles.root(), "Performance Throttles")
        .first()?
        .read_real("decorator fade dist scale")
}

/// The stipple's pattern: `default bitmaps[9]`'s first image, one value a
/// pixel, which `rasterizer_stipple_initialize` writes into the stencil.
fn read_stipple(reader: &TagReader) -> Option<Stipple> {
    let globals = reader
        .load("globals\\rasterizer_globals", "rasterizer_globals", b"rasg")
        .ok()?;
    let path = elements(globals.root(), "default bitmaps")
        .get(STIPPLE_BITMAP)?
        .read_tag_ref_path("default bitmaps")?;
    let tag = reader.load(path.trim(), "bitmap", b"bitm").ok()?;
    let bitmap = blam_tags::bitmap::Bitmap::new(&tag).ok()?;
    let image = bitmap.image(0)?;
    let (width, height) = (image.width(), image.height());
    let rgba = blam_tags::bitmap::decode::decode_to_rgba8(
        image.format().ok()?,
        width,
        height,
        image.pixel_bytes().ok()?,
        bitmap.p8_palette(),
    )
    .ok()?;
    let values: Vec<u8> = rgba.chunks_exact(4).map(|texel| texel[0]).collect();
    (width > 0 && height > 0 && values.len() == (width * height) as usize).then(|| Stipple {
        size: [width as usize, height as usize],
        values,
    })
}

/// A type; `placed_by_the_game` for Reach and Halo 4, whose placement
/// (`place_randomized_decorator`) reads `align random` (Halo 3's is
/// Sapien's, unread).
fn read_type(element: TagStruct<'_>, instance_names: &[String], placed_by_the_game: bool) -> DecoratorType {
    let real = |name: &str| element.read_real(name).unwrap_or(0.0);
    let range = |low: &str, high: &str| [real(low), real(high)];
    let mesh_name = usize::try_from(element.read_int_any("mesh").unwrap_or(-1))
        .ok()
        .and_then(|index| instance_names.get(index))
        .cloned()
        .unwrap_or_default();
    DecoratorType {
        mesh_name,
        random_rotation: has_flag(element, "flags", "random rotation"),
        align_random: placed_by_the_game
            && has_flag(element, "flags", "align random")
            && !has_flag(element, "flags", "align to normal"),
        scale: range("scale min", "scale max"),
        tilt: range("tilt min", "tilt max"),
        wind: range("wind min", "wind max"),
        colors: ["color 0", "color 1", "color 2"].map(|name| rgb(element, name)),
        ground_tint: range("ground tint min", "ground tint max"),
        hover: range("hover min", "hover max"),
        minimum_distance: real("minimum distance between decorators"),
        placement_weight: element.read_real("placement weight"),
    }
}

/// A colour field, black where the layout has none of that name.
fn rgb(element: TagStruct<'_>, name: &str) -> [f32; 3] {
    match element.field(name).and_then(|field| field.value()) {
        Some(TagFieldData::RealRgbColor(color)) => [color.red, color.green, color.blue],
        _ => [0.0; 3],
    }
}

/// Each type's mesh. A type's `mesh` names one of the nearest render model's
/// instance placements, and each placement draws the instance mesh's subpart
/// of its own index (`link_render_model` matches them by name). A further
/// level draws the same subpart of its own mesh (`c_decorator_lod_renderer`
/// steps through a level's subparts by type), whatever its placements are
/// named: `subparts` holds each type's, found by the first model read.
fn read_meshes(
    model: &TagFile,
    types: &[DecoratorType],
    subparts: &mut Option<Vec<Option<usize>>>,
    notes: &mut Vec<String>,
) -> Result<Vec<DecoratorMesh>, String> {
    let geometry = blam_tags::render_model::extract_decorator_subparts(model)
        .ok_or_else(|| "Its render model has no decorator mesh.".to_owned())?;
    let strips = decorator_strips(model).unwrap_or_default();
    let placements: Vec<String> = elements(model.root(), "instance placements")
        .iter()
        .map(|element| element.read_string_id("name").unwrap_or_default())
        .collect();
    if let Some(subparts) = subparts {
        return Ok(subparts
            .iter()
            .map(|subpart| {
                subpart
                    .and_then(|index| strips.get(index))
                    .map_or_else(DecoratorMesh::default, |strip| subpart_mesh(&geometry.vertices, strip))
            })
            .collect());
    }
    let mut missing = Vec::new();
    let mut unnamed = Vec::new();
    let mut found = Vec::new();
    let meshes = types
        .iter()
        .enumerate()
        .map(|(type_index, kind)| {
            if kind.mesh_name.is_empty() {
                unnamed.push(type_index.to_string());
                found.push(None);
                return DecoratorMesh::default();
            }
            let subpart = placements
                .iter()
                .position(|name| name.eq_ignore_ascii_case(&kind.mesh_name));
            found.push(subpart);
            let Some(strip) = subpart.and_then(|index| strips.get(index)) else {
                missing.push(kind.mesh_name.clone());
                return DecoratorMesh::default();
            };
            subpart_mesh(&geometry.vertices, strip)
        })
        .collect();
    *subparts = Some(found);
    // `link_render_model` rebuilds the names from the model's placements and
    // moves a name the model has lost past them, where its type draws nothing.
    if !missing.is_empty() {
        notes.push(format!(
            "Its render model no longer has {}, so the game draws none of those types either.",
            missing
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !unnamed.is_empty() {
        notes.push(format!(
            "Type {} names no mesh, so it draws nothing.",
            unnamed.join(", ")
        ));
    }
    Ok(meshes)
}

/// Each subpart of the decorator mesh's strip, as the game draws it: the
/// mesh's `raw indices` (or `raw indices32`) from the subpart's `index
/// start` for its `index count`. `extract_decorator_subparts` reads the same
/// indices but hands back triangles; the game draws the strip, and
/// decimation draws its first vertices only.
fn decorator_strips(model: &TagFile) -> Option<Vec<Vec<u32>>> {
    let root = model.root();
    let temporary = elements_at(root, "render geometry/per mesh temporary");
    let temporary = temporary.first()?;
    let read = |block: &str, field: &str, restart: u32| {
        let indices = elements(*temporary, block);
        (!indices.is_empty()).then(|| {
            indices
                .iter()
                .map(|element| match element.read_int_any(field).unwrap_or(0) as u32 & restart {
                    index if index == restart => u32::MAX,
                    index => index,
                })
                .collect::<Vec<u32>>()
        })
    };
    let indices = read("raw indices", "word", 0xFFFF).or_else(|| read("raw indices32", "dword", u32::MAX))?;
    let meshes = elements_at(root, "render geometry/meshes");
    let subparts = elements(*meshes.first()?, "subparts");
    Some(
        subparts
            .iter()
            .map(|subpart| {
                // Halo 3 declares `index start` a short, so past 32,767 it
                // reads back negative.
                let start = subpart.read_int_any("index start").unwrap_or(0);
                let start = if start < 0 { start as i16 as u16 as usize } else { start as usize };
                let count = subpart.read_int_any("index count").unwrap_or(0).max(0) as usize;
                let end = (start + count).min(indices.len());
                indices.get(start..end).unwrap_or_default().to_vec()
            })
            .collect(),
    )
}

/// The elements of the block at `path` below `holder`.
fn elements_at<'a>(holder: TagStruct<'a>, path: &str) -> Vec<TagStruct<'a>> {
    let block = holder.field_path(path).and_then(|field| field.as_block());
    (0..block.as_ref().map_or(0, |block| block.len()))
        .filter_map(|index| block.as_ref()?.element(index))
        .collect()
}

/// The vertices `strip` names, each once, the strip renumbered, and its
/// triangles. A strip naming a vertex the mesh lacks draws nothing.
fn subpart_mesh(
    vertices: &[blam_tags::render_model::RenderVertex],
    strip: &[u32],
) -> DecoratorMesh {
    let mut mesh = DecoratorMesh::default();
    if strip
        .iter()
        .any(|&index| index != u32::MAX && index as usize >= vertices.len())
    {
        return mesh;
    }
    let mut remapped = std::collections::HashMap::new();
    for &index in strip {
        if index == u32::MAX {
            mesh.strip.push(u32::MAX);
            continue;
        }
        let next = *remapped.entry(index).or_insert_with(|| {
            let vertex = &vertices[index as usize];
            mesh.positions
                .push([vertex.position.x, vertex.position.y, vertex.position.z]);
            mesh.normals
                .push([vertex.normal.i, vertex.normal.j, vertex.normal.k]);
            mesh.texcoords.push([vertex.texcoord.x, vertex.texcoord.y]);
            (mesh.positions.len() - 1) as u32
        });
        mesh.strip.push(next);
    }
    mesh.indices = blam_tags::geometry::strip_to_list_u32(&mesh.strip)
        .into_iter()
        .flat_map(|(a, b, c)| [a, b, c])
        .collect();
    mesh
}
