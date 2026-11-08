use super::*;
use crate::app::model_preview::harness::kit::kit_source;
use crate::core::game::GameId;

fn sniper_eye(source: &TagSource) -> H4FlareAt {
    let flare = read_h4_lens_flare(&crate::app::model_preview::load::TagReader::new(source), "objects\\characters\\storm_jackal\\fx\\sniper_eye")
        .expect("the sniper eye flare");
    H4FlareAt {
        object: 0,
        node: 0,
        position: [0.0; 3],
        forward: [1.0, 0.0, 0.0],
        only_under: None,
        scale_by: "alive".to_owned(),
        secondary_by: String::new(),
        change_color: None,
        flare: Arc::new(flare),
    }
}

/// A pinhole at `eye` looking down -x toward the origin, 60 degrees
/// across a square viewport.
fn project(eye: [f32; 3]) -> impl Fn([f32; 3]) -> Option<([f32; 2], [f32; 2])> {
    move |point: [f32; 3]| {
        let depth = eye[0] - point[0];
        let focal = 1.0 / (30f32.to_radians()).tan();
        (depth > 0.0).then(|| {
            (
                [
                    -(point[1] - eye[1]) * focal / depth,
                    (point[2] - eye[2]) * focal / depth,
                ],
                [focal; 2],
            )
        })
    }
}

fn view<'a>(
    eye: [f32; 3],
    forward: [f32; 3],
    project: &'a dyn Fn([f32; 3]) -> Option<([f32; 2], [f32; 2])>,
) -> H4FlareView<'a> {
    H4FlareView {
        eye,
        forward,
        right: [0.0, -1.0, 0.0],
        up: [0.0, 0.0, 1.0],
        viewport: [800.0, 800.0],
        project,
    }
}

fn alive() -> FunctionValues {
    [("alive".to_owned(), 1.0)].into_iter().collect()
}

/// The kit's sniper eye: two reflections, the second on its own
/// anamorphic bitmap, read by name from the H4 reflection layout.
#[test]
fn the_sniper_eye_reads_both_reflections_and_their_bitmaps() {
    let Some(source) = kit_source(GameId::Halo4) else { return };
    let at = sniper_eye(&source);
    assert_eq!(at.flare.reflections.len(), 2);
    assert_eq!(at.flare.images.len(), 2);
    assert_eq!(at.flare.reflections[1].image, 1);
    assert!((at.flare.cutoff - 25f32.to_radians()).abs() < 1e-5);
    assert!(at.flare.reflections[0].flags.proximity_input);
}

/// Seen from in front it draws both reflections, centred where the eye
/// is; from behind its cone puts it out, and so does a dead Jackal.
#[test]
fn the_sniper_eye_shows_within_its_cone_while_alive() {
    let Some(source) = kit_source(GameId::Halo4) else { return };
    let at = sniper_eye(&source);
    let colors = [[1.0; 3]; 4];
    let front = project([2.0, 0.0, 0.0]);
    let ahead = view([2.0, 0.0, 0.0], [-1.0, 0.0, 0.0], &front);
    let draws = at.draws(Some(&alive()), &colors, None, &ahead, 0.0, 1.0);
    assert_eq!(draws.len(), 2, "{draws:?}");
    assert_eq!(&draws[0].center_rotation[..2], &[0.0, 0.0]);
    assert!(draws[0].modulation[2] > 0.0, "its brightness");
    assert!(
        at.draws(Some(&alive()), &colors, None, &ahead, 0.0, MINIMUM_VISIBLE)
            .is_empty(),
        "too little seen"
    );
    assert!(
        at.draws(None, &colors, None, &ahead, 0.0, 1.0).is_empty(),
        "no scale, no flare"
    );
    let back = project([-2.0, 0.0, 0.0]);
    let behind = view([-2.0, 0.0, 0.0], [1.0, 0.0, 0.0], &back);
    assert!(
        at.draws(Some(&alive()), &colors, None, &behind, 0.0, 1.0)
            .is_empty()
    );
}

/// `use simple occlusion box test`: the patch is the inner share of the
/// occlusion reflection's radius whatever its size, and expects 2.5 times
/// its area in samples.
#[test]
fn a_simple_box_occludes_against_its_inner_share() {
    let Some(source) = kit_source(GameId::Halo4) else { return };
    let at = sniper_eye(&source);
    assert!(at.flare.flags.simple_occlusion_box);
    let front = project([2.0, 0.0, 0.0]);
    let ahead = view([2.0, 0.0, 0.0], [-1.0, 0.0, 0.0], &front);
    let patch = at
        .occlusion_patch(Some(&alive()), &[[1.0; 3]; 4], None, &ahead, 0.0)
        .expect("a patch");
    let half = (patch.corners[2][1] - patch.corners[0][1]).abs() / 2.0;
    let reflection = &at.flare.reflections[0];
    let radius = reflection.radius.evaluate(1.0, 0.0) * at.flare.inner_scale;
    // The occlusion reflection's radius, before its occlusion scaling at
    // full opacity, times the inner share.
    assert!((half - radius).abs() < 1e-4, "{half} {radius}");
    let pixels = 400.0 / 2.0 * half;
    assert!(
        (patch.expected - (2.5 * 4.0 * pixels * pixels).max(1.0)).abs() < 1e-2,
        "{} {pixels}",
        patch.expected
    );
}

#[test]
fn visibility_counts_a_glimpse_as_ten_samples_and_expects_at_least_ten() {
    let Some(source) = kit_source(GameId::Halo4) else { return };
    let at = sniper_eye(&source);
    assert_eq!(at.visibility(Some((5.0, 100.0))), 0.1);
    assert_eq!(at.visibility(Some((0.0, 100.0))), 0.0);
    assert_eq!(at.visibility(Some((20.0, 3.0))), 2.0);
    assert_eq!(at.visibility(None), 0.0, "no patch, nothing seen");
    let mut untested = (*at.flare).clone();
    untested.flags.no_occlusion_test = true;
    let untested = H4FlareAt {
        flare: Arc::new(untested),
        ..at
    };
    assert_eq!(untested.visibility(None), 1.0);
}
