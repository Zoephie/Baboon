use super::*;

/// A view from 10 units down -y, looking along +y.
pub(in crate::app::model_preview) const VIEW: ViewFrame = ViewFrame {
    eye: [0.0, -10.0, 0.0],
    forward: [0.0, 1.0, 0.0],
    right: [1.0, 0.0, 0.0],
    up: [0.0, 0.0, 1.0],
};

fn reflection(position: f32) -> CeReflection {
    CeReflection {
        align_with_screen_center: false,
        radius_not_scaled_by_distance: false,
        radius_scaled_by_occlusion: false,
        image: 0,
        position,
        rotation: 0.0,
        radius: [0.1, 0.5],
        brightness: [1.0, 1.0],
        brightness_scaled_by: 0,
        tint: NO_COLOR,
        lower: NO_COLOR,
        upper: NO_COLOR,
        interpolation: 0,
        animation_function: 0,
        animation_period: 0.0,
        animation_phase: 0.0,
    }
}

fn light(reflections: Vec<CeReflection>) -> CeLight {
    CeLight {
        object: 0,
        node: 0,
        position: [1.0, 0.0, 0.0],
        forward: [0.0, -1.0, 0.0],
        primary: Some(0),
        change_color: None,
        lower: [0.0, 0.0, 0.0, 1.0],
        upper: [1.0, 0.5, 0.0, 1.0],
        interpolation: 0,
        dynamic: true,
        spares_owner: false,
        radius: 2.0,
        radius_bounds: [1.0, 1.0],
        cone: None,
        fixed: None,
        flare: Some(Arc::new(CeFlare {
            cones: [1.0, 1.0],
            fade: [0.0, 0.0],
            corona_scale: [2.0, 1.0],
            rotation_function: 0,
            rotation_scale: std::f32::consts::TAU,
            occlusion_radius: 0.1,
            occlusion_offset: OcclusionOffset::TowardViewer,
            reflections,
            images: Vec::new(),
        })),
    }
}

fn sprites(light: &CeLight, value: f32) -> Vec<FlareSprite> {
    light.sprites(&[value, 0.0, 0.0, 0.0], &[[1.0; 3]; 4], None, &VIEW, 0.0, 1.0)
}

#[test]
fn a_light_at_its_black_bound_shows_nothing() {
    let light = light(vec![reflection(0.0)]);
    assert!(sprites(&light, 0.0).is_empty());
    assert_eq!(sprites(&light, 1.0)[0].color, [1.0, 0.5, 0.0]);
}

#[test]
fn a_light_counts_as_reading_its_owners_output() {
    let mut light = light(vec![reflection(0.0)]);
    light.primary = Some(1);
    let mut read = [false; 4];
    mark_outputs_read(std::slice::from_ref(&light), 1, &mut read);
    assert_eq!(read, [false; 4]);
    mark_outputs_read(&[light], 0, &mut read);
    assert_eq!(read, [false, true, false, false]);
}

fn rows(lights: &[CeLight], object: u8, value: f32) -> [[f32; 4]; CE_POINT_LIGHT_ROWS] {
    let state = |_| ([value, 0.0, 0.0, 0.0], [[1.0; 3]; 4]);
    point_light_rows(lights, object, [0.0; 3], 0.5, state, None)
}

#[test]
fn a_point_light_has_the_games_constants() {
    let mut lit = light(Vec::new());
    lit.cone = Some([0.75, 0.5]);
    let rows = rows(&[lit], 0, 1.0);
    assert_eq!(rows[0], [1.0, 0.0, 0.0, 0.25]);
    assert_eq!(rows[1], [0.0, -1.0, 0.0, 4.0]);
    assert_eq!(rows[2], [1.0, 0.5, 0.0, -2.0]);
}

#[test]
fn a_light_with_no_cone_shines_all_ways() {
    let rows = rows(&[light(Vec::new())], 0, 1.0);
    assert_eq!((rows[1][3], rows[2][3]), (0.0, 1.0));
}

#[test]
fn dark_flare_only_and_out_of_reach_lights_add_nothing() {
    let lit = light(Vec::new());
    assert_eq!(rows(std::slice::from_ref(&lit), 0, 0.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
    let mut flare_only = lit.clone();
    flare_only.dynamic = false;
    assert_eq!(rows(&[flare_only], 0, 1.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
    let mut far = lit;
    far.position = [9.0, 0.0, 0.0];
    assert_eq!(rows(&[far], 0, 1.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
}

#[test]
fn a_light_can_spare_only_its_own_object() {
    let mut lit = light(Vec::new());
    lit.spares_owner = true;
    assert_eq!(rows(std::slice::from_ref(&lit), 0, 1.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
    assert_ne!(rows(&[lit], 1, 1.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
}

#[test]
fn the_two_strongest_lights_win() {
    let dim = |x: f32| CeLight { position: [x, 0.0, 0.0], ..light(Vec::new()) };
    let rows = rows(&[dim(1.5), dim(0.0), dim(1.0)], 0, 1.0);
    assert_eq!((rows[0][0], rows[3][0]), (0.0, 1.0));
}

#[test]
fn tool_bakes_a_lights_cone() {
    use std::f32::consts::{FRAC_PI_3, FRAC_PI_2, PI};
    assert_eq!(baked_cone(0.0, 0.0), None);
    assert_eq!(baked_cone(FRAC_PI_2, 0.0), None);
    assert_eq!(baked_cone(PI, PI), None);
    assert_eq!(baked_cone(FRAC_PI_3, FRAC_PI_2), Some([FRAC_PI_3.cos(), FRAC_PI_2.cos()]));
    // A falloff past the cutoff starts from straight ahead.
    assert_eq!(baked_cone(FRAC_PI_2, FRAC_PI_3), Some([1.0, FRAC_PI_3.cos()]));
}

#[test]
fn tool_whitens_a_light_with_no_color() {
    let black = [0.0, 0.0, 0.0, 0.5];
    assert_eq!(baked_colors(black, black), ([1.0, 1.0, 1.0, 0.5], [1.0, 1.0, 1.0, 0.5]));
    let upper = [1.0, 0.5, 0.0, 1.0];
    assert_eq!(baked_colors(black, upper), (black, upper));
}

#[test]
fn a_hidden_flare_draws_nothing_and_a_half_hidden_one_dims() {
    let light = light(vec![reflection(0.0), reflection(0.0)]);
    let at = |occlusion| light.sprites(&[1.0; 4], &[[1.0; 3]; 4], None, &VIEW, 0.0, occlusion);
    assert!(at(0.0).is_empty());
    let half = at(0.5);
    assert_eq!(half.len(), 2);
    assert_eq!(half[0].alpha, 0.5);
}

#[test]
fn the_occlusion_patch_sits_where_the_flare_says() {
    let patch = |offset| {
        let mut light = light(Vec::new());
        let mut flare = (**light.flare.as_ref().unwrap()).clone();
        flare.occlusion_offset = offset;
        light.flare = Some(Arc::new(flare));
        light.occlusion_patch(&[1.0; 4], &[[1.0; 3]; 4], None, &VIEW).unwrap().center
    };
    assert_eq!(patch(OcclusionOffset::TowardViewer), [1.0, -0.1, 0.0]);
    assert!((patch(OcclusionOffset::MarkerForward)[1] + 0.141_421_4).abs() < 1e-6);
    assert_eq!(patch(OcclusionOffset::None), [1.0, 0.0, 0.0]);
    // A dark light tests nothing.
    assert!(light(Vec::new()).occlusion_patch(&[0.0; 4], &[[1.0; 3]; 4], None, &VIEW).is_none());
}

#[test]
fn a_corona_turns_with_how_the_light_faces_the_view() {
    // Seen head on, the light points straight back at the viewer.
    let facing = [0.0, -1.0, 0.0];
    assert_eq!(corona_rotation(0, facing, [0.0, 1.0, 0.0], &VIEW), 0.0);
    assert_eq!(corona_rotation(1, facing, [0.0, 1.0, 0.0], &VIEW), 0.0);
    // Turned a quarter away to the right, the share is a quarter turn.
    let sideways = corona_rotation(2, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], &VIEW);
    assert!((sideways.abs() - 0.25).abs() < 1e-5, "{sideways}");
    // Only the first reflection, the corona, carries it.
    let mut turning = light(vec![reflection(0.0), reflection(0.0)]);
    let flare = Arc::make_mut(turning.flare.as_mut().unwrap());
    flare.rotation_function = 2;
    turning.forward = [1.0, 0.0, 0.0];
    let drawn = sprites(&turning, 1.0);
    assert!(drawn[0].rotation.abs() > 1.0);
    assert_eq!(drawn[1].rotation, 0.0);
}

#[test]
fn no_source_reads_full() {
    let mut light = light(vec![reflection(0.0)]);
    light.primary = None;
    assert_eq!(sprites(&light, 0.0).len(), 1);
}

#[test]
fn the_radius_follows_the_light_and_only_the_corona_stretches() {
    let light = light(vec![reflection(0.0), reflection(0.0)]);
    let drawn = sprites(&light, 1.0);
    assert_eq!(drawn[0].extent, [1.0, 0.5]);
    assert_eq!(drawn[1].extent, [0.5, 0.5]);
    assert_eq!(sprites(&light, 0.5)[1].extent, [0.3, 0.3]);
}

#[test]
fn reflections_run_across_the_screen_center() {
    let light = light(vec![reflection(0.0), reflection(1.0)]);
    let drawn = sprites(&light, 1.0);
    assert_eq!(drawn[0].center, [1.0, 0.0, 0.0]);
    // Two lengths along the axis lands it mirrored about the center.
    assert_eq!(drawn[1].center, [-1.0, 0.0, 0.0]);
}

#[test]
fn the_first_reflection_dims_the_rest() {
    let mut first = reflection(0.0);
    first.brightness = [0.5, 0.5];
    let light = light(vec![first, reflection(0.0)]);
    let drawn = sprites(&light, 1.0);
    assert_eq!((drawn[0].alpha, drawn[1].alpha), (0.5, 0.5));
}

#[test]
fn a_change_color_tints_by_the_bounds_alpha() {
    assert_eq!(light_color(0, [1.0; 4], [1.0; 4], [0.5; 3], 1.0), [1.0; 3]);
    let unset = [1.0, 1.0, 1.0, 0.0];
    assert_eq!(light_color(0, unset, unset, [0.5; 3], 1.0), [0.5; 3]);
}

#[test]
fn transitions_read_their_tables_and_linear_reads_none() {
    assert_eq!(transition_evaluate(0, 0.3), 0.3);
    assert_eq!(transition_evaluate(0, 1.7), 1.0);
    assert_eq!(transition_evaluate(0, f32::NAN), 0.0);
    // Squared, a byte to the step: near enough a quarter at a half.
    assert!((transition_evaluate(3, 0.5) - 0.25).abs() < 0.01);
    assert!((transition_evaluate(1, 0.25) - 0.5).abs() < 0.01);
    assert_eq!(transition_evaluate(4, 0.0), 0.0);
}

#[test]
fn an_effects_light_fades_by_its_age_and_then_is_gone() {
    let def = CeEffectLightDef { light: light(Vec::new()), duration: 30.0, falloff: 0 };
    let made = |age| def.at(0, 0, [0.0; 3], [1.0, 0.0, 0.0], age, 0.5);
    assert_eq!(made(0).and_then(|light| light.fixed), Some(0.5));
    let half = made(15).and_then(|light| light.fixed).unwrap();
    assert!((half - 0.25).abs() < 1e-6);
    assert!(made(30).is_some());
    assert!(made(31).is_none());
    assert!(def.alive(30) && !def.alive(31));
    // Its color is the bounds' alone, whatever the change colors say.
    let light = made(0).unwrap();
    assert_eq!(light.color(0.5, &[[0.0; 3]; 4]), [0.5, 0.25, 0.0]);
}
