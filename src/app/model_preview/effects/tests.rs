use super::*;

fn location() -> MarkerFrame {
    // Facing +Y, its left along -X.
    MarkerFrame {
        axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        position: [1.0, 2.0, 3.0],
        node: 4,
    }
}

fn times(events: &[([f32; 2], [f32; 2])], loop_start: usize, parallel: bool) -> EffectTimes {
    let events = events.iter().map(|(delay, duration)| EventTime::of(*delay, *duration)).collect();
    EffectTimes { events, loop_start, parallel, once: false }
}

/// A part asked to be nonviolent is not made, whether it says so in its
/// `violence mode` or, in Halo 2, in its second `create in`.
#[test]
fn a_nonviolent_part_is_not_made_in_either_naming() {
    for (game, violence) in [("halo2_mcc", "create in#1"), ("halo3_mcc", "violence mode")] {
        let mut tag = TagFile::new(crate::core::bundled::locate_definitions_root().join(game).join("effect.json"))
            .unwrap_or_else(|error| panic!("{game} effect: {error:?}"));
        for block in ["events", "events[0]/parts"] {
            let mut root = tag.root_mut();
            let mut field = root.field_path_mut(block).unwrap_or_else(|| panic!("{game}: {block}"));
            field.as_block_mut().unwrap().add_element();
        }
        let runs = |tag: &TagFile| {
            let part = tag.root().descend("events[0]/parts[0]").unwrap();
            assert_eq!(part_violence(part), violence, "{game}");
            part_runs(part)
        };
        for (value, made) in [(2, false), (0, true)] {
            let path = format!("events[0]/parts[0]/{violence}");
            let mut root = tag.root_mut();
            let mut field = root.field_path_mut(&path).unwrap_or_else(|| panic!("{game}: {path}"));
            field.set(TagFieldData::ShortEnum { value, name: None }).unwrap();
            assert_eq!(runs(&tag), made, "{game}: violence {value}");
        }
    }
}

#[test]
fn an_effect_restarts_each_time_its_function_comes_on() {
    let mut onsets = EffectOnsets::default();
    let values = |value: f32| FunctionValues::from([("on".to_owned(), value)]);
    let of = |onsets: &mut EffectOnsets, value, time| onsets.of(0, "on", Some(&values(value)), time);
    assert_eq!(of(&mut onsets, 1.0, 5.0), 0.0, "on from the start");
    assert_eq!(of(&mut onsets, 0.0, 6.0), 0.0);
    assert_eq!(of(&mut onsets, 1.0, 7.0), 7.0);
    assert_eq!(of(&mut onsets, 1.0, 8.0), 7.0);
    assert_eq!(of(&mut onsets, 1.0, 2.0), 0.0, "the clock went back");
    assert_eq!(onsets.of(0, "", None, 9.0), 0.0);
}

#[test]
fn seconds_run_from_an_events_delay_and_a_tick_runs_from_the_start() {
    let two = times(&[([1.0, 1.0], [2.0, 2.0]), ([0.0, 0.0], [3.0, 3.0])], 0, false);
    assert_eq!(two.elapsed(0, 0.5), None);
    assert_eq!(two.elapsed(0, 2.5), Some(1.5));
    assert_eq!(two.elapsed(1, 4.0), Some(1.0));
    let tick = times(&[([0.0, 0.0], [0.0, 0.0])], 0, false);
    assert_eq!(tick.elapsed(0, 7.0), Some(7.0));
}

#[test]
fn a_delayed_tick_of_an_event_is_found_however_late_it_was_played() {
    let delayed = EffectTimes { once: true, ..times(&[([0.1, 0.1], [0.0, 0.0])], 0, false) };
    let inside = delayed.begins_within(0, 0.05, 0.11).unwrap();
    assert!(delayed.age(0, inside).is_some());
    // The sum this must not go through: late enough, it rounds down.
    let late = (0..3000).map(|at| 100.0 + at as f32 * 0.1169).any(|onset: f32| (0.1 + onset) - onset < 0.1);
    assert!(late);
}

#[test]
fn a_step_longer_than_an_event_still_sees_it_begin() {
    let two = times(&[([0.0, 0.0], [0.0, 0.0]), ([0.0, 0.0], [0.0, 0.0])], 0, false);
    assert_eq!(two.begins_within(1, 0.0, 0.05), Some(ONE_TICK * 1.5));
    assert_eq!(two.begins_within(1, 0.05, 0.1), None);
    assert_eq!(two.begins_within(0, -0.01, 0.05), Some(ONE_TICK * 0.5));
    assert_eq!(two.begins_within(0, 0.0, 0.05), None, "already begun");
}

/// A cheap particle part is made out of the environment, violence and
/// camera it asks for, and kept out only when disabled; any other part
/// keeps all four tests.
#[test]
fn a_cheap_particle_part_skips_all_but_the_disabled_test() {
    let volume = u32::from_be_bytes(*b"ltvl");
    assert!(part_made(CHEAP_PARTICLE_EMITTER, false, false), "under water, first person: still made");
    assert!(!part_made(volume, false, false), "another part is not");
    assert!(!part_made(CHEAP_PARTICLE_EMITTER, true, true), "disabled for debugging");
    assert!(part_made(volume, true, false));
}

#[test]
fn a_named_vector_sits_at_the_object_and_points_its_way() {
    let down = part_markers(&[], "gravity", true);
    assert_eq!(down[0].position, [0.0; 3]);
    assert_eq!(down[0].axes[0], [0.0, 0.0, -1.0]);
    assert_eq!(part_markers(&[], "up", true)[0].axes[0], [0.0, 0.0, 1.0]);
    assert_eq!(part_markers(&[], "backward", true)[0].axes[0], [-1.0, 0.0, 0.0]);
}

#[test]
fn a_looping_effect_needs_its_marker_unless_it_asks_for_the_root() {
    assert!(part_markers(&[], "fx_vent", false).is_empty());
    assert_eq!(part_markers(&[], "root", false).len(), 1);
}

#[test]
fn a_real_marker_of_that_name_wins() {
    let marker = RenderModelPreviewMarker {
        name: "up".to_owned(),
        node_index: 3,
        position: [1.0, 2.0, 3.0],
        axes: axes_of_vector([1.0, 0.0, 0.0]),
        scale: 1.0,
        only_under: None,
    };
    assert_eq!(part_markers(&[marker], "up", true)[0].position, [1.0, 2.0, 3.0]);
}

fn attachment(vectors: bool) -> ObjectEffect {
    ObjectEffect {
        path: String::new(),
        marker: "fx_anchor".to_owned(),
        scale_by: String::new(),
        secondary_by: String::new(),
        change_color: None,
        once: false,
        vectors,
    }
}

fn named(name: &str, node_index: i16, position: [f32; 3]) -> RenderModelPreviewMarker {
    RenderModelPreviewMarker { name: name.to_owned(), node_index, position, axes: WORLD_AXES, scale: 1.0, only_under: None }
}

fn at_rest(marker: &RenderModelPreviewMarker) -> (MarkerFrame, usize) {
    (MarkerFrame { axes: marker.axes, position: marker.position, node: marker.node_index }, 0)
}

/// Reach's `build_gravity_effect_vectors`: `up` and `gravity` sit at the
/// attachment's marker, on its node, facing up and down, even where the
/// model has a marker of that name; H3's attachment has no vectors.
#[test]
fn a_reach_attachment_finds_its_vectors_at_its_marker_before_any_marker() {
    let markers = [named("fx_anchor", 2, [1.0, 2.0, 3.0]), named("gravity", 1, [4.0, 5.0, 6.0])];
    let up = attachment(true).locations(&markers[..1], "up", &at_rest);
    assert_eq!(up.len(), 1);
    assert_eq!((up[0].position, up[0].node_index), ([1.0, 2.0, 3.0], 2));
    assert_eq!(up[0].axes, [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], "forward, left, up");
    let down = attachment(true).locations(&markers, "gravity", &at_rest);
    assert_eq!(down[0].position, [1.0, 2.0, 3.0], "the vector, not the `gravity` marker");
    assert_eq!(down[0].axes, [[0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]]);
    assert!(attachment(false).locations(&markers, "up", &at_rest).is_empty(), "H3: a looping effect needs its marker");
    assert_eq!(attachment(false).locations(&markers, "gravity", &at_rest)[0].position, [4.0, 5.0, 6.0]);
    assert_eq!(attachment(true).locations(&markers, "fx_anchor", &at_rest)[0].axes, WORLD_AXES, "a marker is a marker");
}

/// The vector is the world's: on an object turned onto its side it is
/// turned back into the object's space, so it still faces world up.
#[test]
fn a_vector_faces_the_worlds_way_on_a_turned_object() {
    // The object's x stays, its y is the world's z and its z the world's -y.
    let turn = |[x, y, z]: [f32; 3]| [x, -z, y];
    let turned = |marker: &RenderModelPreviewMarker| {
        (MarkerFrame { axes: marker.axes.map(turn), position: turn(marker.position), node: 0 }, 0)
    };
    let up = attachment(true).locations(&[], "up", &turned);
    assert_eq!(up[0].axes[0], [0.0, 1.0, 0.0], "the object's y is the world's up");
    assert_eq!(turned(&up[0]).0.axes[0], [0.0, 0.0, 1.0]);
}

#[test]
fn an_events_effect_runs_once_and_an_attachment_loops() {
    let looped = times(&[([0.0, 0.0], [2.0, 2.0])], 0, false);
    assert!(looped.age(0, 3.0).is_some());
    let once = EffectTimes { once: true, ..times(&[([0.0, 0.0], [2.0, 2.0])], 0, false) };
    assert!(once.age(0, 1.0).is_some());
    assert_eq!(once.age(0, 3.0), None);
    assert!(once.moves());
}

#[test]
fn an_attached_objects_event_effect_moves_to_its_owner_and_a_function_of_its_own_does_not() {
    let (mut function, mut object) = ("firing effect: firing".to_owned(), 3);
    adopt_event_function("gun", &mut function, &mut object);
    assert_eq!((function.as_str(), object), ("gun: firing effect: firing", 0));
    assert!(is_event_function("gun: firing effect: firing"));
    let (mut function, mut object) = ("power".to_owned(), 3);
    adopt_event_function("gun", &mut function, &mut object);
    assert_eq!((function.as_str(), object), ("power", 3));
}

#[test]
fn an_events_effect_is_named_by_its_field_and_tag() {
    assert_eq!(event_function_name("firing effect", "objects\\weapons\\fx\\firing"), "firing effect: firing");
}

/// With `BLAM_TEST_HREK` and `BLAM_TEST_H3EK`: Reach's effects are told
/// from Halo 3's by `looping sounds`.
#[test]
fn only_reach_effects_keep_their_durations() {
    use crate::app::export::geometry::load_referenced_tag_from_source;
    use crate::core::game::GameId;
    for (game, path, baked) in [
        (GameId::HaloReach, "objects\\equipment\\armor_lockup\\fx\\looping", false),
        (GameId::Halo3, "objects\\weapons\\support_low\\brute_shot\\fx\\detonation", true),
    ] {
        let Some(source) = crate::app::model_preview::harness::kit::kit_source(game) else {
            continue;
        };
        let effect = load_referenced_tag_from_source(&source, path, "effect", b"effe").unwrap();
        assert_eq!(effect.root().field("looping sounds").is_none(), baked, "{game:?}");
    }
}

#[test]
fn reach_rolls_a_duration_that_can_be_zero_as_authored() {
    assert_eq!(EventTime::unbaked([0.0; 2], [0.0, 3.0]).duration, 1.5);
    assert_eq!(EventTime::unbaked([0.0; 2], [0.0, 0.0]).duration, ONE_TICK, "an instant is an update");
    assert_eq!(EventTime::of([0.0; 2], [0.0, 3.0]).duration, ONE_TICK, "Halo 3 bakes it");
}

#[test]
fn a_duration_that_can_be_zero_bakes_to_a_tick_and_stays_at_age_zero() {
    let effect = times(&[([0.0; 2], [0.0; 2])], 0, false);
    assert_eq!(effect.events[0].duration, ONE_TICK);
    for time in [0.0, 0.001, 7.3] {
        assert_eq!(effect.age(0, time), Some(0.0));
    }
    assert!(!effect.moves());
    assert_eq!(EventTime::of([0.0; 2], [0.0, 3.0]).duration, ONE_TICK);
    assert_eq!(EventTime::of([0.0; 2], [1.0, 3.0]).duration, 2.0);
}

#[test]
fn an_event_waits_out_its_delay_then_ages_across_its_duration() {
    let effect = times(&[([1.0; 2], [2.0; 2])], 0, false);
    assert_eq!(effect.age(0, 0.5), None);
    assert_eq!(effect.age(0, 2.0), Some(0.5));
    assert_eq!(effect.age(0, 3.5), None, "looped round into its delay");
    assert_eq!(effect.age(0, 5.0), Some(0.5));
    assert!(effect.moves());
}

#[test]
fn events_follow_each_other_and_loop_from_the_loop_start() {
    let effect = times(&[([0.0; 2], [1.0; 2]), ([0.0; 2], [2.0; 2])], 1, false);
    assert_eq!(effect.age(0, 0.5), Some(0.5));
    assert_eq!(effect.age(1, 0.5), None);
    assert_eq!(effect.age(1, 2.0), Some(0.5));
    assert_eq!(effect.age(0, 3.5), None, "the first event never comes round again");
    assert_eq!(effect.age(1, 3.5), Some(0.25));
}

#[test]
fn parallel_events_run_together_and_restart_when_the_longest_is_done() {
    let effect = times(&[([0.0; 2], [1.0; 2]), ([0.0; 2], [2.0; 2])], 0, true);
    assert_eq!(effect.age(0, 0.5), Some(0.5));
    assert_eq!(effect.age(1, 0.5), Some(0.25));
    assert_eq!(effect.age(0, 1.5), None);
    assert_eq!(effect.age(1, 1.5), Some(0.75));
    assert_eq!(effect.age(0, 2.5), Some(0.5));
}

#[test]
fn a_part_with_no_placement_is_its_location() {
    let frame = part_frame(&location(), [0.0; 3], 0.0, 0.0);
    assert_eq!(frame.position, [1.0, 2.0, 3.0]);
    assert_eq!(frame.axes, location().axes);
    assert_eq!(frame.node, 4);
}

#[test]
fn an_offset_runs_along_the_locations_axes_not_the_worlds() {
    let frame = part_frame(&location(), [0.5, 0.0, 0.0], 0.0, 0.0);
    assert_eq!(frame.position, [1.0, 2.5, 3.0]);
    let frame = part_frame(&location(), [0.0, 0.5, 0.0], 0.0, 0.0);
    assert_eq!(frame.position, [0.5, 2.0, 3.0]);
}

#[test]
fn yaw_turns_toward_left_and_pitch_toward_up() {
    let quarter = std::f32::consts::FRAC_PI_2;
    let [forward, _, _] = part_rotation(quarter, 0.0);
    assert!((forward[1] - 1.0).abs() < 1e-6 && forward[0].abs() < 1e-6);
    let [forward, _, up] = part_rotation(0.0, quarter);
    assert!((forward[2] - 1.0).abs() < 1e-6 && forward[0].abs() < 1e-6);
    assert!((up[0] + 1.0).abs() < 1e-6);
}
