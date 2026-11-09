//! Halo 2's `contrail`: points let go at a marker, each stepping through the
//! tag's `point states`, drawn as a ribbon. Unlike Halo 3's they keep coming
//! from something standing still.

use std::sync::Arc;

use blam_tags::{TagFieldData, TagFile, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::common::fields::{elements, has_flag, object_block};
use crate::app::model_preview::common::sprites::AntennaQuad;
use super::super::common::view::{ViewFrame, posed_on};
use super::super::functions::FunctionValues;
use super::lights::attachment_value;
use super::super::common::image::Image;
use super::super::effects::marker_name;
use super::super::gen3::shaders::H3Blend;
use super::super::materials::decode_image;
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::model_preview::common::math::{checked_unit, cross};
use crate::app::model_preview::common::random::{high16_over_65536, lcg_step};
use crate::app::model_preview::common::attached::attached;

const PRE_ROLL: f32 = 4.0;
/// A generous bound; the game's is a shared pool.
const MOST_POINTS: usize = 256;
/// `physics_constants_reset`, for the air a point sits in with no level around it.
const GRAVITY: f32 = 4.171_259_4;
const AIR_DENSITY: f32 = 0.0011;
/// What `point_physics_group_postprocess` turns a density into.
const MASS_OF_DENSITY: f32 = 0.001 * 4.188_790_3 * 28_316_846.0;
const FRICTION_SCALE: f32 = 10_000.0;

/// A `point_physics`, as it is once loaded.
#[derive(Debug, Clone, Copy)]
struct PointPhysics {
    mass: f32,
    density: f32,
    air_friction: f32,
    no_gravity: bool,
}

impl PointPhysics {
    fn read(tag: &TagFile) -> Option<Self> {
        let root = tag.root();
        let density = root.read_real("density")?;
        Some(Self {
            mass: density * MASS_OF_DENSITY,
            density,
            air_friction: root.read_real("air friction")? * FRICTION_SCALE,
            no_gravity: has_flag(root, "flags", "no gravity"),
        })
    }

    /// `point_physics_update` for a point of this radius in still air, which
    /// is all a preview has: no wind, no water, nothing to run into.
    fn step(&self, place: &mut [f32; 3], velocity: &mut [f32; 3], radius: f32, dt: f32) {
        let buoyancy = if self.no_gravity { 0.0 } else { 1.0 - 2.0 * self.density / (AIR_DENSITY + self.density) };
        let bulk = (AIR_DENSITY * MASS_OF_DENSITY + self.mass) * radius * radius * radius;
        let drag = self.air_friction * radius * radius;
        let slowing = if bulk == 0.0 {
            if drag == 0.0 { 0.0 } else { 1.0 }
        } else {
            (drag * dt / bulk).clamp(0.0, 1.0)
        };
        velocity[2] += GRAVITY * buoyancy * dt;
        for axis in 0..3 {
            velocity[axis] -= velocity[axis] * slowing;
            place[axis] += velocity[axis] * dt;
        }
    }
}

/// One of `point states`.
#[derive(Debug, Clone, Copy)]
struct PointState {
    duration: [f32; 2],
    transition: [f32; 2],
    physics: Option<PointPhysics>,
    width: f32,
    /// Alpha, red, green and blue, lower and upper.
    colors: [[f32; 4]; 2],
    /// `duration`, `duration delta`, `transition duration`, `transition duration
    /// delta`, `width` and `color`, scaled by what the point was born with.
    scaled: [bool; 6],
}

/// A `contrail` at a marker of the object.
#[derive(Debug, Clone)]
pub(crate) struct H2ContrailAt {
    pub(crate) object: u8,
    /// The region and permutation its marker belongs to, if only one.
    only_under: Option<(String, String)>,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    scale_by: String,
    rate: f32,
    speed: [f32; 2],
    cone: f32,
    render_type: String,
    repeats: f32,
    repeats_v: f32,
    fade_first: bool,
    fade_last: bool,
    blend: H3Blend,
    states: Arc<Vec<PointState>>,
    bitmap: Option<Arc<Image>>,
}

fn read_state(reader: &TagReader, state: TagStruct<'_>) -> Option<PointState> {
    let duration = state.read_real_bounds("duration");
    let transition = state.read_real_bounds("transition duration");
    let color = |name: &str| {
        match state.field(name).and_then(|field| field.value()) {
            Some(TagFieldData::RealArgbColor(color)) => [color.alpha, color.red, color.green, color.blue],
            _ => [0.0; 4],
        }
    };
    let flagged = |wanted: &str| has_flag(state, "scale flags", wanted);
    let physics = state.read_tag_ref_path("physics").filter(|path| !path.is_empty());
    let physics = physics.and_then(|path| reader.load(&path, "point_physics", b"pphy").ok());
    Some(PointState {
        duration: [duration.lower, duration.upper],
        transition: [transition.lower, transition.upper],
        physics: physics.and_then(|tag| PointPhysics::read(&tag)),
        width: state.read_real("width")?,
        colors: [color("color lower bound"), color("color upper bound")],
        scaled: ["duration", "duration delta", "transition duration", "transition duration delta", "width", "color"]
            .map(flagged),
    })
}

/// What `framebuffer blend function` names.
fn blend_named(name: &str) -> H3Blend {
    match name {
        "add" => H3Blend::Additive,
        "multiply" => H3Blend::Multiply,
        "double multiply" => H3Blend::DoubleMultiply,
        "alpha-multiply add" => H3Blend::PreMultiplied,
        _ => H3Blend::AlphaBlend,
    }
}

/// `object_attachment`s of group `cont`, at every marker of their name.
pub(in crate::app::model_preview) fn read_h2_contrails(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H2ContrailAt> {
    let group = u32::from_be_bytes(*b"cont");
    let mut found = Vec::new();
    let Some(block) = object_block(object, "attachments") else {
        return found;
    };
    for attachment in elements(block, "attachments") {
        let Some((kind, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if kind != group || path.is_empty() {
            continue;
        }
        let Ok(tag) = reader.load(&path, "contrail", b"cont") else {
            continue;
        };
        let root = tag.root();
        let states: Vec<PointState> = elements(root, "point states").into_iter().filter_map(|state| read_state(reader, state)).collect();
        if states.len() < 2 {
            continue;
        }
        let flagged = |wanted: &str| has_flag(root, "flags", wanted);
        let speed = root.read_real_bounds("point velocity");
        let bitmap = root.read_tag_ref_path("bitmap").filter(|path| !path.is_empty());
        let bitmap = bitmap.and_then(|path| decode_image(reader, &path, SamplerKind::Texture2D, 0)).map(Arc::new);
        let states = Arc::new(states);
        let name = marker_name(attachment, "marker");
        for marker in markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(&name)) {
            let (place, node) = frame(marker);
            found.push(H2ContrailAt {
                object: owner,
                only_under: marker.only_under.clone(),
                node,
                position: place.position,
                forward: place.axes[0],
                scale_by: attachment.read_string_id("primary scale").unwrap_or_default(),
                rate: root.read_real("point generation rate").unwrap_or(0.0),
                speed: [speed.lower, speed.upper],
                cone: root.read_real("point velocity cone angle").unwrap_or(0.0),
                render_type: root.read_enum_option("render type").unwrap_or_default(),
                repeats: root.read_real("texture repeats u").unwrap_or(1.0),
                repeats_v: root.read_real("texture repeats v").unwrap_or(1.0),
                fade_first: !flagged("first point unfaded"),
                fade_last: !flagged("last point unfaded"),
                blend: blend_named(&root.read_enum_option("framebuffer blend function").unwrap_or_default()),
                states: Arc::clone(&states),
                bitmap: bitmap.clone(),
            });
        }
    }
    found
}

/// A `contrail_point_datum`.
#[derive(Debug, Clone)]
struct Point {
    place: [f32; 3],
    velocity: [f32; 3],
    /// Before the first state until the first update.
    state: i32,
    age: f32,
    inverse: f32,
    moving_on: bool,
    done: bool,
    /// The game's first update leaves a new point where it was let go.
    new: bool,
    scale: f32,
}

impl Point {
    fn new(place: [f32; 3], velocity: [f32; 3], scale: f32) -> Self {
        Self { place, velocity, state: -1, age: 0.0, inverse: 0.0, moving_on: true, done: false, new: true, scale }
    }
}

fn next_random(random: &mut u32) -> f32 {
    *random = lcg_step(*random);
    high16_over_65536(*random)
}

#[derive(Debug, Clone, Default)]
struct RunningContrail {
    /// Newest first, as the game links them.
    points: Vec<Point>,
    owed: f32,
    random: u32,
}

/// `contrail_point_scaled_range`, with `flag` the scale flag of the lower bound.
fn scaled_range(bounds: [f32; 2], scaled: &[bool; 6], flag: usize, scale: f32, random: f32) -> f32 {
    let lower = bounds[0] * if scaled[flag] { scale } else { 1.0 };
    let delta = (bounds[1] - bounds[0]) * if scaled[flag + 1] { scale } else { 1.0 };
    lower + random * delta
}

/// `contrail_update_points`: time in a state, then on the way to the next.
fn step_point(point: &mut Point, states: &[PointState], dt: f32, random: &mut u32) {
    if point.done {
        return;
    }
    point.age += point.inverse * dt;
    while point.inverse == 0.0 || point.age > 1.0 {
        let seconds = if point.moving_on {
            point.state += 1;
            point.moving_on = false;
            let state = &states[point.state as usize];
            scaled_range(state.duration, &state.scaled, 0, point.scale, next_random(random))
        } else if point.state as usize + 1 >= states.len() {
            point.done = true;
            return;
        } else {
            point.moving_on = true;
            let state = &states[point.state as usize];
            scaled_range(state.transition, &state.scaled, 2, point.scale, next_random(random))
        };
        point.age = 0.0;
        point.inverse = if seconds != 0.0 { seconds.recip() } else { 0.0 };
    }
    if point.new {
        point.new = false;
    } else {
        let state = &states[point.state as usize];
        if let Some(physics) = &state.physics {
            physics.step(&mut point.place, &mut point.velocity, state.width * 0.5, dt);
        }
    }
}

/// Width and color, blended on the way between two states.
fn point_look(point: &Point, states: &[PointState]) -> (f32, [f32; 4]) {
    let at = |state: &PointState| {
        let color_by = if state.scaled[5] { point.scale } else { 1.0 };
        let [low, high] = state.colors;
        let color: [f32; 4] = std::array::from_fn(|channel| low[channel] + (high[channel] - low[channel]) * color_by);
        (state.width * if state.scaled[4] { point.scale } else { 1.0 }, color)
    };
    let (width, color) = at(&states[point.state.max(0) as usize]);
    if !point.moving_on {
        return (width, color);
    }
    let (next_width, next) = at(&states[(point.state + 1) as usize]);
    (width + (next_width - width) * point.age, std::array::from_fn(|channel| color[channel] + (next[channel] - color[channel]) * point.age))
}

impl H2ContrailAt {
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }



    pub(crate) fn bitmap(&self) -> Option<&Arc<Image>> {
        self.bitmap.as_ref()
    }

    pub(crate) fn blend(&self) -> H3Blend {
        self.blend
    }
}



/// What a slot keeps of its contrails.
#[derive(Debug, Clone, Default)]
pub(crate) struct H2ContrailField {
    time: Option<f32>,
    running: Vec<RunningContrail>,
}

impl H2ContrailField {
    pub(crate) fn advance(
        &mut self,
        contrails: &[H2ContrailAt],
        values: impl Fn(u8) -> Option<FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        time: f32,
    ) {
        if self.running.len() != contrails.len() || self.time.is_none_or(|last| time < last) {
            self.running = (0..contrails.len())
                .map(|at| RunningContrail { random: at as u32 * 977 + 1, ..Default::default() })
                .collect();
            self.time = Some(time - PRE_ROLL);
        }
        // `game_frame` runs `contrails_update` each frame, by the frame's time.
        let now = resume_point(self.time.unwrap_or(time), time, PRE_ROLL);
        for step in frame_steps(now, time) {
            for (contrail, running) in contrails.iter().zip(&mut self.running) {
                // Only asked for a name: each answer is a fresh map.
                let scale = if contrail.scale_by.is_empty() {
                    1.0
                } else {
                    attachment_value(&contrail.scale_by, values(contrail.object).as_ref())
                };
                if scale > 0.0 {
                    running.owed += contrail.rate * step;
                }
                let (origin, forward) = posed_on(contrail.node, contrail.position, contrail.forward, bones);
                while running.owed >= 1.0 {
                    running.owed -= 1.0;
                    if running.points.len() >= MOST_POINTS {
                        break;
                    }
                    // `random_vector3d_in_cone`, near enough for what is mostly a narrow one.
                    let spread = contrail.cone * next_random(&mut running.random);
                    let turn = next_random(&mut running.random) * std::f32::consts::TAU;
                    let side = cross(forward, [0.0, 0.0, 1.0]);
                    let side = checked_unit(side).unwrap_or([0.0, 1.0, 0.0]);
                    let lift = cross(side, forward);
                    let direction: [f32; 3] = std::array::from_fn(|axis| {
                        forward[axis] * spread.cos()
                            + (side[axis] * turn.cos() + lift[axis] * turn.sin()) * spread.sin()
                    });
                    let speed = contrail.speed[0] + (contrail.speed[1] - contrail.speed[0]) * next_random(&mut running.random);
                    running.points.insert(0, Point::new(origin, direction.map(|axis| axis * speed), scale));
                }
                let RunningContrail { points, random, .. } = running;
                for point in points {
                    step_point(point, &contrail.states, step, random);
                }
                while running.points.last().is_some_and(|point| point.done) {
                    running.points.pop();
                }
            }
        }
        self.time = Some(time);
    }

    /// `contrail_vertex_builder`, a quad between each two points.
    pub(crate) fn quads(&self, index: usize, contrail: &H2ContrailAt, view: &ViewFrame) -> Vec<AntennaQuad> {
        let Some(running) = self.running.get(index).filter(|running| running.points.len() >= 2) else {
            return Vec::new();
        };
        // The game logs the rest as unsupported, and draws nothing.
        let shapes = ["vertical orientation", "horizontal orientation", "media mapped", "viewer facing"];
        if !shapes.contains(&contrail.render_type.as_str()) {
            return Vec::new();
        }
        let points = &running.points;
        let last = points.len() - 1;
        let across_at = |at: usize| -> Option<[f32; 3]> {
            let (a, b) = if at == 0 { (&points[0], &points[1]) } else { (&points[at - 1], &points[at]) };
            let along: [f32; 3] = std::array::from_fn(|axis| b.place[axis] - a.place[axis]);
            let to_eye: [f32; 3] = std::array::from_fn(|axis| view.eye[axis] - a.place[axis]);
            match contrail.render_type.as_str() {
                "vertical orientation" => Some([0.0, 0.0, 1.0]),
                "horizontal orientation" | "media mapped" => checked_unit([along[1], -along[0], 0.0]),
                _ => checked_unit(cross(along, to_eye)),
            }
        };
        let vertex = |at: usize| {
            let (width, [alpha, red, green, blue]) = point_look(&points[at], &contrail.states);
            let ends = (at == 0 && contrail.fade_first && points.len() > 2) || (at == last && contrail.fade_last);
            let alpha = if ends { 0.0 } else { alpha.clamp(0.0, 1.0) };
            let across = across_at(at).unwrap_or([0.0, 0.0, 1.0]).map(|axis| axis * width * 0.5);
            let u = at as f32 * -contrail.repeats;
            let place = points[at].place;
            let side = |sign: f32, v: f32| -> [f32; 5] {
                let [x, y, z] = std::array::from_fn(|axis| place[axis] + across[axis] * sign);
                [x, y, z, u, v]
            };
            (side(1.0, 0.0), side(-1.0, contrail.repeats_v), [red, green, blue, alpha])
        };
        (0..last)
            .map(|at| {
                let (top, bottom, near) = vertex(at);
                let (next_top, next_bottom, far) = vertex(at + 1);
                let color = std::array::from_fn(|channel| (near[channel] + far[channel]) * 0.5);
                AntennaQuad { corners: [top, next_top, bottom, next_bottom], color }
            })
            .collect()
    }
}

attached!(keeps function: H2ContrailAt);

#[cfg(test)]
mod tests {
    use super::*;

    const SMOKE: PointPhysics = PointPhysics { mass: 0.001 * MASS_OF_DENSITY, density: 0.001, air_friction: 10.0, no_gravity: false };

    fn state(duration: f32, transition: f32) -> PointState {
        PointState {
            duration: [duration; 2],
            transition: [transition; 2],
            physics: Some(SMOKE),
            width: 0.1,
            colors: [[0.0; 4], [1.0; 4]],
            scaled: [false, false, false, false, false, true],
        }
    }

    fn fresh() -> Point {
        Point::new([0.0; 3], [1.0, 0.0, 0.0], 0.5)
    }

    #[test]
    fn a_point_holds_moves_on_and_ends_at_the_last_state() {
        let states = [state(0.0, 1.0), state(0.0, 0.0)];
        let (mut point, mut random) = (fresh(), 1);
        step_point(&mut point, &states, 0.1, &mut random);
        assert!(point.moving_on, "a state of no duration goes straight to its way out");
        assert_eq!(point.state, 0);
        for _ in 0..12 {
            step_point(&mut point, &states, 0.1, &mut random);
        }
        assert!(point.done, "and the last state has nowhere to go");
    }

    #[test]
    fn a_first_update_leaves_a_point_where_it_was_let_go() {
        let states = [state(1.0, 1.0), state(0.0, 0.0)];
        let (mut point, mut random) = (fresh(), 1);
        step_point(&mut point, &states, 0.1, &mut random);
        assert_eq!(point.place, [0.0; 3]);
        step_point(&mut point, &states, 0.1, &mut random);
        assert!(point.place[0] > 0.0);
    }

    #[test]
    fn what_has_no_physics_stays_where_it_was_let_go() {
        let mut still = state(1.0, 1.0);
        still.physics = None;
        let states = [still, state(0.0, 0.0)];
        let (mut point, mut random) = (fresh(), 1);
        for _ in 0..3 {
            step_point(&mut point, &states, 0.1, &mut random);
        }
        assert_eq!(point.place, [0.0; 3]);
    }

    #[test]
    fn smoke_lighter_than_air_rises_and_what_is_heavier_falls() {
        let rise = |physics: PointPhysics| {
            let (mut place, mut velocity) = ([0.0; 3], [0.0; 3]);
            for _ in 0..30 {
                physics.step(&mut place, &mut velocity, 0.05, 1.0 / 30.0);
            }
            place[2]
        };
        assert!(rise(SMOKE) > 0.0);
        assert!(rise(PointPhysics { density: 1.0, mass: MASS_OF_DENSITY, ..SMOKE }) < 0.0);
        assert_eq!(rise(PointPhysics { no_gravity: true, ..SMOKE }), 0.0);
    }

    #[test]
    fn friction_takes_up_what_a_point_was_thrown_with() {
        let (mut place, mut velocity) = ([0.0; 3], [1.0, 0.0, 0.0]);
        SMOKE.step(&mut place, &mut velocity, 0.05, 1.0 / 30.0);
        assert!(velocity[0] < 1.0 && velocity[0] > 0.0);
        let free = PointPhysics { air_friction: 0.0, ..SMOKE };
        let mut velocity = [1.0, 0.0, 0.0];
        free.step(&mut place, &mut velocity, 0.05, 1.0 / 30.0);
        assert_eq!(velocity[0], 1.0);
    }

    #[test]
    fn a_range_is_scaled_only_where_the_tag_says() {
        let plain = [false; 6];
        assert_eq!(scaled_range([1.0, 3.0], &plain, 0, 0.5, 0.5), 2.0);
        let lower = [true, false, false, false, false, false];
        assert_eq!(scaled_range([1.0, 3.0], &lower, 0, 0.5, 0.5), 1.5);
        let delta = [false, true, false, false, false, false];
        assert_eq!(scaled_range([1.0, 3.0], &delta, 0, 0.5, 0.5), 1.5);
        assert_eq!(scaled_range([1.0, 3.0], &lower, 2, 0.5, 0.5), 2.0, "the transition's flags are its own");
    }

    #[test]
    fn a_blend_comes_from_its_option_name() {
        assert_eq!(blend_named("add"), H3Blend::Additive);
        assert_eq!(blend_named("alpha blend"), H3Blend::AlphaBlend);
        assert_eq!(blend_named("alpha-multiply add"), H3Blend::PreMultiplied);
    }

    #[test]
    fn color_is_scaled_where_the_tag_says() {
        let states = [state(1.0, 1.0), state(0.0, 0.0)];
        let (mut point, mut random) = (fresh(), 1);
        step_point(&mut point, &states, 0.01, &mut random);
        assert_eq!(point_look(&point, &states).1, [0.5; 4]);
        let mut plain = state(1.0, 1.0);
        plain.scaled = [false; 6];
        assert_eq!(point_look(&point, &[plain, plain]).1, [1.0; 4], "the upper bound, without");
    }
}
