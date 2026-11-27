//! Reading a Halo CE detail object collection and its sprite plate.

use super::super::super::common::fields::elements;
use super::super::super::load::TagReader;
use super::{DetailType, HaloCeCollection, Plate, Sprite};
use blam_tags::TagFile;
use blam_tags::bitmap::BitmapDefinition;

/// The collection `tag`.
pub(in crate::app::model_preview) fn read_halo_ce_collection(
    tag: &TagFile,
    reader: Option<&TagReader>,
) -> Result<HaloCeCollection, String> {
    let root = tag.root();
    let types: Vec<DetailType> = elements(root, "types")
        .iter()
        .map(|element| DetailType {
            name: element.read_string("name").unwrap_or_default(),
            sequence: element.read_int_any("sequence index").unwrap_or(0).max(0) as usize,
            size: element.read_real("size").unwrap_or(0.0),
        })
        .collect();
    if types.is_empty() {
        return Err("This detail object collection has no types.".to_owned());
    }
    let plate_path = root
        .read_tag_ref_path("sprite plate")
        .map(|path| path.trim().to_owned())
        .unwrap_or_default();
    if plate_path.is_empty() {
        return Err("This detail object collection names no sprite plate.".to_owned());
    }
    let reader = reader.ok_or_else(|| {
        "A detail object collection's sprite plate needs a loaded editing kit.".to_owned()
    })?;
    let bitmap = reader
        .load(&plate_path, "bitmap", b"bitm")
        .map_err(|error| format!("Could not load {plate_path}.bitmap: {error}"))?;
    let definition = BitmapDefinition::from_tag(&bitmap);
    let plate = Plate {
        sequences: definition
            .sequences
            .iter()
            .map(|sequence| {
                sequence
                    .sprites
                    .iter()
                    .map(|sprite| Sprite {
                        image: sprite.bitmap_index.max(0) as usize,
                        bounds: sprite.rect(),
                    })
                    .collect()
            })
            .collect(),
        images: definition
            .images
            .iter()
            .map(|image| [f32::from(image.width), f32::from(image.height)])
            .collect(),
    };
    Ok(HaloCeCollection {
        z_offset: root.read_real("global z offset").unwrap_or(0.0),
        plate_path,
        plate,
        types,
        instances: Vec::new(),
    })
}
