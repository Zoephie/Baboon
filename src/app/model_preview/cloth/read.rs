//! Reading cloth tags and the cloth on an object.

use super::*;

/// A `cloth` on an object, and where its mesh lies in the model's vertices.
#[derive(Debug, Clone)]
pub(crate) struct H3ClothAt {
    pub(super) places: Vec<[f32; 3]>,
    pub(super) indices: Vec<usize>,
    pub(super) links: Vec<ClothLink>,
    pub(super) pins: Vec<ClothPin>,
    pub(super) spheres: Vec<ClothSphere>,
    pub(super) iterations: i32,
    pub(super) gravity: f32,
    pub(super) drag: f32,
    pub(super) wind_scale: f32,
    pub(super) flappiness: f32,
    /// The sheet's two sides in the model's vertices, each `places` long.
    pub(super) first_vertex: usize,
    pub(super) game: ClothGame,
    /// Reach's: the middle of the object's bounding sphere, and the node it
    /// is posed on, which `cloth_enforce_attachments` follows.
    pub(super) center: (usize, [f32; 3]),
}

impl H3ClothAt {
    pub(crate) fn halo2(&self) -> bool {
        self.game == ClothGame::Halo2
    }

    #[cfg(test)]
    pub(crate) fn game(&self) -> ClothGame {
        self.game
    }

    /// Each pin's vertex, in the order the markers sort.
    #[cfg(test)]
    pub(crate) fn pinned_vertices(&self) -> Vec<usize> {
        self.pins.iter().map(|pin| pin.vertex).collect()
    }
}

fn cloth_tags(object: &TagFile) -> Vec<String> {
    let group = u32::from_be_bytes(*b"clwd");
    let mut paths = Vec::new();
    let widgets =
        object_block(object, "widgets").map_or(Vec::new(), |holder| elements(holder, "widgets"));
    let attachments = object_block(object, "attachments")
        .map_or(Vec::new(), |holder| elements(holder, "attachments"));
    for entry in widgets.iter().chain(&attachments) {
        if let Some((found, path)) = entry.read_tag_ref_with_group("type")
            && found == group
            && !path.is_empty()
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    paths
}

fn read_cloth_tag(
    root: TagStruct<'_>,
    markers: &[RenderModelPreviewMarker],
    frame: &impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
    game: ClothGame,
    center: [f32; 3],
) -> Option<(H3ClothAt, Vec<[f32; 2]>, Option<(String, String)>)> {
    let clamps = game.rules().clamps_properties;
    // The ranges are `tool.exe`'s postprocess, which tags on disk have not been through.
    let int = |name: &str| root.read_int_any(name).unwrap_or(0).clamp(2, 32) as usize;
    let size = [int("grid x dimension"), int("grid y dimension")];
    let spacing = [
        root.read_real("grid spacing x")?,
        root.read_real("grid spacing y")?,
    ];
    let (places, uvs, indices) = flag_grid(size, spacing, game);
    if places.is_empty() {
        return None;
    }
    let properties = root.descend("properties")?;
    // Halo 2's tool limits none of these.
    let real = |name: &str, low: f32, high: f32| {
        let value = properties.read_real(name).unwrap_or(0.0);
        if clamps {
            value.clamp(low, high)
        } else {
            value
        }
    };
    // The bounding sphere's middle, carried as a marker of the object's own.
    let middle = RenderModelPreviewMarker {
        position: center,
        scale: 1.0,
        ..Default::default()
    };
    let (middle, middle_node) = frame(&middle);
    let placed = |name: &str| -> Vec<(usize, [f32; 3])> {
        let named = markers
            .iter()
            .filter(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(name));
        named
            .map(|marker| {
                let (place, node) = frame(marker);
                (node, place.position)
            })
            .collect()
    };
    let attach_name = marker_name(root, "marker attachment name");
    let attach = placed(&attach_name);
    if attach.is_empty() {
        return None;
    }
    // The game finds markers only on shown permutations, so a sheet hung on
    // one goes with it.
    let under = markers
        .iter()
        .find(|marker| marker.name.eq_ignore_ascii_case(&attach_name))
        .and_then(|marker| marker.only_under.clone());
    let secondary = placed(&marker_name(root, "secondary axis attachment name"))
        .first()
        .copied();
    let high = places.len() / size[0].max(1);
    let spheres = elements(root, "collision spheres")
        .into_iter()
        .take(MOST_SPHERES)
        .filter_map(|sphere| {
            let (node, position) = *placed(&marker_name(sphere, "object marker name")).first()?;
            Some(ClothSphere {
                node,
                position,
                radius: sphere.read_real("radius")?,
            })
        })
        .collect();
    let links = generate_links(&places, &indices, real("longest_rod", 1.0, 10.0));
    let top = has_flag(root, "flags", "Uses grid attach top");
    let pins = cloth_pins(attach, secondary, [size[0], high], places.len(), top, game);
    let cloth = H3ClothAt {
        places,
        indices,
        links,
        pins,
        spheres,
        iterations: properties
            .read_int_any("Number iterations")
            .unwrap_or(1)
            .clamp(1, 8) as i32,
        gravity: real("weight", -10.0, 10.0) * gravity_per_tick(tick_rate(game)),
        drag: real("drag", 0.0, 0.5),
        wind_scale: real("wind_scale", 0.0, 3.0),
        flappiness: properties.read_real("wind_flappiness_scale").unwrap_or(0.0),
        first_vertex: 0,
        game,
        center: (middle_node, middle.position),
    };
    Some((cloth, uvs, under))
}

/// Reach's binormal for a cloth vertex: `deform.hlsl_include` takes
/// `cross(normal, tangent)`, its sign from the position's w, which
/// `cloth_render_opaque` (the 360's 0x82C8D6A0) writes as 1; the tangent is
/// `global_forward3d`.
pub(super) fn reach_binormal(normal: [f32; 3]) -> [f32; 3] {
    cross(normal, [1.0, 0.0, 0.0])
}

/// `cloth_render_opaque`'s mesh: both sides, to be written over each frame.
pub(super) fn sheet_vertices(uvs: &[[f32; 2]], game: ClothGame) -> Vec<RenderModelPreviewVertex> {
    [1.0, -1.0]
        .iter()
        .flat_map(|side| {
            let (tangent, binormal) = match game.rules().sheet_basis {
                ClothSheetBasis::TurnsOver => ([1.0 * side, 0.0, 0.0], [0.0, 0.0, 1.0 * side]),
                ClothSheetBasis::BinormalTurns => ([1.0, 0.0, 0.0], [0.0, *side, 0.0]),
                ClothSheetBasis::RestNormal => ([1.0, 0.0, 0.0], reach_binormal([0.0, 1.0, 0.0])),
            };
            uvs.iter().map(move |uv| RenderModelPreviewVertex {
                texcoord: *uv,
                tangent,
                binormal,
                ..Default::default()
            })
        })
        .collect()
}

/// The region the preview lists a sheet under, which no game has: the tag's
/// name. Reach's keeps its `.cloth`, since its models' variants name regions
/// the render model lacks, and one such (the flag's `flag`) would hide a
/// sheet of the same name.
pub(super) fn cloth_region_name(path: &str, game: ClothGame) -> String {
    let name = path.rsplit('\\').next().unwrap_or(path);
    if game.rules().region_keeps_extension {
        format!("{name}.cloth")
    } else {
        name.to_owned()
    }
}

/// An object's cloth, added to its model as a mesh of its own, drawn with the
/// tag's shader from the first active permutation of a region of its own.
/// `center` is the middle of the object's bounding sphere, in its own space,
/// where Reach starts a sheet; the other games start one at its first pin.
pub(in crate::app::model_preview) fn read_cloth<
    F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
>(
    cx: &super::super::load::LoadCx<'_, F>,
    preview: &mut RenderModelPreview,
    game: ClothGame,
    center: [f32; 3],
) {
    let super::super::load::LoadCx {
        reader,
        object,
        owner,
        markers,
        frame,
        ..
    } = *cx;
    for path in cloth_tags(object) {
        let Ok(tag) = reader.load(&path, "cloth", b"clwd") else {
            continue;
        };
        let root = tag.root();
        let Some((shader_group, shader_path)) = root.read_tag_ref_with_group("Shader") else {
            continue;
        };
        let Some((mut cloth, uvs, under)) = read_cloth_tag(root, markers, &frame, game, center)
        else {
            continue;
        };
        cloth.first_vertex = preview.vertices.len();
        let sheet = sheet_vertices(&uvs, game);
        let Some(base) = VertexBase::of(cloth.first_vertex, sheet.len(), Overflow::RefusesAppend)
        else {
            continue;
        };
        let count = cloth.places.len() as u32;
        let index_start = preview.indices.len() as u32;
        preview.vertices.extend(sheet);
        // The sides face apart, as `cloth_render_opaque`'s cull modes have it.
        preview
            .indices
            .extend(cloth.indices.iter().map(|index| base.offset(*index as u32)));
        preview
            .indices
            .extend(cloth.indices.chunks_exact(3).rev().flat_map(|triangle| {
                [triangle[2], triangle[1], triangle[0]]
                    .map(|index| base.offset(index as u32) + count)
            }));
        let name = cloth_region_name(&path, game);
        preview.materials.push(RenderModelPreviewMaterial {
            shader_path,
            shader_group,
            object: owner,
            ..Default::default()
        });
        // On the opened object it follows the region of the marker it hangs
        // from; an attached object's regions are renamed when it is merged.
        let (name, permutation) = match under.filter(|_| owner == 0) {
            Some(under) => under,
            None => {
                let permutation = "cloth".to_owned();
                preview.regions.push(RenderModelPreviewRegion {
                    name: name.clone(),
                    permutations: vec![permutation.clone()],
                });
                (name, permutation)
            }
        };
        preview.batches.push(RenderModelPreviewBatch {
            region_name: name,
            permutation_name: permutation,
            material_index: (preview.materials.len() - 1) as u16,
            index_start,
            index_count: cloth.indices.len() as u32 * 2,
            flat_color: None,
            layer: ModelPreviewLayer::Render,
            sort_position: None,
        });
        // Either game's PRT attributes stay parallel to the vertices.
        if let Some(prt) = preview.h2.prt.as_mut() {
            prt.pad(cloth.places.len() * 2);
        }
        if let Some(prt) = preview.gen3.prt.as_mut() {
            prt.pad(cloth.places.len() * 2);
        }
        preview.cloth.push(cloth);
    }
}
