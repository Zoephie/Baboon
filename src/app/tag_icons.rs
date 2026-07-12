//! Embedded tag-group icon lookup and display-scale selection.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;

pub(super) fn get_icon_svg(group_tag: &str) -> &'static str {
    match group_tag {
        "actr" => include_str!("../../assets/icons/actor.svg"),
        "actv" => include_str!("../../assets/icons/actor_variant.svg"),
        "antr" => include_str!("../../assets/icons/model_animations.svg"),
        "jmad" => include_str!("../../assets/icons/animation_graph.svg"),
        "bipd" => include_str!("../../assets/icons/biped.svg"),
        "bitm" => include_str!("../../assets/icons/bitmap.svg"),
        "trak" => include_str!("../../assets/icons/camera_track.svg"),
        "char" => include_str!("../../assets/icons/character.svg"),
        "gldf" | "chmt" => include_str!("../../assets/icons/chocolate_mountain.svg"),
        "coll" => include_str!("../../assets/icons/collision_model.svg"),
        "bloc" => include_str!("../../assets/icons/crate.svg"),
        "jpt!" => include_str!("../../assets/icons/damage_effect.svg"),
        "matg" => include_str!("../../assets/icons/default_globals.svg"),
        "ctrl" => include_str!("../../assets/icons/device_control.svg"),
        "mach" => include_str!("../../assets/icons/device_machine.svg"),
        "udlg" => include_str!("../../assets/icons/dialogue.svg"),
        "effe" => include_str!("../../assets/icons/effect.svg"),
        "eqip" => include_str!("../../assets/icons/equipment.svg"),
        "garb" => include_str!("../../assets/icons/garbage.svg"),
        "hudg" | "nhdt" | "chdt" | "chgd" => include_str!("../../assets/icons/hud_definition.svg"),
        "lens" => include_str!("../../assets/icons/lens_flare.svg"),
        "ligh" => include_str!("../../assets/icons/light.svg"),
        "hlmt" => include_str!("../../assets/icons/model.svg"),
        "mod2" => include_str!("../../assets/icons/gbxmodel.svg"),
        "phys" => include_str!("../../assets/icons/physics_model.svg"),
        "phmo" => include_str!("../../assets/icons/physics_model.svg"),
        "proj" => include_str!("../../assets/icons/projectile.svg"),
        "mode" => include_str!("../../assets/icons/render_model.svg"),
        "scnr" => include_str!("../../assets/icons/scenario.svg"),
        "scen" => include_str!("../../assets/icons/scenery.svg"),
        "spas" => include_str!("../../assets/icons/shader_pass.svg"),
        "stem" => include_str!("../../assets/icons/shader_template.svg"),
        "shad" | "shdr" | "rmsh" => include_str!("../../assets/icons/shader.svg"),
        "sky " => include_str!("../../assets/icons/sky.svg"),
        "snd!" => include_str!("../../assets/icons/sound.svg"),
        "styl" => include_str!("../../assets/icons/style.svg"),
        "vehi" => include_str!("../../assets/icons/vehicle.svg"),
        "weap" => include_str!("../../assets/icons/weapon.svg"),
        _ => include_str!("../../assets/icons/default_tag.svg"),
    }
}

pub(super) fn draw_tag_icon(ui: &mut Ui, group_tag: u32, size: f32) {
    let group = format_group_tag(group_tag);
    draw_tag_icon_svg(ui, &group, size);
}

pub(super) fn paint_tag_icon_at(ui: &Ui, group_tag: Option<u32>, rect: egui::Rect) {
    let group = group_tag
        .map(format_group_tag)
        .unwrap_or_else(|| "default".to_owned());
    let uri = tag_icon_uri(ui.ctx(), &group);
    egui::Image::from_bytes(uri, get_icon_svg(&group).as_bytes())
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}

fn draw_tag_icon_svg(ui: &mut Ui, group: &str, size: f32) {
    let uri = tag_icon_uri(ui.ctx(), group);
    ui.add(
        egui::Image::from_bytes(uri, get_icon_svg(&group).as_bytes())
            .fit_to_exact_size(Vec2::splat(size))
            .sense(Sense::hover()),
    );
}

pub(super) fn tag_icon_uri(ctx: &egui::Context, group: &str) -> String {
    tag_icon_uri_for_pixels_per_point(group, ctx.pixels_per_point())
}

fn tag_icon_uri_for_pixels_per_point(group: &str, pixels_per_point: f32) -> String {
    let dpi = (pixels_per_point * 100.0).round().max(1.0) as u32;
    format!("bytes://baboon_tag_icons/{group}-dpi{dpi}.svg")
}

#[cfg(test)]
#[path = "tests/tag_icons.rs"]
mod tests;
