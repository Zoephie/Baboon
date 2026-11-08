//! Halo CE's transition functions, which lights and object functions evaluate.

/// `transition_function_build_table`: a byte per step of 1023.
fn transition_table(function: i32) -> [u8; 1024] {
    std::array::from_fn(|at| {
        let x = at as f32 * 0.000_977_517_1;
        let y = match function {
            1 => x.powf(0.5),
            2 => x.powf(0.25),
            3 => x.powi(2),
            4 => x.powi(4),
            _ => ((x * std::f32::consts::PI - std::f32::consts::FRAC_PI_2).sin() + 1.0) * 0.5,
        };
        ((y * 255.0) as i32).clamp(0, 255) as u8
    })
}

/// `transition_function_evaluate`: linear as it is, the rest read from their
/// tables between neighbours.
pub(in crate::app::model_preview) fn transition_evaluate(function: i32, value: f32) -> f32 {
    // NaN goes to zero as the game's comparisons send it.
    let value = if value >= 0.0 { value.min(1.0) } else { 0.0 };
    if !(1..=5).contains(&function) {
        return value;
    }
    static TABLES: std::sync::OnceLock<[[u8; 1024]; 6]> = std::sync::OnceLock::new();
    let table = &TABLES.get_or_init(|| std::array::from_fn(|function| transition_table(function as i32)))[function as usize];
    let scaled = value * 1023.0;
    let index = (scaled - 0.5) as i32;
    let fraction = scaled % 1.0;
    let at = |index: i32| f32::from(table[index.clamp(0, 1023) as usize]) * 0.003_921_569;
    if index == 1023 { at(1023) } else { at(index) * (1.0 - fraction) + at(index + 1) * fraction }
}
