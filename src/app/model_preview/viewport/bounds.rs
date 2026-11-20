//! The object's bounding sphere: its `bounding radius` about its `bounding
//! offset` from its root node, drawn as three outline circles.

use super::super::PreviewCamera;
use blam_tags::math::Matrix4;
use eframe::egui;
use eframe::egui::{Color32, Stroke};

/// The sphere's outline colour.
const BOUNDS_COLOR: Color32 = Color32::from_rgb(255, 210, 60);

/// Points on each circle.
const SEGMENTS: usize = 64;

/// A sphere `radius` world units about `center`, as an object's bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app) struct BoundingSphere {
    pub(in crate::app) center: [f32; 3],
    pub(in crate::app) radius: f32,
}

impl BoundingSphere {
    /// The sphere where the game puts it: its `bounding offset` (`center`)
    /// is taken through the object's root node, as a biped's pelvis or a
    /// vehicle's frame, not from the object's origin at its feet. With no
    /// nodes, the origin.
    pub(in crate::app::model_preview) fn about_root(self, root: Option<&Matrix4>) -> Self {
        let Some(root) = root else {
            return self;
        };
        let [x, y, z] = self.center;
        let center = std::array::from_fn(|row| {
            let m = root.m[row];
            m[0] * x + m[1] * y + m[2] * z + m[3]
        });
        Self { center, ..self }
    }
}

/// The circles of `sphere` in each axis plane, `SEGMENTS + 1` points apiece,
/// the last back at the first.
pub(in crate::app::model_preview) fn bounding_circles(sphere: BoundingSphere) -> [Vec<[f32; 3]>; 3] {
    let [x, y, z] = sphere.center;
    let r = sphere.radius;
    let circle = |point: &dyn Fn(f32, f32) -> [f32; 3]| {
        (0..=SEGMENTS)
            .map(|step| {
                let angle = step as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                point(angle.cos() * r, angle.sin() * r)
            })
            .collect()
    };
    [
        circle(&|a, b| [x + a, y + b, z]),
        circle(&|a, b| [x + a, y, z + b]),
        circle(&|a, b| [x, y + a, z + b]),
    ]
}

/// Draw `sphere` over the viewport, each circle outlined dark under its
/// colour as the armature is, skipping what lies behind the eye.
pub(in crate::app::model_preview) fn draw_bounding_sphere(
    painter: &egui::Painter,
    camera: &PreviewCamera,
    sphere: BoundingSphere,
) {
    for circle in bounding_circles(sphere) {
        let projected: Vec<_> = circle
            .iter()
            .map(|&point| {
                let projected = camera.project(point);
                projected.in_front.then_some(projected.pos)
            })
            .collect();
        for pair in projected.windows(2) {
            if let [Some(a), Some(b)] = pair {
                painter.line_segment(
                    [*a, *b],
                    Stroke::new(3.0_f32, Color32::from_rgba_unmultiplied(0, 0, 0, 150)),
                );
                painter.line_segment([*a, *b], Stroke::new(1.25_f32, BOUNDS_COLOR));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_is_taken_from_the_root_node() {
        let sphere = BoundingSphere {
            center: [0.1, 0.0, 0.0],
            radius: 0.4,
        };
        assert_eq!(sphere.about_root(None), sphere);
        // A root node 0.35 up, turned a quarter about z.
        let mut root = Matrix4::IDENTITY;
        root.m = [
            [0.0, -1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.35],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let placed = sphere.about_root(Some(&root));
        assert_eq!(placed.radius, 0.4);
        let expected = [0.0, 0.1, 0.35];
        assert!(
            (0..3).all(|axis| (placed.center[axis] - expected[axis]).abs() < 1e-6),
            "{placed:?}"
        );
    }

    #[test]
    fn each_circle_sits_on_the_sphere_and_closes() {
        let sphere = BoundingSphere {
            center: [1.0, -2.0, 0.5],
            radius: 0.75,
        };
        for circle in bounding_circles(sphere) {
            assert_eq!(circle.len(), SEGMENTS + 1);
            for point in &circle {
                let distance = (0..3)
                    .map(|axis| (point[axis] - sphere.center[axis]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                assert!((distance - sphere.radius).abs() < 1e-4, "{point:?}");
            }
            let (first, last) = (circle[0], circle[SEGMENTS]);
            assert!((0..3).all(|axis| (first[axis] - last[axis]).abs() < 1e-4));
        }
    }
}
