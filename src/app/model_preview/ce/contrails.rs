//! Halo CE's `contrail`: what `attachments_new` hangs on a marker through
//! `contrail_new`, points let go by `contrail_add_points` that step through the
//! tag's `point states` (`contrail_update_points`), drawn as a ribbon by
//! `render_contrail` through the `effect` shader. Read from the Xbox 360
//! build's symbols. Halo 2's are its descendant, in `h2::contrails`.

use std::sync::Arc;

use blam_tags::{TagFile, TagStruct};

use super::attachments::{elements, has_flag, object_block};
use super::lights::{CeFlareView, posed_on};
use super::particles::CePointPhysics;
use super::passes::argb;
use super::super::dx9::passes::H2Image;
use super::super::materials::decode_h2_image;
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;
use crate::app::model_preview::common::math::{cross, dot, unit_or_up};
use crate::app::model_preview::common::random::{high24_over_2_24, lcg_step};

const PRE_ROLL: f32 = 4.0;
/// A generous bound; the game's is a shared pool.
const MOST_POINTS: usize = 256;
/// `contrail_new` follows at most this many markers of the name.
const MOST_MARKERS: usize = 4;

/// The contrail's `scale flags`.
const SCALED_RATE: i128 = 0x1;
const SCALED_VELOCITY: u32 = 1;
const SCALED_CONE: i128 = 0x8;
const SCALED_ANIMATION_RATE: i128 = 0x20;
const SCALED_REPEATS_U: i128 = 0x40;
const SCALED_REPEATS_V: i128 = 0x80;
const SCALED_SCROLL_U: i128 = 0x100;
const SCALED_SCROLL_V: i128 = 0x200;
/// Its `flags`.
const FIRST_POINT_UNFADED: i128 = 0x1;
const LAST_POINT_UNFADED: i128 = 0x2;
const EDGE_FADES_SLOWLY: i128 = 0x40;

/// One of `point states`.
#[derive(Debug, Clone)]
struct PointState {
    duration: [f32; 2],
    transition: [f32; 2],
    physics: Option<CePointPhysics>,
    width: f32,
    /// Alpha, red, green and blue, lower and upper.
    colors: [[f32; 4]; 2],
    /// `duration`, `duration delta`, `transition duration`, `transition
    /// duration delta`, `width` and `color`, scaled by what the point was born with.
    scaled: i128,
}

fn read_state(source: &TagSource, state: TagStruct<'_>) -> PointState {
    let bounds = |name: &str| {
        let bounds = state.read_real_bounds(name);
        [bounds.lower, bounds.upper]
    };
    let physics = state
        .read_tag_ref_path("physics")
        .filter(|path| !path.is_empty())
        .and_then(|path| load_referenced_tag_from_source(source, &path, "point_physics", b"pphy").ok())
        .map(|tag| CePointPhysics::read(&tag));
    let color = |name: &str| {
        let [red, green, blue, alpha] = argb(state, name);
        [alpha, red, green, blue]
    };
    PointState {
        duration: bounds("duration"),
        transition: bounds("transition duration"),
        physics,
        width: state.read_real("width").unwrap_or(0.0),
        colors: [color("color lower bound"), color("color upper bound")],
        scaled: state.read_int_any("scale flags").unwrap_or(0),
    }
}

/// A `contrail` tag.
#[derive(Debug, Clone)]
pub(crate) struct CeContrailDef {
    flags: i128,
    scale_flags: i128,
    rate: f32,
    velocity: [f32; 2],
    cone: f32,
    render_type: i32,
    repeats: [f32; 2],
    scroll: [f32; 2],
    animation_rate: f32,
    sequences: Vec<[usize; 2]>,
    first_sequence: i32,
    sequence_count: i32,
    pub(crate) blend: i32,
    fade_mode: i32,
    pub(crate) nonlinear_tint: bool,
    /// `shader flags`' `sort bias`: sorted a quarter unit nearer.
    pub(crate) sort_bias: bool,
    states: Vec<PointState>,
    /// Every image of its bitmap, by index.
    pub(crate) images: Vec<Option<Arc<H2Image>>>,
}

/// A contrail at the markers of its attachment's name.
#[derive(Debug, Clone)]
pub(crate) struct CeContrailAt {
    pub(crate) object: u8,
    pub(crate) def: Arc<CeContrailDef>,
    /// `A out`.. of `primary scale` (its density) and `change color`.
    primary: Option<usize>,
    change_color: Option<usize>,
    /// Node, position and forward of each marker.
    markers: Vec<(usize, [f32; 3], [f32; 3])>,
}

fn function_slot(value: Option<i128>) -> Option<usize> {
    usize::try_from(value? - 1).ok().filter(|slot| *slot < 4)
}

fn read_contrail(source: &TagSource, path: &str) -> Option<CeContrailDef> {
    let tag = load_referenced_tag_from_source(source, path, "contrail", b"cont").ok()?;
    let root = tag.root();
    let states: Vec<PointState> = elements(root, "point states").into_iter().map(|state| read_state(source, state)).collect();
    if states.is_empty() {
        return None;
    }
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let velocity = root.read_real_bounds("point velocity");
    let short = |name: &str| root.read_int_any(name).unwrap_or(0) as i32;
    let bitmap_path = root.read_tag_ref_path("bitmap").filter(|path| !path.is_empty());
    let (sequences, images) = match &bitmap_path {
        Some(path) => match load_referenced_tag_from_source(source, path, "bitmap", b"bitm") {
            Ok(bitmap) => {
                let sequences = elements(bitmap.root(), "sequences")
                    .iter()
                    .map(|sequence| {
                        let first = sequence.read_int_any("first bitmap index").unwrap_or(0).max(0) as usize;
                        let count = sequence.read_int_any("bitmap count").unwrap_or(0).max(0) as usize;
                        [first, count]
                    })
                    .collect();
                let count = elements(bitmap.root(), "bitmaps").len();
                let images = (0..count)
                    .map(|index| decode_h2_image(source, path, SamplerKind::Texture2D, index).map(Arc::new))
                    .collect();
                (sequences, images)
            }
            Err(_) => (Vec::new(), Vec::new()),
        },
        None => (Vec::new(), Vec::new()),
    };
    let shader_flags = root.read_int_any("shader flags").unwrap_or(0);
    Some(CeContrailDef {
        flags: root.read_int_any("flags").unwrap_or(0),
        scale_flags: root.read_int_any("scale flags").unwrap_or(0),
        rate: real("point generation rate"),
        velocity: [velocity.lower, velocity.upper],
        cone: real("point velocity cone angle"),
        render_type: short("render type"),
        repeats: [real("texture repeats u"), real("texture repeats v")],
        scroll: [real("texture animation u"), real("texture animation v")],
        animation_rate: real("animation rate"),
        sequences,
        first_sequence: short("first sequence index"),
        sequence_count: short("sequence count"),
        blend: short("framebuffer blend function"),
        fade_mode: short("framebuffer fade mode"),
        nonlinear_tint: shader_flags & 2 != 0,
        sort_bias: has_flag(root, "shader flags", "sort bias"),
        states,
        images,
    })
}

/// The contrails among `object`'s attachments.
pub(in crate::app::model_preview) fn read_ce_contrails(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeContrailAt> {
    let Some(block) = object_block(object, "attachments") else {
        return Vec::new();
    };
    let group = u32::from_be_bytes(*b"cont");
    let mut found = Vec::new();
    for attachment in elements(block, "attachments") {
        let Some((kind, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if kind != group || path.is_empty() {
            continue;
        }
        let Some(def) = read_contrail(source, &path) else {
            continue;
        };
        let name = attachment.read_string("marker").unwrap_or_default();
        // `object_get_marker_by_name`: no name is the origin.
        let at: Vec<(usize, [f32; 3], [f32; 3])> = if name.is_empty() {
            vec![(0, [0.0; 3], [1.0, 0.0, 0.0])]
        } else {
            markers
                .iter()
                .filter(|marker| marker.name.eq_ignore_ascii_case(&name))
                .take(MOST_MARKERS)
                .map(|marker| (marker.node_index.max(0) as usize, marker.position, marker.axes[0]))
                .collect()
        };
        if at.is_empty() {
            continue;
        }
        found.push(CeContrailAt {
            object: owner,
            def: Arc::new(def),
            primary: function_slot(attachment.read_int_any("primary scale")),
            change_color: function_slot(attachment.read_int_any("change color")),
            markers: at,
        });
    }
    found
}

/// A `contrail_point_datum`.
#[derive(Debug, Clone)]
struct Point {
    place: [f32; 3],
    velocity: [f32; 3],
    state: i32,
    age: f32,
    inverse: f32,
    /// On the way to the next state, rather than in one.
    moving_on: bool,
    done: bool,
    /// Its first update leaves it where it was let go.
    new: bool,
    density: f32,
}

struct Random(u32);

impl Random {
    fn next(&mut self) -> f32 {
        self.0 = lcg_step(self.0);
        high24_over_2_24(self.0)
    }
}

fn scaled_range(bounds: [f32; 2], flags: i128, bit: u32, scale: f32, random: f32) -> f32 {
    let lower = bounds[0] * if flags & (1 << bit) != 0 { scale } else { 1.0 };
    let delta = (bounds[1] - bounds[0]) * if flags & (1 << (bit + 1)) != 0 { scale } else { 1.0 };
    lower + random * delta
}

/// `contrail_update_points` for one point.
fn step_point(point: &mut Point, states: &[PointState], ground: f32, dt: f32, random: &mut Random) {
    if point.done {
        return;
    }
    point.age += point.inverse * dt;
    while point.inverse == 0.0 || point.age > 1.0 {
        let seconds = if point.moving_on {
            point.state += 1;
            point.moving_on = false;
            let state = &states[point.state as usize];
            scaled_range(state.duration, state.scaled, 0, point.density, random.next())
        } else if point.state as usize + 1 >= states.len() {
            point.done = true;
            return;
        } else {
            point.moving_on = true;
            let state = &states[point.state as usize];
            scaled_range(state.transition, state.scaled, 2, point.density, random.next())
        };
        point.age = 0.0;
        point.inverse = if seconds != 0.0 { seconds.recip() } else { 0.0 };
    }
    if point.new {
        point.new = false;
    } else if let Some(physics) = &states[point.state as usize].physics {
        let radius = states[point.state as usize].width * 0.5;
        physics.step(&mut point.place, &mut point.velocity, radius, ground, dt);
    }
}

/// Width and color (alpha first), blended on the way between two states.
fn point_look(point: &Point, states: &[PointState]) -> (f32, [f32; 4]) {
    let at = |state: &PointState| {
        let color_by = if state.scaled & 0x20 != 0 { point.density } else { 1.0 };
        let [low, high] = state.colors;
        let color: [f32; 4] = std::array::from_fn(|channel| low[channel] + (high[channel] - low[channel]) * color_by);
        (state.width * if state.scaled & 0x10 != 0 { point.density } else { 1.0 }, color)
    };
    let (width, color) = at(&states[point.state.max(0) as usize]);
    if !point.moving_on || point.state + 1 >= states.len() as i32 {
        return (width, color);
    }
    let (next_width, next) = at(&states[(point.state + 1) as usize]);
    (
        width + (next_width - width) * point.age,
        std::array::from_fn(|channel| color[channel] + (next[channel] - color[channel]) * point.age),
    )
}

#[derive(Debug, Clone)]
struct RunningContrail {
    /// Newest first, per marker, as the game links them.
    points: Vec<Vec<Point>>,
    /// Time to the next point: `contrail_compute_new_point_count`.
    until_next: f32,
    density: f32,
    on: bool,
    sequence: i32,
    frame: i32,
    frame_time: f32,
    scroll: [f32; 2],
    random: Random,
}

impl std::fmt::Debug for Random {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Random")
    }
}

impl Clone for Random {
    fn clone(&self) -> Self {
        Random(self.0)
    }
}

/// One ribbon segment: its image, corners (place then u, v) and color.
#[derive(Debug, Clone)]
pub(crate) struct CeContrailQuad {
    pub(crate) contrail: usize,
    pub(crate) image: usize,
    pub(crate) corners: [[f32; 5]; 4],
    /// Each corner's color, as `render_contrail` writes one per vertex.
    pub(crate) colors: [[f32; 4]; 4],
}

/// What a slot keeps of its contrails.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeContrailField {
    time: Option<f32>,
    running: Vec<RunningContrail>,
}

/// `contrail_next_frame`: the next frame of the sequence, or a fresh random
/// sequence of the tag's range past the last.
fn next_frame(running: &mut RunningContrail, def: &CeContrailDef) {
    running.frame_time = 0.0;
    running.frame += 1;
    let count = usize::try_from(running.sequence).ok().and_then(|at| def.sequences.get(at)).map_or(0, |sequence| sequence[1]);
    if running.sequence < 0 || running.frame < 0 || running.frame as usize >= count {
        let lower = def.first_sequence;
        let upper = lower + def.sequence_count;
        running.sequence = lower + (running.random.next() * (upper - lower) as f32) as i32;
        running.frame = 0;
    }
}

impl CeContrailField {
    pub(crate) fn advance(
        &mut self,
        contrails: &[CeContrailAt],
        inputs: impl Fn(u8) -> ([f32; 4], [bool; 4]),
        bones: Option<&[[f32; 4]]>,
        ground: f32,
        time: f32,
    ) {
        if self.running.len() != contrails.len() || self.time.is_none_or(|last| time < last) {
            self.running = contrails
                .iter()
                .enumerate()
                .map(|(at, contrail)| {
                    let mut running = RunningContrail {
                        points: vec![Vec::new(); contrail.markers.len()],
                        until_next: 0.0,
                        density: 0.0,
                        on: false,
                        sequence: -1,
                        frame: 0,
                        frame_time: 0.0,
                        scroll: [0.0; 2],
                        random: Random(at as u32 * 977 + 1),
                    };
                    next_frame(&mut running, &contrail.def);
                    running
                })
                .collect();
            self.time = Some(time - PRE_ROLL);
        }
        // `game_frame` runs `contrails_update` each frame, by the frame's time.
        let now = resume_point(self.time.unwrap_or(time), time, PRE_ROLL);
        for step in frame_steps(now, time) {
            for (contrail, running) in contrails.iter().zip(&mut self.running) {
                let def = &contrail.def;
                // `object_get_function_value`: none is on at one.
                let (density, on) = match contrail.primary {
                    Some(output) => {
                        let (outputs, active) = inputs(contrail.object);
                        (outputs[output], active[output])
                    }
                    None => (1.0, true),
                };
                // Turning on or off lets go one point of no density.
                if on != running.on {
                    running.density = 0.0;
                    add_points(contrail, running, 1, bones);
                }
                running.density = density;
                let mut count = 0;
                running.on = on;
                if on {
                    // `contrail_compute_new_point_count`.
                    let rate = def.rate * if def.scale_flags & SCALED_RATE != 0 { density } else { 1.0 };
                    let period = if rate != 0.0 { 1.0 / rate } else { f32::MAX };
                    let mut left = step;
                    while running.until_next <= left {
                        left -= running.until_next;
                        running.until_next = period;
                        count += 1;
                        if left == 0.0 || count > MOST_POINTS {
                            break;
                        }
                    }
                    running.until_next -= left.max(0.0).min(running.until_next);
                }
                if count > 0 {
                    add_points(contrail, running, count, bones);
                }
                let rate = def.animation_rate * if def.scale_flags & SCALED_ANIMATION_RATE != 0 { density } else { 1.0 };
                if rate > 0.0 {
                    let mut left = step;
                    loop {
                        let span = 1.0 / rate - running.frame_time;
                        if span > left {
                            running.frame_time += left;
                            break;
                        }
                        next_frame(running, def);
                        // A frame shorter than `left`'s precision would never
                        // use it up.
                        if left - span == left {
                            break;
                        }
                        left -= span;
                        if left <= 0.0 {
                            break;
                        }
                    }
                }
                let scroll = |speed: f32, flag: i128| speed * if def.scale_flags & flag != 0 { density } else { 1.0 };
                running.scroll[0] -= scroll(def.scroll[0], SCALED_SCROLL_U) * step;
                running.scroll[1] += scroll(def.scroll[1], SCALED_SCROLL_V) * step;
                let RunningContrail { points, random, .. } = running;
                for line in points.iter_mut() {
                    for point in line.iter_mut() {
                        step_point(point, &def.states, ground, step, random);
                    }
                    // `contrail_update_points` drops the last point only once the
                    // one before it is done too, so a done point anchors the tail.
                    while line.len() > 1 && line[line.len() - 1].done && line[line.len() - 2].done {
                        line.pop();
                    }
                    if line.len() == 1 && line[0].done {
                        line.clear();
                    }
                }
            }
        }
        self.time = Some(time);
    }

    /// `render_contrail`, a quad between each two points of each marker's line.
    pub(crate) fn quads(
        &self,
        contrails: &[CeContrailAt],
        change_colors: impl Fn(u8) -> [[f32; 3]; 4],
        view: &CeFlareView,
    ) -> Vec<CeContrailQuad> {
        let mut quads = Vec::new();
        for (index, (contrail, running)) in contrails.iter().zip(&self.running).enumerate() {
            let def = &contrail.def;
            // Ground mapped and double-marker linked draw nothing.
            if def.render_type == 3 || def.render_type > 4 {
                continue;
            }
            let image = usize::try_from(running.sequence)
                .ok()
                .and_then(|at| def.sequences.get(at))
                .map_or(0, |[first, _]| first + running.frame.max(0) as usize);
            let tint = contrail.change_color.map_or([1.0; 3], |channel| change_colors(contrail.object)[channel]);
            let repeats_u = def.repeats[0] * if def.scale_flags & SCALED_REPEATS_U != 0 { running.density } else { 1.0 };
            let repeats_v = def.repeats[1] * if def.scale_flags & SCALED_REPEATS_V != 0 { running.density } else { 1.0 };
            for points in &running.points {
                if points.len() < 2 {
                    continue;
                }
                let last = points.len() - 1;
                let vertex = |at: usize| {
                    let point = &points[at];
                    let neighbour = if at == 0 { &points[1] } else { &points[at - 1] };
                    let (from, to) = if at == 0 { (point.place, neighbour.place) } else { (neighbour.place, point.place) };
                    let (width, [alpha, red, green, blue]) = point_look(point, &def.states);
                    let half = width * 0.5;
                    let along: [f32; 3] = std::array::from_fn(|axis| to[axis] - from[axis]);
                    // `render_contrail`: the first vertex of each pair (the far
                    // edge of the texture) lies on the minus side of `across`.
                    let (across, normal) = match def.render_type {
                        0 => ([0.0, 0.0, 1.0], unit_or_up([-along[1], along[0], 0.0])),
                        1 | 2 => (unit_or_up([-along[1], along[0], 0.0]), [0.0, 0.0, 1.0]),
                        _ => {
                            let to_eye: [f32; 3] = std::array::from_fn(|axis| view.eye[axis] - from[axis]);
                            let across = unit_or_up(cross(to_eye, along));
                            (across, unit_or_up(cross(across, along)))
                        }
                    };
                    // `contrail_fade`.
                    let mut fade = 1.0;
                    if def.fade_mode != 0 {
                        let to_eye: [f32; 3] = std::array::from_fn(|axis| view.eye[axis] - point.place[axis]);
                        let distance = dot(to_eye, to_eye).sqrt().max(0.0001);
                        fade = (dot(normal, to_eye) / distance).abs();
                        if def.flags & EDGE_FADES_SLOWLY != 0 {
                            fade = super::lights::transition_evaluate(2, fade);
                        }
                        if def.fade_mode == 2 {
                            fade = 1.0 - fade;
                        }
                    }
                    let unfaded = if at == 0 { def.flags & FIRST_POINT_UNFADED != 0 } else { def.flags & LAST_POINT_UNFADED != 0 };
                    let ends = (at == 0 || at == last) && !unfaded;
                    let alpha = if ends { 0.0 } else { (alpha * fade).clamp(0.0, 1.0) };
                    let u = running.scroll[0] - at as f32 * repeats_u;
                    let place = point.place;
                    let side = |sign: f32, v: f32| -> [f32; 5] {
                        let [x, y, z] = std::array::from_fn(|axis| place[axis] + across[axis] * half * sign);
                        [x, y, z, u, v]
                    };
                    (
                        side(-1.0, repeats_v + running.scroll[1]),
                        side(1.0, running.scroll[1]),
                        [red * tint[0], green * tint[1], blue * tint[2], alpha],
                    )
                };
                for at in 0..last {
                    let (top, bottom, near) = vertex(at);
                    let (next_top, next_bottom, far) = vertex(at + 1);
                    let [near, far] = [near, far].map(|color| color.map(|channel| channel.clamp(0.0, 1.0)));
                    let colors = [near, far, near, far];
                    quads.push(CeContrailQuad { contrail: index, image, corners: [top, next_top, bottom, next_bottom], colors });
                }
            }
        }
        quads
    }
}

impl super::functions::ReadsOutputs for CeContrailAt {
    fn object(&self) -> u8 {
        self.object
    }
    fn outputs_read(&self) -> Vec<usize> {
        self.primary.into_iter().collect()
    }
}

/// `contrail_add_points`: `count` new points at each marker, aimed into the
/// tag's cone round the marker's forward.
fn add_points(contrail: &CeContrailAt, running: &mut RunningContrail, count: usize, bones: Option<&[[f32; 4]]>) {
    let def = &contrail.def;
    let density = running.density;
    let cone = def.cone * if def.scale_flags & SCALED_CONE != 0 { density } else { 1.0 };
    for (line, (node, position, forward)) in running.points.iter_mut().zip(&contrail.markers) {
        let (origin, forward) = posed_on(*node, *position, *forward, bones);
        let forward = unit_or_up(forward);
        // `contrail_add_points` spreads what one frame adds along the way
        // from the last head point to the marker; the last lands on it.
        let head = line.first().map(|point| (point.place, point.velocity, point.density));
        for added in 1..=count {
            if line.len() >= MOST_POINTS {
                break;
            }
            let speed = scaled_range(def.velocity, def.scale_flags, SCALED_VELOCITY, density, running.random.next());
            // `seed_random_vector_in_cone3d`.
            let spread = cone * running.random.next();
            let turn = running.random.next() * std::f32::consts::TAU;
            let side = unit_or_up(cross(forward, [0.0, 0.0, 1.0]));
            let side = if dot(side, side) < 0.5 { [0.0, 1.0, 0.0] } else { side };
            let lift = cross(side, forward);
            let direction: [f32; 3] = std::array::from_fn(|axis| {
                forward[axis] * spread.cos() + (side[axis] * turn.cos() + lift[axis] * turn.sin()) * spread.sin()
            });
            let velocity = direction.map(|axis| axis * speed);
            let (place, velocity, point_density) = match head.filter(|_| added < count) {
                Some((from, from_velocity, from_density)) => {
                    let t = added as f32 / count as f32;
                    let lerp = |from: [f32; 3], to: [f32; 3]| -> [f32; 3] {
                        std::array::from_fn(|axis| from[axis] * (1.0 - t) + to[axis] * t)
                    };
                    (lerp(from, origin), lerp(from_velocity, velocity), from_density * (1.0 - t) + density * t)
                }
                None => (origin, velocity, density),
            };
            line.insert(
                0,
                Point {
                    place,
                    velocity,
                    state: -1,
                    age: 0.0,
                    inverse: 0.0,
                    moving_on: true,
                    done: false,
                    new: true,
                    density: point_density,
                },
            );
        }
    }
}




#[cfg(test)]
mod tests {
    use super::*;

    fn state(duration: f32, transition: f32) -> PointState {
        PointState {
            duration: [duration; 2],
            transition: [transition; 2],
            physics: None,
            width: 0.1,
            colors: [[0.0; 4], [1.0; 4]],
            scaled: 0x20,
        }
    }

    #[test]
    fn a_point_holds_moves_on_and_ends_at_the_last_state() {
        let states = [state(0.0, 1.0), state(0.0, 0.0)];
        let mut point = Point {
            place: [0.0; 3],
            velocity: [0.0; 3],
            state: -1,
            age: 0.0,
            inverse: 0.0,
            moving_on: true,
            done: false,
            new: true,
            density: 1.0,
        };
        let mut random = Random(1);
        step_point(&mut point, &states, 0.0, 0.1, &mut random);
        assert!(point.moving_on);
        assert_eq!(point.state, 0);
        for _ in 0..12 {
            step_point(&mut point, &states, 0.0, 0.1, &mut random);
        }
        assert!(point.done);
    }
}
