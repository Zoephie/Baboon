//! Pose mode's move gizmo on the selected joint: an arrow per model axis
//! (x forward red, y left green, z up blue) to drag the joint along that
//! axis alone, and a square per plane between two of them to drag it within
//! that plane. The joint itself still drags across the plane facing the
//! camera.

use super::super::PreviewCamera;
use super::super::pose::Lock;
use eframe::egui::{self, Color32, Pos2, Shape, Stroke, Vec2};

/// How long an arrow is on screen, in points, whatever the zoom.
const ARROW: f32 = 70.0;
/// Where along an arrow its hit area begins, as a share of its length: the
/// joint's own dot is nearer.
const ARROW_FROM: f32 = 0.22;
/// How near an arrow, in points, a press takes it.
const ARROW_PICK: f32 = 6.0;
/// A plane square's corners, as shares of an arrow's length along each of
/// its two axes.
const SQUARE: [f32; 2] = [0.25, 0.45];

/// Each axis's colour: x red, y green, z blue.
const AXIS_COLORS: [Color32; 3] = [
    Color32::from_rgb(225, 70, 70),
    Color32::from_rgb(90, 195, 90),
    Color32::from_rgb(80, 130, 235),
];
/// A handle under the pointer or in use.
const LIT: Color32 = Color32::from_rgb(255, 215, 80);

/// The gizmo as it lies on screen.
pub(super) struct PoseGizmo {
    center: Pos2,
    /// Each axis arrow's tip.
    tips: [Pos2; 3],
    /// Each plane square's corners, by the axis square to the plane.
    squares: [[Pos2; 4]; 3],
}

impl PoseGizmo {
    /// The gizmo at `joint`, seen through `camera`; none where the joint is
    /// behind the eye.
    pub(super) fn at(camera: &PreviewCamera, joint: [f32; 3]) -> Option<Self> {
        let projected = camera.project(joint);
        if !projected.in_front {
            return None;
        }
        let per_unit = camera.points_per_unit(joint);
        if per_unit <= 1e-6 {
            return None;
        }
        let length = ARROW / per_unit;
        let along = |axes: &[(usize, f32)]| {
            let mut point = joint;
            for &(axis, share) in axes {
                point[axis] += length * share;
            }
            camera.project(point).pos
        };
        let tips = [0, 1, 2].map(|axis| along(&[(axis, 1.0)]));
        let squares = [0, 1, 2].map(|normal| {
            let (a, b) = ((normal + 1) % 3, (normal + 2) % 3);
            let [near, far] = SQUARE;
            [
                along(&[(a, near), (b, near)]),
                along(&[(a, far), (b, near)]),
                along(&[(a, far), (b, far)]),
                along(&[(a, near), (b, far)]),
            ]
        });
        Some(Self {
            center: projected.pos,
            tips,
            squares,
        })
    }

    /// The handle under `pointer`: a plane square first, as they lie
    /// between the arrows, then the nearest arrow.
    pub(super) fn hit(&self, pointer: Pos2) -> Option<Lock> {
        if let Some(normal) =
            (0..3).find(|&normal| self.shown(normal) && inside(&self.squares[normal], pointer))
        {
            return Some(Lock::Plane(normal));
        }
        (0..3)
            .map(|axis| {
                let start = self.center + (self.tips[axis] - self.center) * ARROW_FROM;
                (axis, segment_distance(pointer, start, self.tips[axis]))
            })
            .filter(|&(_, distance)| distance <= ARROW_PICK)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(axis, _)| Lock::Axis(axis))
    }

    /// Paint it, `lit` the handle under the pointer or in use.
    pub(super) fn draw(&self, painter: &egui::Painter, lit: Option<Lock>) {
        for normal in (0..3).filter(|&normal| self.shown(normal)) {
            let color = if lit == Some(Lock::Plane(normal)) {
                LIT
            } else {
                // A plane takes the colour of the axis square to it.
                AXIS_COLORS[normal]
            };
            painter.add(Shape::convex_polygon(
                self.squares[normal].to_vec(),
                color.gamma_multiply(0.35),
                Stroke::new(1.0_f32, color),
            ));
        }
        for axis in 0..3 {
            let color = if lit == Some(Lock::Axis(axis)) {
                LIT
            } else {
                AXIS_COLORS[axis]
            };
            let tip = self.tips[axis];
            let span = tip - self.center;
            if span.length() < 1.0 {
                // Seen end on: a dot where the arrow points at the eye.
                painter.circle_filled(tip, 3.0, color);
                continue;
            }
            let start = self.center + span * ARROW_FROM;
            painter.line_segment(
                [start, tip],
                Stroke::new(3.0_f32, Color32::from_black_alpha(160)),
            );
            painter.line_segment([start, tip], Stroke::new(2.0_f32, color));
            let back = -span.normalized() * 9.0;
            let side = Vec2::new(-back.y, back.x) * 0.45;
            painter.add(Shape::convex_polygon(
                vec![tip, tip + back + side, tip + back - side],
                color,
                Stroke::new(1.0_f32, Color32::from_black_alpha(160)),
            ));
        }
    }
}

/// A plane square's least area on screen, in square points, to be drawn
/// and taken (face on it is about 200): seen nearer edge on it is a sliver,
/// whose outline's sharp corners egui mitres far out along the sliver, at
/// 3 square points clean across the picture.
const SQUARE_AREA: f32 = 40.0;

impl PoseGizmo {
    /// Whether the plane square to axis `normal` faces the eye enough to show.
    fn shown(&self, normal: usize) -> bool {
        area(&self.squares[normal]) >= SQUARE_AREA
    }
}

/// The area of the quad `corners` on screen (the shoelace formula).
fn area(corners: &[Pos2; 4]) -> f32 {
    let twice: f32 = (0..4)
        .map(|index| {
            let (a, b) = (corners[index], corners[(index + 1) % 4]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    twice.abs() * 0.5
}

/// Whether `point` lies in the convex quad `corners`, either winding.
fn inside(corners: &[Pos2; 4], point: Pos2) -> bool {
    let side = |a: Pos2, b: Pos2| (b - a).x * (point - a).y - (b - a).y * (point - a).x;
    let sides: Vec<f32> = (0..4)
        .map(|index| side(corners[index], corners[(index + 1) % 4]))
        .collect();
    sides.iter().all(|&value| value >= 0.0) || sides.iter().all(|&value| value <= 0.0)
}

/// How far `point` lies from the segment `a`–`b`.
fn segment_distance(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let span = b - a;
    let length = span.length_sq();
    if length <= f32::EPSILON {
        return (point - a).length();
    }
    let t = ((point - a).dot(span) / length).clamp(0.0, 1.0);
    (point - (a + span * t)).length()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gizmo() -> PoseGizmo {
        // Seen straight on: x across to the right, z up, y away from the eye.
        let center = Pos2::new(100.0, 100.0);
        PoseGizmo {
            center,
            tips: [
                Pos2::new(170.0, 100.0),
                Pos2::new(130.0, 80.0),
                Pos2::new(100.0, 30.0),
            ],
            squares: [
                [
                    Pos2::new(107.0, 95.0),
                    Pos2::new(113.0, 91.0),
                    Pos2::new(113.0, 77.0),
                    Pos2::new(107.0, 81.0),
                ],
                [
                    Pos2::new(117.0, 82.0),
                    Pos2::new(117.0, 68.0),
                    Pos2::new(131.0, 68.0),
                    Pos2::new(131.0, 82.0),
                ],
                [
                    Pos2::new(117.0, 95.0),
                    Pos2::new(131.0, 95.0),
                    Pos2::new(137.0, 91.0),
                    Pos2::new(123.0, 91.0),
                ],
            ],
        }
    }

    /// A press on an arrow takes its axis, on a square its plane, and near
    /// the joint's own dot neither.
    #[test]
    fn a_press_takes_the_handle_under_it() {
        let gizmo = gizmo();
        assert_eq!(gizmo.hit(Pos2::new(160.0, 102.0)), Some(Lock::Axis(0)));
        assert_eq!(gizmo.hit(Pos2::new(101.0, 50.0)), Some(Lock::Axis(2)));
        assert_eq!(gizmo.hit(Pos2::new(124.0, 75.0)), Some(Lock::Plane(1)));
        assert_eq!(
            gizmo.hit(Pos2::new(101.0, 99.0)),
            None,
            "the joint's own dot drags freely"
        );
        assert_eq!(gizmo.hit(Pos2::new(60.0, 160.0)), None);
    }

    /// A plane seen edge on is neither drawn nor taken: its square has
    /// collapsed to a sliver.
    #[test]
    fn a_plane_seen_edge_on_is_left_out() {
        let mut gizmo = gizmo();
        assert!(gizmo.shown(1));
        // Clear of the arrows, so only the square could take a press there.
        gizmo.squares[1] = [
            Pos2::new(117.0, 60.0),
            Pos2::new(131.0, 60.2),
            Pos2::new(131.0, 60.4),
            Pos2::new(117.0, 60.2),
        ];
        assert!(!gizmo.shown(1));
        assert_eq!(gizmo.hit(Pos2::new(124.0, 60.2)), None);
    }
}
