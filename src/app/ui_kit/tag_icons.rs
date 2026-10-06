//! Embedded tag-group icon lookup and display-scale selection.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

/// A tag-group icon: its name, which keys its texture, and its SVG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct TagIcon {
    pub(in crate::app) name: &'static str,
    pub(in crate::app) svg: &'static str,
}

macro_rules! tag_icon {
    ($name:literal) => {
        TagIcon {
            name: $name,
            svg: include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/icons/",
                $name,
                ".svg"
            )),
        }
    };
}

const DEFAULT_ICON: TagIcon = tag_icon!("default_tag");
const SHADER_ICON: TagIcon = tag_icon!("shader");

/// Each icon with the groups it stands for, each as its FOURCC and the
/// group's name. A FOURCC alone is not enough: games reuse them for other
/// groups (`gldf` is Halo 2's `chocolate_mountain` but Reach's
/// `cheap_light`), so a group takes an icon only where its game gives the
/// FOURCC that name. Halo CE's `mode` is named `model` there, and draws as
/// the render model it is.
/// A group as its FOURCC and its name.
type IconGroup = (&'static [u8; 4], &'static str);

const GROUP_ICONS: &[(TagIcon, &[IconGroup])] = &[
    (tag_icon!("actor"), &[(b"actr", "actor")]),
    (tag_icon!("actor_variant"), &[(b"actv", "actor_variant")]),
    (
        tag_icon!("model_animations"),
        &[(b"antr", "model_animations")],
    ),
    (
        tag_icon!("animation_graph"),
        &[(b"jmad", "model_animation_graph")],
    ),
    (tag_icon!("biped"), &[(b"bipd", "biped")]),
    (tag_icon!("bitmap"), &[(b"bitm", "bitmap")]),
    (tag_icon!("camera_track"), &[(b"trak", "camera_track")]),
    (tag_icon!("character"), &[(b"char", "character")]),
    (
        tag_icon!("chocolate_mountain"),
        &[
            (b"gldf", "chocolate_mountain"),
            (b"chmt", "chocolate_mountain_new"),
        ],
    ),
    (
        tag_icon!("collision_model"),
        &[
            (b"coll", "collision_model"),
            (b"coll", "model_collision_geometry"),
        ],
    ),
    (tag_icon!("crate"), &[(b"bloc", "crate")]),
    (tag_icon!("damage_effect"), &[(b"jpt!", "damage_effect")]),
    (tag_icon!("default_globals"), &[(b"matg", "globals")]),
    (tag_icon!("device_control"), &[(b"ctrl", "device_control")]),
    (tag_icon!("device_machine"), &[(b"mach", "device_machine")]),
    (tag_icon!("dialogue"), &[(b"udlg", "dialogue")]),
    (tag_icon!("effect"), &[(b"effe", "effect")]),
    (tag_icon!("equipment"), &[(b"eqip", "equipment")]),
    (tag_icon!("garbage"), &[(b"garb", "garbage")]),
    (
        tag_icon!("hud_definition"),
        &[
            (b"hudg", "hud_globals"),
            (b"nhdt", "new_hud_definition"),
            (b"chdt", "chud_definition"),
            (b"chgd", "chud_globals_definition"),
        ],
    ),
    (tag_icon!("lens_flare"), &[(b"lens", "lens_flare")]),
    (tag_icon!("light"), &[(b"ligh", "light")]),
    (tag_icon!("model"), &[(b"hlmt", "model")]),
    (
        tag_icon!("render_model"),
        &[
            (b"mod2", "gbxmodel"),
            (b"mode", "render_model"),
            (b"mode", "model"),
        ],
    ),
    (
        tag_icon!("physics_model"),
        &[(b"phys", "physics"), (b"phmo", "physics_model")],
    ),
    (tag_icon!("projectile"), &[(b"proj", "projectile")]),
    (tag_icon!("scenario"), &[(b"scnr", "scenario")]),
    (tag_icon!("scenery"), &[(b"scen", "scenery")]),
    (tag_icon!("shader_pass"), &[(b"spas", "shader_pass")]),
    (
        tag_icon!("shader_template"),
        &[(b"stem", "shader_template")],
    ),
    (tag_icon!("sky"), &[(b"sky ", "sky")]),
    (tag_icon!("sound"), &[(b"snd!", "sound")]),
    (tag_icon!("style"), &[(b"styl", "style")]),
    (tag_icon!("vehicle"), &[(b"vehi", "vehicle")]),
    (tag_icon!("weapon"), &[(b"weap", "weapon")]),
];

/// The icon for `group_tag` in `game`, read from that game's definitions:
/// the group's own icon where it has one, the shader icon for every kind of
/// shader, otherwise the default. Without a game (or a group) nothing says
/// what a FOURCC means, so it is the default.
pub(in crate::app) fn tag_icon(group_tag: Option<u32>, game: Option<GameId>) -> TagIcon {
    let (Some(group_tag), Some(game)) = (group_tag, game) else {
        return DEFAULT_ICON;
    };
    let groups = crate::app::help::bundled_group_hierarchy(Some(game));
    let Some(name) = groups.name(group_tag) else {
        return DEFAULT_ICON;
    };
    let fourcc = group_tag.to_be_bytes();
    if let Some((icon, _)) = GROUP_ICONS.iter().find(|(_, members)| {
        members
            .iter()
            .any(|(tag, member)| **tag == fourcc && *member == name)
    }) {
        return *icon;
    }
    if is_shader_group(&groups, group_tag, name) {
        return SHADER_ICON;
    }
    DEFAULT_ICON
}

/// Whether a group of this game is a kind of shader: Halo 2's `shader`, or a
/// group that inherits `render_method` (Halo 3 on: terrain, glass, screen,
/// ...) or Halo CE's `shader` (environment, model, the transparent types).
/// By ancestry, so a subclass a game adds gets the icon without being listed.
fn is_shader_group(groups: &crate::app::help::GroupHierarchy, group_tag: u32, name: &str) -> bool {
    (group_tag == u32::from_be_bytes(*b"shad") && name == "shader")
        || groups.is_a(group_tag, u32::from_be_bytes(*b"rm  "))
        || groups.is_a(group_tag, u32::from_be_bytes(*b"shdr"))
}

pub(in crate::app) fn paint_tag_icon_at(
    ui: &Ui,
    group_tag: Option<u32>,
    game: Option<GameId>,
    rect: egui::Rect,
) {
    tag_icon_image(ui.ctx(), tag_icon(group_tag, game), rect.width())
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}

/// `icon` as an image drawn `size` points wide, rasterized for the display.
pub(in crate::app) fn tag_icon_image(
    ctx: &egui::Context,
    icon: TagIcon,
    size: f32,
) -> egui::Image<'static> {
    egui::Image::from_bytes(
        tag_icon_uri_for_pixels_per_point_and_size(icon.name, ctx.pixels_per_point(), size),
        icon.svg.as_bytes(),
    )
}

#[cfg(test)]
fn tag_icon_uri_for_pixels_per_point(icon: &str, pixels_per_point: f32) -> String {
    tag_icon_uri_for_pixels_per_point_and_size(icon, pixels_per_point, 16.0)
}

/// Keyed by icon, not group: one FOURCC draws different icons in different
/// games, and egui keeps the first bytes it is given for a URI.
fn tag_icon_uri_for_pixels_per_point_and_size(
    icon: &str,
    pixels_per_point: f32,
    size: f32,
) -> String {
    let dpi = (pixels_per_point * 100.0).round().max(1.0) as u32;
    let pixels = (size * pixels_per_point).round().max(1.0) as u32;
    format!("bytes://baboon_tag_icons/{icon}-dpi{dpi}-{pixels}px.svg")
}

#[cfg(test)]
mod tests {
    //! Unit tests for tag-group icon selection.
    //! It owns test-only characterization and does not participate in runtime application behavior.

    use super::*;

    fn icon(fourcc: &[u8; 4], game: GameId) -> &'static str {
        tag_icon(Some(u32::from_be_bytes(*fourcc)), Some(game)).name
    }

    /// A FOURCC means a different group in some games, and each game's tag
    /// takes the icon of what it is there.
    #[test]
    fn a_fourcc_takes_the_icon_of_its_group_in_that_game() {
        // Halo 2's chocolate mountain; Reach's, H4's and H2A's cheap light.
        assert_eq!(icon(b"gldf", GameId::Halo2), "chocolate_mountain");
        for game in [GameId::HaloReach, GameId::Halo4, GameId::Halo2Amp] {
            assert_eq!(icon(b"gldf", game), "default_tag", "{game:?}");
        }
        // Halo CE's meter shader; H2A's and H4's structure meta.
        assert_eq!(icon(b"smet", GameId::HaloCe), "shader");
        assert_eq!(icon(b"smet", GameId::Halo4), "default_tag");
        assert_eq!(icon(b"smet", GameId::Halo2Amp), "default_tag");
        // Halo CE's `mode` is named `model` there; `hlmt` is `model` too.
        assert_eq!(icon(b"mode", GameId::HaloCe), "render_model");
        assert_eq!(icon(b"mode", GameId::HaloReach), "render_model");
        assert_eq!(icon(b"hlmt", GameId::HaloReach), "model");
        assert_eq!(icon(b"mod2", GameId::HaloCe), "render_model");
        // A group a game doesn't have is nothing in it.
        assert_eq!(icon(b"mod2", GameId::HaloReach), "default_tag");
        assert_eq!(icon(b"actr", GameId::Halo3), "default_tag");
        // Without a game a FOURCC says nothing.
        assert_eq!(
            tag_icon(Some(u32::from_be_bytes(*b"bipd")), None).name,
            "default_tag"
        );
    }

    /// Every kind of shader in every game draws the shader icon, found by
    /// ancestry: Halo 3 on inherit `render_method`, Halo CE's inherit
    /// `shader`. Groups that only share a name or prefix don't.
    #[test]
    fn every_shader_type_draws_the_shader_icon() {
        for (game, fourccs) in [
            (
                GameId::HaloCe,
                &[
                    b"shdr", b"senv", b"soso", b"schi", b"scex", b"sotr", b"sgla", b"smet",
                    b"spla", b"swat",
                ][..],
            ),
            (GameId::Halo2, &[b"shad"][..]),
            (
                GameId::Halo3,
                &[
                    b"rm  ", b"rmsh", b"rmtr", b"rmw ", b"rmd ", b"rmfl", b"rmhg", b"rmsk",
                    b"rmct", b"rmcs", b"rmb ", b"rmlv", b"?rmp", b"?rmc",
                ][..],
            ),
            (GameId::Halo3Odst, &[b"rmss", b"rmbk"][..]),
            (
                GameId::HaloReach,
                &[b"rmss", b"rmgl", b"rmfu", b"rmfs", b"rmmx", b"rmmm"][..],
            ),
            (GameId::Halo4, &[b"rmwf", b"rmsh"][..]),
            (GameId::CampaignEvolved, &[b"rmsh", b"rmtr"][..]),
        ] {
            for fourcc in fourccs {
                assert_eq!(
                    icon(fourcc, game),
                    "shader",
                    "{game:?} {:?}",
                    String::from_utf8_lossy(*fourcc)
                );
            }
        }
        for (game, fourcc) in [
            (GameId::HaloReach, b"rmbl"),
            (GameId::HaloReach, b"rmdf"),
            (GameId::HaloReach, b"rmop"),
            (GameId::HaloReach, b"rmt2"),
            (GameId::Halo2, b"slit"),
            (GameId::Halo2, b"pixl"),
            (GameId::Halo3, b"vtsh"),
        ] {
            assert_eq!(
                icon(fourcc, game),
                "default_tag",
                "{game:?} {:?}",
                String::from_utf8_lossy(fourcc)
            );
        }
        assert_eq!(icon(b"stem", GameId::Halo2), "shader_template");
        assert_eq!(icon(b"spas", GameId::Halo2), "shader_pass");
    }

    /// Every group the icon table names exists, with that name, in some
    /// game: an entry that matches nothing is a typo or a renamed group.
    #[test]
    fn every_icon_table_entry_names_a_real_group() {
        for (icon, members) in GROUP_ICONS {
            for (fourcc, name) in *members {
                let group_tag = u32::from_be_bytes(**fourcc);
                assert!(
                    GameId::ALL.into_iter().any(|game| {
                        crate::app::help::bundled_group_hierarchy(Some(game)).name(group_tag)
                            == Some(*name)
                    }),
                    "{} {:?} {name}",
                    icon.name,
                    String::from_utf8_lossy(*fourcc)
                );
            }
        }
    }

    #[test]
    fn tag_icon_uri_changes_with_pixels_per_point() {
        let low = tag_icon_uri_for_pixels_per_point("bipd", 1.0);
        let high = tag_icon_uri_for_pixels_per_point("bipd", 2.0);
        assert_ne!(low, high);
        assert!(low.contains("dpi100"));
        assert!(high.contains("dpi200"));
    }
}
