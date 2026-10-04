use super::*;
use crate::app::chimp::level::MeshPlacement;

/// Both level exports decode each mesh once, drop the placements of the
/// meshes that did not decode, and segment the rest.
#[test]
fn a_level_pass_drops_placements_of_meshes_that_did_not_decode() {
    let mut scene = LevelScene::default();
    scene.meshes = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    let at = |mesh: usize, x: f64| {
        let mut world = [0.0; 16];
        world[0] = 1.0;
        world[5] = 1.0;
        world[10] = 1.0;
        world[15] = 1.0;
        world[12] = x;
        MeshPlacement { mesh, world }
    };
    scene.placements = vec![at(0, 0.0), at(1, 10.0), at(2, 20.0), at(0, 30.0)];
    let progressed = std::cell::Cell::new(0);
    let pass = level_pass(
        &scene,
        SegmentBudget::default(),
        &|_, _, _| progressed.set(progressed.get() + 1),
        |package| {
            (package != "b").then(|| Prototype {
                prim: package.to_owned(),
                mesh: StaticMesh {
                    indices: vec![0; 3],
                    vertices: Vec::new(),
                },
                material: String::new(),
            })
        },
        |prototype| Ok(prototype.prim),
    )
    .unwrap();

    assert_eq!(
        pass.meshes,
        [Some("a".to_owned()), None, Some("c".to_owned())]
    );
    assert_eq!(pass.unreadable, 1);
    assert_eq!(pass.placed, [0, 2, 3], "b's placement has nothing to place");
    let segmented: usize = pass.segments.iter().map(|s| s.placements.len()).sum();
    assert_eq!(segmented, 3);
    assert_eq!(progressed.get(), 3);
}
