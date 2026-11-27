//! Object/model summaries and reference presentation.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

/// Whether a group is a kind of object, by the ancestry in its game's
/// definitions, so a subclass a game adds comes along without being listed.
/// A single file or a monolithic cache has no game to ask, so any game's word
/// is taken; no group code that means two things across games is an object.
pub(in crate::app) fn is_object_family_group(group_tag: u32, game: Option<GameId>) -> bool {
    let object = u32::from_be_bytes(*b"obje");
    match game {
        Some(_) => bundled_group_hierarchy(game).is_a(group_tag, object),
        None => GameId::ALL.iter().any(|game| bundled_group_hierarchy(Some(*game)).is_a(group_tag, object)),
    }
}

/// Where a child group's opening parent struct leaves the object's fields.
const OBJECT_PARENTS: [&str; 4] = ["unit/object", "item/object", "device/object", "object"];

/// An object tag's own reference (Halo CE's `model`, `animation graph`,
/// `collision model`, …; a `.model` after that). A child group opens with its parent's struct, so
/// the object fields sit under `unit/object`, `item/object`, `device/object`
/// or, for a direct child, `object`.
pub(in crate::app) fn object_reference(tag: &TagFile, field: &str) -> Option<String> {
    let root = tag.root();
    OBJECT_PARENTS
        .iter()
        .find_map(|parent| tag_ref_path(&root, &format!("{parent}/{field}")))
}

/// The variant an object starts in, when it names one.
pub(in crate::app) fn object_default_variant(tag: &TagFile) -> Option<String> {
    let root = tag.root();
    OBJECT_PARENTS.iter().find_map(|parent| {
        let field = root.field_path(&format!("{parent}/default model variant"))?;
        match field.value()? {
            TagFieldData::StringId(id) | TagFieldData::OldStringId(id) if !id.string.is_empty() => {
                Some(id.string)
            }
            _ => None,
        }
    })
}

pub(in crate::app) fn is_model_group(group_tag: u32, names: &TagNameIndex) -> bool {
    // By FOURCC and the kit's game's name for it: Halo CE names its legacy
    // render geometry `mode` "model" too, and that is not a model wrapper.
    crate::app::browser::group_in(
        group_tag,
        names.name_for(group_tag),
        &[
            (b"hlmt", "model"),
            // Halo CE has no `.model` (hlmt) wrapper — objects reference a
            // `.gbxmodel` (mod2) directly, which IS the render geometry, so
            // treat it as previewable in its own right.
            (b"mod2", "gbxmodel"),
        ],
    )
}

/// Tags that get the Fields / Model Preview tab pair and a geometry
/// viewport — everything [`is_model_group`] covers, plus geometry-bearing
/// tags that are not "models" in the object sense.
///
/// Kept separate from [`is_model_group`] on purpose: that predicate names
/// an object's model wrapper, and a `particle`'s `Model` → `pmdf` field is
/// not one.
pub(in crate::app) fn is_previewable_geometry_group(group_tag: u32, names: &TagNameIndex) -> bool {
    use crate::app::browser::{COLLISION_GROUPS, RENDER_GEOMETRY_GROUPS, group_in};
    // The group as the kit's game names it: Halo CE's `mode` is its legacy
    // `model`, which previews as render geometry, and its `coll` is
    // `model_collision_geometry`.
    let name = names.name_for(group_tag);
    is_model_group(group_tag, names)
        || blam_tags::is_particle_model_group(group_tag)
        // A bare `render_model` (mode) IS the render geometry — the preview
        // loader draws the tag itself instead of chasing an hlmt's reference.
        // Not part of `is_model_group` for the same reason particle models are
        // not: an hlmt's own `render model` field must not read as an object's
        // model link.
        || group_in(group_tag, name, RENDER_GEOMETRY_GROUPS)
        // Derived previews: collision BSPs walked into triangles, physics
        // primitives tessellated, structure BSPs and scenario composites
        // through the ASS/JMS scene builders. Same tab pair, different
        // geometry source (`model_preview::derived`).
        || group_in(group_tag, name, COLLISION_GROUPS)
        || group_in(
            group_tag,
            name,
            &[
                (b"phmo", "physics_model"),
                (b"sbsp", "scenario_structure_bsp"),
                (b"scnr", "scenario"),
                (b"impo", "imposter_model"),
                (b"dctr", "decorator_set"),
                (b"DECR", "decorator_set"),
                (b"dobc", "detail_object_collection"),
            ],
        )
}

/// An object previews what it references: its gbxmodel on Halo CE, its
/// `.model` after that. Either way that takes a kit to read it from.
pub(in crate::app) fn is_previewable_geometry_group_for_game(
    group_tag: u32,
    names: &TagNameIndex,
    source_game: Option<GameId>,
) -> bool {
    is_previewable_geometry_group(group_tag, names)
        || (source_game.is_some() && is_object_family_group(group_tag, source_game))
}

pub(in crate::app) fn format_reference_path(
    names: &TagNameIndex,
    group_tag: u32,
    path: &str,
) -> String {
    if let Some(extension) = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
    {
        format!("{path}.{extension}")
    } else {
        format!("{}:{path}", format_group_tag(group_tag))
    }
}

pub(in crate::app) fn draw_tag_metadata(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Header group:").color(subtle_dark()));
        ui.monospace(RichText::new(group_label(names, tag.group().tag)).color(text_dark()));
        ui.label(RichText::new("Version:").color(subtle_dark()));

        ui.monospace(RichText::new(tag.group().version.to_string()).color(text_dark()));
        ui.label(RichText::new("Endian:").color(subtle_dark()));
        ui.monospace(
            RichText::new(match tag.endian {
                Endian::Le => "LE",
                Endian::Be => "BE",
            })
            .color(text_dark()),
        );
        // The fields below are live either way; this says what happens to what
        // is typed into them, at the one moment the user is looking at where
        // this tag came from.
        if let Some(reason) = unsaveable_reason(entry, tag) {
            ui.label(
                RichText::new("Cannot be saved")
                    .color(read_only_badge())
                    .strong(),
            )
            .on_hover_text(reason);
        }
    });
}

#[cfg(test)]
mod object_family_tests {
    use super::*;

    fn group(code: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*code)
    }

    #[test]
    fn a_group_is_an_object_by_its_games_ancestry() {
        let game = Some(GameId::Halo3);
        for code in [b"bipd", b"vehi", b"scen", b"bloc", b"proj", b"efsc", b"crea", b"term", b"obje"] {
            assert!(is_object_family_group(group(code), game), "{code:?}");
        }
        for code in [b"hlmt", b"mode", b"effe", b"scnr", b"bitm"] {
            assert!(!is_object_family_group(group(code), game), "{code:?}");
        }
    }

    #[test]
    fn a_group_only_another_game_has_is_not_an_object_here() {
        assert!(is_object_family_group(group(b"argd"), Some(GameId::Halo3Odst)));
        assert!(!is_object_family_group(group(b"argd"), Some(GameId::Halo3)));
        assert!(!is_object_family_group(group(b"efsc"), Some(GameId::HaloCe)));
    }

    #[test]
    fn with_no_game_any_games_word_will_do() {
        assert!(is_object_family_group(group(b"argd"), None));
        assert!(is_object_family_group(group(b"bipd"), None));
        assert!(!is_object_family_group(group(b"bitm"), None));
    }
}
