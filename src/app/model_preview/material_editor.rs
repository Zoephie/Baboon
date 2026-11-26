//! The Material Editor's view of a preview: the shaders the model draws
//! with, each once, and which of its parts each one covers.
//!
//! A preview's materials are one per element of each material block it
//! read: the render model's, then its LOD model's, the objects it holds and
//! the BSPs a scenario draws, each appended after the last. The same shader
//! is often several of them (two permutations of a Halo CE shader, a weapon
//! the unit shares a shader with, a BSP's), and editing it changes every one,
//! as saving it would. So the editor lists each shader once, as a
//! [`MaterialKey`], with the indices that draw it.
//!
//! The window that lists them and edits their tags is the tag pane's
//! (`editor::material_editor`); this is what it reads from the preview, and
//! what the viewport draws for its hover.

use super::{ModelPreviewLayer, RenderModelPreview};

/// One shader the preview draws with: its group and tag path, the path
/// compared without case or slash style, as tag paths are.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(in crate::app) struct MaterialKey {
    pub(in crate::app) group: u32,
    pub(in crate::app) path: String,
}

impl MaterialKey {
    pub(in crate::app) fn new(group: u32, path: &str) -> Self {
        Self {
            group,
            path: path.replace('/', "\\").to_ascii_lowercase(),
        }
    }
}

/// The Material Editor of one preview, while it is open.
#[derive(Clone, Debug, Default)]
pub(in crate::app) struct MaterialEditorState {
    /// The material it shows.
    pub(in crate::app) selected: Option<MaterialKey>,
    /// The material the pointer is over in its lists, this pass: the
    /// viewport lights its parts.
    pub(in crate::app) hovered: Option<MaterialKey>,
    /// The shader documents a Reset is waiting on a second click to discard.
    pub(in crate::app) confirm_reset: Option<Vec<String>>,
}

/// What keeps a preview's materials as the shader documents its Material
/// Editor holds have them, resolving the materials of one again when it
/// changes rather than loading the model again. Kept while the editor is
/// closed: a shader left with unsaved edits stays held, and drawn so.
#[derive(Default)]
pub(in crate::app) struct MaterialLive {
    /// The key of each material's shader tag in the kit, once looked up,
    /// or why it has none.
    pub(in crate::app) documents: std::collections::BTreeMap<MaterialKey, Result<String, String>>,
    /// Each held document as the drawn materials were resolved from it. A
    /// document missing here is drawn as its file, which a clean one is.
    pub(in crate::app) drawn: std::collections::HashMap<String, DrawnFrom>,
    /// What a picker the editor opened would write now, drawn before OK
    /// writes it.
    pub(in crate::app) draft: Option<DraftPreview>,
    /// A resolve is running: one at a time, so they land in order.
    pub(in crate::app) in_flight: bool,
    /// The caches the editor's resolves of this load share, with the
    /// `geometry_id` of the load they belong to.
    pub(in crate::app::model_preview) caches: Option<(u64, std::sync::Arc<std::sync::Mutex<super::materials::ResolveCaches>>)>,
}

/// What a held document's materials were resolved from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct DrawnFrom {
    /// The document's content stamp, or `None` for its file (the document
    /// gone, its edits discarded).
    pub(in crate::app) stamp: Option<(u64, u64)>,
    /// The picker draft drawn over it, by [`DraftPreview::seq`]; 0 for none.
    pub(in crate::app) draft: u64,
}

impl DrawnFrom {
    pub(in crate::app) fn document(stamp: Option<(u64, u64)>) -> Self {
        Self { stamp, draft: 0 }
    }
}

/// A live picker's draft: the edits OK would make to `document`, drawn
/// over it while the picker is open.
#[derive(Clone)]
pub(in crate::app) struct DraftPreview {
    /// The picker's [`crate::app::editor::LivePreviewTarget::draft_id`].
    pub(in crate::app) draft_id: u64,
    pub(in crate::app) document: String,
    pub(in crate::app) ops: crate::core::document::apply::DeferredOps,
    /// The document's layout stamp the picker opened on: a draft for a
    /// layout since changed is dropped, as OK would refuse it.
    pub(in crate::app) layout_stamp: Option<(u64, u64)>,
    /// Bumped with each draft, so each is drawn.
    pub(in crate::app) seq: u64,
}

impl MaterialLive {
    /// A load has begun: what was resolved belonged to the one before, and
    /// the held documents are drawn as the load reads them, at `stamps`.
    pub(in crate::app) fn restart(&mut self, stamps: std::collections::HashMap<String, Option<(u64, u64)>>) {
        self.drawn = stamps.into_iter().map(|(document, stamp)| (document, DrawnFrom::document(stamp))).collect();
        self.caches = None;
    }
}

/// A shader as the Material Editor lists it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct EditorMaterial {
    pub(in crate::app) key: MaterialKey,
    /// The tag path as the first material naming it writes it.
    pub(in crate::app) path: String,
    /// Its file name, which the list shows.
    pub(in crate::app) label: String,
    /// Where it draws besides the opened model's own parts, which the list
    /// adds after the name: an object it holds, its LOD model.
    pub(in crate::app) places: Vec<String>,
    /// The preview materials that are this shader.
    pub(in crate::app) indices: Vec<u16>,
    /// Drawn over its object's parts rather than on parts of its own: Halo
    /// CE's modifier shader, Halo 2's hologram and shield shaders. Hovering
    /// one lights the object it covers.
    pub(in crate::app) overlay: bool,
}

impl EditorMaterial {
    /// The name with where it draws, as the dropdown shows it.
    pub(in crate::app) fn display_name(&self) -> String {
        if self.places.is_empty() {
            self.label.clone()
        } else {
            format!("{} ({})", self.label, self.places.join(", "))
        }
    }
}

/// The shaders `preview` draws with, in the order it first names them.
/// Materials with no shader (collision, physics) are left out.
pub(in crate::app) fn editor_materials(preview: &RenderModelPreview) -> Vec<EditorMaterial> {
    let mut list: Vec<EditorMaterial> = Vec::new();
    for (index, material) in preview.materials.iter().enumerate() {
        if material.shader_path.trim().is_empty() {
            continue;
        }
        let key = MaterialKey::new(material.shader_group, &material.shader_path);
        let at = match list.iter().position(|listed| listed.key == key) {
            Some(at) => at,
            None => {
                let label = material
                    .shader_path
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or(&material.shader_path)
                    .to_owned();
                list.push(EditorMaterial {
                    key,
                    path: material.shader_path.clone(),
                    label,
                    places: Vec::new(),
                    indices: Vec::new(),
                    overlay: false,
                });
                list.len() - 1
            }
        };
        list[at].indices.push(index as u16);
    }
    for material in &mut list {
        material.overlay = !preview
            .batches
            .iter()
            .any(|batch| material.indices.contains(&batch.material_index));
        material.places = places(preview, material);
    }
    list
}

/// Where `material` draws besides the opened model's own parts.
fn places(preview: &RenderModelPreview, material: &EditorMaterial) -> Vec<String> {
    let mut places = Vec::new();
    let mut add = |place: String| {
        if !places.contains(&place) {
            places.push(place);
        }
    };
    if material.overlay {
        add("overlay".to_owned());
    }
    for batch in preview.batches.iter().filter(|batch| material.indices.contains(&batch.material_index)) {
        let object = preview
            .materials
            .get(usize::from(batch.material_index))
            .map_or(0, |material| material.object);
        if object != 0 {
            // `merge_attached` names an attached object's regions
            // "<object>: <region>".
            if let Some((name, _)) = batch.region_name.split_once(": ") {
                add(name.to_owned());
            }
        } else if batch.layer == ModelPreviewLayer::LodModel {
            add("LOD".to_owned());
        }
    }
    places
}

/// The batches hovering `material` lights: those it draws, or, for an
/// overlay, every part of the objects it covers.
pub(in crate::app) fn highlighted_batches(preview: &RenderModelPreview, material: &EditorMaterial) -> Vec<usize> {
    let owners: Vec<u8> = material
        .indices
        .iter()
        .filter_map(|&index| preview.materials.get(usize::from(index)))
        .map(|material| material.object)
        .collect();
    preview
        .batches
        .iter()
        .enumerate()
        .filter(|(_, batch)| {
            if material.overlay {
                matches!(batch.layer, ModelPreviewLayer::Render | ModelPreviewLayer::LodModel)
                    && preview
                        .materials
                        .get(usize::from(batch.material_index))
                        .is_some_and(|drawn| !drawn.shader_path.is_empty() && owners.contains(&drawn.object))
            } else {
                material.indices.contains(&batch.material_index)
            }
        })
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::{RenderModelPreviewBatch, RenderModelPreviewMaterial};

    fn material(path: &str, object: u8) -> RenderModelPreviewMaterial {
        RenderModelPreviewMaterial {
            shader_path: path.to_owned(),
            shader_group: u32::from_be_bytes(*b"rmsh"),
            object,
            ..Default::default()
        }
    }

    fn batch(region: &str, material_index: u16, layer: ModelPreviewLayer) -> RenderModelPreviewBatch {
        RenderModelPreviewBatch {
            region_name: region.to_owned(),
            material_index,
            layer,
            ..Default::default()
        }
    }

    /// A unit holding a rifle, with an LOD model, a modifier-style overlay
    /// and a collision overlay's empty material.
    fn preview() -> RenderModelPreview {
        RenderModelPreview {
            materials: vec![
                material("objects\\unit\\shaders\\body", 0),
                material("objects\\unit\\shaders\\visor", 0),
                // The LOD model's copy of the body, written another way.
                material("objects/unit/shaders/BODY", 0),
                // The rifle, sharing the unit's visor.
                material("objects\\rifle\\shaders\\rifle", 1),
                material("objects\\unit\\shaders\\visor", 1),
                // Drawn over the unit, with no parts of its own.
                material("objects\\unit\\shaders\\shield", 0),
                // Collision.
                RenderModelPreviewMaterial::default(),
            ],
            batches: vec![
                batch("body", 0, ModelPreviewLayer::Render),
                batch("head", 1, ModelPreviewLayer::Render),
                batch("body", 2, ModelPreviewLayer::LodModel),
                batch("rifle: base", 3, ModelPreviewLayer::Render),
                batch("rifle: scope", 4, ModelPreviewLayer::Render),
                batch("body", 6, ModelPreviewLayer::Collision),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn each_shader_is_listed_once_with_every_material_that_draws_it() {
        let list = editor_materials(&preview());
        let names: Vec<String> = list.iter().map(EditorMaterial::display_name).collect();
        assert_eq!(names, ["body (LOD)", "visor (rifle)", "rifle (rifle)", "shield (overlay)"]);
        assert_eq!(list[0].indices, [0, 2], "the LOD's copy, though written another way");
        assert_eq!(list[1].indices, [1, 4], "the unit's and the rifle's visor");
        assert!(list[3].overlay && !list[0].overlay);
    }

    #[test]
    fn hovering_lights_the_parts_a_shader_draws_or_an_overlays_object() {
        let preview = preview();
        let list = editor_materials(&preview);
        assert_eq!(highlighted_batches(&preview, &list[1]), [1, 4], "both visors");
        assert_eq!(highlighted_batches(&preview, &list[2]), [3]);
        // The overlay is the unit's (object 0): its parts, the LOD's too, but
        // not the rifle's, and not collision.
        assert_eq!(highlighted_batches(&preview, &list[3]), [0, 1, 2]);
    }
}
