use super::*;
use blam_tags::math::{RealPoint2d, RealPoint3d, RealVector3d};
use blam_tags::render_model::{GeometryPartType, RenderMeshPart, RenderVertex};

/// Two previews on screen draw alternately. Each model is uploaded once
/// and then stays; with one slot, every draw replaced the other model.
#[test]
fn two_visible_previews_each_upload_once() {
    let mut lru = ModelSlotLru::default();
    let mut uploads = 0;
    let mut slots = std::collections::HashSet::new();
    for _ in 0..100 {
        for geometry in [10, 20] {
            let (slot, upload) = lru.acquire(geometry, MODEL_GPU_SLOTS);
            uploads += usize::from(upload);
            slots.insert((geometry, slot));
        }
    }
    assert_eq!(uploads, 2);
    assert_eq!(slots.len(), 2, "each model keeps its own slot");
}

/// Past capacity, the model drawn longest ago gives up its slot, not one
/// still being drawn.
#[test]
fn the_least_recently_drawn_model_is_evicted() {
    let mut lru = ModelSlotLru::default();
    for geometry in 1..=4 {
        lru.acquire(geometry, 4);
    }
    // 1 is drawn again, so 2 is now the oldest.
    assert_eq!(lru.acquire(1, 4), (0, false));
    let (slot, upload) = lru.acquire(5, 4);
    assert!(upload);
    assert_eq!(slot, 1, "model 2's slot");
    assert_eq!(lru.acquire(1, 4), (0, false), "still resident");
    assert!(lru.acquire(2, 4).1, "evicted, so uploaded again");
}

#[test]
fn dense_indices_use_full_u32_offsets_and_counts() {
    let start = 70_002_u32;
    let count = 120_003_u32;
    assert_eq!(
        model_draw_range(start, count, 250_000),
        Some(((start * 4) as i32, count as i32))
    );
}

#[test]
fn marker_transform_uses_its_animated_bone() {
    let marker = RenderModelPreviewMarker {
        name: "weapon".to_owned(),
        node_index: 1,
        position: [1.0, 2.0, 3.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    let rows = vec![
        [1.0, 0.0, 0.0, 99.0],
        [0.0, 1.0, 0.0, 99.0],
        [0.0, 0.0, 1.0, 99.0],
        [1.0, 0.0, 0.0, 0.5],
        [0.0, 1.0, 0.0, -0.25],
        [0.0, 0.0, 1.0, 1.0],
    ];
    let (position, axes) = animated_marker_transform(&marker, Some(&rows));
    assert_eq!(position, [1.5, 1.75, 4.0]);
    assert_eq!(axes, marker.axes);
}

/// The grid is a flat z = 0 line list with no skinning influences (a
/// stray weight would let an animated draw warp it), snapped to world
/// spacing, with the two axis lines appended last for their own colors.
#[test]
fn ground_grid_lies_flat_unskinned_and_ends_with_the_axes() {
    let preview = RenderModelPreview {
        bounds_min: [-0.3, -0.25, 0.0],
        bounds_max: [0.31, 0.25, 0.65],
        ..Default::default()
    };
    let (vertices, axis_start) = grid_line_vertices(&preview);
    assert_eq!(vertices.len(), axis_start + 4, "two axis lines at the end");
    assert_eq!(vertices.len() % 2, 0, "a line list pairs up");
    assert!(axis_start > 0, "no minor lines");
    for vertex in &vertices {
        assert_eq!(vertex.position[2], 0.0, "grid left the ground plane");
        assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
        assert_eq!(vertex.node_weights, [0.0; 4], "grid must not skin");
    }
    // Biped-sized bounds pick the 0.1-unit tier; every line then sits on
    // a multiple of it.
    for vertex in &vertices[..axis_start] {
        for value in [vertex.position[0], vertex.position[1]] {
            let snapped = (value / 0.1).round() * 0.1;
            assert!(
                (value - snapped).abs() < 1e-4,
                "line off the 0.1 spacing: {value}"
            );
        }
    }
    // The axis lines really are the world axes.
    assert_eq!(vertices[axis_start].position[1], 0.0);
    assert_eq!(vertices[axis_start + 1].position[1], 0.0);
    assert_eq!(vertices[axis_start + 2].position[0], 0.0);
    assert_eq!(vertices[axis_start + 3].position[0], 0.0);
}

/// The depth window must cover an animation's travel: the reach bound is
/// at least the largest per-frame chain length, and an empty pose adds
/// nothing.
#[test]
fn pose_reach_covers_the_travelling_frame() {
    let frame = |x: f32| {
        vec![
            PreviewNodeTransform {
                rotation: [0.0, 0.0, 0.0, 1.0],
                translation: [x, 0.0, 0.9],
                scale: 1.0,
            },
            PreviewNodeTransform {
                rotation: [0.0, 0.0, 0.0, 1.0],
                translation: [0.0, 0.0, 0.25],
                scale: 1.0,
            },
        ]
    };
    let pose = PreviewAnimationPose::new(0, vec![frame(0.0), frame(3.0)]);
    let reach = pose.reach;
    let travelled = (3.0f32 * 3.0 + 0.9 * 0.9).sqrt() + 0.25;
    assert!(
        reach >= travelled - 1e-4,
        "reach {reach} misses the travelled frame {travelled}"
    );
    let empty = PreviewAnimationPose::new(0, Vec::new());
    assert_eq!(empty.reach, 0.0);
}

/// The extended render distance must clip exactly at its declared bounds
/// in both projections, wherever the dolly has put the eye: near plane
/// just in front of the eye, far plane at t = K, and post-divide depth
/// monotone between them so the depth test orders correctly.
#[test]
fn depth_remap_clips_at_the_near_and_far_planes_for_any_eye() {
    let k = PREVIEW_RENDER_DISTANCE;

    // Orthographic: w = 1, so z = t·x + y must span NDC over t ∈ [-K, K].
    let [x, y] = depth_map_coefficients(None);
    assert!(((-k) * x + y - -1.0).abs() < 1e-5);
    assert!((k * x + y - 1.0).abs() < 1e-5);

    // Perspective, from the default framing's eye down to a dolly deep
    // inside a level.
    for eye in [1.9, 0.5, 0.02, 0.0005] {
        let [x, y] = depth_map_coefficients(Some(eye));
        let z = |t: f32| t * x + y;
        let w = |t: f32| 1.0 + t / eye;
        let near = -eye * (1.0 - PERSPECTIVE_NEAR_FRACTION);
        let tolerance = 1e-4 * w(k);
        assert!(
            (z(near) + w(near)).abs() < tolerance,
            "eye {eye}: near plane moved"
        );
        assert!(
            (z(k) - w(k)).abs() < tolerance,
            "eye {eye}: far plane short of K"
        );
        let samples = (0..=64)
            .map(|step| near + (k - near) * step as f32 / 64.0)
            .map(|t| z(t) / w(t))
            .collect::<Vec<_>>();
        // Strictly increasing wherever f32 can tell the samples apart;
        // at the deepest dolly the far samples round together, so ties
        // are allowed there but never a reversal.
        assert!(
            samples.windows(2).all(|pair| if eye >= 0.02 {
                pair[1] > pair[0]
            } else {
                pair[1] >= pair[0]
            }),
            "eye {eye}: post-divide depth not monotone"
        );
    }
}

fn camera(scale: f32, perspective: bool) -> PreviewCamera {
    PreviewCamera {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0)),
        center: [0.0; 3],
        radius: 1.0,
        depth_radius: 1.0,
        // Unrotated: view space is world space, the eye looks along +Y.
        yaw: 0.0,
        pitch: 0.0,
        scale,
        perspective,
    }
}

/// How far from the viewport center a point lands, in pixels.
fn screen_offset(camera: &PreviewCamera, point: [f32; 3]) -> f32 {
    (camera.project(point).pos - camera.rect.center()).length()
}

/// On the orbit point's plane, perspective frames exactly like the
/// orthographic view at every zoom, so toggling never jumps and the
/// pan / zoom-to-cursor math holds in both.
#[test]
fn perspective_matches_orthographic_at_the_focus_plane() {
    for scale in [0.05, 1.0, 20.0, 400.0] {
        let point = [0.3 / scale, 0.0, 0.2 / scale];
        let flat = camera(scale, false).project(point).pos;
        let deep = camera(scale, true).project(point).pos;
        assert!(
            (flat - deep).length() < 1e-3,
            "scale {scale}: {flat:?} vs {deep:?}"
        );
    }
}

/// The reported defect: zooming in only magnified the picture, so the
/// perspective stayed the same strength and up close looked flat. With
/// the eye dollying in, a point the same world distance nearer the
/// camera than the orbit point grows ever larger relative to one on the
/// focus plane.
#[test]
fn zooming_in_dollies_the_eye_and_deepens_the_perspective() {
    let convergence = |scale: f32| {
        let camera = camera(scale, true);
        let offset = 0.1 / scale;
        let nearer = screen_offset(&camera, [offset, -0.2, 0.0]);
        let on_plane = screen_offset(&camera, [offset, 0.0, 0.0]);
        nearer / on_plane
    };
    let (wide, close) = (convergence(1.0), convergence(8.0));
    assert!(wide > 1.05, "no perspective at the default zoom: {wide}");
    assert!(
        close > wide * 1.5,
        "zoom did not deepen the perspective: {wide} → {close}"
    );

    // The lens itself does not change: at every zoom the eye sits where
    // a 60° field of view frames the focus plane, `1.1 · radius /
    // tan(30°)` out in scaled units — `1 / scale` of that in world units.
    for scale in [1.0, 8.0, 100.0] {
        let eye_world = camera(scale, true).eye_distance() / scale;
        let expected = 1.1 / (30.0f32.to_radians().tan() * scale);
        assert!((eye_world - expected).abs() < 1e-5 * expected.max(1.0));
    }
}

/// Overlays behind the eye are dropped rather than drawn stretched across
/// the viewport; the orthographic view has no eye and keeps everything.
#[test]
fn overlay_points_behind_the_eye_are_not_drawn() {
    let scale = 10.0;
    let perspective = camera(scale, true);
    let eye_world = perspective.eye_distance() / scale;
    let in_front = [0.0, -eye_world * 0.5, 0.0];
    let behind = [0.0, -eye_world * 1.5, 0.0];
    assert!(perspective.project(in_front).in_front);
    assert!(!perspective.project(behind).in_front);
    assert!(camera(scale, false).project(behind).in_front);
}

#[test]
fn draw_range_clamps_to_complete_valid_triangles() {
    assert_eq!(model_draw_range(6, 10, 14), Some((24, 6)));
    assert_eq!(model_draw_range(20, 3, 14), None);
}

/// The attribute offsets in `ModelGlRenderer::new` are hand-written against
/// this layout, and nothing else checks them — a reordered or resized field
/// would silently feed the shader the wrong bytes and show as a model that
/// renders but looks wrong.
/// A malformed shader disables the whole preview and reports it only on
/// stderr, so these check the structure no GPU is here to check.
/// The pan and zoom-to-cursor math turns screen deltas back into world
/// moves through `unrotate_view_vector`; if it drifts from the forward
/// rotation, panning smears diagonally and zoom-to-cursor orbits away
/// from the pointer instead of onto it.
#[test]
fn unrotate_inverts_rotate_for_any_view_angles() {
    for (yaw, pitch) in [(0.0, 0.0), (-0.45, 0.25), (1.2, -1.4), (3.0, 0.9)] {
        for vector in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, -2.0, 5.0]] {
            let there = rotate_view_vector(yaw, pitch, vector);
            let back = unrotate_view_vector(yaw, pitch, there);
            for axis in 0..3 {
                assert!(
                    (back[axis] - vector[axis]).abs() < 1e-5,
                    "round trip failed at yaw {yaw} pitch {pitch}: {vector:?} -> {back:?}"
                );
            }
        }
    }
}

/// `both_shader_dialects_declare_what_the_renderer_binds` reads the GLSL
/// as text; a compile error would still
/// only surface at runtime, as a preview that draws nothing. With
/// `glslangValidator` on PATH (`brew install glslang`), compile every
/// dialect egui can hand the renderer; without it, skip by name.
#[test]
fn every_shader_dialect_compiles_under_glslang() {
    if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
        eprintln!("skipping: glslangValidator is not on PATH (brew install glslang)");
        return;
    }
    let dir = crate::test_kits::unique_temp_dir("model_preview_glsl");
    for (index, (declaration, modern, precision)) in [
        ("#version 330\n", true, ""),
        ("#version 140\n", true, ""),
        ("#version 300 es\n", true, "precision mediump float;\n"),
        ("#version 120\n", false, ""),
        ("#version 100\n", false, "precision mediump float;\n"),
    ]
    .into_iter()
    .enumerate()
    {
        let (vertex, fragment) = model_shader_sources(declaration, modern, precision);
        for (stage, source) in [("vert", vertex), ("frag", fragment)] {
            let path = dir.join(format!("model_{index}.{stage}"));
            std::fs::write(&path, source).expect("write shader");
            let output = std::process::Command::new("glslangValidator")
                .arg(&path)
                .output()
                .expect("run glslangValidator");
            assert!(
                output.status.success(),
                "{} {stage} does not compile:\n{}",
                declaration.trim(),
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}

#[test]
fn both_shader_dialects_declare_what_the_renderer_binds() {
    for (declaration, modern, precision) in [
        ("#version 330\n", true, ""),
        ("#version 100\n", false, "precision mediump float;\n"),
    ] {
        let (vertex, fragment) = model_shader_sources(declaration, modern, precision);

        // Every attribute the vertex array points at, and every uniform the
        // renderer looks up, has to actually be declared.
        for name in [
            "a_position",
            "a_normal",
            "a_texcoord",
            "a_tangent",
            "a_binormal",
            "a_node_indices",
            "a_node_weights",
        ] {
            assert!(vertex.contains(name), "{name} missing from vertex shader");
        }
        for name in ["u_animated", "u_bones"] {
            assert!(vertex.contains(name), "{name} missing from vertex shader");
        }
        for name in SAMPLER_UNIFORMS {
            assert!(
                fragment.contains(name),
                "{name} missing from fragment shader"
            );
        }
        for name in [
            "u_have_a",
            "u_have_b",
            "u_uv_scale_a",
            "u_uv_scale_b",
            "u_shaded",
        ] {
            assert!(
                fragment.contains(name),
                "{name} missing from fragment shader"
            );
        }
        assert!(fragment.contains("uniform vec4 u_have_b;"));
        assert!(fragment.contains("uniform vec4 u_uv_scale_b;"));
        // The environment term must stay behind the ambient-strength gate: a
        // shader that asks for none of it has to light exactly as it did
        // before the term existed.

        // A surface with no specular mask must reflect LESS, not more.
        // Having that backwards buried dervish's bare skin — which carries
        // no mask — under a flat wash of environment tint.

        // The detail normal adds to the base one rather than replacing it.
        assert!(
            fragment.contains("tn = vec3(tn.xy + dn, tn.z);"),
            "bump_detail_map should blend into the base normal"
        );
        assert!(
            fragment.contains("u_have_a.z > 0.5 || u_have_a.w > 0.5"),
            "a detail normal with no base bump map should still perturb"
        );
        // The unpack is the engine's own, verbatim from the kit's
        // rasterizer/hlsl/bump_mapping.fx: BUMP_CONVERT on X and Y (byte
        // 128 = exactly flat) for BOTH maps, Z always reconstructed, and
        // NO channel negation — the authored tangent frame carries the
        // convention. `*2-1` unpacks and green flips have both been tried
        // and both bend the lighting; the engine source is the authority.
        assert_eq!(
            fragment
                .matches("* (255.0 / 127.0) - (128.0 / 127.0)")
                .count(),
            2,
            "base and detail bumps must both unpack through BUMP_CONVERT"
        );
        assert!(
            fragment.contains("sqrt(1.0 - min(dot(sampled, sampled), 1.0))"),
            "Z must be reconstructed from X/Y, never sampled"
        );
        assert!(
            !fragment.contains("tn.y = -tn.y"),
            "no green flip: the engine feeds sampled Y straight into the tangent frame"
        );
        for name in [
            "u_center",
            "u_scale",
            "u_angles",
            "u_clip_scale",
            "u_depth_scale",
            "u_depth_map",
            "u_perspective",
        ] {
            assert!(vertex.contains(name), "{name} missing from vertex shader");
        }
        for name in ["u_base_color", "u_unlit"] {
            assert!(
                fragment.contains(name),
                "{name} missing from fragment shader"
            );
        }

        // The varyings must be declared on both sides or the link fails.
        for name in ["v_uv", "v_normal", "v_tangent", "v_binormal"] {
            assert!(
                vertex.contains(name) && fragment.contains(name),
                "{name} not on both sides"
            );
        }

        // Dialect: `texture` vs `texture2D`, and the output keyword pair.
        if modern {
            assert!(fragment.contains("out vec4 out_color;"));
            assert!(fragment.contains("texture(u_tex_base"));
            assert!(vertex.contains("in vec3 a_position"));
        } else {
            assert!(fragment.contains("gl_FragColor"));
            assert!(fragment.contains("texture2D(u_tex_base"));
            assert!(vertex.contains("attribute vec3 a_position"));
        }

        // Only the alpha-test map may discard.
        //
        // A base map's alpha channel carries a mask in Halo — usually
        // specular — not coverage. Discarding on it made dervish, and every
        // other character with a dark diffuse mask, render mostly
        // see-through, while masterchief happened to look fine because his
        // mask is bright. One `discard`, in the alpha-test branch.
        assert_eq!(
            fragment.matches("discard;").count(),
            1,
            "the fragment shader should discard only on the alpha-test map"
        );
        assert!(
            fragment.contains("u_tex_alpha, v_uv * u_uv_scale_c.xy).a < 0.5) discard"),
            "the one discard should be the alpha-test map's"
        );

        for (label, source) in [("vertex", &vertex), ("fragment", &fragment)] {
            assert_eq!(
                source.matches('{').count(),
                source.matches('}').count(),
                "unbalanced braces in the {label} shader — a `format!` escape slipped"
            );
            assert!(
                source.starts_with(declaration),
                "the {label} shader must open with its version declaration"
            );
            assert!(
                !source.contains("{{") && !source.contains("}}"),
                "a `format!` brace escape survived into the {label} source"
            );
        }
    }
}

#[test]
fn gpu_vertex_layout_matches_the_hand_written_attribute_offsets() {
    assert_eq!(std::mem::size_of::<RenderModelPreviewVertex>(), 88);
    let vertex = RenderModelPreviewVertex::default();
    let base = std::ptr::addr_of!(vertex) as usize;
    let offset = |field: usize| field - base;
    assert_eq!(offset(std::ptr::addr_of!(vertex.normal) as usize), 12);
    assert_eq!(offset(std::ptr::addr_of!(vertex.texcoord) as usize), 24);
    assert_eq!(offset(std::ptr::addr_of!(vertex.tangent) as usize), 32);
    assert_eq!(offset(std::ptr::addr_of!(vertex.binormal) as usize), 44);
    assert_eq!(offset(std::ptr::addr_of!(vertex.node_indices) as usize), 56);
    assert_eq!(offset(std::ptr::addr_of!(vertex.node_weights) as usize), 72);
}

#[test]
fn indexed_preview_keeps_shared_vertices_and_part_batches() {
    let vertex = |x, y| RenderVertex {
        position: RealPoint3d { x, y, z: 0.0 },
        texcoord: RealPoint2d { x, y },
        normal: RealVector3d {
            i: 0.0,
            j: 0.0,
            k: 1.0,
        },
        tangent: RealVector3d::ZERO,
        binormal: RealVector3d::ZERO,
        node_indices: [0; 4],
        node_weights: [0.0; 4],
        lightmap_texcoord: RealPoint2d { x: 0.0, y: 0.0 },
        vert_color: RealVector3d::ZERO,
    };
    let part = |index_start| RenderMeshPart {
        material_index: index_start as u16 / 3,
        index_start,
        index_count: 3,
        part_type: GeometryPartType::OpaqueNonShadowing,
        transparent_sorting_index: -1,
        sort_position: None,
    };
    let mesh = RenderMesh {
        vertices: vec![
            vertex(0.0, 0.0),
            vertex(1.0, 0.0),
            vertex(1.0, 1.0),
            vertex(0.0, 1.0),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        parts: vec![part(0), part(3)],
        rigid_node_index: None,
        water_data: None,
        prt_vertex_type: Default::default(),
        has_prt_vertex_stream: false,
        prt_ambient_stream: Vec::new(),
        has_vertex_color: false,
        use_region_index_for_sorting: false,
        has_lightmap_uvs: false,
    };
    let mut preview = RenderModelPreview {
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };

    append_render_mesh_to_preview(&mut preview, &mesh, "body", "default");

    assert_eq!(preview.vertices.len(), 4);
    assert_eq!(preview.indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(preview.batches.len(), 2);
    assert_eq!(
        (
            preview.batches[0].index_start,
            preview.batches[0].index_count
        ),
        (0, 3)
    );
    assert_eq!(
        (
            preview.batches[1].index_start,
            preview.batches[1].index_count
        ),
        (3, 3)
    );
}

#[test]
fn error_face_hover_detects_inside_and_outside_points() {
    let face = [
        egui::pos2(10.0, 10.0),
        egui::pos2(50.0, 10.0),
        egui::pos2(50.0, 40.0),
        egui::pos2(10.0, 40.0),
    ];
    assert!(point_in_polygon(egui::pos2(30.0, 25.0), &face));
    assert!(!point_in_polygon(egui::pos2(60.0, 25.0), &face));
    assert!(polygon_area_twice(&face).abs() > 0.5);

    let edge_on = [
        egui::pos2(10.0, 20.0),
        egui::pos2(30.0, 20.0),
        egui::pos2(50.0, 20.0),
    ];
    assert_eq!(polygon_area_twice(&edge_on), 0.0);
}

#[test]
fn error_visibility_follows_its_owning_model_layer() {
    let mut state = ModelPreviewState::default();
    state.overlays_loaded = true;
    state.show_render = true;
    state.show_collision = false;
    state.show_physics = true;
    assert!(model_layer_visible(&state, ModelPreviewLayer::Render));
    assert!(!model_layer_visible(&state, ModelPreviewLayer::Collision));
    assert!(model_layer_visible(&state, ModelPreviewLayer::Physics));

    state.show_render = false;
    assert!(!model_layer_visible(&state, ModelPreviewLayer::Render));
    assert!(model_layer_visible(&state, ModelPreviewLayer::Physics));
}

#[test]
fn non_critical_errors_are_opt_in() {
    let mut state = ModelPreviewState::default();
    assert!(!state.show_non_critical_errors);
    state.show_non_critical_errors = true;
    assert!(state.show_non_critical_errors);
}

#[test]
fn error_overlay_uses_the_primitives_authored_color() {
    let error = ModelErrorPrimitive {
        label: "open edge".to_owned(),
        non_critical: false,
        color: [12, 34, 56, 78],
        layer: ModelPreviewLayer::Collision,
        shape: ModelErrorShape::Point(ModelErrorPoint {
            position: [0.0; 3],
            node_indices: [-1; 4],
            node_weights: [0.0; 4],
        }),
    };
    assert_eq!(
        model_error_color(&error),
        Color32::from_rgba_unmultiplied(12, 34, 56, 78)
    );
    assert_eq!(
        model_error_fill(&error),
        Color32::from_rgba_unmultiplied(12, 34, 56, 22)
    );
}
