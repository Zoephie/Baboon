//! Pose mode's gizmo on the selected joint, in the model's axes (x forward
//! red, y left green, z up blue). To move it: an arrow per axis to drag the
//! joint along that axis alone, and a square per plane between two of them
//! to drag it within that plane; the joint itself still drags across the
//! plane facing the camera. To rotate it: a ring per axis, round which a
//! drag turns the joint about that axis. To scale it: a circle flat to the
//! screen, which a drag outward grows and inward shrinks.

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

/// A rotate ring's radius on screen, in points, whatever the zoom.
pub(super) const RING: f32 = 60.0;
/// Points round a ring, as it is drawn and taken.
const RING_POINTS: usize = 64;
/// How near a ring, in points, a press takes it.
const RING_PICK: f32 = 6.0;

/// The rotate gizmo as it lies on screen.
pub(super) struct PoseRings {
    /// Each axis's ring, its points in the order of a turn about the axis
    /// from the right hand (`cos t · u + sin t · v`, `u × v` the axis).
    rings: [Vec<Pos2>; 3],
    /// Whether each ring's point lies on the near half of its ring, which
    /// draws over the far half.
    near: [Vec<bool>; 3],
}

/// A ring taken by a press: its axis, and its way round on screen there.
pub(super) struct RingHit {
    pub(super) axis: usize,
    pub(super) tangent: Vec2,
}

impl PoseRings {
    /// The rings about `joint`, seen through `camera`.
    pub(super) fn at(camera: &PreviewCamera, joint: [f32; 3]) -> Option<Self> {
        if !camera.project(joint).in_front {
            return None;
        }
        let per_unit = camera.points_per_unit(joint);
        if per_unit <= 1e-6 {
            return None;
        }
        let radius = RING / per_unit;
        let eye = camera.ray(camera.project(joint).pos).1;
        let ring = |axis: usize| {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            (0..RING_POINTS)
                .map(|index| {
                    let t = std::f32::consts::TAU * index as f32 / RING_POINTS as f32;
                    let mut point = joint;
                    point[u] += radius * t.cos();
                    point[v] += radius * t.sin();
                    // Nearer the eye than the joint: on the near half.
                    let toward: f32 = (0..3)
                        .map(|axis| (point[axis] - joint[axis]) * eye[axis])
                        .sum();
                    (camera.project(point).pos, toward <= 0.0)
                })
                .unzip::<_, _, Vec<Pos2>, Vec<bool>>()
        };
        let [x, y, z] = [ring(0), ring(1), ring(2)];
        Some(Self {
            rings: [x.0, y.0, z.0],
            near: [x.1, y.1, z.1],
        })
    }

    /// The ring under `pointer`, the nearest, and its way round there.
    pub(super) fn hit(&self, pointer: Pos2) -> Option<RingHit> {
        let mut best: Option<(f32, usize, Vec2)> = None;
        for (axis, ring) in self.rings.iter().enumerate() {
            for index in 0..ring.len() {
                let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
                let distance = segment_distance(pointer, a, b);
                if distance <= RING_PICK && best.is_none_or(|(closest, _, _)| distance < closest) {
                    let way = b - a;
                    if way.length() > 1e-3 {
                        best = Some((distance, axis, way.normalized()));
                    }
                }
            }
        }
        best.map(|(_, axis, tangent)| RingHit { axis, tangent })
    }

    /// Paint them, `lit` the ring under the pointer or in use: the far
    /// halves faint beneath, the near halves over them.
    pub(super) fn draw(&self, painter: &egui::Painter, lit: Option<usize>) {
        for near in [false, true] {
            for (axis, ring) in self.rings.iter().enumerate() {
                let color = if lit == Some(axis) {
                    LIT
                } else {
                    AXIS_COLORS[axis]
                };
                let color = if near {
                    color
                } else {
                    color.gamma_multiply(0.35)
                };
                for index in 0..ring.len() {
                    let next = (index + 1) % ring.len();
                    if self.near[axis][index] != near || self.near[axis][next] != near {
                        continue;
                    }
                    painter.line_segment([ring[index], ring[next]], Stroke::new(2.0_f32, color));
                }
            }
        }
    }
}

/// The scale circle's radius on screen, in points, whatever the zoom:
/// inside the rotate rings', so the two never sit on each other.
pub(super) const SCALE_CIRCLE: f32 = 45.0;
/// The scale circle's colour, apart from the axes', as it scales on none.
const SCALE_COLOR: Color32 = Color32::from_rgb(225, 225, 225);

/// The scale gizmo as it lies on screen: a circle round the joint, flat to
/// the screen, as the scale is the same along every axis.
pub(super) struct PoseScale {
    pub(super) centre: Pos2,
}

impl PoseScale {
    /// The circle about `joint`, seen through `camera`.
    pub(super) fn at(camera: &PreviewCamera, joint: [f32; 3]) -> Option<Self> {
        let projected = camera.project(joint);
        projected.in_front.then_some(Self { centre: projected.pos })
    }

    /// Whether `pointer` takes the circle.
    pub(super) fn hit(&self, pointer: Pos2) -> bool {
        ((pointer - self.centre).length() - SCALE_CIRCLE).abs() <= RING_PICK
    }

    /// Paint it, `lit` while under the pointer or in use, and beside it the
    /// joint's scale while it is scaled.
    pub(super) fn draw(&self, painter: &egui::Painter, lit: bool, scale: f32) {
        let color = if lit { LIT } else { SCALE_COLOR };
        painter.circle_stroke(self.centre, SCALE_CIRCLE, Stroke::new(2.0_f32, color));
        if lit || (scale - 1.0).abs() >= 1e-4 {
            painter.text(
                self.centre + Vec2::new(SCALE_CIRCLE * 0.75, -SCALE_CIRCLE * 0.75),
                egui::Align2::LEFT_BOTTOM,
                format!("×{scale:.2}"),
                egui::FontId::proportional(13.0),
                color,
            );
        }
    }
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

    /// A press on a ring takes its axis and the ring's way round there.
    #[test]
    fn a_press_takes_the_ring_under_it() {
        // Seen down z: the z ring a circle, x and y edge on.
        let circle: Vec<Pos2> = (0..RING_POINTS)
            .map(|index| {
                let t = std::f32::consts::TAU * index as f32 / RING_POINTS as f32;
                Pos2::new(100.0 + 60.0 * t.cos(), 100.0 - 60.0 * t.sin())
            })
            .collect();
        let line = |vertical: bool| -> Vec<Pos2> {
            (0..RING_POINTS)
                .map(|index| {
                    let t = std::f32::consts::TAU * index as f32 / RING_POINTS as f32;
                    if vertical {
                        Pos2::new(100.0, 100.0 - 60.0 * t.sin())
                    } else {
                        Pos2::new(100.0 + 60.0 * t.cos(), 100.0)
                    }
                })
                .collect()
        };
        let rings = PoseRings {
            rings: [line(true), line(false), circle],
            near: [
                vec![true; RING_POINTS],
                vec![true; RING_POINTS],
                vec![true; RING_POINTS],
            ],
        };
        // At 45° round, clear of the edge-on rings.
        let hit = rings.hit(Pos2::new(142.4, 57.6)).expect("on the z ring");
        assert_eq!(hit.axis, 2);
        assert!(
            hit.tangent.x < -0.6 && hit.tangent.y < -0.6,
            "anticlockwise on screen there: up and left {:?}",
            hit.tangent
        );
        assert!(
            rings.hit(Pos2::new(130.0, 130.0)).is_none(),
            "inside, clear of every ring"
        );
    }
}
