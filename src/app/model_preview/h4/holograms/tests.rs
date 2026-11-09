use super::pass::*;
use super::*;

#[test]
fn a_new_loads_post_uploads_again_but_the_same_post_does_not() {
    let held = Arc::new(0u8);
    assert!(uploaded_from(Some(&held), &held.clone()));
    // A later load reads an equal post into a new allocation.
    assert!(!uploaded_from(Some(&held), &Arc::new(0u8)));
    assert!(!uploaded_from(None, &held));
}

/// The sphere rides node 0 and shrinks with node 1's scale: at rest it is
/// the offset from node 0's bind place; posed half size and moved up, the
/// radius halves and the centre follows.
#[test]
fn the_sphere_follows_the_root_and_the_animations_scale() {
    // Node 0 bound one unit up: its inverse bind takes it back down.
    let inverse_bind = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, -1.0],
    ];
    let (center, radius) = scaled_sphere([0.0, 0.0, 0.3], 0.35, Some(inverse_bind), None, None);
    assert!((center[2] - 1.3).abs() < 1e-6 && radius == 0.35);
    let half = [
        [0.5, 0.0, 0.0, 0.0],
        [0.0, 0.5, 0.0, 0.0],
        [0.0, 0.0, 0.5, 2.0],
    ];
    let (center, radius) = scaled_sphere(
        [0.0, 0.0, 0.3],
        0.35,
        Some(inverse_bind),
        Some(half),
        Some(half),
    );
    assert!((radius - 0.175).abs() < 1e-6, "{radius}");
    assert!((center[2] - (0.5 * 1.3 + 2.0)).abs() < 1e-5, "{center:?}");
}

/// A sphere straight ahead, 10 away, 1 across, through a 90° lens on a
/// 200 × 100 picture whose pixels are square at that depth.
fn ahead(point: [f32; 3]) -> Option<([f32; 2], f32)> {
    let depth = point[1];
    (depth > 0.0).then(|| {
        (
            [
                100.0 + point[0] / depth * 100.0,
                50.0 + point[2] / depth * 50.0,
            ],
            depth,
        )
    })
}

#[test]
fn coverage_is_the_spheres_box_with_the_games_top_as_gl_y() {
    let screen = hologram_screen(
        [0.0, 10.0, 0.0],
        1.0,
        ahead,
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        false,
        1.0,
        [200, 100],
    )
    .unwrap();
    assert_eq!(
        screen.coverage,
        [90.0, 55.0, 20.0, -10.0],
        "x, the top in GL's y, width, and height running down"
    );
    // The box's near corners reach further out than the sphere's points.
    let [x0, y0, x1, y1] = screen.crop;
    assert!(
        x0 < 90 && x1 > 110 && y0 < 45 && y1 > 55,
        "{:?}",
        screen.crop
    );
    assert_eq!(screen.texcoords[0], x0 as f32 / 200.0);
    assert_eq!(screen.texcoords[3], y1 as f32 / 100.0);
}

/// Inside its box the whole picture serves; off the picture, nothing.
#[test]
fn the_camera_inside_takes_the_whole_picture_and_off_screen_takes_none() {
    let inside = hologram_screen(
        [0.0, 10.0, 0.0],
        1.0,
        ahead,
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        true,
        1.0,
        [200, 100],
    )
    .unwrap();
    assert_eq!(
        (inside.crop, inside.texcoords),
        ([0, 0, 200, 100], [0.0, 1.0, 0.0, 1.0])
    );
    let aside = |point: [f32; 3]| ahead(point).map(|([x, y], depth)| ([x + 1000.0, y], depth));
    assert!(
        hologram_screen(
            [0.0, 10.0, 0.0],
            1.0,
            aside,
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            false,
            1.0,
            [200, 100]
        )
        .is_none()
    );
}

/// `max(1, d · vfov / 0.86 · 720 / H)`: near, the bloom keeps its own
/// strength; far, it grows with the distance.
#[test]
fn the_bloom_grows_with_distance_past_where_it_starts() {
    let at = |depth: f32| {
        let point = |p: [f32; 3]| ahead([p[0], p[1] - 10.0 + depth, p[2]]);
        hologram_screen(
            [0.0, 10.0, 0.0],
            1.0,
            point,
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            false,
            1.0,
            [200, 720],
        )
        .map(|screen| screen.bloom_scale)
    };
    assert_eq!(at(0.5), Some(1.0));
    let far = at(8.6).unwrap();
    assert!((far - 10.0).abs() < 1e-3, "{far}");
}
