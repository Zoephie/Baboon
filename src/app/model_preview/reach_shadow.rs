//! Reach's object self shadow: who casts, and the orthographic camera and
//! hull `c_lightmap_shadows_view` renders and applies a shadow through. See
//! `docs/reach-preview-research/shadow.md`.

use blam_tags::TagFile;

use super::ce_attachments::object_block;

/// `c_rasterizer_states`' shadow generate biases when the model sets none
/// (MCC .data 0x1420305C0/C4).
const DEFAULT_DEPTH_BIAS: f32 = 0.003;
const DEFAULT_SLOPE_BIAS: f32 = 1.414;
/// MCC's high quality shadows: twice the resolution.
const HIGH_QUALITY: i32 = 2;
/// The shadow surface MCC's resolutions are a share of.
const SURFACE: f32 = 1600.0;

/// An object that casts its own shadow, and the biases its map is drawn with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReachShadowCaster {
    /// Of a whole of window depth.
    pub(crate) depth_bias: f32,
    pub(crate) slope_bias: f32,
}

/// `render_object_has_lightmap_shadow`: not if the object says it casts
/// none; by default only bipeds and vehicles; `always` all but equipment,
/// projectiles and light fixtures. `blur` takes the occlusion spheres'
/// ambient path, which is not built.
pub(crate) fn reach_shadow_caster(object: &TagFile, model: &TagFile) -> Option<ReachShadowCaster> {
    let holder = object_block(object, "lightmap shadow mode")?;
    let flags = holder.read_flag_names("flags").unwrap_or_default();
    if flags.iter().any(|(_, name)| name.starts_with("does not cast shadow")) {
        return None;
    }
    let mode = holder.read_enum_name("lightmap shadow mode").unwrap_or_default();
    let group = object.group().tag.to_be_bytes();
    casts(&mode, &group).then_some(())?;
    // `render_third_person_shadows`: the model's own pair, if it sets a slope.
    let root = model.root();
    let slope = root.read_real("shadow slope scale bias").unwrap_or(0.0);
    Some(if slope != 0.0 {
        ReachShadowCaster { depth_bias: root.read_real("shadow depth compare bias").unwrap_or(0.0), slope_bias: slope }
    } else {
        ReachShadowCaster { depth_bias: DEFAULT_DEPTH_BIAS, slope_bias: DEFAULT_SLOPE_BIAS }
    })
}

fn casts(mode: &str, group: &[u8; 4]) -> bool {
    match mode.split(['#', '{']).next().unwrap_or_default() {
        "default" => matches!(group, b"bipd" | b"vehi"),
        "always" => !matches!(group, b"eqip" | b"proj" | b"lifi"),
        _ => false,
    }
}

type Vector = [f32; 3];

fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vector, b: Vector) -> Vector {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalized(vector: Vector) -> Vector {
    let length = dot(vector, vector).sqrt();
    if length > 0.0 { vector.map(|value| value / length) } else { vector }
}

/// `object_shadow_visible`'s camera over one object's bounding sphere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReachShadowCamera {
    /// From the light toward the object.
    pub(crate) direction: Vector,
    /// The hull's frame: its centre, then `left` and `up` across the light.
    pub(crate) center: Vector,
    pub(crate) left: Vector,
    pub(crate) up: Vector,
    /// Half the box's depth, and its half width.
    pub(crate) half_depth: f32,
    pub(crate) radius: f32,
    /// The map's side, in texels.
    pub(crate) pixels: i32,
    /// `compute_object_shadow_resolution`'s share of the surface, unclamped.
    share: f32,
    /// `View_Projection` for the map, rowed for HLSL, in OpenGL's clip depth.
    pub(crate) view_projection: [f32; 16],
}

/// Which `shadow_apply` filters the map (MCC's remap of `dword_1418553E8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReachShadowFilter {
    /// 76, one fetch.
    Point,
    /// 77, 3x3 points weighted to a bilinear 2x2.
    Bilinear,
    /// 78, 4x4 points weighted to a bilinear 3x3.
    Fancy,
}

impl ReachShadowCamera {
    /// `direction` from the light toward the object; `diameter` the object's
    /// size on screen, in pixels.
    pub(crate) fn new(direction: Vector, center: Vector, radius: f32, diameter: f32) -> Self {
        let d = normalized(direction);
        let radius = radius.max(2e-4);
        let along = d[0].abs().max(d[1].abs());
        let plane: Vector = std::array::from_fn(|axis| center[axis] - radius * d[axis]);
        let depth = 3.0 * radius + along;
        let side = if d[1].abs() >= 0.9 { [-1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let up = normalized(cross(d, side));
        let left = cross(up, d);
        let a = normalized(cross(d, up));
        let b = normalized(cross(a, d));
        // `compute_shadow_resolution_pixels`, doubled by MCC's high quality.
        let pixels = (diameter as i32).clamp(16, 512) * HIGH_QUALITY;
        // x and y across the map, depth along it into [-1, 1]; the half texel
        // offset `render_camera_build_orthogonal_projection` keeps.
        let texel = 1.0 / pixels as f32;
        let rows = [
            (a.map(|value| value / radius), -dot(plane, a) / radius - texel),
            (b.map(|value| value / radius), -dot(plane, b) / radius + texel),
            (d.map(|value| 2.0 * value / depth), -2.0 * dot(plane, d) / depth - 1.0),
        ];
        let mut view_projection = [0.0; 16];
        for (column, (axis, offset)) in rows.iter().enumerate() {
            for row in 0..3 {
                view_projection[row * 4 + column] = axis[row];
            }
            view_projection[12 + column] = *offset;
        }
        view_projection[15] = 1.0;
        let half_depth = depth / 2.0;
        let hull_center = std::array::from_fn(|axis| plane[axis] + half_depth * d[axis]);
        let share = HIGH_QUALITY as f32 * diameter / SURFACE;
        Self { direction: d, center: hull_center, left, up, half_depth, radius, pixels, share, view_projection }
    }

    /// `render_shadow_apply`'s choice by how much of MCC's surface the
    /// object would take.
    pub(crate) fn filter(&self) -> ReachShadowFilter {
        if self.share <= 0.18 {
            ReachShadowFilter::Point
        } else if self.share < 1.0 {
            ReachShadowFilter::Bilinear
        } else {
            ReachShadowFilter::Fancy
        }
    }
}

/// The apply's hull (M .rdata 0x141855050): an octagonal prism along the
/// light, its near end bevelled; x along the light, y left, z up.
const HULL_VERTICES: [Vector; 16] = [
    [1.0, 1.0, 0.4142],
    [1.0, 0.4142, 1.0],
    [1.0, 0.4142, -1.0],
    [1.0, 1.0, -0.4142],
    [1.0, -0.4142, 1.0],
    [1.0, -0.4142, -1.0],
    [1.0, -1.0, 0.4142],
    [1.0, -1.0, -0.4142],
    [-0.6667, 1.0, 0.4142],
    [-0.6667, 1.0, -0.4142],
    [-1.0, 0.4142, -1.0],
    [-1.0, 0.4142, 1.0],
    [-1.0, -0.4142, -1.0],
    [-1.0, -0.4142, 1.0],
    [-0.6667, -1.0, -0.4142],
    [-0.6667, -1.0, 0.4142],
];
const HULL_QUADS: [[usize; 4]; 14] = [
    [0, 1, 2, 3],
    [1, 4, 5, 2],
    [4, 6, 7, 5],
    [8, 9, 10, 11],
    [11, 10, 12, 13],
    [13, 12, 14, 15],
    [0, 3, 9, 8],
    [6, 15, 14, 7],
    [1, 11, 13, 4],
    [2, 5, 12, 10],
    [1, 0, 8, 11],
    [5, 7, 14, 12],
    [3, 2, 10, 9],
    [4, 13, 15, 6],
];

/// The hull's faces as planes facing out, `(normal, distance)`: a point is
/// inside where it lies behind all of them. The stencil the game marks the
/// hull's pixels with keeps exactly the scene inside it.
pub(crate) fn hull_planes() -> [[f32; 4]; 14] {
    let count = HULL_VERTICES.len() as f32;
    let middle: Vector = std::array::from_fn(|axis| HULL_VERTICES.iter().map(|vertex| vertex[axis]).sum::<f32>() / count);
    HULL_QUADS.map(|[first, second, third, _]| {
        let [p, q, r] = [first, second, third].map(|index| HULL_VERTICES[index]);
        let edge = |to: Vector| std::array::from_fn(|axis| to[axis] - p[axis]);
        let mut normal = normalized(cross(edge(q), edge(r)));
        let toward_middle: Vector = std::array::from_fn(|axis| middle[axis] - p[axis]);
        if dot(normal, toward_middle) > 0.0 {
            normal = normal.map(|value| -value);
        }
        [normal[0], normal[1], normal[2], dot(normal, p)]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(point: Vector) -> bool {
        hull_planes().iter().all(|plane| dot([plane[0], plane[1], plane[2]], point) <= plane[3] + 1e-4)
    }

    #[test]
    fn the_hull_holds_its_middle_and_not_its_bevelled_corner() {
        assert!(inside([0.0, 0.0, 0.0]));
        assert!(inside([0.99, 0.0, 0.0]) && inside([-0.99, 0.0, 0.0]));
        assert!(!inside([1.1, 0.0, 0.0]) && !inside([0.0, 1.1, 0.0]));
        assert!(!inside([0.0, 0.95, 0.95]), "the octagon cuts the corners");
        assert!(inside([0.9, 0.95, 0.0]), "full width at the far end");
        assert!(!inside([-0.95, 0.95, 0.0]), "the near end is bevelled");
        for vertex in HULL_VERTICES {
            assert!(inside(vertex), "{vertex:?} on the hull");
        }
    }

    #[test]
    fn the_map_takes_the_sphere_from_the_light_plane_back() {
        let camera = ReachShadowCamera::new([0.1066, 0.6032, -0.7905], [1.0, 2.0, 3.0], 1.0, 300.0);
        let clip = |point: Vector| {
            let m = &camera.view_projection;
            std::array::from_fn::<f32, 3, _>(|column| {
                (0..3).map(|row| point[row] * m[row * 4 + column]).sum::<f32>() + m[12 + column]
            })
        };
        let d = camera.direction;
        // The sphere runs from the light plane two radii into a box three
        // radii (and a little) deep.
        let near = clip(std::array::from_fn(|axis| [1.0, 2.0, 3.0][axis] - d[axis]));
        let far_point = clip(std::array::from_fn(|axis| [1.0, 2.0, 3.0][axis] + d[axis]));
        assert!((near[2] + 1.0).abs() < 1e-4, "the light plane is the sphere's near side: {near:?}");
        assert!(far_point[2] > near[2] && far_point[2] < 1.0, "{far_point:?}");
        let middle = clip([1.0, 2.0, 3.0]);
        assert!(middle[0].abs() <= 2.0 / camera.pixels as f32 && middle[1].abs() <= 2.0 / camera.pixels as f32);
        assert_eq!(camera.pixels, 600);
    }

    #[test]
    fn resolution_picks_the_filter() {
        let camera = |diameter| ReachShadowCamera::new([0.0, 0.0, -1.0], [0.0; 3], 1.0, diameter);
        assert_eq!(camera(100.0).filter(), ReachShadowFilter::Point);
        assert_eq!(camera(300.0).filter(), ReachShadowFilter::Bilinear);
        assert_eq!(camera(4.0).pixels, 32, "the floor, doubled");
        assert_eq!(camera(5000.0).pixels, 1024, "the ceiling, doubled");
        assert_eq!(camera(5000.0).filter(), ReachShadowFilter::Fancy, "by the size on screen, not the clamped map");
    }

    /// With `BLAM_TEST_HREK`: the Spartan casts on its model's own biases,
    /// and the assault rifle, a weapon left at `default`, does not.
    #[test]
    fn the_kits_spartan_casts_and_its_rifle_does_not() {
        use crate::app::export::geometry::load_referenced_tag_from_source;
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            return;
        };
        let source = crate::core::source::TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloReach),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let load = |path: &str, extension: &str, group: &[u8; 4]| load_referenced_tag_from_source(&source, path, extension, group).unwrap();
        let spartan = "objects\\characters\\spartans\\spartans";
        let caster = reach_shadow_caster(&load(spartan, "biped", b"bipd"), &load(spartan, "model", b"hlmt"));
        assert_eq!(caster, Some(ReachShadowCaster { depth_bias: 0.002, slope_bias: 81.0 }));
        let rifle = "objects\\weapons\\rifle\\assault_rifle\\assault_rifle";
        assert_eq!(reach_shadow_caster(&load(rifle, "weapon", b"weap"), &load(rifle, "model", b"hlmt")), None);
    }

    #[test]
    fn bipeds_and_vehicles_cast_by_default() {
        assert!(casts("default", b"bipd") && casts("default", b"vehi"));
        assert!(!casts("default", b"weap") && !casts("default", b"scen"));
        assert!(casts("always", b"weap") && !casts("always", b"eqip"));
        assert!(!casts("never", b"bipd") && !casts("blur", b"bipd"));
    }
}
