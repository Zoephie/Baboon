use super::*;

fn translation_of(matrix: &WorldMatrix) -> [f64; 3] {
    [matrix[12], matrix[13], matrix[14]]
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn an_unrotated_component_places_where_it_says() {
    let matrix = compose([-38869.7, 19286.6, 12838.1], [0.0; 3], [3.0; 3]);
    assert!(close(translation_of(&matrix), [-38869.7, 19286.6, 12838.1]));
    // Scale lands on the basis rows, not the translation.
    assert!((matrix[0] - 3.0).abs() < 1e-9);
    assert!((matrix[5] - 3.0).abs() < 1e-9);
    assert!((matrix[10] - 3.0).abs() < 1e-9);
}

#[test]
fn a_yaw_turns_x_towards_y() {
    // Yaw is the second component of an FRotator, not the first: getting
    // that wrong rotates a level about the wrong axis and still looks
    // plausible in isolation.
    let matrix = compose([0.0; 3], [0.0, 90.0, 0.0], [1.0; 3]);
    assert!((matrix[0]).abs() < 1e-9);
    assert!((matrix[1] - 1.0).abs() < 1e-9);
    assert!((matrix[4] + 1.0).abs() < 1e-9);
    assert!((matrix[5]).abs() < 1e-9);
}

#[test]
fn a_child_is_placed_through_its_parent() {
    // Applying the child first and then the parent is what puts an attached
    // component in the right place; the other order rotates the parent's
    // offset by the child's rotation.
    let parent = compose([100.0, 0.0, 0.0], [0.0, 90.0, 0.0], [1.0; 3]);
    let child = compose([10.0, 0.0, 0.0], [0.0; 3], [1.0; 3]);
    let world = multiply(&child, &parent);
    assert!(close(translation_of(&world), [100.0, 10.0, 0.0]));
}

#[test]
fn scale_multiplies_down_the_chain() {
    let parent = compose([0.0; 3], [0.0; 3], [2.0; 3]);
    let child = compose([5.0, 0.0, 0.0], [0.0; 3], [3.0; 3]);
    let world = multiply(&child, &parent);
    assert!(close(translation_of(&world), [10.0, 0.0, 0.0]));
    assert!((world[0] - 6.0).abs() < 1e-9);
}

#[test]
fn identity_is_neutral() {
    let matrix = compose([1.0, 2.0, 3.0], [10.0, 20.0, 30.0], [1.5; 3]);
    assert_eq!(multiply(&matrix, &IDENTITY), matrix);
    assert_eq!(multiply(&IDENTITY, &matrix), matrix);
}

#[test]
fn a_scene_names_each_mesh_once() {
    let mut scene = LevelScene::default();
    scene.place("/Game/SM_Tree", IDENTITY);
    scene.place("/Game/SM_Rock", IDENTITY);
    scene.place(
        "/Game/SM_Tree",
        compose([5.0, 0.0, 0.0], [0.0; 3], [1.0; 3]),
    );
    assert_eq!(scene.meshes, ["/Game/SM_Tree", "/Game/SM_Rock"]);
    assert_eq!(scene.placements.len(), 3);
    assert_eq!(scene.placements[2].mesh, 0);
}

/// A reader that panics loses its cells; that has to reach the export's
/// summary rather than vanish.
#[test]
fn cells_lost_to_a_panicked_reader_are_reported() {
    let lost = chunk_scene(12, Err(Box::new("reader panicked")));
    assert_eq!(lost.cells, 12);
    assert_eq!(lost.skipped.lost_cells, 12);
    let mut scene = LevelScene::default();
    scene.absorb(lost);
    assert_eq!(
        scene.skipped.summary().as_deref(),
        Some("Left out: 12 cell(s) lost to a reader crash")
    );
    assert_eq!(LevelSkips::default().summary(), None);
}

#[test]
fn skips_are_counted_by_reason() {
    let mut skips = LevelSkips::default();
    skips.absorb(LevelSkips {
        inherited_mesh: 2,
        unresolved_mesh: 1,
        unreadable_instances: 3,
        ..LevelSkips::default()
    });
    skips.absorb(LevelSkips {
        inherited_mesh: 1,
        ..LevelSkips::default()
    });
    assert_eq!(skips.inherited_mesh, 3);
    assert_eq!(skips.total(), 7);
}
