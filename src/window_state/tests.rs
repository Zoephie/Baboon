use super::*;

fn monitor(
    name: &str,
    primary: bool,
    bounds: [f32; 4],
    work: [f32; 4],
    scale: f32,
) -> MonitorGeometry {
    MonitorGeometry {
        name: Some(name.to_owned()),
        is_primary: primary,
        bounds_px: PixelRect {
            x: bounds[0],
            y: bounds[1],
            width: bounds[2],
            height: bounds[3],
        },
        work_area_px: PixelRect {
            x: work[0],
            y: work[1],
            width: work[2],
            height: work[3],
        },
        scale_factor: scale,
    }
}

/// How far inside the frame origin the fixture window's content starts —
/// a left border and a title bar.
const DECORATION_INSET: [f32; 2] = [8.0, 32.0];
/// Total size the fixture window's decorations add around its content:
/// a border on both sides, and the title bar plus a bottom border.
const DECORATION_SIZE: [f32; 2] = [16.0, 40.0];

/// `restore_viewport` returns whatever `ViewportBuilder::with_position`
/// expects, and winit reads that as the *content* top-left on macOS but the
/// *frame* top-left on Windows and X11. Turn a frame origin into the value
/// the platform under test should produce.
fn expected_position(frame_origin: [f32; 2]) -> Option<[f32; 2]> {
    let mut position = frame_origin;
    if cfg!(target_os = "macos") {
        position[0] += DECORATION_INSET[0];
        position[1] += DECORATION_INSET[1];
    }
    Some(position)
}

fn state(mode: WindowMode, position: [f32; 2], size: [f32; 2]) -> PersistedWindowState {
    PersistedWindowState {
        schema_version: SCHEMA_VERSION,
        mode,
        normal: NormalWindowBounds {
            inner_position_px: Some(Point {
                x: position[0] + DECORATION_INSET[0],
                y: position[1] + DECORATION_INSET[1],
            }),
            outer_position_px: Some(Point {
                x: position[0],
                y: position[1],
            }),
            inner_size_logical: Size {
                width: size[0],
                height: size[1],
            },
            outer_size_logical: Size {
                width: size[0] + DECORATION_SIZE[0],
                height: size[1] + DECORATION_SIZE[1],
            },
            native_scale_factor: 1.0,
            monitor_name: Some("primary".to_owned()),
            monitor_bounds_px: Some(PixelRect {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            }),
        },
    }
}

fn primary_monitor() -> MonitorGeometry {
    monitor(
        "primary",
        true,
        [0.0, 0.0, 1920.0, 1080.0],
        [0.0, 0.0, 1920.0, 1040.0],
        1.0,
    )
}

#[test]
fn schema_round_trips_every_window_mode() {
    for mode in [
        WindowMode::Normal,
        WindowMode::Maximized,
        WindowMode::Fullscreen,
    ] {
        let original = state(mode, [120.0, 80.0], [1000.0, 700.0]);
        let encoded = serde_json::to_string(&original).unwrap();
        assert_eq!(parse_state(&encoded), Some(original));
    }
}

#[test]
fn corrupt_partial_future_and_non_finite_states_fall_back() {
    assert!(parse_state("not json").is_none());
    assert!(parse_state(r#"{"schema_version":1}"#).is_none());
    let future = serde_json::to_string(&PersistedWindowState {
        schema_version: 2,
        ..state(WindowMode::Normal, [0.0, 0.0], [800.0, 600.0])
    })
    .unwrap();
    assert!(parse_state(&future).is_none());

    let invalid = r#"{
            "schema_version": 1,
            "mode": "normal",
            "normal": {
                "inner_position_px": {"x": 0.0, "y": 0.0},
                "outer_position_px": {"x": 0.0, "y": 0.0},
                "inner_size_logical": {"width": -1.0, "height": 600.0},
                "outer_size_logical": {"width": 816.0, "height": 640.0},
                "native_scale_factor": 1.0,
                "monitor_name": null,
                "monitor_bounds_px": null
            }
        }"#;
    assert!(parse_state(invalid).is_none());
    assert!(
        !Point {
            x: f32::NAN,
            y: 0.0
        }
        .is_finite()
    );
    let mut invalid_values = state(WindowMode::Normal, [0.0, 0.0], [800.0, 600.0]);
    invalid_values.normal.native_scale_factor = f32::INFINITY;
    assert!(!invalid_values.validate());
    invalid_values.normal.native_scale_factor = 1.0;
    invalid_values.normal.inner_size_logical.width = f32::NAN;
    assert!(!invalid_values.validate());
    invalid_values.normal.inner_size_logical.width = 900.0;
    invalid_values.normal.outer_size_logical.width = 800.0;
    assert!(!invalid_values.validate());
}

/// The saved sample in `testdata/compat` still loads, and the same file
/// claiming a newer schema does not.
#[test]
fn compat_window_state_sample() {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/compat/samples/window_state/window-state.json"),
    )
    .unwrap();
    let state = parse_state(&text).expect("sample loads");
    assert_eq!(state.mode, WindowMode::Maximized);
    let future = text.replace("\"schema_version\": 1", "\"schema_version\": 2");
    assert_ne!(future, text);
    assert!(parse_state(&future).is_none());
}

#[test]
fn restored_dimensions_obey_minimum_and_work_area() {
    let tiny = state(WindowMode::Normal, [10.0, 10.0], [100.0, 100.0]);
    let restored = restore_viewport(&tiny, &[primary_monitor()]).unwrap();
    assert_eq!(restored.inner_size, MIN_INNER_SIZE);

    let huge = state(WindowMode::Normal, [10.0, 10.0], [4000.0, 3000.0]);
    let restored = restore_viewport(&huge, &[primary_monitor()]).unwrap();
    assert_eq!(restored.inner_size, [1904.0, 1000.0]);
}

#[test]
fn negative_coordinate_monitor_is_restored_without_primary_fallback() {
    let monitors = [
        monitor(
            "left",
            false,
            [-1600.0, 0.0, 1600.0, 900.0],
            [-1600.0, 0.0, 1600.0, 860.0],
            1.0,
        ),
        primary_monitor(),
    ];
    let mut saved = state(WindowMode::Normal, [-1400.0, 100.0], [900.0, 600.0]);
    saved.normal.monitor_name = Some("left".to_owned());
    let restored = restore_viewport(&saved, &monitors).unwrap();
    assert_eq!(restored.position, expected_position([-1400.0, 100.0]));
}

#[test]
fn disconnected_or_barely_visible_window_is_centered_on_primary() {
    let disconnected = state(WindowMode::Normal, [4000.0, 300.0], [1000.0, 700.0]);
    let restored = restore_viewport(&disconnected, &[primary_monitor()]).unwrap();
    assert_eq!(restored.position, expected_position([452.0, 150.0]));

    let barely_visible = state(WindowMode::Normal, [1910.0, 100.0], [1000.0, 700.0]);
    let restored = restore_viewport(&barely_visible, &[primary_monitor()]).unwrap();
    assert_eq!(restored.position, expected_position([452.0, 150.0]));

    let secondary = monitor(
        "secondary",
        false,
        [1920.0, 0.0, 1920.0, 1080.0],
        [1920.0, 0.0, 1920.0, 1040.0],
        1.0,
    );
    let mut stale_secondary = state(WindowMode::Normal, [5000.0, 300.0], [1000.0, 700.0]);
    stale_secondary.normal.monitor_name = Some("secondary".to_owned());
    let restored = restore_viewport(&stale_secondary, &[primary_monitor(), secondary]).unwrap();
    assert_eq!(restored.position, expected_position([452.0, 150.0]));
}

#[test]
fn dpi_change_preserves_logical_size_and_clamps_physical_placement() {
    let mut saved = state(WindowMode::Normal, [100.0, 100.0], [900.0, 600.0]);
    saved.normal.native_scale_factor = 1.0;
    let scaled = monitor(
        "primary",
        true,
        [0.0, 0.0, 2560.0, 1440.0],
        [0.0, 0.0, 2560.0, 1400.0],
        2.0,
    );
    let restored = restore_viewport(&saved, &[scaled]).unwrap();
    assert_eq!(restored.inner_size, [900.0, 600.0]);
    assert_eq!(restored.position, expected_position([50.0, 50.0]));
}

#[test]
fn special_modes_restore_without_losing_normal_bounds() {
    for (mode, maximized) in [
        (WindowMode::Normal, false),
        (WindowMode::Maximized, true),
        (WindowMode::Fullscreen, false),
    ] {
        let restored = restore_viewport(
            &state(mode, [120.0, 80.0], [1000.0, 700.0]),
            &[primary_monitor()],
        )
        .unwrap();
        assert_eq!(restored.position, expected_position([120.0, 80.0]));
        assert_eq!(restored.inner_size, [1000.0, 700.0]);
        assert_eq!(restored.maximized, maximized);
    }
}

#[test]
fn missing_platform_position_restores_size_and_leaves_placement_to_the_os() {
    let mut saved = state(WindowMode::Normal, [120.0, 80.0], [1000.0, 700.0]);
    saved.normal.inner_position_px = None;
    saved.normal.outer_position_px = None;
    let restored = restore_viewport(&saved, &[primary_monitor()]).unwrap();
    assert_eq!(restored.position, None);
    assert_eq!(restored.inner_size, [1000.0, 700.0]);
}

#[test]
fn tracker_preserves_normal_bounds_through_special_modes() {
    let initial = state(WindowMode::Normal, [120.0, 80.0], [1000.0, 700.0]);
    let initial_normal = initial.normal.clone();
    let changed_normal = state(WindowMode::Normal, [200.0, 160.0], [900.0, 600.0]).normal;
    let mut tracker = WindowStateTracker::new(None, vec![primary_monitor()], Some(initial));

    tracker.record_observation(WindowMode::Maximized, Some(changed_normal.clone()));
    assert_eq!(
        tracker.current.as_ref().unwrap().normal,
        initial_normal,
        "maximized frames must not replace the OS restore rectangle"
    );
    tracker.record_observation(WindowMode::Fullscreen, Some(changed_normal.clone()));
    assert_eq!(tracker.current.as_ref().unwrap().normal, initial_normal);
    tracker.record_observation(WindowMode::Normal, Some(changed_normal.clone()));
    assert_eq!(tracker.current.as_ref().unwrap().normal, changed_normal);
}

#[test]
fn atomic_writer_replaces_complete_state() {
    let temp = std::env::temp_dir().join(format!(
        "baboon-window-state-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&temp).unwrap();
    let path = temp.join("window-state.json");
    fs::write(&path, "old").unwrap();
    let saved = state(WindowMode::Maximized, [20.0, 30.0], [900.0, 600.0]);
    write_state_atomic(&path, &saved).unwrap();
    assert_eq!(
        parse_state(&fs::read_to_string(&path).unwrap()),
        Some(saved)
    );
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn uncommitted_atomic_write_preserves_previous_file() {
    let temp = std::env::temp_dir().join(format!(
        "baboon-window-state-interrupted-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&temp).unwrap();
    let path = temp.join("window-state.json");
    fs::write(&path, "previous").unwrap();
    {
        let mut file = AtomicWriteFile::open(&path).unwrap();
        file.write_all(b"incomplete").unwrap();
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), "previous");
    fs::remove_dir_all(temp).unwrap();
}
