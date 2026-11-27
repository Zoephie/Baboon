//! Reading a Halo 2 decorator set, its shaders' sprite plates too.

use super::super::super::common::fields::{elements, has_flag};
use super::super::super::load::TagReader;
use super::{Class, ClassKind, Halo2Set, Permutation, Plate, RawVertex, Sprite};
use blam_tags::bitmap::BitmapDefinition;
use blam_tags::{TagFieldData, TagFile, TagStruct};

/// The set `tag`; `notes` gains what the preview shows without, and why.
pub(in crate::app::model_preview) fn read_halo_2_set(
    tag: &TagFile,
    reader: Option<&TagReader>,
    notes: &mut Vec<String>,
) -> Result<Halo2Set, String> {
    let root = tag.root();
    let shaders: Vec<String> = elements(root, "shaders")
        .iter()
        .map(|element| {
            element
                .read_tag_ref_path("shader")
                .map(|path| path.trim().to_owned())
                .unwrap_or_default()
        })
        .collect();
    let classes: Vec<Class> = elements(root, "classes")
        .iter()
        .map(|element| read_class(*element, notes))
        .collect();
    if classes.iter().all(|class| class.permutations.is_empty()) {
        return Err("This decorator set has no permutations.".to_owned());
    }
    let models = elements(root, "models")
        .iter()
        .map(|model| {
            let start = model.read_int_any("index start").unwrap_or(0).max(0) as usize;
            let count = model.read_int_any("index count").unwrap_or(0).max(0) as usize;
            start..start + count
        })
        .collect();
    let vertices = elements(root, "raw vertices")
        .iter()
        .map(|vertex| read_vertex(*vertex))
        .collect();
    let indices = elements(root, "indices")
        .iter()
        // `short_integer`s, read unsigned as an index buffer of `u16`s is.
        .map(|index| index.read_int_any("index").unwrap_or(0) as u16 as u32)
        .collect();
    let plates = shaders
        .iter()
        .map(|shader| reader.and_then(|reader| read_plate(reader, shader)))
        .collect();
    Ok(Halo2Set {
        shaders,
        plates,
        classes,
        models,
        vertices,
        indices,
        records: Vec::new(),
    })
}

fn read_class(element: TagStruct<'_>, notes: &mut Vec<String>) -> Class {
    let name = element.read_string_id("name").unwrap_or_default();
    let kind_name = element.read_enum_option("type").unwrap_or_default();
    let kind = ClassKind::from_option_name(&kind_name);
    if kind.is_none() {
        notes.push(format!(
            "Class \"{name}\" is a \"{kind_name}\", which this build does not know."
        ));
    }
    Class {
        name,
        kind,
        permutations: elements(element, "permutations")
            .iter()
            .map(|permutation| read_permutation(*permutation))
            .collect(),
    }
}

fn read_permutation(element: TagStruct<'_>) -> Permutation {
    let scale = match element.field("scale").and_then(|field| field.value()) {
        Some(TagFieldData::RealBounds(bounds)) => [bounds.lower, bounds.upper],
        _ => [0.0; 2],
    };
    Permutation {
        name: element.read_string_id("name").unwrap_or_default(),
        shader: usize::try_from(element.read_int_any("shader").unwrap_or(-1)).ok(),
        align_to_normal: has_flag(element, "flags", "align to normal"),
        index: element.read_int_any("index").unwrap_or(0).max(0) as usize,
        distribution_weight: element
            .read_int_any("distribution weight")
            .unwrap_or(0)
            .max(0) as f32,
        scale,
        tints: ["tint 1", "tint 2"].map(|name| pixel_color(element, name)),
        base_map_tint: element
            .read_real("base map tint percentage")
            .unwrap_or(0.0)
            .clamp(0.0, 1.0),
        lightmap_tint: element
            .read_real("lightmap tint percentage")
            .unwrap_or(0.0)
            .clamp(0.0, 1.0),
        fade: element
            .read_enum_option("fade distance")
            .and_then(|name| match name.as_str() {
                "close" => Some(0.375),
                "medium" => Some(0.1875),
                "far" => Some(0.125),
                _ => None,
            }),
    }
}

/// An `rgb_color`, a pixel32 (`pixel32_to_real_rgb_color`).
fn pixel_color(element: TagStruct<'_>, name: &str) -> [f32; 3] {
    match element.field(name).and_then(|field| field.value()) {
        Some(TagFieldData::RgbColor(color)) => {
            [16, 8, 0].map(|shift| ((color.0 >> shift) & 0xFF) as f32 / 255.0)
        }
        _ => [0.0; 3],
    }
}

fn read_vertex(element: TagStruct<'_>) -> RawVertex {
    let three = |name: &str| match element.field(name).and_then(|field| field.value()) {
        Some(TagFieldData::RealPoint3d(point)) => [point.x, point.y, point.z],
        Some(TagFieldData::RealVector3d(vector)) => [vector.i, vector.j, vector.k],
        _ => [0.0; 3],
    };
    let texcoord = match element.field("texcoord").and_then(|field| field.value()) {
        Some(TagFieldData::RealPoint2d(point)) => [point.x, point.y],
        _ => [0.0; 2],
    };
    RawVertex {
        position: three("position"),
        normal: three("normal"),
        tangent: three("tangent"),
        binormal: three("binormal"),
        texcoord,
    }
}

/// The sprites of the base map `shader`'s `parameters` name.
fn read_plate(reader: &TagReader, shader: &str) -> Option<Plate> {
    let shader = reader.load(shader, "shader", b"shad").ok()?;
    let parameters = elements(shader.root(), "parameters");
    let path = parameters
        .iter()
        .find(|parameter| parameter.read_string_id("name").as_deref() == Some("base_map"))?
        .read_tag_ref_path("bitmap")
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())?;
    let bitmap = reader.load(&path, "bitmap", b"bitm").ok()?;
    let definition = BitmapDefinition::from_tag(&bitmap);
    let size = definition.images.first().map_or([0.0; 2], |image| {
        [f32::from(image.width), f32::from(image.height)]
    });
    let sequences = definition
        .sequences
        .iter()
        .map(|sequence| {
            sequence
                .sprites
                .iter()
                .map(|sprite| Sprite {
                    bounds: sprite.rect(),
                    registration: sprite.registration_point,
                })
                .collect()
        })
        .collect();
    Some(Plate { sequences, size })
}
