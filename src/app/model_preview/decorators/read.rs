//! Reading a decorator set and the meshes its types draw.

use super::super::common::fields::{elements, has_flag};
use super::super::load::TagReader;
use super::{DecoratorMesh, DecoratorSet, DecoratorShader, DecoratorType, WindDefinition};
use blam_tags::{TagFieldData, TagFile, TagStruct};

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
        .map(|element| read_type(*element, &instance_names))
        .collect();
    if types.is_empty() {
        return Err("This decorator set has no decorator types.".to_owned());
    }
    // Reach and Halo 4 name four LOD models; the first is the one up close.
    let render_model = ["render model", "Base"]
        .iter()
        .find_map(|name| root.read_tag_ref_path(name))
        .map(|path| path.trim().to_owned())
        .unwrap_or_default();
    if render_model.is_empty() {
        return Err("This decorator set names no render model.".to_owned());
    }
    let reader = reader
        .ok_or_else(|| "A decorator set's render model needs a loaded editing kit.".to_owned())?;
    let model = reader
        .load(&render_model, "render_model", b"mode")
        .map_err(|error| format!("Could not load {render_model}.render_model: {error}"))?;
    let meshes = read_meshes(&model, &types, notes)?;
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    Ok(DecoratorSet {
        shader,
        two_sided: has_flag(root, "render flags", "render two sided"),
        texture: root
            .read_tag_ref_path("texture")
            .map(|path| path.trim().to_owned())
            .unwrap_or_default(),
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
        wind: WindDefinition::default_for(reader).map(std::sync::Arc::new),
        types,
        meshes,
        layout: Default::default(),
        halo2: None,
        halo_ce: None,
        placements: Vec::new(),
    })
}

fn read_type(element: TagStruct<'_>, instance_names: &[String]) -> DecoratorType {
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

/// Each type's mesh. A type's `mesh` names one of the render model's instance
/// placements, and each placement draws the instance mesh's subpart of its
/// own index (`link_render_model` matches them by name).
fn read_meshes(
    model: &TagFile,
    types: &[DecoratorType],
    notes: &mut Vec<String>,
) -> Result<Vec<DecoratorMesh>, String> {
    let geometry = blam_tags::render_model::extract_decorator_subparts(model)
        .ok_or_else(|| "Its render model has no decorator mesh.".to_owned())?;
    let placements: Vec<String> = elements(model.root(), "instance placements")
        .iter()
        .map(|element| element.read_string_id("name").unwrap_or_default())
        .collect();
    let mut missing = Vec::new();
    let mut unnamed = Vec::new();
    let meshes = types
        .iter()
        .enumerate()
        .map(|(type_index, kind)| {
            if kind.mesh_name.is_empty() {
                unnamed.push(type_index.to_string());
                return DecoratorMesh::default();
            }
            let subpart = placements
                .iter()
                .position(|name| name.eq_ignore_ascii_case(&kind.mesh_name))
                .and_then(|index| geometry.subpart_indices.get(index));
            let Some(indices) = subpart else {
                missing.push(kind.mesh_name.clone());
                return DecoratorMesh::default();
            };
            subpart_mesh(&geometry.vertices, indices)
        })
        .collect();
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

/// The vertices `indices` name, each once, and the triangles renumbered.
fn subpart_mesh(
    vertices: &[blam_tags::render_model::RenderVertex],
    indices: &[u32],
) -> DecoratorMesh {
    let mut mesh = DecoratorMesh::default();
    let mut remapped = std::collections::HashMap::new();
    for triangle in indices.chunks_exact(3) {
        if triangle
            .iter()
            .any(|&index| index as usize >= vertices.len())
        {
            continue;
        }
        for &index in triangle {
            let next = *remapped.entry(index).or_insert_with(|| {
                let vertex = &vertices[index as usize];
                mesh.positions
                    .push([vertex.position.x, vertex.position.y, vertex.position.z]);
                mesh.normals
                    .push([vertex.normal.i, vertex.normal.j, vertex.normal.k]);
                mesh.texcoords.push([vertex.texcoord.x, vertex.texcoord.y]);
                (mesh.positions.len() - 1) as u32
            });
            mesh.indices.push(next);
        }
    }
    mesh
}
