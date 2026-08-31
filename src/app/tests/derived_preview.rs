//! The derived-preview builders' non-drawing halves: JMS/ASS scenes into
//! preview geometry, physics primitives into meshes, and the merge/rename
//! plumbing the hlmt overlays and the scenario composite stand on.
//!
//! Everything here is pure geometry — no tag files, no GL — because that is
//! the half that can be wrong quietly: a missed ÷100 draws a BSP a hundred
//! times too big, a bad winding turns a hull inside out, and both look like
//! "the preview is broken" with no error anywhere.

use super::*;
use blam_tags::math::RealRgbColor;
use blam_tags::{AssInstance, AssMaterial, AssObject, AssVertex, JmsTriangle, JmsVertex};

fn jms_vertex(x: f32, y: f32, z: f32) -> JmsVertex {
    JmsVertex {
        position: RealPoint3d { x, y, z },
        normal: RealVector3d {
            i: 0.0,
            j: 0.0,
            k: 1.0,
        },
        tangent: None,
        binormal: None,
        node_sets: Vec::new(),
        uvs: Vec::new(),
    }
}

/// A JMS mesh lands ÷100 in world units, in its named region, with the flat
/// overlay color — and with a *recomputed* face normal, because the collision
/// JMS writer emits a constant `(0,0,1)` that would shade every wall flat.
#[test]
fn a_jms_collision_mesh_scales_down_and_recomputes_its_normals() {
    let jms = JmsFile {
        // A wall in the XZ plane: its face normal is ±Y, nothing like the
        // constant +Z the vertices claim.
        vertices: vec![
            jms_vertex(0.0, 0.0, 0.0),
            jms_vertex(100.0, 0.0, 0.0),
            jms_vertex(0.0, 0.0, 100.0),
        ],
        triangles: vec![JmsTriangle {
            material: 0,
            v: [0, 1, 2],
            region: 0,
        }],
        ..Default::default()
    };
    let mut preview = empty_preview();
    append_jms_triangles(&mut preview, &jms, COLLISION_REGION, Some(COLLISION_COLOR), false);

    assert_eq!(preview.vertices.len(), 3);
    assert_eq!(preview.vertices[1].position, [1.0, 0.0, 0.0], "÷100");
    let normal = preview.vertices[0].normal;
    assert!(
        normal[1].abs() > 0.99 && normal[0].abs() < 0.01 && normal[2].abs() < 0.01,
        "the face normal must be recomputed, not the writer's constant +Z: {normal:?}"
    );
    assert_eq!(preview.batches.len(), 1);
    assert_eq!(preview.batches[0].region_name, COLLISION_REGION);
    assert_eq!(preview.batches[0].flat_color, Some(COLLISION_COLOR));
    assert_eq!(preview.regions.len(), 1, "the region doubles as the toggle");
}

/// Authored normals survive when asked for — H1 BSP render geometry carries
/// real ones worth keeping.
#[test]
fn authored_normals_are_kept_when_the_source_has_real_ones() {
    let jms = JmsFile {
        vertices: vec![
            jms_vertex(0.0, 0.0, 0.0),
            jms_vertex(100.0, 0.0, 0.0),
            jms_vertex(0.0, 0.0, 100.0),
        ],
        triangles: vec![JmsTriangle {
            material: 0,
            v: [0, 1, 2],
            region: 0,
        }],
        ..Default::default()
    };
    let mut preview = empty_preview();
    append_jms_triangles(&mut preview, &jms, "render", None, true);
    assert_eq!(preview.vertices[0].normal, [0.0, 0.0, 1.0]);
    assert_eq!(preview.batches[0].flat_color, None);
}

#[test]
fn a_sphere_tessellates_onto_its_own_surface() {
    let mut triples = Vec::new();
    push_sphere(
        &mut triples,
        &RealQuaternion::IDENTITY,
        [100.0, 0.0, 0.0],
        50.0,
    );
    assert!(!triples.is_empty());
    for (position, normal) in &triples {
        // Centered at 1.0 world units, radius 0.5.
        let d = ((position[0] - 1.0).powi(2) + position[1].powi(2) + position[2].powi(2)).sqrt();
        assert!((d - 0.5).abs() < 0.01, "vertex off the sphere: {position:?}");
        let len = (normal[0].powi(2) + normal[1].powi(2) + normal[2].powi(2)).sqrt();
        assert!((len - 1.0).abs() < 0.01, "non-unit normal");
    }
}

#[test]
fn a_box_tessellates_to_its_full_extents() {
    let shape = blam_tags::JmsBox {
        name: String::new(),
        parent: -1,
        material: 0,
        rotation: RealQuaternion::IDENTITY,
        translation: RealPoint3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        width: 200.0,
        length: 100.0,
        height: 50.0,
    };
    let mut triples = Vec::new();
    push_box(&mut triples, &shape);
    assert_eq!(triples.len(), 36, "6 faces × 2 triangles × 3 vertices");
    let max = |axis: usize| {
        triples
            .iter()
            .map(|(p, _)| p[axis].abs())
            .fold(0.0f32, f32::max)
    };
    // Full extents 200/100/50 cm → half extents 1.0/0.5/0.25 world units.
    assert!((max(0) - 1.0).abs() < 1e-4);
    assert!((max(1) - 0.5).abs() < 1e-4);
    assert!((max(2) - 0.25).abs() < 1e-4);
}

#[test]
fn a_capsule_spans_bottom_cap_to_top_cap() {
    let capsule = blam_tags::JmsCapsule {
        name: String::new(),
        parent: -1,
        material: 0,
        rotation: RealQuaternion::IDENTITY,
        translation: RealPoint3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        height: 100.0,
        radius: 25.0,
    };
    let mut triples = Vec::new();
    push_capsule(&mut triples, &capsule);
    assert!(!triples.is_empty());
    let (mut min_z, mut max_z) = (f32::INFINITY, f32::NEG_INFINITY);
    for (position, _) in &triples {
        min_z = min_z.min(position[2]);
        max_z = max_z.max(position[2]);
    }
    // Anchored at the bottom-cap center: caps extend a radius past each end.
    assert!((min_z - -0.25).abs() < 0.01, "bottom cap at {min_z}");
    assert!((max_z - 1.25).abs() < 0.01, "top cap at {max_z}");
}

/// Every hull face must point away from the shape's centroid — an inside-out
/// hull culls to nothing and reads as "physics preview is empty".
#[test]
fn a_convex_hull_winds_every_face_outward() {
    let corners = [
        [-100.0, -100.0, -100.0],
        [100.0, -100.0, -100.0],
        [-100.0, 100.0, -100.0],
        [100.0, 100.0, -100.0],
        [-100.0, -100.0, 100.0],
        [100.0, -100.0, 100.0],
        [-100.0, 100.0, 100.0],
        [100.0, 100.0, 100.0],
    ];
    let convex = blam_tags::JmsConvex {
        name: String::new(),
        parent: -1,
        material: 0,
        rotation: RealQuaternion::IDENTITY,
        translation: RealPoint3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        vertices: corners
            .iter()
            .map(|[x, y, z]| RealPoint3d {
                x: *x,
                y: *y,
                z: *z,
            })
            .collect(),
    };
    let mut triples = Vec::new();
    push_convex(&mut triples, &convex);
    assert!(!triples.is_empty(), "a cube's corners must produce a hull");
    for face in triples.chunks_exact(3) {
        let (a, b, c) = (face[0].0, face[1].0, face[2].0);
        let normal = face[0].1;
        let center = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        // The hull is centered on the origin, so outward == away from origin.
        let outward = center[0] * normal[0] + center[1] * normal[1] + center[2] * normal[2];
        assert!(outward > 0.0, "face wound inward: {center:?} {normal:?}");
    }
}

fn ass_vertex(x: f32, y: f32, z: f32) -> AssVertex {
    AssVertex {
        position: RealPoint3d { x, y, z },
        normal: RealVector3d {
            i: 0.0,
            j: 0.0,
            k: 1.0,
        },
        color: RealRgbColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
        },
        node_set: Vec::new(),
        uvs: Vec::new(),
    }
}

fn ass_mesh(material: i32) -> AssObject {
    AssObject {
        xref_filepath: String::new(),
        xref_objectname: String::new(),
        payload: AssObjectPayload::Mesh {
            vertices: vec![
                ass_vertex(0.0, 0.0, 0.0),
                ass_vertex(100.0, 0.0, 0.0),
                ass_vertex(0.0, 100.0, 0.0),
            ],
            triangles: vec![AssTriangle {
                material,
                v: [0, 1, 2],
            }],
        },
    }
}

fn ass_material(name: &str) -> AssMaterial {
    AssMaterial {
        name: name.to_owned(),
        lightmap_variant: String::new(),
        bm_strings: Vec::new(),
    }
}

/// The special ASS layers land in their own regions with their own colors;
/// ordinary shaders land in `render`, scaled and placed by their instance.
#[test]
fn ass_scenes_split_into_layer_regions_and_apply_instance_transforms() {
    let ass = AssFile {
        materials: vec![ass_material("some_shader"), ass_material("+portal")],
        objects: vec![ass_mesh(0), ass_mesh(1)],
        instances: vec![
            AssInstance {
                object_index: 0,
                local_translation: RealPoint3d {
                    x: 100.0,
                    y: 0.0,
                    z: 0.0,
                },
                ..Default::default()
            },
            AssInstance {
                object_index: 1,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let preview = ass_to_preview(&ass, false);

    let regions: Vec<&str> = preview
        .regions
        .iter()
        .map(|region| region.name.as_str())
        .collect();
    assert_eq!(regions, ["render", "portals"]);
    // The render instance sits 100 cm along X: its first vertex lands ÷100 at
    // exactly one world unit.
    assert_eq!(preview.vertices[0].position, [1.0, 0.0, 0.0]);
    let portal_batch = preview
        .batches
        .iter()
        .find(|batch| batch.region_name == "portals")
        .expect("portal layer batch");
    assert!(portal_batch.flat_color.is_some(), "layers keep fixed colors");
    let render_batch = preview
        .batches
        .iter()
        .find(|batch| batch.region_name == "render")
        .expect("render batch");
    assert_eq!(render_batch.flat_color, None, "shaders use the palette");

    // The scenario composite asks for render only.
    let render_only = ass_to_preview(&ass, true);
    assert!(
        render_only
            .batches
            .iter()
            .all(|batch| batch.region_name == "render"),
        "render_only must drop the marker layers"
    );
}

/// The @collision_only layer is what the standalone sbsp preview shows as its
/// collision toggle.
#[test]
fn the_ass_collision_layer_becomes_the_collision_region() {
    let ass = AssFile {
        materials: vec![ass_material("@collision_only")],
        objects: vec![ass_mesh(0)],
        instances: vec![AssInstance::default()],
        ..Default::default()
    };
    let preview = ass_to_preview(&ass, false);
    assert_eq!(preview.regions.len(), 1);
    assert_eq!(preview.regions[0].name, COLLISION_REGION);
}

/// Merging offsets vertices, indices, and material references — an overlay
/// whose indices still pointed at its own vertex 0 would draw garbage out of
/// the render model's buffer.
#[test]
fn merging_an_overlay_offsets_every_reference() {
    let mut dst = empty_preview();
    append_jms_triangles(
        &mut dst,
        &JmsFile {
            vertices: vec![
                jms_vertex(0.0, 0.0, 0.0),
                jms_vertex(100.0, 0.0, 0.0),
                jms_vertex(0.0, 100.0, 0.0),
            ],
            triangles: vec![JmsTriangle {
                material: 0,
                v: [0, 1, 2],
                region: 0,
            }],
            ..Default::default()
        },
        "render",
        None,
        false,
    );
    let mut src = empty_preview();
    append_jms_triangles(
        &mut src,
        &JmsFile {
            vertices: vec![
                jms_vertex(0.0, 0.0, 200.0),
                jms_vertex(100.0, 0.0, 200.0),
                jms_vertex(0.0, 100.0, 200.0),
            ],
            triangles: vec![JmsTriangle {
                material: 0,
                v: [0, 1, 2],
                region: 0,
            }],
            ..Default::default()
        },
        COLLISION_REGION,
        Some(COLLISION_COLOR),
        false,
    );

    merge_preview_append(&mut dst, &src);

    assert_eq!(dst.vertices.len(), 6);
    assert_eq!(dst.batches.len(), 2);
    let merged = &dst.batches[1];
    assert_eq!(merged.region_name, COLLISION_REGION);
    assert_eq!(merged.material_index, 1, "materials must offset");
    let first_index = dst.indices[merged.index_start as usize];
    assert_eq!(first_index, 3, "indices must offset past dst's vertices");
    assert_eq!(
        dst.regions.len(),
        2,
        "the overlay's region joins the region list as its toggle"
    );
    assert!(dst.bounds_max[2] >= 2.0 - 1e-4, "bounds must expand");
}

#[test]
fn rebranding_collapses_a_bsp_into_one_toggle() {
    let ass = AssFile {
        materials: vec![ass_material("shader"), ass_material("+portal")],
        objects: vec![ass_mesh(0), ass_mesh(1)],
        instances: vec![
            AssInstance {
                object_index: 0,
                ..Default::default()
            },
            AssInstance {
                object_index: 1,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut preview = ass_to_preview(&ass, false);
    rebrand_preview_region(&mut preview, "010_jungle");
    assert_eq!(preview.regions.len(), 1);
    assert_eq!(preview.regions[0].name, "010_jungle");
    assert!(
        preview
            .batches
            .iter()
            .all(|batch| batch.region_name == "010_jungle")
    );
}

/// Instanced geometry lands where its basis puts it — a swapped basis or a
/// missed scale scatters a level's crates and railings into the wrong rooms.
#[test]
fn an_instance_placement_applies_basis_scale_and_position() {
    let placement = InstancePlacement {
        // A quarter-turn: local X maps to world Y, local Y to world -X.
        forward: [0.0, 1.0, 0.0],
        left: [-1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        position: [10.0, 20.0, 30.0],
        scale: 2.0,
    };
    assert_eq!(placement.apply([1.0, 0.0, 0.0]), [10.0, 22.0, 30.0]);
    assert_eq!(placement.apply([0.0, 1.0, 0.0]), [8.0, 20.0, 30.0]);
    // Normals rotate but never scale or translate.
    assert_eq!(placement.rotate([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);

    // A zero or garbage scale falls back to 1 rather than collapsing the
    // instance into a point.
    let degenerate = InstancePlacement {
        scale: 0.0,
        ..InstancePlacement::identity()
    };
    assert_eq!(degenerate.apply([3.0, 0.0, 0.0]), [3.0, 0.0, 0.0]);
}

/// Winding flips only when an ODD number of compression axes mirror.
#[test]
fn compression_mirroring_flips_winding_only_on_odd_axes() {
    let mut bounds = CompressionBounds::identity();
    assert!(!bounds_axis_flip(&bounds), "identity never flips");

    bounds.pos_compressed = true;
    assert!(!bounds_axis_flip(&bounds), "no mirrored axis");
    (bounds.px_min, bounds.px_max) = (1.0, -1.0);
    assert!(bounds_axis_flip(&bounds), "one mirrored axis flips");
    (bounds.py_min, bounds.py_max) = (1.0, -1.0);
    assert!(!bounds_axis_flip(&bounds), "two mirrors cancel");
}

#[test]
fn a_bsp_toggle_shows_the_reference_leaf() {
    assert_eq!(
        bsp_display_name("levels\\solo\\010_jungle\\010_jungle"),
        "010_jungle"
    );
    assert_eq!(bsp_display_name("010_jungle"), "010_jungle");
}

/// Flipping an overlay toggle or the BSP selection must invalidate the cached
/// preview exactly like the CE high-detail toggle does — the panel's spinner
/// gate and the loader's early return read the same predicate.
#[test]
fn overlay_and_selection_changes_invalidate_the_cached_preview() {
    let mut state = ModelPreviewState::default();
    state.loaded_key = Some("file:a.model".to_owned());
    state.data = Some(Err("placeholder".to_owned()));
    state.loaded_high_detail = state.high_detail;
    assert!(!state.needs_preview_load("file:a.model"));

    state.show_collision = true;
    assert!(state.needs_preview_load("file:a.model"));
    state.loaded_show_collision = true;
    assert!(!state.needs_preview_load("file:a.model"));

    state.scenario_bsp_selection.insert(2);
    assert!(state.needs_preview_load("file:a.model"));
    state.loaded_scenario_selection.insert(2);
    assert!(!state.needs_preview_load("file:a.model"));

    assert!(state.needs_preview_load("file:b.model"), "a different tag");
}

/// Point this at an editing kit's `tags` folder to run the derived builders
/// against real collision, physics, and BSP tags. Absent, this self-skips.
const KIT_TAGS_ENV: &str = "BABOON_MODEL_KIT";

#[test]
fn real_kit_collision_physics_and_bsp_tags_build_previews() {
    let Some(tags_root) = std::env::var_os(KIT_TAGS_ENV).map(std::path::PathBuf::from) else {
        eprintln!("skipping: set {KIT_TAGS_ENV} to an editing kit's tags folder");
        return;
    };
    if !tags_root.is_dir() {
        eprintln!("skipping: {} is not a folder", tags_root.display());
        return;
    }

    let mut built = 0usize;
    let collect = |extension: &str, group: &[u8; 4], take: usize| -> Vec<std::path::PathBuf> {
        let _ = group;
        walkdir::WalkDir::new(&tags_root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|found| {
                found.file_type().is_file()
                    && found
                        .path()
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
            })
            .map(|found| found.path().to_path_buf())
            .take(take)
            .collect()
    };

    for path in collect("collision_model", b"coll", 10) {
        let Ok(tag) =
            crate::source::read_tag_at_path(&path, None, None, u32::from_be_bytes(*b"coll"))
        else {
            continue;
        };
        if let Ok(preview) = build_collision_preview(&tag, None) {
            assert!(!preview.vertices.is_empty(), "{}", path.display());
            built += 1;
        }
    }
    for path in collect("physics_model", b"phmo", 10) {
        let Ok(tag) =
            crate::source::read_tag_at_path(&path, None, None, u32::from_be_bytes(*b"phmo"))
        else {
            continue;
        };
        if let Ok(preview) = build_physics_preview(&tag, None) {
            assert!(!preview.vertices.is_empty(), "{}", path.display());
            built += 1;
        }
    }
    for path in collect("scenario_structure_bsp", b"sbsp", 2) {
        let Ok(tag) =
            crate::source::read_tag_at_path(&path, None, None, u32::from_be_bytes(*b"sbsp"))
        else {
            continue;
        };
        if let Ok(preview) = build_sbsp_preview(&tag, false) {
            assert!(!preview.vertices.is_empty(), "{}", path.display());
            assert!(
                preview.regions.iter().any(|region| region.name == "render"),
                "{} produced no render layer",
                path.display()
            );
            built += 1;
        }
    }
    assert!(built > 0, "nothing under {} built a preview", tags_root.display());
    eprintln!("built {built} derived previews from the real kit");
}
