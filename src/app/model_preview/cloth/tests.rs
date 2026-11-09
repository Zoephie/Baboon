use super::*;

#[test]
fn a_grid_hangs_down_from_its_first_row() {
    let (places, uvs, indices) = flag_grid([3, 2], [0.5, 0.25], ClothGame::Halo3);
    assert_eq!(
        places,
        [
            [0.0, 0.0, 0.0],
            [0.5, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, -0.25],
            [0.5, 0.0, -0.25],
            [1.0, 0.0, -0.25]
        ]
    );
    assert_eq!(
        uvs[0],
        [0.0, 0.5],
        "a square of texture spans the longer side"
    );
    assert_eq!(uvs[5], [1.0, 0.0]);
    assert_eq!(indices[..6], [0, 3, 1, 1, 3, 4]);
    assert_eq!(indices.len(), 12);
}

#[test]
fn a_grid_past_the_limit_gives_way_on_its_longer_side() {
    let (places, ..) = flag_grid([20, 20], [1.0, 1.0], ClothGame::Halo3);
    assert_eq!(places.len(), 120);
    assert!(flag_grid([1, 5], [1.0, 1.0], ClothGame::Halo3).0.is_empty());
}

#[test]
fn every_edge_is_a_link_once_and_each_keeps_its_length() {
    let (places, _, indices) = flag_grid([3, 3], [1.0, 1.0], ClothGame::Halo3);
    let links = generate_links(&places, &indices, 0.5);
    let pairs: std::collections::BTreeSet<_> = links
        .iter()
        .map(|link| {
            (
                link.ends[0].min(link.ends[1]),
                link.ends[0].max(link.ends[1]),
            )
        })
        .collect();
    assert_eq!(pairs.len(), links.len(), "no link twice");
    assert!(
        find_link(&links, 0, 3).is_some() && find_link(&links, 3, 1).is_some(),
        "a side and a diagonal"
    );
    for link in &links {
        assert!((link.length - distance(places[link.ends[0]], places[link.ends[1]])).abs() < 1e-6);
    }
}

#[test]
fn a_longer_rod_allows_links_across_more_of_the_sheet() {
    let (places, _, indices) = flag_grid([4, 4], [1.0, 1.0], ClothGame::Halo3);
    let short = generate_links(&places, &indices, 0.5).len();
    let long = generate_links(&places, &indices, 2.0).len();
    assert!(long > short, "{long} against {short}");
}

#[test]
fn pins_take_the_markers_in_order_from_corner_to_corner() {
    let markers = vec![
        (1, [0.0, 0.0, 2.0]),
        (2, [0.0, 0.0, 0.0]),
        (3, [0.0, 0.0, 1.0]),
    ];
    let pins = cloth_pins(
        markers,
        Some((4, [9.0; 3])),
        [8, 8],
        64,
        false,
        ClothGame::Halo3,
    );
    assert_eq!(
        pins.iter().map(|pin| pin.vertex).collect::<Vec<_>>(),
        [0, 24, 56, 63]
    );
    assert_eq!(pins[0].node, 2, "the lowest first");
    assert_eq!(pins[3].node, 4, "the secondary takes the last vertex");
}

fn hung() -> (ClothAt, RunningCloth, Vec<[f32; 3]>) {
    let (places, _, indices) = flag_grid([4, 4], [0.1, 0.1], ClothGame::Halo3);
    let links = generate_links(&places, &indices, 2.0);
    let pins = cloth_pins(
        vec![(0, [0.0, 0.0, 1.0]), (0, [0.0, 0.0, 0.7])],
        None,
        [4, 4],
        16,
        false,
        ClothGame::Halo3,
    );
    let cloth = ClothAt {
        places,
        indices,
        links,
        pins,
        spheres: Vec::new(),
        iterations: 5,
        gravity: 0.5 * gravity_per_tick(tick_rate(ClothGame::Halo3)),
        drag: 0.01,
        wind_scale: 0.15,
        flappiness: 0.25,
        first_vertex: 0,
        game: ClothGame::Halo3,
        center: (0, [0.0; 3]),
    };
    let targets: Vec<[f32; 3]> = cloth.pins.iter().map(|pin| pin.position).collect();
    let running = RunningCloth::at_rest(&cloth, &targets, [0.0; 3]);
    (cloth, running, targets)
}

/// `cloth_update` runs a cloth at most once a tick, so a faster frame
/// rate adds no steps, and Halo 3 ticks twice as often as Halo 2 and
/// Reach.
#[test]
fn cloth_steps_by_its_games_ticks_whatever_the_frame_rate() {
    let ticks_over_a_second = |game: ClothGame, frames: usize| {
        let (cloth, _, _) = hung();
        let cloths = [ClothAt { game, ..cloth }];
        let mut field = ClothField::default();
        field.advance(&cloths, None, 10.0);
        let settled = field.ticks;
        (1..=frames).for_each(|frame| {
            _ = field.advance(&cloths, None, 10.0 + frame as f32 / frames as f32)
        });
        field.ticks - settled
    };
    assert_eq!(ticks_over_a_second(ClothGame::Halo3, 60), 60.0);
    assert_eq!(
        ticks_over_a_second(ClothGame::Halo3, 240),
        60.0,
        "frames between ticks add none"
    );
    assert_eq!(
        ticks_over_a_second(ClothGame::Halo2, 60),
        30.0,
        "Halo 2 ticks at 30"
    );
    assert_eq!(
        ticks_over_a_second(ClothGame::Reach, 60),
        FIXED_TICK_RATE,
        "Reach at its own rate"
    );
}

#[test]
fn a_sheet_that_comes_apart_has_its_pass_over_up_to_the_sixteenth() {
    let (mut cloth, mut running, pins) = hung();
    assert_eq!(
        running.relax(&cloth, &pins),
        5,
        "a sound sheet takes what the tag asks"
    );
    cloth.iterations = 40;
    assert_eq!(
        running.relax(&cloth, &pins),
        15,
        "and no more than the limit"
    );
    cloth.iterations = 5;
    let held = cloth.pins[0].vertex;
    let link = cloth
        .links
        .iter_mut()
        .find(|link| link.ends.contains(&held))
        .unwrap();
    link.length += 100.0;
    assert_eq!(
        running.relax(&cloth, &pins),
        16,
        "one that never settles stops all the same"
    );
}

#[test]
fn spheres_push_points_to_their_skin_and_leave_the_rest() {
    let center = [1.0, 2.0, 3.0];
    let mut running = RunningCloth {
        places: vec![[1.0, 2.0, 3.5], [1.0, 2.0, 5.5], [1.0, 2.0, 3.005]],
        ..RunningCloth::default()
    };
    running.collide(&[(center, 1.0)]);
    assert_eq!(
        running.places[0],
        [1.0, 2.0, 4.0],
        "inside goes out along its own line"
    );
    assert_eq!(running.places[1], [1.0, 2.0, 5.5], "outside stays");
    assert_eq!(
        running.places[2],
        [1.0, 3.0, 3.0],
        "at the centre it goes to the left"
    );
}

#[test]
fn a_sheet_stays_on_its_pins_and_near_its_size() {
    let (cloth, mut running, pins) = hung();
    let mut wind = SharedWind::default();
    for tick in 0..300 {
        let blown = wind.step(tick as f32);
        running.relax(&cloth, &pins);
        running.integrate(&cloth, blown);
        running.calculate_normals(&cloth);
    }
    assert_eq!(running.places[cloth.pins[0].vertex], pins[0]);
    for link in &cloth.links {
        let length = distance(running.places[link.ends[0]], running.places[link.ends[1]]);
        assert!(
            length.is_finite() && length < link.length * 1.5,
            "{length} against {}",
            link.length
        );
    }
}

#[test]
fn gravity_pulls_what_is_not_pinned_and_wind_stirs_it() {
    let settled = |gravity: f32| {
        let (mut cloth, mut running, pins) = hung();
        cloth.gravity = gravity;
        for _ in 0..120 {
            running.relax(&cloth, &pins);
            running.integrate(&cloth, [0.0; 3]);
            running.calculate_normals(&cloth);
        }
        running.places[15]
    };
    let (weightless, weighted) = (
        settled(0.0),
        settled(gravity_per_tick(tick_rate(ClothGame::Halo3))),
    );
    assert!(
        weighted[2] < weightless[2] - 1e-4,
        "{weighted:?} against {weightless:?}"
    );
    assert!(
        weighted[1].abs() < 1e-3,
        "nothing pushes it out of its plane"
    );
    let (cloth, mut blown, pins) = hung();
    let mut wind = SharedWind::default();
    for tick in 0..120 {
        let push = wind.step(tick as f32);
        blown.relax(&cloth, &pins);
        blown.integrate(&cloth, push);
        blown.calculate_normals(&cloth);
    }
    assert!(blown.places[15][1].abs() > 1e-4, "{:?}", blown.places[15]);
}

#[test]
fn halo_2_allows_the_larger_sheet() {
    assert_eq!(
        flag_grid([16, 8], [1.0, 1.0], ClothGame::Halo2).0.len(),
        128
    );
    assert_eq!(
        flag_grid([16, 8], [1.0, 1.0], ClothGame::Halo3).0.len(),
        120
    );
}

#[test]
fn the_rough_root_is_near_the_true_one() {
    for value in [0.01_f32, 0.25, 1.0, 4.0, 100.0] {
        assert!(
            (rough_sqrt(value) / value.sqrt() - 1.0).abs() < 0.07,
            "{value}"
        );
    }
    assert_ne!(rough_sqrt(2.0), 2.0_f32.sqrt(), "and is not the true one");
}

#[test]
fn halo_2_links_pull_a_stretched_sheet_back_with_the_pin_held() {
    let (mut cloth, _, pins) = hung();
    cloth.game = ClothGame::Halo2;
    cloth.pins.truncate(1);
    let pins = &pins[..1];
    let mut running = RunningCloth::at_rest(&cloth, pins, [0.0; 3]);
    running.places[15][1] += 0.3;
    for _ in 0..200 {
        running.relax_pulling(&cloth, pins);
    }
    assert_eq!(running.places[cloth.pins[0].vertex], pins[0]);
    for link in &cloth.links {
        let length = distance(running.places[link.ends[0]], running.places[link.ends[1]]);
        assert!(
            (length - link.length).abs() < 0.05,
            "{length} against {}",
            link.length
        );
    }
}

#[test]
fn a_halo_2_link_at_length_does_not_pull() {
    let (mut cloth, mut running, pins) = hung();
    cloth.game = ClothGame::Halo2;
    cloth.pins.clear();
    let before = running.places.clone();
    running.relax_pulling(&cloth, &pins);
    for (place, was) in running.places.iter().zip(&before) {
        assert!(
            distance(*place, *was) < 0.02,
            "the rough root leaves only a little"
        );
    }
}

#[test]
fn halo_2_normals_keep_some_of_the_last_tick() {
    let (mut cloth, mut running, _) = hung();
    let fresh = |running: &RunningCloth| running.normals[5];
    running.normals = vec![[1.0, 0.0, 0.0]; running.places.len()];
    running.calculate_normals(&cloth);
    let halo3 = fresh(&running);
    cloth.game = ClothGame::Halo2;
    running.normals = vec![[1.0, 0.0, 0.0]; running.places.len()];
    running.calculate_normals(&cloth);
    assert_ne!(fresh(&running), halo3);
}

#[test]
fn the_halo_2_wind_stays_a_unit_away_from_straight_up() {
    let limit = HELD_WIND_POLE_ANGLE.cos();
    let held = held_from_straight_up([0.0, 0.0, 1.0], HELD_WIND_POLE_ANGLE);
    assert!((held[2] - limit).abs() < 1e-6);
    assert!((dot(held, held) - 1.0).abs() < 1e-5);
    assert!(
        held[0] > 0.0,
        "with nothing level to keep, it goes to the front"
    );
    let clear = unit_or_unchanged([1.0, 0.0, 0.2]);
    let kept = held_from_straight_up(clear, HELD_WIND_POLE_ANGLE);
    assert!(distance(kept, clear) < 1e-5, "{kept:?}");
}

#[test]
fn the_halo_2_wind_stays_within_its_strength() {
    let mut wind = HeldWind::default();
    for tick in 0..5000 {
        let push = wind.step(tick as f32);
        assert!(dot(push, push).sqrt() <= (0.1 + 0.0075) * 0.4 * 0.2 + 1e-5);
    }
}

#[test]
fn the_wind_stays_out_of_the_straight_up_cone() {
    let up = [0.0, 0.0, 1.0];
    let moved = away_from_pole(up, 0.5, unit_or_unchanged([0.01, 0.0, 1.0]));
    assert!(dot(moved, up).acos() >= 0.499, "{moved:?}");
    let clear = unit_or_unchanged([1.0, 0.0, 0.2]);
    assert_eq!(away_from_pole(up, 0.5, clear), clear);
}

#[test]
fn the_wind_never_blows_harder_than_its_limit() {
    let mut wind = SharedWind::default();
    for tick in 0..5000 {
        let push = wind.step(tick as f32);
        assert!(dot(push, push).sqrt() <= (0.1 + 0.0075) * 0.4 * 0.2 + 1e-5);
    }
}

/// Reach's `create_cloth_grid_data` spans the texture across each side by
/// its own step; Halo 3's one square spans the longer side.
#[test]
fn a_reach_grid_spans_its_texture_across_each_side() {
    let (places, uvs, indices) = flag_grid([3, 2], [0.5, 0.25], ClothGame::Reach);
    assert_eq!(
        places,
        flag_grid([3, 2], [0.5, 0.25], ClothGame::Halo3).0,
        "the same sheet"
    );
    assert_eq!(indices, flag_grid([3, 2], [0.5, 0.25], ClothGame::Halo3).2);
    assert_eq!(
        uvs,
        [
            [0.0, 1.0],
            [0.5, 1.0],
            [1.0, 1.0],
            [0.0, 0.0],
            [0.5, 0.0],
            [1.0, 0.0]
        ]
    );
    let (_, halo3, _) = flag_grid([3, 2], [0.5, 0.25], ClothGame::Halo3);
    assert_eq!(halo3[0], [0.0, 0.5], "Halo 3 keeps its square");
    let (_, flag, _) = flag_grid([10, 10], [0.03, 0.04], ClothGame::Reach);
    assert_eq!(
        (flag[0], flag[99]),
        ([0.0, 1.0], [1.0, 0.0]),
        "the kit's flag, corner to corner"
    );
}

/// Reach lists a sheet as its tag's whole name, apart from any region a
/// model's variant names; the others as the tag's name.
#[test]
fn a_reach_sheet_is_listed_by_its_tags_whole_name() {
    let path = "objects\\weapons\\multiplayer\\flag\\flag";
    assert_eq!(cloth_region_name(path, ClothGame::Reach), "flag.cloth");
    assert_eq!(cloth_region_name(path, ClothGame::Halo3), "flag");
    assert_eq!(cloth_region_name(path, ClothGame::Halo2), "flag");
}

/// Reach's flag has ten `flag_attach` markers up its pole, every one a pin
/// down the first column from the lowest; Halo 3 keeps six, and only it
/// pins a secondary.
#[test]
fn reach_pins_all_ten_markers_and_no_secondary() {
    let markers: Vec<_> = (0..10)
        .rev()
        .map(|at| (at, [0.0, 0.0, 0.26 + 0.04 * at as f32]))
        .collect();
    let secondary = Some((11, [9.0; 3]));
    let reach = cloth_pins(
        markers.clone(),
        secondary,
        [10, 10],
        100,
        false,
        ClothGame::Reach,
    );
    assert_eq!(
        reach.iter().map(|pin| pin.vertex).collect::<Vec<_>>(),
        [0, 10, 20, 30, 40, 50, 60, 70, 80, 90]
    );
    assert_eq!(reach[0].node, 0, "row 0 on the lowest marker");
    let halo3 = cloth_pins(markers, secondary, [10, 10], 100, false, ClothGame::Halo3);
    assert_eq!(halo3.len(), 6);
    assert_eq!(
        halo3.last().map(|pin| pin.vertex),
        Some(90),
        "the sixth ends the column"
    );
    let fewer: Vec<_> = (0..3).map(|at| (at, [0.0, 0.0, at as f32])).collect();
    assert_eq!(
        cloth_pins(
            fewer.clone(),
            secondary,
            [10, 10],
            100,
            false,
            ClothGame::Reach
        )
        .len(),
        3
    );
    let halo3 = cloth_pins(fewer, secondary, [10, 10], 100, false, ClothGame::Halo3);
    assert_eq!(
        halo3.last().map(|pin| (pin.vertex, pin.node)),
        Some((99, 11)),
        "Halo 3's secondary"
    );
}

/// Reach relaxes at most 9 times, and its tenth pass ends a sheet that
/// keeps coming apart; Halo 3's limits are 15 and 16.
#[test]
fn a_reach_sheet_that_comes_apart_has_its_pass_over_up_to_the_tenth() {
    let (cloth, mut running, pins) = hung();
    let mut cloth = ClothAt {
        game: ClothGame::Reach,
        ..cloth
    };
    assert_eq!(
        running.relax(&cloth, &pins),
        5,
        "a sound sheet takes what the tag asks"
    );
    cloth.iterations = 40;
    assert_eq!(running.relax(&cloth, &pins), 9);
    cloth.iterations = 5;
    let held = cloth.pins[0].vertex;
    let link = cloth
        .links
        .iter_mut()
        .find(|link| link.ends.contains(&held))
        .unwrap();
    link.length += 100.0;
    assert_eq!(running.relax(&cloth, &pins), 10);
}

/// Reach's reset leaves the sheet at the world's origin, and enforcing its
/// attachments carries it to the object's bounding sphere once that is
/// more than 0.4 away; Halo 3 moves it up to its first pin.
#[test]
fn a_reach_sheet_starts_at_its_objects_bounding_sphere() {
    let (cloth, _, pins) = hung();
    let reach = ClothAt {
        game: ClothGame::Reach,
        ..cloth.clone()
    };
    let start = |cloth: &ClothAt, center: [f32; 3]| {
        let mut running = RunningCloth::at_rest(cloth, &pins, center);
        running.pin(cloth, &pins);
        running
    };
    let far = start(&reach, [0.0, 0.0, 2.0]);
    assert_eq!(
        far.places[15],
        [cloth.places[15][0], 0.0, cloth.places[15][2] + 2.0]
    );
    assert_eq!(far.last[15], far.places[15], "its last places go with it");
    assert_eq!(
        start(&reach, [0.0, 0.0, 0.3]).places[15],
        cloth.places[15],
        "a sphere within 0.4 leaves it"
    );
    let halo3 = start(&cloth, [0.0, 0.0, 2.0]);
    assert_eq!(
        halo3.places[15],
        [cloth.places[15][0], 0.0, cloth.places[15][2] + pins[0][2]],
        "at the first pin"
    );
    // Once there, it is carried only by a jump.
    let mut moved = far.clone();
    moved.center = [0.0, 0.0, 2.3];
    moved.pin(&reach, &pins);
    assert_eq!(moved.places[15], far.places[15]);
}

/// Reach's wind is never stiller than 0.05 nor stronger than 0.2, more
/// than Halo 3's ever blows.
#[test]
fn the_reach_wind_stays_between_its_least_and_most() {
    let mut wind = FrameWind::default();
    for frame in 1..5000 {
        let push = wind.step(frame, frame as f32 * FIXED_TICK_RATE);
        let strength = dot(push, push).sqrt();
        assert!(
            (FRAME_WIND_LEAST - 1e-5..=FRAME_WIND_MOST + 1e-5).contains(&strength),
            "{strength}"
        );
        assert!(strength > (0.1 + 0.0075) * 0.4 * 0.2, "past Halo 3's");
    }
}

/// It moves once a drawn frame, however many ticks that frame runs.
#[test]
fn the_reach_wind_moves_only_on_a_new_frame() {
    let mut wind = FrameWind::default();
    let first = wind.step(1, 0.0);
    assert_eq!(wind.step(1, 30.0), first, "the same frame");
    assert_ne!(wind.step(2, 60.0), first);
}

/// Reach derives a vertex's binormal from its normal, `cross(n, x)`, each
/// step, on both sides; Halo 3 keeps the one it was loaded with.
#[test]
fn a_reach_sheet_turns_its_binormal_with_its_normal() {
    let (cloth, _, _) = hung();
    let reach = [ClothAt {
        game: ClothGame::Reach,
        ..cloth.clone()
    }];
    let written = ClothField::default().advance(&reach, None, 10.0);
    let (_, vertices) = &written[0];
    assert_eq!(vertices.len(), cloth.places.len() * 2);
    for (_, normal, binormal) in vertices {
        assert_eq!(*binormal, Some(cross(*normal, [1.0, 0.0, 0.0])));
    }
    assert_eq!(
        sheet_vertices(&[[0.0; 2]], ClothGame::Reach)[1].binormal,
        [0.0, 0.0, -1.0],
        "from the rest normal"
    );
    let written = ClothField::default().advance(&[cloth], None, 10.0);
    assert!(
        written[0]
            .1
            .iter()
            .all(|(_, _, binormal)| binormal.is_none())
    );
}
