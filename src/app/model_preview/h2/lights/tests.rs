use super::*;
use super::super::super::effects::EffectTimes;

/// What every kind of attachment reads for its `scale by`: full with no
/// name, the object's value, and with none an idle object's input.
#[test]
fn an_attachment_scale_reads_the_object_then_its_idle_input() {
    let values = FunctionValues::from([("throttle".to_owned(), 0.25)]);
    assert_eq!(attachment_value("", None), 1.0);
    assert_eq!(attachment_value("throttle", Some(&values)), 0.25);
    assert_eq!(attachment_value("alive", None), 1.0, "nothing to ask: the idle object is alive");
    assert_eq!(attachment_value("boost", None), 0.0);
}

#[test]
fn a_frustum_takes_the_combined_pass_only_with_specular_to_show() {
    let light = |specular| H2DefaultLight { position: [0.0; 3], color: [1.0; 3], specular, radius: 1.0 };
    assert!(has_specular(&light([0.0, 0.5, 0.0])));
    assert!(!has_specular(&light([0.01; 3])), "the game's limit is past one hundredth");
    assert!(!has_specular(&light([0.0; 3])));
}

fn view() -> ViewFrame {
    ViewFrame {
        eye: [0.0, 0.0, 0.0],
        forward: [1.0, 0.0, 0.0],
        right: [0.0, -1.0, 0.0],
        up: [0.0, 0.0, 1.0],
    }
}

fn reflection(flags: ReflectionFlags) -> H2Reflection {
    H2Reflection {
        flags,
        image: 0,
        position: 0.0,
        rotation: 0.0,
        radius: [0.1, 0.3],
        brightness: [0.5, 1.0],
        modulation: 0.25,
        color: [1.0, 0.5, 0.25],
    }
}

fn flare(flags: FlareFlags, reflections: Vec<H2Reflection>) -> H2FlareAt {
    H2FlareAt {
        object: 0,
        node: 0,
        position: [2.0, 0.0, 0.0],
        forward: [-1.0, 0.0, 0.0],
        marker_scale: 3.0,
        only_under: None,
        scale_by: "power".to_owned(),
        change_color: None,
        light: None,
        effect: None,
        flare: Arc::new(H2Flare {
            flags,
            cones: [1.0, 1.0],
            fade: [0.0, 0.0],
            occlusion_radius: 0.2,
            occlusion_offset: OcclusionOffset::None,
            inner_scale: Some(0.5),
            corona_scale: [2.0, 3.0],
            rotation_function: 0,
            rotation_scale: 0.0,
            reflections,
            brightness: None,
            color: None,
            rotation: None,
            synchronized: true,
            falloff: 0,
            images: Vec::new(),
        }),
    }
}

fn values(power: f32) -> FunctionValues {
    FunctionValues::from([("power".to_owned(), power)])
}

#[test]
fn a_function_runs_a_reflection_between_its_bounds() {
    let at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
    let sprites = at.sprites(Some(&values(0.5)), &[[1.0; 3]; 4], None, &view(), 0.0, 0.0, [1.0, 1.0]);
    assert!((sprites[0].alpha - 0.75).abs() < 1e-5);
    assert!((sprites[0].extent[0] - 0.4).abs() < 1e-5, "0.2, then the corona's stretch");
    assert_eq!(sprites[0].highlight, Some(0.75));
}

#[test]
fn scale_by_marker_takes_the_upper_bounds_and_the_markers_scale() {
    let at = flare(FlareFlags { scale_by_marker: true, ..Default::default() }, vec![reflection(ReflectionFlags::default())]);
    let sprites = at.sprites(Some(&values(0.0)), &[[1.0; 3]; 4], None, &view(), 0.0, 0.0, [1.0, 1.0]);
    assert!((sprites[0].alpha - 1.0).abs() < 1e-5);
    assert!((sprites[0].extent[1] - 2.7).abs() < 1e-5);
}

#[test]
fn only_some_reflections_ignore_inner_occlusion() {
    let kept = ReflectionFlags { not_affected_by_inner_occlusion: true, ..Default::default() };
    let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default()), reflection(kept)]);
    at.scale_by.clear();
    let sprites = at.sprites(None, &[[1.0; 3]; 4], None, &view(), 0.0, 0.0, [0.0, 0.5]);
    assert_eq!(sprites.len(), 0, "with the chained test all hidden nothing draws");
    let sprites = at.sprites(None, &[[1.0; 3]; 4], None, &view(), 0.0, 0.0, [0.25, 0.5]);
    assert!((sprites[0].alpha - 0.25).abs() < 1e-5);
    assert!((sprites[1].alpha - 0.5).abs() < 1e-5, "the outer test alone");
}

#[test]
fn a_change_color_tints_unless_a_reflection_ignores_it() {
    let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default()), reflection(ReflectionFlags { ignore_light_color: true, ..Default::default() })]);
    at.scale_by.clear();
    at.change_color = Some(1);
    let colors = [[1.0; 3], [0.5, 0.5, 0.5], [1.0; 3], [1.0; 3]];
    let sprites = at.sprites(None, &colors, None, &view(), 0.0, 0.0, [1.0, 1.0]);
    assert_eq!(sprites[0].color, [0.5, 0.25, 0.125]);
    assert_eq!(sprites[1].color, [1.0, 0.5, 0.25]);
}

fn light() -> H2Light {
    H2Light {
        interpolation: 0,
        size: [1.0, 1.0],
        specular: [[0.0; 3]; 2],
        diffuse: [[0.0, 0.0, 0.0], [1.0, 0.5, 0.25]],
        brightness: [1.0, 1.0],
        color_animation: None,
        brightness_animation: None,
        diffuse_layer: Dx9Layer::LightDiffuse,
        radius: 2.0,
        specular_radius: 3.0,
        effect_duration: 0.0,
        effect_falloff: 0,
    }
}

#[test]
fn a_name_nothing_answers_to_is_zero_and_no_name_is_one() {
    let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
    at.scale_by = "no_such_function".to_owned();
    assert_eq!(at.value(None, 0.0, 0.0), Some(0.0));
    assert_eq!(at.value(Some(&values(0.5)), 0.0, 0.0), Some(0.0));
    at.scale_by.clear();
    assert_eq!(at.value(None, 0.0, 0.0), Some(1.0));
}

#[test]
fn a_light_tints_its_flare_by_its_own_color_and_no_change_color() {
    let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
    at.light = Some(light());
    at.change_color = Some(0);
    let colors = [[0.0; 3]; 4];
    let sprites = at.sprites(Some(&values(0.5)), &colors, None, &view(), 0.0, 0.0, [1.0, 1.0]);
    assert_eq!(sprites[0].color, [0.5, 0.125, 0.03125]);
}

#[test]
fn a_light_with_no_color_or_size_is_out() {
    let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
    at.light = Some(light());
    let none = [[1.0; 3]; 4];
    assert!(at.sprites(Some(&values(0.0)), &none, None, &view(), 0.0, 0.0, [1.0, 1.0]).is_empty());
    at.light = Some(H2Light { size: [0.0, 0.0], ..light() });
    assert!(at.sprites(Some(&values(1.0)), &none, None, &view(), 0.0, 0.0, [1.0, 1.0]).is_empty());
}

#[test]
fn a_light_ignores_its_markers_scale() {
    let mut at = flare(FlareFlags { scale_by_marker: true, ..Default::default() }, vec![reflection(ReflectionFlags::default())]);
    at.light = Some(light());
    let none = [[1.0; 3]; 4];
    let sprites = at.sprites(Some(&values(0.5)), &none, None, &view(), 0.0, 0.0, [1.0, 1.0]);
    assert!((sprites[0].extent[1] - 0.9).abs() < 1e-5, "0.3 by the corona's 3");
}

fn point(light: H2Light) -> H2PointLight {
    H2PointLight {
        object: 0,
        node: 0,
        position: [1.0, 2.0, 3.0],
        scale_by: "power".to_owned(),
        light,
        effect: None,
        only_under: None,
    }
}

#[test]
fn an_effects_light_dims_over_its_duration_and_is_gone_after_it() {
    let times = Arc::new(EffectTimes::read_nothing());
    let effect = H2EffectLight::new(times, 0, false);
    let lasting = H2Light { effect_duration: 0.5, ..light() };
    assert_eq!(effect.value(&lasting, 1.0, 0.0, 0.0), Some(1.0));
    let half = effect.value(&lasting, 1.0, 0.25, 0.0).unwrap();
    assert!((half - 0.5).abs() < 0.01, "linear falloff halfway: {half}");
    assert_eq!(effect.value(&lasting, 1.0, 0.6, 0.0), None);
    assert_eq!(effect.value(&lasting, 1.0, 0.5, 0.5), Some(1.0), "runs from its onset");
    assert_eq!(effect.value(&lasting, 0.0, 0.1, 0.0), None, "nothing while its function is off");
    assert_eq!(effect.value(&light(), 1.0, 50.0, 0.0), Some(1.0), "no duration lasts");
    let scaled = H2EffectLight::new(Arc::new(EffectTimes::read_nothing()), 0, true);
    assert_eq!(scaled.value(&light(), 0.25, 1.0, 0.0), Some(0.25));
}

#[test]
fn a_light_doubles_its_brightness_and_reaches_its_larger_radius() {
    let lit = point(light()).lit(Some(&values(1.0)), None, 0.0, 0.0).unwrap();
    assert_eq!(lit.position, [1.0, 2.0, 3.0]);
    assert_eq!(lit.color, [2.0, 1.0, 0.5]);
    assert_eq!(lit.radius, 3.0);
}

#[test]
fn a_bright_light_stops_at_four() {
    let bright = H2Light { brightness: [3.0, 3.0], ..light() };
    let lit = point(bright).lit(Some(&values(1.0)), None, 0.0, 0.0).unwrap();
    assert_eq!(lit.color, [4.0, 2.0, 1.0]);
}

#[test]
fn a_light_with_no_function_value_is_out_and_no_name_is_on() {
    assert!(point(light()).lit(None, None, 0.0, 0.0).is_some(), "`power` rests at 1");
    let mut out = point(light());
    out.scale_by = "no_such_function".to_owned();
    assert!(out.lit(None, None, 0.0, 0.0).is_none());
    out.scale_by.clear();
    assert!(out.lit(None, None, 0.0, 0.0).is_some());
}

#[test]
fn a_bright_light_is_scaled_down_and_drawn_again() {
    let light = |color| H2DefaultLight { position: [0.0; 3], color, specular: [0.0; 3], radius: 1.0 };
    let (dim, draws) = overbrighten(&light([0.5, 1.0, 0.25]));
    assert_eq!((dim.color, draws), ([0.5, 1.0, 0.25], 1));
    let (dim, draws) = overbrighten(&light([2.0, 1.0, 0.5]));
    assert_eq!((dim.color, draws), ([1.0, 0.5, 0.25], 2));
    let (dim, draws) = overbrighten(&light([4.0, 2.0, 1.0]));
    assert_eq!((dim.color, draws), ([1.0, 0.5, 0.25], 4));
    let brighter_specular = H2DefaultLight { specular: [3.0; 3], ..light([1.0; 3]) };
    assert_eq!(overbrighten(&brighter_specular).1, 4, "the brighter of the two decides");
}

#[test]
fn a_flare_with_no_test_has_no_patches() {
    let tested = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
    let (outer, inner) = tested.occlusion_patches(None, &view()).unwrap();
    assert_eq!((outer.extent[0], inner.unwrap().extent[0]), (0.2, 0.1));
    assert!(flare(FlareFlags { no_occlusion_test: true, ..Default::default() }, Vec::new()).occlusion_patches(None, &view()).is_none());
}

fn gel_frustum() -> H2Frustum {
    H2Frustum {
        near_width: 0.6,
        stretch: 0.8,
        half_view: 65f32.to_radians() * 0.5,
        falloff: 3.0,
        cutoff: 6.0,
        gel: None,
    }
}

/// What `lightaccum_frustum_no_specular_gel` gives a point: gel u, v,
/// falloff and near plane colors, before they clamp.
fn gel_at(rows: &H2GelRows, point: [f32; 3]) -> [f32; 4] {
    let to_light: [f32; 3] = std::array::from_fn(|axis| rows.apex[axis] - point[axis]);
    let row = |index: usize| dot(to_light, std::array::from_fn(|axis| rows.rows[index][axis]));
    let w = row(2);
    [
        (row(0) + rows.rows[0][3]) / w,
        (row(1) + rows.rows[1][3]) / w,
        row(2) + rows.rows[2][3],
        row(3) + rows.rows[3][3],
    ]
}

#[test]
fn a_frustum_projects_its_gel_from_a_point_behind_the_near_plane() {
    let frustum = gel_frustum();
    let (forward, up) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let rows = frustum.rows([2.0, 0.0, 0.0], forward, up, 1.0);
    let behind = 0.3 / frustum.half_view.tan();
    assert!((rows.apex[0] - (2.0 - behind)).abs() < 1e-5);
    let ahead = |distance: f32| [2.0 - behind + distance, 0.0, 0.0];
    // The axis lands in the middle of the gel, at every distance.
    for distance in [1.0, 4.0] {
        let [u, v, ..] = gel_at(&rows, ahead(distance));
        assert!((u - 0.5).abs() < 1e-5 && (v - 0.5).abs() < 1e-5);
    }
    // The frustum's edges land on the gel's. `sub_8E84F0` crosses forward
    // into up, so u climbs toward +Y on a +X, +Z light.
    let tan_x = frustum.half_view.tan();
    let left = [2.0 - behind + 4.0, 4.0 * tan_x, 0.0];
    let right = [2.0 - behind + 4.0, -4.0 * tan_x, 0.0];
    assert!((gel_at(&rows, left)[0] - 1.0).abs() < 1e-4);
    assert!(gel_at(&rows, right)[0].abs() < 1e-4);
    let top = [2.0 - behind + 4.0, 0.0, 4.0 * tan_x * frustum.stretch];
    assert!((gel_at(&rows, top)[1] - 1.0).abs() < 1e-4);
    // Dark from the far end, full up to the falloff distance.
    assert!(gel_at(&rows, ahead(behind + 3.0))[2].abs() < 1e-4);
    assert!((gel_at(&rows, ahead(behind + 6.0))[2] - 1.0).abs() < 1e-4);
    // The near edge takes a surface past it and leaves one behind it.
    assert!(gel_at(&rows, ahead(behind + 0.1))[3] > 0.5);
    assert!(gel_at(&rows, ahead(behind - 0.1))[3] < 0.5);
}
