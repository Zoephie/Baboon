use super::super::super::super::Baboon;
use super::super::super::{
    MODEL_ERROR_FALLBACK_COLOR, ModelErrorPoint, ModelErrorPrimitive, ModelErrorShape,
    ModelPreviewLayer, ModelPreviewState, RenderModelPreview, RenderModelPreviewMaterial,
    RenderModelPreviewRegion, TagSource, TextureSlot, resolve_model_textures,
};
use super::super::merge::merge_preview_append;
use super::super::overlays::hlmt_collision_overlay;
use super::super::scenario::{bsp_display_name, rebrand_preview_region};
use super::ass::ass_to_preview;
use super::*;
use crate::app::model_preview::harness::kit::kit_root;
use crate::core::game::GameId;
use blam_tags::geometry::CompressionBounds;
use blam_tags::math::RealRgbColor;
use blam_tags::{AssFile, AssObjectPayload, AssTriangle};
use blam_tags::{
    AssInstance, AssMaterial, AssObject, AssVertex, JmsMaterial, JmsNode, JmsTriangle, JmsVertex,
};

/// An overlay the model names but that will not load says why, where it
/// used to come back as no overlay at all.
#[test]
fn an_overlay_that_will_not_load_says_why() {
    let Some(tags) = kit_root(GameId::Halo3) else {
        return;
    };
    let rel = "objects/vehicles/warthog/warthog.model";
    if !tags.join(rel).is_file() {
        eprintln!("skipping: BLAM_TEST_H3EK has no {rel}");
        return;
    }
    // The model alone, so its collision model has nothing to find.
    let root = std::env::temp_dir().join(format!("baboon-lone-model-{}", std::process::id()));
    std::fs::create_dir_all(root.join("objects/vehicles/warthog")).unwrap();
    std::fs::copy(tags.join(rel), root.join(rel)).unwrap();
    let model = blam_tags::TagFile::read(root.join(rel)).unwrap();
    let source = TagSource::LooseFolder {
        root: root.clone(),
        game: Some(crate::core::game::GameId::Halo3),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let overlay = hlmt_collision_overlay(
        &model,
        &crate::app::model_preview::load::TagReader::new(&source),
    );
    std::fs::remove_dir_all(&root).ok();
    let error = overlay.expect_err("a missing collision model is an error, not no overlay");
    assert!(error.starts_with("Could not load"), "{error}");
}

/// Sidewinder's collision has 54,540 edges, indexed in 16 bits past
/// 32767: every surface still closes its ring of edges into triangles.
#[test]
fn a_collision_bsp_past_32767_edges_keeps_every_surface() {
    let Some(root) = kit_root(GameId::Halo3) else {
        return;
    };
    let path =
        std::path::Path::new(&root).join("levels/dlc/sidewinder/sidewinder.scenario_structure_bsp");
    let tag = blam_tags::TagFile::read(&path).unwrap();
    let block = tag
        .root()
        .field_path("resource interface/raw_resources[0]/raw_items/collision bsp");
    let bsp = block
        .and_then(|field| field.as_block())
        .and_then(|block| block.element(0))
        .unwrap();
    let edges = bsp
        .field_path("edges")
        .and_then(|field| field.as_block())
        .unwrap()
        .len();
    let surfaces = bsp
        .field_path("surfaces")
        .and_then(|field| field.as_block())
        .unwrap()
        .len();
    assert!(edges > 32768, "{edges} edges");
    let mut triples = Vec::new();
    append_collision_bsp_triangles(&bsp, &mut triples);
    assert!(
        triples.len() / 3 >= surfaces,
        "{} triangles for {surfaces} surfaces",
        triples.len() / 3
    );
}

// The derived-preview builders' non-drawing halves: JMS/ASS scenes into
// preview geometry, physics primitives into meshes, and the merge/rename
// plumbing the hlmt overlays and the scenario composite stand on.
//
// Everything here is pure geometry — no tag files, no GL — because that is
// the half that can be wrong quietly: a missed ÷100 draws a BSP a hundred
// times too big, a bad winding turns a hull inside out, and both look like
// "the preview is broken" with no error anywhere.

#[test]
fn overlay_colors_match_their_tag_icons() {
    assert_eq!(COLLISION_COLOR, [0xED, 0x5E, 0xBE]);
    assert_eq!(PHYSICS_COLOR, [0xFF, 0x56, 0x56]);
}

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
        color: None,
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
    append_jms_triangles(
        &mut preview,
        &jms,
        COLLISION_REGION,
        Some(COLLISION_COLOR),
        false,
    );

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

#[test]
fn collision_cells_keep_variant_names_and_remap_bones_by_name() {
    let mut vertices = vec![
        jms_vertex(0.0, 0.0, 0.0),
        jms_vertex(100.0, 0.0, 0.0),
        jms_vertex(0.0, 100.0, 0.0),
    ];
    for vertex in &mut vertices {
        vertex.node_sets = vec![(0, 1.0)];
    }
    let collision_node = JmsNode {
        name: "spine".to_owned(),
        parent: -1,
        rotation: RealQuaternion::IDENTITY,
        translation: RealPoint3d::ZERO,
    };
    let target_skeleton = vec![
        JmsNode {
            name: "pelvis".to_owned(),
            ..collision_node.clone()
        },
        collision_node.clone(),
    ];
    let jms = JmsFile {
        nodes: vec![collision_node],
        // CE JMS keeps its region table for file output as well as carrying
        // the permutation/region cell in the material label. The cell label
        // must win or every triangle becomes the first region + `default`.
        regions: vec!["wrong fallback".to_owned()],
        materials: vec![JmsMaterial {
            name: "metal".to_owned(),
            material_name: "(1) major armor".to_owned(),
        }],
        vertices,
        triangles: vec![JmsTriangle {
            material: 0,
            v: [0, 1, 2],
            region: 0,
        }],
        ..Default::default()
    };
    let mut preview = empty_preview();
    append_collision_jms(&mut preview, &jms, Some(&target_skeleton));

    assert_eq!(preview.batches[0].region_name, "armor");
    assert_eq!(preview.batches[0].permutation_name, "major");
    assert_eq!(preview.batches[0].layer, ModelPreviewLayer::Collision);
    assert_eq!(preview.vertices[0].node_indices[0], 1.0);
    assert_eq!(preview.vertices[0].node_weights[0], 1.0);
}

#[test]
fn collision_error_points_follow_the_same_bind_pose_as_the_mesh() {
    let half_turn = std::f32::consts::FRAC_1_SQRT_2;
    let collision = JmsFile {
        nodes: vec![JmsNode {
            name: "spine".to_owned(),
            parent: -1,
            rotation: RealQuaternion::IDENTITY,
            translation: RealPoint3d::ZERO,
        }],
        ..Default::default()
    };
    let skeleton = vec![
        JmsNode {
            name: "pelvis".to_owned(),
            parent: -1,
            rotation: RealQuaternion::IDENTITY,
            translation: RealPoint3d::ZERO,
        },
        JmsNode {
            name: "spine".to_owned(),
            parent: 0,
            rotation: RealQuaternion {
                i: half_turn,
                j: 0.0,
                k: 0.0,
                w: half_turn,
            },
            translation: RealPoint3d {
                x: 100.0,
                y: 200.0,
                z: 300.0,
            },
        },
    ];
    let point = ModelErrorPoint {
        position: [0.0, 1.0, 0.0],
        node_indices: [0, -1, -1, -1],
        node_weights: [1.0, 0.0, 0.0, 0.0],
    };
    let mut errors = vec![ModelErrorPrimitive {
        label: "open edge".to_owned(),
        non_critical: false,
        color: MODEL_ERROR_FALLBACK_COLOR,
        layer: ModelPreviewLayer::Collision,
        shape: ModelErrorShape::Vector {
            point,
            normal: [0.0, 1.0, 0.0],
            length: 1.0,
        },
    }];

    pose_collision_errors(&mut errors, &collision, &skeleton);

    let ModelErrorShape::Vector { point, normal, .. } = &errors[0].shape else {
        panic!("expected vector error");
    };
    assert_eq!(point.node_indices[0], 1, "bone remapped by name");
    assert!((point.position[0] - 1.0).abs() < 0.001);
    assert!((point.position[1] - 2.0).abs() < 0.001);
    assert!((point.position[2] - 4.0).abs() < 0.001);
    assert!(normal[1].abs() < 0.001 && (normal[2] - 1.0).abs() < 0.001);
}

#[test]
fn physics_shapes_are_placed_and_weighted_by_their_parent_bone() {
    let source_node = JmsNode {
        name: "spine".to_owned(),
        parent: -1,
        rotation: RealQuaternion::IDENTITY,
        translation: RealPoint3d {
            x: 100.0,
            y: 0.0,
            z: 0.0,
        },
    };
    let target_skeleton = vec![
        JmsNode {
            name: "pelvis".to_owned(),
            ..source_node.clone()
        },
        source_node.clone(),
    ];
    let jms = JmsFile {
        nodes: vec![source_node],
        ..Default::default()
    };
    let mut vertices = Vec::new();
    append_physics_shape(
        &mut vertices,
        &[([0.25, 0.0, 0.0], [0.0, 0.0, 1.0])],
        0,
        &jms,
        Some(&target_skeleton),
    );

    assert_eq!(vertices[0].position, [1.25, 0.0, 0.0]);
    assert_eq!(vertices[0].node_indices[0], 1.0);
    assert_eq!(vertices[0].node_weights[0], 1.0);
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
        assert!(
            (d - 0.5).abs() < 0.01,
            "vertex off the sphere: {position:?}"
        );
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
    assert!(
        portal_batch.flat_color.is_some(),
        "layers keep fixed colors"
    );
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

    // A region both have keeps the render model's permutations: the
    // overlay's would be choices that show nothing of the render model.
    dst.regions[0].permutations = vec!["default".to_owned()];
    let mut overlay = empty_preview();
    overlay.regions = vec![RenderModelPreviewRegion {
        name: dst.regions[0].name.clone(),
        permutations: vec!["damaged".to_owned()],
    }];
    merge_preview_append(&mut dst, &overlay);
    assert_eq!(dst.regions[0].permutations, ["default"]);
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

/// The scenario BSP selection invalidates the cached preview; the overlay
/// toggles must NOT — they are draw-time filters over geometry a worker
/// merged in once, and re-parsing collision and physics tags on the UI
/// thread for every tick is exactly the freeze this design removed.
#[test]
fn overlay_toggles_never_invalidate_but_the_bsp_selection_does() {
    let mut state = ModelPreviewState {
        loaded_key: Some("file:a.model".to_owned()),
        data: Some(Err("placeholder".to_owned())),
        ..Default::default()
    };
    state.campaign_evolved.loaded_high_detail = state.campaign_evolved.high_detail;
    assert!(!state.needs_preview_load("file:a.model"));

    state.show_collision = true;
    state.show_physics = true;
    state.show_render = false;
    assert!(
        !state.needs_preview_load("file:a.model"),
        "overlay toggles are frame-level filters, never rebuilds"
    );

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
    let collect = |extension: &str, take: usize| -> Vec<std::path::PathBuf> {
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

    for path in collect("collision_model", 10) {
        let Ok(tag) =
            crate::core::source::read_tag_at_path(&path, None, None, u32::from_be_bytes(*b"coll"))
        else {
            continue;
        };
        if let Ok(preview) = build_collision_preview(&tag, None) {
            assert!(!preview.vertices.is_empty(), "{}", path.display());
            built += 1;
        }
    }
    for path in collect("physics_model", 10) {
        let Ok(tag) =
            crate::core::source::read_tag_at_path(&path, None, None, u32::from_be_bytes(*b"phmo"))
        else {
            continue;
        };
        if let Ok(preview) = build_physics_preview(&tag, None) {
            assert!(!preview.vertices.is_empty(), "{}", path.display());
            built += 1;
        }
    }
    for path in collect("scenario_structure_bsp", 2) {
        let Ok(tag) =
            crate::core::source::read_tag_at_path(&path, None, None, u32::from_be_bytes(*b"sbsp"))
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
    assert!(
        built > 0,
        "nothing under {} built a preview",
        tags_root.display()
    );
    eprintln!("built {built} derived previews from the real kit");
}

// The textured BSP contract against a real kit: the native H3 decode must
// keep UVs, tangent frames, and the materials block's shader paths — the
// three things the ASS text path lost, and the three things diffuse, normal
// mapping, and alpha-test each depend on.

/// Point `BABOON_MODEL_KIT` at an H3-family kit's `tags` folder to run this
/// against real BSPs; absent, it self-skips like the other fixture tests.
///
/// Several candidates rather than the first found: a working kit accumulates
/// converted and experimental BSPs whose materials are legitimately empty, and
/// the contract only claims that a *shipped* BSP keeps its shading inputs — so
/// one candidate passing the full chain is the assertion.
#[test]
fn a_real_bsps_render_layer_keeps_uvs_tangents_and_shader_paths() {
    let Some(tags_root) = std::env::var_os("BABOON_MODEL_KIT").map(std::path::PathBuf::from) else {
        eprintln!("skipping: set BABOON_MODEL_KIT to an editing kit's tags folder");
        return;
    };
    let candidates: Vec<std::path::PathBuf> = walkdir::WalkDir::new(&tags_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|found| {
            found.file_type().is_file()
                && found.path().extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("scenario_structure_bsp")
                })
        })
        .map(|found| found.path().to_path_buf())
        .take(8)
        .collect();
    if candidates.is_empty() {
        eprintln!(
            "skipping: no .scenario_structure_bsp under {}",
            tags_root.display()
        );
        return;
    }

    let source = TagSource::LooseFolder {
        root: tags_root.clone(),
        game: Some(GameId::Halo3),
        definitions_root: std::path::PathBuf::new(),
    };
    for path in &candidates {
        let Ok(tag) =
            crate::core::source::read_tag_at_path(path, None, None, u32::from_be_bytes(*b"sbsp"))
        else {
            continue;
        };
        let Ok(preview) = build_sbsp_preview(&tag, false) else {
            continue;
        };
        let with_shader = preview
            .materials
            .iter()
            .filter(|material| !material.shader_path.is_empty())
            .count();
        eprintln!(
            "{}: {} vertices, {} batches, {with_shader}/{} materials with shaders",
            path.display(),
            preview.vertices.len(),
            preview.batches.len(),
            preview.materials.len(),
        );
        if with_shader == 0 {
            continue;
        }

        // UVs must not all be zero, or every texture would smear one texel.
        let nonzero_uv = preview
            .vertices
            .iter()
            .filter(|vertex| vertex.texcoord[0].abs() > 0.001 || vertex.texcoord[1].abs() > 0.001)
            .count();
        assert!(
            nonzero_uv > preview.vertices.len() / 4,
            "{}: UVs did not survive the decode",
            path.display()
        );
        // Tangent frames must survive for normal mapping to perturb.
        let with_tangent = preview
            .vertices
            .iter()
            .filter(|vertex| {
                let t = vertex.tangent;
                (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt() > 0.5
            })
            .count();
        assert!(
            with_tangent > preview.vertices.len() / 4,
            "{}: tangent frames did not survive the decode",
            path.display()
        );

        let sample: Vec<RenderModelPreviewMaterial> =
            preview.materials.iter().take(12).cloned().collect();
        let resolved = resolve_model_textures(&source, &sample, true);
        let base = resolved
            .iter()
            .filter(|material| material.get(TextureSlot::Base).is_some())
            .count();
        let bump = resolved
            .iter()
            .filter(|material| material.get(TextureSlot::Bump).is_some())
            .count();
        eprintln!(
            "  of {} sampled materials: {base} diffuse, {bump} normal maps resolved",
            sample.len()
        );
        assert!(base > 0, "{}: no diffuse maps resolved", path.display());
        assert!(bump > 0, "{}: no normal maps resolved", path.display());
        return;
    }
    panic!(
        "none of {} candidate BSPs under {} carried shader materials",
        candidates.len(),
        tags_root.display()
    );
}

/// Collision and physics overlays land while the model's textures are
/// still resolving. The merge appends flat-coloured materials after the
/// model's own, so the resolve in flight is still the right answer and
/// has to be kept, not dropped and run again.
#[test]
fn an_overlay_merge_keeps_the_texture_resolve_in_flight() {
    let mut app = Baboon::for_test();
    let stamp = app.model.kit_stamp();
    let key = "file:a.model".to_owned();
    let preview = RenderModelPreview {
        materials: vec![Default::default()],
        ..Default::default()
    };
    let data =
        super::super::super::model_preview_data(key.clone(), key.clone(), preview, Vec::new());
    let (geometry_id, textures_id) = (data.geometry_id, data.textures_id);
    let state = app.views[app.model.kits[0].id]
        .caches
        .model_previews
        .entry(key.clone())
        .or_default();
    state.data = Some(Ok(data));
    state.textures_pending = true;

    let overlay = RenderModelPreview {
        materials: vec![Default::default()],
        ..Default::default()
    };
    app.handle_model_overlays_built(
        stamp,
        key.clone(),
        geometry_id,
        Some(overlay),
        None,
        Vec::new(),
    );
    let state = &app.views[app.model.kits[0].id].caches.model_previews[&key];
    assert!(
        state.textures_pending,
        "the resolve in flight is still awaited"
    );
    let Some(Ok(data)) = state.data.as_ref() else {
        panic!("preview data");
    };
    assert_ne!(data.mesh_id, geometry_id, "the mesh did change");
    assert_eq!(
        data.geometry_id, geometry_id,
        "the effects and inputs keep theirs"
    );

    app.handle_model_textures_resolved(stamp, key.clone(), textures_id, false, vec![Default::default()]);
    let Some(Ok(data)) = app.views[app.model.kits[0].id].caches.model_previews[&key]
        .data
        .as_ref()
    else {
        panic!("preview data");
    };
    assert!(data.textures.is_some(), "and its result is kept");
}

/// A unit's weapons land in the preview whose picker asked for them, and the
/// picker stops waiting; ones listed for an earlier load are dropped, which
/// still stops it waiting, so it can ask again.
#[test]
fn listed_weapons_land_in_the_load_that_asked_for_them() {
    use crate::app::model_preview::weapons::{ListedWeapons, UnitWeapons};
    let mut app = Baboon::for_test();
    let stamp = app.model.kit_stamp();
    let key = "file:a.biped".to_owned();
    let mut data = super::super::super::model_preview_data(
        key.clone(),
        key.clone(),
        RenderModelPreview::default(),
        Vec::new(),
    );
    data.weapons = Some(UnitWeapons::default());
    let geometry_id = data.geometry_id;
    let state = app.views[app.model.kits[0].id]
        .caches
        .model_previews
        .entry(key.clone())
        .or_default();
    state.data = Some(Ok(data));
    let listed = |app: &Baboon| {
        let state = &app.views[app.model.kits[0].id].caches.model_previews[&key];
        let Some(Ok(data)) = state.data.as_ref() else {
            panic!("preview data");
        };
        let weapons = data.weapons.as_ref().expect("the unit's weapons");
        (state.weapons_pending, weapons.equippable.as_ref().map(Vec::len))
    };
    let pending = |app: &mut Baboon| {
        let id = app.model.kits[0].id;
        app.views[id].caches.model_previews.get_mut(&key).unwrap().weapons_pending = true;
    };

    pending(&mut app);
    app.handle_unit_weapons_listed(
        stamp,
        key.clone(),
        geometry_id + 1,
        Some(ListedWeapons::Unit(Vec::new())),
    );
    assert_eq!(listed(&app), (false, None), "another load's list is dropped");

    pending(&mut app);
    app.handle_unit_weapons_listed(stamp, key.clone(), geometry_id, Some(ListedWeapons::Unit(Vec::new())));
    assert_eq!(listed(&app), (false, Some(0)), "its own lands");
}
