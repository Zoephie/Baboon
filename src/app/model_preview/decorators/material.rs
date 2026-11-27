//! A decorator set as its own material: its `texture`, which the game
//! samples for colour and cuts out by alpha (`clip(alpha - 0.3)` in Halo 3's
//! `decorators.hlsl` on DX11, 0.5 in Reach's and Halo 4's).

use super::super::load::TagReader;
use super::super::materials::{SlotBitmap, cached_bitmap};
use super::super::{MaterialTextures, ResolveCaches, TextureSlot};
use blam_tags::TagFile;

/// `set`'s texture, as the base map and the map that cuts it out.
pub(in crate::app::model_preview) fn resolve_decorator_material(
    reader: &TagReader,
    set: &TagFile,
    caches: &mut ResolveCaches,
) -> MaterialTextures {
    let path = set
        .root()
        .read_tag_ref_path("texture")
        .map(|path| path.trim().to_owned())
        .unwrap_or_default();
    if path.is_empty() {
        return MaterialTextures::failed("this decorator set names no texture");
    }
    let binding = SlotBitmap {
        path: path.clone(),
        image_index: 0,
        repeat_x: true,
        repeat_y: true,
        scale: [1.0, 1.0],
    };
    let Some(image) = cached_bitmap(reader, &binding, caches) else {
        return MaterialTextures::failed(format!("could not read its texture, {path}"));
    };
    let mut textures = MaterialTextures::default();
    textures.slots[TextureSlot::Base as usize] = Some(image.clone());
    textures.slots[TextureSlot::AlphaTest as usize] = Some(image);
    textures
}
