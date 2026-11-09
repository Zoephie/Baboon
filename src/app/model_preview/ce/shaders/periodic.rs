//! Halo CE's periodic functions: the tables the game builds once and evaluates by name.

use super::{high16_times_inverse_65535, lcg_step};
use std::sync::LazyLock;

/// `periodic_functions_initialize`'s seed: every table comes out the same.
const PERIODIC_SEED: u32 = 0x20F3_F660;

const PERIODIC_SAMPLES: usize = 1024;

/// Cycles each table holds, of which `periodic_function_evaluate` steps
/// `PERIODIC_RATE` samples a period: the game's periods run long by this.
const PERIODIC_CYCLES: f32 = 28.0;

const PERIODIC_RATE: f32 = 25.6;

pub(super) const PERIODIC_ONE: usize = 0;

pub(super) const PERIODIC_ZERO: usize = 1;

const PERIODIC_SLIDES: [usize; 2] = [6, 7];

struct SeedRandom(u32);

impl SeedRandom {
    fn real(&mut self) -> f32 {
        self.0 = lcg_step(self.0);
        high16_times_inverse_65535(self.0)
    }
}

/// `periodic_function_build_variable_period_x_table`.
fn variable_period_table(random: &mut SeedRandom) -> Vec<f32> {
    let mut table = Vec::with_capacity(PERIODIC_SAMPLES);
    let mut total = 0.0;
    for index in 0..PERIODIC_SAMPLES {
        table.push(total);
        let index = index as f32;
        let first = random.real();
        let wave_a = (index * 0.025_157_286).cos() + 1.0;
        let scale_a = random.real();
        let wave_b = (index * 0.031_293_21).cos() + 1.0;
        let scale_b = random.real();
        let wave_c = (index * 0.044_792_242).cos() + 1.0;
        let scale_c = random.real();
        total += (first + 1.0) * 0.25 + wave_a * scale_a + scale_c * wave_c + wave_b * scale_b;
    }
    table.iter().map(|value| value / total).collect()
}

/// `periodic_function_build_table`.
fn periodic_table(function: usize, random: &mut SeedRandom) -> Vec<u8> {
    use std::f32::consts::{PI, TAU};
    let x_table = variable_period_table(random);
    let wave = |x: f32| {
        let x = x.fract();
        if x < 0.5 {
            x * 2.0
        } else {
            1.0 - (x - 0.5) * 2.0
        }
    };
    let values: Vec<f32> = x_table
        .iter()
        .enumerate()
        .map(|(index, varied)| {
            let steady = index as f32 * (PERIODIC_CYCLES / PERIODIC_SAMPLES as f32);
            let varied = varied * PERIODIC_CYCLES;
            match function {
                PERIODIC_ONE => 1.0,
                PERIODIC_ZERO => 0.0,
                2 => (steady * TAU).cos(),
                3 => (varied * TAU).cos(),
                4 => wave(steady),
                5 => wave(varied),
                6 => steady.fract(),
                7 => varied.fract(),
                8 => random.real(),
                // Jitter takes wander's path through the game's table builder.
                9 | 10 => {
                    ((steady * 43.9823).cos() * (steady * PI / 2.0).sin()
                        + (steady * 25.132_742).cos() * (steady * 0.897_597_9).cos())
                        * 0.5
                        + (steady * PI).sin() * (steady * TAU).cos()
                }
                _ => varied.fract() * varied.fract(),
            }
        })
        .collect();
    let lowest = values.iter().copied().fold(f32::MAX, f32::min);
    let highest = values.iter().copied().fold(f32::MIN, f32::max);
    let range = if PERIODIC_SLIDES.contains(&function) {
        0.0
    } else {
        highest - lowest
    };
    values
        .iter()
        .map(|value| {
            let value = if range != 0.0 {
                (value - lowest) / range
            } else {
                *value
            };
            (value * 255.0).clamp(0.0, 255.0) as u8
        })
        .collect()
}

static PERIODIC_TABLES: LazyLock<Vec<Vec<u8>>> = LazyLock::new(|| {
    let mut random = SeedRandom(PERIODIC_SEED);
    (0..12)
        .map(|function| periodic_table(function, &mut random))
        .collect()
});

/// `periodic_function_evaluate`.
pub(in crate::app::model_preview) fn periodic_evaluate(function: usize, time: f32) -> f32 {
    if function == PERIODIC_ONE {
        return 1.0;
    }
    let Some(table) = PERIODIC_TABLES.get(function) else {
        return 0.0;
    };
    let position = time * PERIODIC_RATE;
    let blend = position.rem_euclid(1.0);
    let index = (position - blend) as i64 as usize & (PERIODIC_SAMPLES - 1);
    let low = f32::from(table[index]) / 255.0;
    let mut high = f32::from(table[(index + 1) & (PERIODIC_SAMPLES - 1)]) / 255.0;
    if !PERIODIC_SLIDES.contains(&function) {
        return low + (high - low) * blend;
    }
    if low > 0.75 && high < 0.25 {
        high += 1.0;
    }
    let value = low + (high - low) * blend;
    if value > 1.0 { value - 1.0 } else { value }
}
