use super::*;

pub(super) fn get_icon_svg(group_tag: &str) -> &'static str {
    match group_tag {
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
    let uri = format!("bytes://baboon_tag_icons/{group}.svg");
    egui::Image::from_bytes(uri, get_icon_svg(&group).as_bytes())
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}

fn draw_tag_icon_svg(ui: &mut Ui, group: &str, size: f32) {
    let uri = format!("bytes://baboon_tag_icons/{group}.svg");
    ui.add(
        egui::Image::from_bytes(uri, get_icon_svg(&group).as_bytes())
            .fit_to_exact_size(Vec2::splat(size))
            .sense(Sense::hover()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_icon_lookup_uses_expected_group_mappings() {
        assert!(get_icon_svg("bipd").contains("<svg"));
        assert!(get_icon_svg("shad").contains("<svg"));
        assert!(get_icon_svg("rmsh").contains("<svg"));
        assert!(get_icon_svg("char").contains("<svg"));
        assert!(get_icon_svg("jpt!").contains("<svg"));
        assert!(get_icon_svg("lens").contains("<svg"));
        assert!(get_icon_svg("ligh").contains("<svg"));
        assert!(get_icon_svg("matg").contains("<svg"));
        assert!(get_icon_svg("styl").contains("<svg"));
        assert!(get_icon_svg("unknown").contains("<svg"));
    }
}
