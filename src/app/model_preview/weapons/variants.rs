//! A model variant's child objects, `model_variant_create_children`'s list,
//! attached at their markers.

use blam_tags::{TagFile, TagStruct};
use super::super::attachment::MarkerFrame;
use super::super::common::fields::elements;
use super::super::load::build_render_preview;
use super::super::*;
use crate::core::source::TagSource;
use super::attached::{AttachedObject, fold_attached};
use super::holder_of;

/// The first owner a variant's child object takes, after the two hands.
const FIRST_CHILD_OWNER: u8 = 3;

/// One entry of a variant's `objects`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct VariantObject {
    parent_marker: String,
    child_marker: String,
    child_variant: String,
    group: u32,
    path: String,
}

/// Each variant's `objects`, `model_variant_create_children`'s list.
fn variant_objects(model: &TagFile) -> Vec<Vec<VariantObject>> {
    let name = |entry: TagStruct<'_>, field: &str| entry.read_string_id(field).unwrap_or_default();
    elements(model.root(), "variants")
        .iter()
        .map(|variant| {
            let read = |entry: &TagStruct<'_>| {
                let (group, path) = entry.read_tag_ref_with_group("child object")?;
                (!path.is_empty()).then(|| VariantObject {
                    parent_marker: name(*entry, "parent marker"),
                    child_marker: name(*entry, "child marker"),
                    child_variant: name(*entry, "child variant name"),
                    group,
                    path,
                })
            };
            elements(*variant, "objects").iter().filter_map(read).collect()
        })
        .collect()
}

/// A child object's `.model`, and its variant by name, else its own default.
fn child_model(source: &TagSource, entry: &VariantObject) -> Option<(TagFile, TagFile, Option<ModelVariantPreview>)> {
    let extension = blam_tags::paths::group_tag_to_extension(entry.group)?;
    let object = load_referenced_tag_from_source(source, &entry.path, extension, &entry.group.to_be_bytes()).ok()?;
    let model = holder_of(object.root(), "model")?.read_tag_ref_path("model")?;
    let model = load_referenced_tag_from_source(source, &model, "model", b"hlmt").ok()?;
    let chosen = object_variant(&object, &model, &entry.child_variant);
    Some((object, model, chosen))
}

/// The variant of `model` an object is made as: `wanted`, or with none the
/// object's default, falling back to `default` and then the first.
pub(in crate::app::model_preview) fn object_variant(object: &TagFile, model: &TagFile, wanted: &str) -> Option<ModelVariantPreview> {
    let variants = super::super::variants::read_model_variants(model);
    let wanted = match wanted {
        "" => object_default_variant(object).unwrap_or_default(),
        name => name.to_owned(),
    };
    let chosen = variants.iter().find(|variant| variant.name == wanted);
    let chosen = chosen.or(variants.iter().find(|variant| variant.name == "default")).or(variants.first());
    chosen.cloned()
}

/// What a `.model`'s variants attach, such as the warthog's turret: every one
/// goes in once, drawn only under the variants that name it.
pub(in crate::app::model_preview) fn attach_variant_objects(
    source: &TagSource,
    model: &TagFile,
    variants: &mut [ModelVariantPreview],
    preview: &mut RenderModelPreview,
) -> Vec<AttachedObject> {
    let per_variant = variant_objects(model);
    let mut objects: Vec<VariantObject> = Vec::new();
    for entry in per_variant.iter().flatten() {
        if !objects.contains(entry) {
            objects.push(entry.clone());
        }
    }
    let mut attached_objects = Vec::new();
    let mut names: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (index, entry) in objects.iter().enumerate() {
        let Ok(owner) = u8::try_from(index + usize::from(FIRST_CHILD_OWNER)) else {
            break;
        };
        let Some((object, child_tag, chosen)) = child_model(source, entry) else {
            continue;
        };
        let render = child_tag.root().read_tag_ref_path("render model");
        let render = render.and_then(|path| load_referenced_tag_from_source(source, &path, "render_model", b"mode").ok());
        let Some(attached) = render.and_then(|render| build_render_preview(&render).ok()) else {
            continue;
        };
        let parent = MarkerFrame::find(preview, &entry.parent_marker);
        let child = MarkerFrame::find(&attached, &entry.child_marker);
        // Two children of one name (a tag on two markers, or two tags in
        // different folders) keep their regions and effects apart.
        let base = tag_path::name(&entry.path);
        let seen = names.entry(base.to_owned()).or_insert(0);
        *seen += 1;
        let name = if *seen == 1 { base.to_owned() } else { format!("{base} {seen}") };
        let choices: Vec<(String, String)> = attached
            .regions
            .iter()
            .map(|region| {
                let named = chosen.as_ref().and_then(|variant| variant.regions.get(&region.name));
                let permutation = named.cloned().or_else(|| region.permutations.first().cloned());
                (format!("{name}: {}", region.name), permutation.unwrap_or_default())
            })
            .collect();
        let variant = chosen.as_ref().map_or("", |variant| variant.name.as_str());
        let folded =
            fold_attached(source, &object, &child_tag, variant, attached, parent, child, owner, &name, preview);
        attached_objects.extend(folded);
        for (region, permutation) in choices {
            // With no variant chosen, no child shows.
            preview.instances.empty_regions.insert(region.clone());
            for (variant, listed) in variants.iter_mut().zip(&per_variant) {
                if listed.contains(entry) {
                    variant.regions.insert(region.clone(), permutation.clone());
                }
            }
        }
    }
    attached_objects
}
