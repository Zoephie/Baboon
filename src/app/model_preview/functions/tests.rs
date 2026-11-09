use super::super::h2::function_exports::{DEVICE_EXPORTS, OBJECT_EXPORTS, TypeExports};
use super::*;
use crate::app::model_preview::harness::kit::kit_root;
use crate::core::game::GameId;
use blam_tags::tag_function::FunctionType;
use blam_tags::{TagFile, TagFunction};
use std::sync::Arc;

fn identity() -> Arc<TagFunction> {
    let mut function = blam_tags::tag_function::H2Function::new(FunctionType::Identity);
    function.set_clamp_range(0.0, 1.0).unwrap();
    let encoding = blam_tags::tag_function::FunctionEncoding::H2;
    Arc::new(TagFunction::parse_encoded(encoding, &function.to_bytes()).unwrap())
}

#[test]
fn an_events_function_goes_by_its_field_and_a_region_name_does_not() {
    use super::super::effects::is_event_function;
    assert!(is_event_function("firing effect: firing"));
    assert!(is_event_function(
        "transition effect (generic): human_explosion_large"
    ));
    assert!(!is_event_function("headlights"));
    assert!(!is_event_function("turret: power"));
}

#[test]
fn only_what_reaches_something_drawn_counts() {
    let mut shield = function("current_shield_damage", "shield_hit");
    shield.scale_by = "scale".to_owned();
    let mut down = function("shield_depleted", "shield_down");
    down.turn_off_with = "body_vitality".to_owned();
    let chained = function("shield_hit", "glow");
    let functions = ObjectFunctions {
        functions: vec![shield, down, chained],
        ..Default::default()
    };
    let reached = functions.reaching(&["glow".to_owned()]);
    for name in ["glow", "shield_hit", "current_shield_damage", "scale"] {
        assert!(reached.iter().any(|known| known == name), "{name}");
    }
    for name in ["shield_down", "shield_depleted", "body_vitality"] {
        assert!(!reached.iter().any(|known| known == name), "{name}");
    }
}

fn function(import: &str, export: &str) -> ObjectFunction {
    ObjectFunction {
        flags: FunctionFlags::default(),
        import: import.to_owned(),
        export: export.to_owned(),
        turn_off_with: String::new(),
        range: String::new(),
        min_value: 0.0,
        function: identity(),
        scale_by: String::new(),
    }
}

fn value(functions: Vec<ObjectFunction>, name: &str, inputs: &[(&str, f32)]) -> f32 {
    let inputs = inputs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect();
    let functions = ObjectFunctions {
        functions,
        ..Default::default()
    };
    functions
        .value(name, &inputs, &FunctionValues::new(), Clock::default(), 0)
        .0
}

#[test]
fn a_script_sets_what_a_function_makes_and_what_it_reads() {
    let functions = vec![function("", "invincibility"), function("zero", "effects")];
    let functions = ObjectFunctions {
        functions,
        ..Default::default()
    };
    let none = FunctionValues::new();
    let scripted: FunctionValues = [("invincibility".to_owned(), 1.0)].into_iter().collect();
    assert_eq!(
        functions
            .value("invincibility", &none, &none, Clock::default(), 0)
            .0,
        0.0,
        "an empty import reads nothing"
    );
    assert_eq!(
        functions
            .value("invincibility", &none, &scripted, Clock::default(), 0)
            .0,
        1.0
    );
    let by_import: FunctionValues = [("zero".to_owned(), 0.75)].into_iter().collect();
    assert_eq!(
        functions
            .value("effects", &none, &by_import, Clock::default(), 0)
            .0,
        0.75,
        "the import's script value, unmapped"
    );
}

fn switch(switch: &str, export: &str, cases: &[([f32; 2], &str)]) -> FunctionSwitch {
    let cases = cases
        .iter()
        .map(|(bounds, function)| (*bounds, (*function).to_owned()))
        .collect();
    FunctionSwitch {
        switch: switch.to_owned(),
        export: export.to_owned(),
        cases,
    }
}

fn switched(
    functions: Vec<ObjectFunction>,
    switches: Vec<FunctionSwitch>,
    inputs: &[(&str, f32)],
) -> FunctionValues {
    let functions = ObjectFunctions {
        functions,
        switches,
        rules: FunctionRules::Gen3,
        exports: None,
    };
    let inputs: FunctionInputs = inputs
        .iter()
        .map(|(name, value)| ((0, (*name).to_owned()), *value))
        .collect();
    functions.values(&inputs, 0, &[], 0.0, 0)
}

/// `ObjectFunctionSwitchDefinition::Evaluate`: the first case holding the
/// switch's value, bounds included, else 0.
#[test]
fn a_switch_answers_with_the_first_case_its_value_falls_in() {
    let functions = vec![function("red", "red_out"), function("blue", "blue_out")];
    let cases = [([0.0, 0.5], "red_out"), ([0.5, 1.0], "blue_out")];
    let switches = vec![switch("pick", "color", &cases)];
    let at = |pick: f32| {
        switched(
            functions.clone(),
            switches.clone(),
            &[("pick", pick), ("red", 0.25), ("blue", 0.75)],
        )["color"]
    };
    assert_eq!(at(0.0), 0.25);
    assert_eq!(at(0.5), 0.25, "both ends are in, and the first case wins");
    assert_eq!(at(0.8), 0.75);
    let gap = vec![switch("pick", "color", &[([0.0, 0.25], "red_out")])];
    assert_eq!(
        switched(functions.clone(), gap, &[("pick", 0.5), ("red", 0.25)])["color"],
        0.0
    );
}

/// Halo 4's `object_get_function_value` asks a switch only when no
/// function has the name, and the last switch of a name stands.
#[test]
fn a_function_beats_a_switch_and_the_last_switch_stands() {
    let functions = vec![
        function("red", "color"),
        function("blue", "blue_out"),
        function("red", "red_out"),
    ];
    let switches = vec![switch("", "color", &[([0.0, 1.0], "blue_out")])];
    assert_eq!(
        switched(
            functions.clone(),
            switches,
            &[("red", 0.25), ("blue", 0.75)]
        )["color"],
        0.25
    );
    let switches = vec![
        switch("", "shade", &[([0.0, 1.0], "red_out")]),
        switch("", "shade", &[([0.0, 1.0], "blue_out")]),
    ];
    assert_eq!(
        switched(functions, switches, &[("red", 0.25), ("blue", 0.75)])["shade"],
        0.75
    );
}

/// The panel offers what a switch reads, and not what it makes.
#[test]
fn a_switch_reads_its_switch_function_and_makes_its_export() {
    let functions = ObjectFunctions {
        functions: vec![function("red", "red_out")],
        switches: vec![switch("pick", "color", &[([0.0, 1.0], "red_out")])],
        ..Default::default()
    };
    let imports = functions.imports();
    assert!(
        imports.contains(&"pick") && imports.contains(&"red"),
        "{imports:?}"
    );
    assert!(functions.makes("color"));
    let reached = functions.reaching(&["color".to_owned()]);
    for name in ["pick", "red_out", "red"] {
        assert!(reached.iter().any(|known| known == name), "{name}");
    }
}

/// With `BLAM_TEST_H4EK`: Cortana's hologram colors come from switches.
#[test]
fn halo_4_cortana_reads_her_function_switches() {
    let Some(root) = kit_root(GameId::Halo4) else {
        return;
    };
    let path =
        std::path::Path::new(&root).join("objects/characters/storm_cortana/storm_cortana.biped");
    let tag = TagFile::read(&path).unwrap();
    let functions = read_object_functions(&tag, None).unwrap();
    assert!(!functions.switches.is_empty());
    for switch in &functions.switches {
        assert!(
            !switch.export.is_empty() && !switch.switch.is_empty(),
            "{switch:?}"
        );
        assert!(
            switch
                .cases
                .iter()
                .all(|(_, function)| !function.is_empty()),
            "{switch:?}"
        );
        assert!(functions.makes(&switch.export), "{}", switch.export);
    }
    let values = functions.values(&FunctionInputs::new(), 0, &[], 0.0, 0);
    assert!(
        functions
            .switches
            .iter()
            .all(|switch| values.contains_key(&switch.export))
    );
}

#[test]
fn an_output_follows_what_it_imports() {
    assert_eq!(
        value(vec![function("speed", "glow")], "glow", &[("speed", 0.5)]),
        0.5
    );
    assert_eq!(
        value(vec![function("speed", "glow")], "glow", &[("speed", 0.0)]),
        0.0
    );
}

#[test]
fn nothing_named_reads_full_and_an_unknown_name_reads_nothing() {
    assert_eq!(value(Vec::new(), "", &[]), 1.0);
    assert_eq!(value(Vec::new(), "one", &[]), 1.0);
    assert_eq!(value(Vec::new(), "no_such_function", &[]), 0.0);
}

#[test]
fn invert_and_scale_by_shape_the_value() {
    let inverted = ObjectFunction {
        flags: FunctionFlags {
            invert: true,
            ..Default::default()
        },
        ..function("speed", "glow")
    };
    assert_eq!(value(vec![inverted], "glow", &[("speed", 0.25)]), 0.75);
    let scaled = ObjectFunction {
        scale_by: "power".to_owned(),
        ..function("speed", "glow")
    };
    assert_eq!(
        value(vec![scaled], "glow", &[("speed", 0.5), ("power", 0.5)]),
        0.25
    );
}

fn device(functions: Vec<ObjectFunction>) -> ObjectFunctions {
    let tables = &[OBJECT_EXPORTS, DEVICE_EXPORTS];
    let exports = TypeExports {
        tables,
        has_body: true,
        has_shield: false,
        variants: 4,
    };
    ObjectFunctions {
        functions,
        exports: Some(std::sync::Arc::new(exports)),
        ..Default::default()
    }
}

#[test]
fn what_the_type_exports_answers_before_a_function_of_its_name() {
    // A control with its own `position` function, as several kit
    // switches have: the device's exporter wins, idle at 0.
    let functions = device(vec![function("one", "position")]);
    let none = FunctionValues::new();
    assert_eq!(
        functions.value("position", &none, &none, Clock::default(), 0),
        (0.0, false)
    );
    assert!(
        !functions.makes("position"),
        "it is an input, the panel's to set"
    );
    let set = FunctionValues::from([("position".to_owned(), 0.5)]);
    assert_eq!(
        functions
            .value("position", &set, &none, Clock::default(), 0)
            .0,
        0.5
    );
}

#[test]
fn an_idle_object_reads_what_its_type_and_model_give() {
    let functions = device(Vec::new());
    assert_eq!(functions.idle("power"), 1.0, "a device starts powered");
    assert_eq!(functions.idle("loop"), 1.0);
    assert_eq!(functions.idle("body_vitality"), 1.0);
    assert_eq!(
        functions.idle("shield_vitality"),
        0.0,
        "this model has no shield"
    );
    assert_eq!(functions.idle("shield_depleted"), 1.0);
    assert_eq!(functions.idle("variant"), 0.25, "the first of four");
    assert_eq!(
        functions.idle("turned_on"),
        0.0,
        "a weapon's, which a device does not export"
    );
}

#[test]
fn without_known_exporters_the_functions_answer_first() {
    // Halo 3 and ODST, whose exporters this does not have.
    let functions = ObjectFunctions {
        functions: vec![function("one", "position")],
        ..Default::default()
    };
    let none = FunctionValues::new();
    assert_eq!(
        functions.value("position", &none, &none, Clock::default(), 0),
        (1.0, true)
    );
}

#[test]
fn a_min_value_keeps_a_function_off_below_it() {
    let held = ObjectFunction {
        min_value: 0.5,
        ..function("speed", "glow")
    };
    assert_eq!(value(vec![held.clone()], "glow", &[("speed", 0.25)]), 0.0);
    assert_eq!(value(vec![held], "glow", &[("speed", 0.75)]), 0.75);
}

#[test]
fn turn_off_with_needs_its_export_on() {
    let tied = ObjectFunction {
        turn_off_with: "alive".to_owned(),
        ..function("speed", "glow")
    };
    assert_eq!(
        value(
            vec![tied.clone()],
            "glow",
            &[("speed", 1.0), ("alive", 0.0)]
        ),
        0.0
    );
    assert_eq!(
        value(vec![tied], "glow", &[("speed", 1.0), ("alive", 1.0)]),
        1.0
    );
}

fn gen3(functions: Vec<ObjectFunction>, name: &str, inputs: &[(&str, f32)]) -> (f32, bool) {
    let inputs = inputs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect();
    let functions = ObjectFunctions {
        functions,
        rules: FunctionRules::Gen3,
        ..Default::default()
    };
    functions.value(name, &inputs, &FunctionValues::new(), Clock::default(), 0)
}

#[test]
fn halo_3_and_reach_take_the_first_function_of_a_name() {
    let list = vec![function("speed", "glow"), function("power", "glow")];
    assert_eq!(
        gen3(list.clone(), "glow", &[("speed", 0.25), ("power", 0.75)]).0,
        0.25
    );
    assert_eq!(
        value(list, "glow", &[("speed", 0.25), ("power", 0.75)]),
        0.75,
        "Halo 2 the last"
    );
}

#[test]
fn halo_3_and_reach_look_turn_off_with_up_in_full_and_can_ask_its_magnitude() {
    // `alive` is made by a function here, which Halo 2's exported read misses.
    let made = function("power", "alive");
    let tied = ObjectFunction {
        turn_off_with: "alive".to_owned(),
        ..function("speed", "glow")
    };
    let inputs = [("speed", 1.0), ("power", 0.0)];
    assert_eq!(
        gen3(vec![tied.clone(), made.clone()], "glow", &inputs),
        (0.0, false)
    );
    assert_eq!(
        value(vec![tied.clone(), made.clone()], "glow", &inputs),
        1.0,
        "Halo 2 reads the idle export"
    );
    let inputs = [("speed", 1.0), ("power", 0.5)];
    assert_eq!(
        gen3(vec![tied.clone(), made.clone()], "glow", &inputs),
        (1.0, true)
    );
    // On, but by magnitude: a mapping that holds it active at 0.
    let held = ObjectFunction {
        flags: FunctionFlags {
            always_active: true,
            ..Default::default()
        },
        ..made
    };
    let magnitude = ObjectFunction {
        flags: FunctionFlags {
            turn_off_with_uses_magnitude: true,
            ..Default::default()
        },
        ..tied.clone()
    };
    let inputs = [("speed", 1.0), ("power", 0.0)];
    assert_eq!(
        gen3(vec![tied, held.clone()], "glow", &inputs),
        (1.0, true),
        "on is enough"
    );
    assert_eq!(
        gen3(vec![magnitude, held], "glow", &inputs),
        (0.0, false),
        "0 turns it off"
    );
}

#[test]
fn always_exports_value_keeps_its_value_while_off() {
    let held = ObjectFunction {
        min_value: 0.5,
        ..function("speed", "glow")
    };
    let exporting = ObjectFunction {
        flags: FunctionFlags {
            always_exports_value: true,
            ..Default::default()
        },
        ..held.clone()
    };
    assert_eq!(gen3(vec![held], "glow", &[("speed", 0.25)]), (0.0, false));
    assert_eq!(
        gen3(vec![exporting], "glow", &[("speed", 0.25)]),
        (0.25, false)
    );
}

#[test]
fn a_range_name_feeds_the_curves_range() {
    // A curve whose output is its range: a constant of 0 below, 1 above.
    let mut ranged = blam_tags::tag_function::H2Function::new(FunctionType::Constant);
    ranged.set_clamp_range(0.0, 1.0).unwrap();
    let encoding = blam_tags::tag_function::FunctionEncoding::H2;
    let curve = TagFunction::parse_encoded(encoding, &ranged.to_bytes()).unwrap();
    let low = curve.evaluate(0.5, 0.0);
    let high = curve.evaluate(0.5, 1.0);
    let function = ObjectFunction {
        range: "heat".to_owned(),
        function: Arc::new(curve),
        ..function("speed", "glow")
    };
    assert_eq!(
        gen3(
            vec![function.clone()],
            "glow",
            &[("speed", 0.5), ("heat", 0.0)]
        )
        .0,
        low.clamp(0.0, 1.0)
    );
    assert_eq!(
        gen3(vec![function], "glow", &[("speed", 0.5), ("heat", 1.0)]).0,
        high.clamp(0.0, 1.0)
    );
}

#[test]
fn the_last_function_of_a_name_stands_and_a_loop_ends() {
    let list = vec![function("speed", "glow"), function("power", "glow")];
    assert_eq!(
        value(list, "glow", &[("speed", 0.25), ("power", 0.75)]),
        0.75
    );
    let looped = ObjectFunction {
        scale_by: "glow".to_owned(),
        ..function("speed", "glow")
    };
    assert_eq!(value(vec![looped], "glow", &[("speed", 1.0)]), 0.0);
}
