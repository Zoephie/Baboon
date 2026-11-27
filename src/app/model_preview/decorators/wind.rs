//! The wind a decorator sways in: a `.wind` tag's functions, run on the
//! preview's clock as Halo 3's `s_wind_definition::update` (dllcache
//! 0x180907f80) runs them, and the constants `setup_wind_constants`
//! (0x180908290) hands the shader.
//!
//! A level names its wind; a set on its own has none, so the preview blows
//! the globals' `default wind settings`.

use super::super::common::named_function::tag_function_at;
use super::super::load::TagReader;
use blam_tags::{TagFile, TagFunction};

/// A `.wind` tag, as its functions and gust noise.
#[derive(Debug, Clone)]
pub(crate) struct WindDefinition {
    pub path: String,
    direction: Option<TagFunction>,
    speed: Option<TagFunction>,
    bend: Option<TagFunction>,
    oscillation: Option<TagFunction>,
    frequency: Option<TagFunction>,
    gust_size: f32,
    /// The gust noise the vertex shader samples; empty for the
    /// rasterizer globals' default.
    pub gust_bitmap: String,
}

/// Every function is evaluated at the middle of its range.
const RANGE: f32 = 0.5;

/// `update`: a speed is in these units.
const SPEED_SCALE: f32 = 0.146_667 * 0.005;

/// The scroll accumulates in 20 bits of fraction.
const SCROLL_STEPS: f32 = 1_048_576.0;

/// The preview's step: the game's is its frame's length.
pub(crate) const WIND_STEP: f32 = 1.0 / 30.0;

impl WindDefinition {
    /// The globals' default wind, if the kit has one that reads.
    pub(in crate::app::model_preview) fn default_for(reader: &TagReader) -> Option<Self> {
        let globals = reader.load("globals\\globals", "globals", b"matg").ok()?;
        let path = globals
            .root()
            .read_tag_ref_path("default wind settings")?
            .trim()
            .to_owned();
        let wind = reader.load(&path, "wind", b"wind").ok()?;
        Some(Self::read(&wind, path))
    }

    fn read(tag: &TagFile, path: String) -> Self {
        let root = tag.root();
        let function = |name: &str| {
            root.descend(name)
                .and_then(|holder| tag_function_at(&holder, "Mapping"))
        };
        Self {
            path,
            direction: function("direction"),
            speed: function("speed"),
            bend: function("bend"),
            oscillation: function("oscillation"),
            frequency: function("frequency"),
            gust_size: root.read_real("gust size").unwrap_or(0.0),
            gust_bitmap: root
                .read_tag_ref_path("gust noise bitmap")
                .map(|path| path.trim().to_owned())
                .unwrap_or_default(),
        }
    }

    fn evaluate(function: &Option<TagFunction>, input: f32) -> f32 {
        function
            .as_ref()
            .map_or(0.0, |function| function.evaluate(input, RANGE))
    }
}

/// The wind's state between frames: its clock and how far the gusts have
/// scrolled, each in 20 bits of fraction as `s_wind_values` keeps them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct WindClock {
    pub time: f32,
    scroll: [u32; 2],
}

impl WindClock {
    /// Run the wind on to `time`, in steps of [`WIND_STEP`]; a clock that ran
    /// back starts over.
    pub(crate) fn advance(&mut self, wind: &WindDefinition, time: f32) {
        if time < self.time {
            *self = Self::default();
        }
        while self.time + WIND_STEP <= time {
            let (sin, cos) = direction(wind, self.time).sin_cos();
            let frequency = WindDefinition::evaluate(&wind.frequency, speed(wind, self.time));
            for (scroll, along) in self.scroll.iter_mut().zip([sin, cos]) {
                // Summed as a float, then cut to a whole step, as `update` does.
                let moved = (frequency * along * WIND_STEP * SCROLL_STEPS + *scroll as f32) as i32;
                *scroll = (moved & 0xF_FFFF) as u32;
            }
            self.time += WIND_STEP;
        }
    }

    /// `wind_data` and `wind_data2`.
    pub(crate) fn constants(&self, wind: &WindDefinition) -> [[f32; 4]; 2] {
        let (sin, cos) = direction(wind, self.time).sin_cos();
        let speed = speed(wind, self.time);
        let bend = WindDefinition::evaluate(&wind.bend, speed);
        let oscillation = WindDefinition::evaluate(&wind.oscillation, speed);
        let gusts = if wind.gust_size != 0.0 {
            1.0 / wind.gust_size
        } else {
            0.0
        };
        [
            [
                self.scroll[0] as f32 / SCROLL_STEPS,
                self.scroll[1] as f32 / SCROLL_STEPS,
                gusts,
                0.0,
            ],
            [bend * sin, bend * cos, oscillation, self.time],
        ]
    }
}

/// The wind's heading at `time`, in radians: its function gives degrees.
fn direction(wind: &WindDefinition, time: f32) -> f32 {
    WindDefinition::evaluate(&wind.direction, time).to_radians()
}

fn speed(wind: &WindDefinition, time: f32) -> f32 {
    WindDefinition::evaluate(&wind.speed, time) * SPEED_SCALE
}
