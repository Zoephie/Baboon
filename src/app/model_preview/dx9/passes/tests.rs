use super::super::super::functions::object_input;
use super::*;
use blam_tags::TagFunction;
use std::sync::Arc;

fn animation(kind: Dx9AnimationKind, period: f32, value: f32) -> Dx9Animation {
    let function = blam_tags::tag_function::H2Function::new_constant(value);
    Dx9Animation {
        kind,
        period,
        held_input: None,
        range_input: None,
        function: Arc::new(
            TagFunction::parse_encoded(
                blam_tags::tag_function::FunctionEncoding::H2,
                &function.to_bytes(),
            )
            .unwrap(),
        ),
    }
}

#[test]
fn a_period_wraps_time_and_no_period_reads_it_raw() {
    let cycling = animation(Dx9AnimationKind::Value, 4.0, 0.0);
    assert_eq!(cycling.input(1.0.into()), 0.25);
    assert_eq!(cycling.input(5.0.into()), 0.25);
    assert_eq!(
        animation(Dx9AnimationKind::Value, 0.0, 0.0).input(5.0.into()),
        5.0
    );
}

#[test]
fn a_texture_transform_moves_scales_and_turns_as_the_game_builds_it() {
    let row = |form, animations| {
        let mut rows = [[0.0; 4]; DX9_TEXTURE_ROWS];
        Dx9AnimatedRow {
            register: 1,
            form,
            scale: [2.0, 4.0, 8.0],
            animations,
        }
        .apply(&mut rows, 0.0.into());
        rows[1]
    };
    let moved = vec![
        animation(Dx9AnimationKind::TranslationX, 0.0, 0.25),
        animation(Dx9AnimationKind::TranslationY, 0.0, 0.5),
    ];
    // `sub_6D8B60` leaves a translation unscaled.
    assert_eq!(
        row(Dx9RowForm::SimpleXform, moved.clone()),
        [2.0, 4.0, 0.25, 0.5]
    );
    assert_eq!(row(Dx9RowForm::Row1, moved.clone()), [2.0, 0.0, 0.0, 0.25]);
    assert_eq!(row(Dx9RowForm::Row2, moved), [0.0, 4.0, 0.0, 0.5]);

    let deep = vec![animation(Dx9AnimationKind::TranslationZ, 0.0, 0.5)];
    assert_eq!(row(Dx9RowForm::Row3, deep), [0.0, 0.0, 8.0, 0.5]);
    let even = vec![animation(Dx9AnimationKind::ScaleUniform, 0.0, 3.0)];
    assert_eq!(
        row(Dx9RowForm::Row3, even),
        [0.0, 0.0, 3.0, 0.0],
        "uniform scale reaches z"
    );

    let turned = row(
        Dx9RowForm::Row1,
        vec![animation(
            Dx9AnimationKind::Rotation,
            0.0,
            std::f32::consts::FRAC_PI_2,
        )],
    );
    // A quarter turn about the middle of the map keeps the middle in place.
    assert!(turned[0].abs() < 1e-6, "{turned:?}");
    assert!((turned[1] + 2.0).abs() < 1e-6, "{turned:?}");
    assert!((turned[3] - 2.0).abs() < 1e-6, "{turned:?}");
}

#[test]
fn an_object_input_holds_still_at_what_a_whole_object_exports() {
    assert_eq!(object_input("shield_vitality"), 1.0);
    assert_eq!(object_input("object_overshield_amount"), 0.0);
    assert_eq!(object_input("boost"), 0.0);
    let held = Dx9Animation {
        held_input: Some(("boost".to_owned(), 1.0)),
        ..animation(Dx9AnimationKind::Value, 4.0, 0.0)
    };
    assert_eq!(held.input(3.0.into()), 1.0);
    assert!(!held.moves());
    // An object's functions take over from the resting value.
    let values = FunctionValues::from([("boost".to_owned(), 0.25)]);
    assert_eq!(
        held.input(Dx9Clock {
            time: 3.0,
            values: Some(&values)
        }),
        0.25
    );
}

#[test]
fn only_a_non_constant_function_moves() {
    assert!(!animation(Dx9AnimationKind::Value, 1.0, 0.5).moves());
}

#[test]
fn normalization_faces_hold_their_own_direction() {
    let center = |face: usize, size: usize| -> [u8; 3] {
        let rgba = normalization_face(face, size);
        let at = (size / 2 * size + size / 2) * 4;
        [rgba[at], rgba[at + 1], rgba[at + 2]]
    };
    assert!(center(0, 33)[0] > 250, "+X");
    assert!(center(3, 33)[1] < 5, "-Y");
    assert!(center(4, 33)[2] > 250, "+Z");
    assert!(center(5, 33)[2] < 5, "-Z");
}

/// The dx9 pipeline is neutral: Halo CE's shaders hook into it through
/// `PassAnimation` and `PassVisibility`, and its drawing through `Dx9Game`,
/// which the renderer's game table picks.
#[test]
fn dx9_names_no_halo_ce() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/model_preview/dx9");
    let mut files = vec![root.clone()];
    let mut naming = Vec::new();
    while let Some(path) = files.pop() {
        if path.is_dir() {
            files.extend(
                std::fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        // How many modules below `dx9` this file's module is: a `super`
        // chain one longer reaches the module root, where `ce` is Halo CE's
        // folder; a shorter one names dx9's own `stage::ce`.
        let relative = path.strip_prefix(&root).unwrap();
        let depth = relative.components().count()
            - usize::from(relative.file_name().is_some_and(|name| name == "mod.rs"));
        let source = std::fs::read_to_string(&path).unwrap();
        for (number, line) in source.lines().enumerate() {
            // Spelled apart, so this test's own source is not a match.
            let game = concat!("model_preview::", "ce::");
            let names_game = line.contains(game)
                || line.match_indices(concat!("ce", "::")).any(|(at, _)| {
                    let before = &line[..at];
                    let supers = before
                        .strip_suffix("::")
                        .map(|chain| {
                            chain
                                .rsplit(|c: char| !(c.is_alphanumeric() || c == ':' || c == '_'))
                                .next()
                                .unwrap_or("")
                        })
                        .unwrap_or("");
                    !supers.is_empty()
                        && supers.split("::").all(|segment| segment == "super")
                        && supers.split("::").count() == depth + 1
                });
            if names_game {
                naming.push(format!("{}:{}", path.display(), number + 1));
            }
        }
    }
    assert!(naming.is_empty(), "dx9 names Halo CE at {naming:?}");
}
